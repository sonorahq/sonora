use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ElementId, Hsla, PathBuilder, Pixels, Point, Window, canvas,
    ease_in_out, point, px,
};

use crate::motion;
use crate::theme::ActiveTheme as _;

/// The mark's size, the same as a button's icon.
const SIZE: Pixels = px(16.);
/// The grid the mark is drawn on, Lucide's `circle-arrow-down`.
const GRID: f32 = 24.;
const STROKE: f32 = 2.;
const RADIUS: f32 = 10.;
/// How many segments a whole ring is drawn with.
const SEGMENTS: f32 = 64.;
/// How long one turn of the ring takes while the amount done is unknown.
const TURN: Duration = Duration::from_millis(900);
/// How much of the ring turns while the amount done is unknown.
const SWEEP: f32 = 0.25;
/// How strongly the empty part of the ring shows.
const TRACK: f32 = 0.3;
/// How quickly the drawn ring closes on the amount done, as the time it takes to cover most of
/// the way. It is about one redraw of a running download, so steps between redraws blend into
/// one movement.
const CATCH_UP: f32 = 0.22;
/// How near the amount done the drawn ring has to be to stop asking for frames.
const SETTLED: f32 = 0.0005;
/// How long a closed ring takes to fade out over a backdrop, before the chosen pace.
const FADE: Duration = Duration::from_millis(300);
/// How far the turning arc travels at least to reach the top once the amount done is known, so
/// it slows into place rather than stopping short.
const ROOM: f32 = 0.25;
/// How much longer the turning arc takes to reach the top than it would at full speed, which
/// lets it ease to a stop without a jolt.
const EASE_IN: f32 = 1.5;

/// A down arrow in a ring that fills clockwise from the top as work goes, and closes into
/// Lucide's `circle-arrow-down` when it is done. It is drawn rather than loaded, so it looks the
/// same whatever icon pack is active. With the amount unknown, a short arc turns instead, and
/// holds still when the system asks for less motion.
#[derive(IntoElement)]
pub struct ArrowRing {
    id: ElementId,
    done: Option<f32>,
    color: Option<Hsla>,
    arrow: Option<Hsla>,
    backdrop: Option<Hsla>,
}

impl ArrowRing {
    /// A ring `done` of the way round, from 0 to 1, or turning when `None`.
    pub fn new(id: impl Into<ElementId>, done: Option<f32>) -> Self {
        Self {
            id: id.into(),
            done,
            color: None,
            arrow: None,
            backdrop: None,
        }
    }

    /// The colour of the filled part of the ring, and of the arrow unless `arrow` sets its own.
    /// The muted foreground by default.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// The colour of the arrow alone.
    pub fn arrow(mut self, color: Hsla) -> Self {
        self.arrow = Some(color);
        self
    }

    /// Fills the whole circle behind the arrow with `color` and leaves out the empty part of the
    /// ring. The filled part then fades out once the work is done, leaving the disc alone.
    pub fn backdrop(mut self, color: Hsla) -> Self {
        self.backdrop = Some(color);
        self
    }
}

impl RenderOnce for ArrowRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let color = self.color.unwrap_or(theme.muted_foreground);
        let look = Look {
            ring: color,
            arrow: self.arrow.unwrap_or(color),
            track: theme.muted_foreground.opacity(TRACK),
            backdrop: self.backdrop,
        };
        let animates = motion::animates(cx);
        let sweep = window.use_keyed_state((self.id, "sweep"), cx, |_, _| Sweep::new(self.done));
        let frame = sweep.update(cx, |sweep, _| sweep.step(self.done, animates));
        let fades = look.backdrop.is_some() && frame.closed;
        if frame.moving || (fades && frame.shown > 0.) {
            window.request_animation_frame();
        }
        let shown = match fades {
            true => ease_in_out(frame.shown),
            false => 1.,
        };
        mark(frame.arc, look.fade(shown))
    }
}

/// The colours one frame of the mark is painted in.
#[derive(Clone, Copy)]
struct Look {
    ring: Hsla,
    arrow: Hsla,
    track: Hsla,
    backdrop: Option<Hsla>,
}

