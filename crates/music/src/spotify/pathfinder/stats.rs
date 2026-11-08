use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};
use librespot_core::Session;
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::metadata::Track as TrackMessage;
use protobuf::Message as _;
use serde::Deserialize;

use crate::spotify::{collection, pb};
use crate::{
    Board, Descriptors, ListeningStats, Move, Overview, Ranked, SavedArtist, StatsPeriod, Track,
    album_of, compare,
};

/// Reads the same personal rankings as Spotify's profile, without the Web API. Each is compared
/// with the next broader window, which shows who is rising and who is fading.
pub(crate) async fn stats(session: &Session, period: StatsPeriod) -> Result<ListeningStats> {
    let (range, broader) = match period {
        StatsPeriod::Weeks => ("SHORT_TERM", Some("MID_TERM")),
        StatsPeriod::Months => ("MID_TERM", Some("LONG_TERM")),
        StatsPeriod::Year => ("LONG_TERM", None),
    };
    let (profile, broader) = tokio::join!(top(session, range), async {
        match broader {
            Some(range) => top(session, range).await.ok(),
            None => None,
        }
    });
    let profile = profile?;
    let earlier = |page: &Page, prefix: &str| -> Vec<String> {
        page.items
            .iter()
            .filter_map(|hit| hit.data.as_ref()?.uri.strip_prefix(prefix))
            .map(str::to_owned)
            .collect()
    };
    let before = broader.map(|broader| {
        (
            earlier(&broader.tracks, "spotify:track:"),
            earlier(&broader.artists, "spotify:artist:"),
        )
    });
    let totals = (profile.tracks.total, profile.artists.total);
    let uris: Vec<_> = profile
        .tracks
        .items
        .into_iter()
        .filter_map(|hit| hit.data)
        .filter(|item| item.kind == "Track")
        .map(|item| item.uri)
        .collect();
    let ids: Vec<_> = uris
        .iter()
        .filter_map(|uri| uri.strip_prefix("spotify:track:"))
        .map(str::to_owned)
        .collect();
    let (metadata, descriptors, features, credited) = tokio::join!(
        released(session, &uris),
        descriptors(session, &uris),
        features(session, &ids),
        credits(session, &uris)
    );
    let mut metadata = metadata?;
    let mut years = Vec::new();
    let mut descriptors = descriptors.unwrap_or_else(|error| {
        log::warn!("stats: cannot read Spotify track descriptors: {error:#}");
        HashMap::new()
    });
    let features = features.unwrap_or_else(|error| {
        log::warn!("stats: cannot read Spotify audio features: {error:#}");
        HashMap::new()
    });
    let credited = credited.unwrap_or_else(|error| {
        log::warn!("stats: cannot read Spotify credits: {error:#}");
        HashMap::new()
    });
    let mut feeling = Board::default();
    let (mut songwriters, mut producers) = (Board::default(), Board::default());
    let mut genres = Board::default();
    let mut moods = Board::default();
    let mut albums = Board::default();
    let mut tracks = Vec::new();
    for uri in uris {
        let Some((mut track, release)) = metadata.remove(&uri) else {
            continue;
        };
        if let Some(year) = release.year {
            years.push(year);
        }
        if let Some(found) = uri
            .strip_prefix("spotify:track:")
            .and_then(|id| features.get(id))
        {
            for mood in found.moods() {
                feeling.add(mood.to_owned(), 1, || mood.to_owned());
            }
        }
        for (writes, produces, artist) in credited.get(&uri).into_iter().flatten() {
            let make = || artist.clone();
            if *writes {
                songwriters.add(artist.id.clone(), 1, make);
            }
            if *produces {
                producers.add(artist.id.clone(), 1, make);
            }
        }
        let tags = descriptors.remove(&uri).unwrap_or_default();
        track.tags = tags.genres.iter().chain(&tags.moods).cloned().collect();
        for (board, names) in [(&mut genres, tags.genres), (&mut moods, tags.moods)] {
            for name in names {
                board.add(name.clone(), 1, || name);
            }
        }
        if let Some((key, album)) = album_of(&track) {
            albums.add(key, 1, || album);
        }
        tracks.push(Ranked::new(track, None));
    }
    let mut artists: Vec<Ranked<SavedArtist>> = profile
        .artists
        .items
        .into_iter()
        .filter_map(|hit| hit.data)
        .filter(|item| item.kind == "Artist")
        .filter_map(|item| {
            let id = item.uri.strip_prefix("spotify:artist:")?.to_owned();
            let name = item.profile?.name;
            let cover = item
                .visuals
                .and_then(|visuals| visuals.avatar_image)
                .and_then(|image| image.sources.into_iter().max_by_key(|source| source.height))
                .map(|source| source.url);
            let artist = SavedArtist {
                id,
                name,
                cover,
                ..Default::default()
            };
            Some(Ranked::new(artist, None))
        })
        .collect();
    let mut overview = about(&tracks);
    overview.avg_year = average(&years);
    let tempos: Vec<f32> = features
        .values()
        .map(|found| found.tempo)
        .filter(|bpm| *bpm > 0.)
        .collect();
    overview.avg_bpm = mean_tempo(&tempos);
    // Spotify counts every song and artist it ranks for the window, not just the fifty it lists.
    overview.tracks = Some(totals.0).filter(|total| *total > 0);
    overview.artists = Some(totals.1).filter(|total| *total > 0);
    overview.recent_share = recent(&years);
    if let Some((tracks_before, artists_before)) = &before {
        compare(&mut tracks, tracks_before, |track| {
            track.id.clone().unwrap_or_default()
        });
        compare(&mut artists, artists_before, |artist| artist.id.clone());
        // What is new against the broader window is what the listener has newly taken to.
        let new_in = |moves: Vec<Option<Move>>| {
            moves
                .into_iter()
                .filter(|moved| *moved == Some(Move::New))
                .count()
        };
        overview.new_tracks = Some(new_in(tracks.iter().map(|row| row.moved).collect()));
        overview.new_artists = Some(new_in(artists.iter().map(|row| row.moved).collect()));
    }
    // Moods read from the audio say more than a song's descriptor words. Those words stay as tags
    // beside them, and stand in as the moods when the analysis is missing.
    let (moods, tags) = match feeling.is_empty() {
        false => (feeling.ranked(), moods.ranked()),
        true => (moods.ranked(), Vec::new()),
    };
    // Spotify ranks by affinity without counts, so albums and tags rank by how many of the
    // top tracks they cover.
    Ok(ListeningStats {
        tracks,
        artists,
        albums: unranked(albums, usize::MAX),
        genres: genres.ranked(),
        moods,
        tags,
        songwriters: unranked(songwriters, 12),
        producers: unranked(producers, 12),
        years: tally(&years),
        tempos: bands(&tempos),
        overview,
        ..Default::default()
    })
}

