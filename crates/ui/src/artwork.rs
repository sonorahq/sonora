use crate::metrics::snapped;
use crate::palette::{CoverPalette, of_image};
use crate::skeleton::Skeleton;
use crate::theme::ActiveTheme as _;
use futures::AsyncReadExt as _;
use gpui::prelude::*;
use gpui::{
    App, Asset, AssetLogger, Context, Div, ElementId, Entity, Global, Hsla, ImageCache,
    ImageCacheError, ImageId, ImageSource, Interactivity, ObjectFit, Pixels, RenderImage, Resource,
    SharedString, SharedUri, StyleRefinement, Styled, Task, Window, div, img, px, svg,
};
use image::{
    AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, Rgba, RgbaImage,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
    imageops,
};
use std::collections::VecDeque;
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};
use std::{collections::HashMap, sync::Arc};

const FILE_PREFIX: &str = "file://";

const FALLBACK_ICON: &str = "icons/music.svg";
pub(crate) const ROUNDED: Pixels = px(4.);
/// What the cache trims back to. It is allowed past this while a scroll pulls covers
/// in, and only trims once it crosses `CACHE_CEILING`, since a trim asks every window
/// to redraw and is worth doing in one batch rather than a cover at a time.
const CACHE_BYTES: usize = 32 * 1024 * 1024;
/// How far past the budget the cache runs before it trims.
const CACHE_CEILING: usize = 48 * 1024 * 1024;
const CACHE_ITEMS: usize = 256;
const MAX_SAMPLE_EDGE: u32 = 1024;
const GRACE: Duration = Duration::from_secs(5);
const KEEP_ITEMS: usize = 96;
const IDLE: Duration = Duration::from_secs(120);
const ORPHAN: Duration = Duration::from_secs(20);
const SWEEP: Duration = Duration::from_secs(30);
const SOFT_ITEMS: usize = 8;
const SOFT_SIGMA: f32 = 1.6;
const SMALL_BYTES: usize = 64 * 1024;
const BIG_BYTES: usize = 256 * 1024;
const MAX_PENDING: usize = 8;
/// How long a condemned cover is held before it is dropped. One redraw of every window
/// is all it takes for anything still on screen to ask for its cover again, and that
/// redraw is already on its way when the batch is condemned.
const REPRIEVE: Duration = Duration::from_millis(250);
/// How many cover palettes are kept. Each one is two colours, so the map costs
/// nothing beside the frames, and holding them past an eviction is what keeps a
/// button its colour while its cover is decoded again.
const TINT_ITEMS: usize = 4096;
/// A turned cover is cut this many times per revolution — one degree, finer
/// than the eye follows at the speed a record turns — and this many cuts are
/// kept. Holding them is cheaper than making them again, and the handful covers
/// a second of turning, long past the frame that last showed the oldest.
const TURN_STEPS: u32 = 360;
const TURN_HELD: usize = 24;
/// A cut is made from a copy no larger than this on a side. The cost of a turn
/// is the square of the edge, and a cover in motion hides detail a still one
/// would not.
const TURN_EDGE: u32 = 384;
/// How many covers may be held mid-turn at once. Only the fullscreen record
/// turns, but a scroll past it would otherwise leave a base behind per cover.
const TURN_COVERS: usize = 4;

type ArtworkKey = (Resource, u32);

#[derive(Clone, Hash)]
struct ArtworkSource {
    resource: Resource,
    edge: u32,
}

#[derive(Clone)]
enum ArtworkAssetLoader {}

type ArtworkResourceLoader = AssetLogger<ArtworkAssetLoader>;

#[derive(Clone)]
enum ArtworkBytesLoader {}

impl Asset for ArtworkBytesLoader {
    type Source = Resource;
    type Output = Result<Arc<Vec<u8>>, ImageCacheError>;

