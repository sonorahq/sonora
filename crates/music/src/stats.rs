use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::{Album, Descriptors, SavedArtist, Track};

/// The windows a source can rank personal listening over. A source lists the ones it supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatsPeriod {
    Weeks,
    Months,
    Year,
}

/// One ranked item. A missing count means the source supplies affinity order alone.
#[derive(Clone, Debug)]
pub struct Ranked<T> {
    pub item: T,
    pub count: Option<usize>,
    /// The source's own page for the item, when it has no matching catalogue id.
    pub external_url: Option<String>,
    /// Whether the item is absent from the list the source compares it to, where it compares.
    pub fresh: bool,
}

impl<T> Ranked<T> {
    pub fn new(item: T, count: Option<usize>) -> Self {
        Self {
            item,
            count,
            external_url: None,
            fresh: false,
        }
    }

    pub fn linked(mut self, url: Option<String>) -> Self {
        self.external_url = url;
        self
    }
}

/// Optional metrics for a listening period.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overview {
    pub plays: Option<usize>,
    pub tracks: Option<usize>,
    pub artists: Option<usize>,
    pub albums: Option<usize>,
    /// Artists, and songs, whose first play in Sonora's retained log falls inside the period.
    pub new_artists: Option<usize>,
    pub new_tracks: Option<usize>,
    /// The share of plays, in percent, that were of a song already played in the period.
    pub replay_share: Option<u8>,
    /// The average song length in seconds, and how mainstream the songs are from 0 to 100.
    pub avg_length: Option<u64>,
    pub mainstream: Option<u8>,
    /// The average year the songs were released, and the share released in the past year.
    pub avg_year: Option<i32>,
    pub recent_share: Option<u8>,
    /// The average tempo of the songs, in beats per minute.
    pub avg_bpm: Option<u16>,
    /// The share of songs, in percent, that few listeners play, and that credit several artists.
    pub deep_cuts: Option<u8>,
    pub collab_share: Option<u8>,
    /// Plays against the period before, in percent.
    pub change: Option<i32>,
    /// The first and last day of the longest run of consecutive days with plays.
    pub streak: Option<(Date, Date)>,
    pub plays_per_day: Option<f32>,
    pub busiest_day: Option<(Date, usize)>,
    /// Seconds of music played, where the source knows track lengths.
    pub time: Option<u64>,
    /// When the account began, in Unix seconds.
    pub since: Option<i64>,
    /// The local hour with the most starts.
    pub peak_hour: Option<u8>,
    /// Starts in each local hour of each weekday, Monday first, or empty without play times.
    pub clock: Vec<usize>,
}

impl Overview {
    /// Derives weekday and hourly activity from play start times.
    pub fn timed(mut self, times: &[i64]) -> Self {
        let zone = TimeZone::system();
        let mut clock = vec![0; 7 * 24];
        let mut days: BTreeMap<Date, usize> = BTreeMap::new();
        for at in times {
            if let Ok(stamp) = jiff::Timestamp::from_second(*at) {
                let local = stamp.to_zoned(zone.clone());
                let weekday = local.weekday().to_monday_zero_offset() as usize;
                clock[weekday * 24 + local.hour() as usize] += 1;
                *days.entry(local.date()).or_default() += 1;
            }
        }
        self.clock = clock;
        self.peak_hour = self
            .hourly()
            .iter()
            .enumerate()
            .max_by_key(|(_, count)| **count)
            .filter(|(_, count)| **count > 0)
            .map(|(hour, _)| hour as u8);
        self.streak = longest_streak(days.keys().copied());
        self.plays_per_day = (!days.is_empty()).then(|| times.len() as f32 / days.len() as f32);
        self.busiest_day = days
            .iter()
            .max_by_key(|(_, count)| **count)
            .map(|(day, count)| (*day, *count));
        self
    }

    /// Drops metrics requiring complete history and retains the observed streak as a lower bound.
    pub fn partial(mut self) -> Self {
        self.plays_per_day = None;
        self.busiest_day = None;
        self
    }

    /// Starts in each local hour, over every weekday.
    pub fn hourly(&self) -> Vec<usize> {
        let mut hours = vec![0; 24];
        for (at, count) in self.clock.iter().enumerate() {
            hours[at % 24] += count;
        }
        hours
    }
}

