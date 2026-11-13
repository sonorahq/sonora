//! Runs a curve over interleaved samples on the audio thread: a preamp, then one state variable
//! filter per filter per channel, in f64. A small move of one filter glides, anything else
//! crosses through an exact identity, and the preamp falls at once but rises only once the
//! filters have settled, so no change clicks. Nothing here allocates, locks or blocks.

use std::time::Duration;

use super::design::{Coefficients, Shape, State};
use super::shared::Snapshot;
use super::{Equalizer, SLOTS};

/// How many channels the equalizer keeps state for. A stream with more passes through untouched.
pub const MAX_CHANNELS: usize = 8;
/// How long a glide, and each half of a replace, takes.
const RAMP: Duration = Duration::from_millis(40);
/// Every this many frames the state is swept for values too small to matter or not finite.
const FLUSH_EVERY: u32 = 64;
/// Below this, about -500 dBFS, a state value is zeroed before it goes subnormal, which would
/// slow every sample after it for as long as the silence lasts.
const FLUSH_BELOW: f64 = 1e-25;

/// The equalizer with the rodio plumbing left out, so it can be driven frame by frame.
pub(crate) struct FilterStage {
    reader: Equalizer,
    seen: u32,
    replaces: u32,
    rate: f64,
    channels: usize,
    /// How many frames a ramp takes at this rate.
    steps: u32,
    slots: [Slot; SLOTS],
    /// One past the last slot that runs, so a short curve costs no empty slots per sample.
    used: usize,
    preamp: Ramp,
    /// Indexed by channel, then filter.
    state: [[State; SLOTS]; MAX_CHANNELS],
    /// Whether anything runs. While nothing does, samples pass through untouched.
    active: bool,
    frame: u32,
    /// Set for a stream with more channels than there is state for, which passes through.
    bypass: bool,
}

/// One filter's place in the chain and the transition it is in.
#[derive(Clone, Copy)]
struct Slot {
    /// What the latest snapshot asks of this slot, identity when nothing.
    target: Option<Shape>,
    current: Coefficients,
    phase: Phase,
    /// Whether the slot does anything this frame, so an identity costs no sample time.
    running: bool,
}

#[derive(Clone, Copy)]
enum Phase {
    Settled,
    /// Redesigned every frame: the shape sweeps from `from` to `to` for `left` more frames
    /// while the mix fades in from identity for `fade` more, so a drag during a fade-in re-aims
    /// the sweep instead of starting over.
    Glide {
        from: Shape,
        to: Shape,
        left: u32,
        fade: u32,
    },
    /// The mix fades to identity with `g` and `k` held: an exact crossfade to dry.
    ToIdentity {
        from: Coefficients,
        left: u32,
    },
}

/// A gain that falls as soon as it is asked to and rises only once the stage lets it. Its
/// reciprocal moves linearly: a filter crossfades linearly too, so the level at any frequency
/// is a ratio of two lines, which moves one way and never overshoots.
struct Ramp {
    /// The gain as a factor, what every sample is multiplied by.
    current: f64,
    /// One over the gain, which moves linearly.
    inverse: f64,
    target_db: f64,
    target_inverse: f64,
    step: f64,
    left: u32,
}

impl FilterStage {
    /// A stage holding the current curve, which it takes up without a ramp at its first
    /// format. Built off the audio thread, so it may wait out a write under way.
    pub(crate) fn new(reader: Equalizer) -> Self {
        let mut engine = Self {
            reader,
            seen: u32::MAX,
            replaces: 0,
            rate: 0.,
            channels: 0,
            steps: 1,
            slots: [Slot::IDLE; SLOTS],
            used: 0,
            preamp: Ramp::new(0.),
            state: [[State::default(); SLOTS]; MAX_CHANNELS],
            active: false,
            frame: 0,
            bypass: false,
        };
        loop {
            if let Some((seq, snapshot)) = engine.reader.poll(engine.seen) {
                engine.seen = seq;
                engine.take(&snapshot);
                return engine;
            }
            std::hint::spin_loop();
        }
    }

    /// Takes up a new rate and channel count. Every slot snaps to its target at the new rate,
    /// and the state is zeroed, since what it holds belongs to another format.
    pub(crate) fn set_format(&mut self, rate: u32, channels: u16) {
        self.rate = f64::from(rate);
        self.channels = usize::from(channels);
        self.bypass = self.channels > MAX_CHANNELS;
        if self.bypass {
            self.reader.flag_unprocessed();
        }
        self.steps = (RAMP.as_secs_f64() * self.rate).round().max(1.) as u32;
        for slot in &mut self.slots {
            slot.settle(self.rate);
        }
        self.preamp.snap();
        self.reset();
        self.active = self.recount();
        self.frame = 0;
    }