    fn load(
        resource: Self::Source,
        cx: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        let client = cx.http_client();
        let asset_source = cx.asset_source().clone();

        async move {
            let bytes = match resource {
                Resource::Path(path) => std::fs::read(path.as_ref())?,
                Resource::Uri(uri) => {
                    let mut response = client.get(uri.as_ref(), ().into(), true).await?;
                    let mut body = Vec::new();
                    response.body_mut().read_to_end(&mut body).await?;
                    if !response.status().is_success() {
                        let mut body = String::from_utf8_lossy(&body).into_owned();
                        let first_line = body.lines().next().unwrap_or("").trim_end();
                        body.truncate(first_line.len());
                        return Err(ImageCacheError::BadStatus {
                            uri,
                            status: response.status(),
                            body,
                        });
                    }
                    body
                }
                Resource::Embedded(path) => {
                    let Some(data) = asset_source.load(&path)? else {
                        return Err(ImageCacheError::Asset(
                            format!("Embedded resource not found: {path}").into(),
                        ));
                    };
                    data.into_owned()
                }
            };

            Ok(Arc::new(bytes))
        }
    }
}

impl Asset for ArtworkAssetLoader {
    type Source = ArtworkSource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        let svg_renderer = cx.svg_renderer();
        let (bytes, _) = cx.fetch_asset::<ArtworkBytesLoader>(&source.resource);

        async move {
            let bytes = bytes.await?;

            let Ok(format) = image::guess_format(&bytes) else {
                return svg_renderer
                    .render_single_frame(&bytes, 1.0)
                    .map_err(Into::into);
            };

            Ok(Arc::new(RenderImage::new(raster_frames(
                &bytes,
                format,
                source.edge,
            )?)))
        }
    }
}

fn raster_frames(
    bytes: &[u8],
    format: ImageFormat,
    edge: u32,
) -> Result<Vec<Frame>, ImageCacheError> {
    match format {
        ImageFormat::Gif => animated_frames(GifDecoder::new(Cursor::new(bytes))?, edge),
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))?;
            if decoder.has_animation() {
                let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                animated_frames(decoder, edge)
            } else {
                static_frame(decoder, edge)
            }
        }
        _ => {
            let decoder =
                image::ImageReader::with_format(Cursor::new(bytes), format).into_decoder()?;
            static_frame(decoder, edge)
        }
    }
}

fn static_frame(mut decoder: impl ImageDecoder, edge: u32) -> Result<Vec<Frame>, ImageCacheError> {
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(vec![Frame::new(artwork_frame(image.into_rgba8(), edge))])
}

fn animated_frames<'a>(
    decoder: impl AnimationDecoder<'a>,
    edge: u32,
) -> Result<Vec<Frame>, ImageCacheError> {
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        match frame {
            Ok(frame) => {
                let delay = frame.delay();
                frames.push(Frame::from_parts(
                    artwork_frame(frame.into_buffer(), edge),
                    0,
                    0,
                    delay,
                ));
            }
            Err(error) => log::debug!("Skipping artwork animation frame: {error}"),
        }
    }
    if frames.is_empty() {
        return Err(ImageCacheError::Asset(
            "Animated artwork contained no decodable frames".into(),
        ));
    }
    Ok(frames)
}

fn artwork_frame(mut image: RgbaImage, edge: u32) -> RgbaImage {
    if edge == 0 {
        bgra(&mut image);
        return image;
    }

    let (width, height) = image.dimensions();
    let side = width.min(height);
    if side <= edge {
        bgra(&mut image);
        return image;
    }

    let square = imageops::crop_imm(&image, (width - side) / 2, (height - side) / 2, side, side);
    let mut image = match side > edge.saturating_mul(2) {
        true => imageops::thumbnail(&*square, edge, edge),
        false => imageops::resize(&*square, edge, edge, imageops::FilterType::Triangle),
    };
    bgra(&mut image);
    image
}

fn bgra(image: &mut RgbaImage) {
    for pixel in image.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
}

struct Cached {
    value: Result<Arc<RenderImage>, ImageCacheError>,
    bytes: usize,
    used: Instant,
}

