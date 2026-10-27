//! The parametric equalizer: the curve a user edits and the settings keep, the response that
//! curve promises, and the presets. `stage` runs the filters and `shared` carries a curve from
//! its one writer to every output.

pub mod apo;
mod design;
mod shared;
mod stage;

use std::f64::consts::{FRAC_1_SQRT_2, LN_2, SQRT_2};
use std::fmt;
use std::ops::RangeInclusive;

use design::{Biquad, phi};
pub(crate) use design::{Coefficients, Shape, State};
pub use shared::{Edit, Equalizer, EqualizerWriter};
pub(crate) use stage::FilterStage;
pub use stage::MAX_CHANNELS;

/// How many filters a curve holds at most, which is also how many the audio thread reserves.
pub const MAX_FILTERS: usize = 20;
/// The frequencies a filter accepts, in hertz.
pub const FREQUENCY_RANGE: RangeInclusive<f64> = 10.0..=24_000.0;
/// The gains a peak or shelf accepts, in decibels.
pub const GAIN_RANGE: RangeInclusive<f64> = -24.0..=24.0;
/// The Q a filter accepts. The top is Equalizer APO's default for a notch, with room over it.
pub const Q_RANGE: RangeInclusive<f64> = 0.05..=50.0;
/// The bandwidth in octaves a filter accepts.
pub const OCTAVES_RANGE: RangeInclusive<f64> = 0.01..=6.0;
/// The preamp a user may set by hand, in decibels.
pub const PREAMP_RANGE: RangeInclusive<f64> = -30.0..=12.0;
/// The gains the bass and treble shelves of a `Tone` accept, in decibels.
pub const TONE_RANGE: RangeInclusive<f64> = -12.0..=12.0;
/// How many filters the audio thread runs at most: a full curve, then a tone's two shelves.
pub(crate) const SLOTS: usize = MAX_FILTERS + 2;
/// Where the bass and treble shelves turn, and their Q, the steepest slope a shelf takes
/// without a bump beside its step.
const BASS_FREQUENCY: f64 = 100.;
const TREBLE_FREQUENCY: f64 = 10_000.;
const TONE_Q: f64 = FRAC_1_SQRT_2;

/// The rates the automatic preamp is read at, so a curve that is safe at one playing rate is
/// safe at every other. The same filter reads higher near the Nyquist frequency of a higher rate.
const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
/// The grid the automatic preamp reads the response on, from 20 Hz up in one percent steps,
/// plus every filter's centre, without which a narrow peak between grid points is under-read.
const GRID_START: f64 = 20.;
const GRID_RATIO: f64 = 1.01;
const GRID_TOP: f64 = 24_000.;
/// The centre frequencies of the graphic equalizer the presets and the sliders are drawn on,
/// one per octave, and the Q that makes adjacent bands meet at their half gain points.
const BAND_FREQUENCIES: [f64; 10] = [
    31.25, 62.5, 125., 250., 500., 1_000., 2_000., 4_000., 8_000., 16_000.,
];
const BAND_Q: f64 = SQRT_2;

/// What a filter does to the band around its frequency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    Notch,
    BandPass,
    AllPass,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Peak,
        Self::LowShelf,
        Self::HighShelf,
        Self::LowPass,
        Self::HighPass,
        Self::Notch,
        Self::BandPass,
        Self::AllPass,
    ];

    /// The name the settings file spells the kind with.
    pub fn id(self) -> &'static str {
        match self {
            Self::Peak => "peak",
            Self::LowShelf => "low-shelf",
            Self::HighShelf => "high-shelf",
            Self::LowPass => "low-pass",
            Self::HighPass => "high-pass",
            Self::Notch => "notch",
            Self::BandPass => "band-pass",
            Self::AllPass => "all-pass",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// Whether the kind boosts or cuts. The others only shape, and their gain is kept at zero.
    pub fn has_gain(self) -> bool {
        matches!(self, Self::Peak | Self::LowShelf | Self::HighShelf)
    }

    fn is_shelf(self) -> bool {
        matches!(self, Self::LowShelf | Self::HighShelf)
    }

    pub(crate) fn code(self) -> u8 {
        Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0) as u8
    }

    pub(crate) fn of(code: u8) -> Self {
        Self::ALL
            .get(usize::from(code))
            .copied()
            .unwrap_or(Self::Peak)
    }
}

