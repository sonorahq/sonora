use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, EntityId, SpringConfig, SpringState, Window};
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
const REST: SpringState = at(0.);

struct State {
    shown: Levels,
    spectrum: Spectrum,
    listening: Option<Listening>,
    armed: bool,
    /// Set by every `show` and `hide`, so a step can tell the view stopped rendering.
    seen: bool,
    visible: bool,
    beat: Option<Instant>,
    /// How far the spectrum has come into view, from 0 hidden to 1 at full height.
    presence: SpringState,
}

impl Default for State {
    fn default() -> Self {
        Self {
            shown: Levels::default(),
            spectrum: Spectrum::default(),
            listening: None,
            armed: false,
            seen: false,
            visible: false,
            beat: None,
            presence: REST,
        }
    }
}

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

    pub fn show(&self, watch: EntityId, spectrum: Spectrum, window: &mut Window) {
        let mut state = self.state.borrow_mut();
        state.visible = true;
        state.seen = true;
        state.listening = Some(spectrum.listen());
        state.spectrum = spectrum;
        if state.armed {
            return;
        }
        state.armed = true;
        state.beat = None;
        drop(state);

        let drive = self.clone();
        window.on_next_frame(move |window, cx| drive.step(watch, window, cx));
    }

    /// Sends the spectrum sinking out of view. The steps go on until it has sunk.
    pub fn hide(&self) {
        let mut state = self.state.borrow_mut();
        state.visible = false;
        state.seen = true;
    }

    fn step(&self, watch: EntityId, window: &mut Window, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            if !std::mem::take(&mut state.seen) {
                state.armed = false;
                state.listening = None;
                state.presence = REST;
                return;
            }
            let now = Instant::now();
            let since = state
                .beat
                .replace(now)
                .map(|beat| now.duration_since(beat))
                .filter(|since| *since < STALL);

            let (target, spring) = match state.visible {
                true => (1., RISE),
                false => (0., SINK),
            };
            let stepped = match (since, cx.reduce_motion()) {
                (Some(since), false) => spring.step(state.presence, target, since.as_secs_f32()),
                // A first step has no time to cover, so the spring only starts from here.
                (None, false) => state.presence,
                (_, true) => at(target),
            };
            state.presence = match spring.is_settled(stepped, target, PRESENCE_REST) {
                true => at(target),
                false => stepped,
            };
            if !state.visible && state.presence == REST {
                state.armed = false;
                state.listening = None;
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
            ease(&mut state.shown.left, spectrum.left(), rate);
            ease(&mut state.shown.right, spectrum.right(), rate);
        }

        cx.notify(watch);
        let drive = self.clone();
        window.on_next_frame(move |window, cx| drive.step(watch, window, cx));
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
/// count changes.
fn ease(shown: &mut Vec<f32>, target: Vec<f32>, rate: f32) {
    if shown.len() != target.len() {
        *shown = target;
        return;
    }
    for (shown, target) in shown.iter_mut().zip(&target) {
        *shown += (target - *shown) * rate;
    }
}
