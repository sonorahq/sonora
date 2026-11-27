//! Aggregation of a timestamped play history, shared by Stats consumers.

use std::collections::{HashMap, HashSet};

use music::{
    Album, Board, Descriptors, HourMood, ListeningStats, Milestone, Overview, SavedArtist,
    TimeSlice, Track, TrackBreakdown, TrackTotals, album_of, compare, credits, infer_mood,
    milestone_numbers,
};

const DENSITY_BUCKETS: usize = 64;

/// A retained play log and the statistics that are independent of the selected dates.
#[derive(Default)]
pub struct RecordedHistory {
    pub tracks: Vec<Track>,
    pub overview: Overview,
    pub bounds: Option<(i64, i64)>,
    pub density: Vec<usize>,
    milestones: Vec<Milestone>,
    first_plays: Firsts,
}

impl RecordedHistory {
    /// Computes the all-history information once when the play log changes.
    pub fn new(tracks: Vec<Track>) -> Self {
        let times: Vec<_> = tracks.iter().filter_map(|track| track.added_at).collect();
        let bounds = times.iter().copied().min().zip(times.iter().copied().max());
        Self {
            overview: Overview::default().timed(&times),
            bounds,
            density: bounds.map_or_else(Vec::new, |bounds| slice_plays(&tracks, bounds)),
            milestones: milestones_of(&tracks),
            first_plays: first_plays(&tracks),
            tracks,
        }
    }

    /// Builds a ranking and compares it to the preceding window of the same length.
    pub fn stats(
        &self,
        dates: Option<(i64, i64)>,
        descriptors: &HashMap<String, Descriptors>,
    ) -> ListeningStats {
        let mut data = aggregate(&self.tracks, &self.first_plays, dates, descriptors);
        if let Some((from, to)) = dates {
            let range = Some((from - (to - from + 1), from - 1));
            let before = aggregate(&self.tracks, &self.first_plays, range, descriptors);
            compare_with(&mut data, &before);
        }
        data.milestones.clone_from(&self.milestones);
        data
    }
}

/// Counts dated plays, preferring provider descriptors and crediting each artist, genre and mood.
fn aggregate(
    tracks: &[Track],
    first_play: &Firsts,
    dates: Option<(i64, i64)>,
    descriptors: &HashMap<String, Descriptors>,
) -> ListeningStats {
    let (mut songs, mut artists, mut albums) =
        (Board::default(), Board::default(), Board::default());
    let mut breakdown = TrackBreakdown::default();
    let mut totals = TrackTotals::default();
    let mut times = Vec::new();
    let mut new_artists = HashSet::new();
    let mut new_tracks = 0;
    let mut play_samples = Vec::new();
    for track in tracks {
        let (Some(at), Some(id)) = (track.added_at, track.id.as_ref()) else {
            continue;
        };
        if dates.is_some_and(|(from, to)| at < from || at > to) {
            continue;
        }
        totals.add(track, 1);
        new_tracks += usize::from(first_play.tracks.get(id) == Some(&at));
        times.push(at);
        songs.add(id.clone(), 1, || track.clone());
        if let Some((key, album)) = album_of(track) {
            albums.add(key, 1, || album);
        }
        for (key, artist) in credits(track) {
            if first_play.artists.get(&key) == Some(&at) {
                new_artists.insert(key.clone());
            }
            artists.add(key, 1, || artist);
        }
        let descriptor = descriptors.get(id);
        let bpm = breakdown.add(track, descriptor, 1);
        let (valence, energy) = descriptor
            .map(|found| {
                let (inferred_v, inferred_e) = infer_mood(&found.moods, &found.tags);
                (found.valence.or(inferred_v), found.energy.or(inferred_e))
            })
            .unwrap_or_else(|| infer_mood(&track.moods, &track.tags));
        play_samples.push(PlaySample {
            at,
            valence,
            energy,
            bpm,
        });
    }

    let plays = totals.count;
    let overview = match plays {
        0 => Overview::default(),
        _ => Overview {
            plays: Some(plays),
            time: Some(totals.seconds),
            tracks: Some(songs.len()),
            artists: Some(artists.len()),
            albums: Some(albums.len()),
            new_artists: Some(new_artists.len()),
            new_tracks: Some(new_tracks),
            replay_share: Some((plays.saturating_sub(songs.len()) * 100 / plays) as u8),
            avg_length: totals.avg_length(),
            collab_share: totals.collab_share(),
            ..Default::default()
        }
        .timed(&times),
    };
    let mut stats = ListeningStats {
        tracks: songs.ranked(),
        artists: artists.ranked(),
        albums: albums.ranked(),
        timeline: build_timeline(&play_samples, dates),
        hourly_mood: build_hourly_mood(&play_samples),
        overview,
        ..Default::default()
    };
    breakdown.apply(&mut stats);
    stats
}

struct PlaySample {
    at: i64,
    valence: Option<f32>,
    energy: Option<f32>,
    bpm: Option<f32>,
}

/// Running averages for the mood and tempo series.
#[derive(Clone, Default)]
struct Samples {
    sums: [f32; 3],
    counts: [usize; 3],
    bpm_min: Option<f32>,
    bpm_max: Option<f32>,
    plays: usize,
}

impl Samples {
    fn add(&mut self, sample: &PlaySample) {
        for (index, value) in [sample.valence, sample.energy, sample.bpm]
            .into_iter()
            .enumerate()
        {
            if let Some(value) = value {
                self.sums[index] += value;
                self.counts[index] += 1;
            }
        }
        if let Some(bpm) = sample.bpm {
            self.bpm_min = Some(self.bpm_min.map_or(bpm, |min| min.min(bpm)));
            self.bpm_max = Some(self.bpm_max.map_or(bpm, |max| max.max(bpm)));
        }
        self.plays += 1;
    }

