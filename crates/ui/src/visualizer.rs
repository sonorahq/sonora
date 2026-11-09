//! The spectrum drawn behind the fullscreen artwork: its style setting, a frame of levels, and the
//! live drawing a producer feeds every frame without the view running.

use std::sync::{Arc, Mutex, MutexGuard};

use i18n::{SharedString, t};
use rune::{El, Live, Pixels, el, row};

use crate::color::{Hsla, Rgba};
use crate::theme::Theme;

const GAP: f32 = 3.;
const OPACITY: f32 = 0.32;
const STROKE: f32 = 2.;
/// How strongly the left and the right channel's wave show.
const CHANNELS: [f32; 2] = [0.62, 1.];
/// How far the wave's glow blurs, as a Gaussian sigma in points.
const GLOW: f32 = 10.;
const GLOW_STROKE: f32 = 2.4;
const GLOW_OPACITY: f32 = 0.9;
/// The glow is drawn at this share of the wave's resolution: the blur hides the rest.
const GLOW_RASTER: f32 = 0.5;
/// A slight softening of the wave line, as a Gaussian sigma in points.
const LINE_BLUR: f32 = 0.7;
/// A share of the band's own height, not of the box: a fixed share of the box leaves the line
/// floating over stubby bars on a quiet track.
const BEDDED: f32 = 0.55;
const RIDE: f32 = 0.4;
const FILL: f32 = 0.22;
const FILL_BASE: f32 = 0.05;
const CONTRAST: f32 = 0.34;
/// How many straight edges stand in for each span of the wave when it is flattened.
const WAVE_STEPS: usize = 8;
/// The most pixels a wave raster holds along either side, so a huge window cannot ask for an
/// enormous upload every frame.
const MAX_RASTER: f32 = 2048.;

/// How the spectrum is drawn behind the fullscreen artwork.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisualizerStyle {
    None,
    Bars,
    Wave,
    #[default]
    Both,
}

impl VisualizerStyle {
    pub const ALL: [Self; 4] = [Self::None, Self::Bars, Self::Wave, Self::Both];

    pub fn shown(self) -> bool {
        self != Self::None
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bars => "bars",
            Self::Wave => "wave",
            Self::Both => "both",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "none" => Self::None,
            "bars" => Self::Bars,
            "wave" => Self::Wave,
            _ => Self::default(),
        }
    }

    /// The localized name. Resolved at render, never stored.
    pub fn label(self) -> SharedString {
        match self {
            Self::None => t!("settings-visualizer-style-none"),
            Self::Bars => t!("settings-visualizer-style-bars"),
            Self::Wave => t!("settings-visualizer-style-wave"),
            Self::Both => t!("settings-visualizer-style-both"),
        }
    }

    pub fn bars(self) -> bool {
        matches!(self, Self::Bars | Self::Both)
    }

    pub fn wave(self) -> bool {
        matches!(self, Self::Wave | Self::Both)
    }
}

/// A frame of the spectrum as the visualizer draws it: one band set per channel.
#[derive(Clone, Debug, Default)]
pub struct Levels {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Levels {
    /// Both channels folded together, the louder of the two per band. What the bars draw.
    pub fn mixed(&self) -> Vec<f32> {
        let mut bands = self.left.clone();
        bands.resize(bands.len().max(self.right.len()), 0.);
        for (band, right) in bands.iter_mut().zip(&self.right) {
            *band = band.max(*right);
        }
        bands
    }
}

/// The live spectrum drawing. Clone it freely: the producer that analyses the audio keeps one and
/// calls [`Visualizer::push`] every frame from any thread, and the view builds the element with
/// [`Visualizer::view`]. A push moves the bars and replaces the wave's pixels, waking only the
/// windows that show them; the view never runs for it.
#[derive(Clone)]
pub struct Visualizer(Arc<Feed>);

struct Feed {
    /// Each bar's offset down from the top of the box, so only its level shows above the floor.
    bars: Vec<Live<[f32; 2]>>,
    wave: Pixels,
    glow: Pixels,
    frame: Mutex<Frame>,
}

/// What the last build asked for, and the last levels pushed, so a build that changes the size or
/// the colour redraws them at once.
#[derive(Clone, Default)]
struct Frame {
    width: f32,
    max: f32,
    style: VisualizerStyle,
    color: Hsla,
    levels: Levels,
}

impl Visualizer {
    /// A drawing of `bands` bars, the band count the analyser produces.
    pub fn new(bands: usize) -> Self {
        Self(Arc::new(Feed {
            bars: (0..bands).map(|_| Live::new([0., MAX_RASTER])).collect(),
            wave: Pixels::new(),
            glow: Pixels::new(),
            frame: Mutex::new(Frame::default()),
        }))
    }

