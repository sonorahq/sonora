use std::collections::HashMap;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

use crate::{collection, session::Session};
use music::{
    Board, Descriptors, ListeningStats, Overview, Ranked, SavedArtist, StatsPeriod, Track,
    TrackBreakdown, TrackTotals, album_of, compare,
};
use spotifyapis::proto::extended_metadata::{ExtensionKind, TrackCredits};

/// Reads profile rankings and compares them with the next broader Spotify preset.
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
    let (metadata, credited, described) = tokio::join!(
        collection::stats_metadata(session, &uris),
        credits(session, &uris),
        describe(session, &ids, std::iter::empty())
    );
    let mut metadata = metadata?;
    let mut described = described;
    for track in metadata.values() {
        described
            .entry(track.id.clone().unwrap_or_default())
            .or_insert_with(|| Descriptors {
                genres: track.genres.clone(),
                tags: track.tags.clone(),
                moods: track.moods.clone(),
                bpm: track.bpm,
                ..Default::default()
            });
    }
    let mut years = Vec::new();
    let credited = optional("credits", credited);
    let (mut songwriters, mut producers) = (Board::default(), Board::default());
    let mut breakdown = TrackBreakdown::default();
    let mut albums = Board::default();
    let mut tracks = Vec::new();
    for uri in uris {
        let Some(track) = metadata.remove(&uri) else {
            continue;
        };
        years.extend(track.release_date.map(|date| date.year() as i32));
        for (writes, produces, artist) in credited.get(&uri).into_iter().flatten() {
            let make = || artist.clone();
            if *writes {
                songwriters.add(artist.id.clone(), 1, make);
            }
            if *produces {
                producers.add(artist.id.clone(), 1, make);
            }
        }
        breakdown.add(
            &track,
            track.id.as_ref().and_then(|id| described.get(id)),
            1,
        );
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
    // Spotify counts every song and artist it ranks for the window, not just the fifty it lists.
    overview.tracks = Some(totals.0).filter(|total| *total > 0);
    overview.artists = Some(totals.1).filter(|total| *total > 0);
    overview.recent_share = recent(&years);
    if let Some((tracks_before, artists_before)) = before {
        compare(&mut tracks, tracks_before, |track| {
            track.id.clone().unwrap_or_default()
        });
        compare(&mut artists, artists_before, |artist| artist.id.clone());
        // What is new against the broader window is what the listener has newly taken to.
        overview.new_tracks = Some(tracks.iter().filter(|row| row.fresh).count());
        overview.new_artists = Some(artists.iter().filter(|row| row.fresh).count());
    }
    // Albums rank by how many of the top tracks they cover.
    let mut stats = ListeningStats {
        tracks,
        artists,
        albums: unranked(albums, usize::MAX),
        songwriters: unranked(songwriters, 12),
        producers: unranked(producers, 12),
        years: tally(&years),
        overview,
        ..Default::default()
    };
    breakdown.apply(&mut stats);
    Ok(stats)
}

/// Logs optional enrichment failures and supplies an empty value.
fn optional<T: Default>(what: &str, result: Result<T>) -> T {
    result.unwrap_or_else(|error| {
        log::warn!("stats: cannot read Spotify {what}: {error:#}");
        T::default()
    })
}

/// Collects the writer and producer roles of each credited artist per track.
async fn credits(
    session: &Session,
    uris: &[String],
) -> Result<HashMap<String, Vec<(bool, bool, SavedArtist)>>> {
    collection::project(
        session,
        uris,
        ExtensionKind::TrackExtendedCredits,
        collection::BATCH,
        |_, credits: TrackCredits| {
            let mut found: Vec<(bool, bool, SavedArtist)> = Vec::new();
            for credit in credits.people {
                let Some(id) = credit.uri.strip_prefix("spotify:artist:") else {
                    continue;
                };
                let role = credit.role.to_lowercase();
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
                            name: credit.name,
                            ..Default::default()
                        },
                    )),
                }
            }
            Some(found)
        },
    )
    .await
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
/// How many track IDs Spotify accepts per audio analysis request.
const FEATURES_AT_ONCE: usize = 100;

/// Requests the profile's top tracks and artists over one of Spotify's three windows.
async fn top(session: &Session, range: &str) -> Result<Profile> {
    let data: Data = session.api().top_rankings(range).await?;
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
    let mut totals = TrackTotals::default();
    let mut popular = Vec::new();
    for row in tracks {
        totals.add(&row.item, 1);
        popular.extend(Some(row.item.popularity).filter(|score| *score > 0));
    }
    Overview {
        mainstream: (popular.iter().sum::<u32>() as usize)
            .checked_div(popular.len())
            .map(|score| score as u8),
        deep_cuts: (popular.iter().filter(|score| **score < DEEP_CUT).count() * 100)
            .checked_div(popular.len())
            .map(|share| share as u8),
        collab_share: totals.collab_share(),
        avg_length: totals.avg_length(),
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
/// Reuses catalogue metadata and adds the stats mood categories from audio analysis.
pub(crate) async fn descriptors(
    session: &Session,
    ids: &[String],
) -> Result<HashMap<String, Descriptors>> {
    let uris: Vec<_> = ids
        .iter()
        .map(|id| spotifyapis::id::track_uri(id))
        .collect();
    let tracks = collection::stats_metadata(session, &uris).await?;
    Ok(describe(session, ids, tracks.values()).await)
}

/// Builds stats descriptors using an already hydrated batch of tracks.
async fn describe<'a>(
    session: &Session,
    ids: &[String],
    tracks: impl IntoIterator<Item = &'a music::Track>,
) -> HashMap<String, Descriptors> {
    let mut found: HashMap<_, _> = tracks
        .into_iter()
        .filter_map(|track| {
            Some((
                track.id.clone()?,
                Descriptors {
                    genres: track.genres.clone(),
                    tags: track.tags.clone(),
                    moods: track.moods.clone(),
                    bpm: track.bpm,
                    ..Default::default()
                },
            ))
        })
        .collect();
    match features(session, ids).await {
        Ok(sound) => {
            for (id, features) in sound {
                let track = found.entry(id).or_default();
                let moods = features.moods();
                if !moods.is_empty() {
                    track.moods = moods.into_iter().map(str::to_owned).collect();
                }
                track.bpm = track
                    .bpm
                    .or_else(|| Some(features.tempo).filter(|bpm| bpm.is_finite() && *bpm > 0.));
                track.valence = Some(features.valence)
                    .filter(|value| value.is_finite())
                    .map(|value| value.clamp(0., 1.));
                track.energy = Some(features.energy)
                    .filter(|value| value.is_finite())
                    .map(|value| value.clamp(0., 1.));
            }
        }
        Err(error) => log::debug!("stats: cannot read Spotify audio features: {error:#}"),
    }
    found
}

/// Normalized audio features and tempo in beats per minute.
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
    /// Returns all applicable mood categories as Fluent keys.
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

/// Requests audio features in batches of 100 and omits unknown tracks.
async fn features(session: &Session, ids: &[String]) -> Result<HashMap<String, Features>> {
    #[derive(Deserialize)]
    struct Answer {
        audio_features: Vec<Option<Features>>,
    }
    let mut found = HashMap::new();
    for ids in ids.chunks(FEATURES_AT_ONCE) {
        let answer: Answer = session.api().audio_features(ids).await?;
        found.extend(
            answer
                .audio_features
                .into_iter()
                .flatten()
                .map(|features| (features.id.clone(), features)),
        );
    }
    Ok(found)
}
