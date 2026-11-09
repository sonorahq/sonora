//! The toasts above the player bar, piled with the newest in front. Pointing at the pile spreads
//! it out and stops every toast's clock, and a toast dragged right far or fast enough is
//! dismissed.
//!
//! Rune moves the cards: each is laid out where it belongs in the pile or the spread, and its
//! place, height, scale and opacity spring there. A toast slides up out from under the bar as it
//! arrives and sinks back under it, or flies off to the right after a swipe, as it leaves.

use i18n::FluentArgs;
use router::{Destination, SettingsPage};
use rune::{Cx, Drag, El, Live, Phase, Spring, SpringPair, Transition, Tx, col, el};
use state::{Outcome, Target, Toast, ToastButton};
use ui::Springs;

use crate::Ui;
use crate::chrome::player_bar;

/// The toast width in multiples of the font size.
const WIDTH: f32 = 26.;
/// The space between toasts when the stack is spread out, and how far each one behind the front
/// peeks out above it when the stack is piled up.
const GAP: f32 = 14.;
/// How much smaller each toast piled behind the front one is drawn.
const SHRINK: f32 = 0.1;
/// How many toasts the pile shows. The rest wait out of sight behind the last one.
const VISIBLE: usize = 3;
/// How far a dismissed toast that was piled behind the front one sinks, as a share of its height.
const SINK: f32 = 0.4;
/// How far past the release point a swiped toast drifts while it fades, as a share of its width.
const DRIFT: f32 = 0.15;
/// How far a toast has to be dragged right to dismiss it.
const FLING: f32 = 45.;
/// How fast, in points per millisecond, a shorter drag right has to be to dismiss the toast.
const FLICK: f32 = 0.11;
/// How tall a toast is taken to be before it has been laid out once.
const ESTIMATE: f32 = 64.;
/// The spring every card moves to its place on, Sonner's 400 ms settle.
const SETTLE: Spring = Spring::new(0.4, 0.);
/// How quickly a swiped toast fades out, so it is gone after a nudge rather than seen crossing
/// the window.
const THROW: Spring = Spring::new(0.12, 0.);
/// How closely a held toast follows the pointer, and how it springs back when let go short.
const FOLLOW: Spring = Spring::new(0.12, 0.);

/// The stack's own state: whether the pointer spreads it, and the toast being swiped.
pub struct State {
    hovered: bool,
    /// The toast a press is dragging right now.
    dragging: Option<Drag0>,
    /// The toast the swipe offset belongs to, kept after the drag so it springs back.
    swiped: Option<usize>,
    /// Where the swiped toast is held off its place, drawn without running the view.
    offset: Live<[f32; 2]>,
}

/// A drag on a toast: which one, which axis its first real move picked, and when it began.
#[derive(Clone, Copy)]
struct Drag0 {
    id: usize,
    vertical: Option<bool>,
    began: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            hovered: false,
            dragging: None,
            swiped: None,
            offset: Live::new([0.; 2]),
        }
    }
}

/// One card's place in the stack, worked out from the cards in front of it.
struct Place {
    rise: f32,
    scale: f32,
    clip: f32,
    shown: f32,
    opacity: f32,
    /// Whether the card is fully shown and so takes presses.
    active: bool,
    /// Whether the card is the front one or the stack is spread, which decides how it leaves.
    leading: bool,
    height: f32,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let _ = layer;
    let theme = &ui.theme;
    let state = &ui.chrome.toasts;
    let shown = ui.store.toasts.shown();
    let bar = player_bar::height(ui, cx);
    let below = theme.metrics.pad * 2.;
    let width = theme.font_size * WIDTH;
    let expanded = state.hovered || state.dragging.is_some();

    // newest first: the front card is the last one raised
    let order: Vec<&Toast> = shown.iter().rev().collect();
    let heights: Vec<f32> = order
        .iter()
        .map(|toast| {
            cx.layout_with(&format!("{}/body", key(toast.id)), |rect| rect[3].round())
                .unwrap_or(ESTIMATE)
        })
        .collect();
    let front = heights.first().copied().unwrap_or(0.);
    let mut reach = 0.;
    let mut last = 0.;
    let places: Vec<Place> = heights
        .iter()
        .enumerate()
        .map(|(index, &height)| {
            // a toast past the visible ones waits behind the last of them, so when one ahead
            // leaves it fades in where it belongs instead of dropping in from far above
            let parked = index >= VISIBLE;
            let piled = index.min(VISIBLE - 1) as f32;
            let leading = (expanded || index == 0) && !parked;
            let rise = match (expanded, parked) {
                (true, false) => reach,
                (true, true) => last,
                (false, _) => piled * GAP,
            };
            if !parked {
                last = reach;
                reach += height + GAP;
            }
            Place {
                rise,
                scale: match expanded {
                    true => 1.,
                    false => 1. - piled * SHRINK,
                },
                clip: match leading {
                    true => height,
                    false => front,
                },
                shown: match leading {
                    true => 1.,
                    false => 0.,
                },
                opacity: match parked {
                    true => 0.,
                    false => 1.,
                },
                active: leading,
                leading: expanded || index == 0,
                height,
            }
        })
        .collect();
    let piled = order.len().clamp(1, VISIBLE) as f32 - 1.;
    let height = match expanded {
        true => (reach - GAP).max(0.),
        false => front + piled * GAP,
    };

