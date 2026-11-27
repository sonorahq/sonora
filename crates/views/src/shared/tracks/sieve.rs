use std::collections::{BTreeMap, BTreeSet};

use gpui::SharedString;
use music::Track;
use ui::{Filter, Unit};

use crate::chrome::tools;

/// A track label accessor.
type Words = fn(&Track) -> &[String];

/// An optional numeric track field.
type Reading = fn(&Track) -> Option<f32>;

/// Continuous tempo filter, displayed in whole BPM.
pub(super) const BPM: &str = "filter-bpm";

/// Continuous duration filter.
pub(super) const DURATION: &str = "filter-duration";

/// Numeric filter keys, accessors and display units.
pub(super) const RANGES: [(&str, Reading, Unit); 5] = [
    (
        "filter-duration",
        |track| Some(track.duration.as_secs_f32()),
        Unit::Clock,
    ),
    (
        "filter-year",
        |track| track.release_date.map(|date| date.year() as f32),
        Unit::Plain,
    ),
    (
        "filter-play-count",
        |track| track.playcount.map(|count| count as f32),
        Unit::Plain,
    ),
    (
        "filter-popularity",
        |track| (track.popularity > 0).then_some(track.popularity as f32),
        Unit::Plain,
    ),
    (BPM, |track| track.bpm.map(f32::round), Unit::Plain),
];

/// Choice filter keys and label accessors.
pub(super) const FACETS: [(&str, Words); 3] = [
    ("filter-genre", |track| &track.genres),
    ("filter-mood", |track| &track.moods),
    ("filter-tag", |track| &track.tags),
];

/// Resolves a filter's label independently of its persisted key.
pub(super) fn filter_label(key: &str) -> SharedString {
    i18n::lookup(
        match key {
            BPM => "common-bpm",
            "filter-mood" => "common-mood",
            "filter-genre" => "column-genre",
            "filter-popularity" => "song-popularity",
            _ => key,
        },
        None,
    )
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct TrackSieve {
    /// Active numeric ranges by filter key.
    pub ranges: BTreeMap<&'static str, (f32, f32)>,
    /// Selected option ids by filter key; empty selections have no entry.
    pub picks: BTreeMap<&'static str, BTreeSet<String>>,
    /// Drop tracks marked explicit.
    pub hide_explicit: bool,
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

    /// Flags supported by this track source.
    pub(super) fn flags(&self, catalog: bool, downloads: bool) -> Vec<Filter> {
        let mut flags = vec![
            tools::flag("filter-hide-explicit", self.hide_explicit),
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

    pub(super) fn keeps(&self, track: &Track) -> bool {
        if self.hide_explicit && track.explicit {
            return false;
        }
        if self.collaborations && track.artist_refs.len() < 2 {
            return false;
        }
        RANGES.iter().all(|(key, read, _)| {
            self.ranges.get(key).is_none_or(|bounds| {
                read(track)
                    .is_some_and(|value| value >= bounds.0 - SLACK && value <= bounds.1 + SLACK)
            })
        }) && FACETS.iter().all(|(key, words)| {
            self.picks
                .get(key)
                .is_none_or(|picked| words(track).iter().any(|word| picked.contains(word)))
        })
    }
}

/// Endpoint tolerance for sliders rounded to whole seconds or BPM.
const SLACK: f32 = 0.5;

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
        assert!(sieve.keeps(&track(60, false, true)));
        assert!(sieve.keeps(&track(180, false, true)));
        assert!(sieve.keeps(&track(120, false, true)));
        assert!(!sieve.keeps(&track(59, false, true)));
        assert!(!sieve.keeps(&track(181, false, true)));
    }

    #[test]
    fn every_axis_must_pass() {
        let sieve = TrackSieve {
            ranges: BTreeMap::from([("filter-duration", (60., 180.))]),
            hide_explicit: true,
            favorites: false,
            downloaded: false,
            ..TrackSieve::default()
        };

        assert!(sieve.keeps(&track(120, false, true)));
        assert!(!sieve.keeps(&track(120, true, true)));
        assert!(!sieve.keeps(&track(400, false, true)));
    }
}