/// Personal listening for one period: the ranked items and the figures that describe them.
#[derive(Clone, Debug, Default)]
pub struct ListeningStats {
    pub tracks: Vec<Ranked<Track>>,
    pub artists: Vec<Ranked<SavedArtist>>,
    pub albums: Vec<Ranked<Album>>,
    /// Catalogue genres, when the source has a genre taxonomy.
    pub genres: Vec<Ranked<String>>,
    /// Audio-derived mood categories or descriptive provider labels.
    pub moods: Vec<Ranked<String>>,
    /// Descriptive tags supplied alongside audio-derived moods.
    pub tags: Vec<Ranked<String>>,
    /// Writers and producers ranked by number of songs, without published counts.
    pub songwriters: Vec<Ranked<SavedArtist>>,
    pub producers: Vec<Ranked<SavedArtist>>,
    /// How many songs came out in each year, oldest first.
    pub years: Vec<(i32, usize)>,
    /// Counts per ten-BPM band, keyed by the lower bound in ascending order.
    pub tempos: Vec<(u16, usize)>,
    /// Chronological slices of mood and tempo over the period.
    pub timeline: Vec<TimeSlice>,
    /// Average mood for each local hour of the day (`0..24`), when mood data is known.
    pub hourly_mood: Vec<HourMood>,
    /// Plays of the whole history that reached a round number, oldest first.
    pub milestones: Vec<Milestone>,
    /// Plays in each equal slice of the account's history, for a source that can read them.
    pub density: Vec<usize>,
    /// The figures of the selected period.
    pub overview: Overview,
}

/// Mood and tempo for one chronological slice of listening activity.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeSlice {
    /// The start of the slice, in Unix seconds.
    pub at: i64,
    /// Average positivity from 0 to 1, when known.
    pub valence: Option<f32>,
    /// Average intensity from 0 to 1, when known.
    pub energy: Option<f32>,
    /// Average tempo in beats per minute, when known.
    pub bpm: Option<f32>,
    /// Slowest and fastest tempo in this slice, when known.
    pub bpm_min: Option<f32>,
    pub bpm_max: Option<f32>,
    /// Total plays in this slice.
    pub plays: usize,
}

/// Average positivity and intensity for one local hour of the day.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HourMood {
    pub valence: Option<f32>,
    pub energy: Option<f32>,
    pub plays: usize,
}

/// A play that reached a round number in the listener's whole history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Milestone {
    pub number: usize,
    pub title: String,
    pub artist: String,
    /// When the play started, in Unix seconds.
    pub at: i64,
}

/// The round play numbers a history can have reached.
const LADDER: [usize; 11] = [
    100, 500, 1_000, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000,
];
/// How many of the round numbers, the latest ones, are marked beside the first play.
const MARKED: usize = 4;

/// Returns the first play and the most recent round-number milestones.
pub fn milestone_numbers(total: usize) -> Vec<usize> {
    let passed: Vec<_> = LADDER
        .into_iter()
        .filter(|number| *number < total)
        .collect();
    let skip = passed.len().saturating_sub(MARKED);
    (total > 1)
        .then_some(1)
        .into_iter()
        .chain(passed.into_iter().skip(skip))
        .collect()
}

/// How many traits at most are shown, the first that the figures support.
const MOST_TRAITS: usize = 4;

/// What a listener's figures say about how they listen, each a short label to wear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trait {
    EarlyBird,
    Daytime,
    Evening,
    NightOwl,
    Weekender,
    Weekday,
    Explorer,
    Loyalist,
    Mainstream,
    DeepDiver,
    Throwback,
    FreshEars,
    Collaborator,
    Superfan,
    Daily,
    Rising,
    FastLane,
    SlowBurner,
    Sunlit,
    Melancholic,
    MoodShifter,
    SteadyVibe,
    MidnightMellow,
    MorningBoost,
    Accelerating,
}

