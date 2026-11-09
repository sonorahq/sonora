//! The arrow that answers a two-finger touchpad swipe across the page. [`follow`] reads the
//! gesture from the wheel events over the page, turns the page when it completes and moves the
//! arrow through live values, so following the fingers never runs the view; [`view`] draws it.
//!
//! The page's own scroll view hears a wheel first, so the swipe is read wherever the page lets
//! it through: over what does not scroll, and as the gesture the page passes on.

use std::time::{Duration, Instant};

use rune::{Color, El, GesturePhase, Live, Tx, WheelEvent, el, icon};
use ui::{Springs, mix};

use crate::Ui;

/// How far two fingers travel sideways before the swipe turns the page.
const REACH: f32 = 150.;
/// How far a gesture may wander up or down before it counts as scrolling the page instead.
const DRIFT: f32 = 30.;
/// The quiet that ends a gesture on a platform that never reports the fingers lifting.
const PAUSE: Duration = Duration::from_millis(150);
const ICON: f32 = 16.;
/// How far the chevron leans towards the page it points to once the arrow is armed.
const LEAN: f32 = 2.;
/// How far past the turning point the arrow can still be dragged, as a share of the way there,
/// however far the fingers go.
const STRETCH: f32 = 0.3;
/// How small the arrow is as it first peeks in, against its full size.
const PEEK: f32 = 0.6;
/// How much bigger the arrow grows once letting go would turn the page. The spring that takes
/// it there overshoots, which is what makes it pop.
const ARMED: f32 = 0.12;
/// How much further the arrow grows as it flies off with the page it turned.
const LAUNCH_GROWTH: f32 = 0.35;

/// Which page a swipe turns to. Swiping right goes back, as in a browser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Direction {
    #[default]
    Back,
    Forward,
}

/// The gesture under way and the arrow's live pose.
pub struct State {
    travel: [f32; 2],
    last: Option<Instant>,
    /// Whether the platform said when the fingers landed, so it will say when they lift too and
    /// the swipe waits for that before it turns the page.
    phased: bool,
    /// Whether the rest of the gesture is ignored, because it already turned the page or went
    /// up or down.
    spent: bool,
    /// The side the arrow is on, which outlives the swipe while the arrow animates away.
    direction: Direction,
    /// How far in the arrow is, from out of sight to a gap in from the page's edge and past it.
    edge: Live<[f32; 2]>,
    reach: Live<f32>,
    scale: Live<f32>,
    /// How armed the arrow looks: the primary face over the quiet one.
    armed: Live<f32>,
    lean: Live<[f32; 2]>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            travel: [0.; 2],
            last: None,
            phased: false,
            spent: false,
            direction: Direction::Back,
            edge: Live::new([0.; 2]),
            reach: Live::new(0.),
            scale: Live::new(PEEK),
            armed: Live::new(0.),
            lean: Live::new([0.; 2]),
        }
    }
}

impl State {
    /// Draws the arrow `pull` of the way to turning the page, 1 being where letting go turns it,
    /// and `launch` of the way through flying off with the page it turned.
    fn pose(&self, pull: f32, launch: f32, size: f32, pad: f32) {
        let reach = stretched(pull);
        let armed = match pull >= 1. || launch > 0. {
            true => 1.,
            false => 0.,
        };
        let sign = match self.direction {
            Direction::Back => 1.,
            Direction::Forward => -1.,
        };
        let edge = -size + (size + pad) * reach + size * launch;
        self.edge.set([edge * sign, 0.]);
        self.reach.set(reach.min(1.) * (1. - launch));
        self.scale.set(
            ((PEEK + (1. - PEEK) * reach.min(1.))
                * (1. + ARMED * armed)
                * (1. + LAUNCH_GROWTH * launch))
                .max(0.01),
        );
        self.armed.set(armed);
        self.lean
            .set([(LEAN * armed + LEAN * 2. * launch) * -sign, 0.]);
    }
}

