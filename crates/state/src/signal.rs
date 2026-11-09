//! What the equalizer screen shows of the sound on its way out: the spectrum of the music ahead
//! of the equalizer, the signal path from the track to the device, and the output meter. Nothing
//! is analysed or read unless a screen holds a watch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use music::output::{DeviceInfo, Sample};
use music::{Level, Normalisation, TrackSignal};

use crate::{Change, Equalizer, Job, Outputs, Playback, PlaybackState, Store, Topic};

/// How often the spectrum is read while watched, about sixty times a second. The analysis moves
/// on every 32nd of its window, about 11 ms at 48 kHz, so every read has something new.
const SPECTRUM_TICK: Duration = Duration::from_millis(16);
/// How many spectrum ticks pass between two reads of the output meter: every one, so the meter
/// moves as smoothly as the spectrum.
const METER_EVERY: u32 = 1;
/// How many bits of a sample the 32-bit floats Sonora works in carry.
pub const FLOAT_BITS: u8 = 24;

/// The spectrum heard ahead of the equalizer, the signal path and the output meter, read while a
/// screen holds the one watch [`Store::watch_signal`] returns.
pub struct Signal {
    /// What the watch's tick refreshes off the UI thread, shared with it.
    live: Arc<Live>,
    path: Option<SignalPath>,
    /// Alive while a watch job or a `Following` is held, so the path is read only then. Every
    /// holder shares the one `Arc`, so the path stops being read only once the last lets go.
    watched: Weak<()>,
}

/// What the spectrum tick writes and the window reads on every frame, without the store raising
/// a change for each tick.
#[derive(Default)]
struct Live {
    /// The latest levels, `None` while nothing is loaded or before a window has filled.
    levels: Mutex<Option<Vec<Level>>>,
    /// Whether a track is loaded, as playback last said, so the tick clears the levels without
    /// asking the UI thread.
    loaded: AtomicBool,
    /// Whether the levels or the output meter moved since the window last asked.
    fresh: AtomicBool,
}

/// Keeps the signal path read for as long as it is held, without the spectrum or the meter.
pub struct Following {
    _held: Arc<()>,
}

/// What happens to the sound on its way out, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalPath {
    /// What the engine knows about the track playing: how it is encoded, what it decodes to and
    /// how it is normalised. `None` until the engine has opened it.
    pub track: Option<TrackSignal>,
    /// The equalizer, while it is on.
    pub equalizer: Option<Equalizing>,
    /// The volume as the slider shows it, from 0 to 1.
    pub level: f32,
    /// The gain the volume applies, 1 at full volume.
    pub volume: f32,
    /// The stream Sonora plays into, which every track is resampled to, with what the sound
    /// server says of the sink it plays on. `None` while none is open.
    pub stream: Option<DeviceInfo>,
}

/// What the equalizer does while on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Equalizing {
    pub preamp_db: f64,
    /// Whether its filters leave every sample untouched at the rate that plays.
    pub passes_through: bool,
}

/// Something Sonora does that changes the samples it decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SignalChange {
    /// The track carries more bits than the floats Sonora works in, so they are rounded.
    Rounded {
        from: u8,
    },
    /// A fixed normalisation gain, in decibels.
    Normalised(f32),
    /// librespot's normaliser, whose gain follows the music.
    Limited,
    /// Sonora's resampler, for a track at another rate than the stream.
    Resampled {
        from: u32,
        to: u32,
    },
    Equalized,
    /// The volume's gain as a factor.
    Volume(f32),
    Remixed {
        from: u16,
        to: u16,
    },
    /// The device takes fewer bits than the track carries, or integers for a track that has
    /// no bit depth of its own. `from` is the bits Sonora carries, `None` for such a track.
    Requantized {
        from: Option<u8>,
        to: u8,
    },
}

impl SignalPath {
    /// Everything that keeps what leaves Sonora from being the decoded track bit for bit, in
    /// the order it happens. Empty means bit-perfect from Sonora.
    pub fn changes(&self) -> Vec<SignalChange> {
        let mut changes = Vec::new();
        let bits = self
            .track
            .as_ref()
            .and_then(|track| track.encoding?.depth());
        if let Some(from) = bits.filter(|bits| *bits > FLOAT_BITS) {
            changes.push(SignalChange::Rounded { from });
        }
        match self.track.as_ref().map(|track| track.normalisation) {
            Some(Normalisation::Gain(db)) if db != 0. => changes.push(SignalChange::Normalised(db)),
            Some(Normalisation::Limited) => changes.push(SignalChange::Limited),
            _ => {}
        }
        if let Some((from, to)) = self.resampled() {
            changes.push(SignalChange::Resampled { from, to });
        }
        if self
            .equalizer
            .is_some_and(|eq| eq.preamp_db != 0. || !eq.passes_through)
        {
            changes.push(SignalChange::Equalized);
        }
        if self.volume != 1. {
            changes.push(SignalChange::Volume(self.volume));
        }
        if let (Some(track), Some(stream)) = (&self.track, &self.stream) {
            if track.channels != stream.format.channels {
                changes.push(SignalChange::Remixed {
                    from: track.channels,
                    to: stream.format.channels,
                });
            }
            if let Sample::Integer(to) = stream.format.sample
                && bits.is_none_or(|bits| bits.min(FLOAT_BITS) > to)
            {
                changes.push(SignalChange::Requantized {
                    from: bits.map(|bits| bits.min(FLOAT_BITS)),
                    to,
                });
            }
        }
        changes
    }

    /// The track's rate and the stream's, when Sonora resamples one to the other.
    pub fn resampled(&self) -> Option<(u32, u32)> {
        let (from, to) = (self.track.as_ref()?.rate, self.stream.as_ref()?.format.rate);
        (from != to).then_some((from, to))
    }

