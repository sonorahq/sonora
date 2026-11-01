use crate::metrics::snapped;
use crate::palette::{CoverPalette, of_image};
use crate::skeleton::Skeleton;
use crate::theme::ActiveTheme as _;
use futures::AsyncReadExt as _;
use futures::channel::oneshot;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Asset, AssetLogger, Bounds, Context, Div, ElementId, Entity, Global,
    GlobalElementId, Hsla, ImageCache, ImageCacheError, ImageSource, InspectorElementId,
    Interactivity, LayoutId, ObjectFit, Pixels, Position, RenderImage, Resource, SharedString,
    SharedUri, Style, StyleRefinement, Styled, Task, Window, div, img, px, svg,
};
use image::{
    AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, RgbaImage,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
    imageops,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::io::Cursor;
use std::panic::Location;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

const FILE_PREFIX: &str = "file://";

const FALLBACK_ICON: &str = "icons/music.svg";
pub(crate) const ROUNDED: Pixels = px(4.);
/// What the cache trims back to at the least. The budget grows to one screenful of the
/// largest window a cover was drawn in, plus the `PREFETCH_REACH` laid out past both of its
/// edges, so a HiDPI grid never trims what it shows or is about to. The cache is allowed half
/// as much again past its budget while a scroll pulls covers in, since a trim asks every
/// window to redraw and is worth doing in one batch.
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ITEMS: usize = 256;
/// The largest edge a cover is decoded at. It sits above a fullscreen cover on a 5K screen at
/// 2x, so it only ever stops a local original of several thousand pixels from being kept whole.
const MAX_SAMPLE_EDGE: u32 = 4096;
/// The step decode edges are rounded up to. A power of two would decode up to four times
/// the pixels drawn.
const EDGE_STEP: u32 = 64;
/// The edge a cover is decoded at when only its palette is wanted.
const PALETTE_EDGE: u32 = 128;
/// Set on a decode edge when the image keeps its own aspect and is scaled to that width, for
/// banners that a square crop would cut down. A wide decode never stands in for a square one.
const WIDE: u32 = 1 << 31;
const GRACE: Duration = Duration::from_secs(5);
const KEEP_ITEMS: usize = 96;
const IDLE: Duration = Duration::from_secs(120);
const ORPHAN: Duration = Duration::from_secs(20);
const SWEEP: Duration = Duration::from_secs(30);
/// How many softened covers are kept. Only the cover waiting on its large art is drawn
/// soft, so two cover the current track and a skip back.
const SOFT_ITEMS: usize = 2;
const SOFT_SIGMA: f32 = 1.6;
const SMALL_BYTES: usize = 64 * 1024;
const BIG_BYTES: usize = 256 * 1024;
const MAX_PENDING: usize = 8;
/// How many of the `MAX_PENDING` loads a prefetched cover may take, so a cover on screen always
/// finds a free one.
const PREFETCH_PENDING: usize = MAX_PENDING / 2;
/// How far past each edge of a scrolled region rows are laid out unseen so their covers load
/// before they scroll in, as a share of the region's length.
pub const PREFETCH_REACH: f32 = 0.5;
/// How many covers decode at once. A decode holds the whole source image, so this bounds
/// the transient memory, and running decodes on their own threads keeps those buffers in a
/// couple of malloc arenas rather than one per worker thread.
const DECODERS: usize = 2;
/// How long a condemned cover is held before it is dropped. One redraw of every window
/// is all it takes for anything still on screen to ask for its cover again, and that
/// redraw is already on its way when the batch is condemned.
const REPRIEVE: Duration = Duration::from_millis(250);
/// How many cover palettes are kept. Each one is two colours, so the map costs
/// nothing beside the frames, and holding them past an eviction is what keeps a
/// button its colour while its cover is decoded again.
const TINT_ITEMS: usize = 4096;

type ArtworkKey = (Resource, u32);

thread_local! {
    /// Set while `prefetch` lays out rows that will not be drawn.
    static PREFETCHING: Cell<bool> = const { Cell::new(false) };
}

type Job = Box<dyn FnOnce() + Send>;

#[derive(Clone, Hash)]
struct ArtworkSource {
    resource: Resource,
    edge: u32,
}

#[derive(Clone)]
enum ArtworkAssetLoader {}

type ArtworkResourceLoader = AssetLogger<ArtworkAssetLoader>;

/// Reads the encoded bytes of a cover. Nothing holds on to them once the decode has
/// landed, because the HTTP disk cache makes a second read cheap.
fn fetch(
    resource: Resource,
    cx: &App,
) -> impl std::future::Future<Output = Result<Vec<u8>, ImageCacheError>> + Send + 'static {
    let client = cx.http_client();
    let asset_source = cx.asset_source().clone();

    async move {
        match resource {
            Resource::Path(path) => Ok(std::fs::read(path.as_ref())?),
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
                Ok(body)
            }
            Resource::Embedded(path) => {
                let Some(data) = asset_source.load(&path)? else {
                    return Err(ImageCacheError::Asset(
                        format!("Embedded resource not found: {path}").into(),
                    ));
                };
                Ok(data.into_owned())
            }
        }
    }
}

