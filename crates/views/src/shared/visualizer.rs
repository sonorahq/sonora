use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, EntityId, Window};
use music::{Listening, Spectrum};
use ui::Levels;

/// How long the drawn levels take to close 63% of the gap to the analyzer's. A time constant
/// rather than a share per frame, so the motion is the same on a 60 Hz and a 144 Hz display
/// and a frame that came late moves further instead of falling behind.
const SETTLE: Duration = Duration::from_millis(40);
/// The longest gap between two frames that still counts as one step: past it the levels adopt
/// the target outright, so coming back from a stalled window snaps rather than swoops.
const STALL: Duration = Duration::from_millis(250);

#[derive(Default)]
struct State {
    shown: Levels,
    spectrum: Spectrum,
    listening: Option<Listening>,
    armed: bool,
    /// Set by every `show`, so a step can tell the view stopped rendering.
    seen: bool,
    visible: bool,
    beat: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct VisualizerDrive {
    state: Rc<RefCell<State>>,
}

impl VisualizerDrive {
    pub fn levels(&self) -> Levels {
        self.state.borrow().shown.clone()
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

    pub fn hide(&self) {
        self.state.borrow_mut().visible = false;
    }

    fn step(&self, watch: EntityId, window: &mut Window, cx: &mut App) {
        {
            let mut state = self.state.borrow_mut();
            if !state.visible || !std::mem::take(&mut state.seen) {
                state.armed = false;
                state.listening = None;
                return;
            }
            let spectrum = state.spectrum.clone();
            let now = Instant::now();
            let rate = match state.beat.replace(now) {
                Some(beat) if now.duration_since(beat) < STALL => {
                    1. - (-now.duration_since(beat).as_secs_f32() / SETTLE.as_secs_f32()).exp()
                }
                _ => 1.,
            };
            ease(&mut state.shown.left, spectrum.left(), rate);
            ease(&mut state.shown.right, spectrum.right(), rate);
        }

        cx.notify(watch);
        let drive = self.clone();
        window.on_next_frame(move |window, cx| drive.step(watch, window, cx));
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
