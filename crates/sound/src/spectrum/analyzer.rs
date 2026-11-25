//! The spectrum of the music on its way into the equalizer, for drawing behind its response.
//! Each output's equalizer stage has a probe on its input that sends nothing unless something
//! watches. A level sums the power of every bin within a twelfth of an octave of its frequency,
//! scaled so a full-scale sine reads 0 dBFS, which makes music come out roughly level.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::feed::{self, Feed, Heard, Layout, Listener};

/// The levels run from `LOW` to `HIGH` in hertz, `STEPS` to an octave. The graph behind the
/// equalizer only hints at what the curve shapes, so a coarse spectrum is all it needs, and
/// every level is a path point painted each frame.
const LOW: f64 = 20.;
const HIGH: f64 = 20_000.;
const STEPS: f64 = 12.;
/// How wide a band each level sums, in octaves around its frequency.
const SMOOTHING: f64 = 1. / 4.;
/// How many bins a level reaches at least to each side of its frequency, so a tone's main lobe
/// lands whole in the level nearest it where the band is narrower than that. Two
/// bins each side hold all but 0.005 dB of a Hann window's lobe wherever the tone falls.
const LOBE: f64 = 2.;
/// How long a window lasts, about: its length is the power of two nearest above this at the
/// rate, 16384 samples at 44.1 and 48 kHz. Long enough to part the lowest bands.
const SPAN: f64 = 0.3;
/// How many windows overlap at any time: one starts every 32nd of a window, so the spectrum can
/// move at the sixty reads a second the screen takes.
const OVERLAP: usize = 32;
/// How long the average takes to close 63% of the gap to a new level, in seconds of audio.
const SETTLE: f64 = 0.4;
/// How many samples wait for the analysis at most: more than a fifth of a second of stereo at
/// 96 kHz. A frame that does not fit is dropped.
const RING: usize = 1 << 16;
/// The quietest level reported, far below anything drawn.
const FLOOR: f64 = -200.;

/// The spectrum heard ahead of the equalizer, shared by every output like the equalizer itself.
#[derive(Clone, Default)]
pub struct Analyzer(Arc<Shared>);

#[derive(Default)]
struct Shared {
    /// How many watches are held. Nothing is sent or analysed while there are none.
    watchers: AtomicUsize,
    /// Bumped by every watch, after it has dropped what the lanes held, so each probe sends its
    /// format again.
    generation: AtomicUsize,
    /// One per probe, removed once its stage is gone.
    lanes: Mutex<Vec<Lane>>,
}

/// One frequency's level, in dBFS: a full-scale sine reads 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub frequency: f32,
    pub dbfs: f32,
}

/// Keeps the analyzer running while held. Dropping it stops the probes sending.
pub struct Watch(Analyzer);

/// The input side of one equalizer stage, on the audio thread.
pub(crate) struct Probe {
    feed: Feed,
    shared: Arc<Shared>,
    /// Whether the frame under way is sent, read once at its first sample.
    on: bool,
    /// The watch generation the format was last sent for.
    generation: usize,
}

/// What one probe sends, and the analysis of it.
struct Lane {
    listener: Listener,
    /// Set up for the format heard, once one has been.
    analysis: Option<Analysis>,
}

/// The transform and the running average for one format.
struct Analysis {
    channels: usize,
    size: usize,
    hop: usize,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// What turns a band's summed bin power into a sine's power: 4 / (size · Σ window²).
    scale: f64,
    /// The last `size` frames of each channel, as a ring written at `at`.
    history: Vec<Vec<f32>>,
    at: usize,
    filled: usize,
    /// Frames since the last transform.
    since: usize,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    /// Power per bin up to Nyquist, averaged over the channels.
    power: Vec<f64>,
    /// One per level below Nyquist.
    bands: Vec<Band>,
    /// The running average of each band's power, `None` before the first transform.
    average: Option<Vec<f64>>,
    /// The share of the gap to a new transform the average closes.
    blend: f64,
}

/// The bins one level sums, the two at its edges counted in part.
#[derive(Clone, Copy)]
struct Band {
    first: usize,
    last: usize,
    first_weight: f64,
    last_weight: f64,
}

impl Analyzer {
    /// A probe for one chain's input ahead of its equalizer, heard from the next watch on.
    pub(crate) fn probe(&self) -> Probe {
        let (feed, listener) = feed::channel(RING);
        let mut lanes = self.lanes();
        // Lanes whose stage has gone are dropped here, off the audio thread.
        lanes.retain(|lane| !lane.listener.abandoned());
        lanes.push(Lane {
            listener,
            analysis: None,
        });
        Probe {
            feed,
            shared: self.0.clone(),
            on: false,
            generation: self.0.generation.load(Ordering::Relaxed),
        }
    }