/// A decoded cover with the palette sampled from it. The loader samples it on its own
/// thread, so a cover landing mid-scroll costs the frame nothing beyond storing it.
#[derive(Clone)]
struct Decoded {
    image: Arc<RenderImage>,
    palette: CoverPalette,
}

impl Asset for ArtworkAssetLoader {
    type Source = ArtworkSource;
    type Output = Result<Decoded, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        let svg_renderer = cx.svg_renderer();
        let bytes = fetch(source.resource, cx);

        async move {
            let bytes = bytes.await?;

            let Ok(format) = image::guess_format(&bytes) else {
                let image = svg_renderer.render_single_frame(&bytes, 1.0)?;
                let palette = of_image(&image);
                return Ok(Decoded { image, palette });
            };
            on_decoder(move || {
                let image = Arc::new(RenderImage::new(raster_frames(
                    &bytes,
                    format,
                    source.edge,
                )?));
                let palette = of_image(&image);
                Ok(Decoded { image, palette })
            })
            .await
            .unwrap_or_else(|| Err(ImageCacheError::Asset("artwork decoder stopped".into())))
        }
    }
}

/// Runs `decode` on one of the `DECODERS` artwork threads and resolves with its result, or
/// with none when the job died with its thread. A job whose caller stopped waiting is skipped.
fn on_decoder<T: Send + 'static>(
    decode: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Option<T>> + Send + 'static {
    static QUEUE: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

    let (sender, receiver) = oneshot::channel();
    let job: Job = Box::new(move || {
        if !sender.is_canceled() {
            sender.send(decode()).ok();
        }
    });
    if let Err(mpsc::SendError(job)) = QUEUE.get_or_init(decoders).send(job) {
        job();
    }

    async move { receiver.await.ok() }
}