impl ListeningStats {
    /// Returns the first supported traits in display order.
    pub fn traits(&self) -> Vec<Trait> {
        let o = &self.overview;
        let hour = o.peak_hour.map(|hour| match hour {
            5..=11 => Trait::EarlyBird,
            12..=17 => Trait::Daytime,
            18..=22 => Trait::Evening,
            _ => Trait::NightOwl,
        });
        let total: usize = o.clock.iter().sum();
        let weekend: usize = o.clock.iter().skip(5 * 24).sum();
        let days = match (total > 0).then(|| weekend * 100 / total) {
            Some(40..) => Some(Trait::Weekender),
            Some(..=15) => Some(Trait::Weekday),
            _ => None,
        };
        // Without a record of plays or a window to compare with, how many different artists the
        // top songs spread over says as much: many suggest exploring, a few suggest devotion.
        let mut leads: HashMap<String, usize> = HashMap::new();
        for track in self.tracks.iter().map(|ranked| &ranked.item) {
            let lead = track
                .artist_refs
                .first()
                .and_then(|artist| artist.id.clone())
                .unwrap_or_else(|| track.artists.clone());
            *leads.entry(lead).or_default() += 1;
        }
        let listed = self.tracks.len();
        let explorer = match (o.replay_share, o.new_tracks) {
            (Some(share), _) => Some(share < 40),
            (None, Some(fresh)) if listed > 0 => Some(fresh * 100 / listed >= 20),
            (None, None) if listed >= 10 => Some(leads.len() * 100 / listed >= 50),
            _ => None,
        };
        let novelty = explorer.map(|explorer| match explorer {
            true => Trait::Explorer,
            false => Trait::Loyalist,
        });
        let taste = match (o.mainstream, o.deep_cuts) {
            (Some(60..), _) => Some(Trait::Mainstream),
            (Some(..=50), _) | (_, Some(30..)) => Some(Trait::DeepDiver),
            _ => None,
        };
        let era = match (o.recent_share, o.avg_year) {
            (Some(40..), _) => Some(Trait::FreshEars),
            (_, Some(..=2012)) => Some(Trait::Throwback),
            _ => None,
        };
        let collaborator = (o.collab_share >= Some(40)).then_some(Trait::Collaborator);
        // One artist taking a quarter of the plays, or of the top songs where there are no plays.
        let by_plays = self
            .artists
            .first()
            .and_then(|artist| artist.count)
            .zip(o.plays.filter(|plays| *plays > 0))
            .map(|(top, plays)| top * 100 / plays);
        let by_songs =
            (listed > 0).then(|| leads.values().max().copied().unwrap_or_default() * 100 / listed);
        let superfan = (by_plays.or(by_songs) >= Some(25)).then_some(Trait::Superfan);
        let week = o.streak.is_some_and(|(from, to)| {
            to.since((jiff::Unit::Day, from))
                .is_ok_and(|span| span.get_days() >= 6)
        });
        let daily = week.then_some(Trait::Daily);
        let rising = (o.change >= Some(15)).then_some(Trait::Rising);
        let tempo = o.avg_bpm.and_then(|bpm| match bpm {
            125.. => Some(Trait::FastLane),
            ..=100 => Some(Trait::SlowBurner),
            _ => None,
        });
        let (tone, swing, momentum) = timeline_traits(&self.timeline);
        let tone = tone.or_else(|| mood_list_trait(&self.moods));
        let circadian = hourly_mood_trait(&self.hourly_mood);
        [
            hour,
            novelty,
            taste,
            tone,
            tempo,
            era,
            days,
            superfan,
            collaborator,
            daily,
            rising,
            swing,
            circadian,
            momentum,
        ]
        .into_iter()
        .flatten()
        .take(MOST_TRAITS)
        .collect()
    }
}

/// Counts weighted hits per key and ranks them, keeping first-seen order between equal counts.
pub struct Board<T> {
    slots: HashMap<String, usize>,
    rows: Vec<Ranked<T>>,
}

/// Accumulates the genre, mood, tag and tempo breakdown from any source's tracks.
#[derive(Default)]
pub struct TrackBreakdown {
    genres: Board<String>,
    moods: Board<String>,
    tags: Board<String>,
    tempos: Vec<(f32, usize)>,
}

