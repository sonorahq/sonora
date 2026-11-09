use std::collections::VecDeque;
use std::time::{Duration, Instant};

use music::equalizer::PREAMP_RANGE;

/// How long the clip warning stays up after the last read that went over.
const HOLD: Duration = Duration::from_secs(2);
/// How long the warning's figure holds still before it may show a newer worst over.
const REFRESH: Duration = Duration::from_secs(1);
/// How long reads are ignored after the level changed: the gain's ramp and the device's buffer,
/// so the samples already on their way out when it changed are not counted.
const SETTLE: Duration = Duration::from_millis(250);
/// How far below the measured over a lowered preamp goes, for peaks the meter has not met yet.
const MARGIN: f64 = 0.5;
/// What a preamp in tenths of a decibel may miss a tenth by and still be that tenth, since
/// `8.2` comes out of the arithmetic as `8.1999…`.
const EPSILON: f64 = 1e-6;
/// The lowest level the output meter reads, in dBFS. Silence rests here, so a paused track
/// stops redrawing the meter once it has fallen.
const FLOOR: f32 = -60.;
/// How fast the meter's level and its peak fall, in decibels a second.
const FALL: f32 = 24.;
/// How long the meter's peak tick holds the loudest read before it starts to fall.
const PEAK_HOLD: Duration = Duration::from_millis(1500);
/// The step the meter's readings are rounded to, in decibels, so a change too small to see
/// redraws nothing.
const STEP: f32 = 0.5;

/// How far the output went over full scale lately, as the meter saw it, and the clip warning
/// made of it. The warning comes up on the first over, stays for the hold after the last one,
/// and its figure changes at most once per refresh.
#[derive(Default)]
pub(crate) struct Overload {
    /// Each read that went over, in decibels, and when, oldest first and none past the hold.
    overs: VecDeque<(Instant, f64)>,
    /// Until when reads are ignored, after the level changed.
    settling: Option<Instant>,
    /// The over the warning shows, to a tenth of a decibel, and when that figure was set.
    shown: Option<(f64, Instant)>,
}

/// The output's recent level as a meter draws it, in dBFS: the level falling smoothly after
/// each peak, and the loudest read held a moment before it falls too. Kept apart from what other
/// models follow, so the meter redraws on every read without waking everything that watches
/// playback.
pub struct OutputLevel {
    level: f32,
    peak: f32,
    /// When the peak was last raised, which its hold counts from.
    peaked: Option<Instant>,
    /// When the last read was taken, which the fall is measured from.
    read: Option<Instant>,
}

impl Overload {
    /// Takes in one meter read, the loudest sample since the one before, made at `now`.
    /// Returns whether the warning changed.
    pub(crate) fn observe(&mut self, peak: f32, now: Instant) -> bool {
        let before = self.db();
        self.overs
            .retain(|(seen, _)| now.duration_since(*seen) < HOLD);
        let settling = self.settling.is_some_and(|until| now < until);
        if !settling && peak > 1. {
            self.overs.push_back((now, 20. * f64::from(peak).log10()));
        }
        self.shown = match (self.worst().map(tenth), self.shown) {
            (None, _) => None,
            (Some(worst), None) => Some((worst, now)),
            (Some(worst), Some((shown, since)))
                if worst != shown && now.duration_since(since) >= REFRESH =>
            {
                Some((worst, now))
            }
            (Some(_), shown) => shown,
        };
        self.db() != before
    }

    /// Forgets every over and takes the warning down, and ignores reads for a moment, since the
    /// level just changed and what was measured before no longer says what plays.
    pub(crate) fn settle(&mut self, now: Instant) {
        self.overs.clear();
        self.shown = None;
        self.settling = Some(now + SETTLE);
    }

    /// The over the warning shows, in decibels to a tenth, or `None` while it is down.
    pub(crate) fn db(&self) -> Option<f64> {
        self.shown.map(|(db, _)| db)
    }

    /// The manual preamp that takes `preamp_db` down by the worst over within the hold and a
    /// margin, floored to a tenth of a decibel. `None` without an over, or when the preamp
    /// cannot go lower.
    pub(crate) fn lowered(&self, preamp_db: f64) -> Option<f64> {
        let target = preamp_db - self.worst()? - MARGIN;
        let target = ((target * 10. + EPSILON).floor() / 10.).max(*PREAMP_RANGE.start());
        (target < preamp_db).then_some(target)
    }

    /// The worst over within the hold, in decibels, unrounded.
    fn worst(&self) -> Option<f64> {
        self.overs.iter().map(|(_, db)| *db).reduce(f64::max)
    }
}

impl Default for OutputLevel {
    fn default() -> Self {
        Self {
            level: FLOOR,
            peak: FLOOR,
            peaked: None,
            read: None,
        }
    }
}

impl OutputLevel {
    /// The level in dBFS, between the floor and whatever the output reached over full scale.
    pub fn level_db(&self) -> f32 {
        self.level
    }

    /// The loudest recent level in dBFS, held a moment and then falling.
    pub fn peak_db(&self) -> f32 {
        self.peak
    }

    /// Takes in one meter read, the loudest sample since the one before, made at `now`. Returns
    /// whether either reading moved by a visible step, which is when the meter has to redraw.
    pub(crate) fn observe(&mut self, peak: f32, now: Instant) -> bool {
        let fallen = self
            .read
            .map_or(0., |read| now.duration_since(read).as_secs_f32() * FALL);
        self.read = Some(now);
        let heard = stepped(20. * peak.max(f32::MIN_POSITIVE).log10());
        let level = heard.max(stepped(self.level - fallen));
        let held = self
            .peaked
            .is_some_and(|peaked| now.duration_since(peaked) < PEAK_HOLD);
        let peak = match heard >= self.peak {
            true => {
                self.peaked = Some(now);
                heard
            }
            false => match held {
                true => self.peak,
                false => level.max(stepped(self.peak - fallen)),
            },
        };
        if (level, peak) == (self.level, self.peak) {
            return false;
        }
        (self.level, self.peak) = (level, peak);
        true
    }

    /// Drops every reading back to the floor, for a meter that starts watching afresh.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// `db` rounded to a tenth, the precision the warning shows.
fn tenth(db: f64) -> f64 {
    (db * 10.).round() / 10.
}

/// `db` rounded to the meter's step and kept off the floor.
fn stepped(db: f32) -> f32 {
    ((db / STEP).round() * STEP).max(FLOOR)
}
