//! Turns a filter into coefficients at one rate. The audio thread runs Simper's trapezoidal
//! state variable filter, and the writer side reads the RBJ cookbook biquad the same filter
//! means, whose magnitude has a closed form. Both go through `place`, so the rules for a filter
//! near the Nyquist frequency are decided once.

use std::f64::consts::{LN_2, PI, TAU};

use super::{Filter, Kind, Width};

/// Above this fraction of the rate a filter is designed at its limit or left out, since its
/// effect lies where the stream cannot carry it.
const NYQUIST: f64 = 0.49;
/// How far a filter may move in one event and still glide: an octave, six decibels, and twice
/// as wide or narrow. A bigger jump, as after a stalled UI, is replaced instead of swept.
const GLIDE_OCTAVES: f64 = 1.;
const GLIDE_DECIBELS: f64 = 6.;
const GLIDE_WIDTH: f64 = 1.;

/// What a filter sounds like, without whether it is on. The audio thread interpolates these
/// during a glide and redesigns from them every sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    pub kind: Kind,
    pub frequency: f64,
    pub gain: f64,
    pub width: Width,
}

impl Shape {
    pub fn of(filter: &Filter) -> Self {
        Self {
            kind: filter.kind,
            frequency: filter.frequency,
            gain: filter.gain,
            width: filter.width,
        }
    }

    /// The shape `t` of the way to `to`: frequency and width move on a log scale, gain in
    /// decibels, which is the path a user hears as even.
    pub fn toward(&self, to: &Self, t: f64) -> Self {
        Self {
            kind: self.kind,
            frequency: self.frequency * (to.frequency / self.frequency).powf(t),
            gain: self.gain + (to.gain - self.gain) * t,
            width: self.width.toward(to.width, t),
        }
    }

    /// Whether a glide from here to `to` stays on one filter's path at `rate`: the same kind and
    /// width unit, a small move, and the same side of the Nyquist rules.
    pub fn near(&self, to: &Self, rate: f64) -> bool {
        self.kind == to.kind
            && (self.frequency / to.frequency).log2().abs() <= GLIDE_OCTAVES
            && (self.gain - to.gain).abs() <= GLIDE_DECIBELS
            && self.width.near(to.width)
            && place(self, rate).same_region(&place(to, rate))
    }
}

impl Width {
    /// The width as the Q the design uses at `frequency` and `rate`. Octaves convert with the
    /// prewarped relation Equalizer APO uses, which is why they depend on the rate.
    pub(crate) fn q(self, frequency: f64, rate: f64) -> f64 {
        match self {
            Self::Q(q) => q,
            Self::Octaves(octaves) => {
                let w0 = TAU * frequency / rate;
                1. / (2. * (LN_2 / 2. * octaves * w0 / w0.sin()).sinh())
            }
        }
    }

    fn toward(self, to: Self, t: f64) -> Self {
        match (self, to) {
            (Self::Q(from), Self::Q(to)) => Self::Q(from * (to / from).powf(t)),
            (Self::Octaves(from), Self::Octaves(to)) => Self::Octaves(from * (to / from).powf(t)),
            _ => to,
        }
    }

    fn near(self, to: Self) -> bool {
        match (self, to) {
            (Self::Q(from), Self::Q(to)) | (Self::Octaves(from), Self::Octaves(to)) => {
                (from / to).log2().abs() <= GLIDE_WIDTH
            }
            _ => false,
        }
    }
}

/// Where a filter lands at one rate. A filter whose frequency the rate cannot carry is left out,
/// kept as a low shelf's flat gain, or held at the limit for a high pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Placed {
    Identity,
    Flat(f64),
    Filter {
        kind: Kind,
        frequency: f64,
        gain: f64,
        q: f64,
    },
}

impl Placed {
    fn same_region(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Identity, Self::Identity)
                | (Self::Flat(_), Self::Flat(_))
                | (Self::Filter { .. }, Self::Filter { .. })
        )
    }
}

pub(crate) fn place(shape: &Shape, rate: f64) -> Placed {
    let limit = NYQUIST * rate;
    let mut frequency = shape.frequency;
    if frequency >= limit {
        match shape.kind {
            Kind::HighPass => frequency = limit,
            Kind::LowShelf => return Placed::Flat(shape.gain),
            _ => return Placed::Identity,
        }
    }
    Placed::Filter {
        kind: shape.kind,
        frequency,
        gain: shape.gain,
        q: shape.width.q(frequency, rate),
    }
}

/// One Simper state variable filter's coefficients: `g` and `k` set the resonance, `m0..m2` mix
/// the input, band pass and low pass into the output. The mix `(1, 0, 0)` is an exact identity
/// for any `g` and `k`, which is what a transition passes through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coefficients {
    pub g: f64,
    pub k: f64,
    pub m0: f64,
    pub m1: f64,
    pub m2: f64,
}

impl Coefficients {
    pub const IDENTITY: Self = Self {
        g: 0.,
        k: 0.,
        m0: 1.,
        m1: 0.,
        m2: 0.,
    };

