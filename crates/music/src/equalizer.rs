//! A ten band graphic equalizer applied to the interleaved PCM just before the volume ramp.
//! `Equalizer` is the handle the UI and the settings hold, `Equalized` is the rodio `Source`
//! that reads it on the audio thread. One handle can feed several outputs at once, since each
//! output keeps its own filter state and recomputes its own coefficients for its own rate.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError;

use crate::audio::{Format, Frames};

/// How many bands the equalizer has, one per octave from 31 Hz to 16 kHz.
pub const BANDS: usize = 10;
/// The centre frequency of each band, in hertz.
pub const FREQUENCIES: [f32; BANDS] = [
    31.25, 62.5, 125., 250., 500., 1_000., 2_000., 4_000., 8_000., 16_000.,
];
/// The lowest gain a band accepts, in decibels.
pub const MIN_GAIN: f32 = -12.;
/// The highest gain a band accepts, in decibels.
pub const MAX_GAIN: f32 = 12.;

/// Per band gains, in decibels, from the lowest band to the highest.
pub type Gains = [f32; BANDS];

/// One octave wide bands. Q relates the centre frequency to the bandwidth at the half gain
/// points, and the square root of two is what makes adjacent octave bands meet there.
const Q: f32 = std::f32::consts::SQRT_2;
/// How long a gain change takes to settle, so a dragged slider never clicks.
const RAMP: Duration = Duration::from_millis(40);
/// A band this close to the Nyquist frequency is left flat rather than aliased.
const NYQUIST_MARGIN: f32 = 0.95;
/// How many channels' delay lines are reserved up front, so a format change on the audio thread
/// allocates nothing up to this many.
const CHANNELS: usize = 8;

/// A named set of gains that a user can pick instead of shaping each band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Flat,
    BassBoost,
    BassReducer,
    TrebleBoost,
    Vocal,
    Rock,
    Pop,
    Jazz,
    Classical,
    Electronic,
    Acoustic,
    Loudness,
}

impl Preset {
    pub const ALL: [Self; 12] = [
        Self::Flat,
        Self::BassBoost,
        Self::BassReducer,
        Self::TrebleBoost,
        Self::Vocal,
        Self::Rock,
        Self::Pop,
        Self::Jazz,
        Self::Classical,
        Self::Electronic,
        Self::Acoustic,
        Self::Loudness,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::BassBoost => "bass-boost",
            Self::BassReducer => "bass-reducer",
            Self::TrebleBoost => "treble-boost",
            Self::Vocal => "vocal",
            Self::Rock => "rock",
            Self::Pop => "pop",
            Self::Jazz => "jazz",
            Self::Classical => "classical",
            Self::Electronic => "electronic",
            Self::Acoustic => "acoustic",
            Self::Loudness => "loudness",
        }
    }

    pub fn gains(self) -> Gains {
        match self {
            Self::Flat => [0.; BANDS],
            Self::BassBoost => [6., 5., 4., 2.5, 1., 0., 0., 0., 0., 0.],
            Self::BassReducer => [-6., -5., -4., -2.5, -1., 0., 0., 0., 0., 0.],
            Self::TrebleBoost => [0., 0., 0., 0., 0., 1., 2.5, 4., 5., 6.],
            Self::Vocal => [-2., -3., -3., 1., 4., 4., 3., 1.5, 0., -1.5],
            Self::Rock => [5., 4., 3., 1., -0.5, -1., 0.5, 2.5, 3.5, 4.5],
            Self::Pop => [-1.5, -1., 0., 2., 4., 4., 2., 0., -1., -1.5],
            Self::Jazz => [4., 3., 1.5, 2., -1.5, -1.5, 0., 1.5, 3., 4.],
            Self::Classical => [4.5, 3.5, 3., 2.5, -1.5, -1.5, 0., 2.5, 3.5, 4.],
            Self::Electronic => [4.5, 4., 1., 0., -2., 2., 1., 1.5, 4., 5.],
            Self::Acoustic => [5., 4.5, 3.5, 1., 2., 1.5, 3.5, 4., 3.5, 2.],
            Self::Loudness => [6., 4., 0., 0., -2., 0., -1., -5., 5., 1.],
        }
    }

    /// The preset these gains spell, if they match one to within a tenth of a decibel.
    pub fn matching(gains: &Gains) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| {
            preset
                .gains()
                .iter()
                .zip(gains)
                .all(|(want, have)| (want - have).abs() < 0.05)
        })
    }
}

/// Clamps every band into the accepted range and turns anything that is not a number flat.
pub fn clamped(gains: &Gains) -> Gains {
    gains.map(|gain| match gain.is_finite() {
        true => gain.clamp(MIN_GAIN, MAX_GAIN),
        false => 0.,
    })
}

struct Shared {
    enabled: AtomicBool,
    gains: [AtomicU32; BANDS],
    /// Bumped on every write so an output reads the bands only when they moved.
    generation: AtomicU32,
}

/// The equalizer settings as the audio thread sees them: lock free, read once per frame.
/// Cloning shares the same values, so every output an engine opens follows one setting.
#[derive(Clone)]
pub struct Equalizer(Arc<Shared>);