    fn means(&self) -> [Option<f32>; 3] {
        std::array::from_fn(|index| {
            let count = self.counts[index];
            (count > 0).then(|| self.sums[index] / count as f32)
        })
    }
}

/// Buckets plays chronologically for the mood and tempo charts.
fn build_timeline(samples: &[PlaySample], dates: Option<(i64, i64)>) -> Vec<TimeSlice> {
    if samples.len() < 2 {
        return Vec::new();
    }
    let (min_at, max_at) = match dates {
        Some((from, to)) if to > from => (from, to),
        _ => (
            samples.iter().map(|s| s.at).min().unwrap_or(0),
            samples.iter().map(|s| s.at).max().unwrap_or(0),
        ),
    };
    let span = (max_at - min_at).max(1);
    let buckets = ((span / 86_400).max(1) as usize).min(24);
    let mut acc = vec![Samples::default(); buckets];
    for sample in samples {
        let clamped = sample.at.clamp(min_at, max_at);
        let slot = (((clamped - min_at) as f64 / span as f64) * buckets as f64) as usize;
        acc[slot.min(buckets - 1)].add(sample);
    }
    let slices: Vec<TimeSlice> = acc
        .into_iter()
        .enumerate()
        .filter(|(_, entry)| entry.plays > 0)
        .map(|(slot, entry)| {
            let [valence, energy, bpm] = entry.means();
            TimeSlice {
                at: min_at + (span * slot as i64) / buckets as i64,
                valence,
                energy,
                bpm,
                bpm_min: entry.bpm_min,
                bpm_max: entry.bpm_max,
                plays: entry.plays,
            }
        })
        .collect();
    match slices.len() >= 2 {
        true => slices,
        false => Vec::new(),
    }
}

/// Averages positivity and energy across each local hour of the day (`0..24`).
fn build_hourly_mood(samples: &[PlaySample]) -> Vec<HourMood> {
    let zone = jiff::tz::TimeZone::system();
    let mut acc: [Samples; 24] = std::array::from_fn(|_| Samples::default());
    for sample in samples {
        if sample.valence.is_none() && sample.energy.is_none() {
            continue;
        }
        let Ok(stamp) = jiff::Timestamp::from_second(sample.at) else {
            continue;
        };
        let hour = stamp.to_zoned(zone.clone()).hour() as usize;
        acc[hour].add(sample);
    }
    let has_data = acc.iter().filter(|entry| entry.plays > 0).count() >= 2;
    if !has_data {
        return Vec::new();
    }
    acc.into_iter()
        .map(|entry| {
            let [valence, energy, _] = entry.means();
            HourMood {
                valence,
                energy,
                plays: entry.plays,
            }
        })
        .collect()
}

/// When each artist and each song was first played in the log.
#[derive(Default)]
struct Firsts {
    artists: HashMap<String, i64>,
    tracks: HashMap<String, i64>,
}

/// The recorded plays that reached a round number, oldest first.
fn milestones_of(tracks: &[Track]) -> Vec<Milestone> {
    let mut plays: Vec<_> = tracks
        .iter()
        .filter_map(|track| Some((track.added_at?, track)))
        .collect();
    plays.sort_by_key(|(at, _)| *at);
    milestone_numbers(plays.len())
        .into_iter()
        .filter_map(|number| {
            let (at, track) = plays.get(number - 1)?;
            Some(Milestone {
                number,
                title: track.name.clone(),
                artist: track.artists.clone(),
                at: *at,
            })
        })
        .collect()
}

fn first_plays(tracks: &[Track]) -> Firsts {
    let mut firsts = Firsts::default();
    let earliest = |map: &mut HashMap<String, i64>, key: String, at: i64| {
        map.entry(key)
            .and_modify(|first| *first = (*first).min(at))
            .or_insert(at);
    };
    for track in tracks {
        let Some(at) = track.added_at else { continue };
        for (key, _) in credits(track) {
            earliest(&mut firsts.artists, key, at);
        }
        if let Some(id) = &track.id {
            earliest(&mut firsts.tracks, id.clone(), at);
        }
    }
    firsts
}

/// Compares rankings and play totals when a previous period exists.
fn compare_with(data: &mut ListeningStats, before: &ListeningStats) {
    let Some(was) = before.overview.plays.filter(|was| *was > 0) else {
        return;
    };
    let named = |id: &str, name: &str| if id.is_empty() { name } else { id }.to_owned();
    let artist = |artist: &SavedArtist| named(&artist.id, &artist.name);
    let song = |track: &Track| track.id.clone().unwrap_or_default();
    let album = |album: &Album| named(&album.id, &album.name);
    compare(
        &mut data.artists,
        before.artists.iter().map(|row| artist(&row.item)),
        artist,
    );
    compare(
        &mut data.tracks,
        before.tracks.iter().map(|row| song(&row.item)),
        song,
    );
    compare(
        &mut data.albums,
        before.albums.iter().map(|row| album(&row.item)),
        album,
    );
    let now = data.overview.plays.unwrap_or_default();
    data.overview.change = Some(((now as i64 - was as i64) * 100 / was as i64) as i32);
}

/// Counts plays in equal slices of `bounds`.
fn slice_plays(tracks: &[Track], (first, last): (i64, i64)) -> Vec<usize> {
    let span = (last - first).max(1) as f64;
    let mut slices = vec![0; DENSITY_BUCKETS];
    for at in tracks.iter().filter_map(|track| track.added_at) {
        let share = (at - first) as f64 / span;
        slices[((share * DENSITY_BUCKETS as f64) as usize).min(DENSITY_BUCKETS - 1)] += 1;
    }
    slices
}
