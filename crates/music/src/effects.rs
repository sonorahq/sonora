//! Speed, a low-pass filter and reverb, applied to the interleaved PCM ahead of the equalizer.
//! `Effects` is the handle the UI and the settings hold, `Effected` is the rodio `Source` that
//! reads it on the audio thread. Speed changes how fast the queue is drawn on, so the samples
//! no longer line up with the formats the chunks publish: `Effected` brings every frame to the
//! device's channel count as it reads it, and hands the rest of the chain one format for good.

use std::collections::VecDeque;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError;

use crate::audio::{Frames, PublishedFormat};
use crate::equalizer::{Coefficients, Kind, MAX_CHANNELS, Shape, State, Width};

/// The slowest playback runs, as a share of normal speed.
pub const MIN_SPEED: f32 = 0.5;
/// The fastest playback runs, as a share of normal speed.
pub const MAX_SPEED: f32 = 2.;
/// The lowest cutoff the low-pass accepts, in hertz.
pub const MIN_CUTOFF: f32 = 200.;
/// The highest cutoff the low-pass accepts, in hertz. At this cutoff the filter is open.
pub const MAX_CUTOFF: f32 = 20_000.;

/// How long a cutoff or reverb change takes to settle, so a dragged slider never clicks.
const RAMP: Duration = Duration::from_millis(40);
/// Speed glides slower than the rest, like a tape winding up rather than jumping.
const SPEED_RAMP: Duration = Duration::from_millis(150);
/// The highest a drawn cutoff goes, as a share of the rate. Asked for more, the filter opens.
const CUTOFF_CEILING: f32 = 0.45;
/// The Q of the two stages that make a fourth order Butterworth low-pass.
const BUTTERWORTH: [f64; 2] = [0.541_196_1, 1.306_563];

/// Half a stretch segment: what one stretch step writes, and how long its crossfade runs.
const STRETCH_HOP: Duration = Duration::from_millis(30);
/// How far either side of where it should be a stretch step looks for the best match.
const STRETCH_SEEK: Duration = Duration::from_millis(10);
/// The stride, in frames, of the coarse search a stretch step refines.
const COARSE: usize = 4;

/// Freeverb's comb and allpass lengths, tuned in frames at `TUNED_RATE`.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// How much longer every delay runs on odd channels, so the reverb spreads in stereo.
const SPREAD: usize = 23;
const TUNED_RATE: f32 = 44_100.;
const REVERB_INPUT: f32 = 0.015;
const DAMPING: f32 = 0.2;
const ROOM_SCALE: f32 = 0.28;
const ROOM_OFFSET: f32 = 0.7;
const ALLPASS_FEEDBACK: f32 = 0.5;
/// Values this small in a feedback loop are flushed to zero, since denormals stall the CPU.
const DENORMAL: f32 = 1e-20;

/// Every effect's value. These are what the user dialled in, kept while effects are off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// How fast playback runs, as a share of normal speed.
    pub speed: f32,
    /// Whether a speed change leaves the pitch alone, rather than shifting it like a tape.
    pub keep_pitch: bool,
    /// The low-pass cutoff in hertz. `MAX_CUTOFF` leaves the filter open.
    pub cutoff: f32,
    /// How much reverb is mixed in, from none at zero to fully wet at one.
    pub reverb: f32,
    /// How large the reverb's room is, from zero to one. Larger rings longer.
    pub room: f32,
}

impl Settings {
    /// Effects that leave the sound as it is.
    pub const NEUTRAL: Self = Self {
        speed: 1.,
        keep_pitch: false,
        cutoff: MAX_CUTOFF,
        reverb: 0.,
        room: 0.5,
    };

