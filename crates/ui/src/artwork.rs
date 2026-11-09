//! Cover art: a shared service that fetches covers by URL, decodes them at the size they draw at
//! and hands each one to Rune as pixels, plus the cover and avatar elements that show them.
//!
//! A cover element binds its pixels and a fade the moment it is built, so a cover that lands
//! later shows and fades in without the view running again. `ui` knows nothing of HTTP: the app
//! gives [`Artwork::new`] the function that reads a URL's bytes (its HTTP client and disk cache)
//! and the one that runs a future on its runtime.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use image::{DynamicImage, ImageDecoder as _, imageops};
use rune::{El, Live, Pixels, SurfaceFit, el, icon};

use crate::motion::Springs;
use crate::palette::CoverPalette;
use crate::theme::Theme;

const FILE_PREFIX: &str = "file://";
const FALLBACK_ICON: &str = "icons/music.svg";
/// The corner a cover takes at most when the caller names none, so a round theme still draws
/// covers as tiles rather than pills.
const ROUNDED: f32 = 4.;
/// The size a cover takes when the caller names none.
const SIZE: f32 = 28.;
/// How much of the cover's shorter side its placeholder glyph spans.
const GLYPH: f32 = 0.46;
/// How strongly the placeholder's box and glyph show, as shares of the muted foreground.
const BLANK_FILL: f32 = 0.12;
const BLANK_GLYPH: f32 = 0.5;
/// What the cache trims back to, counting decoded pixels: a screenful of thumbnails and a couple
/// of full-window covers at 2x, with room to spare.
const CACHE_BYTES: usize = 96 * 1024 * 1024;
const CACHE_ITEMS: usize = 384;
/// A cover asked for this recently is on screen or about to be, so trimming never takes it.
const GRACE: Duration = Duration::from_secs(2);
/// How long a failed load stands before the next ask tries it again.
const RETRY: Duration = Duration::from_secs(60);
/// The largest edge a cover is decoded at, above a fullscreen cover on a 5K screen at 2x.
const MAX_EDGE: u32 = 4096;
/// The step decode edges are rounded up to, so nearby sizes share one decode while a drawn cover
/// is never more than this many pixels smaller than its decode.
const EDGE_STEP: u32 = 32;
/// The edge a cover is decoded at when only its palette is wanted.
const PALETTE_EDGE: u32 = 128;
/// Set on a decode edge when the image keeps its aspect and is scaled to that width, for banners
/// that a square crop would cut down. A wide decode never stands in for a square one.
const WIDE: u32 = 1 << 31;
/// How many fetches run at once, and how many of those a prefetched cover may take, so a cover on
/// screen always finds a free one.
const MAX_FETCHES: usize = 8;
const PREFETCH_FETCHES: usize = MAX_FETCHES / 2;
/// How many asks wait for a fetch. The newest go first, since they are what is on screen now, and
/// the oldest drop off the end once this many wait.
const QUEUE: usize = 128;
/// How many covers decode at once. A decode holds the whole source image, so this bounds the
/// transient memory.
const DECODERS: usize = 2;
/// How many palettes are kept. Each one is a few floats, so the map costs nothing beside the
/// pixels, and holding them past an eviction keeps a card its colour while its cover reloads.
const TINT_ITEMS: usize = 4096;
/// How far past each edge of a scrolled region items are asked for unseen, as a share of the
/// region's length, so their covers are loading before they scroll in. Virtual rows already
/// mount two viewports either side; a shelf that lays out every card uses this.
pub const PREFETCH_REACH: f32 = 0.5;