struct ArtworkCache {
    items: HashMap<ArtworkKey, Cached>,
    /// Covers taken out of `items` and kept alive until the windows have redrawn once. See
    /// `condemn`.
    condemned: HashMap<ArtworkKey, Cached>,
    condemned_soft: Vec<Arc<RenderImage>>,
    condemned_at: Option<Instant>,
    pending: HashMap<ArtworkKey, Instant>,
    soft: HashMap<(Resource, u32), Arc<RenderImage>>,
    /// The palette of every cover decoded this run, kept apart from the frames
    /// so an eviction never costs a button its colour.
    tints: HashMap<Resource, CoverPalette>,
    /// The covers being turned, kept apart from `items` like `soft` is: a cut
    /// is a frame of its own, and an eviction should not cost the record its
    /// pose.
    turns: HashMap<ArtworkKey, Turned>,
    bytes: usize,
    _sweep: Task<()>,
}

/// One cover being turned: the square every cut is taken from, and the cuts
/// taken from it so far.
struct Turned {
    /// The frames the square was taken from, so a cover decoded again — or
    /// softened, or sampled at another edge — is cut anew.
    of: ImageId,
    base: RgbaImage,
    held: VecDeque<(u32, Arc<RenderImage>)>,
}

struct Installed(Entity<ArtworkCache>);

impl Global for Installed {}

