//! The rules of a station, the radio a queue is topped up from so it never runs dry.

use std::collections::HashSet;

use music::Track;

/// How few tracks may be left to play before the next stretch of radio is asked for. Asking this
/// early keeps the wait for the station off the gap between two tracks.
pub const LOOKAHEAD: usize = 10;

/// How many continuations in a row may bring nothing the queue has not heard before the station
/// is dropped. An empty stretch lands with the queue still short, which asks for the next one
/// straight away, so a provider serving the same tracks over would otherwise never be let go of.
pub const DRY_LIMIT: u8 = 3;

/// How many stretches in a row have brought nothing new once one lands with `fresh` tracks the
/// queue had not heard, `dry` being the count before it.
pub fn dry_after(dry: u8, fresh: usize) -> u8 {
    match fresh {
        0 => dry.saturating_add(1),
        _ => 0,
    }
}

/// Whether a station that came back empty `dry` times in a row is dropped.
pub fn ran_dry(dry: u8) -> bool {
    dry >= DRY_LIMIT
}

/// Whether a queue with `ahead` tracks left to play is short enough to ask for more.
pub fn runs_low(ahead: usize) -> bool {
    ahead < LOOKAHEAD
}

/// Keeps the playable tracks that are not among `heard`, the ids a queue already holds.
pub fn unheard(tracks: &mut Vec<Track>, heard: &HashSet<String>) {
    tracks.retain(|track| {
        track.playable
            && track
                .id
                .as_ref()
                .is_some_and(|id| !heard.contains(id.as_str()))
    });
}

/// A station a queue is following: the track it grew from, the continuation its next stretch is
/// fetched with, and how many stretches in a row brought nothing new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Station {
    seed: String,
    next: String,
    dry: u8,
}

impl Station {
    /// The station grown from `seed`, which the provider continues at `next`.
    pub fn new(seed: impl Into<String>, next: impl Into<String>) -> Self {
        Self {
            seed: seed.into(),
            next: next.into(),
            dry: 0,
        }
    }

    /// The id of the track the station grew from.
    pub fn seed(&self) -> &str {
        &self.seed
    }

    /// The continuation the next stretch is asked for with.
    pub fn next(&self) -> &str {
        &self.next
    }

    /// The station after a stretch landed with `fresh` tracks the queue had not heard and the
    /// continuation `next`. It is gone when the provider has no more of it, or when it came back
    /// empty [`DRY_LIMIT`] times in a row.
    pub fn landed(self, fresh: usize, next: Option<String>) -> Option<Self> {
        let dry = dry_after(self.dry, fresh);
        next.filter(|_| !ran_dry(dry)).map(|next| Self {
            seed: self.seed,
            next,
            dry,
        })
    }
}