/// Reads the encoded bytes behind a cover URL: the app's HTTP client behind its disk cache.
pub type Fetch = Arc<dyn Fn(String) -> BoxFuture<'static, anyhow::Result<Vec<u8>>> + Send + Sync>;
/// Runs a future to completion on the app's runtime, detached.
pub type Spawn = Arc<dyn Fn(BoxFuture<'static, ()>) + Send + Sync>;

type Job = Box<dyn FnOnce() + Send>;
type Key = (String, u32);

/// The cover art service. Clone it freely: every clone shares one cache, and it can be captured by
/// the row builders of a virtual list. Views ask for covers while they build; the first ask for a
/// URL at a size starts its fetch.
#[derive(Clone)]
pub struct Artwork(Arc<Shared>);

/// What a cover element binds: the pixels the decode lands in and the fades between the
/// placeholder and the image. Both are live values, so a landing cover redraws without the view.
#[derive(Clone)]
pub struct Art {
    pub pixels: Pixels,
    /// 0 until the cover lands, then 1.
    pub shown: Live<f32>,
    /// 1 once the load failed, so the placeholder glyph fades in.
    pub failed: Live<f32>,
}

struct Shared {
    inner: Mutex<Inner>,
    fetch: Fetch,
    spawn: Spawn,
    wake: rune::Waker,
    jobs: Mutex<mpsc::Sender<Job>>,
    /// The window's device pixels per point, as bits, for elements built without one.
    scale: AtomicU32,
    /// Set when a palette lands, until `take_news` reads it.
    news: AtomicBool,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Key, Entry>,
    /// Keys waiting for a fetch, newest first, each marked when only a prefetch asked for it.
    queue: VecDeque<(Key, bool)>,
    fetching: usize,
    prefetching: usize,
    bytes: usize,
    tints: HashMap<String, CoverPalette>,
    /// URLs whose palette was asked for before any cover of them landed.
    asked: HashSet<String>,
    budget: Option<(usize, usize)>,
}

struct Entry {
    art: Art,
    load: Load,
    used: Instant,
}

#[derive(Clone, Copy, PartialEq)]
enum Load {
    Queued,
    Fetching,
    Decoding,
    Ready(usize),
    Failed(Instant),
}

/// A decoded cover in straight-alpha RGBA rows, with the palette sampled from it.
struct Decoded {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    palette: CoverPalette,
}

impl Artwork {
    /// Starts the service. `fetch` reads a URL's bytes, `spawn` runs the fetch on the app's
    /// runtime, and `wake` (the waker `AppSpec::setup` hands out) runs the app's `poll` when a
    /// palette lands. `file://` URLs are read straight from disk without `fetch`.
    pub fn new(fetch: Fetch, spawn: Spawn, wake: rune::Waker) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        let queue = Arc::new(Mutex::new(queue));
        for index in 0..DECODERS {
            let queue = queue.clone();
            let started = std::thread::Builder::new()
                .name(format!("artwork-{index}"))
                .spawn(move || {
                    loop {
                        let next = queue.lock().ok().and_then(|queue| queue.recv().ok());
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

        Self(Arc::new(Shared {
            inner: Mutex::new(Inner::default()),
            fetch,
            spawn,
            wake,
            jobs: Mutex::new(jobs),
            scale: AtomicU32::new(2f32.to_bits()),
            news: AtomicBool::new(false),
        }))
    }

    /// Tells the service the device pixels per point of the window it draws in, for elements that
    /// name no scale. Call it as the view starts (`cx.env().scale`); the largest window wins.
    pub fn set_scale(&self, scale: f32) {
        if scale.is_finite() && scale > 0. {
            self.0.scale.store(scale.to_bits(), Ordering::Relaxed);
        }
    }

    /// The scale `set_scale` last gave.
    pub fn scale(&self) -> f32 {
        f32::from_bits(self.0.scale.load(Ordering::Relaxed))
    }

    /// Bounds the cache to `bytes` of decoded pixels and `items` covers, in place of the defaults.
    pub fn set_budget(&self, bytes: usize, items: usize) {
        let mut inner = self.0.lock();
        inner.budget = Some((bytes, items.max(1)));
        inner.trim();
    }

    /// The cover at `url` drawn `size_pt` square at `scale`, cropped to its centre square. The
    /// first ask starts its fetch; until it lands the pixels are empty and `shown` is 0.
    pub fn art(&self, url: &str, size_pt: f32, scale: f32) -> Art {
        self.load(url, edge(size_pt, scale), false)
    }

    /// The cover at `url` drawn `width_pt` wide at its own aspect, for banners.
    pub fn wide(&self, url: &str, width_pt: f32, scale: f32) -> Art {
        self.load(url, edge(width_pt, scale) | WIDE, false)
    }

    /// Starts loading the covers at `urls` without drawing them, so they are ready when they
    /// scroll in. Prefetches take at most half the fetches, so a cover on screen never waits on
    /// one that is not.
    pub fn prefetch<'a>(&self, urls: impl IntoIterator<Item = &'a str>, size_pt: f32, scale: f32) {
        let edge = edge(size_pt, scale);
        for url in urls {
            self.load(url, edge, true);
        }
    }

    /// The palette of a cover already decoded, or none while none has landed. Nothing is fetched,
    /// so the colours land on the frame the cover appears.
    pub fn cover_palette(&self, url: &str) -> Option<CoverPalette> {
        self.0.lock().tints.get(url).copied()
    }

    /// The palette of the cover at `url`, sampling it from a small decode when no cover of it has
    /// landed yet. Until then it answers none; when the sample lands the app's waker runs and
    /// [`Artwork::take_news`] reports it.
    pub fn palette(&self, url: &str) -> Option<CoverPalette> {
        let mut inner = self.0.lock();
        if let Some(palette) = inner.tints.get(url) {
            return Some(*palette);
        }
        if !inner.asked.insert(url.to_owned()) {
            return None;
        }
        drop(inner);
        let shared = self.0.clone();
        let url = url.to_owned();
        let read = self.read(&url);
        (self.0.spawn)(Box::pin(async move {
            let bytes = match read.await {
                Ok(bytes) => bytes,
                Err(error) => {
                    log::debug!("artwork: cannot sample {url}: {error:#}");
                    return;
                }
            };
            let job = {
                let shared = shared.clone();
                Box::new(move || match decode(&bytes, PALETTE_EDGE) {
                    Ok(decoded) => shared.tint(&url, decoded.palette),
                    Err(error) => log::debug!("artwork: cannot decode {url}: {error}"),
                })
            };
            shared.decode(job);
        }));
        None
    }

    /// Whether a palette landed since the last call. The app's `poll` returns it on screens whose
    /// view reads palettes, so the view runs again once to take the colours.
    pub fn take_news(&self) -> bool {
        self.0.news.swap(false, Ordering::AcqRel)
    }

    /// How many covers the cache holds and their decoded bytes.
    pub fn usage(&self) -> (usize, usize) {
        let inner = self.0.lock();
        (inner.entries.len(), inner.bytes)
    }

    fn load(&self, url: &str, edge: u32, prefetch: bool) -> Art {
        let mut inner = self.0.lock();
        let now = Instant::now();
        let key = (url.to_owned(), edge);
        if let Some(entry) = inner.entries.get_mut(&key) {
            entry.used = now;
            let art = entry.art.clone();
            if matches!(entry.load, Load::Failed(at) if at.elapsed() >= RETRY) {
                entry.load = Load::Queued;
                entry.art.failed.set(0.);
                inner.enqueue(key, prefetch);
                drop(inner);
                self.pump();
            }
            return art;
        }
        if let Some(art) = inner.larger(url, edge) {
            return art;
        }
        let entry = Entry {
            art: Art {
                pixels: Pixels::new(),
                shown: Live::new(0.),
                failed: Live::new(0.),
            },
            load: Load::Queued,
            used: now,
        };
        let fresh = entry.art.clone();
        inner.entries.insert(key.clone(), entry);
        inner.enqueue(key, prefetch);
        // A smaller decode of the same cover stands in while this one loads, so a cover that
        // grows past an edge step never flashes its placeholder.
        let shown = inner.smaller(url, edge).unwrap_or(fresh);
        inner.trim();
        drop(inner);
        self.pump();
        shown
    }

    /// The bytes behind `url`: read from disk for a `file://` URL, else through the app's fetch.
    fn read(&self, url: &str) -> BoxFuture<'static, anyhow::Result<Vec<u8>>> {
        match url.strip_prefix(FILE_PREFIX) {
            Some(path) => {
                let path = path.to_owned();
                Box::pin(async move { Ok(std::fs::read(path)?) })
            }
            None => (self.0.fetch)(url.to_owned()),
        }
    }

    /// Starts fetches for the newest waiting asks while slots are free.
    fn pump(&self) {
        loop {
            let mut inner = self.0.lock();
            let Some((key, prefetch)) = inner.next() else {
                return;
            };
            inner.fetching += 1;
            if prefetch {
                inner.prefetching += 1;
            }
            if let Some(entry) = inner.entries.get_mut(&key) {
                entry.load = Load::Fetching;
            }
            drop(inner);

            let read = self.read(&key.0);
            let shared = self.0.clone();
            (self.0.spawn)(Box::pin(async move {
                let bytes = read.await;
                let artwork = Artwork(shared);
                artwork.fetched(key, prefetch, bytes);
            }));
        }
    }

    /// Takes a finished fetch: frees its slot, hands the bytes to a decoder and starts the next.
    fn fetched(&self, key: Key, prefetch: bool, bytes: anyhow::Result<Vec<u8>>) {
        {
            let mut inner = self.0.lock();
            inner.fetching = inner.fetching.saturating_sub(1);
            if prefetch {
                inner.prefetching = inner.prefetching.saturating_sub(1);
            }
            match &bytes {
                Ok(_) => {
                    if let Some(entry) = inner.entries.get_mut(&key) {
                        entry.load = Load::Decoding;
                    }
                }
                Err(error) => {
                    log::debug!("artwork: cannot fetch {}: {error:#}", key.0);
                    inner.fail(&key);
                }
            }
        }
        if let Ok(bytes) = bytes {
            let shared = self.0.clone();
            self.0.decode(Box::new(move || {
                let decoded = decode(&bytes, key.1);
                shared.land(key, decoded);
            }));
        }
        self.pump();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `job` on a decoder thread, or in place when none could start.
    fn decode(&self, job: Job) {
        let sent = self
            .jobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send(job);
        if let Err(mpsc::SendError(job)) = sent {
            job();
        }
    }

    /// Puts a decoded cover in its pixels and fades it in, or marks it failed.
    fn land(&self, key: Key, decoded: Result<Decoded, String>) {
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) => {
                log::debug!("artwork: cannot decode {}: {error}", key.0);
                self.lock().fail(&key);
                return;
            }
        };
        self.tint(&key.0, decoded.palette);
        let mut inner = self.lock();
        let size = decoded.rgba.len();
        let Some(entry) = inner.entries.get_mut(&key) else {
            return;
        };
        entry
            .art
            .pixels
            .set(decoded.width, decoded.height, decoded.rgba);
        entry.art.shown.set(1.);
        entry.load = Load::Ready(size);
        inner.bytes = inner.bytes.saturating_add(size);
        inner.trim();
    }

    /// Keeps the palette of a landed cover, waking the app when it was waited for.
    fn tint(&self, url: &str, palette: CoverPalette) {
        let mut inner = self.lock();
        if inner.tints.contains_key(url) {
            return;
        }
        if inner.tints.len() >= TINT_ITEMS {
            let live: HashSet<String> = inner.entries.keys().map(|(url, _)| url.clone()).collect();
            inner.tints.retain(|url, _| live.contains(url));
            if inner.tints.len() >= TINT_ITEMS {
                inner.tints.clear();
            }
        }
        inner.tints.insert(url.to_owned(), palette);
        let waited = inner.asked.remove(url);
        drop(inner);
        self.news.store(true, Ordering::Release);
        if waited {
            (self.wake)();
        }
    }
}

impl Inner {
    fn budget(&self) -> (usize, usize) {
        self.budget.unwrap_or((CACHE_BYTES, CACHE_ITEMS))
    }

    /// Queues `key` at the front, dropping the oldest asks past `QUEUE`, which forget their
    /// entries so a later ask starts them afresh.
    fn enqueue(&mut self, key: Key, prefetch: bool) {
        self.queue.retain(|(queued, _)| *queued != key);
        self.queue.push_front((key, prefetch));
        while self.queue.len() > QUEUE {
            let Some((dropped, _)) = self.queue.pop_back() else {
                break;
            };
            if self
                .entries
                .get(&dropped)
                .is_some_and(|entry| entry.load == Load::Queued)
            {
                self.entries.remove(&dropped);
            }
        }
    }

    /// The newest waiting ask a free slot can take: anything while slots are free, a prefetch only
    /// while prefetches hold less than their share.
    fn next(&mut self) -> Option<(Key, bool)> {
        if self.fetching >= MAX_FETCHES {
            return None;
        }
        let prefetch_room = self.prefetching < PREFETCH_FETCHES;
        let at = self
            .queue
            .iter()
            .position(|(_, prefetch)| !prefetch || prefetch_room)?;
        let (key, prefetch) = self.queue.remove(at)?;
        match self.entries.get(&key) {
            Some(entry) if entry.load == Load::Queued => Some((key, prefetch)),
            _ => self.next(),
        }
    }

    fn fail(&mut self, key: &Key) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.load = Load::Failed(Instant::now());
            entry.art.failed.set(1.);
        }
    }

    /// A landed decode of the same cover at `edge` to twice it, which draws as well as one at
    /// `edge` would.
    fn larger(&mut self, url: &str, edge: u32) -> Option<Art> {
        let (_, entry) = self
            .entries
            .iter_mut()
            .filter(|((held, size), entry)| {
                held == url
                    && size & WIDE == edge & WIDE
                    && *size & !WIDE >= edge & !WIDE
                    && *size & !WIDE <= (edge & !WIDE).saturating_mul(2)
                    && matches!(entry.load, Load::Ready(_))
            })
            .min_by_key(|((_, size), _)| *size)?;
        entry.used = Instant::now();
        Some(entry.art.clone())
    }

    /// The largest landed decode of the same cover at no less than half of `edge`, to draw while
    /// the one at `edge` loads.
    fn smaller(&mut self, url: &str, edge: u32) -> Option<Art> {
        let (_, entry) = self
            .entries
            .iter_mut()
            .filter(|((held, size), entry)| {
                held == url
                    && size & WIDE == edge & WIDE
                    && *size & !WIDE >= (edge & !WIDE) / 2
                    && matches!(entry.load, Load::Ready(_))
            })
            .max_by_key(|((_, size), _)| *size)?;
        entry.used = Instant::now();
        Some(entry.art.clone())
    }

    /// Drops the least recently asked covers until the cache is inside its budget. A cover asked
    /// for within `GRACE`, or one still loading, is never taken: it is on screen.
    fn trim(&mut self) {
        let (bytes, items) = self.budget();
        while self.bytes > bytes || self.entries.len() > items {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| {
                    entry.used.elapsed() > GRACE
                        && !matches!(entry.load, Load::Fetching | Load::Decoding)
                })
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                return;
            };
            if let Some(Entry {
                load: Load::Ready(size),
                ..
            }) = self.entries.remove(&oldest)
            {
                self.bytes = self.bytes.saturating_sub(size);
            }
        }
    }
}