impl ArtworkCache {
    fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let cache = cx.new(|cx| Self {
                items: HashMap::new(),
                condemned: HashMap::new(),
                condemned_soft: Vec::new(),
                condemned_at: None,
                pending: HashMap::new(),
                soft: HashMap::new(),
                tints: HashMap::new(),
                turns: HashMap::new(),
                bytes: 0,
                _sweep: sweeper(cx),
            });
            cx.set_global(Installed(cache));
        }
        cx.global::<Installed>().0.clone()
    }

    fn insert(
        &mut self,
        resource: ArtworkKey,
        value: Result<Arc<RenderImage>, ImageCacheError>,
        cx: &mut App,
    ) {
        let bytes = value.as_ref().map_or(0, |image| image_bytes(image));
        if let Ok(image) = &value
            && !self.tints.contains_key(&resource.0)
        {
            let palette = of_image(image);
            self.trim_tints();
            self.tints.insert(resource.0.clone(), palette);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.items.insert(
            resource,
            Cached {
                value,
                bytes,
                used: Instant::now(),
            },
        );

        self.trim(cx);
    }

    /// Drops the palettes of covers no longer held, once the map has grown past
    /// its cap. Scrolling through more art than that pays one pass.
    fn trim_tints(&mut self) {
        if self.tints.len() < TINT_ITEMS {
            return;
        }
        let live = &self.items;
        self.tints
            .retain(|resource, _| live.keys().any(|key| &key.0 == resource));
    }

    fn oldest(&self) -> Option<(ArtworkKey, Instant)> {
        self.items
            .iter()
            .min_by_key(|(_, cached)| cached.used)
            .map(|(resource, cached)| (resource.clone(), cached.used))
    }

    /// Takes a cover out of the cache without dropping it. Dropping one frees its tile in
    /// the GPU atlas, and a view whose layout was cached redraws from the primitives it
    /// recorded last frame, so a cover still shown there would be painted with whatever art
    /// took its tile over. A condemned cover keeps its frames until `flush`, and `load_at`
    /// takes it back the moment anything asks for it again.
    fn condemn(&mut self, resource: &ArtworkKey) {
        let Some(cached) = self.items.remove(resource) else {
            return;
        };
        self.bytes = self.bytes.saturating_sub(cached.bytes);
        self.condemned.insert(resource.clone(), cached);
    }

    /// Condemns the least recently drawn covers until the cache is back inside its budget,
    /// then asks every window to redraw. The redraw is what makes the condemned batch safe
    /// to drop: it rebuilds every cached view, so nothing is replayed from last frame's
    /// primitives and everything still on screen asks for its cover again.
    fn trim(&mut self, cx: &mut App) {
        if self.bytes <= CACHE_CEILING && self.items.len() <= CACHE_ITEMS {
            return;
        }
        let before = self.condemned.len();
        while self.items.len() > 1 && (self.bytes > CACHE_BYTES || self.items.len() > CACHE_ITEMS) {
            let Some((resource, _)) = self.oldest() else {
                break;
            };
            self.condemn(&resource);
        }
        if self.condemned.len() > before {
            self.condemned_at = Some(Instant::now());
            cx.refresh_windows();
        }
    }

    /// Drops every cover still condemned once the redraw that `trim` asked for has been and
    /// gone. Whatever is left here was on no screen through a full rebuild of every view.
    fn flush(&mut self, cx: &mut App) {
        if self.condemned_at.is_none_or(|at| at.elapsed() < REPRIEVE) {
            return;
        }
        self.condemned_at = None;
        for (resource, cached) in std::mem::take(&mut self.condemned) {
            cx.remove_asset::<ArtworkResourceLoader>(&ArtworkSource {
                resource: resource.0.clone(),
                edge: resource.1,
            });
            if let Ok(image) = cached.value {
                cx.drop_image(image, None);
            }
            self.release_bytes_if_unused(&resource.0, cx);
        }
        for image in std::mem::take(&mut self.condemned_soft) {
            cx.drop_image(image, None);
        }
    }

    fn release_bytes_if_unused(&self, resource: &Resource, cx: &mut App) {
        let is_used = self.items.keys().any(|key| &key.0 == resource)
            || self.pending.keys().any(|key| &key.0 == resource);
        if !is_used {
            cx.remove_asset::<ArtworkBytesLoader>(resource);
        }
    }

    fn prepared(&self, resource: &Resource, edge: u32, soft: bool) -> Option<Arc<RenderImage>> {
        match soft {
            true => self.soft.get(&(resource.clone(), edge)).cloned(),
            false => None,
        }
    }

    fn prepare(
        &mut self,
        resource: &Resource,
        edge: u32,
        soft: bool,
        image: Arc<RenderImage>,
        cx: &mut App,
    ) -> Arc<RenderImage> {
        if !soft {
            return image;
        }

        let key = (resource.clone(), edge);
        if let Some(found) = self.soft.get(&key) {
            return found.clone();
        }
        if self.soft.len() >= SOFT_ITEMS {
            self.condemned_soft
                .extend(self.soft.drain().map(|(_, image)| image));
            self.condemned_at = Some(Instant::now());
            cx.refresh_windows();
        }
        let Some(softened) = blurred(&image) else {
            return image;
        };
        self.soft.insert(key, softened.clone());
        softened
    }

    fn sweep(&mut self, cx: &mut App) {
        let held = self.items.len();
        let abandoned: Vec<ArtworkKey> = self
            .pending
            .iter()
            .filter(|(_, started)| started.elapsed() > ORPHAN)
            .map(|(resource, _)| resource.clone())
            .collect();

        for resource in &abandoned {
            self.pending.remove(resource);
            cx.remove_asset::<ArtworkResourceLoader>(&ArtworkSource {
                resource: resource.0.clone(),
                edge: resource.1,
            });
            self.release_bytes_if_unused(&resource.0, cx);
        }

        let mut ages: Vec<(ArtworkKey, Instant, usize)> = self
            .items
            .iter()
            .map(|(resource, cached)| (resource.clone(), cached.used, cached.bytes))
            .collect();
        ages.sort_unstable_by_key(|(_, used, _)| *used);

        let idle = ages
            .iter()
            .filter(|(_, used, _)| used.elapsed() > IDLE)
            .count();
        let protected = ages.len().saturating_sub(KEEP_ITEMS);
        let mut bytes = self.bytes;
        let mut stale = Vec::new();

        for (index, (resource, used, size)) in ages.iter().enumerate() {
            if index >= protected || used.elapsed() <= GRACE {
                break;
            }
            if bytes <= CACHE_BYTES && used.elapsed() <= IDLE {
                break;
            }
            stale.push(resource.clone());
            bytes = bytes.saturating_sub(*size);
        }

        for resource in &stale {
            self.condemn(resource);
        }
        self.trim_turns();
        if !stale.is_empty() {
            self.condemned_at = Some(Instant::now());
            cx.refresh_windows();
        }

        let tiny = self.count(..SMALL_BYTES);
        let small = self.count(SMALL_BYTES..BIG_BYTES);
        let big = self.count(BIG_BYTES..);

        log::debug!(
            "artwork: {} held / {} KiB, dropped {}, idle {idle}, abandoned {}, waiting {}, sizes {tiny}/{small}/{big}",
            self.items.len(),
            self.bytes / 1024,
            held - self.items.len(),
            abandoned.len(),
            self.pending.len()
        );
    }

    fn count(&self, range: impl std::ops::RangeBounds<usize>) -> usize {
        self.items
            .values()
            .filter(|cached| range.contains(&cached.bytes))
            .count()
    }
}