    /// Once per frame, before its first sample: picks up a new curve and moves every
    /// transition one frame on.
    pub(crate) fn begin(&mut self) {
        if let Some((seq, snapshot)) = self.reader.poll(self.seen) {
            self.seen = seq;
            self.take(&snapshot);
        } else if !self.active && !self.preamp.waiting() {
            return;
        }
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.advance(self.rate, self.steps) {
                for channel in &mut self.state {
                    channel[index] = State::default();
                }
            }
        }
        if self.preamp.waiting()
            && (!self.preamp.rises() || self.slots.iter().all(Slot::lets_the_preamp_rise))
        {
            self.preamp.start(self.steps);
        }
        self.preamp.advance();
        self.active = self.recount();
        self.frame = self.frame.wrapping_add(1);
        if self.frame.is_multiple_of(FLUSH_EVERY) {
            self.flush();
        }
    }

    #[inline]
    pub(crate) fn process(&mut self, channel: usize, sample: f32) -> f32 {
        if !self.active || self.bypass {
            return sample;
        }
        let mut x = f64::from(sample) * self.preamp.current;
        let states = &mut self.state[channel];
        for (slot, state) in self.slots[..self.used].iter().zip(states.iter_mut()) {
            if slot.running {
                x = state.tick(&slot.current, x);
            }
        }
        x as f32
    }

    /// Forgets what the filters have heard, for a seek or a format change.
    pub(crate) fn reset(&mut self) {
        self.state = [[State::default(); SLOTS]; MAX_CHANNELS];
    }

    /// Retargets every slot and the preamp from a snapshot. Before the first format the targets
    /// are only stored, and `set_format` designs them.
    fn take(&mut self, snapshot: &Snapshot) {
        let force = snapshot.replaces != self.replaces;
        self.replaces = snapshot.replaces;
        let target = |index: usize| {
            let filter = &snapshot.filters[index];
            (snapshot.enabled && filter.enabled).then(|| Shape::of(filter))
        };
        let preamp_db = match snapshot.enabled {
            true => snapshot.preamp_db,
            false => 0.,
        };
        if self.rate == 0. {
            for (index, slot) in self.slots.iter_mut().enumerate() {
                slot.target = target(index);
            }
            self.preamp = Ramp::new(preamp_db);
            return;
        }

        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.retarget(target(index), force, self.rate, self.steps) {
                for channel in &mut self.state {
                    channel[index] = State::default();
                }
            }
        }
        self.preamp.aim(preamp_db);
    }

    /// Counts the slots that run, so a short curve costs no empty slots per sample, and says
    /// whether anything runs at all, the preamp included.
    fn recount(&mut self) -> bool {
        self.used = self
            .slots
            .iter()
            .rposition(|slot| slot.running)
            .map_or(0, |last| last + 1);
        self.used > 0 || self.preamp.current != 1. || self.preamp.left > 0
    }

    fn flush(&mut self) {
        for channel in &mut self.state[..self.channels.min(MAX_CHANNELS)] {
            for state in &mut channel[..self.used] {
                flush(state);
            }
        }
    }
}

impl Slot {
    const IDLE: Self = Self {
        target: None,
        current: Coefficients::IDENTITY,
        phase: Phase::Settled,
        running: false,
    };

    /// Aims the slot at `target`: a small move of the same kind glides unless `force` says the
    /// edit was a replace, anything else crosses through identity. Returns whether the state is
    /// to be zeroed now, which is inaudible only at an exact identity.
    fn retarget(&mut self, target: Option<Shape>, force: bool, rate: f64, steps: u32) -> bool {
        if target == self.target {
            return false;
        }
        if !force
            && let (Some(from), Some(to)) = (self.shape(steps), target)
            && from.near(&to, rate)
        {
            self.target = target;
            let fade = match self.phase {
                Phase::Glide { fade, .. } => fade,
                _ => 0,
            };
            self.phase = Phase::Glide {
                from,
                to,
                left: steps,
                fade,
            };
            return false;
        }
        self.target = target;
        match self.phase {
            Phase::ToIdentity { .. } => false,
            _ if self.current.is_identity() => {
                self.snap(rate, steps);
                true
            }
            _ => {
                self.phase = Phase::ToIdentity {
                    from: self.current,
                    left: steps,
                };
                false
            }
        }
    }