impl TrackBreakdown {
    /// Adds one track's metadata, weighted by plays or by its presence in a ranking.
    pub fn add(
        &mut self,
        track: &Track,
        descriptor: Option<&Descriptors>,
        weight: usize,
    ) -> Option<f32> {
        let (genres, moods, tags) = descriptor
            .map_or((&track.genres, &track.moods, &track.tags), |found| {
                (&found.genres, &found.moods, &found.tags)
            });
        for (board, names) in [
            (&mut self.genres, genres),
            (&mut self.moods, moods),
            (&mut self.tags, tags),
        ] {
            let mut seen = HashSet::new();
            for name in names {
                if let Some(key) = tag_key(name).filter(|key| seen.insert(key.clone())) {
                    board.add(key, weight, || name.trim().to_owned());
                }
            }
        }
        let bpm = descriptor
            .and_then(|found| found.bpm)
            .or(track.bpm)
            .filter(|bpm| bpm.is_finite() && *bpm > 0.);
        self.tempos.extend(bpm.map(|bpm| (bpm, weight)));
        bpm
    }

    /// Fills missing chart fields while preserving values the source supplied directly.
    pub fn apply(self, stats: &mut ListeningStats) {
        let (moods, tags) = match self.moods.is_empty() {
            true => (self.tags.ranked(), Vec::new()),
            false => (self.moods.ranked(), self.tags.ranked()),
        };
        for (rows, fallback) in [
            (&mut stats.genres, self.genres.ranked()),
            (&mut stats.moods, moods),
            (&mut stats.tags, tags),
        ] {
            if rows.is_empty() {
                *rows = fallback;
            }
        }
        if stats.tempos.is_empty() {
            stats.tempos = tempo_bands(&self.tempos);
        }
        stats.overview.avg_bpm = stats.overview.avg_bpm.or_else(|| mean_tempo(&self.tempos));
    }
}

/// Counts tracks or plays for duration and collaboration averages.
#[derive(Default)]
pub struct TrackTotals {
    pub count: usize,
    pub seconds: u64,
    collaborations: usize,
}

impl TrackTotals {
    pub fn add(&mut self, track: &Track, weight: usize) {
        self.count += weight;
        self.seconds += track.duration.as_secs() * weight as u64;
        self.collaborations += usize::from(track.artist_refs.len() > 1) * weight;
    }

    pub fn avg_length(&self) -> Option<u64> {
        self.seconds.checked_div(self.count as u64)
    }

    pub fn collab_share(&self) -> Option<u8> {
        (self.collaborations * 100)
            .checked_div(self.count)
            .map(|share| share as u8)
    }
}

impl<T> Default for Board<T> {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            rows: Vec::new(),
        }
    }
}

impl<T> Board<T> {
    /// Adds `weight` to `key`, building its item on the first hit.
    pub fn add(&mut self, key: String, weight: usize, make: impl FnOnce() -> T) {
        let at = *self.slots.entry(key).or_insert_with(|| {
            self.rows.push(Ranked::new(make(), Some(0)));
            self.rows.len() - 1
        });
        let count = &mut self.rows[at].count;
        *count = count.map(|count| count.saturating_add(weight));
    }

    /// How many distinct keys were hit.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows from most to least hit.
    pub fn ranked(mut self) -> Vec<Ranked<T>> {
        self.rows.sort_by_key(|row| std::cmp::Reverse(row.count));
        self.rows
    }
}

/// Marks each row that is absent from `before`, the keys of the list it is compared with.
pub fn compare<T>(
    rows: &mut [Ranked<T>],
    before: impl IntoIterator<Item = String>,
    key: impl Fn(&T) -> String,
) {
    let known: HashSet<String> = before.into_iter().collect();
    for row in rows {
        row.fresh = !known.contains(&key(&row.item));
    }
}

/// The artists credited on a track, each once and keyed by catalogue id when there is one.
pub fn credits(track: &Track) -> Vec<(String, SavedArtist)> {
    let mut seen = HashSet::new();
    let mut artists: Vec<_> = track
        .artist_refs
        .iter()
        .filter_map(|artist| {
            let key = artist.id.clone().unwrap_or_else(|| artist.name.clone());
            seen.insert(key.clone()).then(|| {
                let id = artist.id.clone().unwrap_or_default();
                let name = artist.name.clone();
                (
                    key,
                    SavedArtist {
                        id,
                        name,
                        ..Default::default()
                    },
                )
            })
        })
        .collect();
    if artists.is_empty() && !track.artists.is_empty() {
        let name = track.artists.clone();
        artists.push((
            name.clone(),
            SavedArtist {
                name,
                ..Default::default()
            },
        ));
    }
    artists
}

