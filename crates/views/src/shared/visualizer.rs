//! Drives the spectrum drawn behind the fullscreen artwork. A thread of its own reads the playing
//! engine's spectrum about sixty times a second, eases the drawn levels toward it and pushes them
//! into `ui::Visualizer`, whose bars and wave are live values: the view never runs for them, and
//! the UI thread never rasterises the wave.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use music::Spectrum;
use ui::{Levels, Visualizer};

/// How many bands the analyser produces.
const BANDS: usize = 32;
/// How often the levels are read and drawn.
const FRAME: Duration = Duration::from_millis(16);
/// How long the drawn levels take to close 63% of the gap to the analyzer's. A time constant
/// rather than a share per frame, so a frame that came late moves further instead of falling
/// behind.
const SETTLE: Duration = Duration::from_millis(40);
/// The longest gap between two frames that still counts as one step: past it the levels adopt
/// the target outright, so coming back from a stall snaps rather than swoops.
const STALL: Duration = Duration::from_millis(250);
/// Below this much movement in every band a frame draws nothing, so a paused track's still
/// spectrum costs no frames.
const STILL: f32 = 1e-4;

/// The spectrum the thread reads, and a count bumped each time it is handed a new one.
type Feed = Arc<Mutex<(u64, Option<Spectrum>)>>;

/// The visualizer and the thread that feeds it while it shows.
pub(crate) struct VisualizerDrive {
    visualizer: Visualizer,
    feed: Feed,
    running: Option<Arc<AtomicBool>>,
}

impl Default for VisualizerDrive {
    fn default() -> Self {
        Self {
            visualizer: Visualizer::new(BANDS),
            feed: Arc::new(Mutex::new((0, None))),
            running: None,
        }
    }
}

impl VisualizerDrive {
    /// The drawing the view builds the element from.
    pub(crate) fn visualizer(&self) -> &Visualizer {
        &self.visualizer
    }

    /// Follows `spectrum` while the visualizer is on screen, starting the thread on the first
    /// one; `None` stops it. The playing engine can change between tracks, so the owner hands
    /// the current spectrum in on every poll.
    pub(crate) fn follow(&mut self, spectrum: Option<Spectrum>) {
        let Some(spectrum) = spectrum else {
            self.stop();
            return;
        };
        {
            let mut feed = self.feed.lock().unwrap_or_else(PoisonError::into_inner);
            feed.0 += 1;
            feed.1 = Some(spectrum);
        }
        if self.running.is_some() {
            return;
        }
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let feed = self.feed.clone();
        let visualizer = self.visualizer.clone();
        let spawned = thread::Builder::new()
            .name("visualizer".to_owned())
            .spawn(move || drive(&flag, &feed, &visualizer));
        match spawned {
            Ok(_) => self.running = Some(running),
            Err(error) => log::warn!("fullscreen: cannot start the visualizer: {error}"),
        }
    }

    /// Stops the thread, which lets go of the spectrum so its taps stop feeding the analyser.
    pub(crate) fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.store(false, Ordering::Relaxed);
        }
        self.feed.lock().unwrap_or_else(PoisonError::into_inner).1 = None;
    }
}

impl Drop for VisualizerDrive {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The thread: listens to the spectrum it was last handed, eases the shown levels toward it and
/// draws them, until `running` clears.
fn drive(running: &AtomicBool, feed: &Feed, visualizer: &Visualizer) {
    let mut shown = Levels::default();
    let mut beat: Option<Instant> = None;
    let mut heard: Option<(u64, Spectrum, music::Listening)> = None;
    while running.load(Ordering::Relaxed) {
        {
            let feed = feed.lock().unwrap_or_else(PoisonError::into_inner);
            match &feed.1 {
                None => heard = None,
                Some(spectrum) if heard.as_ref().is_none_or(|(at, ..)| *at != feed.0) => {
                    let listening = spectrum.listen();
                    heard = Some((feed.0, spectrum.clone(), listening));
                }
                Some(_) => {}
            }
        }
        if let Some((_, spectrum, _)) = &heard {
            let now = Instant::now();
            let rate = match beat.replace(now) {
                Some(beat) if now.duration_since(beat) < STALL => {
                    1. - (-now.duration_since(beat).as_secs_f32() / SETTLE.as_secs_f32()).exp()
                }
                _ => 1.,
            };
            let moved = ease(&mut shown.left, spectrum.left(), rate)
                | ease(&mut shown.right, spectrum.right(), rate);
            if moved {
                visualizer.push(&shown);
            }
        }
        thread::sleep(FRAME);
    }
}

/// Walks `shown` a share of the way towards `target`, adopting it outright when the band count
/// changes. True when any band moved enough to draw.
fn ease(shown: &mut Vec<f32>, target: Vec<f32>, rate: f32) -> bool {
    if shown.len() != target.len() {
        *shown = target;
        return true;
    }
    let mut moved = false;
    for (shown, target) in shown.iter_mut().zip(&target) {
        let step = (target - *shown) * rate;
        moved |= step.abs() > STILL;
        *shown += step;
    }
    moved
}