    /// Every value pulled into its range, with anything that is not a number reset.
    pub fn clamped(self) -> Self {
        let within = |value: f32, low: f32, high: f32, fallback: f32| match value.is_finite() {
            true => value.clamp(low, high),
            false => fallback,
        };
        let neutral = Self::NEUTRAL;
        Self {
            speed: within(self.speed, MIN_SPEED, MAX_SPEED, neutral.speed),
            keep_pitch: self.keep_pitch,
            cutoff: within(self.cutoff, MIN_CUTOFF, MAX_CUTOFF, neutral.cutoff),
            reverb: within(self.reverb, 0., 1., neutral.reverb),
            room: within(self.room, 0., 1., neutral.room),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

/// A named set of effects that a user can pick instead of dialling each one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Off,
    SlowedReverb,
    Nightcore,
    LoFi,
    Muffled,
    Hall,
}

impl Preset {
    pub const ALL: [Self; 6] = [
        Self::Off,
        Self::SlowedReverb,
        Self::Nightcore,
        Self::LoFi,
        Self::Muffled,
        Self::Hall,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::SlowedReverb => "slowed-reverb",
            Self::Nightcore => "nightcore",
            Self::LoFi => "lo-fi",
            Self::Muffled => "muffled",
            Self::Hall => "hall",
        }
    }

    /// The preset's values. Presets that leave speed alone keep `current`'s pitch choice, and
    /// the two that change it shift pitch like a tape, since that is the sound they are named
    /// for.
    pub fn settings(self, current: Settings) -> Settings {
        let keep_pitch = current.keep_pitch;
        let neutral = Settings {
            keep_pitch,
            ..Settings::NEUTRAL
        };
        match self {
            Self::Off => neutral,
            Self::SlowedReverb => Settings {
                speed: 0.85,
                keep_pitch: false,
                reverb: 0.35,
                room: 0.85,
                ..neutral
            },
            Self::Nightcore => Settings {
                speed: 1.25,
                keep_pitch: false,
                ..neutral
            },
            Self::LoFi => Settings {
                cutoff: 3_500.,
                reverb: 0.15,
                room: 0.4,
                ..neutral
            },
            Self::Muffled => Settings {
                cutoff: 400.,
                reverb: 0.2,
                room: 0.6,
                ..neutral
            },
            Self::Hall => Settings {
                reverb: 0.5,
                room: 0.9,
                ..neutral
            },
        }
    }

    /// The preset these values spell, if they match one closely. The pitch choice only counts
    /// when the speed differs from normal, since it changes nothing otherwise.
    pub fn matching(settings: &Settings) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| {
            let want = preset.settings(*settings);
            (want.speed - settings.speed).abs() < 0.005
                && (want.speed == 1. || want.keep_pitch == settings.keep_pitch)
                && (want.cutoff - settings.cutoff).abs() < 1.
                && (want.reverb - settings.reverb).abs() < 0.005
                && (want.room - settings.room).abs() < 0.005
        })
    }
}

struct Shared {
    enabled: AtomicBool,
    speed: AtomicU32,
    keep_pitch: AtomicBool,
    cutoff: AtomicU32,
    reverb: AtomicU32,
    room: AtomicU32,
    /// Bumped on every write so an output reads the values only when they moved.
    generation: AtomicU32,
}

/// The effect settings as the audio thread sees them: lock free, read once per frame.
/// Cloning shares the same values, so every output an engine opens follows one setting.
#[derive(Clone)]
pub struct Effects(Arc<Shared>);

impl Effects {
    pub fn new(enabled: bool, settings: Settings) -> Self {
        let settings = settings.clamped();
        Self(Arc::new(Shared {
            enabled: AtomicBool::new(enabled),
            speed: AtomicU32::new(settings.speed.to_bits()),
            keep_pitch: AtomicBool::new(settings.keep_pitch),
            cutoff: AtomicU32::new(settings.cutoff.to_bits()),
            reverb: AtomicU32::new(settings.reverb.to_bits()),
            room: AtomicU32::new(settings.room.to_bits()),
            generation: AtomicU32::new(0),
        }))
    }

    pub fn enabled(&self) -> bool {
        self.0.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.store(enabled, Ordering::Relaxed);
        self.0.generation.fetch_add(1, Ordering::Release);
    }

