use std::time::{Duration, Instant};

use gpui::{
    App, Global, NavigationDirection, Pixels, Point, ScrollHandle, ScrollWheelEvent, TouchPhase, px,
};

/// How far two fingers travel sideways before the swipe turns the page.
const REACH: Pixels = px(150.);
/// How far a gesture may wander up or down before it counts as scrolling the page instead.
const DRIFT: Pixels = px(30.);
/// The quiet that ends a gesture on a platform that never reports the fingers lifting.
const PAUSE: Duration = Duration::from_millis(150);

/// The touchpad gesture under way, as far as a sideways swipe across the page is concerned.
#[derive(Default)]
struct Swipe {
    travel: Point<Pixels>,
    last: Option<Instant>,
    /// Whether the platform said when the fingers landed, so it will say when they lift too and
    /// the swipe waits for that before it turns the page.
    phased: bool,
    /// Whether the rest of the gesture is ignored, because it already turned the page, moved a
    /// sideways scroller or went up or down.
    spent: bool,
    /// Whether a sideways scroller took the event now being dispatched.
    held: bool,
}

impl Global for Swipe {}

/// Claims the touchpad gesture under way for a sideways scroller with somewhere to go, so moving
/// a rail never turns the page as well. Call it from the scroller's own wheel listener, which
/// runs before the page's.
pub fn hold_swipe(event: &ScrollWheelEvent, scroll: &ScrollHandle, cx: &mut App) {
    if event.delta.precise()
        && event.delta.pixel_delta(px(1.)).x != Pixels::ZERO
        && scroll.max_offset().x > Pixels::ZERO
    {
        cx.default_global::<Swipe>().held = true;
    }
}

/// Follows a two-finger touchpad swipe across the page and returns where it turns the page, once
/// per gesture. Swiping right goes back, as in a browser. Where the platform reports the fingers
/// lifting, the page turns then, and only if they are still far enough out, so pulling back
/// cancels it. Elsewhere it turns the moment the fingers have gone far enough.
pub fn swiped(event: &ScrollWheelEvent, cx: &mut App) -> Option<NavigationDirection> {
    let swipe = cx.default_global::<Swipe>();
    let held = std::mem::take(&mut swipe.held);
    if !event.delta.precise() {
        return None;
    }

    let now = Instant::now();
    let started = event.touch_phase == TouchPhase::Started;
    if started || swipe.last.is_none_or(|last| now - last > PAUSE) {
        swipe.travel = Point::default();
        swipe.phased = started;
        swipe.spent = false;
    }
    swipe.last = Some(now);
    if held {
        swipe.spent = true;
    }
    if swipe.spent {
        return None;
    }

    let lifted = match event.touch_phase {
        TouchPhase::Started | TouchPhase::Moved => false,
        TouchPhase::Ended => true,
        // The system took the touch, which turns no page.
        TouchPhase::Cancelled => {
            swipe.spent = true;
            return None;
        }
    };
    swipe.travel += event.delta.pixel_delta(px(1.));
    let Point { x, y } = swipe.travel;
    if y.abs() > DRIFT && y.abs() > x.abs() {
        swipe.spent = true;
        return None;
    }
    // Momentum keeps scrolling after the fingers lift, which must not finish a swipe they did
    // not.
    if lifted {
        swipe.spent = true;
    }
    if x.abs() < REACH || swipe.phased != lifted {
        return None;
    }
    swipe.spent = true;
    Some(direction(x))
}

/// Where a swipe still under the fingers points and how far it has got towards turning the page,
/// for drawing a cue. 1 is where letting go turns the page, and the fingers may go on past it.
/// `None` once the fingers lift, or where the platform never says when they do, since there the
/// page turns without waiting.
pub fn swipe_pull(cx: &App) -> Option<(NavigationDirection, f32)> {
    let swipe = cx.try_global::<Swipe>()?;
    let x = swipe.travel.x;
    if !swipe.phased || swipe.spent || x == Pixels::ZERO {
        return None;
    }
    Some((direction(x), x.abs() / REACH))
}

fn direction(x: Pixels) -> NavigationDirection {
    match x > Pixels::ZERO {
        true => NavigationDirection::Back,
        false => NavigationDirection::Forward,
    }
}
