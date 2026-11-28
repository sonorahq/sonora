//! A year of listening folded into the figures a yearly recap shows.
//!
//! Every count here comes from plays Sonora already stored, so the recap works offline and
//! never asks a provider for anything. A play carries the track's full duration rather than
//! the time actually heard, so listening time is the sum of the tracks played.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use music::ArtistRef;

/// How many entries each ranking keeps.
pub const TOP: usize = 5;

/// One play as the local history stores it. `played_at` is milliseconds since the Unix epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Play {
    pub track_id: String,
    pub name: String,
    pub artists: String,
    pub artist_refs: Vec<ArtistRef>,
    pub album: String,
    pub album_id: Option<String>,
    pub cover: Option<String>,
    pub duration_ms: u64,
    pub played_at: i64,
    pub genres: Vec<String>,
}

/// A song on the year's ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopSong {
    pub id: String,
    pub name: String,
    pub artists: String,
    pub cover: Option<String>,
    pub plays: u32,
}

/// An artist on the year's ranking, credited once per play that features them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopArtist {
    pub id: Option<String>,
    pub name: String,
    pub plays: u32,
}

/// An album on the year's ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopAlbum {
    pub id: Option<String>,
    pub name: String,
    pub artists: String,
    pub cover: Option<String>,
    pub plays: u32,
}

/// A genre on the year's ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopGenre {
    pub name: String,
    pub plays: u32,
}

/// Everything the recap screen shows for one year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Year {
    pub year: i32,
    pub plays: u32,
    pub total_ms: u64,
    pub top_songs: Vec<TopSong>,
    pub top_artists: Vec<TopArtist>,
    pub top_albums: Vec<TopAlbum>,
    /// `None` when no play that year carried a genre, rather than an empty ranking.
    pub top_genres: Option<Vec<TopGenre>>,
    /// Listening time per month, January first, in milliseconds.
    pub months: [u64; 12],
}

impl Year {
    /// The year's listening time as a duration.
    pub fn total(&self) -> Duration {
        Duration::from_millis(self.total_ms)
    }

    /// The listening time of one month, January being `0`.
    pub fn month(&self, index: usize) -> Duration {
        Duration::from_millis(self.months.get(index).copied().unwrap_or(0))
    }
}

