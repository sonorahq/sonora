use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    Context, DispatchPhase, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, Window, canvas, div, point, px, relative,
};
use router::{Destination, SettingsPage, navigate};
use state::{Outcome, Target, Toasts};
use ui::{ActiveTheme as _, Toast, css_ease, css_ease_out, paced};

/// The toast width in multiples of the font size.
const WIDTH: f32 = 26.;
/// The space between toasts when the stack is spread out, and how far each one behind the front
/// peeks out above it when the stack is piled up.
const GAP: Pixels = px(14.);
/// How much smaller each toast piled behind the front one is drawn.
const SHRINK: f32 = 0.1;
/// How many toasts the pile shows. The rest wait out of sight behind the last one.
const VISIBLE: usize = 3;
/// How long a toast takes to move into a new place in the stack, entering, leaving and springing
/// back from a short swipe included. These timings are Sonner's.
const SETTLE: Duration = Duration::from_millis(400);
/// How long a dismissed toast stays on screen. It is cut off partway through its slide, which is
/// what makes the exit quick.
const UNMOUNT: Duration = Duration::from_millis(200);
/// How long a dismissed toast that was piled behind the front one takes to sink.
const SINK: Duration = Duration::from_millis(500);
/// How long a swiped toast takes to fade out. It is short so the toast is gone after a nudge
/// rather than seen crossing the window.
const THROW: Duration = Duration::from_millis(120);
/// How far past the release point a swiped toast drifts while it fades, as a share of its width.
const DRIFT: f32 = 0.15;
/// How far a toast has to be dragged right to dismiss it.
const FLING: Pixels = px(45.);
/// How fast, in pixels per millisecond, a shorter drag right has to be to dismiss the toast.
const FLICK: f32 = 0.11;

/// The toasts above the player bar, piled with the newest in front. Pointing at the pile
/// spreads it out and stops every toast's clock, and a toast dragged right far or fast enough
/// is dismissed.
pub struct ToastStack {
    toasts: Entity<Toasts>,
    slots: Vec<Slot>,
    measured: Rc<RefCell<HashMap<usize, Pixels>>>,
    hovered: bool,
    swipe: Option<Swipe>,
}

/// One toast on screen, kept after the toast is dismissed for as long as it takes to leave.
struct Slot {
    toast: state::Toast,
    height: Option<Pixels>,
    index: usize,
    /// Whether the toast is fully shown and so takes presses. A toast piled behind the front one
    /// or waiting out of sight does not.
    active: bool,
    entered: bool,
    leaving: Option<Instant>,
    flung: bool,
    rise: Tween,
    scale: Tween,
    opacity: Tween,
    shown: Tween,
    clip: Tween,
    drop: Tween,
    sway: Tween,
}

/// A drag on a toast, measured from where the button went down.
struct Swipe {
    id: usize,
    origin: Point<Pixels>,
    began: Instant,
    vertical: Option<bool>,
    offset: Point<Pixels>,
}

/// A value easing from one number to another, which can be sent somewhere new halfway.
#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    span: Duration,
    ease: fn(f32) -> f32,
}

