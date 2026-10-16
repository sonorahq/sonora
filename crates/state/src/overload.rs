use std::collections::VecDeque;
use std::time::{Duration, Instant};

use music::equalizer::PREAMP_RANGE;

/// How long an over stays reported after the read that measured it.
const HOLD: Duration = Duration::from_secs(2);
/// How long reads are ignored after the level changed: the gain's ramp and the device's buffer,
/// so the samples already on their way out when it changed are not counted.
const SETTLE: Duration = Duration::from_millis(250);
/// How far below the measured over a lowered preamp goes, for peaks the meter has not met yet.
const MARGIN: f64 = 0.5;
/// What a preamp in tenths of a decibel may miss a tenth by and still be that tenth, since
/// `8.2` comes out of the arithmetic as `8.1999…`.
const EPSILON: f64 = 1e-6;

/// How far the output went over full scale lately, as the meter saw it: the worst over within
/// the hold, with reads ignored for a moment after the level changed.
#[derive(Default)]
pub(crate) struct Overload {
    /// Each read that went over, in decibels, and when, oldest first and none past the hold.
    overs: VecDeque<(Instant, f64)>,
    /// Until when reads are ignored, after the level changed.
    settling: Option<Instant>,
}

impl Overload {
    /// Takes in one meter read, the loudest sample since the one before, made at `now`.
    /// Returns whether `db` changed.
    pub(crate) fn observe(&mut self, peak: f32, now: Instant) -> bool {
        let before = self.db();
        self.overs
            .retain(|(seen, _)| now.duration_since(*seen) < HOLD);
        let settling = self.settling.is_some_and(|until| now < until);
        if !settling && peak > 1. {
            self.overs.push_back((now, 20. * f64::from(peak).log10()));
        }
        self.db() != before
    }

    /// Forgets every over and ignores reads for a moment, since the level just changed and what
    /// was measured before no longer says what plays.
    pub(crate) fn settle(&mut self, now: Instant) {
        self.overs.clear();
        self.settling = Some(now + SETTLE);
    }

    /// The worst over within the hold, in decibels.
    pub(crate) fn db(&self) -> Option<f64> {
        self.overs.iter().map(|(_, db)| *db).reduce(f64::max)
    }

    /// The manual preamp that takes `preamp_db` down by the worst over and a margin, floored to
    /// a tenth of a decibel. `None` without an over, or when the preamp cannot go lower.
    pub(crate) fn lowered(&self, preamp_db: f64) -> Option<f64> {
        let target = preamp_db - self.db()? - MARGIN;
        let target = ((target * 10. + EPSILON).floor() / 10.).max(*PREAMP_RANGE.start());
        (target < preamp_db).then_some(target)
    }
}
