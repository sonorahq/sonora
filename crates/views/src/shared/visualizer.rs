use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, EntityId, SpringConfig, SpringState, Task, Window};
use music::{Listening, Spectrum};
use ui::Levels;

/// How long the drawn levels take to close 63% of the gap to the analyzer's. A time constant
/// rather than a share per frame, so the motion is the same on a 60 Hz and a 144 Hz display
/// and a frame that came late moves further instead of falling behind.
const SETTLE: Duration = Duration::from_millis(40);
/// The longest gap between two frames that still counts as one step: past it the levels adopt
/// the target outright, so coming back from a stalled window snaps rather than swoops.
const STALL: Duration = Duration::from_millis(250);
/// The spectrum growing up from the floor as it appears.
const RISE: SpringConfig = ui::spring(0.5, 0.);
/// The spectrum sinking back as it goes, quicker than it came.
const SINK: SpringConfig = ui::spring(0.3, 0.);
/// How far the presence may sit from its target and still count as landed.
const PRESENCE_REST: f32 = 1e-3;
/// A spectrum that is out of view and going nowhere.
const HIDDEN: SpringState = at(0.);
/// How often levels at rest are redrawn anyway. A redraw is what tells the drive the view is
/// still on screen, so this bounds how long it keeps listening after the view goes away.
const HEARTBEAT: Duration = Duration::from_millis(500);
/// How close a drawn level has to come to its target to be put on it, so a paused track comes
/// to rest instead of easing towards zero forever.
const REST: f32 = 1e-3;

/// Holds the eased levels and the next timed update without keeping its view alive.
#[derive(Default)]
struct State {
    shown: Levels,
    spectrum: Spectrum,
    listening: Option<Listening>,
    armed: bool,
    /// The last render that asked for this visualizer, shown or hidden.
    seen: Option<Instant>,
    visible: bool,
    beat: Option<Instant>,
    /// When the view was last asked to redraw.
    drawn: Option<Instant>,
    /// Owns the next update without retaining a hidden or dropped visualizer.
    pacing: Option<Task<()>>,
    /// How far the spectrum has come into view, from 0 hidden to 1 at full height.
    presence: SpringState,
}

/// Eases the analyzer levels at most 120 times a second and redraws only when they move.
/// Its timer keeps the update rate independent of frames that GPUI chooses not to draw.
#[derive(Clone, Default)]
pub struct VisualizerDrive {
    state: Rc<RefCell<State>>,
}

impl VisualizerDrive {
    /// The levels to draw, scaled by how far the spectrum has come into view.
    pub fn levels(&self) -> Levels {
        let state = self.state.borrow();
        let presence = state.presence.position.clamp(0., 1.);
        let scaled = |bands: &[f32]| bands.iter().map(|band| band * presence).collect();
        Levels {
            left: scaled(&state.shown.left),
            right: scaled(&state.shown.right),
        }
    }

    /// How far the spectrum has come into view, from 0 hidden to 1 at full height. The owner
    /// draws the visualizer while this is above 0, which outlasts `hide` by the sink.
    pub fn presence(&self) -> f32 {
        self.state.borrow().presence.position.clamp(0., 1.)
    }

    /// Marks the visualizer as on screen in `watch` and starts stepping it if it is not
    /// already. Call it from every render that draws the visualizer.
    pub fn show(&self, watch: EntityId, spectrum: Spectrum, window: &mut Window) {
        let mut state = self.state.borrow_mut();
        state.visible = true;
        state.seen = Some(Instant::now());
        state.listening = Some(spectrum.listen());
        state.spectrum = spectrum;
        if state.armed {
            return;
        }
        state.armed = true;
        state.beat = None;
        state.drawn = None;
        drop(state);

        let drive = self.clone();
        window.on_next_frame(move |window, cx| drive.step(watch, window, cx));
    }

    /// Sends the spectrum sinking out of view. The updates go on until it has sunk, so call it
    /// from every render that leaves the visualizer out.
    pub fn hide(&self) {
        let mut state = self.state.borrow_mut();
        state.visible = false;
        state.seen = Some(Instant::now());
    }

    /// Samples the analyzer, moves the spectrum in or out of view and schedules the next update
    /// independently of drawing. It stops once the view no longer renders or the spectrum has
    /// sunk away.
    fn step(&self, watch: EntityId, window: &mut Window, cx: &mut App) {
        let now = Instant::now();
        let redraw = {
            let mut state = self.state.borrow_mut();
            let gone = state
                .seen
                .is_none_or(|seen| now.duration_since(seen) > HEARTBEAT + STALL);
            if gone {
                state.armed = false;
                state.listening = None;
                state.pacing = None;
                state.presence = HIDDEN;
                return;
            }
            let since = state
                .beat
                .replace(now)
                .map(|beat| now.duration_since(beat))
                .filter(|since| *since < STALL);

            let (target, spring) = match state.visible {
                true => (1., RISE),
                false => (0., SINK),
            };
            let before = state.presence;
            let stepped = match (since, cx.reduce_motion()) {
                (Some(since), false) => spring.step(before, target, since.as_secs_f32()),
                // A first step has no time to cover, so the spring only starts from here.
                (None, false) => before,
                (_, true) => at(target),
            };
            state.presence = match spring.is_settled(stepped, target, PRESENCE_REST) {
                true => at(target),
                false => stepped,
            };
            if !state.visible && state.presence == HIDDEN {
                state.armed = false;
                state.listening = None;
                state.pacing = None;
                drop(state);
                // The owner draws once more, to drop a visualizer that has sunk.
                cx.notify(watch);
                return;
            }

            let spectrum = state.spectrum.clone();
            let rate = match since {
                Some(since) => 1. - (-since.as_secs_f32() / SETTLE.as_secs_f32()).exp(),
                None => 1.,
            };
            let left = ease(&mut state.shown.left, spectrum.left(), rate);
            let right = ease(&mut state.shown.right, spectrum.right(), rate);
            let redraw = left
                || right
                || state.presence != before
                || state
                    .drawn
                    .is_none_or(|drawn| now.duration_since(drawn) >= HEARTBEAT);
            if redraw {
                state.drawn = Some(now);
            }
            redraw
        };

        if redraw {
            cx.notify(watch);
            // Arm the platform frame source as well as dirtying the view.
            window.on_next_frame(|_, _| {});
        }
        let weak = Rc::downgrade(&self.state);
        let handle = window.window_handle();
        let timer = super::animation::next(1, cx);
        self.state.borrow_mut().pacing = Some(cx.spawn(async move |cx| {
            timer.await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            handle
                .update(cx, |_, window, cx| Self { state }.step(watch, window, cx))
                .ok();
        }));
    }
}

/// A presence resting at `position`.
const fn at(position: f32) -> SpringState {
    SpringState {
        position,
        velocity: 0.,
    }
}

/// Walks `shown` a share of the way towards `target`, adopting it outright when the band
/// count changes or a band comes within `REST` of it. Returns whether any level moved.
fn ease(shown: &mut Vec<f32>, target: Vec<f32>, rate: f32) -> bool {
    if shown.len() != target.len() {
        *shown = target;
        return true;
    }
    let mut moved = false;
    for (shown, target) in shown.iter_mut().zip(&target) {
        let next = match (target - *shown).abs() < REST {
            true => *target,
            false => *shown + (target - *shown) * rate,
        };
        moved |= next != *shown;
        *shown = next;
    }
    moved
}
