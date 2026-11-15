use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{App, Div, Pixels, SharedString, SpringConfig, Window, canvas, div, point, px};

use crate::motion::{Sprung, ease_out_cubic, paced, spring};
use crate::veil::effects;

/// How long one row takes to surface out of its blur.
const SURFACE: Duration = Duration::from_millis(220);
/// How long after the row above it each new row starts to surface.
const STAGGER: Duration = Duration::from_millis(18);
/// How far below its place a row starts, and how hard it blurs there.
const RISE: Pixels = px(12.);
const BLUR: Pixels = px(10.);
/// Below this a blur is not worth a layer of its own.
const BLUR_LEAST: Pixels = px(0.05);
/// The box growing and shrinking to fit what it holds.
const GROW: SpringConfig = spring(0.22, 0.04);
/// How close the height has to come to its target to count as landed, in pixels.
const GROW_REST: f32 = 0.5;

/// Rows that surface one after another as a list fills: each new row rises a little out of a
/// blur a beat after the one above it. A row that stays across an update keeps its place in
/// the entrance, so only the rows that are new animate. Keep one per list across frames.
#[derive(Default)]
pub struct Cascade {
    started: RefCell<HashMap<SharedString, Instant>>,
}

impl Cascade {
    /// Forgets every row, so the next list to show plays its whole entrance again.
    pub fn clear(&self) {
        self.started.borrow_mut().clear();
    }

    /// Notes the rows on show now, in reading order, by a key that stays the same for the same
    /// row. Call it on every frame before reading `level`.
    pub fn show(&self, keys: impl IntoIterator<Item = SharedString>) {
        let now = Instant::now();
        let mut started = self.started.borrow_mut();
        let mut kept = HashMap::with_capacity(started.len());
        let mut fresh = 0;
        for key in keys {
            let start = started.get(&key).copied().unwrap_or_else(|| {
                let start = now + paced(STAGGER) * fresh;
                fresh += 1;
                start
            });
            kept.insert(key, start);
        }
        *started = kept;
    }

    /// How far through surfacing the row under `key` is, from 0 still hidden to 1 in place. It
    /// asks for another frame while the row is on its way, and a row never shown counts as in
    /// place.
    pub fn level(&self, key: &str, window: &Window, cx: &App) -> f32 {
        let Some(start) = self.started.borrow().get(key).copied() else {
            return 1.;
        };
        if cx.reduce_motion() {
            return 1.;
        }
        let span = paced(SURFACE).as_secs_f32();
        let progress = match Instant::now().checked_duration_since(start) {
            Some(elapsed) if span > 0. => (elapsed.as_secs_f32() / span).min(1.),
            Some(_) => 1.,
            None => 0.,
        };
        if progress < 1. {
            window.request_animation_frame();
        }
        ease_out_cubic(progress)
    }

    /// Wraps `row` in its entrance at `level`, as `level` answered it. A row in place carries
    /// no layer at all.
    pub fn surface(row: impl IntoElement, level: f32) -> Div {
        let hidden = 1. - level;
        div()
            .flex_none()
            .when(hidden > 0., |this| {
                this.opacity(level)
                    .layer_translate(point(px(0.), RISE * hidden))
            })
            .map(|this| match BLUR * hidden {
                blur if blur > BLUR_LEAST && effects() => this.blur(blur),
                _ => this,
            })
            .child(row)
    }
}

/// A box whose height springs to the height of what it holds, so a list that gains or loses
/// rows stretches to fit instead of jumping. The content is measured as it prepaints, one frame
/// behind. Keep one per box across frames.
pub struct Unfold {
    height: Cell<Sprung>,
    natural: Rc<Cell<Pixels>>,
}

impl Default for Unfold {
    fn default() -> Self {
        Self {
            height: Cell::new(Sprung::at(0.)),
            natural: Rc::default(),
        }
    }
}

impl Unfold {
    /// Collapses the box at once, so the next content it holds grows in from nothing.
    pub fn reset(&self) {
        self.height.set(Sprung::at(0.));
        self.natural.set(Pixels::ZERO);
    }

    /// Where the height stands, as of the last `wrap`.
    pub fn height(&self) -> Pixels {
        px(self.height.get().position().max(0.))
    }

    /// Holds `content` in a box at the springing height, clipping whatever is still past it.
    /// An `empty` box springs shut whatever it measured before. The content keeps its own
    /// height and must not shrink to the box.
    pub fn wrap(&self, content: impl IntoElement, empty: bool, window: &Window, cx: &App) -> Div {
        let target = match empty {
            true => 0.,
            false => self.natural.get() / px(1.),
        };
        let mut height = self.height.get();
        height.follow(target, GROW, GROW_REST, window, cx);
        self.height.set(height);
        let measured = self.natural.clone();

        div()
            .flex_none()
            .w_full()
            .h(self.height())
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w_full()
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                if measured.replace(bounds.size.height) != bounds.size.height {
                                    window.request_animation_frame();
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(content),
            )
    }
}