    /// Moves one frame on. Returns whether the state is to be zeroed now, which is at an exact
    /// identity, so a slot that does not run holds no state.
    fn advance(&mut self, rate: f64, steps: u32) -> bool {
        let was_running = self.running;
        let mut zero = false;
        match self.phase {
            Phase::Settled => {}
            Phase::Glide {
                from,
                to,
                left,
                fade,
            } => {
                let left = left.saturating_sub(1);
                let fade = fade.saturating_sub(1);
                let design = Coefficients::of(&along(&from, &to, left, steps), rate);
                self.current = match fade {
                    0 => design,
                    _ => design
                        .at_identity()
                        .mixed_toward(design.mix(), fraction(fade, steps)),
                };
                self.phase = match (left, fade) {
                    (0, 0) => Phase::Settled,
                    _ => Phase::Glide {
                        from,
                        to,
                        left,
                        fade,
                    },
                };
            }
            Phase::ToIdentity { from, left } => match left - 1 {
                0 => {
                    self.snap(rate, steps);
                    zero = true;
                }
                left => {
                    self.current =
                        from.mixed_toward(Coefficients::IDENTITY.mix(), fraction(left, steps));
                    self.phase = Phase::ToIdentity { from, left };
                }
            },
        }
        self.running = !(matches!(self.phase, Phase::Settled) && self.current.is_identity());
        zero || (was_running && !self.running)
    }

    /// At an exact identity: takes the target's `g` and `k` and starts fading its mix in, or
    /// rests when the target is identity.
    fn snap(&mut self, rate: f64, steps: u32) {
        match self.target {
            Some(shape) => {
                self.current = Coefficients::of(&shape, rate).at_identity();
                self.phase = Phase::Glide {
                    from: shape,
                    to: shape,
                    left: 0,
                    fade: steps,
                };
            }
            None => {
                self.current = Coefficients::IDENTITY;
                self.phase = Phase::Settled;
            }
        }
    }

    /// Lands on the target at once, for a new format.
    fn settle(&mut self, rate: f64) {
        self.current = self.target.map_or(Coefficients::IDENTITY, |shape| {
            Coefficients::of(&shape, rate)
        });
        self.phase = Phase::Settled;
        self.running = !self.current.is_identity();
    }

    /// Whether the preamp may rise over this slot now: only while it rests or moves its gain
    /// alone, since a fade or a frequency sweep would let the level overshoot.
    fn lets_the_preamp_rise(&self) -> bool {
        match self.phase {
            Phase::Settled => true,
            Phase::Glide { from, to, fade, .. } => {
                fade == 0 && from.frequency == to.frequency && from.width == to.width
            }
            Phase::ToIdentity { .. } => false,
        }
    }

    /// The parameters the slot sounds like now, when it is on one filter's path.
    fn shape(&self, steps: u32) -> Option<Shape> {
        match self.phase {
            Phase::Settled => self.target,
            Phase::Glide { from, to, left, .. } => Some(along(&from, &to, left, steps)),
            Phase::ToIdentity { .. } => None,
        }
    }
}

impl Ramp {
    /// A ramp at rest on `db`.
    fn new(db: f64) -> Self {
        Self {
            current: amplitude(db),
            inverse: amplitude(-db),
            target_db: db,
            target_inverse: amplitude(-db),
            step: 0.,
            left: 0,
        }
    }

    /// Takes a new target and stops, so the next frame starts toward it once it may.
    fn aim(&mut self, target_db: f64) {
        if target_db == self.target_db {
            return;
        }
        self.target_db = target_db;
        self.target_inverse = amplitude(-target_db);
        self.left = 0;
    }

    /// Whether the ramp is off its target and not moving.
    fn waiting(&self) -> bool {
        self.left == 0 && self.inverse != self.target_inverse
    }

    /// Whether the target is above the gain now.
    fn rises(&self) -> bool {
        self.target_inverse < self.inverse
    }

    fn start(&mut self, steps: u32) {
        self.left = steps;
        self.step = (self.target_inverse - self.inverse) / f64::from(steps);
    }

    fn snap(&mut self) {
        self.inverse = self.target_inverse;
        self.current = amplitude(self.target_db);
        self.left = 0;
    }

    fn advance(&mut self) {
        if self.left == 0 {
            return;
        }
        self.left -= 1;
        match self.left {
            0 => self.snap(),
            _ => {
                self.inverse += self.step;
                self.current = 1. / self.inverse;
            }
        }
    }
}

/// How far a ramp with `left` of `steps` frames to go has come.
fn fraction(left: u32, steps: u32) -> f64 {
    f64::from(steps - left) / f64::from(steps)
}

/// The shape a sweep from `from` to `to` is at with `left` of `steps` frames to go, exactly
/// `to` once none are.
fn along(from: &Shape, to: &Shape, left: u32, steps: u32) -> Shape {
    match left {
        0 => *to,
        _ => from.toward(to, fraction(left, steps)),
    }
}

fn amplitude(db: f64) -> f64 {
    10f64.powf(db / 20.)
}