    pub fn settings(&self) -> Settings {
        let load = |slot: &AtomicU32| f32::from_bits(slot.load(Ordering::Relaxed));
        Settings {
            speed: load(&self.0.speed),
            keep_pitch: self.0.keep_pitch.load(Ordering::Relaxed),
            cutoff: load(&self.0.cutoff),
            reverb: load(&self.0.reverb),
            room: load(&self.0.room),
        }
    }

    pub fn set_settings(&self, settings: Settings) {
        let settings = settings.clamped();
        let store = |slot: &AtomicU32, value: f32| slot.store(value.to_bits(), Ordering::Relaxed);
        store(&self.0.speed, settings.speed);
        self.0
            .keep_pitch
            .store(settings.keep_pitch, Ordering::Relaxed);
        store(&self.0.cutoff, settings.cutoff);
        store(&self.0.reverb, settings.reverb);
        store(&self.0.room, settings.room);
        self.0.generation.fetch_add(1, Ordering::Release);
    }

    /// How fast the track moves against the wall clock: the speed while effects are on, and
    /// normal speed while they are off.
    pub fn speed(&self) -> f32 {
        self.effective().speed
    }

    /// What the output should apply right now: neutral while disabled.
    fn effective(&self) -> Settings {
        let settings = self.settings();
        match self.enabled() {
            true => settings,
            false => Settings {
                keep_pitch: settings.keep_pitch,
                ..Settings::NEUTRAL
            },
        }
    }

    fn generation(&self) -> u32 {
        self.0.generation.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for Effects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effects")
            .field("enabled", &self.enabled())
            .field("settings", &self.settings())
            .finish()
    }
}

impl Default for Effects {
    fn default() -> Self {
        Self::new(false, Settings::NEUTRAL)
    }
}

/// One value gliding toward its target in equal steps.
#[derive(Clone, Copy)]
struct Glide {
    current: f32,
    target: f32,
    step: f32,
    left: u32,
}

impl Glide {
    fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.,
            left: 0,
        }
    }

    /// Heads for `target` over `frames` frames, from wherever the glide is now.
    fn aim(&mut self, target: f32, frames: u32) {
        if target == self.target {
            return;
        }
        self.target = target;
        self.left = frames.max(1);
        self.step = (target - self.current) / self.left as f32;
    }

    /// Moves one frame on. True while the value is still changing.
    fn tick(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        match self.left {
            0 => self.current = self.target,
            _ => self.current += self.step,
        }
        true
    }
}

/// The input frames the speed stage may still read, interleaved, from frame `start` on. Each
/// frame is brought to the output's channel count as it is read, placed by the format its chunk
/// published, so what is held has one shape whatever played.
struct Feed<I> {
    input: I,
    frames: Frames,
    /// The input frame being gathered, one sample per channel.
    gathering: [f32; MAX_CHANNELS],
    samples: VecDeque<f32>,
    start: u64,
    channels: usize,
    ended: bool,
}

impl<I: Source> Feed<I> {
    /// The frame after the last one held.
    fn end(&self) -> u64 {
        self.start + (self.samples.len() / self.channels) as u64
    }

    /// Reads until every frame before `frame` is held. False when the input ran out first.
    fn reach(&mut self, frame: u64) -> bool {
        while self.end() < frame {
            if self.ended || !self.read() {
                self.ended = true;
                return false;
            }
        }
        true
    }

    /// Reads one input frame and keeps it at the output's channel count. False when the input
    /// ended.
    fn read(&mut self) -> bool {
        loop {
            let Some((sample, place)) = self.frames.pull(&mut self.input) else {
                return false;
            };
            // Before any chunk publishes a format the queue plays silence, one sample a frame.
            let Some(place) = place else {
                self.samples
                    .extend(std::iter::repeat_n(sample, self.channels));
                return true;
            };
            let channel = usize::from(place.channel);
            if let Some(slot) = self.gathering.get_mut(channel) {
                *slot = sample;
            }
            let incoming = usize::from(self.frames.channels());
            if channel + 1 == incoming {
                self.conform(incoming.min(MAX_CHANNELS));
                return true;
            }
        }
    }