    /// Starts the analysis afresh and keeps it running until the watch is dropped. Whatever the
    /// probes sent before is dropped, and each starts again with its format.
    pub fn watch(&self) -> Watch {
        for lane in self.lanes().iter_mut() {
            lane.listener.forget();
            lane.analysis = None;
        }
        self.0.generation.fetch_add(1, Ordering::Release);
        self.0.watchers.fetch_add(1, Ordering::Relaxed);
        Watch(self.clone())
    }

    fn lanes(&self) -> MutexGuard<'_, Vec<Lane>> {
        self.0.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Watch {
    /// Analyses what the probes sent since the last read and returns the levels, or `None`
    /// until a window has filled. It runs transforms, so call it off the UI thread.
    pub fn read(&self) -> Option<Vec<Level>> {
        let mut lanes = self.0.lanes();
        lanes.retain_mut(|lane| {
            lane.hear();
            !lane.listener.abandoned()
        });
        let mut total: Vec<f64> = Vec::new();
        for average in lanes
            .iter()
            .filter_map(|lane| lane.analysis.as_ref()?.average.as_ref())
        {
            if total.len() < average.len() {
                total.resize(average.len(), 0.);
            }
            for (total, power) in total.iter_mut().zip(average) {
                *total += power;
            }
        }
        (!total.is_empty()).then(|| {
            total
                .iter()
                .enumerate()
                .map(|(index, power)| Level {
                    frequency: frequency(index) as f32,
                    dbfs: (10. * power.log10()).max(FLOOR) as f32,
                })
                .collect()
        })
    }
}

impl std::fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analyzer")
            .field("watchers", &self.0.watchers.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.0.0.watchers.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Probe {
    /// Declares the format of the frames that follow.
    pub(crate) fn format(&mut self, rate: u32, channels: u16) {
        self.feed.format(rate, channels);
    }

    /// Takes one sample, `channel` being its place in its frame. Unwatched, a frame costs two
    /// loads at its first sample and nothing after.
    #[inline]
    pub(crate) fn hear(&mut self, channel: u16, sample: f32) {
        if channel == 0 {
            self.on = self.shared.watchers.load(Ordering::Relaxed) > 0;
            let generation = self.shared.generation.load(Ordering::Acquire);
            if generation != self.generation {
                self.generation = generation;
                self.feed.reannounce();
            }
        }
        if self.on {
            self.feed.push(channel, sample);
        }
    }
}

impl Lane {
    /// Reads everything waiting, transforming as windows complete.
    fn hear(&mut self) {
        while let Some(heard) = self.listener.next() {
            match heard {
                Heard::Format(layout) => self.analysis = Analysis::new(layout),
                Heard::Frame(frame) => {
                    if let Some(analysis) = self.analysis.as_mut() {
                        analysis.push(frame);
                    }
                }
            }
        }
    }
}

impl Analysis {
    fn new(layout: Layout) -> Option<Self> {
        let channels = usize::from(layout.channels);
        let rate = f64::from(layout.rate);
        if channels == 0 || rate <= 0. {
            return None;
        }
        let size = ((rate * SPAN) as usize).next_power_of_two();
        let hop = size / OVERLAP;
        let window: Vec<f32> = (0..size)
            .map(|n| 0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / size as f64).cos() as f32)
            .collect();
        let energy: f64 = window.iter().map(|w| f64::from(*w).powi(2)).sum();
        let fft = FftPlanner::<f32>::new().plan_fft_forward(size);
        let scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        let bin = rate / size as f64;
        let nyquist = rate / 2.;
        let bands = (0..)
            .map(frequency)
            .take_while(|frequency| *frequency <= HIGH.min(nyquist) * (1. + 1e-9))
            .map(|frequency| Band::around(frequency, bin, size / 2))
            .collect();
        Some(Self {
            channels,
            size,
            hop,
            fft,
            window,
            scale: 4. / (size as f64 * energy),
            history: vec![vec![0.; size]; channels],
            at: 0,
            filled: 0,
            since: 0,
            buffer: vec![Complex32::default(); size],
            scratch,
            power: vec![0.; size / 2 + 1],
            bands,
            average: None,
            blend: 1. - (-(hop as f64 / rate) / SETTLE).exp(),
        })
    }

    fn push(&mut self, frame: &[f32]) {
        for (history, sample) in self.history.iter_mut().zip(frame) {
            history[self.at] = *sample;
        }
        self.at = (self.at + 1) % self.size;
        self.filled = (self.filled + 1).min(self.size);
        self.since += 1;
        if self.filled == self.size && self.since >= self.hop {
            self.since = 0;
            self.transform();
        }
    }

    /// Transforms the last window of every channel and moves the average towards it. Two
    /// channels share one complex transform, one as its real part and one as its imaginary,
    /// and the sum of their powers at bin k is half that of bins k and size − k together.
    fn transform(&mut self) {
        let size = self.size;
        self.power.iter_mut().for_each(|power| *power = 0.);
        for pair in self.history.chunks(2) {
            for (n, slot) in self.buffer.iter_mut().enumerate() {
                let index = (self.at + n) % size;
                let weight = self.window[n];
                let imaginary = pair.get(1).map_or(0., |channel| channel[index]);
                *slot = Complex32::new(pair[0][index] * weight, imaginary * weight);
            }
            self.fft
                .process_with_scratch(&mut self.buffer, &mut self.scratch);
            for (k, power) in self.power.iter_mut().enumerate() {
                let mirrored = self.buffer[(size - k) % size];
                *power +=
                    (f64::from(self.buffer[k].norm_sqr()) + f64::from(mirrored.norm_sqr())) / 2.;
            }
        }
        let scale = self.scale / self.channels as f64;
        let levels = self.bands.iter().map(|band| band.sum(&self.power) * scale);
        match self.average.as_mut() {
            Some(average) => {
                for (average, level) in average.iter_mut().zip(levels) {
                    *average += (level - *average) * self.blend;
                }
            }
            None => self.average = Some(levels.collect()),
        }
    }
}

impl Band {
    /// The bins within `SMOOTHING` of an octave around `frequency`, and never fewer than `LOBE` each
    /// side, bin k covering half a bin either side of its centre. `top` is the Nyquist bin.
    fn around(frequency: f64, bin: f64, top: usize) -> Self {
        let half = 2f64.powf(SMOOTHING / 2.);
        let low = (frequency / half).min(frequency - LOBE * bin) / bin;
        let high = (frequency * half).max(frequency + LOBE * bin) / bin;
        let low = low.max(0.);
        let high = high.min(top as f64);
        let first = (low + 0.5).floor() as usize;
        let last = ((high + 0.5).floor() as usize).min(top);
        // The share of a bin's width [k − ½, k + ½) inside [low, high).
        let share = |k: usize| {
            let k = k as f64;
            ((k + 0.5).min(high) - (k - 0.5).max(low)).clamp(0., 1.)
        };
        Self {
            first,
            last,
            first_weight: share(first),
            last_weight: share(last),
        }
    }