    // oldest first in the tree, so the front card paints over the ones behind it
    let cards = order
        .iter()
        .zip(&places)
        .rev()
        .map(|(toast, place)| card(ui, toast, place, width, below));
    let stack = el()
        .key("toast-stack")
        .role("log")
        .w_full()
        .max_w(width)
        .h(height)
        .motion(SETTLE)
        .on_hover_change(|ui: &mut Ui, hovered: bool, _: &mut Tx| {
            ui.chrome.toasts.hovered = hovered;
            hold(ui);
        })
        .children(cards);

    // the stack hangs from the top of the player bar and clips there, so a toast entering or
    // leaving slides out from under it rather than across it
    col()
        .key("toasts")
        .pin(Some(0.), Some(0.), Some(0.), Some(bar))
        .clip_rounded(0.)
        .justify_end()
        .items_center()
        .px(theme.metrics.pad * 2.)
        .pb(below)
        .child(stack)
}

/// The key of toast `id`'s card.
fn key(id: usize) -> String {
    format!("toast-{id}")
}

/// One toast's card at its place.
fn card(ui: &Ui, toast: &Toast, place: &Place, width: f32, below: f32) -> El {
    let theme = &ui.theme;
    let state = &ui.chrome.toasts;
    let id = toast.id;
    let mut args = FluentArgs::new();
    if let Some(name) = &toast.name {
        args.set("name", i18n::Value::value(name.as_ref()));
    }
    if let Some(count) = toast.count {
        args.set("count", i18n::Value::value(count));
    }
    let message = i18n::lookup(&toast.key, Some(&args)).into_owned();
    let mut card = ui::Toast::new(key(id), message);
    if let Some(name) = &toast.name {
        card = card.strong(name.to_string());
    }
    if toast.single_line {
        card = card.single_line();
    }
    let detail = toast
        .detail
        .clone()
        .or_else(|| toast.note.as_ref().map(|key| i18n::lookup(key, None)));
    if let Some(detail) = detail {
        card = card.detail(detail.into_owned());
    }
    if let Some(action) = &toast.action {
        card = match (&action.url, &action.target) {
            (Some(_), _) => card.action_icon("icons/square-arrow-out-up-right.svg"),
            (None, Some(_)) => card.action_icon("icons/external-link.svg"),
            (None, None) => card,
        };
    }
    card = match toast.outcome {
        Outcome::Plain => card,
        Outcome::Done => card.icon("icons/circle-check.svg"),
        Outcome::Info => card.icon("icons/info.svg"),
        Outcome::Warning => card.icon("icons/triangle-alert.svg"),
        Outcome::Failed => card.icon("icons/octagon-x.svg"),
        Outcome::Loading => match toast.progress {
            Some(done) => card.progress(done),
            None => card.loading(),
        },
    };
    card = card
        .contents(place.shown)
        .on_dismiss(move |ui: &mut Ui, _: &mut Tx| {
            ui.store.discard_toast(id);
            ui.settle();
        });
    if let Some(target) = toast.target.clone() {
        card = card.on_open(move |ui: &mut Ui, _: &mut Tx| {
            ui.go(destination(target.clone()));
            ui.settle();
        });
    }
    if let Some(cancel) = &toast.cancel {
        card = card.cancel(
            i18n::lookup(&cancel.key, None),
            move |ui: &mut Ui, tx: &mut Tx| press(ui, tx, id, ToastButton::Cancel),
        );
    }
    if let Some(action) = &toast.action {
        card = card.action(
            i18n::lookup(&action.key, None),
            move |ui: &mut Ui, tx: &mut Tx| press(ui, tx, id, ToastButton::Action),
        );
    }
    // a covered toast keeps every button so its text does not reflow, but lets every press
    // through to the toast in front
    if !place.active {
        card = card.inert();
    }

    let enter = Transition::offset(0., place.height + below).combined(Transition::opacity());
    let swiping = state.dragging.is_some_and(|drag| drag.id == id);
    let exit = match (swiping, place.leading) {
        (true, _) => Transition::offset(width * DRIFT, 0.)
            .combined(Transition::opacity())
            .springs(SpringPair::new(THROW, THROW)),
        (false, true) => Transition::offset(0., place.height + below)
            .combined(Transition::opacity())
            .springs(SpringPair::new(SETTLE, SETTLE)),
        (false, false) => Transition::offset(0., place.height * SINK)
            .combined(Transition::opacity())
            .springs(SpringPair::new(SETTLE, Springs::QUICK)),
    };
    let mut card = card
        .build(theme)
        .pin(Some(0.), None, Some(0.), Some(place.rise))
        .h(place.clip)
        .opacity(place.opacity)
        .scale(place.scale.max(0.01))
        .origin(0.5, 0.)
        .motion(SETTLE)
        .transition(Transition::asymmetric(enter, exit).springs(SpringPair::new(SETTLE, SETTLE)));
    if state.swiped == Some(id) {
        card = card.live_offset(state.offset.smooth(FOLLOW));
    }
    if place.active {
        card =
            card.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| swipe(ui, id, drag, tx));
    }
    card
}

