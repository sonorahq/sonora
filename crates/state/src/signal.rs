//! What the equalizer screen shows of the sound on its way out: the spectrum of the music ahead
//! of the equalizer, and the signal path from the track to the device. Nothing is analysed or
//! read unless a screen holds a watch.

use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{Context, Entity, Task};
use music::output::{self, Device, Sample};
use music::{Encoding, Level, Normalisation};

use crate::{Outputs, Playback, PlaybackState};

/// How often the spectrum is read while watched. The analysis moves on every eighth of its
/// window, about 43 ms at 48 kHz, so this is as often as anything new can be drawn.
const SPECTRUM_TICK: Duration = Duration::from_millis(33);
/// How often the signal path is read while watched.
const PATH_TICK: Duration = Duration::from_millis(250);
/// How many bits of a sample the 32-bit floats Sonora works in carry.
const FLOAT_BITS: u8 = 24;

/// The spectrum heard ahead of the equalizer and the signal path, read while a screen watches.
pub struct Signal {
    playback: Entity<Playback>,
    outputs: Entity<Outputs>,
    /// The latest levels, `None` while nothing is loaded or before a window has filled.
    levels: Option<Vec<Level>>,
    path: Option<SignalPath>,
}

/// What happens to the sound on its way out, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalPath {
    /// The track as it reaches Sonora's chain, `None` until it has.
    pub source: Option<Source>,
    pub normalisation: Normalisation,
    /// The equalizer, while it is on.
    pub equalizer: Option<Equalizing>,
    /// The volume as the slider shows it, from 0 to 1.
    pub level: f32,
    /// The gain the volume applies, 1 at full volume.
    pub volume: f32,
    /// What Sonora sends the device, `None` while no output is open.
    pub sent: Option<output::Format>,
    /// The device the sound server plays on, when it says.
    pub device: Option<Device>,
}

/// The track as decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub rate: u32,
    pub channels: u16,
    /// How the provider encodes it, when it says.
    pub encoding: Option<Encoding>,
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
pub enum Change {
    /// The track carries more bits than the floats Sonora works in, so they are rounded.
    Rounded {
        from: u8,
    },
    /// A fixed normalisation gain, in decibels.
    Normalised(f32),
    /// librespot's normaliser, whose gain follows the music.
    Limited,
    Equalized,
    /// The volume's gain as a factor.
    Volume(f32),
    Resampled {
        from: u32,
        to: u32,
    },
    Remixed {
        from: u16,
        to: u16,
    },
    /// The device takes fewer bits than the track carries, or integers for a track that has
    /// no bit depth of its own.
    Requantized {
        to: u8,
    },
}

impl SignalPath {
    /// Everything that keeps what leaves Sonora from being the decoded track bit for bit, in
    /// the order it happens. Empty means bit-perfect from Sonora.
    pub fn changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();
        let bits = self
            .source
            .as_ref()
            .and_then(|source| source.encoding?.bits);
        if let Some(from) = bits.filter(|bits| *bits > FLOAT_BITS) {
            changes.push(Change::Rounded { from });
        }
        match self.normalisation {
            Normalisation::Gain(db) if db != 0. => changes.push(Change::Normalised(db)),
            Normalisation::Limited => changes.push(Change::Limited),
            _ => {}
        }
        if self
            .equalizer
            .is_some_and(|eq| eq.preamp_db != 0. || !eq.passes_through)
        {
            changes.push(Change::Equalized);
        }
        if self.volume != 1. {
            changes.push(Change::Volume(self.volume));
        }
        if let (Some(source), Some(sent)) = (&self.source, &self.sent) {
            if source.rate != sent.rate {
                changes.push(Change::Resampled {
                    from: source.rate,
                    to: sent.rate,
                });
            }
            if source.channels != sent.channels {
                changes.push(Change::Remixed {
                    from: source.channels,
                    to: sent.channels,
                });
            }
            if let Sample::Integer(to) = sent.sample
                && bits.is_none_or(|bits| bits.min(FLOAT_BITS) > to)
            {
                changes.push(Change::Requantized { to });
            }
        }
        changes
    }

    /// The rate the sound server runs the device at when it differs from what Sonora sends,
    /// so it resamples again. `None` when it is the same or nothing says.
    pub fn server_rate(&self) -> Option<u32> {
        let device = self.device.as_ref()?.format?;
        let sent = self.sent?;
        (device.rate != sent.rate).then_some(device.rate)
    }
}

impl Signal {
    pub fn new(playback: Entity<Playback>, outputs: Entity<Outputs>) -> Self {
        Self {
            playback,
            outputs,
            levels: None,
            path: None,
        }
    }

    /// The signal path as last read, while a screen watches it and a track is loaded.
    pub fn path(&self) -> Option<&SignalPath> {
        self.path.as_ref()
    }

    /// Reads the signal path for as long as the returned task is held.
    pub fn watch_path(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.read_path(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PATH_TICK).await;
                if this.update(cx, |this, cx| this.read_path(cx)).is_err() {
                    break;
                }
            }
        })
    }

    fn read_path(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let path = playback.route().map(|(route, id)| {
            let track = route.track(&id);
            let source = route.heard().map(|(rate, channels)| Source {
                rate,
                channels,
                encoding: track.as_ref().and_then(|track| track.encoding),
            });
            let rate = source.as_ref().map(|source| source.rate);
            let equalizer = playback.equalizer().then(|| Equalizing {
                preamp_db: playback.equalizer_preamp_db(),
                passes_through: rate
                    .is_some_and(|rate| playback.equalizer_curve().passes_through(rate)),
            });
            SignalPath {
                source,
                normalisation: track.map_or(Normalisation::Off, |track| track.normalisation),
                equalizer,
                level: playback.volume(),
                volume: playback.volume_gain(),
                sent: output::stream().map(|stream| stream.format),
                device: self.outputs.read(cx).device().cloned(),
            }
        });
        if path != self.path {
            self.path = path;
            cx.notify();
        }
    }

    /// The spectrum ahead of the equalizer, in dBFS, while a track is loaded and watched.
    pub fn levels(&self) -> Option<&[Level]> {
        self.levels.as_deref()
    }

    /// Analyses the music ahead of the equalizer for as long as the returned task is held. The
    /// analysis starts afresh, and stops with the task.
    pub fn watch(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let watch = Arc::new(self.playback.read(cx).equalizer_heard().watch());
        self.levels = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SPECTRUM_TICK).await;
                let reading = watch.clone();
                let levels = cx.background_spawn(async move { reading.read() }).await;
                if this.update(cx, |this, cx| this.show(levels, cx)).is_err() {
                    break;
                }
            }
        })
    }

    /// Takes in a read, cleared while nothing is loaded.
    fn show(&mut self, levels: Option<Vec<Level>>, cx: &mut Context<Self>) {
        let loaded = matches!(
            self.playback.read(cx).state(),
            PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Loading
        );
        let levels = levels.filter(|_| loaded);
        if levels != self.levels {
            self.levels = levels;
            cx.notify();
        }
    }
}