    /// Keeps the gathered frame of `incoming` channels: as it is when the counts match, averaged
    /// down to mono, and otherwise with each output channel taking an input one in turn.
    fn conform(&mut self, incoming: usize) {
        let frame = &self.gathering[..incoming];
        match (incoming, self.channels) {
            (from, to) if from == to => self.samples.extend(frame),
            (_, 1) => self
                .samples
                .push_back(frame.iter().sum::<f32>() / incoming as f32),
            (from, to) => self
                .samples
                .extend((0..to).map(|channel| frame[channel % from])),
        }
    }

    /// One channel of one frame, silent outside what is held.
    #[inline]
    fn sample(&self, frame: u64, channel: usize) -> f32 {
        frame
            .checked_sub(self.start)
            .and_then(|offset| self.samples.get(offset as usize * self.channels + channel))
            .copied()
            .unwrap_or(0.)
    }

    /// Every channel of one frame summed, for matching shapes regardless of the stereo image.
    fn mono(&self, frame: u64) -> f32 {
        (0..self.channels)
            .map(|channel| self.sample(frame, channel))
            .sum()
    }

    /// Lets go of every frame before `frame`.
    fn release(&mut self, frame: u64) {
        let held = (self.samples.len() / self.channels) as u64;
        let dropped = frame.saturating_sub(self.start).min(held);
        self.samples.drain(..dropped as usize * self.channels);
        self.start += dropped;
    }

    /// Forgets everything held, after a seek moved the input.
    fn clear(&mut self) {
        self.samples.clear();
        self.start = 0;
        self.ended = false;
    }
}

/// Where the speed stage reads from.
#[derive(Clone, Copy)]
enum Mode {
    /// Reads at a fractional frame, stepping by the speed, so the pitch follows it. At normal
    /// speed the position stays whole and frames pass through untouched.
    Tape { position: f64 },
    /// Keeps the pitch with WSOLA: each step crossfades out of the segment it was in, starting
    /// at `next`, into the input segment near `nominal` that best continues it. `nominal`
    /// moves by the speed while the output moves by a hop, which stretches or squeezes time.
    Stretch { next: u64, nominal: f64 },
}

/// The speed stage. It changes over between tape and stretch at a segment boundary: the
/// stretch picks up from the frame tape would have read next, and tape picks up from the
/// frame that continues the last stretched segment, so neither side ever jumps.
struct Varispeed {
    mode: Mode,
    channels: usize,
    hop: usize,
    seek: usize,
    /// The weight of the outgoing segment at each frame of a crossfade; the incoming one gets
    /// the rest, so the two always sum to one.
    fall: Vec<f32>,
    /// One stretch step's output, interleaved, and how many of its frames are out.
    block: Vec<f32>,
    block_at: usize,
    /// Mono scratch: the segment to continue, and the stretch of input searched for it.
    wanted: Vec<f32>,
    window: Vec<f32>,
}

impl Varispeed {
    fn new(channels: usize, rate: u32) -> Self {
        let frames = |span: Duration| (span.as_secs_f64() * f64::from(rate)).round() as usize;
        let hop = frames(STRETCH_HOP).max(COARSE);
        let seek = frames(STRETCH_SEEK).max(COARSE);
        let fall = (0..hop)
            .map(|frame| {
                let phase = (frame as f32 + 0.5) / hop as f32;
                (phase * std::f32::consts::FRAC_PI_2).cos().powi(2)
            })
            .collect();

        Self {
            mode: Mode::Tape { position: 0. },
            channels,
            hop,
            seek,
            fall,
            block: vec![0.; hop * channels],
            block_at: hop,
            wanted: vec![0.; hop],
            window: vec![0.; 2 * seek + 1 + hop],
        }
    }

