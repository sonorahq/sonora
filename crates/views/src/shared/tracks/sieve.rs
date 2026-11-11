use std::collections::{BTreeMap, BTreeSet};

use music::Track;
use ui::{Filter, Unit};

use super::sort::{Found, described, genres_of};
use crate::chrome::tools;

/// Reads the words of one kind from a track's descriptors.
type Words = for<'a> fn(&'a Track, Found<'a>) -> &'a [String];

/// Reads a number off a track, or nothing when the track has none.
type Reading = fn(&Track, Found<'_>) -> Option<f32>;

/// The key of the tempo axis, which comes from a track's descriptors and not from the track.
pub(super) const BPM: &str = "filter-bpm";

/// The key of the duration axis, which slides freely where the others snap to the values present.
pub(super) const DURATION: &str = "filter-duration";

/// The numeric filters over a track's own fields, with the unit each is shown in.
pub(super) const RANGES: [(&str, Reading, Unit); 5] = [
    (
        "filter-duration",
        |track, _| Some(track.duration.as_secs_f32()),
        Unit::Clock,
    ),
    (
        "filter-year",
        |track, _| (track.year > 0).then_some(track.year as f32),
        Unit::Plain,
    ),
    (
        "filter-play-count",
        |track, _| track.playcount.map(|count| count as f32),
        Unit::Plain,
    ),
    (
        "filter-popularity",
        |track, _| (track.popularity > 0).then_some(track.popularity as f32),
        Unit::Plain,
    ),
    (
        BPM,
        |track, found| described(track, found)?.bpm.map(f32::round),
        Unit::Plain,
    ),
];

/// The choice filters by axis key, each reading the words a track's descriptors give it.
pub(super) const FACETS: [(&str, Words); 3] = [
    ("filter-genre", genres_of),
    ("filter-mood", |track, found| {
        described(track, found)
            .map(|d| d.moods.as_slice())
            .unwrap_or_default()
    }),
    ("filter-tag", |track, found| {
        described(track, found)
            .map(|d| d.tags.as_slice())
            .unwrap_or_default()
    }),
];

#[derive(Clone, Default, PartialEq)]
pub(crate) struct TrackSieve {
    /// The narrowed range of each numeric axis by key: the track fields in `RANGES` and `BPM`.
    pub ranges: BTreeMap<&'static str, (f32, f32)>,
    /// The ids picked on each choice filter. An axis nothing is picked on has no entry.
    pub picks: BTreeMap<&'static str, BTreeSet<String>>,
    /// Drop tracks marked explicit.
    pub hide_explicit: bool,
    pub playable: bool,
    /// Keep only starred tracks. `keeps` cannot see the library, so the source applies it.
    pub favorites: bool,
    /// Keep only downloaded tracks, which the source applies for the same reason.
    pub downloaded: bool,
    pub collaborations: bool,
}

impl TrackSieve {
    pub(crate) fn active(&self) -> bool {
        self != &Self::default()
    }

    /// The flag definitions used by every track table; unavailable library/download controls stay out.
    pub(super) fn flags(&self, catalog: bool, downloads: bool) -> Vec<Filter> {
        let mut flags = vec![
            tools::flag("filter-hide-explicit", self.hide_explicit),
            tools::flag("filter-playable", self.playable),
            tools::flag("filter-collaborations", self.collaborations),
        ];
        if catalog {
            flags.push(tools::flag("filter-favorites", self.favorites));
        }
        if downloads {
            flags.push(tools::flag("filter-downloaded", self.downloaded));
        }
        flags
    }

    /// Picks or drops one option of a choice filter.
    pub(super) fn pick(&mut self, key: &'static str, id: &str, on: bool) {
        let picked = self.picks.entry(key).or_default();
        match on {
            true => picked.insert(id.to_owned()),
            false => picked.remove(id),
        };
        if picked.is_empty() {
            self.picks.remove(key);
        }
    }

    pub(super) fn keeps(&self, track: &Track, found: Found) -> bool {
        if self.hide_explicit && track.explicit {
            return false;
        }
        if self.collaborations && track.artist_refs.len() < 2 {
            return false;
        }
        if self.playable && !track.playable {
            return false;
        }
        RANGES
            .iter()
            .all(|(key, read, _)| keeps_range(self.ranges.get(key).copied(), read(track, found)))
            && FACETS.iter().all(|(key, words)| {
                self.picks.get(key).is_none_or(|picked| {
                    words(track, found).iter().any(|word| picked.contains(word))
                })
            })
    }
}

/// Whether `value` is inside `range`, with no range meaning untouched.
fn keeps_range(range: Option<(f32, f32)>, value: Option<f32>) -> bool {
    range.is_none_or(|bounds| value.is_some_and(|value| within(value, bounds)))
}

/// Half a unit of slack, so a slider snapped to a whole second or tempo keeps its own end.
const SLACK: f32 = 0.5;

fn within(value: f32, (low, high): (f32, f32)) -> bool {
    value >= low - SLACK && value <= high + SLACK
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::TrackSieve;
    use crate::shared::tracks::fixture::track;

    #[test]
    fn duration_bounds_are_inclusive() {
        let sieve = TrackSieve {
            ranges: BTreeMap::from([("filter-duration", (60., 180.))]),
            ..TrackSieve::default()
        };

        assert!(sieve.active());
        assert!(sieve.keeps(&track(60, false, true), None));
        assert!(sieve.keeps(&track(180, false, true), None));
        assert!(sieve.keeps(&track(120, false, true), None));
        assert!(!sieve.keeps(&track(59, false, true), None));
        assert!(!sieve.keeps(&track(181, false, true), None));
    }

    #[test]
    fn every_axis_must_pass() {
        let sieve = TrackSieve {
            ranges: BTreeMap::from([("filter-duration", (60., 180.))]),
            hide_explicit: true,
            playable: true,
            favorites: false,
            downloaded: false,
            ..TrackSieve::default()
        };

        assert!(sieve.keeps(&track(120, false, true), None));
        assert!(!sieve.keeps(&track(120, true, true), None));
        assert!(!sieve.keeps(&track(120, false, false), None));
        assert!(!sieve.keeps(&track(400, false, true), None));
    }
}