/// Zeroes a state that has decayed below hearing, and one that is not a number at all.
fn flush(state: &mut State) {
    if !state.ic1.is_finite() || !state.ic2.is_finite() {
        *state = State::default();
        return;
    }
    if state.ic1.abs() < FLUSH_BELOW {
        state.ic1 = 0.;
    }
    if state.ic2.abs() < FLUSH_BELOW {
        state.ic2 = 0.;
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;
    use crate::equalizer::design::{Biquad, phi};
    use crate::equalizer::{Curve, Edit, EqualizerWriter, Filter, Kind, Preamp, Preset, Width};

    const RATE: u32 = 48_000;
    const TOLERANCE_DB: f64 = 1e-6;
    /// How far behind the latest edit the chain may be, two ramps after the first of them, at
    /// any frequency, unless a row says otherwise.
    const TRACK_DB: f64 = 0.5;
    /// How much faster than the slowest legal move of a row, from identity to the farthest
    /// coefficients on its path over one ramp, a slot's mix, `g` or `k` may move in one frame.
    /// A sweep's path bows by up to about 2×, and a re-aimed sweep moves to its new target
    /// from wherever it is.
    const SLEW: f64 = 4.;
    /// What a fresh filter's own start-up may add to a tone on top of the steady bound, which a
    /// filter started on another filter's state exceeds many times over.
    const RING: f64 = 0.01;
    /// How finely a sweep's path is sampled for the envelope it must stay under.
    const FINE: u32 = 1_024;
    /// How many frames apart a drag's moves come, 60 a second.
    const DRAG_EVERY: u32 = 800;

    /// One transition: what it starts on, the edits published at a frame, and whether slot 0
    /// is to cross through identity or to sweep along one filter's path.
    struct Transition {
        name: String,
        /// Whether the equalizer starts off and is switched on to `start` at frame 1.
        from_off: bool,
        start: Curve,
        /// Each published just before that frame's `begin`, the first frame being 1.
        edits: Vec<(u32, Curve, Edit)>,
        way: Way,
        /// A tone played through, which after the crossing must stay under what the new filter
        /// allows.
        tone: Option<f64>,
        /// How far behind the latest edit the chain may be from two ramps after the first.
        track: f64,
    }

    enum Way {
        /// Through an exact identity at the end of the first ramp.
        Replace,
        /// On the consistent design of the shape between the two ends, never at identity.
        Glide,
    }

    /// |H| in decibels of a running filter's coefficients, through the bilinear map.
    fn svf_db(c: &Coefficients, frequency: f64, rate: f64) -> f64 {
        if c.g == 0. {
            return 20. * c.m0.abs().log10();
        }
        let w = TAU * frequency / rate;
        let (zr, zi) = (w.cos(), -w.sin());
        let (nr, ni) = (1. - zr, -zi);
        let (dr, di) = (1. + zr, zi);
        let d = dr * dr + di * di;
        let s = ((nr * dr + ni * di) / d / c.g, (ni * dr - nr * di) / d / c.g);
        let s2 = (s.0 * s.0 - s.1 * s.1, 2. * s.0 * s.1);
        let dd = (s2.0 + c.k * s.0 + 1., s2.1 + c.k * s.1);
        let dm = dd.0 * dd.0 + dd.1 * dd.1;
        let bp = (
            (s.0 * dd.0 + s.1 * dd.1) / dm,
            (s.1 * dd.0 - s.0 * dd.1) / dm,
        );
        let lp = (dd.0 / dm, -dd.1 / dm);
        let h = (c.m0 + c.m1 * bp.0 + c.m2 * lp.0, c.m1 * bp.1 + c.m2 * lp.1);
        10. * (h.0 * h.0 + h.1 * h.1).log10()
    }

    /// The five numbers of a slot's coefficients, the three of the mix first.
    fn parts(c: &Coefficients) -> [f64; 5] {
        [c.m0, c.m1, c.m2, c.g, c.k]
    }

    /// A curve of one filter under `preamp`.
    fn under(preamp: Preamp, kind: Kind, frequency: f64, gain: f64, q: f64) -> Curve {
        let filter = Filter::new(kind, frequency, gain, Width::Q(q), true).unwrap();
        Curve::new(preamp, vec![filter]).unwrap()
    }

    fn one(kind: Kind, frequency: f64, gain: f64, q: f64) -> Curve {
        under(Preamp::Manual(0.), kind, frequency, gain, q)
    }

    /// A peak under an automatic preamp.
    fn auto(frequency: f64, gain: f64, q: f64) -> Curve {
        under(Preamp::Auto, Kind::Peak, frequency, gain, q)
    }

    /// What slot 0 is asked for by `curve`: its first filter when that is on.
    fn slot0(curve: &Curve) -> Option<Shape> {
        curve
            .filters()
            .first()
            .filter(|filter| filter.enabled())
            .map(Shape::of)
    }

    fn preamp_db(engine: &FilterStage) -> f64 {
        20. * engine.preamp.current.log10()
    }

    fn build(enabled: bool, curve: Curve, channels: u16) -> (EqualizerWriter, FilterStage) {
        let writer = EqualizerWriter::new(enabled, curve);
        let mut engine = FilterStage::new(writer.reader());
        engine.set_format(RATE, channels);
        engine.begin();
        (writer, engine)
    }

    /// Deterministic noise in the open interval (-1, 1), so a run can be repeated exactly.
    fn noise(seed: &mut u64) -> f32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        ((*seed >> 40) as f32 / (1u64 << 23) as f32) - 1.
    }

    /// A transition from `start` through `edits`, crossing through identity and tracked to
    /// `TRACK_DB`.
    fn row(name: impl Into<String>, start: Curve, edits: Vec<(u32, Curve, Edit)>) -> Transition {
        Transition {
            name: name.into(),
            from_off: false,
            start,
            edits,
            way: Way::Replace,
            tone: None,
            track: TRACK_DB,
        }
    }

    /// A single edit at frame 1.
    fn edit(name: impl Into<String>, from: Curve, to: Curve, edit: Edit, way: Way) -> Transition {
        Transition {
            way,
            ..row(name, from, vec![(1, to, edit)])
        }
    }

    /// A drag of one filter, one glide every `DRAG_EVERY` frames.
    fn drag(
        name: &str,
        start: Curve,
        moves: impl IntoIterator<Item = Curve>,
        way: Way,
        track: f64,
    ) -> Transition {
        let at = |i: usize| DRAG_EVERY * (i as u32 + 1);
        let edits = moves.into_iter().enumerate();
        Transition {
            way,
            track,
            ..row(
                name,
                start,
                edits.map(|(i, c)| (at(i), c, Edit::Glide)).collect(),
            )
        }
    }

    #[test]
    fn a_transition_never_rises_above_the_filters_on_its_path() {
        let rate = f64::from(RATE);
        let steps = (RAMP.as_secs_f64() * rate) as u32;
        let grid: Vec<f64> = (0..100)
            .map(|i| 10. * (0.49 * rate / 10.).powf(f64::from(i) / 99.))
            .collect();
        let peak = |gain: f64| one(Kind::Peak, 1_000., gain, 1.);
        let mut rows: Vec<Transition> = Kind::ALL
            .into_iter()
            .flat_map(|from| {
                Kind::ALL.into_iter().map(move |to| {
                    let name = format!("{from:?} -> {to:?}");
                    let (high, low) = (one(from, 8_000., 12., 0.7), one(to, 200., 12., 0.7));
                    edit(name, high, low, Edit::Replace, Way::Replace)
                })
            })
            .collect();
        for (name, from, to) in [
            (
                "a small move the writer marked as a replace",
                peak(6.),
                peak(7.),
            ),
            ("Flat -> Rock", Preset::Flat.curve(), Preset::Rock.curve()),
            ("Rock -> Flat", Preset::Rock.curve(), Preset::Flat.curve()),
            (
                "a +24 dB peak removed under an automatic preamp",
                auto(1_000., 24., 1.),
                Curve::new(Preamp::Auto, vec![]).unwrap(),
            ),
            (
                "a +24 dB peak made a -24 dB one under an automatic preamp",
                auto(1_000., 24., 1.),
                auto(1_000., -24., 1.),
            ),
        ] {
            rows.push(edit(name, from, to, Edit::Replace, Way::Replace));
        }
        // The preamp against the filters: the level may never pass above where it settles on
        // either side of switching on.
        for (name, curve) in [
            ("a +12 dB peak", auto(1_000., 12., 1.4)),
            (
                "+12, +12, +12 and +6 dB bands",
                Curve::bands(&[12., 12., 12., 6.]),
            ),
        ] {
            let name = format!("switched on to {name} under an automatic preamp");
            rows.push(Transition {
                from_off: true,
                ..row(name, curve, vec![])
            });
        }
        // A drag's `track` allows for the moves still in flight: one ramp of 1920 frames holds
        // 2.4 of them.
        let mut jump = drag(
            "a drag that starts with a jump and streams 1 dB moves",
            peak(1.),
            (1..=12).map(|i| peak(10. + f64::from(i))),
            Way::Replace,
            3.,
        );
        jump.edits.insert(0, (1, peak(10.), Edit::Glide));
        rows.extend([
            jump,
            edit(
                "a kind change asked for as a glide, as a coalesced snapshot might",
                one(Kind::HighShelf, 8_000., 12., 0.7),
                one(Kind::LowShelf, 200., 12., 0.7),
                Edit::Glide,
                Way::Replace,
            ),
            edit(
                "a shelf glide",
                one(Kind::LowShelf, 200., 6., 0.7),
                one(Kind::LowShelf, 300., 2., 1.),
                Edit::Glide,
                Way::Glide,
            ),
            edit(
                "a glide that lowers a +12 dB peak to +6 dB while moving it up half an octave under an automatic preamp",
                auto(1_000., 12., 1.),
                auto(1_400., 6., 1.),
                Edit::Glide,
                Way::Glide,
            ),
            Transition {
                tone: Some(7_777.),
                ..edit(
                    "a low filter replacing a high one must not ring on the old state",
                    one(Kind::Peak, 8_000., 12., 4.),
                    one(Kind::Peak, 30., 24., 2.),
                    Edit::Replace,
                    Way::Replace,
                )
            },
            drag(
                "a drag that raises one band 6 dB and lowers it again under an automatic preamp",
                auto(1_000., 3., 1.),
                (1..=12)
                    .map(|i| auto(1_000., 3. + 0.5 * f64::from(i), 1.))
                    .chain((1..=12).map(|i| auto(1_000., 9. - 0.5 * f64::from(i), 1.))),
                Way::Glide,
                1.5,
            ),
            // A twelfth of an octave is a fifth of an octave on the peak's steepest slope.
            drag(
                "a drag that moves a peak up an octave in twelfths",
                peak(12.),
                (1..=12).map(|i| one(Kind::Peak, 1_000. * 2f64.powf(f64::from(i) / 12.), 12., 1.)),
                Way::Glide,
                4.,
            ),
            drag(
                "a drag that raises a low pass's Q from 0.7 to 2.8 in twelfths of a doubling",
                one(Kind::LowPass, 1_000., 0., 0.7),
                (1..=24).map(|i| one(Kind::LowPass, 1_000., 0., 0.7 * 2f64.powf(f64::from(i) / 12.))),
                Way::Glide,
                1.5,
            ),
        ]);

        let off = Curve::new(Preamp::Manual(0.), Vec::new()).unwrap();
        for row in rows {
            let end = row.edits.last().map_or(1, |(at, ..)| *at) + 3 * steps + 2;
            let first_at = row.edits.first().map_or(1, |(at, ..)| *at);
            let second_at = row.edits.get(1).map_or(u32::MAX, |(at, ..)| *at);
            let points: Vec<&Curve> = std::iter::once(match row.from_off {
                true => &off,
                false => &row.start,
            })
            .chain(row.from_off.then_some(&row.start))
            .chain(row.edits.iter().map(|(_, curve, _)| curve))
            .collect();
            let preamps: Vec<f64> = points.iter().map(|curve| curve.preamp_db()).collect();
            let responses: Vec<_> = points.iter().map(|curve| curve.response(RATE)).collect();
            let targets: Vec<Option<Shape>> = points.iter().map(|curve| slot0(curve)).collect();
            // Slot 0's path under the preamp, which runs with a sweep except that a rise waits
            // for a sweep of frequency or width to end.
            let fine: Vec<(f64, Shape)> = match row.way {
                Way::Replace => preamps
                    .iter()
                    .zip(&targets)
                    .filter_map(|(preamp, shape)| shape.map(|shape| (*preamp, shape)))
                    .collect(),
                Way::Glide => preamps
                    .windows(2)
                    .zip(targets.windows(2))
                    .flat_map(|(db, shapes)| {
                        let (from_db, to_db) = (db[0], db[1]);
                        let (from, to) = (shapes[0].unwrap(), shapes[1].unwrap());
                        let held = to_db > from_db
                            && (from.frequency != to.frequency || from.width != to.width);
                        (0..=FINE).map(move |k| {
                            let t = f64::from(k) / f64::from(FINE);
                            let preamp = match held {
                                true => from_db,
                                false => from_db + (to_db - from_db) * t,
                            };
                            (preamp, from.toward(&to, t))
                        })
                    })
                    .collect(),
            };
            let designs: Vec<(f64, Biquad)> = fine
                .iter()
                .map(|(preamp, shape)| (*preamp, Biquad::of(shape, rate)))
                .collect();
            // A replace passes through dry, so its bound starts at the lower preamp.
            let lowest = preamps.iter().copied().fold(f64::MAX, f64::min);
            let (bound, level_bound): (Vec<f64>, Vec<f64>) = grid
                .iter()
                .map(|f| {
                    let phi = phi(*f, rate);
                    let slot = designs.iter().map(|(_, b)| b.db(phi)).fold(0., f64::max);
                    let at_rest = preamps.iter().zip(&responses).map(|(db, r)| db + r.db(*f));
                    let level = match row.way {
                        Way::Replace => at_rest.fold(lowest, f64::max),
                        Way::Glide => designs
                            .iter()
                            .map(|(preamp, biquad)| preamp + biquad.db(phi))
                            .chain(at_rest)
                            .fold(f64::MIN, f64::max),
                    };
                    (slot, level)
                })
                .unzip();
            // Once the slot has crossed identity only the new filter plays, so a tone may not
            // rise above what that filter or dry allows.
            let tone_bound = row.tone.map(|tone| {
                let last = slot0(points.last().unwrap()).unwrap();
                let db = Biquad::of(&last, rate).db(phi(tone, rate)).max(0.);
                0.5 * 10f64.powf(db / 20.) + RING
            });
            let slew: [f64; 5] = std::array::from_fn(|i| {
                let farthest = fine
                    .iter()
                    .map(|(_, shape)| {
                        (parts(&Coefficients::of(shape, rate))[i]
                            - parts(&Coefficients::IDENTITY)[i])
                            .abs()
                    })
                    .fold(0., f64::max);
                SLEW * farthest / f64::from(steps)
            });
            let starts_running =
                targets[0].is_some_and(|shape| !Coefficients::of(&shape, rate).is_identity());

            let (mut writer, mut engine) = build(!row.from_off, row.start.clone(), 1);
            let mut edits = row.edits.iter().peekable();
            let mut latest = &row.start;
            let mut previous = engine.slots[0].current;
            for frame in 1..=end {
                if row.from_off && frame == 1 {
                    writer.set_enabled(true);
                }
                while let Some((_, curve, edit)) = edits.next_if(|(at, ..)| *at == frame) {
                    writer.set_curve(curve.clone(), *edit);
                    latest = curve;
                }
                engine.begin();
                if let (Some(tone), Some(tone_bound)) = (row.tone, tone_bound) {
                    let x = 0.5 * (TAU * tone * f64::from(frame) / rate).sin() as f32;
                    let y = engine.process(0, x);
                    assert!(
                        frame <= steps || f64::from(y.abs()) <= tone_bound,
                        "{}: frame {frame}: |{y}| over the tone's bound {tone_bound}",
                        row.name
                    );
                }
                let c = engine.slots[0].current;
                // `g` and `k` may jump where the mix is an exact identity, which hears none
                // of them.
                let checked = match previous.is_identity() || c.is_identity() {
                    true => 3,
                    false => 5,
                };
                for (i, (now, before)) in parts(&c)
                    .into_iter()
                    .zip(parts(&previous))
                    .enumerate()
                    .take(checked)
                {
                    assert!(
                        (now - before).abs() <= slew[i],
                        "{}: frame {frame}: coefficient {i} moved {:.6} in one frame, the edit's own pace allows {:.6}",
                        row.name,
                        (now - before).abs(),
                        slew[i]
                    );
                }
                previous = c;
                let rest: Vec<Coefficients> = engine.slots[..engine.used]
                    .iter()
                    .skip(1)
                    .filter(|slot| slot.running)
                    .map(|slot| slot.current)
                    .collect();
                let target = (frame >= 2 * steps).then(|| latest.response(RATE));
                // A sweep's first ramp keeps slot 0 on the path's own design.
                let on_path = match row.way {
                    Way::Glide
                        if (first_at..first_at + steps).contains(&frame) && frame < second_at =>
                    {
                        let t = f64::from(frame - first_at + 1) / f64::from(steps);
                        let along = targets[0].unwrap().toward(&targets[1].unwrap(), t);
                        Some(Biquad::of(&along, rate))
                    }
                    _ => None,
                };
                for (j, f) in grid.iter().enumerate() {
                    let slot = svf_db(&c, *f, rate);
                    let chain = slot + rest.iter().map(|c| svf_db(c, *f, rate)).sum::<f64>();
                    assert!(
                        slot <= bound[j] + TOLERANCE_DB,
                        "{}: frame {frame} at {f:.0} Hz: {slot:.6} dB over {:.6} dB",
                        row.name,
                        bound[j]
                    );
                    let level = preamp_db(&engine) + chain;
                    assert!(
                        level <= level_bound[j] + TOLERANCE_DB,
                        "{}: frame {frame} at {f:.0} Hz: the level {level:.6} dB is over {:.6} dB, the highest it settles at",
                        row.name,
                        level_bound[j]
                    );
                    if let Some(target) = &target {
                        let want = target.db(*f);
                        assert!(
                            (chain - want).abs() <= row.track,
                            "{}: frame {frame} at {f:.0} Hz: {chain:.3} dB, the latest edit asks {want:.3} dB",
                            row.name
                        );
                    }
                    if let Some(design) = &on_path {
                        let want = design.db(phi(*f, rate));
                        assert!(
                            (slot - want).abs() <= TOLERANCE_DB,
                            "{}: frame {frame} at {f:.0} Hz: {slot:.6} dB off the path's {want:.6} dB",
                            row.name
                        );
                    }
                }
                match row.way {
                    Way::Replace => assert!(
                        frame != steps || !starts_running || c.is_identity(),
                        "{}: not at identity after one ramp",
                        row.name
                    ),
                    Way::Glide => {
                        assert!(!c.is_identity(), "{}: frame {frame} at identity", row.name)
                    }
                }
            }
            let target = slot0(latest).map_or(Coefficients::IDENTITY, |shape| {
                Coefficients::of(&shape, rate)
            });
            assert_eq!(
                engine.slots[0].current, target,
                "{}: not exactly on target",
                row.name
            );
            assert!(
                (preamp_db(&engine) - latest.preamp_db()).abs() <= TOLERANCE_DB,
                "{}: the preamp rests at {} dB, not {} dB",
                row.name,
                preamp_db(&engine),
                latest.preamp_db()
            );
        }
    }

    /// Plays `frames` frames on two channels, each sample from `sample`.
    fn play(engine: &mut FilterStage, frames: u32, mut sample: impl FnMut() -> f32) {
        for _ in 0..frames {
            engine.begin();
            for channel in 0..2 {
                engine.process(channel, sample());
            }
        }
    }

    /// Plays the same noise through both stages, which must agree bit for bit.
    fn same(
        engine: &mut FilterStage,
        fresh: &mut FilterStage,
        frames: u32,
        seed: &mut u64,
        what: &str,
    ) {
        for i in 0..frames {
            engine.begin();
            fresh.begin();
            for channel in 0..2 {
                let sample = noise(seed);
                let (out, want) = (
                    engine.process(channel, sample),
                    fresh.process(channel, sample),
                );
                assert_eq!(
                    out.to_bits(),
                    want.to_bits(),
                    "{what}: frame {i} channel {channel}"
                );
            }
        }
    }

    /// Plays awkward values, then noise, which must come out bit for bit as they went in.
    fn through(engine: &mut FilterStage, seed: &mut u64, what: &str) {
        let odd = [
            0.,
            -0.,
            1.,
            -1.,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            f32::NAN,
            f32::INFINITY,
        ];
        for i in 0..4_000 {
            engine.begin();
            for channel in 0..2 {
                let sample = odd
                    .get(i * 2 + channel)
                    .copied()
                    .unwrap_or_else(|| noise(seed));
                let out = engine.process(channel, sample);
                assert_eq!(
                    out.to_bits(),
                    sample.to_bits(),
                    "{what}: frame {i} channel {channel}"
                );
            }
        }
    }

    #[test]
    fn samples_pass_through_bit_for_bit_once_the_equalizer_is_off() {
        let mut seed = 0x9E37_79B9_7F4A_7C15;
        let (mut writer, mut engine) = build(true, Preset::Rock.curve(), 2);
        play(&mut engine, RATE / 2, || noise(&mut seed));
        writer.set_enabled(false);
        play(&mut engine, RATE / 5, || noise(&mut seed));
        through(&mut engine, &mut seed, "switched off");

        let (mut fresh_writer, mut fresh) = build(false, Preset::Rock.curve(), 2);
        writer.set_enabled(true);
        fresh_writer.set_enabled(true);
        same(
            &mut engine,
            &mut fresh,
            RATE / 2,
            &mut seed,
            "switched on again",
        );

        // A filter that glided to 0 dB stops running and forgets what it heard, so gliding it
        // back up sounds exactly like a fresh stage. The preamp keeps the chain busy meanwhile.
        let dip = |gain| under(Preamp::Manual(-6.), Kind::Peak, 1_000., gain, 1.);
        let (mut writer, mut engine) = build(true, dip(6.), 2);
        writer.set_curve(dip(0.), Edit::Glide);
        play(&mut engine, RATE / 5, || noise(&mut seed));
        play(&mut engine, RATE / 5, || 0.);
        let (mut fresh_writer, mut fresh) = build(true, dip(0.), 2);
        writer.set_curve(dip(6.), Edit::Glide);
        fresh_writer.set_curve(dip(6.), Edit::Glide);
        same(
            &mut engine,
            &mut fresh,
            RATE / 5,
            &mut seed,
            "glided back up from 0 dB",
        );

        let (_writer, mut engine) = build(true, Preset::Flat.curve(), 2);
        through(&mut engine, &mut seed, "flat and on");
    }

    #[test]
    fn a_decayed_filter_is_flushed_to_exact_zero() {
        let shelf = one(Kind::LowShelf, 10., 24., 0.05);
        let writer = EqualizerWriter::new(true, shelf);
        let mut engine = FilterStage::new(writer.reader());
        engine.set_format(44_100, 1);
        engine.begin();
        engine.process(0, 1.);
        for _ in 0..1_500_000 {
            engine.begin();
            engine.process(0, 0.);
        }
        assert!(
            engine
                .state
                .iter()
                .flatten()
                .all(|state| *state == State::default())
        );
    }
}