/// Presses one of toast `id`'s buttons, then opens the page or the link the button leads to.
fn press(ui: &mut Ui, tx: &mut Tx, id: usize, button: ToastButton) {
    if let Some(action) = ui.store.press_toast(id, button) {
        if let Some(target) = action.target.clone() {
            ui.go(destination(target));
        }
        if let Some(url) = &action.url
            && let Err(error) = tx.open_url(url)
        {
            log::warn!("toasts: cannot open {url}: {error}");
        }
    }
    ui.settle();
}

/// Follows a drag on toast `id`. Right is the way out, so a drag right moves the toast one to
/// one while a drag any other way only stretches it a little. The first real move picks the
/// axis. Far or fast enough right dismisses the toast; anything else springs it back.
fn swipe(ui: &mut Ui, id: usize, drag: &mut Drag, tx: &mut Tx) {
    let state = &mut ui.chrome.toasts;
    match drag.phase {
        Phase::Down => {
            state.dragging = Some(Drag0 {
                id,
                vertical: None,
                began: tx.clock(),
            });
            state.swiped = Some(id);
            state.offset.snap([0., 0.]);
            hold(ui);
        }
        Phase::Move => {
            tx.live_only();
            let Some(held) = &mut state.dragging else {
                return;
            };
            if held.vertical.is_none() && (drag.dx.abs() > 1. || drag.dy.abs() > 1.) {
                held.vertical = Some(drag.dy.abs() >= drag.dx.abs());
            }
            let offset = match held.vertical {
                None => [0., 0.],
                Some(false) if drag.dx > 0. => [drag.dx, 0.],
                Some(false) => [damped(drag.dx), 0.],
                Some(true) => [0., damped(drag.dy)],
            };
            state.offset.set(offset);
        }
        Phase::Up => {
            let Some(held) = state.dragging else {
                return;
            };
            let travel = match held.vertical {
                Some(false) => drag.dx.max(0.),
                _ => 0.,
            };
            let millis = ((tx.clock() - held.began) * 1000.) as f32;
            let speed = travel / millis.max(1.);
            let flung = !drag.cancelled() && (travel >= FLING || (travel > 0. && speed > FLICK));
            match flung {
                // the card keeps where it was let go and leaves from there to the right
                true => {
                    // the card leaves on the swipe's exit, built while the drag was held
                    state.dragging = None;
                    ui.store.discard_toast(held.id);
                }
                false => {
                    state.dragging = None;
                    state.offset.set([0., 0.]);
                }
            }
            hold(ui);
        }
    }
}

/// Stops the clocks while the stack is spread out and starts them again once it piles up.
fn hold(ui: &mut Ui) {
    let state = &ui.chrome.toasts;
    let expanded = state.hovered || state.dragging.is_some();
    ui.store.toasts.pause(expanded, &ui.store.cx);
    ui.settle();
}

/// Shrinks a drag the wrong way so the toast gives a little and then stiffens.
fn damped(moved: f32) -> f32 {
    moved / (1.5 + moved.abs() / 20.)
}

/// The page a toast's target opens.
fn destination(target: Target) -> Destination {
    match target {
        Target::Song(id) => Destination::Song(id.as_ref().into()),
        Target::Album(id) => Destination::Album(id.as_ref().into()),
        Target::Artist(id) => Destination::Artist(id.as_ref().into()),
        Target::Playlist(id) => Destination::Playlist(id.as_ref().into()),
        Target::PlaybackSettings => Destination::Settings(SettingsPage::Playback),
        Target::ProviderSettings => Destination::Settings(SettingsPage::Providers),
    }
}