    pub fn of(shape: &Shape, rate: f64) -> Self {
        let (kind, frequency, gain, q) = match place(shape, rate) {
            Placed::Identity => return Self::IDENTITY,
            Placed::Flat(db) => {
                return Self {
                    m0: 10f64.powf(db / 20.),
                    ..Self::IDENTITY
                };
            }
            Placed::Filter {
                kind,
                frequency,
                gain,
                q,
            } => (kind, frequency, gain, q),
        };
        let a = 10f64.powf(gain / 40.);
        let g0 = (PI * frequency / rate).tan();
        let k = 1. / q;
        let (g, k, m0, m1, m2) = match kind {
            Kind::LowPass => (g0, k, 0., 0., 1.),
            Kind::HighPass => (g0, k, 1., -k, -1.),
            Kind::Notch => (g0, k, 1., -k, 0.),
            Kind::BandPass => (g0, k, 0., k, 0.),
            Kind::AllPass => (g0, k, 1., -2. * k, 0.),
            Kind::Peak => {
                let k = k / a;
                (g0, k, 1., k * (a * a - 1.), 0.)
            }
            Kind::LowShelf => (g0 / a.sqrt(), k, 1., k * (a - 1.), a * a - 1.),
            Kind::HighShelf => (g0 * a.sqrt(), k, a * a, k * (1. - a) * a, 1. - a * a),
        };
        Self { g, k, m0, m1, m2 }
    }

    pub fn is_identity(&self) -> bool {
        self.m0 == 1. && self.m1 == 0. && self.m2 == 0.
    }

    pub fn mix(&self) -> [f64; 3] {
        [self.m0, self.m1, self.m2]
    }

    /// The same `g` and `k` with the identity mix, which is exactly `v0` out whatever the state.
    pub fn at_identity(&self) -> Self {
        Self {
            m0: 1.,
            m1: 0.,
            m2: 0.,
            ..*self
        }
    }

    /// The same `g` and `k` with the mix `t` of the way from this one to `mix`. With the
    /// resonance fixed this is an exact crossfade between the two outputs.
    pub fn mixed_toward(&self, mix: [f64; 3], t: f64) -> Self {
        Self {
            m0: self.m0 + (mix[0] - self.m0) * t,
            m1: self.m1 + (mix[1] - self.m1) * t,
            m2: self.m2 + (mix[2] - self.m2) * t,
            ..*self
        }
    }
}

/// `φ = sin²(πf/fs)`, the one number the closed form for a biquad's magnitude at `frequency`
/// needs.
pub(crate) fn phi(frequency: f64, rate: f64) -> f64 {
    (PI * frequency / rate).sin().powi(2)
}

/// The two integrator states one filter keeps for one channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct State {
    pub ic1: f64,
    pub ic2: f64,
}

impl State {
    #[inline]
    pub fn tick(&mut self, c: &Coefficients, v0: f64) -> f64 {
        let a1 = 1. / (1. + c.g * (c.g + c.k));
        let a2 = c.g * a1;
        let a3 = c.g * a2;
        let v3 = v0 - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2. * v1 - self.ic1;
        self.ic2 = 2. * v2 - self.ic2;
        c.m0 * v0 + c.m1 * v1 + c.m2 * v2
    }
}