    /// The rate the sound server runs the sink at when it differs from the stream's, so it
    /// resamples again. `None` when it is the same or nothing says.
    pub fn server_rate(&self) -> Option<u32> {
        let stream = self.stream.as_ref()?;
        let sink = stream.sink.as_ref()?.format?;
        (sink.rate != stream.format.rate).then_some(sink.rate)
    }
}

impl Signal {
    pub fn new() -> Self {
        Self {
            live: Arc::default(),
            path: None,
            watched: Weak::new(),
        }
    }

    /// The signal path as last read, while a screen watches it and a track is loaded.
    pub fn path(&self) -> Option<&SignalPath> {
        self.path.as_ref()
    }

    /// The spectrum ahead of the equalizer, in dBFS, while a track is loaded and watched. The
    /// watch refreshes it about sixty times a second without raising anything, so a screen that
    /// draws it reads it on every frame.
    pub fn levels(&self) -> Option<Vec<Level>> {
        self.live
            .levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the spectrum or the output meter moved since the last call, which is when a
    /// window that shows either has to draw another frame.
    pub fn take_fresh(&self) -> bool {
        self.live.fresh.swap(false, Ordering::AcqRel)
    }

    /// The one `Arc` every watch and follower holds, made afresh when none is held.
    fn hold(&mut self) -> Arc<()> {
        self.watched.upgrade().unwrap_or_else(|| {
            let held = Arc::new(());
            self.watched = Arc::downgrade(&held);
            held
        })
    }

    /// Reads the signal path after a change, while a watch is held.
    fn follow_path(&mut self, playback: &Playback, outputs: &Outputs, eq: &Equalizer) {
        if self.watched.strong_count() > 0 {
            self.read_path(playback, outputs, eq);
        }
    }

    fn read_path(&mut self, playback: &Playback, outputs: &Outputs, eq: &Equalizer) {
        let path = playback.track_signal().map(|track| {
            let stream = outputs.current().cloned();
            let equalizer = eq.enabled().then(|| Equalizing {
                preamp_db: eq.preamp_db(),
                passes_through: stream
                    .as_ref()
                    .is_some_and(|stream| eq.played().passes_through(stream.format.rate)),
            });
            SignalPath {
                track: track.cloned(),
                equalizer,
                level: playback.volume(),
                volume: playback.volume_gain(),
                stream,
            }
        });
        if path != self.path {
            self.path = path;
        }
    }

    /// Notes whether a track is loaded, which the tick clears the levels by.
    fn note_loaded(&self, playback: &Playback) {
        let loaded = matches!(
            playback.state(),
            PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Loading
        );
        self.live.loaded.store(loaded, Ordering::Release);
    }
}

impl Default for Signal {
    fn default() -> Self {
        Self::new()
    }
}

impl Live {
    /// Takes in a read, cleared while nothing is loaded.
    fn show(&self, levels: Option<Vec<Level>>) {
        let levels = levels.filter(|_| self.loaded.load(Ordering::Acquire));
        let mut shown = self.levels.lock().unwrap_or_else(PoisonError::into_inner);
        if levels != *shown {
            *shown = levels;
            self.fresh.store(true, Ordering::Release);
        }
    }
}

impl Store {
    /// Analyses the music ahead of the equalizer, reads the output meter and follows the signal
    /// path for as long as the returned job is held. The spectrum and the meter start afresh,
    /// and everything stops with the job. The spectrum and the meter are refreshed on tokio
    /// without raising a change: the window reads [`Signal::levels`] and
    /// [`Equalizer::output_level`] on every frame, and [`Signal::take_fresh`] says when one moved.
    pub fn watch_signal(&mut self) -> Job {
        let Store {
            signal,
            playback,
            outputs,
            equalizer,
            cx,
            ..
        } = self;
        let watch = playback.analyzer().watch();
        let held = signal.hold();
        signal.note_loaded(playback);
        *signal
            .live
            .levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        signal.live.fresh.store(true, Ordering::Release);
        equalizer.reset_meter(cx);
        signal.read_path(playback, outputs, equalizer);
        let live = signal.live.clone();
        let meter = equalizer.metering();
        cx.spawn(|link| async move {
            let _held = held;
            for tick in (1..=METER_EVERY).cycle() {
                tokio::time::sleep(SPECTRUM_TICK).await;
                live.show(watch.read());
                if tick == METER_EVERY {
                    let read = meter.read(Instant::now());
                    if read.level {
                        live.fresh.store(true, Ordering::Release);
                    }
                    if read.overload {
                        link.post(|store| store.cx.changed(Topic::Equalizer));
                    }
                }
            }
        })
    }

    /// Follows the signal path alone for as long as the returned guard is held, for a screen that
    /// shows where the sound goes but not how loud it is.
    pub fn follow_signal(&mut self) -> Following {
        let Store {
            signal,
            playback,
            outputs,
            equalizer,
            ..
        } = self;
        let held = signal.hold();
        signal.read_path(playback, outputs, equalizer);
        Following { _held: held }
    }

    /// Reads the signal path again whenever playback, the outputs or the equalizer change while
    /// a watch is held, and notes whether a track is still loaded.
    pub(crate) fn react_signal(&mut self, change: &Change) {
        let Store {
            signal,
            playback,
            outputs,
            equalizer,
            ..
        } = self;
        match change {
            Change::Topic(Topic::Playback) => {
                signal.note_loaded(playback);
                signal.follow_path(playback, outputs, equalizer);
            }
            Change::Topic(Topic::Outputs | Topic::Equalizer) => {
                signal.follow_path(playback, outputs, equalizer)
            }
            _ => {}
        }
    }
}