/// Releases condemned covers as their reprieve runs out and sweeps the cache on its own
/// slower clock. The two share one timer because a tick that finds nothing to release costs
/// a look at an empty map.
fn sweeper(cx: &mut Context<ArtworkCache>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut swept = Instant::now();
        loop {
            cx.background_executor().timer(REPRIEVE).await;
            let due = swept.elapsed() >= SWEEP;
            let alive = this.update(cx, |this, cx| {
                this.flush(cx);
                if due {
                    this.sweep(cx);
                }
            });
            if alive.is_err() {
                return;
            }
            if due {
                swept = Instant::now();
            }
        }
    })
}

impl ImageCache for ArtworkCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.load_at(resource, 0, window, cx)
    }
}

impl ArtworkCache {
    fn reap_pending(&mut self, window: &mut Window, cx: &mut App) {
        let pending: Vec<ArtworkKey> = self.pending.keys().cloned().collect();
        for key in pending {
            let source = ArtworkSource {
                resource: key.0.clone(),
                edge: key.1,
            };
            let Some(value) = window.use_asset::<ArtworkResourceLoader>(&source, cx) else {
                continue;
            };
            self.pending.remove(&key);
            self.insert(key, value, cx);
        }
    }

    fn load_at(
        &mut self,
        resource: &Resource,
        edge: u32,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = (resource.clone(), edge);
        if let Some(cached) = self.items.get_mut(&key) {
            cached.used = Instant::now();
            return Some(cached.value.clone());
        }
        // Still on screen after all, so it goes back in the cache rather than being dropped.
        if let Some(mut cached) = self.condemned.remove(&key) {
            cached.used = Instant::now();
            let value = cached.value.clone();
            self.bytes = self.bytes.saturating_add(cached.bytes);
            self.items.insert(key, cached);
            return Some(value);
        }

        if !self.pending.contains_key(&key) && self.pending.len() >= MAX_PENDING {
            self.reap_pending(window, cx);
            if self.pending.len() >= MAX_PENDING {
                return None;
            }
        }

        let source = ArtworkSource {
            resource: resource.clone(),
            edge,
        };
        let Some(value) = window.use_asset::<ArtworkResourceLoader>(&source, cx) else {
            self.pending.insert(key, Instant::now());
            return None;
        };

        self.pending.remove(&key);
        self.insert(key, value.clone(), cx);
        Some(value)
    }

    /// The cover turned by `turns` of a revolution about its own centre. The
    /// renderer has no rotation for images, so a turn is cut on the CPU from
    /// the decoded frames and handed back as a frame of its own; cuts are held
    /// per degree, and only as many as the record is passing through.
    fn turned(
        &mut self,
        key: &ArtworkKey,
        image: &Arc<RenderImage>,
        turns: f32,
    ) -> Arc<RenderImage> {
        let step = ((turns - turns.floor()) * TURN_STEPS as f32) as u32 % TURN_STEPS;
        if self
            .turns
            .get(key)
            .is_none_or(|turned| turned.of != image.id)
        {
            let Some(turned) = Turned::of(image) else {
                log::warn!("artwork: cannot turn a cover");
                return image.clone();
            };
            self.turns.insert(key.clone(), turned);
        }
        let turned = self.turns.get_mut(key).expect("held above");
        if let Some(found) = turned.held.iter().find(|(at, _)| *at == step) {
            return found.1.clone();
        }

        let Some(cut) = cut(image, &turned.base, step) else {
            log::warn!("artwork: cannot cut a turn");
            return image.clone();
        };
        if turned.held.len() >= TURN_HELD {
            turned.held.pop_front();
        }
        turned.held.push_back((step, cut.clone()));
        cut
    }