/// How wide a filter is. Octaves are kept as written, because Equalizer APO converts them to a Q
/// at the playing rate, so converting at import would be wrong at every other rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    Q(f64),
    Octaves(f64),
}

impl Width {
    pub fn value(self) -> f64 {
        match self {
            Self::Q(value) | Self::Octaves(value) => value,
        }
    }
}

/// Why `Filter::new` or `Curve::new` refused a value, carrying the value, so a caller can say
/// which one is wrong.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Invalid {
    Frequency(f64),
    Gain(f64),
    Q(f64),
    Octaves(f64),
    /// A shelf was given a width in octaves, which only a Q describes.
    ShelfOctaves,
    Preamp(f64),
    TooMany(usize),
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frequency(hz) => write!(f, "frequency {hz} Hz is outside {FREQUENCY_RANGE:?}"),
            Self::Gain(db) => write!(f, "gain {db} dB is outside {GAIN_RANGE:?}"),
            Self::Q(q) => write!(f, "Q {q} is outside {Q_RANGE:?}"),
            Self::Octaves(octaves) => {
                write!(f, "width {octaves} octaves is outside {OCTAVES_RANGE:?}")
            }
            Self::ShelfOctaves => write!(f, "a shelf takes a Q, not a width in octaves"),
            Self::Preamp(db) => write!(f, "preamp {db} dB is outside {PREAMP_RANGE:?}"),
            Self::TooMany(count) => {
                write!(
                    f,
                    "a curve holds at most {MAX_FILTERS} filters, not {count}"
                )
            }
        }
    }
}

impl std::error::Error for Invalid {}

/// One filter of a curve. Only `new` makes one, so a filter that exists is in range and the audio
/// thread can trust it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    kind: Kind,
    frequency: f64,
    gain: f64,
    width: Width,
    enabled: bool,
}

impl Filter {
    /// A filter in range, or why it is not. A shelf takes a Q, never octaves, since APO ignores
    /// octaves on shelves. A kind without a gain keeps zero, whatever gain it was given.
    pub fn new(
        kind: Kind,
        frequency: f64,
        gain: f64,
        width: Width,
        enabled: bool,
    ) -> Result<Self, Invalid> {
        if !FREQUENCY_RANGE.contains(&frequency) {
            return Err(Invalid::Frequency(frequency));
        }
        if kind.has_gain() && !GAIN_RANGE.contains(&gain) {
            return Err(Invalid::Gain(gain));
        }
        match width {
            Width::Q(q) if !Q_RANGE.contains(&q) => return Err(Invalid::Q(q)),
            Width::Octaves(_) if kind.is_shelf() => return Err(Invalid::ShelfOctaves),
            Width::Octaves(octaves) if !OCTAVES_RANGE.contains(&octaves) => {
                return Err(Invalid::Octaves(octaves));
            }
            _ => {}
        }
        Ok(Self {
            kind,
            frequency,
            gain: match kind.has_gain() {
                true => gain,
                false => 0.,
            },
            width,
            enabled,
        })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn frequency(&self) -> f64 {
        self.frequency
    }

    pub fn gain(&self) -> f64 {
        self.gain
    }

    pub fn width(&self) -> Width {
        self.width
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn with_enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// The filter as another kind, keeping what still applies. A shelf takes only a Q, so a
    /// width in octaves becomes the Q it means for an analog filter, clamped into range.
    pub fn with_kind(self, kind: Kind) -> Self {
        let width = match (kind.is_shelf(), self.width) {
            (true, Width::Octaves(octaves)) => {
                let q = 1. / (2. * (LN_2 / 2. * octaves).sinh());
                Width::Q(q.clamp(*Q_RANGE.start(), *Q_RANGE.end()))
            }
            (_, width) => width,
        };
        Self::new(kind, self.frequency, self.gain, width, self.enabled)
            .expect("a filter in range stays in range as another kind")
    }

    /// Whether this filter is the one a graphic band of `gain` decibels at `frequency` means.
    fn is_band(&self, frequency: f64, gain: f64) -> bool {
        self.kind == Kind::Peak
            && self.enabled
            && self.frequency == frequency
            && self.width == Width::Q(BAND_Q)
            && (self.gain - gain).abs() < 0.05
    }
}

/// How the level is lowered to make room for a curve's boosts: by the curve's own highest
/// boost, or by what the user or an imported file says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Preamp {
    Auto,
    Manual(f64),
}

/// A preamp and up to `MAX_FILTERS` filters, applied in order. One curve plays on every channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    preamp: Preamp,
    filters: Vec<Filter>,
}