/// The key an album is tallied under: its catalogue id, else its name, or nothing without a name.
pub fn album_key(track: &Track) -> Option<String> {
    (!track.album.is_empty()).then(|| {
        track
            .album_id
            .clone()
            .unwrap_or_else(|| track.album.clone())
    })
}

/// The album a track belongs to, keyed by [`album_key`].
pub fn album_of(track: &Track) -> Option<(String, Album)> {
    let key = album_key(track)?;
    let album = Album {
        id: key.clone(),
        name: track.album.clone(),
        artists: track.artists.clone(),
        artist_refs: track.artist_refs.clone(),
        cover: track.cover.clone(),
        ..Default::default()
    };
    Some((key, album))
}

/// A tag in its tallying form, or nothing for a blank tag or the catch-all "music".
pub fn tag_key(tag: &str) -> Option<String> {
    let key = tag.trim().to_lowercase();
    (!key.is_empty() && key != "music").then_some(key)
}

/// Groups tempos in beats per minute into bands of ten, slowest first, weighted by play count.
pub fn tempo_bands(tempos: &[(f32, usize)]) -> Vec<(u16, usize)> {
    let mut grouped: BTreeMap<u16, usize> = BTreeMap::new();
    for &(bpm, weight) in tempos {
        if bpm > 0. && weight > 0 {
            let band = (bpm.round() as u16 / 10) * 10;
            *grouped.entry(band).or_default() += weight;
        }
    }
    grouped.into_iter().collect()
}

/// Weighted mean tempo in beats per minute, if any valid tempos are present.
pub fn mean_tempo(tempos: &[(f32, usize)]) -> Option<u16> {
    let (sum, total) = tempos
        .iter()
        .filter(|(bpm, weight)| *bpm > 0. && *weight > 0)
        .fold((0f64, 0usize), |(sum, total), &(bpm, weight)| {
            (sum + f64::from(bpm) * weight as f64, total + weight)
        });
    (total > 0).then(|| (sum / total as f64).round() as u16)
}

/// Infers normalized valence and energy from mood keys or tags when audio features are absent.
pub fn infer_mood(moods: &[String], tags: &[String]) -> (Option<f32>, Option<f32>) {
    let (mut v_sum, mut v_count) = (0f32, 0usize);
    let (mut e_sum, mut e_count) = (0f32, 0usize);
    let mut push = |valence: Option<f32>, energy: Option<f32>| {
        if let Some(v) = valence {
            v_sum += v;
            v_count += 1;
        }
        if let Some(e) = energy {
            e_sum += e;
            e_count += 1;
        }
    };
    for term in moods.iter().chain(tags) {
        let lower = term.trim().to_lowercase();
        match lower.as_str() {
            "stats-mood-positive" | "happy" | "upbeat" | "cheerful" | "joyful" | "feel good" => {
                push(Some(0.8), None);
            }
            "stats-mood-negative" | "angry" | "aggressive" | "dark" | "intense" => {
                push(Some(0.2), Some(0.75));
            }
            "stats-mood-sad" | "sad" | "melancholy" | "melancholic" | "depressing" | "lonely" => {
                push(Some(0.15), Some(0.25));
            }
            "stats-mood-energetic" | "energetic" | "energy" | "hype" | "workout" | "driving" => {
                push(None, Some(0.85));
            }
            "stats-mood-calming" | "calm" | "calming" | "chill" | "relaxing" | "ambient"
            | "sleep" | "peaceful" | "soft" | "mellow" => {
                push(None, Some(0.2));
            }
            "stats-mood-danceable" | "dance" | "danceable" | "party" | "groovy" | "disco" => {
                push(Some(0.65), Some(0.75));
            }
            "stats-mood-acoustic" | "acoustic" | "folk" | "unplugged" => {
                push(None, Some(0.35));
            }
            _ => {}
        }
    }
    (
        (v_count > 0).then(|| (v_sum / v_count as f32).clamp(0., 1.)),
        (e_count > 0).then(|| (e_sum / e_count as f32).clamp(0., 1.)),
    )
}