impl std::fmt::Debug for Artwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (items, bytes) = self.usage();
        f.debug_struct("Artwork")
            .field("items", &items)
            .field("bytes", &bytes)
            .finish()
    }
}

/// A cover image: the art at a URL over a placeholder box, which keeps the box while it loads,
/// shows a glyph when there is no URL or the load failed, and fades the image in when it lands.
pub struct Cover<'a> {
    artwork: &'a Artwork,
    url: Option<&'a str>,
    size: f32,
    /// The height of a wide image, which is `size` wide. None keeps the square.
    tall: Option<f32>,
    round: bool,
    radius: Option<f32>,
    fallback: &'a str,
    accent: bool,
    overlay: bool,
    scale: Option<f32>,
}

impl<'a> Cover<'a> {
    pub fn new(artwork: &'a Artwork, url: Option<&'a str>) -> Self {
        Self {
            artwork,
            url: url.filter(|url| !url.is_empty()),
            size: SIZE,
            tall: None,
            round: false,
            radius: None,
            fallback: FALLBACK_ICON,
            accent: false,
            overlay: false,
            scale: None,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Draws the image `size` wide and `height` tall at its own aspect, covering the box instead
    /// of cropping it to a square first. Meant for banners.
    pub fn wide(mut self, height: f32) -> Self {
        self.tall = Some(height);
        self
    }

    pub fn circle(mut self) -> Self {
        self.round = true;
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    /// The placeholder glyph, as `"icons/<name>.svg"`.
    pub fn fallback(mut self, icon: &'a str) -> Self {
        self.fallback = icon;
        self
    }

    /// Draws the placeholder glyph in the accent rather than the muted foreground.
    pub fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    /// Draws nothing while the image loads or after it fails, for a cover stacked over another
    /// that already shows.
    pub fn overlay(mut self) -> Self {
        self.overlay = true;
        self
    }

    /// The device pixels per point to decode for, in place of the one `Artwork::set_scale` gave.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = Some(scale);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let Self {
            artwork,
            url,
            size,
            tall,
            round,
            radius,
            fallback,
            accent,
            overlay,
            scale,
        } = self;
        let height = tall.unwrap_or(size);
        let rounded = match (round, radius) {
            (true, _) => size.min(height) / 2.,
            (false, Some(radius)) => radius,
            (false, None) => theme.radius.min(ROUNDED),
        };
        let muted = theme.muted_foreground;
        let glyph = match accent {
            true => theme.tint.unwrap_or(theme.primary),
            false => muted.opacity(BLANK_GLYPH),
        };
        let mark = |opacity: Option<&Live<f32>>| {
            let mark = icon(icons::get(fallback), size.min(height) * GLYPH).color(glyph.color());
            match opacity {
                Some(failed) => mark.live_opacity(failed.smooth(Springs::BASE)),
                None => mark,
            }
        };
        let base = el().size(size, height).radius(rounded).center();
        let Some(url) = url else {
            return match overlay {
                true => base,
                false => base
                    .fill(muted.opacity(BLANK_FILL).color())
                    .child(mark(None)),
            };
        };
        let scale = scale.unwrap_or_else(|| artwork.scale());
        let art = match tall {
            Some(_) => artwork.wide(url, size, scale),
            None => artwork.art(url, size, scale),
        };
        let image = el()
            .pin(Some(0.), Some(0.), Some(0.), Some(0.))
            .radius(rounded)
            .pixels_fit(&art.pixels, SurfaceFit::Fill)
            .live_opacity(art.shown.smooth(Springs::BASE));
        match overlay {
            true => base.child(image),
            false => base
                .fill(muted.opacity(BLANK_FILL).color())
                .child(mark(Some(&art.failed)))
                .child(image),
        }
    }
}

/// A round cover, as people and artists are drawn.
pub struct Avatar<'a> {
    cover: Cover<'a>,
}

