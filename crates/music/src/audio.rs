use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, SampleRate, Source};

use crate::effects::Effects;
use crate::equalizer::{Equalizer, FilterStage};
use crate::spectrum::{Analyzer, Probe, Spectrum, Tap};

pub const RAMP: Duration = Duration::from_millis(25);
/// The loudest sample an integer device takes: the largest `f32` below one, since one itself
/// overflows a 24-bit sample and wraps to the far end of the scale.
const FULL_SCALE: f32 = 1. - f32::EPSILON / 2.;
#[derive(Clone)]
pub struct Volume(Arc<AtomicU32>);

impl Volume {
    pub fn new(gain: f32) -> Self {
        Self(Arc::new(AtomicU32::new(gain.to_bits())))
    }

    pub fn set(&self, gain: f32) {
        self.0.store(gain.to_bits(), Ordering::Relaxed);
    }

    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The loudest sample any output sent its device since the meter was last read, after the
/// volume and before the clamp an integer device gets, so an over shows however the device
/// takes it. Every output shares one.
#[derive(Clone, Debug, Default)]
pub struct Meter(Arc<AtomicU32>);

impl Meter {
    /// The loudest absolute sample since the last call, as a linear amplitude, and a fresh
    /// start for the next one.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }

    /// Keeps `sample` if it is the loudest yet. A finite non-negative `f32` orders like its
    /// bits, so the maximum is kept on the bits; NaN would sort above infinity and is left out.
    fn record(&self, sample: f32) {
        let bits = sample.abs().to_bits();
        // A plain load first, since a new loudest sample is rare once the meter has warmed up.
        if sample.is_finite() && bits > self.0.load(Ordering::Relaxed) {
            self.0.fetch_max(bits, Ordering::Relaxed);
        }
    }
}

/// The rate and channel count of what one player is playing, published by each chunk as it
/// yields its first sample. The queue's own answer depends on how the previous chunk ended, so
/// the chain follows this instead. Zero means no chunk has played yet.
#[derive(Clone, Default)]
pub(crate) struct PublishedFormat(Arc<AtomicU64>);

impl PublishedFormat {
    pub(crate) fn publish(&self, rate: SampleRate, channels: ChannelCount) {
        let word = u64::from(rate.get()) << 16 | u64::from(channels.get());
        self.0.store(word, Ordering::Relaxed);
    }
}

/// Which channel of its frame each sample a chain stage pulls belongs to, counted from the
/// format the playing chunk published. A change of format starts a new frame.
pub(crate) struct Frames {
    format: PublishedFormat,
    /// The word last read, so a change is noticed on the sample that brings it.
    seen: u64,
    channel: u16,
}

/// Where the sample just pulled sits in its frame.
pub(crate) struct Place {
    pub channel: u16,
    /// Whether this sample brought a new format, which the stage takes up before using it.
    pub changed: bool,
}

impl Frames {
    pub(crate) fn new(format: PublishedFormat) -> Self {
        Self {
            format,
            seen: 0,
            channel: 0,
        }
    }

    /// Pulls the next sample from `input` and places it in its frame. The place is `None` until a
    /// chunk has published a format, and the stage passes those samples through untouched.
    pub(crate) fn pull(
        &mut self,
        input: &mut impl Iterator<Item = f32>,
    ) -> Option<(f32, Option<Place>)> {
        let sample = input.next()?;
        // The chunk publishes during the pull, so the word is read after it.
        let word = self.format.0.load(Ordering::Relaxed);
        let changed = word != self.seen;
        if changed {
            self.seen = word;
            self.channel = 0;
        }
        let channels = self.channels();
        if channels == 0 {
            return Some((sample, None));
        }

        let channel = self.channel;
        self.channel += 1;
        if self.channel >= channels {
            self.channel = 0;
        }
        Some((sample, Some(Place { channel, changed })))
    }

    pub(crate) fn rate(&self) -> u32 {
        (self.seen >> 16) as u32
    }

    pub(crate) fn channels(&self) -> u16 {
        self.seen as u16
    }
}

/// What sits between the queue and the device: the effects, the analyzer listening ahead of the
/// equalizer, then the equalizer and the volume ramp, with the spectrum tap and the meter
/// listening at the end. Every engine builds one and hands it to the output.
pub struct Chain {
    pub volume: Volume,
    /// Shared like `equalizer`, and run by the output ahead of everything else.
    pub effects: Effects,
    /// Hears the music ahead of the equalizer for the equalizer screen, shared like `meter`.
    pub analyzer: Analyzer,
    pub equalizer: Equalizer,
    pub spectrum: Spectrum,
    pub meter: Meter,
}

impl Chain {
    /// `source` run through the equalizer and then the volume ramp, with the spectrum tap at the
    /// end, ready for the mixer. Every chunk queued on `source` publishes into `format`, and
    /// `clamped` holds it within full scale, for a device whose integer samples would wrap.
    pub(crate) fn apply<S: Source>(
        &self,
        source: S,
        format: PublishedFormat,
        clamped: bool,
    ) -> Chained<S> {
        Chained::new(source, self, format, clamped)
    }
}

/// A player's queue run through its `Chain`: the probe hears it, the equalizer filters it and
/// the volume ramps it, then the tap and the meter hear it, and an integer device gets it held
/// within full scale. One frame count places every sample for all of them.
pub(crate) struct Chained<I> {
    input: I,
    frames: Frames,
    probe: Probe,
    equalizer: FilterStage,
    volume: Volume,
    tap: Tap,
    /// Whether the tap hears the samples from before the gain. It is read once per frame so
    /// every channel of a frame agrees.
    unscaled: bool,
    meter: Meter,
    /// Whether the output is held within full scale, for a device whose integer samples would
    /// wrap. The tap and the meter hear the sample before the clamp, so they still see the over.
    clamped: bool,