    /// Writes the next output frame into `frame`. False once the input has ended.
    fn frame<I: Source>(
        &mut self,
        feed: &mut Feed<I>,
        speed: f32,
        keep_pitch: bool,
        frame: &mut [f32],
    ) -> bool {
        let stretching = keep_pitch && speed != 1.;
        loop {
            match self.mode {
                Mode::Stretch { next, .. } => {
                    if self.block_at < self.hop {
                        let at = self.block_at * self.channels;
                        frame.copy_from_slice(&self.block[at..at + self.channels]);
                        self.block_at += 1;
                        return true;
                    }
                    if !stretching {
                        self.mode = Mode::Tape {
                            position: next as f64,
                        };
                        continue;
                    }
                    if !self.step(feed, speed) {
                        return false;
                    }
                }
                Mode::Tape { position } => {
                    if stretching {
                        let next = position.round() as u64;
                        self.mode = Mode::Stretch {
                            next,
                            nominal: next as f64,
                        };
                        self.block_at = self.hop;
                        continue;
                    }
                    return self.tape(feed, position, speed, frame);
                }
            }
        }
    }

    /// One frame read at a fractional position, interpolated through four neighbours.
    fn tape<I: Source>(
        &mut self,
        feed: &mut Feed<I>,
        position: f64,
        speed: f32,
        frame: &mut [f32],
    ) -> bool {
        let whole = position.floor();
        let at = whole as u64;
        let fraction = (position - whole) as f32;
        // Past the input's end the missing neighbours read as silence, so its last frames still
        // come out.
        if !feed.reach(at + 3) && at >= feed.end() {
            return false;
        }

        for (channel, out) in frame.iter_mut().enumerate() {
            *out = match fraction == 0. {
                true => feed.sample(at, channel),
                false => hermite(
                    feed.sample(at.saturating_sub(1), channel),
                    feed.sample(at, channel),
                    feed.sample(at + 1, channel),
                    feed.sample(at + 2, channel),
                    fraction,
                ),
            };
        }
        feed.release(at.saturating_sub(1));

        let next = position + f64::from(speed);
        // At normal speed the position is kept whole, so frames pass through untouched.
        let position = match speed == 1. {
            true => next.round(),
            false => next,
        };
        self.mode = Mode::Tape { position };
        true
    }

    /// One stretch step: finds the input segment near `nominal` shaped most like the one that
    /// would follow the segment in play, crossfades into it over a hop, and moves on.
    fn step<I: Source>(&mut self, feed: &mut Feed<I>, speed: f32) -> bool {
        let Mode::Stretch { next, nominal } = self.mode else {
            return false;
        };
        let (hop, seek) = (self.hop as u64, self.seek as u64);
        let centre = nominal.round() as u64;
        let low = centre.saturating_sub(seek).max(feed.start);
        let high = (centre + seek).max(low);
        if !feed.reach((next + hop).max(high + hop)) {
            return false;
        }

        for (offset, slot) in self.wanted.iter_mut().enumerate() {
            *slot = feed.mono(next + offset as u64);
        }
        let span = (high - low + hop) as usize;
        for (offset, slot) in self.window[..span].iter_mut().enumerate() {
            *slot = feed.mono(low + offset as u64);
        }
        let pick = low + self.best((high - low) as usize) as u64;

        for (offset, fall) in self.fall.iter().enumerate() {
            let frame = offset as u64;
            for channel in 0..self.channels {
                let outgoing = feed.sample(next + frame, channel);
                let incoming = feed.sample(pick + frame, channel);
                self.block[offset * self.channels + channel] =
                    outgoing * fall + incoming * (1. - fall);
            }
        }
        self.block_at = 0;

        let nominal = nominal + hop as f64 * f64::from(speed);
        let next = pick + hop;
        self.mode = Mode::Stretch { next, nominal };
        let keep = (nominal.round() as u64).saturating_sub(seek).min(next);
        feed.release(keep);
        true
    }