impl<'a> Avatar<'a> {
    pub fn new(artwork: &'a Artwork, url: Option<&'a str>) -> Self {
        Self {
            cover: Cover::new(artwork, url).circle().fallback("icons/user.svg"),
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.cover = self.cover.size(size);
        self
    }

    pub fn fallback(mut self, icon: &'a str) -> Self {
        self.cover = self.cover.fallback(icon);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.cover = self.cover.scale(scale);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        self.cover.build(theme)
    }
}

/// The cover at `url`, `size` points square, with the theme's corner.
pub fn cover(artwork: &Artwork, url: Option<&str>, size: f32, theme: &Theme) -> El {
    Cover::new(artwork, url).size(size).build(theme)
}

/// The round cover at `url`, `size` points across.
pub fn avatar(artwork: &Artwork, url: Option<&str>, size: f32, theme: &Theme) -> El {
    Avatar::new(artwork, url).size(size).build(theme)
}

/// The edge a cover drawn at `size_pt` is decoded at: its device size rounded up to the next
/// `EDGE_STEP`, so nearby sizes share one decode, and never past `MAX_EDGE`.
fn edge(size_pt: f32, scale: f32) -> u32 {
    let device = (size_pt.max(1.) * scale.max(1.)).ceil();
    let device = match device.is_finite() {
        true => device as u32,
        false => MAX_EDGE,
    };
    device.max(1).next_multiple_of(EDGE_STEP).min(MAX_EDGE)
}

/// Decodes `bytes` and scales the image to `edge`: its centre square at `edge` across, or with
/// `WIDE` set the whole image at that width. An image already that small keeps its size.
fn decode(bytes: &[u8], edge: u32) -> Result<Decoded, String> {
    let format = image::guess_format(bytes).map_err(|error| error.to_string())?;
    let mut decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|error| error.to_string())?;
    let orientation = decoder.orientation().map_err(|error| error.to_string())?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);
    let image = framed(image, edge).into_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    let palette = CoverPalette::of_rgba(&rgba);
    Ok(Decoded {
        width,
        height,
        rgba,
        palette,
    })
}

/// Crops a decoded cover to its centre square and scales it down to `edge`, in the decoder's own
/// pixel format, so an RGB JPEG only grows a fourth channel at the size it is drawn.
fn framed(image: DynamicImage, edge: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    if edge & WIDE != 0 {
        let wide = edge & !WIDE;
        return match width <= wide {
            true => image,
            false => image.resize(wide, u32::MAX, imageops::FilterType::Triangle),
        };
    }
    let side = width.min(height);
    if side <= edge {
        return match width == height {
            true => image,
            false => image.crop_imm((width - side) / 2, (height - side) / 2, side, side),
        };
    }
    let square = match width == height {
        true => image,
        false => image.crop_imm((width - side) / 2, (height - side) / 2, side, side),
    };
    match side > edge.saturating_mul(2) {
        true => square.thumbnail_exact(edge, edge),
        false => square.resize_exact(edge, edge, imageops::FilterType::Triangle),
    }
}
