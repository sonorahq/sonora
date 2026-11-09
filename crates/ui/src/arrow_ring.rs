//! A down arrow in a ring that fills clockwise from the top as a download goes, and closes into
//! Lucide's `circle-arrow-down` when it is done. With the amount unknown, a short arc turns.

use std::f32::consts::{FRAC_PI_2, TAU};

use rune::{El, LoopSpec, Shape, canvas, el};

use crate::color::Hsla;
use crate::motion::Springs;
use crate::theme::Theme;

/// The mark's size, the same as a button's icon.
const SIZE: f32 = 16.;
/// The grid the mark is drawn on, Lucide's.
const GRID: f32 = 24.;
const STROKE: f32 = 2.;
const RADIUS: f32 = 10.;
/// How many segments a whole ring is drawn with.
const SEGMENTS: f32 = 64.;
/// How long one turn of the arc takes while the amount done is unknown, and how much of the
/// ring it covers.
const TURN: f64 = 0.9;
const SWEEP: f32 = 0.25;
/// How strongly the empty part of the ring shows.
const TRACK: f32 = 0.3;

/// The mark. It is decoration: what it stands for says how far along it is.
pub struct ArrowRing {
    done: Option<f32>,
    color: Option<Hsla>,
    arrow: Option<Hsla>,
    backdrop: Option<Hsla>,
    size: f32,
}

impl ArrowRing {
    /// A ring `done` of the way round, from 0 to 1, or turning when `None`.
    pub fn new(done: Option<f32>) -> Self {
        Self {
            done: done.map(|done| done.clamp(0., 1.)),
            color: None,
            arrow: None,
            backdrop: None,
            size: SIZE,
        }
    }

    /// The filled part of the ring, and the arrow unless `arrow` sets its own. The muted
    /// foreground by default.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn arrow(mut self, color: Hsla) -> Self {
        self.arrow = Some(color);
        self
    }

    /// Fills the circle behind the arrow with `color` and leaves out the ring's empty part. The
    /// filled part fades away once the work is done, leaving the disc alone.
    pub fn backdrop(mut self, color: Hsla) -> Self {
        self.backdrop = Some(color);
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let color = self.color.unwrap_or(theme.muted_foreground);
        let arrow = self.arrow.unwrap_or(color).color();
        let width = STROKE * self.size / GRID;
        let point = |x: f32, y: f32| [x, y];
        let mut base = Shape::new().view_box(GRID, GRID);
        match (self.backdrop, self.done) {
            (Some(backdrop), _) => base = base.fill(circle(12., 12., GRID / 2.), backdrop.color()),
            (None, Some(done)) if done < 1. => {
                base = base.stroke_with(
                    arc(0., 1.),
                    true,
                    width,
                    rune::LineCap::Round,
                    theme.muted_foreground.opacity(TRACK).color(),
                )
            }
            (None, None) => {
                base = base.stroke_with(
                    arc(0., 1.),
                    true,
                    width,
                    rune::LineCap::Round,
                    theme.muted_foreground.opacity(TRACK).color(),
                )
            }
            (None, Some(_)) => {}
        }
        let mark = Shape::new()
            .view_box(GRID, GRID)
            .stroke(vec![point(12., 8.), point(12., 16.)], width, arrow)
            .stroke(
                vec![point(8., 12.), point(12., 16.), point(16., 12.)],
                width,
                arrow,
            );
        let (ring, turning) = match self.done {
            Some(done) => (arc(0., done), false),
            None => (arc(0., SWEEP), true),
        };
        let whole = self.done.is_some_and(|done| done >= 1.);
        let ring = match whole {
            true => Shape::new().view_box(GRID, GRID).stroke_with(
                arc(0., 1.),
                true,
                width,
                rune::LineCap::Round,
                color.color(),
            ),
            false => Shape::new()
                .view_box(GRID, GRID)
                .stroke(ring, width, color.color()),
        };
        let mut ring = canvas(ring)
            .absolute(0., 0.)
            .size(self.size, self.size)
            .opacity(match self.backdrop.is_some() && whole {
                true => 0.,
                false => 1.,
            })
            .motion(Springs::SLOW);
        if turning {
            ring = ring.looping(LoopSpec::rotation(TURN));
        }
        el().size(self.size, self.size)
            .child(canvas(base).absolute(0., 0.).size(self.size, self.size))
            .child(ring)
            .child(canvas(mark).absolute(0., 0.).size(self.size, self.size))
            .a11y_hidden()
    }
}

/// `sweep` of the ring clockwise from `start`, both as fractions of a turn from the top.
fn arc(start: f32, sweep: f32) -> Vec<[f32; 2]> {
    let steps = (SEGMENTS * sweep.clamp(0., 1.)).ceil().max(2.) as usize;
    let whole = sweep >= 1.;
    let points = match whole {
        true => steps,
        false => steps + 1,
    };
    (0..points)
        .map(|step| {
            let angle = (start + sweep * step as f32 / steps as f32) * TAU - FRAC_PI_2;
            [12. + RADIUS * angle.cos(), 12. + RADIUS * angle.sin()]
        })
        .collect()
}

/// A filled disc as a polygon.
fn circle(x: f32, y: f32, radius: f32) -> Vec<[f32; 2]> {
    (0..SEGMENTS as usize)
        .map(|step| {
            let angle = step as f32 / SEGMENTS * TAU;
            [x + radius * angle.cos(), y + radius * angle.sin()]
        })
        .collect()
}