    fn sum(&self, power: &[f64]) -> f64 {
        if self.first == self.last {
            return power[self.first] * self.first_weight.min(self.last_weight);
        }
        let inner: f64 = power[self.first + 1..self.last].iter().sum();
        power[self.first] * self.first_weight + inner + power[self.last] * self.last_weight
    }
}

/// The frequency of level `index`.
fn frequency(index: usize) -> f64 {
    LOW * 2f64.powf(index as f64 / STEPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_reads_its_level_in_dbfs() {
        for (rate, channels) in [(44_100, 2), (48_000, 1), (96_000, 2)] {
            for tone in [31., 1_000., 15_000.] {
                let analyzer = Analyzer::default();
                let mut probe = analyzer.probe();
                let watch = analyzer.watch();
                probe.format(rate, channels);
                for n in 0..rate / 2 {
                    let phase = std::f64::consts::TAU * tone * f64::from(n) / f64::from(rate);
                    for channel in 0..channels {
                        probe.hear(channel, (0.1 * phase.sin()) as f32);
                    }
                }
                let levels = watch.read().expect("a window has filled");
                let nearest = levels
                    .iter()
                    .min_by(|a, b| {
                        let off = |level: &Level| (level.frequency as f64 / tone).ln().abs();
                        off(a).total_cmp(&off(b))
                    })
                    .expect("levels are read");
                assert!(
                    (nearest.dbfs + 20.).abs() < 0.05,
                    "{tone} Hz at {rate} Hz on {channels} channels read {} dBFS",
                    nearest.dbfs
                );
            }
        }
    }
}