/// When a song came out.
struct Release {
    year: Option<i32>,
}

/// Reads the songs with the release year that the same metadata carries.
async fn released(session: &Session, uris: &[String]) -> Result<HashMap<String, (Track, Release)>> {
    let entities = collection::extended(session, uris, ExtensionKind::TRACK_V4)
        .await
        .context("cannot read track metadata")?;
    Ok(entities
        .into_iter()
        .filter_map(|entity| {
            let message = TrackMessage::parse_from_bytes(&entity.extension_data.value).ok()?;
            let album = &message.album;
            let release = Release {
                year: Some(album.date.year()).filter(|year| *year > 0),
            };
            let track = collection::track_from(&entity.entity_uri, &message);
            Some((entity.entity_uri, (track, release)))
        })
        .collect())
}

/// Who is credited with writing and producing each song: the artist, and whether as a writer and
/// as a producer. A song's credits name an artist once for each role.
async fn credits(
    session: &Session,
    uris: &[String],
) -> Result<HashMap<String, Vec<(bool, bool, SavedArtist)>>> {
    let entities =
        collection::extended(session, uris, ExtensionKind::TRACK_EXTENDED_CREDITS).await?;
    entities
        .into_iter()
        .map(|entity| {
            let mut found: Vec<(bool, bool, SavedArtist)> = Vec::new();
            let mut reader = pb::Reader::new(&entity.extension_data.value);
            while let Some((field, value)) = reader.field()? {
                let (1, pb::Value::Bytes(credit)) = (field, value) else {
                    continue;
                };
                let (mut id, mut name, mut role) = (String::new(), String::new(), String::new());
                let mut credit = pb::Reader::new(credit);
                while let Some((field, value)) = credit.field()? {
                    match (field, value) {
                        (1, pb::Value::Bytes(value)) => id = pb::text(value)?,
                        (2, pb::Value::Bytes(value)) => name = pb::text(value)?,
                        (3, pb::Value::Bytes(value)) => role = pb::text(value)?.to_lowercase(),
                        _ => {}
                    }
                }
                let Some(id) = id.strip_prefix("spotify:artist:") else {
                    continue;
                };
                let writes = ["composer", "lyricist", "writer"]
                    .iter()
                    .any(|word| role.contains(word));
                let produces = role.contains("producer");
                if !(writes || produces) {
                    continue;
                }
                match found.iter_mut().find(|(_, _, artist)| artist.id == id) {
                    Some(known) => {
                        known.0 |= writes;
                        known.1 |= produces;
                    }
                    None => found.push((
                        writes,
                        produces,
                        SavedArtist {
                            id: id.to_owned(),
                            name: name.clone(),
                            ..Default::default()
                        },
                    )),
                }
            }
            Ok((entity.entity_uri, found))
        })
        .collect()
}

