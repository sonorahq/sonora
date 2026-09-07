use crate::metrics::snapped;
use crate::skeleton::Skeleton;
use crate::theme::ActiveTheme as _;
use futures::AsyncReadExt as _;
use gpui::prelude::*;
use gpui::{
    App, Asset, AssetLogger, Context, Div, Entity, Global, Hsla, ImageCache, ImageCacheError,
    ImageSource, Interactivity, ObjectFit, Pixels, RenderImage, Resource, SharedString, SharedUri,
    StyleRefinement, Styled, Task, Window, div, img, px, svg,
};
use image::{
    AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, RgbaImage,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
    imageops,
};
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};
use std::{collections::HashMap, sync::Arc};

const FILE_PREFIX: &str = "file://";

const FALLBACK_ICON: &str = "icons/music.svg";
pub(crate) const ROUNDED: Pixels = px(4.);
const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ITEMS: usize = 256;
const HARD_BYTES: usize = 192 * 1024 * 1024;
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
    for pixel in image.chunks_exact_mut(4) {
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
    pending: HashMap<ArtworkKey, Instant>,
    soft: HashMap<(Resource, u32), Arc<RenderImage>>,
    bytes: usize,
    _sweep: Task<()>,
}

struct Installed(Entity<ArtworkCache>);

impl Global for Installed {}

impl ArtworkCache {
    fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let cache = cx.new(|cx| Self {
                items: HashMap::new(),
                pending: HashMap::new(),
                soft: HashMap::new(),
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
        window: &mut Window,
        cx: &mut App,
    ) {
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

        while self.items.len() > 1 {
            let forced = self.bytes > HARD_BYTES;
            if !forced && self.bytes <= CACHE_BYTES && self.items.len() <= CACHE_ITEMS {
                break;
            }
            let Some((resource, used)) = self.oldest() else {
                break;
            };
            if !forced && used.elapsed() < GRACE {
                break;
            }
            self.evict(&resource, Some(&mut *window), cx);
        }
    }

    fn oldest(&self) -> Option<(ArtworkKey, Instant)> {
        self.items
            .iter()
            .min_by_key(|(_, cached)| cached.used)
            .map(|(resource, cached)| (resource.clone(), cached.used))
    }

    fn evict(&mut self, resource: &ArtworkKey, window: Option<&mut Window>, cx: &mut App) {
        let Some(cached) = self.items.remove(resource) else {
            return;
        };
        self.bytes = self.bytes.saturating_sub(cached.bytes);
        cx.remove_asset::<ArtworkResourceLoader>(&ArtworkSource {
            resource: resource.0.clone(),
            edge: resource.1,
        });
        if let Ok(image) = cached.value {
            cx.drop_image(image, window);
        }
        self.release_bytes_if_unused(&resource.0, cx);
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
        window: &mut Window,
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
            for image in self.soft.drain().map(|(_, image)| image) {
                cx.drop_image(image, Some(&mut *window));
            }
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
            self.evict(resource, None, cx);
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

fn sweeper(cx: &mut Context<ArtworkCache>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(SWEEP).await;
            if this.update(cx, |this, cx| this.sweep(cx)).is_err() {
                return;
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
            self.insert(key, value, window, cx);
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
        self.insert(key, value.clone(), window, cx);
        Some(value)
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
            fallback: FALLBACK_ICON.into(),
            accent: false,
            interactivity: Interactivity::new(),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
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
                        if let Some(prepared) =
                            cache.update(cx, |cache, _| cache.prepared(&resource, edge, soft))
                        {
                            return Some(Ok(prepared));
                        }
                        let loaded = cache
                            .update(cx, |cache, cx| cache.load_at(&resource, edge, window, cx))?
                            .map(|image| {
                                cache.update(cx, |cache, cx| {
                                    cache.prepare(&resource, edge, soft, image, window, cx)
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
