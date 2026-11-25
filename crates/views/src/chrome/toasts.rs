use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Context, DispatchPhase, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, SpringConfig, Window, canvas, div, point, px, relative,
};
use router::{Destination, SettingsPage, navigate};
use state::{Outcome, Target, Toasts};
use ui::{ActiveTheme as _, Springs, Sprung, Toast};

/// The toast width in multiples of the font size.
const WIDTH: f32 = 26.;
/// The space between toasts when the stack is spread out, and how far each one behind the front
/// peeks out above it when the stack is piled up.
const GAP: Pixels = px(14.);
/// How much smaller each toast piled behind the front one is drawn.
const SHRINK: f32 = 0.1;
/// How many toasts the pile shows. The rest wait out of sight behind the last one.
const VISIBLE: usize = 3;
/// How far back from the release a swipe's speed is read, so the toast leaves the hand at the
/// speed it was going at the end rather than on average.
const TRAIL: Duration = Duration::from_millis(80);
/// The shortest stretch of a swipe that tells its speed. Two moves closer together than this
/// came in one burst and say nothing about how fast the hand went.
const TRAIL_SHORTEST: Duration = Duration::from_millis(8);
/// How close a value in pixels has to come to its target to count as landed.
const PIXEL_REST: f32 = 0.1;
/// How close a scale or an opacity has to come to its target to count as landed.
const LEVEL_REST: f32 = 0.002;
/// How faint a dismissed toast has to fade before it is dropped.
const GONE: f32 = 0.01;
/// How far past the release point a swiped toast drifts while it fades, as a share of its width.
const DRIFT: f32 = 0.15;
/// How far a toast has to be dragged right to dismiss it.
const FLING: Pixels = px(45.);
/// How fast, in pixels per millisecond, a shorter drag right has to be to dismiss the toast.
const FLICK: f32 = 0.11;
/// How much of an exponential ease-out's opening speed a toast sets off with when it is sent
/// somewhere. At 0 it gathers speed like a spring let go from rest, at 1 it starts at full speed,
/// which looks abrupt.
const ONSET: f32 = 0.5;

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

/// One toast on screen, kept after the toast is dismissed for as long as it takes to fade out.
struct Slot {
    toast: state::Toast,
    height: Option<Pixels>,
    index: usize,
    /// Whether the toast is fully shown and so takes presses. A toast piled behind the front one
    /// or waiting out of sight does not.
    active: bool,
    entered: bool,
    leaving: bool,
    flung: bool,
    rise: Spring,
    scale: Spring,
    opacity: Spring,
    shown: Spring,
    clip: Spring,
    drop: Spring,
    sway: Spring,
}

/// A drag on a toast, measured from where the button went down.
struct Swipe {
    id: usize,
    origin: Point<Pixels>,
    began: Instant,
    vertical: Option<bool>,
    offset: Point<Pixels>,
    /// Where the toast was over the last moments of the drag, which tells how fast it was going
    /// when it was let go.
    trail: Vec<(Instant, Point<Pixels>)>,
}

/// One value of a toast on a spring: where it stands, where it heads and on which spring. Sent
/// somewhere new halfway, it turns with the speed it had rather than starting over.
#[derive(Clone, Copy)]
struct Spring {
    sprung: Sprung,
    target: f32,
    config: SpringConfig,
    /// How close to the target the value has to come to count as landed.
    rest: f32,
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
        let now = Instant::now();
        self.swipe = Some(Swipe {
            id,
            origin: event.position,
            began: now,
            vertical: None,
            offset: Point::default(),
            trail: vec![(now, Point::default())],
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
        let now = Instant::now();
        swipe
            .trail
            .retain(|(at, _)| now.duration_since(*at) <= TRAIL);
        swipe.trail.push((now, swipe.offset));
        cx.notify();
    }

    /// Ends a drag: far or fast enough right dismisses the toast, anything else springs it back.
    /// Either way the toast leaves the hand at the speed it was going, so a flung one carries on
    /// and one let go on its way back keeps its pace into the return.
    fn release(&mut self, cx: &mut Context<Self>) {
        let Some(swipe) = self.swipe.take() else {
            return;
        };
        let travel = swipe.offset.x / px(1.);
        let millis = swipe.began.elapsed().as_secs_f32() * 1000.;
        let speed = travel / millis.max(1.);
        let flung = swipe.offset.x >= FLING || (travel > 0. && speed > FLICK);
        let thrown = thrown(&swipe.trail, Instant::now());
        let snap = Springs::RESPONSIVE;
        let drift = Springs::FLIGHT;
        let width = cx.theme().font_size * WIDTH / px(1.);

        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.toast.id == swipe.id) {
            slot.drop.launch(swipe.offset.y / px(1.), thrown.y);
            slot.drop.aim(0., snap);
            match flung {
                true => {
                    // a run sets off faster the further it has to go, so the toast heads on far
                    // enough to leave the hand at least as fast as it was thrown, and a nudge on
                    // at the least
                    let rate = ONSET * drift.canonical().0;
                    let reach = (thrown.x.max(0.) / rate).max(width * DRIFT);
                    slot.flung = true;
                    slot.sway.jump(travel);
                    slot.sway.aim(travel + reach, drift);
                }
                false => {
                    slot.sway.launch(travel, thrown.x);
                    slot.sway.aim(0., snap);
                }
            }
        }
        if flung {
            Toasts::discard(swipe.id, cx);
        }
        self.hold(cx);
    }