/// The `most` highest ranked rows of a board, without the counts that only ordered them.
fn unranked<T>(board: Board<T>, most: usize) -> Vec<Ranked<T>> {
    board
        .ranked()
        .into_iter()
        .take(most)
        .map(|row| Ranked::new(row.item, None))
        .collect()
}

/// How a song sounds, as Spotify's audio analysis measures it. Each measure runs from none to all.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Features {
    id: String,
    danceability: f32,
    energy: f32,
    valence: f32,
    acousticness: f32,
    instrumentalness: f32,
    tempo: f32,
}

impl Features {
    /// The feelings the song carries, by the Fluent key that names each. A song can carry several,
    /// and a middling one none.
    fn moods(&self) -> Vec<&'static str> {
        let mut moods = Vec::new();
        let mut when = |condition: bool, mood| {
            if condition {
                moods.push(mood);
            }
        };
        when(self.energy >= 0.7, "stats-mood-energetic");
        when(self.danceability >= 0.7, "stats-mood-danceable");
        when(self.valence >= 0.6, "stats-mood-positive");
        when(
            self.valence <= 0.3 && self.energy >= 0.5,
            "stats-mood-negative",
        );
        when(self.valence <= 0.3 && self.energy < 0.5, "stats-mood-sad");
        when(self.energy <= 0.35, "stats-mood-calming");
        when(self.instrumentalness >= 0.5, "stats-mood-instrumental");
        when(self.acousticness >= 0.6, "stats-mood-acoustic");
        moods
    }
}

/// Reads how each of the songs sounds, all in one request, which is what Spotify's own audio
/// analysis answers for. A song it does not know is left out.
async fn features(session: &Session, ids: &[String]) -> Result<HashMap<String, Features>> {
    #[derive(Deserialize)]
    struct Answer {
        audio_features: Vec<Option<Features>>,
    }
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let path = format!("/audio-attributes/v1/audio-features?ids={}", ids.join(","));
    let body = session
        .spclient()
        .request_as_json(&http::Method::GET, &path, None, None)
        .await
        .context("cannot request audio features")?;
    let answer: Answer = serde_json::from_slice(&body).context("cannot decode audio features")?;
    Ok(answer
        .audio_features
        .into_iter()
        .flatten()
        .map(|found| (found.id.clone(), found))
        .collect())
}

/// The mean tempo, rounded.
fn mean_tempo(tempos: &[f32]) -> Option<u16> {
    (!tempos.is_empty()).then(|| (tempos.iter().sum::<f32>() / tempos.len() as f32).round() as u16)
}

/// How many songs fall in each band of ten beats per minute, slowest first.
fn bands(tempos: &[f32]) -> Vec<(u16, usize)> {
    let mut counts = std::collections::BTreeMap::new();
    for bpm in tempos {
        *counts.entry((*bpm as u16) / 10 * 10).or_default() += 1;
    }
    counts.into_iter().collect()
}

/// How many of the years there are of each, oldest first.
fn tally(years: &[i32]) -> Vec<(i32, usize)> {
    let mut counts = std::collections::BTreeMap::new();
    for year in years {
        *counts.entry(*year).or_default() += 1;
    }
    counts.into_iter().collect()
}

/// The mean of the years, rounded.
fn average(years: &[i32]) -> Option<i32> {
    let total: i64 = years.iter().map(|year| i64::from(*year)).sum();
    total
        .checked_div(years.len() as i64)
        .map(|year| year as i32)
}

/// The share of the years that are this year or the one before, in percent.
fn recent(years: &[i32]) -> Option<u8> {
    let now = jiff::Zoned::now().year();
    (!years.is_empty()).then(|| {
        (years
            .iter()
            .filter(|year| **year >= i32::from(now) - 1)
            .count()
            * 100
            / years.len()) as u8
    })
}

/// The popularity under which a song counts as a deep cut.
const DEEP_CUT: u32 = 40;

/// Requests the profile's top tracks and artists over one of Spotify's three windows.
async fn top(session: &Session, range: &str) -> Result<Profile> {
    let input =
        serde_json::json!({"offset": 0, "limit": 50, "sortBy": "AFFINITY", "timeRange": range});
    let data: Data = super::query(
        session,
        "userTopContent",
        serde_json::json!({
            "includeTopArtists": true, "topArtistsInput": input,
            "includeTopTracks": true, "topTracksInput": input,
        }),
    )
    .await?;
    let profile = data
        .me
        .context("stats response has no account")?
        .profile
        .context("stats response has no profile")?;
    ensure!(
        profile.tracks.kind == "TrackPageV2" && profile.artists.kind == "ArtistPageV2",
        "Spotify did not return personal rankings"
    );
    Ok(profile)
}