impl Curve {
    pub fn new(preamp: Preamp, filters: Vec<Filter>) -> Result<Self, Invalid> {
        if filters.len() > MAX_FILTERS {
            return Err(Invalid::TooMany(filters.len()));
        }
        if let Preamp::Manual(db) = preamp
            && !PREAMP_RANGE.contains(&db)
        {
            return Err(Invalid::Preamp(db));
        }
        Ok(Self { preamp, filters })
    }

    /// The graphic equalizer's curve: one octave wide peak per band at `gains` decibels, lowest
    /// band first, with the preamp automatic. Gains beyond the ten bands are dropped, missing
    /// ones are flat, and each is clamped into range.
    pub fn bands(gains: &[f64]) -> Self {
        let filters = BAND_FREQUENCIES
            .iter()
            .enumerate()
            .map(|(band, frequency)| {
                let gain = gains.get(band).copied().unwrap_or(0.);
                let gain = match gain.is_finite() {
                    true => gain.clamp(*GAIN_RANGE.start(), *GAIN_RANGE.end()),
                    false => 0.,
                };
                Filter::new(Kind::Peak, *frequency, gain, Width::Q(BAND_Q), true)
                    .expect("a band is in range")
            })
            .collect();
        Self {
            preamp: Preamp::Auto,
            filters,
        }
    }

    pub fn preamp(&self) -> Preamp {
        self.preamp
    }

    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// The filters to edit in place. A curve has no invariant beyond its length, and a filter
    /// stays valid because only `Filter::new` makes one.
    pub fn filters_mut(&mut self) -> &mut [Filter] {
        &mut self.filters
    }

    /// The preamp in decibels the filter stage applies. An automatic one is lowered by the curve's
    /// highest boost at any rate it may play at, so no steady tone leaves above full scale.
    pub fn preamp_db(&self) -> f64 {
        match self.preamp {
            Preamp::Manual(db) => db,
            Preamp::Auto => (0. - self.boost()).min(0.),
        }
    }

    /// The curve as it plays under `tone`: its own filters, then the tone's bass and treble
    /// shelves, which a flat tone leaves at 0 dB so they pass every sample untouched. An automatic
    /// preamp makes room for the shelves as for any filter, and a manual one is lowered by however
    /// much they raise the highest boost. The result may hold more than `MAX_FILTERS` filters, so
    /// it is for playing and drawing, not for storing.
    pub fn toned(&self, tone: Tone) -> Self {
        let mut toned = Self {
            preamp: self.preamp,
            filters: self.filters.iter().copied().chain(tone.shelves()).collect(),
        };
        if let Preamp::Manual(db) = self.preamp
            && !tone.is_flat()
        {
            let raised = (toned.boost() - self.boost()).max(0.);
            toned.preamp = Preamp::Manual((db - raised).max(*PREAMP_RANGE.start()));
        }
        toned
    }

    /// The curve's highest boost in decibels at any rate it may play at, or 0 when it only cuts.
    fn boost(&self) -> f64 {
        RATES
            .into_iter()
            .map(|rate| self.response(rate).peak())
            .fold(0., f64::max)
    }

    /// Whether the enabled filters leave every sample untouched at `rate`: there are none, or
    /// only peaks and shelves at 0 dB, which design to an exact identity. The preamp is apart.
    pub fn passes_through(&self, rate: u32) -> bool {
        let rate = f64::from(rate);
        self.filters
            .iter()
            .filter(|filter| filter.enabled)
            .all(|filter| Coefficients::of(&Shape::of(filter), rate).is_identity())
    }