    /// Keeps the covers mid-turn to those still held, then to those still worth
    /// holding. A cut is cheap to rebuild and dear to keep, so a cover that has
    /// left the cache takes its turns with it.
    fn trim_turns(&mut self) {
        if self.turns.len() <= TURN_COVERS {
            return;
        }
        self.turns.retain(|key, _| self.items.contains_key(key));
        while self.turns.len() > TURN_COVERS {
            let Some(key) = self.turns.keys().next().cloned() else {
                break;
            };
            self.turns.remove(&key);
        }
    }
}

impl Turned {
    /// The square a cover's turns are cut from: centred, and no larger than
    /// `TURN_EDGE` however large the cover was decoded.
    fn of(image: &RenderImage) -> Option<Self> {
        let size = image.size(0);
        let (width, height) = (size.width.0.max(0) as u32, size.height.0.max(0) as u32);
        let bytes = image.as_bytes(0)?.to_vec();
        let whole = RgbaImage::from_raw(width, height, bytes)?;

        let side = width.min(height);
        let square =
            imageops::crop_imm(&whole, (width - side) / 2, (height - side) / 2, side, side)
                .to_image();
        let base = match side > TURN_EDGE {
            true => imageops::thumbnail(&square, TURN_EDGE, TURN_EDGE),
            false => square,
        };

        Some(Self {
            of: image.id,
            base,
            held: VecDeque::new(),
        })
    }
}

/// Cuts `base` turned by `step` of `TURN_STEPS` about its centre, masked to a
/// circle. A square turned about its centre leaves its corners behind, and the
/// record's label is round, so the mask costs a comparison a pixel and hides
/// what a turn would otherwise show.
fn cut(image: &RenderImage, base: &RgbaImage, step: u32) -> Option<Arc<RenderImage>> {
    let (edge, _) = base.dimensions();
    if edge == 0 {
        return None;
    }
    let (sin, cos) = (std::f32::consts::TAU * step as f32 / TURN_STEPS as f32).sin_cos();
    let middle = edge as f32 / 2.;

    let mut turned = RgbaImage::new(edge, edge);
    for y in 0..edge {
        for x in 0..edge {
            // Walking the destination outwards and asking where the pixel came
            // from: the inverse of the turn, so every pixel is written once.
            let dx = x as f32 - middle + 0.5;
            let dy = y as f32 - middle + 0.5;
            let reach = (dx * dx + dy * dy).sqrt();
            let cover = (middle - reach).clamp(0., 1.);
            if cover <= 0. {
                continue;
            }
            let from = x as f32 - middle;
            let at = y as f32 - middle;
            let sx = from * cos + at * sin + middle - 0.5;
            let sy = -from * sin + at * cos + middle - 0.5;

            let mut pixel = sample(base, sx, sy);
            pixel.0[3] = (pixel.0[3] as f32 * cover) as u8;
            turned.put_pixel(x, y, pixel);
        }
    }

    Some(Arc::new(RenderImage::new([Frame::from_parts(
        turned,
        0,
        0,
        image.delay(0),
    )])))
}

/// The colour of `base` at `(x, y)`, read between pixels so a turn stays smooth
/// instead of snapping from one source pixel to the next.
fn sample(base: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let (width, height) = base.dimensions();
    let x = x.clamp(0., width as f32 - 1.);
    let y = y.clamp(0., height as f32 - 1.);
    let left = x as u32;
    let top = y as u32;
    let right = (left + 1).min(width - 1);
    let bottom = (top + 1).min(height - 1);
    let fx = x - left as f32;
    let fy = y - top as f32;

    let read = |x: u32, y: u32, channel: usize| base.get_pixel(x, y).0[channel] as f32;
    let mut mixed = Rgba([0; 4]);
    for (channel, value) in mixed.0.iter_mut().enumerate() {
        let upper = read(left, top, channel) * (1. - fx) + read(right, top, channel) * fx;
        let lower = read(left, bottom, channel) * (1. - fx) + read(right, bottom, channel) * fx;
        *value = (upper * (1. - fy) + lower * fy) as u8;
    }
    mixed
}