    /// Draws a new frame of levels, each band in `0..=1`. Bands past the drawing's count are
    /// ignored and missing ones stand at zero.
    pub fn push(&self, levels: &Levels) {
        let mut frame = self.0.lock();
        frame.levels = levels.clone();
        let frame = frame.clone();
        self.0.draw(&frame);
    }

    /// The element: bars and a glowing wave `max` points tall at most across `width` points,
    /// drawn from the frames pushed. Build it again when the size, style or colours change.
    pub fn view(&self, width: f32, max: f32) -> VisualizerView {
        VisualizerView {
            feed: self.0.clone(),
            width,
            max,
            style: VisualizerStyle::default(),
            tint: None,
            behind: None,
            corner: None,
        }
    }
}

impl Feed {
    fn lock(&self) -> MutexGuard<'_, Frame> {
        self.frame
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Sets every bar's offset and rasterises the wave and its glow from `frame`'s levels.
    fn draw(&self, frame: &Frame) {
        let ride = match frame.style {
            VisualizerStyle::Both => RIDE,
            _ => 0.,
        };
        let mixed = frame.levels.mixed();
        for (index, bar) in self.bars.iter().enumerate() {
            let level = mixed.get(index).copied().unwrap_or(0.).clamp(0., 1.);
            bar.set([0., frame.max * (1. - level / (1. + ride))]);
        }
        if !frame.style.wave() || frame.width <= 0. || frame.max <= 0. {
            return;
        }
        let ink = Rgba::from(frame.color);
        let size = |share: f32| {
            let width = (frame.width * share).min(MAX_RASTER).round().max(1.) as u32;
            let height = (frame.max * share).min(MAX_RASTER).round().max(1.) as u32;
            (width, height)
        };
        let (width, height) = size(1.);
        let mut wave = Canvas::new(width, height);
        let (glow_width, glow_height) = size(GLOW_RASTER);
        let mut glow = Canvas::new(glow_width, glow_height);
        for (bands, weight) in [&frame.levels.left, &frame.levels.right]
            .into_iter()
            .zip(CHANNELS)
        {
            if bands.len() < 2 {
                continue;
            }
            let crest = wave.crests(bands, STROKE / 2.);
            wave.fill_under(&crest, ink, FILL * weight, FILL * FILL_BASE * weight);
            wave.stroke(&crest, STROKE, ink, OPACITY * 2. * weight);
            let stroke = STROKE * GLOW_STROKE * GLOW_RASTER;
            let crest = glow.crests(bands, stroke / 2.);
            glow.stroke(&crest, stroke, ink, OPACITY * 2. * weight * GLOW_OPACITY);
        }
        self.wave.set(width, height, wave.into_rgba());
        self.glow.set(glow_width, glow_height, glow.into_rgba());
    }
}

/// The visualizer's element, configured for one build.
pub struct VisualizerView {
    feed: Arc<Feed>,
    width: f32,
    max: f32,
    style: VisualizerStyle,
    tint: Option<Hsla>,
    behind: Option<Hsla>,
    corner: Option<f32>,
}

impl VisualizerView {
    pub fn style(mut self, style: VisualizerStyle) -> Self {
        self.style = style;
        self
    }

    /// Defaults to the theme's primary; a caller that knows what is behind the visualizer, a
    /// cover-derived backdrop say, passes its own.
    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    pub fn behind(mut self, behind: Hsla) -> Self {
        self.behind = Some(behind);
        self
    }