    /// What the enabled filters do when playing at `rate`, without the preamp. This is the
    /// response the audio thread reproduces, so a graph drawn from it shows what is heard.
    pub fn response(&self, rate: u32) -> Response {
        let rate = f64::from(rate);
        let biquads = self
            .filters
            .iter()
            .filter(|filter| filter.enabled)
            .map(|filter| Biquad::of(&Shape::of(filter), rate))
            .collect();
        let centres = self
            .filters
            .iter()
            .filter(|filter| filter.enabled)
            .map(|filter| filter.frequency)
            .collect();
        Response {
            biquads,
            centres,
            rate,
        }
    }
}

/// A curve's steady-state magnitude at one rate, ready to read at any frequency.
#[derive(Clone, Debug)]
pub struct Response {
    biquads: Vec<Biquad>,
    centres: Vec<f64>,
    rate: f64,
}

impl Response {
    /// The gain at `frequency` in decibels.
    pub fn db(&self, frequency: f64) -> f64 {
        let phi = phi(frequency, self.rate);
        self.biquads.iter().map(|biquad| biquad.db(phi)).sum()
    }

    /// The highest gain over the band the rate carries, read on the grid and at each centre.
    fn peak(&self) -> f64 {
        let top = (self.rate / 2.).min(GRID_TOP);
        let grid = std::iter::successors(Some(GRID_START), |f| Some(f * GRID_RATIO))
            .take_while(|f| *f <= top);
        let centres = self.centres.iter().copied().filter(|f| *f <= top);
        grid.chain(centres)
            .map(|f| self.db(f))
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// A bass and a treble shelf laid over whatever sound plays, so a listener can tilt it without
/// editing the sound. Both gains are 0 dB when the tone is flat, and always within `TONE_RANGE`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tone {
    bass: f64,
    treble: f64,
}

impl Tone {
    /// A tone with both gains clamped into `TONE_RANGE`. A gain that is not a number is 0 dB.
    pub fn new(bass: f64, treble: f64) -> Self {
        let clamp = |db: f64| match db.is_finite() {
            true => db.clamp(*TONE_RANGE.start(), *TONE_RANGE.end()),
            false => 0.,
        };
        Self {
            bass: clamp(bass),
            treble: clamp(treble),
        }
    }

    pub fn bass(self) -> f64 {
        self.bass
    }

    pub fn treble(self) -> f64 {
        self.treble
    }

    pub fn is_flat(self) -> bool {
        self.bass == 0. && self.treble == 0.
    }

    /// What the tone's shelves alone do when playing at `rate`, which a graph adds to a filter's
    /// own gain to set its node on the curve heard.
    pub fn response(self, rate: u32) -> Response {
        Curve {
            preamp: Preamp::Auto,
            filters: self.shelves().to_vec(),
        }
        .response(rate)
    }

    /// The low shelf for the bass and the high shelf for the treble, in that order.
    fn shelves(self) -> [Filter; 2] {
        let shelf = |kind, frequency, gain| Filter {
            kind,
            frequency,
            gain,
            width: Width::Q(TONE_Q),
            enabled: true,
        };
        [
            shelf(Kind::LowShelf, BASS_FREQUENCY, self.bass),
            shelf(Kind::HighShelf, TREBLE_FREQUENCY, self.treble),
        ]
    }
}

/// A named curve a user can pick instead of shaping each band.
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

    /// The preset as a curve of ten octave wide peaks, the same curve the graphic equalizer
    /// played, now with an automatic preamp under it.
    pub fn curve(self) -> Curve {
        Curve::bands(&self.band_gains())
    }

    /// The preset `curve` spells, if it is ten graphic bands within a tenth of a decibel of one.
    pub fn matching(curve: &Curve) -> Option<Self> {
        if curve.filters.len() != BAND_FREQUENCIES.len() {
            return None;
        }
        Self::ALL.into_iter().find(|preset| {
            curve
                .filters
                .iter()
                .zip(BAND_FREQUENCIES)
                .zip(preset.band_gains())
                .all(|((filter, frequency), gain)| filter.is_band(frequency, gain))
        })
    }

    fn band_gains(self) -> [f64; 10] {
        match self {
            Self::Flat => [0.; 10],
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
}