fn blurred(image: &RenderImage) -> Option<Arc<RenderImage>> {
    let frames: Vec<Frame> = (0..image.frame_count())
        .filter_map(|index| {
            let size = image.size(index);
            let width = size.width.0.max(0) as u32;
            let height = size.height.0.max(0) as u32;
            let bytes = image.as_bytes(index)?.to_vec();
            let whole = RgbaImage::from_raw(width, height, bytes)?;

            Some(Frame::from_parts(
                imageops::fast_blur(&whole, SOFT_SIGMA),
                0,
                0,
                image.delay(index),
            ))
        })
        .collect();
    if frames.len() != image.frame_count() {
        log::warn!("artwork: cannot soften an image");
        return None;
    }

    Some(Arc::new(RenderImage::new(frames)))
}

fn sample_edge(size: Pixels, window: &Window) -> u32 {
    let physical = ((size / px(1.)) * window.scale_factor()).ceil().max(1.) as u32;
    physical
        .checked_next_power_of_two()
        .filter(|edge| *edge <= MAX_SAMPLE_EDGE)
        .unwrap_or(0)
}

pub(crate) fn resource(url: impl Into<SharedString>) -> Resource {
    let url = url.into();
    match url.strip_prefix(FILE_PREFIX) {
        Some(path) => Resource::Path(Arc::from(Path::new(path))),
        None => Resource::Uri(SharedUri::from(url)),
    }
}

/// The palette of a cover the artwork cache has already decoded, or none while
/// it has not been drawn yet. Nothing is decoded here, so the colours land on
/// the frame the cover appears and never before it.
pub fn cover_palette(url: &str, cx: &App) -> Option<CoverPalette> {
    let installed = cx.try_global::<Installed>()?;
    let resource = resource(url.to_owned());

    installed.0.read(cx).tints.get(&resource).copied()
}

pub fn artwork_usage(cx: &App) -> Option<(usize, usize)> {
    let installed = cx.try_global::<Installed>()?;
    let cache = installed.0.read(cx);
    let soft_bytes: usize = cache.soft.values().map(|image| image_bytes(image)).sum();
    Some((
        cache.items.len() + cache.soft.len(),
        cache.bytes + soft_bytes,
    ))
}

fn image_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .filter_map(|frame| image.as_bytes(frame))
        .fold(0, |bytes, frame| bytes.saturating_add(frame.len()))
}

#[derive(IntoElement)]
pub struct Avatar {
    art: Artwork,
}

impl Avatar {
    #[track_caller]
    pub fn new(url: Option<impl Into<SharedString>>) -> Self {
        Self {
            art: Artwork::new(url).circle().flex_none(),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.art = self.art.size(size);
        self
    }
}

impl Styled for Avatar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.art.style()
    }
}

impl RenderOnce for Avatar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.art
    }
}

#[derive(IntoElement)]
pub struct Artwork {
    url: Option<SharedString>,
    size: Pixels,
    circle: bool,
    radius: Option<Pixels>,
    fallback: SharedString,
    accent: bool,
    soft: bool,
    /// How far the cover is turned about its centre, in revolutions.
    spin: Option<f32>,
    interactivity: Interactivity,
}

impl Artwork {
    #[track_caller]
    pub fn new(url: Option<impl Into<SharedString>>) -> Self {
        Self {
            url: url.map(Into::into),
            size: px(28.),
            circle: false,
            radius: None,
            soft: false,
            spin: None,
            fallback: FALLBACK_ICON.into(),
            accent: false,
            interactivity: Interactivity::new(),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.interactivity.element_id = Some(id.into());
        self
    }

    pub fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub fn corner_radius(mut self, radius: Pixels) -> Self {
        self.radius = Some(radius);
        self
    }

    pub fn fallback(mut self, icon: impl Into<SharedString>) -> Self {
        self.fallback = icon.into();
        self
    }

    pub fn soft(mut self, soft: bool) -> Self {
        self.soft = soft;
        self
    }

    pub fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    /// Turns the cover about its own centre by `turns` of a revolution, the way
    /// a record carries its label. Whole revolutions are meaningless, so the
    /// fraction is all that is read.
    pub fn spin(mut self, turns: f32) -> Self {
        self.spin = Some(turns);
        self
    }
}

impl Styled for Artwork {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Artwork {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl RenderOnce for Artwork {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            url,
            size,
            circle,
            radius,
            fallback,
            accent,
            soft,
            spin,
            interactivity,
        } = self;
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let glyph = match accent {
            true => theme.tint.unwrap_or(theme.primary),
            false => muted.opacity(0.5),
        };
        let size = snapped(size, window);
        let rounded = match (circle, radius) {
            (true, _) => size / 2.,
            (false, Some(radius)) => radius,
            (false, None) => cx.theme().radius.min(ROUNDED),
        };
        let placeholder = {
            let fallback = fallback.clone();
            move || blank(size, rounded, muted, glyph, fallback.clone()).into_any_element()
        };