impl Look {
    /// The same look with the filled part of the ring at `shown` of its opacity.
    fn fade(self, shown: f32) -> Self {
        Self {
            ring: self.ring.opacity(shown),
            ..self
        }
    }
}

/// Where the arc is from one frame to the next. While the amount done is unknown a short arc
/// turns. Once it is known the arc eases round to the top as it grows or shrinks to that amount,
/// so the turning hands over to the fill without a jump. A rise in the amount glides, so the
/// steps a download reports in turn into one smooth fill. A fall jumps, since it means another
/// download or a reset rather than progress.
struct Sweep {
    /// Where the turning arc starts, in turns from the top.
    spin: f32,
    /// How much of the ring the fill covers.
    drawn: f32,
    /// Whether the amount done was known on the last frame.
    known: bool,
    morph: Option<Morph>,
    /// How much of a closed ring still shows, from 1 down to 0.
    shown: f32,
    at: Instant,
}

/// One frame of the mark.
struct Frame {
    arc: Arc,
    shown: f32,
    /// Whether the ring is whole and settled at the top.
    closed: bool,
    /// Whether the arc still moves, so another frame is needed.
    moving: bool,
}

impl Sweep {
    /// A ring already closed when first drawn starts faded, so only work finishing in view plays
    /// the fade.
    fn new(done: Option<f32>) -> Self {
        Self {
            spin: 0.,
            drawn: done.unwrap_or(0.),
            known: done.is_some(),
            morph: None,
            shown: match done.is_some_and(|done| done >= 1.) {
                true => 0.,
                false => 1.,
            },
            at: Instant::now(),
        }
    }

    /// Moves the arc on by the time since the last frame. Everything lands at once when nothing
    /// may animate, and the turning arc holds still at the top.
    fn step(&mut self, done: Option<f32>, animates: bool) -> Frame {
        let now = Instant::now();
        let elapsed = now.duration_since(self.at).as_secs_f32();
        self.at = now;
        let Some(done) = done else {
            self.known = false;
            self.morph = None;
            if animates {
                self.spin = (self.spin + elapsed / TURN.as_secs_f32()).fract();
            }
            return Frame {
                arc: Arc {
                    start: self.spin,
                    sweep: SWEEP,
                },
                shown: 1.,
                closed: false,
                moving: animates,
            };
        };
        if !self.known {
            self.known = true;
            self.drawn = done;
            self.morph = animates.then(|| Morph::new(self.spin, now));
        }
        self.drawn = match !animates || done < self.drawn {
            true => done,
            false => {
                let drawn = self.drawn + (done - self.drawn) * (1. - (-elapsed / CATCH_UP).exp());
                match done - drawn < SETTLED {
                    true => done,
                    false => drawn,
                }
            }
        };
        let arc = match self.morph.as_ref().map(|morph| morph.at(now)) {
            Some((start, blend)) if blend < 1. => Arc {
                start,
                sweep: SWEEP + (self.drawn - SWEEP) * blend,
            },
            _ => {
                self.morph = None;
                Arc {
                    start: 0.,
                    sweep: self.drawn,
                }
            }
        };
        let closed = self.morph.is_none() && self.drawn >= 1.;
        self.shown = match (closed, animates) {
            (false, _) => 1.,
            (true, false) => 0.,
            (true, true) => (self.shown - elapsed / motion::paced(FADE).as_secs_f32()).max(0.),
        };
        Frame {
            arc,
            shown: self.shown,
            closed,
            moving: self.morph.is_some() || self.drawn != done,
        }
    }
}

/// The turning arc's way round to the top once the amount done is known. It leaves at the
/// turning speed and slows to a stop on a cubic curve, so its speed never jumps.
struct Morph {
    from: f32,
    to: f32,
    span: f32,
    began: Instant,
}

impl Morph {
    fn new(from: f32, began: Instant) -> Self {
        let to = (from + ROOM).ceil();
        Self {
            from,
            to,
            span: EASE_IN * (to - from) * TURN.as_secs_f32(),
            began,
        }
    }