impl Equalizer {
    pub fn new(enabled: bool, gains: &Gains) -> Self {
        let gains = clamped(gains);
        Self(Arc::new(Shared {
            enabled: AtomicBool::new(enabled),
            gains: gains.map(|gain| AtomicU32::new(gain.to_bits())),
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

    pub fn gains(&self) -> Gains {
        std::array::from_fn(|band| f32::from_bits(self.0.gains[band].load(Ordering::Relaxed)))
    }

    pub fn set_gains(&self, gains: &Gains) {
        for (slot, gain) in self.0.gains.iter().zip(clamped(gains)) {
            slot.store(gain.to_bits(), Ordering::Relaxed);
        }
        self.0.generation.fetch_add(1, Ordering::Release);
    }

    /// The gains the output should apply right now: flat while disabled.
    fn effective(&self) -> Gains {
        match self.enabled() {
            true => self.gains(),
            false => [0.; BANDS],
        }
    }

    fn generation(&self) -> u32 {
        self.0.generation.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for Equalizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Equalizer")
            .field("enabled", &self.enabled())
            .field("gains", &self.gains())
            .finish()
    }
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new(false, &[0.; BANDS])
    }
}

/// One peaking biquad in transposed direct form II. `a0` is normalised away.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// The RBJ cookbook peaking filter, or a pass through when the band sits above what the
    /// rate can carry.
    fn peaking(frequency: f32, gain_db: f32, rate: f32) -> Self {
        if gain_db == 0. || frequency >= rate / 2. * NYQUIST_MARGIN {
            return Self::identity();
        }
        let a = 10f32.powf(gain_db / 40.);
        let w0 = std::f32::consts::TAU * frequency / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2. * Q);

        let a0 = 1. + alpha / a;
        Self {
            b0: (1. + alpha * a) / a0,
            b1: (-2. * cos) / a0,
            b2: (1. - alpha * a) / a0,
            a1: (-2. * cos) / a0,
            a2: (1. - alpha / a) / a0,
        }
    }

    fn identity() -> Self {
        Self {
            b0: 1.,
            ..Self::default()
        }
    }
}

/// The two delay elements one biquad keeps for one channel.
#[derive(Clone, Copy, Default)]
struct Delay {
    z1: f32,
    z2: f32,
}

impl Delay {
    #[inline]
    fn run(&mut self, coefficients: &Biquad, x: f32) -> f32 {
        let y = coefficients.b0 * x + self.z1;
        self.z1 = coefficients.b1 * x - coefficients.a1 * y + self.z2;
        self.z2 = coefficients.b2 * x - coefficients.a2 * y;
        y
    }
}

/// Runs the equalizer over an interleaved source. The gains it applies glide toward the
/// handle's values over `RAMP`, and the whole chain is skipped while every band is flat, so
/// a disabled equalizer costs nothing and changes nothing.
pub struct Equalized<I> {
    input: I,
    equalizer: Equalizer,
    seen: u32,

    current: Gains,
    target: Gains,
    step: Gains,
    frames_left: u32,
    ramp_frames: u32,

    coefficients: [Biquad; BANDS],
    /// Delay lines indexed by band, then channel.
    delays: Vec<[Delay; BANDS]>,
    /// Whether any coefficient is doing work, so a flat chain is skipped whole.
    active: bool,

    frames: Frames,
}

impl<I: Source> Equalized<I> {
    pub(crate) fn new(input: I, equalizer: Equalizer, format: Format) -> Self {
        let current = equalizer.effective();
        let seen = equalizer.generation();
        Self {
            input,
            equalizer,
            seen,
            current,
            target: current,
            step: [0.; BANDS],
            frames_left: 0,
            ramp_frames: 1,
            coefficients: [Biquad::identity(); BANDS],
            delays: Vec::with_capacity(CHANNELS),
            active: false,
            frames: Frames::new(format),
        }
    }

    /// Takes up a new rate and channel count. The delay lines reset, since what they hold
    /// belongs to another format, and every coefficient is recomputed.
    fn resync(&mut self) {
        let channels = usize::from(self.frames.channels());
        let rate = self.frames.rate();
        self.ramp_frames = (RAMP.as_secs_f64() * rate as f64).round().max(1.) as u32;
        self.frames_left = self.frames_left.min(self.ramp_frames);
        self.delays.clear();
        self.delays.resize(channels, [Delay::default(); BANDS]);
        self.recompute();
    }

    fn recompute(&mut self) {
        let rate = self.frames.rate() as f32;
        for (band, coefficients) in self.coefficients.iter_mut().enumerate() {
            *coefficients = Biquad::peaking(FREQUENCIES[band], self.current[band], rate);
        }
        self.active = self.current.iter().any(|gain| *gain != 0.);
    }

    /// Once per frame: picks up a new target and moves one step toward it.
    fn advance(&mut self) {
        let generation = self.equalizer.generation();
        if generation != self.seen {
            self.seen = generation;
            let target = self.equalizer.effective();
            if target != self.target {
                self.target = target;
                self.frames_left = self.ramp_frames;
                for ((step, goal), now) in self.step.iter_mut().zip(target).zip(self.current) {
                    *step = (goal - now) / self.ramp_frames as f32;
                }
            }
        }

        if self.frames_left == 0 {
            return;
        }
        self.frames_left -= 1;
        match self.frames_left {
            0 => self.current = self.target,
            _ => {
                for (now, step) in self.current.iter_mut().zip(self.step) {
                    *now += step;
                }
            }
        }
        self.recompute();
    }
}

impl<I: Source> Iterator for Equalized<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let (sample, place) = self.frames.pull(&mut self.input)?;
        let Some(place) = place else {
            return Some(sample);
        };

        if place.changed {
            self.resync();
        }
        if place.channel == 0 {
            self.advance();
        }

        let output = match self.active {
            true => {
                let delays = &mut self.delays[usize::from(place.channel)];
                self.coefficients
                    .iter()
                    .zip(delays.iter_mut())
                    .fold(sample, |x, (coefficients, delay)| {
                        delay.run(coefficients, x)
                    })
            }
            false => sample,
        };

        Some(output)
    }
}

impl<I: Source> Source for Equalized<I> {
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
        for delays in &mut self.delays {
            *delays = [Delay::default(); BANDS];
        }
        Ok(())
    }
}