    /// Rounds the box's bottom corners, for a visualizer flush with the bottom of a rounded
    /// window. The bars and the wave are clipped to them.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner = Some(radius.max(0.));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let color = legible(
            self.tint.unwrap_or(theme.primary),
            self.behind.unwrap_or(theme.background),
        );
        let changed = {
            let mut frame = self.feed.lock();
            let changed = frame.width != self.width
                || frame.max != self.max
                || frame.style != self.style
                || frame.color != color;
            frame.width = self.width;
            frame.max = self.max;
            frame.style = self.style;
            frame.color = color;
            changed.then(|| frame.clone())
        };
        if let Some(frame) = changed {
            self.feed.draw(&frame);
        }
        let corner = self.corner.unwrap_or(0.);
        let mut root = el()
            .w(self.width)
            .h(self.max)
            .clip_corners([0., 0., corner, corner])
            .a11y_hidden();
        if self.style.bars() {
            let fill = match self.style {
                VisualizerStyle::Both => color.opacity(OPACITY * BEDDED),
                _ => color.opacity(OPACITY),
            }
            .color();
            let bars = self
                .feed
                .bars
                .iter()
                .map(|bar| el().grow(1.).w(0.).h(self.max).fill(fill).live_offset(bar));
            root = root.child(
                row()
                    .pin(Some(0.), Some(0.), Some(0.), Some(0.))
                    .gap(GAP)
                    .children(bars),
            );
        }
        if self.style.wave() {
            root = root
                .child(
                    el().pin(Some(0.), Some(0.), Some(0.), Some(0.))
                        .pixels(&self.feed.glow)
                        .blur(GLOW),
                )
                .child(
                    el().pin(Some(0.), Some(0.), Some(0.), Some(0.))
                        .pixels(&self.feed.wave)
                        .blur(LINE_BLUR),
                );
        }
        root
    }
}