    /// The offset into `window`, up to `last`, whose segment correlates best with `wanted`:
    /// a coarse pass on every `COARSE`th offset and sample, then a fine pass around the winner.
    fn best(&self, last: usize) -> usize {
        let score = |offset: usize, stride: usize| {
            let segment = &self.window[offset..offset + self.hop];
            let (mut dot, mut energy) = (0f32, f32::EPSILON);
            for (sample, wanted) in segment.iter().zip(&self.wanted).step_by(stride) {
                dot += sample * wanted;
                energy += sample * sample;
            }
            dot / energy.sqrt()
        };
        let winner = |range: std::iter::StepBy<std::ops::RangeInclusive<usize>>, stride| {
            range
                .map(|offset| (offset, score(offset, stride)))
                .fold((0, f32::NEG_INFINITY), |best, candidate| {
                    match candidate.1 > best.1 {
                        true => candidate,
                        false => best,
                    }
                })
                .0
        };

        let coarse = winner((0..=last).step_by(COARSE), COARSE);
        let from = coarse.saturating_sub(COARSE - 1);
        let to = (coarse + COARSE - 1).min(last);
        winner((from..=to).step_by(1), 2)
    }

    /// Starts over at the input's first frame, after a seek.
    fn reset(&mut self) {
        self.mode = Mode::Tape { position: 0. };
        self.block_at = self.hop;
    }
}

/// A fourth order Butterworth low-pass, two of the equalizer's state variable filters in a row,
/// whose cutoff glides on a log scale. While the cutoff rests at the top it is skipped whole, so
/// an open filter costs nothing and changes nothing.
struct LowPass {
    rate: f64,
    /// The cutoff as a base two logarithm, so a glide moves evenly through the octaves.
    cutoff: Glide,
    coefficients: [Coefficients; 2],
    /// Filter state indexed by channel, then stage.
    states: Vec<[State; 2]>,
    active: bool,
    /// Set when the filter switches in, so the next frame settles the lines on itself.
    prime: bool,
}

impl LowPass {
    fn new(cutoff: f32, channels: usize, rate: u32) -> Self {
        let mut this = Self {
            rate: f64::from(rate),
            cutoff: Glide::new(cutoff.log2()),
            coefficients: [Coefficients::IDENTITY; 2],
            states: vec![[State::default(); 2]; channels],
            active: false,
            prime: false,
        };
        this.recompute();
        this
    }

    fn aim(&mut self, cutoff: f32, frames: u32) {
        self.cutoff.aim(cutoff.log2(), frames);
    }

    fn tick(&mut self) {
        if self.cutoff.tick() {
            self.recompute();
        }
    }

    fn recompute(&mut self) {
        let ceiling = self.rate * f64::from(CUTOFF_CEILING);
        let cutoff = f64::from(self.cutoff.current.exp2());
        // A hertz of slack, since the round trip through the logarithm lands just short of
        // `MAX_CUTOFF` when the filter should be open.
        let active = cutoff < f64::from(MAX_CUTOFF) - 1. && cutoff < ceiling;
        // What the state held belongs to before the filter opened.
        self.prime |= active && !self.active;
        self.active = active;
        if active {
            for (coefficients, q) in self.coefficients.iter_mut().zip(BUTTERWORTH) {
                let shape = Shape {
                    kind: Kind::LowPass,
                    frequency: cutoff,
                    gain: 0.,
                    width: Width::Q(q),
                };
                *coefficients = Coefficients::of(&shape, self.rate);
            }
        }
    }

