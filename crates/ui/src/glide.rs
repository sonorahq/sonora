use std::cell::RefCell;
use std::mem;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, EntityId, Global, ListState, Pixels, Point, ScrollHandle, SpringConfig, SpringState,
    Window, point, px,
};

use crate::snapped;

/// A scroll position that can be animated by the shared glide controller.
pub trait ScrollPosition: Clone + 'static {
    /// Whether `offset` reads back exactly what `set_offset` wrote, so that any other change
    /// was made by someone else, such as a `Hold` keeping content in place as it reflows.
    const EXACT: bool = false;

    fn offset(&self) -> Point<Pixels>;
    fn set_offset(&self, offset: Point<Pixels>);
    fn max_offset(&self) -> Point<Pixels>;
}

impl ScrollPosition for ScrollHandle {
    const EXACT: bool = true;

    fn offset(&self) -> Point<Pixels> {
        ScrollHandle::offset(self)
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        ScrollHandle::set_offset(self, offset);
    }
    fn max_offset(&self) -> Point<Pixels> {
        ScrollHandle::max_offset(self)
    }
}

impl ScrollPosition for ListState {
    fn offset(&self) -> Point<Pixels> {
        self.scroll_px_offset_for_scrollbar()
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset_from_scrollbar(offset);
    }
    fn max_offset(&self) -> Point<Pixels> {
        self.max_offset_for_scrollbar()
    }
}

const EASE: f32 = 0.12;
const HERTZ: f32 = 180.;
const STALL: Duration = Duration::from_millis(64);
const REST: Pixels = px(0.5);
const SPRING_REST: Pixels = px(0.05);
/// How long a requested frame may take before the glide asks for another, in case the window
/// never delivered the first and the glide would otherwise wait on it forever.
const LOST: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Drift {
    shown: Point<Pixels>,
    target: Point<Pixels>,
    velocity: Point<f32>,
    gliding: bool,
    springing: bool,
    /// When the frame the glide waits on was requested, while one is outstanding.
    armed: Option<Instant>,
    /// Counts the frames the glide requested, so a late one that was asked for again runs once.
    ticket: u64,
    eased: Option<f32>,
    beat: Option<Instant>,
    /// The offset the glide last wrote or started from. A step or aim that finds the scroll
    /// somewhere else carries the glide along by as much, so a hold that kept content in place
    /// mid-glide is not undone.
    placed: Option<Point<Pixels>>,
}

#[derive(Clone)]
pub struct Glide {
    drift: Rc<RefCell<Drift>>,
    pace: f32,
    spring: Option<SpringConfig>,
    watched: Option<EntityId>,
}

impl Default for Glide {
    fn default() -> Self {
        Self::paced(EASE)
    }
}

impl Glide {
    pub fn paced(pace: f32) -> Self {
        Self {
            drift: Rc::default(),
            pace,
            spring: None,
            watched: None,
        }
    }

    pub fn set_pace(&mut self, pace: f32) {
        self.pace = pace;
    }

    pub fn set_spring(&mut self, spring: SpringConfig) {
        self.spring = Some(spring);
    }

    pub fn watch(&mut self, view: EntityId) {
        self.watched = Some(view);
    }

    pub fn sync(&self, scroll: &impl ScrollPosition) {
        let mut drift = self.drift.borrow_mut();
        if !drift.gliding {
            drift.shown = scroll.offset();
            drift.velocity = Point::default();
        }
    }

    /// Asks again for a frame a glide under way has waited on for longer than `LOST`. A render
    /// calls it, so a glide whose frame went missing carries on without new input.
    pub fn revive(&self, scroll: &impl ScrollPosition, window: &mut Window) {
        if self.drift.borrow().gliding {
            self.schedule_frame(scroll, window);
        }
    }