    /// Where the arc starts at `now`, and how far the morph has got on an eased scale from 0
    /// to 1, for blending the arc's length.
    fn at(&self, now: Instant) -> (f32, f32) {
        let done = (now.duration_since(self.began).as_secs_f32() / self.span).min(1.);
        let blend = done * done * (3. - 2. * done);
        let lead = (done * done * done - 2. * done * done + done) * self.span / TURN.as_secs_f32();
        let start = self.from * (1. - blend) + self.to * blend + lead;
        (start.rem_euclid(1.), blend)
    }
}

/// The drawn part of the ring.
#[derive(Clone, Copy)]
struct Arc {
    /// Where it starts, in turns clockwise from the top.
    start: f32,
    /// How far round it reaches from there.
    sweep: f32,
}

fn mark(arc: Arc, look: Look) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint(bounds, arc, look, window),
    )
    .size(SIZE)
    .flex_none()
    .into_any_element()
}

/// Draws the mark into `bounds`. Over a backdrop the empty part of the ring is left out.
fn paint(bounds: Bounds<Pixels>, arc: Arc, look: Look, window: &mut Window) {
    let scale = bounds.size.width / px(GRID);
    let width = px(STROKE * scale);
    let at = |x: f32, y: f32| {
        point(
            bounds.origin.x + px(x * scale),
            bounds.origin.y + px(y * scale),
        )
    };
    let (start, sweep) = (arc.start, arc.sweep.clamp(0., 1.));
    match look.backdrop {
        Some(backdrop) => dot(at(12., 12.), px(GRID / 2. * scale), backdrop, window),
        None if sweep < 1. => stroke_arc(&at, 0., 1., width, look.track, window),
        None => {}
    }
    if sweep > 0. && look.ring.a > 0. {
        stroke_arc(&at, start, sweep, width, look.ring, window);
    }
    stroke(&[at(12., 8.), at(12., 16.)], width, look.arrow, window);
    stroke(
        &[at(8., 12.), at(12., 16.), at(16., 12.)],
        width,
        look.arrow,
        window,
    );
}

/// Strokes `sweep` of the ring clockwise from `start`, both as fractions of a turn from the top.
fn stroke_arc(
    at: &impl Fn(f32, f32) -> Point<Pixels>,
    start: f32,
    sweep: f32,
    width: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let whole = sweep >= 1.;
    let steps = (SEGMENTS * sweep).ceil().max(2.) as usize;
    let points: Vec<Point<Pixels>> = (0..=steps)
        .map(|step| {
            let angle = (start + sweep * step as f32 / steps as f32) * TAU - FRAC_PI_2;
            at(12. + RADIUS * angle.cos(), 12. + RADIUS * angle.sin())
        })
        .collect();
    match whole {
        true => {
            let mut path = PathBuilder::stroke(width);
            path.add_polygon(&points[..steps], true);
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        false => stroke(&points, width, color, window),
    }
}

/// Strokes an open line through `points` with round ends and joins, which gpui's stroke lacks,
/// so a dot is laid over each. An arc's joints are too shallow to need one, so a long line gets
/// dots at its ends alone.
fn stroke(points: &[Point<Pixels>], width: Pixels, color: Hsla, window: &mut Window) {
    let mut path = PathBuilder::stroke(width);
    path.add_polygon(points, false);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
    let dotted: Vec<&Point<Pixels>> = match points.len() > 3 {
        true => points.first().into_iter().chain(points.last()).collect(),
        false => points.iter().collect(),
    };
    for point in dotted {
        dot(*point, width / 2., color, window);
    }
}

fn dot(center: Point<Pixels>, radius: Pixels, color: Hsla, window: &mut Window) {
    let points: Vec<Point<Pixels>> = (0..SEGMENTS as usize)
        .map(|step| {
            let angle = step as f32 / SEGMENTS * TAU;
            point(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect();
    let mut path = PathBuilder::fill();
    path.add_polygon(&points, true);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