/// Starts the artwork decode threads and hands back the queue that feeds them. When none
/// could start, the queue refuses every job and `on_decoder` runs it in place.
fn decoders() -> mpsc::Sender<Job> {
    let (sender, receiver) = mpsc::channel::<Job>();
    let receiver = Arc::new(Mutex::new(receiver));
    for index in 0..DECODERS {
        let receiver = receiver.clone();
        let started = std::thread::Builder::new()
            .name(format!("artwork-{index}"))
            .spawn(move || {
                loop {
                    let next = receiver.lock().ok().and_then(|queue| queue.recv().ok());
                    let Some(job) = next else {
                        return;
                    };
                    job();
                }
            });
        if let Err(error) = started {
            log::warn!("artwork: cannot start a decoder: {error}");
        }
    }

    sender
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
    Ok(vec![Frame::new(artwork_frame(image, edge))])
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
                    artwork_frame(DynamicImage::ImageRgba8(frame.into_buffer()), edge),
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

/// Crops a decoded cover to its centre square, scales it down to `edge` and swaps it to
/// BGRA. The scaling happens in the decoder's own pixel format, so an RGB JPEG only grows
/// a fourth channel at the size it is drawn. An image no bigger than `edge`, or any image
/// when `edge` is zero, keeps its size. An edge flagged `WIDE` keeps the whole image and
/// scales its width down to the rest of the edge.
fn artwork_frame(image: DynamicImage, edge: u32) -> RgbaImage {
    let (width, height) = (image.width(), image.height());
    if edge & WIDE != 0 {
        let wide = edge & !WIDE;
        let image = match width <= wide {
            true => image,
            false => image.resize(wide, u32::MAX, imageops::FilterType::Triangle),
        };
        let mut image = image.into_rgba8();
        bgra(&mut image);
        return image;
    }
    let side = width.min(height);
    let image = match edge == 0 || side <= edge {
        true => image,
        false => {
            let square = match width == height {
                true => image,
                false => image.crop_imm((width - side) / 2, (height - side) / 2, side, side),
            };
            match side > edge.saturating_mul(2) {
                true => square.thumbnail_exact(edge, edge),
                false => square.resize_exact(edge, edge, imageops::FilterType::Triangle),
            }
        }
    };
    let mut image = image.into_rgba8();
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
    bytes: usize,
    /// The size in bytes of the largest window a cover was drawn in, which sets the budget.
    screen: usize,
    _sweep: Task<()>,
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
                bytes: 0,
                screen: 0,
                _sweep: sweeper(cx),
            });
            cx.set_global(Installed(cache));
        }
        cx.global::<Installed>().0.clone()
    }

    fn insert(
        &mut self,
        resource: ArtworkKey,
        decoded: Result<Decoded, ImageCacheError>,
        cx: &mut App,
    ) {
        if let Ok(decoded) = &decoded
            && !self.tints.contains_key(&resource.0)
        {
            self.trim_tints();
            self.tints.insert(resource.0.clone(), decoded.palette);
        }
        let value = decoded.map(|decoded| decoded.image);
        let bytes = value.as_ref().map_or(0, |image| image_bytes(image));
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

    /// What the cache trims back to: `CACHE_BYTES`, or one screenful of the largest window and
    /// its prefetched reach when that is more.
    fn budget(&self) -> usize {
        let reach = (self.screen as f32 * PREFETCH_REACH * 2.) as usize;
        CACHE_BYTES.max(self.screen + reach)
    }

    /// Condemns the least recently drawn covers until the cache is back inside its budget,
    /// then asks every window to redraw. The redraw is what makes the condemned batch safe
    /// to drop: it rebuilds every cached view, so nothing is replayed from last frame's
    /// primitives and everything still on screen asks for its cover again.
    fn trim(&mut self, cx: &mut App) {
        let budget = self.budget();
        if self.bytes <= budget + budget / 2 && self.items.len() <= CACHE_ITEMS {
            return;
        }
        let before = self.condemned.len();
        while self.items.len() > 1 && (self.bytes > budget || self.items.len() > CACHE_ITEMS) {
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
        }
        for image in std::mem::take(&mut self.condemned_soft) {
            cx.drop_image(image, None);
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
        let budget = self.budget();
        let mut bytes = self.bytes;
        let mut stale = Vec::new();

        for (index, (resource, used, size)) in ages.iter().enumerate() {
            if index >= protected || used.elapsed() <= GRACE {
                break;
            }
            if bytes <= budget && used.elapsed() <= IDLE {
                break;
            }
            stale.push(resource.clone());
            bytes = bytes.saturating_sub(*size);
        }

        for resource in &stale {
            self.condemn(resource);
        }
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
        let viewport = window.viewport_size();
        let scale = window.scale_factor();
        let screen =
            (viewport.width.as_f32() * viewport.height.as_f32() * scale * scale) as usize * 4;
        self.screen = self.screen.max(screen);

        let room = match PREFETCHING.get() {
            true => PREFETCH_PENDING,
            false => MAX_PENDING,
        };
        if !self.pending.contains_key(&key) && self.pending.len() >= room {
            self.reap_pending(window, cx);
            if self.pending.len() >= room {
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

        let image = value.clone().map(|decoded| decoded.image);
        self.pending.remove(&key);
        self.insert(key, value, cx);
        Some(image)
    }

    /// Whether a cover at `edge` can be drawn without waiting on a load, counting a failed load,
    /// which draws the placeholder, and a stand-in at a nearby edge.
    fn holds(&self, resource: &Resource, edge: u32, soft: bool) -> bool {
        let key = (resource.clone(), edge);
        if self.items.contains_key(&key) || self.condemned.contains_key(&key) {
            return true;
        }
        match soft {
            true => self.soft.contains_key(&key),
            false => self.items.iter().any(|((held, size), cached)| {
                held == resource && near(*size, edge) && cached.value.is_ok()
            }),
        }
    }

    /// The largest decode the cache holds of the same cover at no less than half of `edge`, to
    /// draw while the one at `edge` loads. A resize that moves a cover to the next edge step
    /// keeps it on screen instead of flashing the skeleton.
    fn stand_in(&mut self, resource: &Resource, edge: u32) -> Option<Arc<RenderImage>> {
        let (_, cached) = self
            .items
            .iter_mut()
            .filter(|((held, size), cached)| {
                held == resource && near(*size, edge) && cached.value.is_ok()
            })
            .max_by_key(|((_, size), _)| *size)?;
        cached.used = Instant::now();
        cached.value.clone().ok()
    }
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

/// The edge a cover drawn at `size` is decoded at: its physical size rounded up to the next
/// `EDGE_STEP`, so nearby sizes share one decode, and never past `MAX_SAMPLE_EDGE`.
fn sample_edge(size: Pixels, window: &Window) -> u32 {
    let physical = ((size / px(1.)) * window.scale_factor()).ceil().max(1.) as u32;
    physical.next_multiple_of(EDGE_STEP).min(MAX_SAMPLE_EDGE)
}

/// Whether a decode at `held` may stand in for one at `edge`: the same shape, square or wide,
/// at no less than half the size.
fn near(held: u32, edge: u32) -> bool {
    held & WIDE == edge & WIDE && held & !WIDE >= (edge & !WIDE) / 2
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

/// The palette of the cover at `url` without keeping its pixels. A cover the cache has
/// drawn answers from its stored palette, and any other is decoded at `PALETTE_EDGE`,
/// which is plenty for a hue count and costs a fraction of the full-size frame.
pub(crate) fn sample_palette(url: SharedString, cx: &mut App) -> Task<CoverPalette> {
    if let Some(palette) = cover_palette(&url, cx) {
        return Task::ready(palette);
    }
    let load = ArtworkAssetLoader::load(
        ArtworkSource {
            resource: resource(url),
            edge: PALETTE_EDGE,
        },
        cx,
    );

    cx.background_spawn(async move {
        load.await
            .map(|decoded| decoded.palette)
            .unwrap_or_default()
    })
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

/// Lays out `element` without drawing it, so the covers inside start loading before it
/// scrolls into view. Those loads take at most `PREFETCH_PENDING` of the slots, so a cover
/// already on screen never waits behind one that is not.
pub fn prefetch(mut element: AnyElement, window: &mut Window, cx: &mut App) {
    let outer = PREFETCHING.replace(true);
    element.request_layout(window, cx);
    PREFETCHING.set(outer);
}

/// Rows just past the edge of a list, laid out with `prefetch` while the list is, so their
/// covers are loading when they scroll in. It takes no room and draws nothing.
pub struct Prefetch {
    rows: Vec<AnyElement>,
}

impl Prefetch {
    pub fn new(rows: impl IntoIterator<Item = AnyElement>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }
}

impl IntoElement for Prefetch {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Prefetch {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        for row in self.rows.drain(..) {
            prefetch(row, window, cx);
        }
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };

        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
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
    /// The height of a wide image, which is `size` wide. None keeps the square.
    tall: Option<Pixels>,
    circle: bool,
    radius: Option<Pixels>,
    fallback: SharedString,
    accent: bool,
    soft: bool,
    overlay: bool,
    interactivity: Interactivity,
}

impl Artwork {
    #[track_caller]
    pub fn new(url: Option<impl Into<SharedString>>) -> Self {
        Self {
            url: url.map(Into::into),
            size: px(28.),
            tall: None,
            circle: false,
            radius: None,
            soft: false,
            overlay: false,
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

    /// Draws the image `size` wide and `height` tall, keeping its own aspect and covering the
    /// box instead of cropping it to a square first. Meant for banners.
    pub fn wide(mut self, height: Pixels) -> Self {
        self.tall = Some(height);
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

    /// Draws nothing while the image loads or after it fails, for a cover stacked over another
    /// that already shows, so neither the skeleton nor the placeholder hides it.
    pub fn overlay(mut self) -> Self {
        self.overlay = true;
        self
    }

    pub fn accent(mut self) -> Self {
        self.accent = true;
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
            tall,
            circle,
            radius,
            fallback,
            accent,
            soft,
            overlay,
            interactivity,
        } = self;
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let glyph = match accent {
            true => theme.tint.unwrap_or(theme.primary),
            false => muted.opacity(0.5),
        };
        let size = snapped(size, window);
        let wide = tall.is_some();
        let tall = tall.map_or(size, |tall| snapped(tall, window));
        let rounded = match (circle, radius) {
            (true, _) => size / 2.,
            (false, Some(radius)) => radius,
            (false, None) => cx.theme().radius.min(ROUNDED),
        };
        let placeholder = {
            let fallback = fallback.clone();
            move || match overlay {
                true => div().w(size).h(tall).into_any_element(),
                false => {
                    blank(size, tall, rounded, muted, glyph, fallback.clone()).into_any_element()
                }
            }
        };

        match url {
            Some(url) => {
                let cache = ArtworkCache::entity(cx);
                let resource = resource(url);
                let edge = match wide {
                    true => sample_edge(size, window) | WIDE,
                    false => sample_edge(size, window),
                };
                if PREFETCHING.get() {
                    cache.update(cx, |cache, cx| cache.load_at(&resource, edge, window, cx));
                    return div().w(size).h(tall).into_any_element();
                }
                // Drawn at once rather than after `img`'s loading delay, and under the cover,
                // so a cover that lands mid-pulse simply covers it.
                let waiting = !overlay && !cache.read(cx).holds(&resource, edge, soft);
                let source = ImageSource::Custom(Arc::new({
                    let cache = cache.clone();
                    move |window, cx| {
                        if let Some(prepared) =
                            cache.update(cx, |cache, _| cache.prepared(&resource, edge, soft))
                        {
                            return Some(Ok(prepared));
                        }
                        let Some(loaded) = cache
                            .update(cx, |cache, cx| cache.load_at(&resource, edge, window, cx))
                        else {
                            return match soft {
                                true => None,
                                false => cache
                                    .update(cx, |cache, _| cache.stand_in(&resource, edge))
                                    .map(Ok),
                            };
                        };
                        let loaded = loaded.map(|image| {
                            cache.update(cx, |cache, cx| {
                                cache.prepare(&resource, edge, soft, image, cx)
                            })
                        });
                        Some(loaded)
                    }
                }));
                refined(
                    div()
                        .relative()
                        .w(size)
                        .h(tall)
                        .when(waiting, |this| {
                            this.child(
                                Skeleton::new()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .w(size)
                                    .h(tall)
                                    .rounded(rounded),
                            )
                        })
                        .child(
                            img(source)
                                .image_cache(&cache)
                                .w(size)
                                .h(tall)
                                .object_fit(ObjectFit::Cover)
                                .rounded(rounded)
                                .with_fallback(placeholder),
                        ),
                    interactivity,
                )
                .into_any_element()
            }
            None => refined(
                blank(size, tall, rounded, muted, glyph, fallback),
                interactivity,
            )
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

fn blank(
    size: Pixels,
    tall: Pixels,
    rounded: Pixels,
    muted: Hsla,
    glyph: Hsla,
    fallback: SharedString,
) -> Div {
    div()
        .w(size)
        .h(tall)
        .rounded(rounded)
        .bg(muted.opacity(0.12))
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(icons::path(fallback))
                .size(size.min(tall) * 0.46)
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
        let frame = artwork_frame(DynamicImage::ImageRgba8(image), 64);

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