/// The RBJ cookbook biquad a filter means, normalised so `a0` is one. This is the response the
/// state variable filter reproduces and the one the writer side reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    pub fn of(shape: &Shape, rate: f64) -> Self {
        let flat = |b0: f64| Self {
            b0,
            b1: 0.,
            b2: 0.,
            a1: 0.,
            a2: 0.,
        };
        let (kind, frequency, gain, q) = match place(shape, rate) {
            Placed::Identity => return flat(1.),
            Placed::Flat(db) => return flat(10f64.powf(db / 20.)),
            Placed::Filter {
                kind,
                frequency,
                gain,
                q,
            } => (kind, frequency, gain, q),
        };
        let a = 10f64.powf(gain / 40.);
        let (sn, cs) = (TAU * frequency / rate).sin_cos();
        let alpha = sn / (2. * q);
        let beta = 2. * a.sqrt() * alpha;
        let (b, a) = match kind {
            Kind::LowPass => (
                [(1. - cs) / 2., 1. - cs, (1. - cs) / 2.],
                [1. + alpha, -2. * cs, 1. - alpha],
            ),
            Kind::HighPass => (
                [(1. + cs) / 2., -(1. + cs), (1. + cs) / 2.],
                [1. + alpha, -2. * cs, 1. - alpha],
            ),
            Kind::BandPass => ([alpha, 0., -alpha], [1. + alpha, -2. * cs, 1. - alpha]),
            Kind::Notch => ([1., -2. * cs, 1.], [1. + alpha, -2. * cs, 1. - alpha]),
            Kind::AllPass => (
                [1. - alpha, -2. * cs, 1. + alpha],
                [1. + alpha, -2. * cs, 1. - alpha],
            ),
            Kind::Peak => (
                [1. + alpha * a, -2. * cs, 1. - alpha * a],
                [1. + alpha / a, -2. * cs, 1. - alpha / a],
            ),
            Kind::LowShelf => (
                [
                    a * ((a + 1.) - (a - 1.) * cs + beta),
                    2. * a * ((a - 1.) - (a + 1.) * cs),
                    a * ((a + 1.) - (a - 1.) * cs - beta),
                ],
                [
                    (a + 1.) + (a - 1.) * cs + beta,
                    -2. * ((a - 1.) + (a + 1.) * cs),
                    (a + 1.) + (a - 1.) * cs - beta,
                ],
            ),
            Kind::HighShelf => (
                [
                    a * ((a + 1.) + (a - 1.) * cs + beta),
                    -2. * a * ((a - 1.) + (a + 1.) * cs),
                    a * ((a + 1.) + (a - 1.) * cs - beta),
                ],
                [
                    (a + 1.) - (a - 1.) * cs + beta,
                    2. * ((a - 1.) - (a + 1.) * cs),
                    (a + 1.) - (a - 1.) * cs - beta,
                ],
            ),
        };
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
        }
    }

    /// The magnitude in decibels at the frequency `phi` stands for, from the closed form in
    /// `φ = sin²(πf/fs)`. The caller works `phi` out once for every biquad of a curve.
    pub fn db(&self, phi: f64) -> f64 {
        let (b0, b1, b2) = (self.b0, self.b1, self.b2);
        let (a1, a2) = (self.a1, self.a2);
        let num = (b0 + b1 + b2).powi(2) - 4. * (b0 * b1 + 4. * b0 * b2 + b1 * b2) * phi
            + 16. * b0 * b2 * phi * phi;
        let den =
            (1. + a1 + a2).powi(2) - 4. * (a1 + 4. * a2 + a1 * a2) * phi + 16. * a2 * phi * phi;
        10. * num.max(0.).log10() - 10. * den.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A long enough impulse response for the filter to have rung out to below the tolerance.
    const IMPULSE_SECONDS: f64 = 0.5;
    const TOLERANCE_DB: f64 = 1e-6;
    /// Below this a reading sits in a notch, where both sides are rounding noise.
    const NOTCH_FLOOR_DB: f64 = -60.;

    /// The magnitude in decibels the running filter has at `frequency`, read from its impulse
    /// response with Goertzel's algorithm.
    fn measured_db(c: &Coefficients, frequency: f64, rate: f64) -> f64 {
        let n = (IMPULSE_SECONDS * rate) as usize;
        let w = TAU * frequency / rate;
        let coefficient = 2. * w.cos();
        let mut state = State::default();
        let (mut s1, mut s2) = (0., 0.);
        for i in 0..n {
            let x = match i {
                0 => 1.,
                _ => 0.,
            };
            let y = state.tick(c, x);
            let s0 = y + coefficient * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let re = s1 - s2 * w.cos();
        let im = s2 * w.sin();
        10. * (re * re + im * im).log10()
    }

    #[test]
    fn state_variable_filter_matches_the_rbj_response() {
        let cases = [(50., 9., 2.), (1_000., -7., 0.707), (12_000., 6., 1.5)];
        for rate in [44_100., 48_000., 96_000., 192_000.] {
            for kind in Kind::ALL {
                for (frequency, gain, q) in cases {
                    let shape = shape(kind, frequency, gain, q);
                    let svf = Coefficients::of(&shape, rate);
                    let rbj = Biquad::of(&shape, rate);
                    for at in [
                        frequency / 3.,
                        frequency / 1.5,
                        frequency,
                        frequency * 1.5,
                        frequency * 3.,
                    ] {
                        if at >= 0.49 * rate {
                            continue;
                        }
                        let want = rbj.db(phi(at, rate));
                        if want < NOTCH_FLOOR_DB {
                            continue;
                        }
                        let got = measured_db(&svf, at, rate);
                        assert!(
                            (got - want).abs() < TOLERANCE_DB,
                            "{kind:?} {frequency} Hz {gain:+} dB Q {q} at {rate} Hz, read at {at} Hz: \
                             svf {got:.7} dB, rbj {want:.7} dB"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rbj_response_matches_the_oracle() {
        let peak = Biquad::of(&shape(Kind::Peak, 1_000., 6., 1.), 48_000.);
        assert!((peak.db(phi(1_000., 48_000.)) - 6.).abs() < 1e-6);
        assert!((peak.db(phi(500., 48_000.)) - 1.879381).abs() < 1e-6);
        let shelf = Biquad::of(&shape(Kind::LowShelf, 105., 6., 0.7), 48_000.);
        assert!((shelf.db(phi(105., 48_000.)) - 3.).abs() < 1e-4);
    }

    fn shape(kind: Kind, frequency: f64, gain: f64, q: f64) -> Shape {
        Shape {
            kind,
            frequency,
            gain,
            width: Width::Q(q),
        }
    }
}