    pub fn nudge(&self, scroll: &impl ScrollPosition, window: &mut Window) {
        {
            let mut drift = self.drift.borrow_mut();
            let landed = scroll.offset();
            let step = landed - drift.shown;
            let from = match drift.gliding {
                true => drift.target,
                false => drift.shown,
            };

            drift.target = held(from + step, scroll);
            drift.gliding = true;
            drift.springing = false;
            drift.velocity = Point::default();
            drift.eased = None;
            drift.placed = Some(drift.shown);
            scroll.set_offset(drift.shown);
        }
        self.schedule_frame(scroll, window);
    }

    pub fn aim<S: ScrollPosition>(&self, scroll: &S, to: Point<Pixels>, window: &mut Window) {
        {
            let mut drift = self.drift.borrow_mut();
            match drift.gliding {
                true => drift.carry(scroll),
                false => {
                    drift.shown = scroll.offset();
                    drift.placed = Some(drift.shown);
                }
            }
            drift.target = held(to, scroll);
            drift.gliding = true;
            let springing = self.spring.is_some();
            if drift.springing != springing {
                drift.velocity = Point::default();
            }
            drift.springing = springing;
            drift.eased = Some(self.pace);
        }
        self.schedule_frame(scroll, window);
    }

    pub fn jump(&self, scroll: &impl ScrollPosition, to: Point<Pixels>) {
        let landed = {
            let mut drift = self.drift.borrow_mut();
            drift.target = held(to, scroll);
            drift.shown = drift.target;
            drift.gliding = false;
            drift.springing = false;
            drift.velocity = Point::default();
            drift.beat = None;
            drift.eased = None;
            drift.shown
        };
        scroll.set_offset(landed);
    }

    /// Stops a glide under way where the scroll stands now, for input that places the view
    /// itself. Returns whether there was one to stop.
    pub fn stop(&self, scroll: &impl ScrollPosition) -> bool {
        let mut drift = self.drift.borrow_mut();
        if !drift.gliding {
            return false;
        }
        drift.shown = scroll.offset();
        drift.target = drift.shown;
        drift.velocity = Point::default();
        drift.gliding = false;
        drift.springing = false;
        drift.beat = None;
        drift.eased = None;
        true
    }

    /// Whether a glide is still under way. It stays true until the spring or ease has landed
    /// on its target, the creep through the last fraction of a pixel included.
    pub fn gliding(&self) -> bool {
        self.drift.borrow().gliding
    }

    pub fn goal(&self, scroll: &impl ScrollPosition) -> Point<Pixels> {
        let drift = self.drift.borrow();

        match drift.gliding {
            true => drift.target,
            false => scroll.offset(),
        }
    }

    pub fn presentation(&self, scroll: &impl ScrollPosition) -> Point<Pixels> {
        let drift = self.drift.borrow();

        match drift.gliding && drift.springing {
            true => drift.shown - scroll.offset(),
            false => Point::default(),
        }
    }

    fn schedule_frame(&self, scroll: &impl ScrollPosition, window: &mut Window) {
        let ticket = {
            let mut drift = self.drift.borrow_mut();
            let now = Instant::now();
            if drift
                .armed
                .is_some_and(|asked| now.duration_since(asked) < LOST)
            {
                return;
            }
            drift.armed = Some(now);
            drift.ticket = drift.ticket.wrapping_add(1);
            drift.ticket
        };

        let glide = self.clone();
        let scroll = scroll.clone();
        window.on_next_frame(move |window, cx| glide.step(ticket, &scroll, window, cx));
    }

