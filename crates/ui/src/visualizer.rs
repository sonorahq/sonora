use gpui::prelude::*;
use std::f32::consts::FRAC_PI_2;

use gpui::{
    App, Background, Bounds, Div, Hsla, IntoElement, PathBuilder, Pixels, Point, RenderOnce,
    SharedString, StyleRefinement, Window, canvas, div, linear_color_stop, linear_gradient, point,
    px,
};
use i18n::t;

use crate::snapped;
use crate::theme::ActiveTheme as _;

const GAP: f32 = 3.;
const OPACITY: f32 = 0.32;
const STROKE: f32 = 2.;
const CHANNELS: [f32; 2] = [0.62, 1.];
const GLOW: f32 = 10.;
/// How many of the glow's standard deviations its blur carries past the line. The glow is cut to
/// a corner this much wider than the window's, so the blur runs out before the window's curve.
const GLOW_REACH: f32 = 3.;
const GLOW_STROKE: f32 = 2.4;
const GLOW_OPACITY: f32 = 0.9;
/// A share of the band's own height, not of the box: a fixed share of the box leaves the line
/// floating over stubby bars on a quiet track.
const BEDDED: f32 = 0.55;
const RIDE: f32 = 0.4;
const FILL: f32 = 0.22;
const FILL_BASE: f32 = 0.05;
const LINE_BLUR: f32 = 0.7;
const CONTRAST: f32 = 0.34;
/// How many straight edges stand in for a quarter circle when a shape is cut to a rounded corner.
const ARC_STEPS: usize = 12;
/// How many straight edges stand in for each span of the wave when it is flattened for cutting.
const WAVE_STEPS: usize = 8;

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

    fn bars(self) -> bool {
        matches!(self, Self::Bars | Self::Both)
    }

    fn wave(self) -> bool {
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

#[derive(IntoElement)]
pub struct Visualizer {
    base: Div,
    levels: Levels,
    max: Pixels,
    style: VisualizerStyle,
    tint: Option<Hsla>,
    behind: Option<Hsla>,
    corner: Option<Pixels>,
}

impl Visualizer {
    #[track_caller]
    pub fn new(levels: Levels, max: Pixels) -> Self {
        Self {
            base: div(),
            levels,
            max,
            style: VisualizerStyle::default(),
            tint: None,
            behind: None,
            corner: None,
        }
    }

    /// Rounds the box's bottom corners, for a visualizer that sits flush with the bottom of a
    /// rounded window. GPUI clips only to rectangles, so every bar and the wave are cut to the
    /// curve before they are painted.
    pub fn corner_radius(mut self, radius: Pixels) -> Self {
        self.corner = Some(radius);
        self
    }

    pub fn style_kind(mut self, style: VisualizerStyle) -> Self {
        self.style = style;
        self
    }

    /// Defaults to the theme's primary; a caller that knows what is actually behind the
    /// visualizer — a cover-derived backdrop, say — passes its own.
    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    pub fn behind(mut self, behind: Hsla) -> Self {
        self.behind = Some(behind);
        self
    }
}

impl Styled for Visualizer {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Visualizer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            levels,
            max,
            style,
            tint,
            behind,
            corner,
        } = self;
        let corner = corner.filter(|radius| *radius > Pixels::ZERO);
        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());
        let color = legible(
            tint.unwrap_or(theme.primary),
            behind.unwrap_or(theme.background),
        );

        let ride = match style {
            VisualizerStyle::Both => RIDE,
            _ => 0.,
        };
        let bar_color = match style {
            VisualizerStyle::Both => color.opacity(OPACITY * BEDDED),
            _ => color.opacity(OPACITY),
        };

        let mut visualizer = base
            .h(max)
            .relative()
            .when(style.bars(), |this| {
                this.child(bars(levels.clone(), max, bar_color, ride, corner))
            })
            .when(style.wave(), |this| {
                this.child(glow(levels.clone(), color, corner))
                    .child(line(levels, color, corner))
            });

        visualizer.style().refine(&overrides);
        visualizer
    }
}

/// Paints the bars straight from their levels rather than laying out an element each, so the
/// ones that reach a rounded corner can be cut to it.
fn bars(
    levels: Levels,
    max: Pixels,
    color: Hsla,
    ride: f32,
    corner: Option<Pixels>,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let levels = levels.mixed();
            let count = levels.len() as f32;
            let gap = px(GAP);
            let width = (bounds.size.width - gap * (count - 1.)) / count;
            if levels.is_empty() || width <= Pixels::ZERO {
                return;
            }
            let outline = corner.map(|radius| outline(bounds, radius));
            let floor = bounds.origin.y + bounds.size.height;

            for (index, level) in levels.into_iter().enumerate() {
                let start = bounds.origin.x + (width + gap) * index as f32;
                let left = snapped(start, window);
                let right = snapped(start + width, window);
                let top = snapped(floor - max * level.clamp(0., 1.) / (1. + ride), window);
                if top >= floor {
                    continue;
                }
                let bar = Bounds::from_corners(point(left, top), point(right, floor));
                let cornered = corner.is_some_and(|radius| {
                    left < bounds.origin.x + radius
                        || right > bounds.origin.x + bounds.size.width - radius
                });
                match (&outline, cornered) {
                    (Some(outline), true) => {
                        let shape = vec![
                            point(left, top),
                            point(right, top),
                            point(right, floor),
                            point(left, floor),
                        ];
                        polygon(&cut(shape, outline), color, window);
                    }
                    _ => window.paint_quad(gpui::fill(bar, color)),
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

fn glow(levels: Levels, color: Hsla, corner: Option<Pixels>) -> Div {
    div().absolute().inset_0().blur(px(GLOW)).child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| wave(bounds, &levels, color, true, corner, window),
        )
        .size_full(),
    )
}

fn line(levels: Levels, color: Hsla, corner: Option<Pixels>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| wave(bounds, &levels, color, false, corner, window),
    )
    .absolute()
    .inset_0()
    .blur(px(LINE_BLUR))
}