    fn run(&mut self, frame: &mut [f32]) {
        if !self.active {
            return;
        }
        if std::mem::take(&mut self.prime) {
            // Settled on a steady `x`, a low-pass passes it whole: the band state rests at zero
            // and the low one at `x`, so the filter picks up from the signal, not from silence.
            for (sample, states) in frame.iter().zip(self.states.iter_mut()) {
                *states = [State {
                    ic1: 0.,
                    ic2: f64::from(*sample),
                }; 2];
            }
        }
        for (sample, states) in frame.iter_mut().zip(self.states.iter_mut()) {
            let filtered = self
                .coefficients
                .iter()
                .zip(states.iter_mut())
                .fold(f64::from(*sample), |x, (coefficients, state)| {
                    state.tick(coefficients, x)
                });
            *sample = filtered as f32;
        }
    }

    fn reset(&mut self) {
        self.states.fill([State::default(); 2]);
    }
}

/// One of Freeverb's lowpass feedback combs.
struct Comb {
    buffer: Vec<f32>,
    at: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn run(&mut self, x: f32, feedback: f32) -> f32 {
        let out = self.buffer[self.at];
        self.store = flush(out * (1. - DAMPING) + self.store * DAMPING);
        self.buffer[self.at] = x + self.store * feedback;
        self.at = (self.at + 1) % self.buffer.len();
        out
    }
}

/// One of Freeverb's allpass diffusers.
struct Allpass {
    buffer: Vec<f32>,
    at: usize,
}

impl Allpass {
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let held = self.buffer[self.at];
        self.buffer[self.at] = flush(x + held * ALLPASS_FEEDBACK);
        self.at = (self.at + 1) % self.buffer.len();
        held - x
    }
}

/// The combs and allpasses one output channel rings through.
struct Lane {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

/// A Freeverb reverb fed with every channel summed, each channel ringing through its own
/// slightly longer or shorter delays. It is skipped while no reverb is mixed in, and starts
/// from silence when it comes back, so nothing from before rings out.
struct Reverb {
    lanes: Vec<Lane>,
    mix: Glide,
    room: Glide,
    active: bool,
}

impl Reverb {
    fn new(mix: f32, room: f32, channels: usize, rate: u32) -> Self {
        let scale = rate as f32 / TUNED_RATE;
        let frames = |tuned: usize, channel: usize| {
            let spread = (channel % 2) * SPREAD;
            (((tuned + spread) as f32 * scale).round() as usize).max(1)
        };
        let lanes = (0..channels)
            .map(|channel| Lane {
                combs: COMBS
                    .iter()
                    .map(|&tuned| Comb {
                        buffer: vec![0.; frames(tuned, channel)],
                        at: 0,
                        store: 0.,
                    })
                    .collect(),
                allpasses: ALLPASSES
                    .iter()
                    .map(|&tuned| Allpass {
                        buffer: vec![0.; frames(tuned, channel)],
                        at: 0,
                    })
                    .collect(),
            })
            .collect();

        Self {
            lanes,
            mix: Glide::new(mix),
            room: Glide::new(room),
            active: mix > 0.,
        }
    }

    fn aim(&mut self, mix: f32, room: f32, frames: u32) {
        self.mix.aim(mix, frames);
        self.room.aim(room, frames);
    }

    fn tick(&mut self) {
        self.mix.tick();
        self.room.tick();
        let active = self.mix.current > 0.;
        if active && !self.active {
            self.reset();
        }
        self.active = active;
    }

    fn run(&mut self, frame: &mut [f32]) {
        if !self.active {
            return;
        }
        let input = frame.iter().sum::<f32>() * REVERB_INPUT;
        let feedback = self.room.current * ROOM_SCALE + ROOM_OFFSET;
        let mix = self.mix.current;
        let dry = 1. - mix / 2.;

        for (sample, lane) in frame.iter_mut().zip(self.lanes.iter_mut()) {
            let mut wet = lane
                .combs
                .iter_mut()
                .map(|comb| comb.run(input, feedback))
                .sum::<f32>();
            for allpass in &mut lane.allpasses {
                wet = allpass.run(wet);
            }
            *sample = *sample * dry + wet * mix;
        }
    }