impl ToastStack {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let toasts = Toasts::entity(cx);
        cx.observe(&toasts, |_, _, cx| cx.notify()).detach();
        Self {
            toasts,
            slots: Vec::new(),
            measured: Rc::new(RefCell::new(HashMap::new())),
            hovered: false,
            swipe: None,
        }
    }

    fn expanded(&self) -> bool {
        self.hovered || self.swipe.is_some()
    }

    fn hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        self.hold(cx);
    }

    /// Stops the clocks while the stack is spread out and starts them again once it piles up.
    fn hold(&mut self, cx: &mut Context<Self>) {
        let expanded = self.expanded();
        self.toasts
            .update(cx, |toasts, cx| toasts.pause(expanded, cx));
        cx.notify();
    }

    fn grab(&mut self, id: usize, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.swipe = Some(Swipe {
            id,
            origin: event.position,
            began: Instant::now(),
            vertical: None,
            offset: Point::default(),
        });
        self.hold(cx);
    }

    /// Follows the pointer. Right is the way out, so a drag right moves the toast one to one while
    /// a drag any other way only stretches it a little. The first real move picks the axis.
    fn drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(swipe) = &mut self.swipe else {
            return;
        };
        let moved = position - swipe.origin;
        if swipe.vertical.is_none() && (moved.x.abs() > px(1.) || moved.y.abs() > px(1.)) {
            swipe.vertical = Some(moved.y.abs() >= moved.x.abs());
        }
        swipe.offset = match swipe.vertical {
            None => Point::default(),
            Some(false) if moved.x > Pixels::ZERO => point(moved.x, Pixels::ZERO),
            Some(false) => point(damped(moved.x), Pixels::ZERO),
            Some(true) => point(Pixels::ZERO, damped(moved.y)),
        };
        cx.notify();
    }

    /// Ends a drag: far or fast enough right dismisses the toast, anything else springs it back.
    fn release(&mut self, cx: &mut Context<Self>) {
        let Some(swipe) = self.swipe.take() else {
            return;
        };
        let now = Instant::now();
        let travel = swipe.offset.x / px(1.);
        let millis = swipe.began.elapsed().as_secs_f32() * 1000.;
        let speed = travel / millis.max(1.);
        let flung = swipe.offset.x >= FLING || (travel > 0. && speed > FLICK);
        let snap = span(SETTLE, cx);

        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.toast.id == swipe.id) {
            slot.drop = Tween::still(swipe.offset.y / px(1.), now);
            slot.sway = Tween::still(travel, now);
            match flung {
                true => slot.flung = true,
                false => {
                    slot.drop.aim(0., snap, now);
                    slot.sway.aim(0., snap, now);
                }
            }
        }
        if flung {
            Toasts::discard(swipe.id, cx);
        }
        self.hold(cx);
    }

    /// Brings the slots in line with the toasts that are up: new ones get a slot, and the
    /// dismissed ones start leaving and are dropped once they are gone.
    fn sync(&mut self, now: Instant, cx: &mut Context<Self>) {
        let shown = self.toasts.read(cx).shown().to_vec();
        let expanded = self.expanded();
        let settle = span(SETTLE, cx);
        let unmount = span(UNMOUNT, cx);
        let sink = span(SINK, cx);
        let throw = span(THROW, cx);
        let below = clearance(cx) / px(1.);
        let width = cx.theme().font_size * WIDTH / px(1.);

        for slot in &mut self.slots {
            if slot.leaving.is_some() || shown.iter().any(|toast| toast.id == slot.toast.id) {
                continue;
            }
            slot.leaving = Some(now);
            let height = slot.height.map_or(0., |height| height / px(1.));
            match (slot.flung, expanded || slot.index == 0) {
                (true, _) => {
                    let away = slot.sway.now(now) + width * DRIFT;
                    slot.sway.aim_eased(away, throw, css_ease_out, now);
                    slot.opacity.aim_eased(0., throw, css_ease_out, now);
                }
                (false, true) => {
                    slot.drop.aim(height + below, settle, now);
                    slot.opacity.aim(0., settle, now);
                }
                (false, false) => {
                    slot.drop.aim(height * 0.4, sink, now);
                    slot.opacity.aim(0., unmount, now);
                }
            }
        }
        self.slots.retain(|slot| {
            slot.leaving
                .is_none_or(|left| now.duration_since(left) < unmount)
        });

        for toast in shown {
            match self.slots.iter_mut().find(|slot| slot.toast.id == toast.id) {
                Some(slot) => slot.toast = toast,
                None => self.slots.push(Slot::new(toast, now)),
            }
        }
        self.slots.sort_by_key(|slot| slot.toast.id);

        let mut measured = self.measured.borrow_mut();
        measured.retain(|id, _| self.slots.iter().any(|slot| slot.toast.id == *id));
        for slot in &mut self.slots {
            slot.height = measured.get(&slot.toast.id).copied();
        }
    }

    /// Sends every measured toast toward its place: spread out one above another when the stack
    /// is expanded, or piled behind the front one, smaller and with its contents hidden.
    fn arrange(&mut self, now: Instant, cx: &mut Context<Self>) -> Pixels {
        let expanded = self.expanded();
        let settle = span(SETTLE, cx);
        let below = clearance(cx) / px(1.);
        let mut order: Vec<usize> = (0..self.slots.len())
            .filter(|at| {
                let slot = &self.slots[*at];
                slot.leaving.is_none() && slot.height.is_some()
            })
            .collect();
        order.reverse();

        let front = order
            .first()
            .and_then(|at| self.slots[*at].height)
            .map_or(0., |height| height / px(1.));
        let gap = GAP / px(1.);
        let mut reach = 0.;
        let mut last = 0.;

        for (index, at) in order.iter().enumerate() {
            let slot = &mut self.slots[*at];
            let Some(height) = slot.height else {
                continue;
            };
            let height = height / px(1.);
            // A toast past the visible ones waits behind the last of them, so when one ahead
            // leaves it fades in where it belongs instead of dropping in from far above.
            let parked = index >= VISIBLE;
            let piled = index.min(VISIBLE - 1) as f32;
            let leading = (expanded || index == 0) && !parked;

            let rise = match (expanded, parked) {
                (true, false) => reach,
                (true, true) => last,
                (false, _) => piled * gap,
            };
            let scale = match expanded {
                true => 1.,
                false => 1. - piled * SHRINK,
            };
            let clip = match leading {
                true => height,
                false => front,
            };
            let shown = match leading {
                true => 1.,
                false => 0.,
            };
            let opacity = match parked {
                true => 0.,
                false => 1.,
            };
            if !parked {
                last = reach;
                reach += height + gap;
            }
            slot.index = index;
            slot.active = leading;

            if !slot.entered {
                slot.entered = true;
                slot.rise = Tween::still(rise - height - below, now);
                slot.rise.aim(rise, settle, now);
                slot.opacity.aim(opacity, settle, now);
                slot.scale = Tween::still(scale, now);
                slot.shown = Tween::still(shown, now);
                slot.clip = Tween::still(clip, now);
                continue;
            }
            slot.rise.aim(rise, settle, now);
            slot.scale.aim(scale, settle, now);
            slot.clip.aim(clip, settle, now);
            slot.shown.aim(shown, settle, now);
            slot.opacity.aim(opacity, settle, now);
        }

        let piled = order.len().clamp(1, VISIBLE) as f32 - 1.;
        px(match expanded {
            true => (reach - gap).max(0.),
            false => front + piled * gap,
        })
    }

    fn card(&self, slot: &Slot, now: Instant, cx: &mut Context<Self>) -> Toast {
        let toast = &slot.toast;
        let id = toast.id;
        let mut args = i18n::FluentArgs::new();
        if let Some(name) = &toast.name {
            args.set("name", i18n::Value::value(name.as_ref()));
        }
        if let Some(count) = toast.count {
            args.set("count", i18n::Value::value(count));
        }
        let message = i18n::lookup(&toast.key, Some(&args));
        let swiped = self.swipe.as_ref().filter(|swipe| swipe.id == id);
        let (sway, drop) = match swiped {
            Some(swipe) => (swipe.offset.x / px(1.), swipe.offset.y / px(1.)),
            None => (slot.sway.now(now), slot.drop.now(now)),
        };
        let scale = slot.scale.now(now).max(0.01);

        let card = Toast::new(("toast", id), message)
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(slot.rise.now(now)))
            .opacity(slot.opacity.now(now))
            .contents(slot.shown.now(now))
            .when(slot.entered, |card| card.h(px(slot.clip.now(now))))
            .when(!slot.entered, |card| card.opacity(0.))
            .when(scale != 1., |card| {
                card.layer_scale(scale).layer_scale_origin(point(0.5, 0.))
            })
            .when(sway != 0. || drop != 0., |card| {
                card.layer_translate(point(px(sway), px(drop)))
            })
            .when_some(toast.name.clone(), Toast::strong)
            .when(toast.single_line, Toast::single_line)
            .when_some(
                toast
                    .detail
                    .clone()
                    .or_else(|| toast.note.as_ref().map(|key| i18n::lookup(key, None))),
                Toast::detail,
            )
            .when_some(toast.action.as_ref(), |card, action| {
                match (&action.url, &action.target) {
                    (Some(_), _) => card.action_icon("icons/square-arrow-out-up-right.svg"),
                    (None, Some(_)) => card.action_icon("icons/external-link.svg"),
                    (None, None) => card,
                }
            })
            .map(|card| match toast.outcome {
                Outcome::Plain => card,
                Outcome::Done => card.icon("icons/circle-check.svg"),
                Outcome::Info => card.icon("icons/info.svg"),
                Outcome::Warning => card.icon("icons/triangle-alert.svg"),
                Outcome::Failed => card.icon("icons/octagon-x.svg"),
                Outcome::Loading => match toast.progress {
                    Some(done) => card.progress(done),
                    None => card.loading(),
                },
            })
            .on_measure({
                let measured = self.measured.clone();
                let stack = cx.entity().downgrade();
                move |height, window, _| {
                    if measured.borrow_mut().insert(id, height) == Some(height) {
                        return;
                    }
                    let stack = stack.clone();
                    window.on_next_frame(move |_, cx| {
                        stack.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
            });

        // A leaving or covered toast keeps every button so its text does not reflow, but lets
        // every press through to the toast in front.
        if slot.leaving.is_some() || !slot.active {
            return card
                .inert()
                .on_dismiss(|_, _, _| {})
                .when_some(toast.cancel.clone(), |card, cancel| {
                    card.cancel(i18n::lookup(&cancel.key, None), |_, _, _| {})
                })
                .when_some(toast.action.clone(), |card, action| {
                    card.action(i18n::lookup(&action.key, None), |_, _, _| {})
                });
        }

        let toasts = self.toasts.clone();
        let card = card
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.grab(id, event, cx)
                }),
            )
            .on_dismiss(move |_, _, cx| Toasts::discard(id, cx))
            .when_some(toast.target.clone().map(destination), |card, dest| {
                card.on_open(move |_, cx| navigate(dest.clone(), cx))
            });
        let card = match toast.cancel.clone() {
            Some(cancel) => {
                let toasts = toasts.clone();
                card.cancel(i18n::lookup(&cancel.key, None), move |_, _, cx| {
                    cancel.run(cx);
                    toasts.update(cx, |this, cx| this.dismiss(id, cx));
                })
            }
            None => card,
        };
        match toast.action.clone() {
            Some(action) => card.action(i18n::lookup(&action.key, None), move |_, _, cx| {
                action.run(cx);
                if let Some(target) = action.target.clone() {
                    navigate(destination(target), cx);
                }
                if let Some(url) = &action.url {
                    cx.open_url(url);
                }
                toasts.update(cx, |this, cx| this.dismiss(id, cx));
            }),
            None => card,
        }
    }

    /// Listens to the pointer across the whole window while a toast is held, so the drag keeps
    /// following it off the toast and ends wherever the button comes up.
    fn tracker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stack = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moved = stack.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        moved
                            .update(cx, |this, cx| this.drag(event.position, cx))
                            .ok();
                    }
                });
                let released = stack.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        released.update(cx, |this, cx| this.release(cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

impl Render for ToastStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let now = Instant::now();
        self.sync(now, cx);
        if self.slots.is_empty() {
            if self.hovered || self.swipe.is_some() {
                self.hovered = false;
                self.swipe = None;
                self.hold(cx);
            }
            return div();
        }
        let height = self.arrange(now, cx);

        let moving = self.swipe.is_some()
            || self.slots.iter().any(|slot| {
                !slot.entered
                    || slot.leaving.is_some()
                    || [
                        slot.rise,
                        slot.scale,
                        slot.opacity,
                        slot.shown,
                        slot.clip,
                        slot.drop,
                        slot.sway,
                    ]
                    .iter()
                    .any(|tween| tween.moving(now))
            });
        if moving {
            window.request_animation_frame();
        }

        let cards: Vec<Toast> = self
            .slots
            .iter()
            .map(|slot| self.card(slot, now, cx))
            .collect();

        // The stack hangs from the top of the player bar and clips there, so a toast entering or
        // leaving slides out from under it rather than across.
        div()
            .absolute()
            .bottom(relative(1.))
            .left_0()
            .right_0()
            .h(window.viewport_size().height)
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_end()
            .items_center()
            .px(theme.metrics.pad * 2)
            .pb(clearance(cx))
            .child(
                div()
                    .id("toast-stack")
                    .role(Role::Log)
                    .relative()
                    .occlude()
                    .w(theme.font_size * WIDTH)
                    .max_w_full()
                    .h(height)
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.hover(*hovered, cx)))
                    .children(cards)
                    .when(self.swipe.is_some(), |stack| stack.child(self.tracker(cx))),
            )
    }
}