/// What the top songs say about the listener's taste: how popular, how rare and how long.
fn about(tracks: &[Ranked<Track>]) -> Overview {
    let songs = tracks.iter().map(|ranked| &ranked.item);
    let popular: Vec<_> = songs
        .clone()
        .map(|track| track.popularity)
        .filter(|score| *score > 0)
        .collect();
    let share = |count: usize, of: usize| (count * 100).checked_div(of).map(|share| share as u8);
    Overview {
        mainstream: (popular.iter().sum::<u32>() as usize)
            .checked_div(popular.len())
            .map(|score| score as u8),
        deep_cuts: share(
            popular.iter().filter(|score| **score < DEEP_CUT).count(),
            popular.len(),
        ),
        collab_share: share(
            songs
                .clone()
                .filter(|track| track.artist_refs.len() > 1)
                .count(),
            tracks.len(),
        ),
        avg_length: songs
            .map(|track| track.duration.as_secs())
            .sum::<u64>()
            .checked_div(tracks.len() as u64),
        ..Default::default()
    }
}

#[derive(Deserialize)]
struct Data {
    me: Option<Account>,
}
#[derive(Deserialize)]
struct Account {
    profile: Option<Profile>,
}
#[derive(Deserialize)]
struct Profile {
    #[serde(rename = "topTracks")]
    tracks: Page,
    #[serde(rename = "topArtists")]
    artists: Page,
}
#[derive(Deserialize)]
struct Page {
    #[serde(rename = "__typename")]
    kind: String,
    #[serde(default)]
    items: Vec<Hit>,
    #[serde(default, rename = "totalCount")]
    total: usize,
}
#[derive(Deserialize)]
struct Hit {
    data: Option<Item>,
}
#[derive(Deserialize)]
struct Item {
    #[serde(rename = "__typename")]
    kind: String,
    #[serde(default)]
    uri: String,
    profile: Option<Name>,
    visuals: Option<Visuals>,
}
#[derive(Deserialize)]
struct Name {
    name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Visuals {
    avatar_image: Option<Image>,
}
#[derive(Deserialize)]
struct Image {
    sources: Vec<Source>,
}
#[derive(Deserialize)]
struct Source {
    url: String,
    height: Option<u32>,
}

/// Reads typed track descriptors over librespot's metadata transport. Missing extensions leave a
/// track untagged; malformed descriptors fail the optional lookup.
pub(crate) async fn descriptors(
    session: &Session,
    uris: &[String],
) -> Result<HashMap<String, Descriptors>> {
    let entities = collection::extended(session, uris, ExtensionKind::TRACK_DESCRIPTOR).await?;
    entities
        .into_iter()
        .map(|entity| {
            let tags = concepts(&entity.extension_data.value)?;
            Ok((entity.entity_uri, tags))
        })
        .collect()
}

/// Reads descriptor text and its repeated type enums; types 1 and 2 are genre and mood.
fn concepts(bytes: &[u8]) -> Result<Descriptors> {
    let mut reader = pb::Reader::new(bytes);
    let mut tags = Descriptors::default();
    let mut seen_genres = HashSet::new();
    let mut seen_moods = HashSet::new();
    while let Some((field, value)) = reader.field()? {
        let (1, pb::Value::Bytes(descriptor)) = (field, value) else {
            continue;
        };
        let mut descriptor = pb::Reader::new(descriptor);
        let mut text = None;
        let mut kinds = Vec::new();
        while let Some((field, value)) = descriptor.field()? {
            match (field, value) {
                (1, pb::Value::Bytes(value)) => text = Some(pb::text(value)?),
                (3, pb::Value::Varint(value)) => kinds.push(value),
                (3, pb::Value::Bytes(values)) => kinds.extend(pb::varints(values)?),
                _ => {}
            }
        }
        let Some(text) = text else { continue };
        let text = text.trim().to_lowercase();
        let text = text
            .strip_prefix("spotify:concept:")
            .unwrap_or(&text)
            .replace('_', " ");
        let text = text.trim();
        if text.is_empty() || text == "track" {
            continue;
        }
        if kinds.contains(&1) && seen_genres.insert(text.to_owned()) {
            tags.genres.push(text.to_owned());
        }
        if (kinds.contains(&2) || kinds.is_empty()) && seen_moods.insert(text.to_owned()) {
            tags.moods.push(text.to_owned());
        }
    }
    Ok(tags)
}