/// The first and last day of the longest run of consecutive calendar days, from the days in order.
fn longest_streak(days: impl Iterator<Item = Date>) -> Option<(Date, Date)> {
    let mut best: Option<(Date, Date, usize)> = None;
    let mut run: Option<(Date, usize)> = None;
    let mut previous = None::<Date>;
    for day in days {
        let continues = previous.and_then(|previous| previous.tomorrow().ok()) == Some(day);
        run = match (continues, run) {
            (true, Some((start, length))) => Some((start, length + 1)),
            _ => Some((day, 1)),
        };
        if let Some((start, length)) = run
            && best.is_none_or(|(_, _, longest)| length > longest)
        {
            best = Some((start, day, length));
        }
        previous = Some(day);
    }
    best.map(|(start, end, _)| (start, end))
}

fn timeline_traits(timeline: &[TimeSlice]) -> (Option<Trait>, Option<Trait>, Option<Trait>) {
    let valences: Vec<f32> = timeline.iter().filter_map(|s| s.valence).collect();
    let energies: Vec<f32> = timeline.iter().filter_map(|s| s.energy).collect();
    let tone = (!valences.is_empty())
        .then(|| {
            let avg = valences.iter().sum::<f32>() / valences.len() as f32;
            match avg {
                v if v >= 0.65 => Some(Trait::Sunlit),
                v if v <= 0.35 => Some(Trait::Melancholic),
                _ => None,
            }
        })
        .flatten();

    let span_of = |vals: &[f32]| -> Option<f32> {
        (vals.len() >= 4).then(|| {
            let min = vals.iter().copied().fold(f32::INFINITY, f32::min);
            let max = vals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            max - min
        })
    };
    let v_span = span_of(&valences);
    let e_span = span_of(&energies);
    let swing = match (v_span, e_span) {
        (Some(v), _) | (_, Some(v)) if v >= 0.35 => Some(Trait::MoodShifter),
        (Some(v), Some(e)) if valences.len() >= 5 && v <= 0.12 && e <= 0.12 => {
            Some(Trait::SteadyVibe)
        }
        _ => None,
    };

    let bpms: Vec<f32> = timeline.iter().filter_map(|s| s.bpm).collect();
    let momentum = if bpms.len() >= 4 {
        let mid = bpms.len() / 2;
        let first = bpms[..mid].iter().sum::<f32>() / mid as f32;
        let second = bpms[mid..].iter().sum::<f32>() / (bpms.len() - mid) as f32;
        (second - first >= 10.).then_some(Trait::Accelerating)
    } else if energies.len() >= 4 {
        let mid = energies.len() / 2;
        let first = energies[..mid].iter().sum::<f32>() / mid as f32;
        let second = energies[mid..].iter().sum::<f32>() / (energies.len() - mid) as f32;
        (second - first >= 0.12).then_some(Trait::Accelerating)
    } else {
        None
    };

    (tone, swing, momentum)
}

fn mood_list_trait(moods: &[Ranked<String>]) -> Option<Trait> {
    let top: Vec<&str> = moods.iter().take(2).map(|m| m.item.as_str()).collect();
    if top.contains(&"stats-mood-positive") {
        Some(Trait::Sunlit)
    } else if top.contains(&"stats-mood-sad") || top.contains(&"stats-mood-negative") {
        Some(Trait::Melancholic)
    } else {
        None
    }
}

fn hourly_mood_trait(hours: &[HourMood]) -> Option<Trait> {
    if hours.len() != 24 {
        return None;
    }
    let avg_energy = |indices: &[usize]| -> Option<f32> {
        let vals: Vec<f32> = indices.iter().filter_map(|&h| hours[h].energy).collect();
        (vals.len() >= 2).then(|| vals.iter().sum::<f32>() / vals.len() as f32)
    };
    let night = avg_energy(&[22, 23, 0, 1, 2, 3]);
    let day = avg_energy(&[9, 10, 11, 12, 13, 14, 15, 16, 17]);
    if let (Some(night), Some(day)) = (night, day)
        && day - night >= 0.12
    {
        return Some(Trait::MidnightMellow);
    }
    let morning = avg_energy(&[5, 6, 7, 8, 9, 10, 11]);
    let evening = avg_energy(&[18, 19, 20, 21, 22]);
    if let (Some(morning), Some(evening)) = (morning, evening)
        && morning - evening >= 0.12
    {
        return Some(Trait::MorningBoost);
    }
    None
}