fn wave(
    bounds: Bounds<Pixels>,
    levels: &Levels,
    color: Hsla,
    blurred: bool,
    corner: Option<Pixels>,
    window: &mut Window,
) {
    let channels = [&levels.left, &levels.right];
    if channels.iter().any(|bands| bands.len() < 2) || bounds.size.width <= px(0.) {
        return;
    }
    let width = match blurred {
        true => STROKE * GLOW_STROKE,
        false => STROKE,
    };
    let inset = px(width / 2.);
    let floor = bounds.origin.y + bounds.size.height;
    let span = (bounds.size.height - inset * 2.).max(px(0.));
    let outline = corner.map(|radius| match blurred {
        true => outline(bounds, radius + px(GLOW * GLOW_REACH)),
        false => outline(bounds, radius),
    });

    for (bands, weight) in channels.into_iter().zip(CHANNELS) {
        // A band owns a column, not a fence post: its point goes at the middle of the bar the
        // same band draws.
        let step = bounds.size.width / bands.len() as f32;
        let crest = |band: f32| floor - inset - span * band.clamp(0., 1.);
        let mut points = Vec::with_capacity(bands.len() + 2);
        points.push(point(bounds.origin.x, crest(bands[0])));
        points.extend(bands.iter().enumerate().map(|(index, band)| {
            point(bounds.origin.x + step * (index as f32 + 0.5), crest(*band))
        }));
        points.push(point(
            bounds.origin.x + bounds.size.width,
            crest(bands[bands.len() - 1]),
        ));

        let weight = match blurred {
            true => weight * GLOW_OPACITY,
            false => weight,
        };
        if !blurred {
            fill(&points, floor, color, weight, outline.as_deref(), window);
        }
        stroke(
            &points,
            px(width),
            color.opacity(OPACITY * 2. * weight),
            outline.as_deref(),
            window,
        );
    }
}

/// The spans of a Catmull-Rom spline through `points`, as the start, the two control points
/// and the end of a cubic each. Unlike a quadratic through the midpoints it passes through
/// every point, so a crest lands on its band rather than shy of it.
fn spans(points: &[Point<Pixels>]) -> impl Iterator<Item = [Point<Pixels>; 4]> + '_ {
    (0..points.len() - 1).map(move |index| {
        let before = points[index.saturating_sub(1)];
        let from = points[index];
        let to = points[index + 1];
        let after = points[(index + 2).min(points.len() - 1)];
        [
            from,
            point(
                from.x + (to.x - before.x) / 6.,
                from.y + (to.y - before.y) / 6.,
            ),
            point(
                to.x - (after.x - from.x) / 6.,
                to.y - (after.y - from.y) / 6.,
            ),
            to,
        ]
    })
}

fn trace(builder: &mut PathBuilder, points: &[Point<Pixels>]) {
    builder.move_to(points[0]);
    for [_, first, second, to] in spans(points) {
        builder.cubic_bezier_to(to, first, second);
    }
}

/// The spline through `points` as a polyline, since a curve can only be cut once it is made of
/// straight edges.
fn flatten(points: &[Point<Pixels>]) -> Vec<Point<Pixels>> {
    let mut flat = vec![points[0]];
    for [from, first, second, to] in spans(points) {
        flat.extend((1..=WAVE_STEPS).map(|step| {
            let t = step as f32 / WAVE_STEPS as f32;
            let u = 1. - t;
            let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
            point(
                from.x * a + first.x * b + second.x * c + to.x * d,
                from.y * a + first.y * b + second.y * c + to.y * d,
            )
        }));
    }
    flat
}

fn stroke(
    points: &[Point<Pixels>],
    width: Pixels,
    color: Hsla,
    outline: Option<&[Point<Pixels>]>,
    window: &mut Window,
) {
    let mut builder = PathBuilder::stroke(width);
    match outline {
        None => trace(&mut builder, points),
        Some(outline) => {
            let runs = cut_line(&flatten(points), outline);
            if runs.is_empty() {
                return;
            }
            for run in runs {
                builder.add_polygon(&run, false);
            }
        }
    }
    match builder.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::warn!("visualizer: cannot build the wave: {error}"),
    }
}