impl Slot {
    fn new(toast: state::Toast, now: Instant) -> Self {
        Self {
            toast,
            height: None,
            index: 0,
            active: false,
            entered: false,
            leaving: None,
            flung: false,
            rise: Tween::still(0., now),
            scale: Tween::still(1., now),
            opacity: Tween::still(0., now),
            shown: Tween::still(1., now),
            clip: Tween::still(0., now),
            drop: Tween::still(0., now),
            sway: Tween::still(0., now),
        }
    }
}

impl Tween {
    fn still(at: f32, now: Instant) -> Self {
        Self {
            from: at,
            to: at,
            start: now,
            span: Duration::ZERO,
            ease: css_ease,
        }
    }

    fn now(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= self.span {
            return self.to;
        }
        let progress = elapsed.as_secs_f32() / self.span.as_secs_f32();
        self.from + (self.to - self.from) * (self.ease)(progress)
    }

    /// Heads for `to` from wherever the value is right now. Aiming where it already heads
    /// changes nothing, so calling this every frame keeps one smooth run.
    fn aim(&mut self, to: f32, span: Duration, now: Instant) {
        self.aim_eased(to, span, css_ease, now);
    }

    fn aim_eased(&mut self, to: f32, span: Duration, ease: fn(f32) -> f32, now: Instant) {
        if (to - self.to).abs() < 0.01 {
            return;
        }
        self.from = self.now(now);
        self.to = to;
        self.start = now;
        self.span = span;
        self.ease = ease;
    }

    fn moving(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) < self.span
    }
}

/// A motion's length at the chosen pace, or nothing when motion is reduced.
fn span(base: Duration, cx: &Context<ToastStack>) -> Duration {
    match ui::motion::animates(cx) {
        true => paced(base),
        false => Duration::ZERO,
    }
}

/// The space between the front toast and the top of the player bar.
fn clearance(cx: &Context<ToastStack>) -> Pixels {
    cx.theme().metrics.pad * 2
}

/// Shrinks a drag the wrong way so the toast gives a little and then stiffens.
fn damped(moved: Pixels) -> Pixels {
    let distance = moved / px(1.);
    px(distance / (1.5 + distance.abs() / 20.))
}

fn destination(target: Target) -> Destination {
    match target {
        Target::Song(id) => Destination::Song(id),
        Target::Album(id) => Destination::Album(id),
        Target::Artist(id) => Destination::Artist(id),
        Target::Playlist(id) => Destination::Playlist(id),
        Target::PlaybackSettings => Destination::Settings(SettingsPage::Playback),
    }
}