/// Follows a wheel event over the page: turns the page when a swipe completes, and moves the
/// arrow while one is under the fingers. Takes a sideways gesture, leaves the rest to scrolling.
pub(crate) fn follow(ui: &mut Ui, wheel: WheelEvent, tx: &mut Tx) -> bool {
    let theme = &ui.theme;
    let (size, pad) = (theme.metrics.control, theme.metrics.pad);
    let swipe = &mut ui.chrome.swipe;
    if !wheel.precise {
        tx.live_only();
        return false;
    }
    let now = Instant::now();
    let started = wheel.phase == GesturePhase::Began;
    if started || swipe.last.is_none_or(|last| now - last > PAUSE) {
        swipe.travel = [0.; 2];
        swipe.phased = started;
        swipe.spent = false;
    }
    swipe.last = Some(now);
    if swipe.spent || wheel.momentum {
        tx.live_only();
        return false;
    }
    let lifted = wheel.phase == GesturePhase::Ended;
    swipe.travel[0] += wheel.dx;
    swipe.travel[1] += wheel.dy;
    let [x, y] = swipe.travel;
    if y.abs() > DRIFT && y.abs() > x.abs() {
        swipe.spent = true;
        swipe.pose(0., 0., size, pad);
        tx.live_only();
        return false;
    }
    // momentum keeps scrolling after the fingers lift, which must not finish a swipe they did not
    if lifted {
        swipe.spent = true;
    }
    let direction = match x > 0. {
        true => Direction::Back,
        false => Direction::Forward,
    };
    let possible = match direction {
        Direction::Back => ui.nav.can_go_back(),
        Direction::Forward => ui.nav.can_go_forward(),
    };
    let swipe = &mut ui.chrome.swipe;
    let sideways = x.abs() >= y.abs();
    if x.abs() < REACH || swipe.phased != lifted || !possible {
        let pull = match swipe.phased && !swipe.spent && possible && x != 0. {
            true => x.abs() / REACH,
            false => 0.,
        };
        // the arrow moves to the other side only through the view
        let turned = pull > 0. && direction != swipe.direction;
        if turned {
            swipe.direction = direction;
        }
        swipe.pose(pull, 0., size, pad);
        if !turned {
            tx.live_only();
        }
        return sideways;
    }
    swipe.spent = true;
    swipe.direction = direction;
    swipe.pose(1., 1., size, pad);
    match direction {
        Direction::Back => ui.back(),
        Direction::Forward => ui.forward(),
    }
    ui.settle();
    true
}

/// The round arrow at the page's edge on the swipe's side. It peeks in as the fingers pull, grows
/// and fills in with a pop once letting go would turn the page, and flies on and fades as it
/// turns. With no swipe it is out of sight.
pub(crate) fn view(ui: &Ui) -> El {
    let theme = &ui.theme;
    let swipe = &ui.chrome.swipe;
    let size = theme.metrics.control;
    let chevron = match swipe.direction {
        Direction::Back => "icons/chevron-left.svg",
        Direction::Forward => "icons/chevron-right.svg",
    };
    let face = |key: &str, fill: ui::Hsla, ink: ui::Hsla, border: ui::Hsla| {
        el().key(key)
            .absolute(0., 0.)
            .size(size, size)
            .radius(size / 2.)
            .center()
            .fill(fill.color())
            .border(1., border.color())
            .child(
                icon(icons::get(chevron), ICON)
                    .color(ink.color())
                    .live_offset(swipe.lean.smooth(Springs::SETTLE)),
            )
    };
    let armed = face(
        "swipe-cue-armed",
        theme.primary,
        theme.primary_foreground,
        mix(theme.border, theme.primary, 1.),
    )
    .live_opacity(swipe.armed.smooth(Springs::SETTLE));
    let circle = el()
        .key("swipe-cue-circle")
        .size(size, size)
        .radius(size / 2.)
        .shadow(4., 12., Color::BLACK.alpha(0.25))
        .live_opacity(swipe.reach.smooth(Springs::QUICK))
        .live_scale(swipe.scale.smooth(Springs::SETTLE))
        .live_offset(swipe.edge.smooth(Springs::QUICK))
        .child(face(
            "swipe-cue-quiet",
            theme.secondary,
            theme.foreground,
            theme.border,
        ))
        .child(armed);
    let slot = el()
        .key("swipe-cue")
        .pin(None, Some(0.), None, Some(0.))
        .center_y()
        .inert()
        .a11y_hidden()
        .child(circle);
    match swipe.direction {
        Direction::Back => slot.pin(Some(0.), Some(0.), None, Some(0.)),
        Direction::Forward => slot.pin(None, Some(0.), Some(0.), Some(0.)),
    }
}

/// Where the arrow is drawn for a pull, giving way past the turning point like a rubber band so
/// it never runs off across the page.
fn stretched(pull: f32) -> f32 {
    match pull <= 1. {
        true => pull,
        false => {
            let over = pull - 1.;
            1. + STRETCH * over / (1. + over)
        }
    }
}