/// Every year with a recorded play, oldest first.
pub fn years(plays: &[Play]) -> Vec<i32> {
    let mut found: Vec<i32> = plays
        .iter()
        .filter_map(|play| date_of(play.played_at).map(|(year, _)| year))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// Folds every play of `year` into its ranking, listening time and monthly breakdown.
pub fn year(plays: &[Play], year: i32) -> Year {
    let mut songs: HashMap<String, TopSong> = HashMap::new();
    let mut artists: HashMap<String, TopArtist> = HashMap::new();
    let mut albums: HashMap<String, TopAlbum> = HashMap::new();
    let mut genres: HashMap<String, TopGenre> = HashMap::new();
    let mut counted_plays = 0;
    let mut total_ms = 0;
    let mut months = [0; 12];
    let mut any_genre = false;

    for play in plays {
        let Some((played_year, month)) = date_of(play.played_at) else {
            continue;
        };
        if played_year != year {
            continue;
        }

        counted_plays += 1;
        total_ms += play.duration_ms;
        months[(month - 1) as usize] += play.duration_ms;

        let song = songs.entry(song_key(play)).or_insert_with(|| TopSong {
            id: play.track_id.clone(),
            name: play.name.clone(),
            artists: play.artists.clone(),
            cover: play.cover.clone(),
            plays: 0,
        });
        song.plays += 1;
        if song.cover.is_none() {
            song.cover = play.cover.clone();
        }

        for (key, name, id) in artist_credits(play) {
            let artist = artists.entry(key).or_insert_with(|| TopArtist {
                id,
                name: name.clone(),
                plays: 0,
            });
            artist.plays += 1;
        }

        if !play.album.trim().is_empty() {
            let album = albums.entry(album_key(play)).or_insert_with(|| TopAlbum {
                id: play.album_id.clone(),
                name: play.album.clone(),
                artists: play.artists.clone(),
                cover: play.cover.clone(),
                plays: 0,
            });
            album.plays += 1;
            if album.cover.is_none() {
                album.cover = play.cover.clone();
            }
        }

        let mut named = HashSet::new();
        for genre in &play.genres {
            let name = genre.trim();
            if name.is_empty() {
                continue;
            }
            any_genre = true;
            if !named.insert(name.to_lowercase()) {
                continue;
            }
            let genre = genres
                .entry(name.to_lowercase())
                .or_insert_with(|| TopGenre {
                    name: name.to_owned(),
                    plays: 0,
                });
            genre.plays += 1;
        }
    }

    Year {
        year,
        plays: counted_plays,
        total_ms,
        top_songs: ranked(songs.into_values().collect(), |song| song.plays),
        top_artists: ranked(artists.into_values().collect(), |artist| artist.plays),
        top_albums: ranked(albums.into_values().collect(), |album| album.plays),
        top_genres: any_genre.then(|| ranked(genres.into_values().collect(), |genre| genre.plays)),
        months,
    }
}

/// The local year and month (1 to 12) a play belongs to, or `None` for a timestamp jiff cannot
/// read. Local time is what a listener expects a year to be.
fn date_of(played_at: i64) -> Option<(i32, u8)> {
    let seconds = played_at.div_euclid(1_000);
    let nanos = (played_at.rem_euclid(1_000) * 1_000_000) as i32;
    let stamp = jiff::Timestamp::new(seconds, nanos).ok()?;
    let zoned = stamp.to_zoned_datetime(jiff::tz::TimeZone::system());
    let month = u8::try_from(zoned.month()).ok()?;
    Some((i32::from(zoned.year()), month))
}

/// The key a song is ranked under. A track id is exact; a play without one falls back to its
/// name and artists so two different anonymous tracks do not merge.
fn song_key(play: &Play) -> String {
    match play.track_id.trim().is_empty() {
        true => format!("{}|{}", play.name, play.artists),
        false => play.track_id.clone(),
    }
}

/// The key an album is ranked under. The provider's album id is exact; otherwise the name and
/// the track's artists keep two albums with the same name apart.
fn album_key(play: &Play) -> String {
    match play.album_id.as_deref().filter(|id| !id.trim().is_empty()) {
        Some(id) => format!("id:{id}"),
        None => format!(
            "name:{}|{}",
            play.album.trim().to_lowercase(),
            play.artists.trim().to_lowercase()
        ),
    }
}

/// The artists a play credits, each once, as their key, name and provider id. A ref's id keeps
/// two artists with the same name apart; without refs the track's artist line stands in as one
/// credit.
fn artist_credits(play: &Play) -> Vec<(String, String, Option<String>)> {
    if play.artist_refs.is_empty() {
        return match play.artists.trim() {
            "" => Vec::new(),
            name => vec![(
                format!("name:{}", name.to_lowercase()),
                name.to_owned(),
                None,
            )],
        };
    }

    let mut seen = HashSet::new();
    play.artist_refs
        .iter()
        .filter_map(|artist| {
            let name = match artist.name.trim() {
                "" => artist.id.as_deref()?.to_owned(),
                name => name.to_owned(),
            };
            let id = artist
                .id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .map(str::to_owned);
            let key = match &id {
                Some(id) => format!("id:{id}"),
                None => format!("name:{}", name.to_lowercase()),
            };
            seen.insert(key.clone()).then_some((key, name, id))
        })
        .collect()
}

/// Sorts a ranking by play count, breaking ties by name, and keeps the first [`TOP`].
fn ranked<T>(mut entries: Vec<T>, plays: impl Fn(&T) -> u32) -> Vec<T>
where
    T: Ranked,
{
    entries.sort_by(|left, right| {
        plays(right)
            .cmp(&plays(left))
            .then_with(|| left.name().cmp(right.name()))
    });
    entries.truncate(TOP);
    entries
}

/// What a ranking needs from its entries to order and cut them.
trait Ranked {
    fn name(&self) -> &str;
}

impl Ranked for TopSong {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Ranked for TopArtist {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Ranked for TopAlbum {
    fn name(&self) -> &str {
        &self.name
    }
}

impl Ranked for TopGenre {
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    /// Noon UTC on the fifteenth keeps every test inside its month in any system timezone.
    const JAN_2024: i64 = 1_705_320_000_000;
    const FEB_2024: i64 = 1_707_998_400_000;
    const MAR_2024: i64 = 1_710_504_000_000;
    const JUN_2024: i64 = 1_718_452_800_000;
    const DEC_2024: i64 = 1_734_264_000_000;
    const JAN_2025: i64 = 1_736_942_400_000;

    fn play(
        id: &str,
        name: &str,
        artist: &str,
        album: &str,
        played_at: i64,
        duration_ms: u64,
        genres: &[&str],
    ) -> Play {
        Play {
            track_id: id.to_owned(),
            name: name.to_owned(),
            artists: artist.to_owned(),
            artist_refs: vec![ArtistRef {
                name: artist.to_owned(),
                id: Some(format!("artist:{artist}")),
                channel: false,
            }],
            album: album.to_owned(),
            album_id: None,
            cover: None,
            duration_ms,
            played_at,
            genres: genres.iter().map(|genre| (*genre).to_owned()).collect(),
        }
    }

    #[test]
    fn top_songs_count_plays_and_keep_the_five_most_played() {
        let plays = vec![
            play("a", "A", "Artist", "Album", JAN_2024, 1_000, &[]),
            play("a", "A", "Artist", "Album", FEB_2024, 1_000, &[]),
            play("a", "A", "Artist", "Album", MAR_2024, 1_000, &[]),
            play("b", "B", "Artist", "Album", JAN_2024, 1_000, &[]),
            play("b", "B", "Artist", "Album", FEB_2024, 1_000, &[]),
            play("c", "C", "Artist", "Album", JAN_2024, 1_000, &[]),
            play("d", "D", "Artist", "Album", JAN_2024, 1_000, &[]),
            play("e", "E", "Artist", "Album", JAN_2024, 1_000, &[]),
            play("f", "F", "Artist", "Album", JAN_2024, 1_000, &[]),
        ];

        let year = year(&plays, 2024);

        assert_eq!(year.plays, 9);
        assert_eq!(year.top_songs.len(), TOP);
        assert_eq!(year.top_songs[0].name, "A");
        assert_eq!(year.top_songs[0].plays, 3);
        assert_eq!(year.top_songs[1].name, "B");
        assert_eq!(year.top_songs[1].plays, 2);
        assert!(!year.top_songs.iter().any(|song| song.name == "F"));
    }

    #[test]
    fn top_artists_count_each_credit_once_per_play() {
        let mut first = play("a", "A", "One", "Album", JAN_2024, 1_000, &[]);
        first.artist_refs = vec![
            ArtistRef {
                name: "One".into(),
                id: Some("1".into()),
                channel: false,
            },
            ArtistRef {
                name: "Two".into(),
                id: Some("2".into()),
                channel: false,
            },
            ArtistRef {
                name: "One".into(),
                id: Some("1".into()),
                channel: false,
            },
        ];
        let mut second = play("b", "B", "One", "Album", JAN_2024, 1_000, &[]);
        second.artist_refs[0].id = Some("1".into());
        let plays = vec![first, second];

        let year = year(&plays, 2024);

        assert_eq!(year.top_artists[0].name, "One");
        assert_eq!(year.top_artists[0].plays, 2);
        assert_eq!(year.top_artists[1].name, "Two");
        assert_eq!(year.top_artists[1].plays, 1);
    }

    #[test]
    fn total_listening_time_sums_the_selected_year_only() {
        let plays = vec![
            play("a", "A", "Artist", "Album", JAN_2024, 180_000, &[]),
            play("b", "B", "Artist", "Album", JUN_2024, 240_000, &[]),
            play("c", "C", "Artist", "Album", JAN_2025, 60_000, &[]),
        ];

        let year = year(&plays, 2024);

        assert_eq!(year.total_ms, 420_000);
        assert_eq!(year.total(), Duration::from_secs(420));
    }

    #[test]
    fn top_albums_group_by_album_id_and_count_their_tracks() {
        let mut plays = vec![
            play("a", "A", "Artist", "Album", JAN_2024, 1_000, &[]),
            play("b", "B", "Artist", "Album", FEB_2024, 1_000, &[]),
            play("c", "C", "Artist", "Other", JAN_2024, 1_000, &[]),
        ];
        plays[0].album_id = Some("album:1".into());
        plays[1].album_id = Some("album:1".into());
        plays[2].album_id = Some("album:2".into());

        let year = year(&plays, 2024);

        assert_eq!(year.top_albums.len(), 2);
        assert_eq!(year.top_albums[0].name, "Album");
        assert_eq!(year.top_albums[0].plays, 2);
        assert_eq!(year.top_albums[1].name, "Other");
    }

    #[test]
    fn months_break_the_years_listening_time_down() {
        let plays = vec![
            play("a", "A", "Artist", "Album", JAN_2024, 120_000, &[]),
            play("b", "B", "Artist", "Album", JAN_2024, 30_000, &[]),
            play("c", "C", "Artist", "Album", FEB_2024, 60_000, &[]),
            play("d", "D", "Artist", "Album", DEC_2024, 90_000, &[]),
        ];

        let year = year(&plays, 2024);

        assert_eq!(year.months[0], 150_000);
        assert_eq!(year.months[1], 60_000);
        assert_eq!(year.months[11], 90_000);
        assert_eq!(year.months.iter().sum::<u64>(), year.total_ms);
        assert_eq!(year.month(0), Duration::from_secs(150));
    }

    #[test]
    fn a_year_without_plays_is_empty() {
        let plays = vec![play("a", "A", "Artist", "Album", JAN_2024, 1_000, &[])];

        let year = year(&plays, 2030);

        assert_eq!(year.year, 2030);
        assert_eq!(year.plays, 0);
        assert_eq!(year.total_ms, 0);
        assert!(year.top_songs.is_empty());
        assert!(year.top_artists.is_empty());
        assert!(year.top_albums.is_empty());
        assert_eq!(year.top_genres, None);
        assert_eq!(year.months, [0; 12]);
    }

    #[test]
    fn genres_are_unavailable_when_no_play_names_one() {
        let plays = vec![play("a", "A", "Artist", "Album", JAN_2024, 1_000, &[])];

        let year = year(&plays, 2024);

        assert_eq!(year.top_genres, None);
    }

    #[test]
    fn genres_rank_and_count_case_insensitively() {
        let plays = vec![
            play("a", "A", "Artist", "Album", JAN_2024, 1_000, &["Rock"]),
            play("b", "B", "Artist", "Album", FEB_2024, 1_000, &["rock"]),
            play("c", "C", "Artist", "Album", MAR_2024, 1_000, &["Jazz"]),
        ];

        let year = year(&plays, 2024);
        let genres = year.top_genres.expect("every play names a genre");

        assert_eq!(genres[0].name, "Rock");
        assert_eq!(genres[0].plays, 2);
        assert_eq!(genres[1].name, "Jazz");
        assert_eq!(genres[1].plays, 1);
    }

    #[test]
    fn years_lists_each_recorded_year_once_in_order() {
        let plays = vec![
            play("a", "A", "Artist", "Album", JUN_2024, 1_000, &[]),
            play("b", "B", "Artist", "Album", JAN_2025, 1_000, &[]),
            play("c", "C", "Artist", "Album", JAN_2024, 1_000, &[]),
        ];

        assert_eq!(years(&plays), vec![2024, 2025]);
    }

    #[test]
    fn an_unreadable_timestamp_is_left_out() {
        let mut broken = play("a", "A", "Artist", "Album", JAN_2024, 1_000, &[]);
        broken.played_at = i64::MAX;

        let year = year(&[broken.clone()], 2024);

        assert_eq!(year.plays, 0);
        assert!(years(&[broken]).is_empty());
    }

    /// Keeps the timestamps above honest: a year is read from the play, not assumed.
    #[test]
    fn timestamps_land_in_their_intended_year() {
        assert_eq!(date_of(JAN_2024).map(|(year, _)| year), Some(2024));
        assert_eq!(date_of(JAN_2025).map(|(year, _)| year), Some(2025));
    }

    /// Uses the clock the recap itself reads, so a run in any timezone stays consistent.
    #[test]
    fn the_current_clock_has_a_year() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(date_of(now).is_some());
    }
}
