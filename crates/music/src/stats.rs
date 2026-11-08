use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::{Album, SavedArtist, Track};

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
    /// How the item's rank moved against what the source compares it to, when it compares.
    pub moved: Option<Move>,
}

/// How an item's rank moved between two lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    New,
    Up(usize),
    Down(usize),
    Same,
}

impl<T> Ranked<T> {
    pub fn new(item: T, count: Option<usize>) -> Self {
        Self {
            item,
            count,
            external_url: None,
            moved: None,
        }
    }

    pub fn linked(mut self, url: Option<String>) -> Self {
        self.external_url = url;
        self
    }
}

/// The figures of a listening period. A source fills in what it knows and leaves the rest empty,
/// so the overview shows exactly the figures that exist.
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
    /// Fills in what the start times of plays say about when the listener listens: the clock,
    /// the days and the busiest hour and day.
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

    /// Drops the figures that only make sense for every play, for times that are just the latest.
    /// A streak found among the latest plays is still a streak, so it stays as a lower bound.
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

    /// Whether there is anything to show.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Personal listening for one period: ranked items and the overview figures.
#[derive(Clone, Debug, Default)]
pub struct ListeningStats {
    pub tracks: Vec<Ranked<Track>>,
    pub artists: Vec<Ranked<SavedArtist>>,
    pub albums: Vec<Ranked<Album>>,
    /// Catalogue genres, when the source has a genre taxonomy.
    pub genres: Vec<Ranked<String>>,
    /// How the songs feel, where the source can say. Either read from the audio, or the words a
    /// source describes songs with that are not genres.
    pub moods: Vec<Ranked<String>>,
    /// The words a source describes songs with, such as slow, soft or chill, where the moods
    /// above are read from the audio instead.
    pub tags: Vec<Ranked<String>>,
    /// The artists credited with writing and with producing the most songs, ranked by how many
    /// songs, without counts.
    pub songwriters: Vec<Ranked<SavedArtist>>,
    pub producers: Vec<Ranked<SavedArtist>>,
    /// How many songs came out in each year, oldest first.
    pub years: Vec<(i32, usize)>,
    /// How many songs play at each tempo, in bands of ten beats per minute named by their
    /// slowest, slowest first.
    pub tempos: Vec<(u16, usize)>,
    /// Plays of the whole history that reached a round number, oldest first.
    pub milestones: Vec<Milestone>,
    /// Plays in each equal slice of the account's history, for a source that can read them.
    pub density: Vec<usize>,
    /// The figures of the selected period.
    pub overview: Overview,
    /// The figures of the whole account, for a source that keeps them apart from the period.
    pub lifetime: Overview,
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

/// The play numbers of a history of `total` plays that are worth marking: the first, and the
/// latest few round numbers it has passed.
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
const MOST_TRAITS: usize = 6;

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
}

impl ListeningStats {
    /// The traits the figures support. A source with few figures has few traits, and a trait
    /// needs the figures it rests on.
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
        let days = (total > 0)
            .then(|| match weekend * 100 / total {
                40.. => Some(Trait::Weekender),
                ..=15 => Some(Trait::Weekday),
                _ => None,
            })
            .flatten();
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
        let collaborator = o
            .collab_share
            .filter(|share| *share >= 40)
            .map(|_| Trait::Collaborator);
        // One artist taking a quarter of the plays, or of the top songs where there are no plays.
        let by_plays = self
            .artists
            .first()
            .and_then(|artist| artist.count)
            .zip(o.plays.filter(|plays| *plays > 0))
            .map(|(top, plays)| top * 100 / plays);
        let by_songs =
            (listed > 0).then(|| leads.values().max().copied().unwrap_or_default() * 100 / listed);
        let superfan = by_plays
            .or(by_songs)
            .filter(|share| *share >= 25)
            .map(|_| Trait::Superfan);
        let daily = o
            .streak
            .filter(|(from, to)| {
                to.since((jiff::Unit::Day, *from))
                    .is_ok_and(|span| span.get_days() >= 6)
            })
            .map(|_| Trait::Daily);
        let rising = o
            .change
            .filter(|change| *change >= 15)
            .map(|_| Trait::Rising);
        let tempo = o.avg_bpm.and_then(|bpm| match bpm {
            125.. => Some(Trait::FastLane),
            ..=100 => Some(Trait::SlowBurner),
            _ => None,
        });
        [
            hour,
            days,
            novelty,
            taste,
            era,
            tempo,
            collaborator,
            superfan,
            daily,
            rising,
        ]
        .into_iter()
        .flatten()
        .take(MOST_TRAITS)
        .collect()
    }
}

/// The genres and moods a source attaches to one track.
#[derive(Clone, Debug, Default)]
pub struct Descriptors {
    pub genres: Vec<String>,
    pub moods: Vec<String>,
}

/// Counts weighted hits per key and ranks them, keeping first-seen order between equal counts.
pub struct Board<T> {
    slots: HashMap<String, usize>,
    rows: Vec<Ranked<T>>,
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

/// Marks each row as new, up, down or the same against `before`, the keys of the list it is
/// compared with in rank order.
pub fn compare<T>(rows: &mut [Ranked<T>], before: &[String], key: impl Fn(&T) -> String) {
    for (rank, row) in rows.iter_mut().enumerate() {
        let was = before.iter().position(|earlier| *earlier == key(&row.item));
        row.moved = Some(match was {
            None => Move::New,
            Some(was) if was > rank => Move::Up(was - rank),
            Some(was) if was < rank => Move::Down(rank - was),
            Some(_) => Move::Same,
        });
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