    /// Brings the slots in line with the toasts that are up: new ones get a slot, and the
    /// dismissed ones start leaving and are dropped once they have faded out.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let shown = self.toasts.read(cx).shown().to_vec();
        let expanded = self.expanded();
        // a leaving toast fades quicker than it slides, so it is gone partway through the slide
        let slide = Springs::FLIGHT;
        let fade = Springs::SWAP_OUT;
        let below = clearance(cx) / px(1.);

        for slot in &mut self.slots {
            if slot.leaving || shown.iter().any(|toast| toast.id == slot.toast.id) {
                continue;
            }
            slot.leaving = true;
            let height = slot.height.map_or(0., |height| height / px(1.));
            match (slot.flung, expanded || slot.index == 0) {
                // the swipe already sent it on its way
                (true, _) => slot.opacity.aim(0., fade),
                (false, true) => {
                    slot.drop.aim(height + below, slide);
                    slot.opacity.aim(0., fade);
                }
                (false, false) => {
                    slot.drop.aim(height * 0.4, slide);
                    slot.opacity.aim(0., fade);
                }
            }
        }
        self.slots.retain(|slot| !slot.gone());

        for toast in shown {
            match self.slots.iter_mut().find(|slot| slot.toast.id == toast.id) {
                Some(slot) => slot.toast = toast,
                None => self.slots.push(Slot::new(toast)),
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
    fn arrange(&mut self, cx: &mut Context<Self>) -> Pixels {
        let expanded = self.expanded();
        // no spring here overshoots: a toast covers most of the way at once and slows into place
        let settle = Springs::RESPONSIVE;
        let below = clearance(cx) / px(1.);
        let mut order: Vec<usize> = (0..self.slots.len())
            .filter(|at| {
                let slot = &self.slots[*at];
                !slot.leaving && slot.height.is_some()
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
                slot.rise.jump(rise - height - below);
                slot.rise.aim(rise, settle);
                slot.opacity.aim(opacity, settle);
                slot.scale.jump(scale);
                slot.shown.jump(shown);
                slot.clip.jump(clip);
                continue;
            }
            slot.rise.aim(rise, settle);
            slot.scale.aim(scale, settle);
            slot.clip.aim(clip, settle);
            slot.shown.aim(shown, settle);
            slot.opacity.aim(opacity, settle);
        }

        let piled = order.len().clamp(1, VISIBLE) as f32 - 1.;
        px(match expanded {
            true => (reach - gap).max(0.),
            false => front + piled * gap,
        })
    }

    fn card(&self, slot: &Slot, cx: &mut Context<Self>) -> Toast {
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
            None => (slot.sway.now(), slot.drop.now()),
        };
        let scale = slot.scale.now().max(0.01);
        let theme = *cx.theme();

        let card = Toast::new(("toast", id), message)
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(slot.rise.now()))
            .opacity(slot.opacity.now().clamp(0., 1.))
            .contents(slot.shown.now().clamp(0., 1.))
            .when(slot.entered, |card| card.h(px(slot.clip.now().max(0.))))
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
                Outcome::Done => card
                    .icon("icons/circle-check.svg")
                    .tint(theme.status_success),
                Outcome::Info => card.icon("icons/info.svg").tint(theme.status_info),
                Outcome::Warning => card
                    .icon("icons/triangle-alert.svg")
                    .tint(theme.status_warning),
                Outcome::Failed => card.icon("icons/octagon-x.svg").tint(theme.status_error),
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
        if slot.leaving || !slot.active {
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
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        stack.update(cx, |this, cx| this.release(cx)).ok();
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
        self.sync(cx);
        if self.slots.is_empty() {
            if self.hovered || self.swipe.is_some() {
                self.hovered = false;
                self.swipe = None;
                self.hold(cx);
            }
            return div();
        }
        let height = self.arrange(cx);
        // every spring asks for the next frame while it moves; a toast waiting to be measured or
        // to be dropped once it has faded asks here, since under reduced motion nothing moves
        for slot in &mut self.slots {
            slot.step(window, cx);
        }
        if self.swipe.is_some() || self.slots.iter().any(|slot| !slot.entered || slot.leaving) {
            window.request_animation_frame();
        }

        let cards: Vec<Toast> = self.slots.iter().map(|slot| self.card(slot, cx)).collect();

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
                    // off the pixel grid, so a toast's place, height and text glide instead of
                    // stepping a pixel at a time as they slow down
                    .pixel_snap(false)
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
    fn new(toast: state::Toast) -> Self {
        Self {
            toast,
            height: None,
            index: 0,
            active: false,
            entered: false,
            leaving: false,
            flung: false,
            rise: Spring::at(0., PIXEL_REST),
            scale: Spring::at(1., LEVEL_REST),
            opacity: Spring::at(0., LEVEL_REST),
            shown: Spring::at(1., LEVEL_REST),
            clip: Spring::at(0., PIXEL_REST),
            drop: Spring::at(0., PIXEL_REST),
            sway: Spring::at(0., PIXEL_REST),
        }
    }

    /// Moves every value of the toast one frame on.
    fn step(&mut self, window: &Window, cx: &App) {
        for spring in [
            &mut self.rise,
            &mut self.scale,
            &mut self.opacity,
            &mut self.shown,
            &mut self.clip,
            &mut self.drop,
            &mut self.sway,
        ] {
            spring.step(window, cx);
        }
    }

    /// Whether a dismissed toast has faded out far enough to drop.
    fn gone(&self) -> bool {
        self.leaving && self.opacity.now() <= GONE
    }
}

impl Spring {
    /// A value resting at `at`. Until it is aimed somewhere the spring it moves on does not
    /// matter.
    fn at(at: f32, rest: f32) -> Self {
        Self {
            sprung: Sprung::at(at),
            target: at,
            config: Springs::RESPONSIVE,
            rest,
        }
    }

    fn now(&self) -> f32 {
        self.sprung.position()
    }

    /// Heads for `to` on `config` from wherever the value stands. It sets off already moving and
    /// slows into place, the way an ease-out curve does, rather than gathering speed first the
    /// way a spring let go from rest would, and keeps whatever speed a throw gave it on top.
    /// Aiming where it already heads changes nothing, so calling this every frame keeps one
    /// smooth run.
    fn aim(&mut self, to: f32, config: SpringConfig) {
        if to == self.target && config == self.config {
            return;
        }
        let carry = self.sprung.velocity() - self.ease();
        self.target = to;
        self.config = config;
        self.sprung.set_velocity(carry + self.ease());
    }

    /// Puts the value at `at` at once, resting there.
    fn jump(&mut self, at: f32) {
        self.sprung.jump(at);
        self.target = at;
    }

    /// Puts the value at `at` easing back to where it was aimed, moving `speed` units a second
    /// faster than that, as a toast let go of after a swipe. A hand that stopped before letting go
    /// leaves it easing straight back, and one thrown back towards the target carries at most as
    /// much speed as still lands there without passing it.
    fn launch(&mut self, at: f32, speed: f32) {
        self.sprung.jump(at);
        let gap = self.target - at;
        let most = (1. - ONSET) * self.config.canonical().0 * gap.abs();
        let speed = match speed * gap > 0. {
            true => speed.clamp(-most, most),
            false => speed,
        };
        self.sprung.set_velocity(speed + self.ease());
    }

    /// How fast the value sets off on a run from where it stands to its target: [`ONSET`] of the
    /// speed at which a critically damped spring closes the gap at an exponential rate.
    fn ease(&self) -> f32 {
        ONSET * self.config.canonical().0 * (self.target - self.now())
    }

    /// Moves the value one frame towards its target, asking for the next frame while it moves.
    /// It lands at once under reduced motion.
    fn step(&mut self, window: &Window, cx: &App) {
        self.sprung
            .follow(self.target, self.config, self.rest, window, cx);
    }
}

/// The space between the front toast and the top of the player bar.
fn clearance(cx: &Context<ToastStack>) -> Pixels {
    cx.theme().metrics.pad * 2
}

/// How fast a swiped toast was moving as it was let go at `now`, in pixels a second, read from
/// the last stretch of the drag up to the release. A hand that slowed before letting go throws
/// less, and one that stopped throws nothing.
fn thrown(trail: &[(Instant, Point<Pixels>)], now: Instant) -> Point<f32> {
    let start = trail.partition_point(|(at, _)| now.duration_since(*at) > TRAIL);
    let (Some((first, from)), Some((last, to))) = (trail[start..].first(), trail.last()) else {
        return Point::default();
    };
    if last.duration_since(*first) < TRAIL_SHORTEST {
        return Point::default();
    }
    let moved = *to - *from;
    let seconds = now.duration_since(*first).as_secs_f32();
    point(moved.x / px(1.) / seconds, moved.y / px(1.) / seconds)
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
        Target::ProviderSettings => Destination::Settings(SettingsPage::Accounts),
    }
}