    fn reset(&mut self) {
        for lane in &mut self.lanes {
            for comb in &mut lane.combs {
                comb.buffer.fill(0.);
                comb.store = 0.;
            }
            for allpass in &mut lane.allpasses {
                allpass.buffer.fill(0.);
            }
        }
    }
}

/// Runs the effects over a player's queue: speed first, then the low-pass, then the reverb, out
/// at `channels` and `rate` for good. Every value glides toward the handle's, so a dragged
/// slider never clicks, and while effects are off frames pass through untouched.
pub struct Effected<I> {
    feed: Feed<I>,
    effects: Effects,
    seen: u32,

    speed: Glide,
    keep_pitch: bool,
    varispeed: Varispeed,
    low_pass: LowPass,
    reverb: Reverb,

    ramp_frames: u32,
    speed_frames: u32,
    /// The frame being handed out, and how many of its samples are out.
    frame: Vec<f32>,
    at: usize,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
}

impl<I: Source> Effected<I> {
    /// `format` is where the chunks queued on `input` publish theirs. `rate` has to be the rate
    /// they are resampled to already, since only the channel count is brought into line here.
    pub(crate) fn new(
        input: I,
        format: PublishedFormat,
        effects: Effects,
        rate: NonZero<u32>,
        channels: NonZero<u16>,
    ) -> Self {
        let count = usize::from(channels.get());
        let frames = |span: Duration| (span.as_secs_f64() * f64::from(rate.get())).round() as u32;
        let settings = effects.effective();
        let seen = effects.generation();

        Self {
            feed: Feed {
                input,
                frames: Frames::new(format),
                gathering: [0.; MAX_CHANNELS],
                samples: VecDeque::new(),
                start: 0,
                channels: count,
                ended: false,
            },
            seen,
            speed: Glide::new(settings.speed),
            keep_pitch: settings.keep_pitch,
            varispeed: Varispeed::new(count, rate.get()),
            low_pass: LowPass::new(settings.cutoff, count, rate.get()),
            reverb: Reverb::new(settings.reverb, settings.room, count, rate.get()),
            effects,
            ramp_frames: frames(RAMP).max(1),
            speed_frames: frames(SPEED_RAMP).max(1),
            frame: vec![0.; count],
            at: count,
            channels,
            rate,
        }
    }

    /// Once per frame: picks up new values and moves every glide one step toward them.
    fn advance(&mut self) {
        let generation = self.effects.generation();
        if generation != self.seen {
            self.seen = generation;
            let settings = self.effects.effective();
            self.speed.aim(settings.speed, self.speed_frames);
            self.keep_pitch = settings.keep_pitch;
            self.low_pass.aim(settings.cutoff, self.ramp_frames);
            self.reverb
                .aim(settings.reverb, settings.room, self.ramp_frames);
        }
        self.speed.tick();
        self.low_pass.tick();
        self.reverb.tick();
    }
}

impl<I: Source> Iterator for Effected<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.at == self.frame.len() {
            self.advance();
            let made = self.varispeed.frame(
                &mut self.feed,
                self.speed.current,
                self.keep_pitch,
                &mut self.frame,
            );
            if !made {
                return None;
            }
            self.low_pass.run(&mut self.frame);
            self.reverb.run(&mut self.frame);
            self.at = 0;
        }

        let sample = self.frame[self.at];
        self.at += 1;
        Some(sample)
    }
}

impl<I: Source> Source for Effected<I> {
    /// One span for good: the format never changes, which is what lets speed draw on the
    /// input at its own pace.
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZero<u16> {
        self.channels
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.feed.input.try_seek(position)?;
        self.feed.clear();
        self.varispeed.reset();
        self.low_pass.reset();
        self.reverb.reset();
        self.at = self.frame.len();
        Ok(())
    }
}

/// Catmull-Rom interpolation between `y1` and `y2`, `t` of the way across.
#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2. * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

#[inline]
fn flush(value: f32) -> f32 {
    match value.abs() < DENORMAL {
        true => 0.,
        false => value,
    }
}