fn fill(
    points: &[Point<Pixels>],
    bottom: Pixels,
    color: Hsla,
    weight: f32,
    outline: Option<&[Point<Pixels>]>,
    window: &mut Window,
) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return;
    };
    let mut builder = PathBuilder::fill();
    match outline {
        None => {
            trace(&mut builder, points);
            builder.line_to(point(last.x, bottom));
            builder.line_to(point(first.x, bottom));
            builder.close();
        }
        Some(outline) => {
            let mut shape = flatten(points);
            shape.extend([point(last.x, bottom), point(first.x, bottom)]);
            let shape = cut(shape, outline);
            if shape.len() < 3 {
                return;
            }
            builder.add_polygon(&shape, true);
        }
    }
    match builder.build() {
        Ok(path) => window.paint_path(
            path,
            linear_gradient(
                180.,
                linear_color_stop(color.opacity(FILL * weight), 0.),
                linear_color_stop(color.opacity(FILL * FILL_BASE * weight), 1.),
            ),
        ),
        Err(error) => log::warn!("visualizer: cannot build the wave body: {error}"),
    }
}

/// `bounds` with its bottom corners rounded by `radius`, as a convex polygon wound clockwise on
/// screen, which is the side `cut` and `cut_line` keep.
fn outline(bounds: Bounds<Pixels>, radius: Pixels) -> Vec<Point<Pixels>> {
    let radius = radius
        .min(bounds.size.width / 2.)
        .min(bounds.size.height / 2.);
    let (left, top) = (bounds.origin.x, bounds.origin.y);
    let (right, bottom) = (left + bounds.size.width, top + bounds.size.height);
    let arc = |center: Point<Pixels>, from: f32| {
        (0..=ARC_STEPS).map(move |step| {
            let angle = from + FRAC_PI_2 * step as f32 / ARC_STEPS as f32;
            point(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
    };

    let mut points = vec![point(left, top), point(right, top)];
    points.extend(arc(point(right - radius, bottom - radius), 0.));
    points.extend(arc(point(left + radius, bottom - radius), FRAC_PI_2));
    points
}

/// Keeps the part of the closed `shape` inside the convex `outline`, by Sutherland-Hodgman.
/// The shape itself may be concave.
fn cut(shape: Vec<Point<Pixels>>, outline: &[Point<Pixels>]) -> Vec<Point<Pixels>> {
    let mut kept = shape;
    for (index, &from) in outline.iter().enumerate() {
        let to = outline[(index + 1) % outline.len()];
        let input = std::mem::take(&mut kept);
        for (at, &current) in input.iter().enumerate() {
            let previous = input[(at + input.len() - 1) % input.len()];
            let (now, before) = (side(from, to, current), side(from, to, previous));
            if (now >= 0.) != (before >= 0.) {
                kept.push(between(previous, current, before / (before - now)));
            }
            if now >= 0. {
                kept.push(current);
            }
        }
        if kept.is_empty() {
            break;
        }
    }
    kept
}

/// Keeps the parts of the open line through `points` inside the convex `outline`, as the runs
/// it breaks into. Each segment is cut by Cyrus-Beck.
fn cut_line(points: &[Point<Pixels>], outline: &[Point<Pixels>]) -> Vec<Vec<Point<Pixels>>> {
    let mut runs: Vec<Vec<Point<Pixels>>> = Vec::new();
    let mut open = false;
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let (mut enter, mut exit) = (0f32, 1f32);
        for (index, &edge) in outline.iter().enumerate() {
            let next = outline[(index + 1) % outline.len()];
            let (near, far) = (side(edge, next, from), side(edge, next, to));
            match (near >= 0., far >= 0.) {
                (false, false) => exit = -1.,
                (false, true) => enter = enter.max(near / (near - far)),
                (true, false) => exit = exit.min(near / (near - far)),
                (true, true) => {}
            }
        }
        if enter > exit {
            open = false;
            continue;
        }
        match runs.last_mut() {
            Some(run) if open && enter <= 0. => run.push(between(from, to, exit)),
            _ => runs.push(vec![between(from, to, enter), between(from, to, exit)]),
        }
        open = exit >= 1.;
    }
    runs
}

/// Which side of the edge from `from` to `to` a point is on. Positive or zero is inside an
/// outline wound clockwise on screen.
fn side(from: Point<Pixels>, to: Point<Pixels>, at: Point<Pixels>) -> f32 {
    (to.x - from.x).as_f32() * (at.y - from.y).as_f32()
        - (to.y - from.y).as_f32() * (at.x - from.x).as_f32()
}

fn between(from: Point<Pixels>, to: Point<Pixels>, t: f32) -> Point<Pixels> {
    point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)
}

fn polygon(points: &[Point<Pixels>], color: impl Into<Background>, window: &mut Window) {
    if points.len() < 3 {
        return;
    }
    let mut builder = PathBuilder::fill();
    builder.add_polygon(points, true);
    match builder.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::warn!("visualizer: cannot build a bar: {error}"),
    }
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