        match url {
            Some(url) => {
                let cache = ArtworkCache::entity(cx);
                let resource = resource(url);
                let edge = sample_edge(size, window);
                let source = ImageSource::Custom(Arc::new({
                    let cache = cache.clone();
                    move |window, cx| {
                        let key = (resource.clone(), edge);
                        if let Some(prepared) =
                            cache.update(cx, |cache, _| cache.prepared(&resource, edge, soft))
                        {
                            return Some(Ok(cache.update(cx, |cache, _| match spin {
                                Some(turns) => cache.turned(&key, &prepared, turns),
                                None => prepared,
                            })));
                        }
                        let loaded = cache
                            .update(cx, |cache, cx| cache.load_at(&resource, edge, window, cx))?
                            .map(|image| {
                                cache.update(cx, |cache, cx| {
                                    let image = cache.prepare(&resource, edge, soft, image, cx);
                                    match spin {
                                        Some(turns) => cache.turned(&key, &image, turns),
                                        None => image,
                                    }
                                })
                            });
                        Some(loaded)
                    }
                }));
                refined(
                    img(source)
                        .image_cache(&cache)
                        .size(size)
                        .object_fit(ObjectFit::Cover)
                        .rounded(rounded)
                        .with_loading(move || {
                            Skeleton::new()
                                .size(size)
                                .rounded(rounded)
                                .into_any_element()
                        })
                        .with_fallback(placeholder),
                    interactivity,
                )
                .into_any_element()
            }
            None => refined(blank(size, rounded, muted, glyph, fallback), interactivity)
                .into_any_element(),
        }
    }
}

fn refined<T: Styled + InteractiveElement>(mut element: T, mut caller: Interactivity) -> T {
    let mut style = std::mem::take(element.style());
    style.refine(&caller.base_style);
    *caller.base_style = style;
    *element.interactivity() = caller;
    element
}

fn blank(size: Pixels, rounded: Pixels, muted: Hsla, glyph: Hsla, fallback: SharedString) -> Div {
    div()
        .size(size)
        .rounded(rounded)
        .bg(muted.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(icons::path(fallback))
                .size(size * 0.46)
                .text_color(glyph),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Delay, Rgba, codecs::gif::GifEncoder};

    #[test]
    fn artwork_loader_targets_the_requested_edge() {
        let image = RgbaImage::from_pixel(240, 120, Rgba([20, 40, 60, 255]));
        let frame = artwork_frame(image, 64);

        assert_eq!(frame.width(), 64);
        assert_eq!(frame.height(), 64);
    }

    #[test]
    fn artwork_loader_preserves_animated_frames() {
        let mut bytes = Vec::new();
        GifEncoder::new(&mut bytes)
            .encode_frames([
                Frame::from_parts(
                    RgbaImage::from_pixel(120, 120, Rgba([20, 40, 60, 255])),
                    0,
                    0,
                    Delay::from_numer_denom_ms(80, 1),
                ),
                Frame::from_parts(
                    RgbaImage::from_pixel(120, 120, Rgba([80, 100, 120, 255])),
                    0,
                    0,
                    Delay::from_numer_denom_ms(120, 1),
                ),
            ])
            .unwrap();

        let frames = raster_frames(&bytes, ImageFormat::Gif, 64).unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].buffer().dimensions(), (64, 64));
        assert_eq!(frames[0].delay(), Delay::from_numer_denom_ms(80, 1));
        assert_eq!(frames[1].delay(), Delay::from_numer_denom_ms(120, 1));
    }
}