    current: f32,
    target: f32,
    step: f32,
    frames_left: u32,
    ramp_frames: u32,
}

impl<I: Source> Chained<I> {
    fn new(input: I, chain: &Chain, format: PublishedFormat, clamped: bool) -> Self {
        let gain = chain.volume.get();
        Self {
            input,
            frames: Frames::new(format),
            probe: chain.analyzer.probe(),
            equalizer: FilterStage::new(chain.equalizer.clone()),
            volume: chain.volume.clone(),
            tap: chain.spectrum.attach(),
            unscaled: false,
            meter: chain.meter.clone(),
            clamped,
            current: gain,
            target: gain,
            step: 0.0,
            frames_left: 0,
            ramp_frames: 1,
        }
    }

    /// Takes up a new format. Every stage learns it, and the ramp keeps its length in time.
    fn resync(&mut self) {
        let rate = self.frames.rate();
        let channels = self.frames.channels();
        self.equalizer.set_format(rate, channels);
        self.probe.format(rate, channels);
        self.tap.format(rate, channels);
        self.ramp_frames = (RAMP.as_secs_f64() * rate as f64).round().max(1.0) as u32;
        self.frames_left = self.frames_left.min(self.ramp_frames);
    }
}

impl<I: Source> Iterator for Chained<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let (sample, place) = self.frames.pull(&mut self.input)?;
        let Some(place) = place else {
            return Some(sample);
        };

        if place.changed {
            self.resync();
        }
        self.probe.hear(place.channel, sample);
        if place.channel == 0 {
            self.equalizer.begin();
            self.unscaled = self.tap.absolute();
            let requested = self.volume.get().max(0.0);

            if requested.to_bits() != self.target.to_bits() {
                self.target = requested;
                self.frames_left = self.ramp_frames;
                self.step = (self.target - self.current) / self.ramp_frames as f32;
            }

            if self.frames_left > 0 {
                self.current += self.step;
                self.frames_left -= 1;

                if self.frames_left == 0 {
                    self.current = self.target;
                }
            }
        }

        let equalized = self.equalizer.process(usize::from(place.channel), sample);
        let output = equalized * self.current;
        let heard = match self.unscaled {
            true => equalized,
            false => output,
        };
        self.tap.push(place.channel, heard);
        self.meter.record(output);

        Some(match self.clamped {
            true => output.clamp(-1., FULL_SCALE),
            false => output,
        })
    }
}

impl<I: Source> Source for Chained<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position)?;
        self.equalizer.reset();
        Ok(())
    }
}

pub struct Trimmed<I> {
    input: I,
    head: u64,
    body: Option<u64>,
    emitted: u64,
    primed: bool,
    rate: u64,
    channels: u64,
}

impl<I: Source> Trimmed<I> {
    pub fn new(input: I, skip: Duration, take: Option<Duration>) -> Self {
        let rate = input.sample_rate().get() as u64;
        let channels = input.channels().get() as u64;

        Self {
            head: whole_frames(skip, rate, channels),
            body: take.map(|take| whole_frames(take, rate, channels)),
            emitted: 0,
            primed: false,
            rate,
            channels,
            input,
        }
    }

    fn offset(&self) -> Duration {
        self.playing_time(self.head)
    }

    /// How long `samples` play for.
    fn playing_time(&self, samples: u64) -> Duration {
        Duration::from_secs_f64(samples as f64 / (self.rate * self.channels) as f64)
    }
}

impl<I: Source> Iterator for Trimmed<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.primed {
            self.primed = true;
            for _ in 0..self.head {
                self.input.next()?;
            }
        }
        if self.body.is_some_and(|body| self.emitted >= body) {
            return None;
        }

        let sample = self.input.next()?;
        self.emitted += 1;
        Some(sample)
    }
}

impl<I: Source> Source for Trimmed<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        match self.body {
            Some(body) => Some(self.playing_time(body)),
            None => self
                .input
                .total_duration()
                .map(|whole| whole.saturating_sub(self.offset())),
        }
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position + self.offset())?;
        self.primed = true;
        self.emitted = whole_frames(position, self.rate, self.channels);
        Ok(())
    }
}

/// The samples `span` covers at `rate` frames a second, rounded to whole frames, so a cut never
/// lands between the channels of one frame and swaps them for the rest of the track.
fn whole_frames(span: Duration, rate: u64, channels: u64) -> u64 {
    (span.as_secs_f64() * rate as f64).round() as u64 * channels
}