    fn step<S: ScrollPosition>(&self, ticket: u64, scroll: &S, window: &mut Window, cx: &mut App) {
        let landed = {
            let mut drift = self.drift.borrow_mut();
            if drift.ticket != ticket {
                return;
            }
            drift.armed = None;
            if !drift.gliding {
                return;
            }
            drift.carry(scroll);

            let now = Instant::now();
            let elapsed = drift
                .beat
                .replace(now)
                .map(|beat| now.duration_since(beat).min(STALL))
                .unwrap_or_else(|| Duration::from_secs_f32(1. / HERTZ));
            let target = held(drift.target, scroll);
            let springing = drift.springing;
            let settled = if springing {
                let spring = self.spring.expect("a springing glide has a spring");
                let x = spring.step(
                    SpringState {
                        position: drift.shown.x.as_f32(),
                        velocity: drift.velocity.x,
                    },
                    target.x.as_f32(),
                    elapsed.as_secs_f32(),
                );
                let y = spring.step(
                    SpringState {
                        position: drift.shown.y.as_f32(),
                        velocity: drift.velocity.y,
                    },
                    target.y.as_f32(),
                    elapsed.as_secs_f32(),
                );
                drift.shown = point(px(x.position), px(y.position));
                drift.velocity = point(x.velocity, y.velocity);
                spring.is_settled(x, target.x.as_f32(), SPRING_REST.as_f32())
                    && spring.is_settled(y, target.y.as_f32(), SPRING_REST.as_f32())
            } else {
                let pace = drift.eased.unwrap_or(EASE);
                let ease = 1. - (1. - pace).powf(elapsed.as_secs_f32() * HERTZ);
                let step = target - drift.shown;
                drift.shown += point(step.x * ease, step.y * ease);
                step.x.abs() < REST && step.y.abs() < REST
            };
            match settled {
                true => {
                    drift.shown = target;
                    drift.gliding = false;
                    drift.springing = false;
                    drift.velocity = Point::default();
                    drift.beat = None;
                    drift.eased = None;
                    target
                }
                // layout walks the pixel grid
                false => match springing {
                    true => grid(held(drift.shown, scroll), window),
                    false => held(drift.shown, scroll),
                },
            }
        };

        scroll.set_offset(landed);
        self.drift.borrow_mut().placed = Some(landed);
        match self.watched {
            Some(view) => {
                mark_scrolling(cx);
                cx.notify(view);
            }
            None => window.refresh(),
        }
        self.schedule_frame(scroll, window);
    }
}

impl Drift {
    /// Carries the glide along by as far as something else moved the scroll since the glide last
    /// wrote it, and takes the scroll where it is now as the glide's own.
    fn carry<S: ScrollPosition>(&mut self, scroll: &S) {
        if !S::EXACT {
            return;
        }
        let Some(placed) = self.placed else {
            return;
        };
        let now = scroll.offset();
        let moved = now - placed;
        self.shown += moved;
        self.target += moved;
        self.placed = Some(now);
    }
}

/// Set while the notifies of a scroll are being delivered.
#[derive(Default)]
struct Scrolling(bool);

impl Global for Scrolling {}

/// Tells observers that the notifies about to be delivered are the work of a scroll. The mark
/// lasts until the effects queued so far have been delivered, so a later change of the view is
/// never taken for scrolling.
pub(crate) fn mark_scrolling(cx: &mut App) {
    let marked = &mut cx.default_global::<Scrolling>().0;
    if mem::replace(marked, true) {
        return;
    }
    cx.defer(|cx| cx.defer(|cx| cx.global_mut::<Scrolling>().0 = false));
}

/// Whether the notify an observer is handling was caused by a scroll. An observer of a scrolled
/// view that only cares about the view's own state can leave such a notify alone.
pub fn scrolling(cx: &App) -> bool {
    cx.try_global::<Scrolling>()
        .is_some_and(|scrolling| scrolling.0)
}

fn grid(at: Point<Pixels>, window: &Window) -> Point<Pixels> {
    point(snapped(at.x, window), snapped(at.y, window))
}

fn held(at: Point<Pixels>, scroll: &impl ScrollPosition) -> Point<Pixels> {
    let reach = scroll.max_offset();

    point(
        at.x.clamp(-reach.x.max(Pixels::ZERO), Pixels::ZERO),
        at.y.clamp(-reach.y.max(Pixels::ZERO), Pixels::ZERO),
    )
}