/// A premultiplied RGBA raster the wave is drawn into, one pixel per point at its share.
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0.; 4]; (width * height) as usize],
        }
    }

    /// Where the wave through `bands` crosses each pixel column. A band owns a column, not a
    /// fence post: its point sits at the middle of the bar the same band draws, and a
    /// Catmull-Rom spline through the points passes through every crest.
    fn crests(&self, bands: &[f32], inset: f32) -> Vec<f32> {
        let (width, height) = (self.width as f32, self.height as f32);
        let floor = height;
        let span = (height - inset * 2.).max(0.);
        let crest = |band: f32| floor - inset - span * band.clamp(0., 1.);
        let step = width / bands.len() as f32;
        let mut points = Vec::with_capacity(bands.len() + 2);
        points.push([0., crest(bands[0])]);
        points.extend(
            bands
                .iter()
                .enumerate()
                .map(|(index, band)| [step * (index as f32 + 0.5), crest(*band)]),
        );
        points.push([width, crest(bands[bands.len() - 1])]);
        let line = flatten(&points);
        let mut crests = Vec::with_capacity(self.width as usize);
        let mut at = 0;
        for column in 0..self.width {
            let x = column as f32 + 0.5;
            while at + 2 < line.len() && line[at + 1][0] < x {
                at += 1;
            }
            let (a, b) = (line[at], line[(at + 1).min(line.len() - 1)]);
            let t = match b[0] - a[0] > f32::EPSILON {
                true => ((x - a[0]) / (b[0] - a[0])).clamp(0., 1.),
                false => 0.,
            };
            crests.push(a[1] + (b[1] - a[1]) * t);
        }
        crests
    }

    /// Fills from the wave down to the floor, fading from `top` opacity at the wave's highest
    /// crest to `bottom` at the floor.
    fn fill_under(&mut self, crests: &[f32], ink: Rgba, top: f32, bottom: f32) {
        let height = self.height as f32;
        let peak = crests.iter().copied().fold(height, f32::min);
        let reach = (height - peak).max(1.);
        for (column, crest) in crests.iter().enumerate() {
            let first = crest.floor().max(0.) as u32;
            for line in first..self.height {
                let y = line as f32 + 0.5;
                let cover = (y - crest + 0.5).clamp(0., 1.);
                if cover <= 0. {
                    continue;
                }
                let fade = ((y - peak) / reach).clamp(0., 1.);
                let alpha = (top + (bottom - top) * fade) * cover;
                self.blend(column as u32, line, ink, alpha);
            }
        }
    }

    /// Strokes the wave `width` thick, each pixel covered by how far it sits from the wave's
    /// tangent in its column.
    fn stroke(&mut self, crests: &[f32], width: f32, ink: Rgba, opacity: f32) {
        let half = width / 2.;
        for (column, crest) in crests.iter().enumerate() {
            let before = crests[column.saturating_sub(1)];
            let after = crests[(column + 1).min(crests.len() - 1)];
            let slope = (after - before) / 2.;
            let stretch = (1. + slope * slope).sqrt();
            let reach = (half + 1.) * stretch;
            let first = (crest - reach).floor().max(0.) as u32;
            let last = ((crest + reach).ceil().max(0.) as u32).min(self.height);
            for line in first..last {
                let y = line as f32 + 0.5;
                let distance = (y - crest).abs() / stretch;
                let cover = (half + 0.5 - distance).clamp(0., 1.);
                if cover > 0. {
                    self.blend(column as u32, line, ink, opacity * cover);
                }
            }
        }
    }

    /// Draws `ink` at `alpha` over the pixel, source over.
    fn blend(&mut self, x: u32, y: u32, ink: Rgba, alpha: f32) {
        let pixel = &mut self.pixels[(y * self.width + x) as usize];
        let alpha = alpha.clamp(0., 1.);
        let keep = 1. - alpha;
        pixel[0] = ink.r * alpha + pixel[0] * keep;
        pixel[1] = ink.g * alpha + pixel[1] * keep;
        pixel[2] = ink.b * alpha + pixel[2] * keep;
        pixel[3] = alpha + pixel[3] * keep;
    }

    /// The raster as straight-alpha sRGB bytes, as `Pixels::set` takes them.
    fn into_rgba(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.pixels.len() * 4);
        for [r, g, b, a] in self.pixels {
            let straight = |channel: f32| match a > 0. {
                true => ((channel / a).clamp(0., 1.) * 255.).round() as u8,
                false => 0,
            };
            bytes.extend([
                straight(r),
                straight(g),
                straight(b),
                (a.clamp(0., 1.) * 255.).round() as u8,
            ]);
        }
        bytes
    }
}

/// The Catmull-Rom spline through `points` as a polyline of `WAVE_STEPS` edges a span.
fn flatten(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut flat = vec![points[0]];
    for index in 0..points.len() - 1 {
        let before = points[index.saturating_sub(1)];
        let from = points[index];
        let to = points[index + 1];
        let after = points[(index + 2).min(points.len() - 1)];
        let first = [
            from[0] + (to[0] - before[0]) / 6.,
            from[1] + (to[1] - before[1]) / 6.,
        ];
        let second = [
            to[0] - (after[0] - from[0]) / 6.,
            to[1] - (after[1] - from[1]) / 6.,
        ];
        flat.extend((1..=WAVE_STEPS).map(|step| {
            let t = step as f32 / WAVE_STEPS as f32;
            let u = 1. - t;
            let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
            [
                from[0] * a + first[0] * b + second[0] * c + to[0] * d,
                from[1] * a + first[1] * b + second[1] * c + to[1] * d,
            ]
        }));
    }
    flat
}

/// Keeps `tint` readable on `behind` by pushing its lightness away from the surface's.
fn legible(tint: Hsla, behind: Hsla) -> Hsla {
    let distance = (tint.l - behind.l).abs();
    if distance >= CONTRAST {
        return tint;
    }
    let lighter = behind.l + CONTRAST;
    let darker = behind.l - CONTRAST;
    let l = match lighter <= 1. {
        true => lighter,
        false => darker.max(0.),
    };
    Hsla { l, ..tint }
}
