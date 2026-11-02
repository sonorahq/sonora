use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};
use librespot_core::{Session, SpotifyUri};
use librespot_protocol::extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::metadata::{Artist as ArtistMessage, Image};
use protobuf::{EnumOrUnknown, Message as _};

use crate::spotify::collection2::SavedItem;
use crate::spotify::pace::paced;
use crate::spotify::{albums, collection, collection2, pathfinder, wire};
use crate::{Album, Artist, ArtistCatalogue, ArtistProfile, ReleaseType, SavedArtist, Track};

const ARTIST_PREFIX: &str = "spotify:artist:";
const ALBUM_PREFIX: &str = "spotify:album:";
const TRACK_PREFIX: &str = "spotify:track:";
const UNKNOWN: &str = "Unknown";
const TOP_TRACKS: usize = 30;
/// How many releases the ranking reads the tracks of. A release holds ten or so tracks, so
/// this hands the sort well over the thirty it keeps, and every release past that is a read
/// of an album nothing will show.
const MINED_RELEASES: usize = 12;

pub async fn artist(session: &Session, artist_id: &str) -> Result<Artist> {
    match pathfinder::artist(session, artist_id).await {
        Ok(overview) => return overview_artist(session, overview).await,
        Err(error) => log::warn!("artists: cannot load Pathfinder artist: {error:#}"),
    }

    legacy_artist(session, artist_id).await
}

/// The whole discography and the popular tracks mined from it, which the page fills in once
/// the overview is already up. Neither can hold up the first frame: a prolific artist has
/// well over a thousand releases, and ranking the rest of the catalogue reads the tracks of
/// forty of them. `known` is what the overview already ranked, so nothing on the page is
/// listed twice.
pub async fn catalogue(
    session: &Session,
    artist_id: &str,
    known: Vec<Track>,
) -> Result<ArtistCatalogue> {
    let albums = discography(session, artist_id).await?;
    if albums.is_empty() {
        return Ok(ArtistCatalogue::default());
    }
    let top_tracks = deepened(session, known, &albums).await;

    Ok(ArtistCatalogue {
        albums,
        top_tracks,
        ..Default::default()
    })
}

pub async fn profile(session: &Session, artist_id: &str) -> Result<ArtistProfile> {
    Ok(profile_from(&metadata(session, artist_id).await?))
}

/// The artist as the overview alone describes it: the profile, the tracks it ranked and the
/// releases it happened to carry. The rest of the discography is `catalogue`'s job.
async fn overview_artist(session: &Session, overview: pathfinder::Overview) -> Result<Artist> {
    let track_uris: Vec<_> = overview.tracks.iter().map(|(uri, _)| uri.clone()).collect();
    let mut known_tracks = match track_uris.is_empty() {
        true => HashMap::<String, Track>::new(),
        false => collection::metadata(session, &track_uris).await?,
    };
    let top_tracks = overview
        .tracks
        .into_iter()
        .filter_map(|(uri, playcount)| {
            let mut track = known_tracks.remove(&uri)?;
            track.playcount = playcount;
            Some(track)
        })
        .collect();

    Ok(Artist {
        name: overview.name,
        cover_large: overview.cover_large,
        banner: overview.banner,
        biography: overview
            .biography
            .map(|biography| plain_text(&biography))
            .filter(|biography| !biography.is_empty()),
        monthly_listeners: overview.monthly_listeners,
        top_tracks,
        albums: overview.albums,
    })
}

/// What the artist protobuf alone says, for when Pathfinder cannot answer. The releases are
/// left to `catalogue` here too, so a failed overview still draws a page at the same speed.
async fn legacy_artist(session: &Session, artist_id: &str) -> Result<Artist> {
    let message = metadata(session, artist_id).await?;

    let track_uris = top_track_uris(&message, &session.country());
    let known_tracks = match track_uris.is_empty() {
        true => HashMap::<String, Track>::new(),
        false => collection::metadata(session, &track_uris).await?,
    };
    let top_tracks = track_uris
        .iter()
        .filter_map(|uri| known_tracks.get(uri).cloned())
        .collect();

    Ok(artist_from(&message, top_tracks, Vec::new()))
}

async fn deepened(session: &Session, ranked: Vec<Track>, releases: &[Album]) -> Vec<Track> {
    if ranked.len() >= TOP_TRACKS || releases.is_empty() {
        return ranked;
    }

    match popular(session, releases, &ranked).await {
        Ok(rest) => ranked.into_iter().chain(rest).take(TOP_TRACKS).collect(),
        Err(error) => {
            log::warn!("artists: cannot rank the rest of the catalogue: {error:#}");
            ranked
        }
    }
}

async fn popular(session: &Session, releases: &[Album], known: &[Track]) -> Result<Vec<Track>> {
    let mined: Vec<String> = releases
        .iter()
        .filter(|album| matches!(album.release_type, ReleaseType::Album | ReleaseType::Ep))
        .take(MINED_RELEASES)
        .map(|album| format!("{ALBUM_PREFIX}{}", album.id))
        .collect();
    if mined.is_empty() {
        return Ok(Vec::new());
    }
    log::debug!(
        "artists: ranking the tracks of {} of {} releases",
        mined.len(),
        releases.len()
    );

    let uris = albums::track_uris(session, &mined).await?;
    if uris.is_empty() {
        return Ok(Vec::new());
    }

    let known_tracks = collection::metadata(session, &uris).await?;
    let mut rest: Vec<Track> = uris
        .iter()
        .filter_map(|uri| known_tracks.get(uri))
        .filter(|track| track.playable && track.id.is_some())
        .cloned()
        .collect();
    rest.sort_by_key(|track| std::cmp::Reverse(track.popularity));

    let mut seen: HashSet<String> = known
        .iter()
        .map(|track| track.name.to_lowercase())
        .collect();
    rest.retain(|track| seen.insert(track.name.to_lowercase()));
    Ok(rest)
}

/// Every release of an artist, newest first: the albums tab of an album page reads its
/// more-from-the-artist half off this.
pub(crate) async fn discography(session: &Session, artist_id: &str) -> Result<Vec<Album>> {
    let message = metadata(session, artist_id).await?;
    releases(session, &message).await
}

async fn releases(session: &Session, artist: &ArtistMessage) -> Result<Vec<Album>> {
    let uris = release_uris(artist);
    if uris.is_empty() {
        return Ok(Vec::new());
    }

    let known = albums::metadata(session, &uris).await?;
    Ok(newest_first(
        uris.iter().filter_map(|uri| known.get(uri).cloned()),
    ))
}

fn newest_first(releases: impl Iterator<Item = Album>) -> Vec<Album> {
    let mut seen = HashSet::new();
    let mut releases: Vec<_> = releases
        .filter(|album| seen.insert((album.name.to_lowercase(), album.release_type)))
        .collect();

    releases.sort_by(|left, right| right.release_date.cmp(&left.release_date));
    releases
}

async fn metadata(session: &Session, artist_id: &str) -> Result<ArtistMessage> {
    let uri = SpotifyUri::from_uri(&format!("{ARTIST_PREFIX}{artist_id}"))
        .context("invalid artist ID")?;
    let body = paced(|| session.spclient().get_artist_metadata(&uri))
        .await
        .context("cannot read artist metadata")?;
    ArtistMessage::parse_from_bytes(&body).context("cannot decode artist metadata protobuf")
}

pub async fn saved_artists(session: &Session) -> Result<Vec<SavedArtist>> {
    let items = followed(session).await?;
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.uri.strip_prefix(ARTIST_PREFIX).map(str::to_owned))
        .collect();
    let mut known = cards(session, &ids).await?;

    Ok(items
        .into_iter()
        .filter_map(|item| {
            let mut artist = known.remove(item.uri.strip_prefix(ARTIST_PREFIX)?)?;
            artist.added_at = item.added_at;
            Some(artist)
        })
        .collect())
}

async fn followed(session: &Session) -> Result<Vec<SavedItem>> {
    match collection2::saved_items(session, collection2::ARTISTS, ARTIST_PREFIX).await {
        Ok(items) if !items.is_empty() => return Ok(items),
        Ok(_) => log::debug!("artists: the followed set is empty, reading the collection set"),
        Err(error) => log::warn!("artists: cannot read the followed set: {error:#}"),
    }

    collection2::saved_items(session, collection2::COLLECTION, ARTIST_PREFIX).await
}

pub async fn images(session: &Session, ids: &[String]) -> Result<HashMap<String, String>> {
    Ok(cards(session, ids)
        .await?
        .into_iter()
        .filter_map(|(id, artist)| artist.cover.map(|url| (id, url)))
        .collect())
}

async fn cards(session: &Session, ids: &[String]) -> Result<HashMap<String, SavedArtist>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let request = BatchedEntityRequest {
        entity_request: ids
            .iter()
            .map(|id| EntityRequest {
                entity_uri: format!("{ARTIST_PREFIX}{id}"),
                query: vec![ExtensionQuery {
                    extension_kind: EnumOrUnknown::new(ExtensionKind::ARTIST_V4),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };

    let response = paced(|| session.spclient().get_extended_metadata(request.clone()))
        .await
        .context("cannot read artist portraits")?;

    let mut found = HashMap::new();
    for array in response.extended_metadata {
        for entity in array.extension_data {
            let Ok(message) = ArtistMessage::parse_from_bytes(&entity.extension_data.value) else {
                continue;
            };
            let Some(id) = entity.entity_uri.strip_prefix(ARTIST_PREFIX) else {
                continue;
            };
            let smallest = portraits(&message)
                .into_iter()
                .min_by_key(|image| wire::image_width(image));

            found.insert(
                id.to_owned(),
                SavedArtist {
                    id: id.to_owned(),
                    name: match message.name() {
                        "" => UNKNOWN.to_owned(),
                        name => name.to_owned(),
                    },
                    cover: smallest.and_then(|image| wire::image_url(image.file_id())),
                    added_at: None,
                },
            );
        }
    }

    Ok(found)
}

fn artist_from(artist: &ArtistMessage, top_tracks: Vec<Track>, albums: Vec<Album>) -> Artist {
    let profile = profile_from(artist);

    Artist {
        name: profile.name,
        cover_large: profile.cover_large,
        banner: None,
        biography: profile.biography,
        monthly_listeners: None,
        top_tracks,
        albums,
    }
}

fn profile_from(artist: &ArtistMessage) -> ArtistProfile {
    let portraits = portraits(artist);

    ArtistProfile {
        name: artist.name().to_owned(),
        cover_large: portraits
            .iter()
            .max_by_key(|image| wire::image_width(image))
            .and_then(|image| wire::image_url(image.file_id())),
        biography: artist.biography.iter().find_map(|bio| {
            bio.text
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(plain_text)
                .filter(|text| !text.is_empty())
        }),
    }
}

fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut tag = String::new();
    let mut inside_tag = false;

    for character in html.chars() {
        match character {
            '<' if !inside_tag => {
                inside_tag = true;
                tag.clear();
            }
            '>' if inside_tag => {
                inside_tag = false;
                let tag = tag.trim().to_ascii_lowercase();
                if (tag.starts_with("br")
                    || tag.starts_with("/p")
                    || tag.starts_with("/li")
                    || tag.starts_with("/div"))
                    && !text.ends_with(char::is_whitespace)
                {
                    text.push(' ');
                }
            }
            _ if inside_tag => tag.push(character),
            _ => text.push(character),
        }
    }

    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn top_track_uris(artist: &ArtistMessage, country: &str) -> Vec<String> {
    artist
        .top_track
        .iter()
        .find(|tracks| tracks.country() == country)
        .or_else(|| {
            artist
                .top_track
                .iter()
                .find(|tracks| tracks.country().is_empty())
        })
        .into_iter()
        .flat_map(|tracks| tracks.track.iter())
        .filter_map(|track| collection::base62(track.gid()))
        .map(|id| format!("{TRACK_PREFIX}{id}"))
        .collect()
}

fn release_uris(artist: &ArtistMessage) -> Vec<String> {
    let mut seen = HashSet::new();

    artist
        .album_group
        .iter()
        .chain(artist.single_group.iter())
        .chain(artist.compilation_group.iter())
        .flat_map(|group| group.album.iter())
        .filter_map(|album| collection::base62(album.gid()))
        .filter(|id| seen.insert(id.clone()))
        .map(|id| format!("{ALBUM_PREFIX}{id}"))
        .collect()
}

fn portraits(artist: &ArtistMessage) -> Vec<&Image> {
    let mut portraits: Vec<_> = artist
        .portrait_group
        .as_ref()
        .into_iter()
        .flat_map(|group| group.image.iter())
        .filter(|image| image.has_file_id())
        .collect();
    portraits.extend(artist.portrait.iter().filter(|image| image.has_file_id()));
    portraits
}

#[cfg(test)]
mod tests {
    use super::{Album, newest_first, plain_text};
    use crate::ReleaseType;

    fn release(name: &str, date: &str, release_type: ReleaseType) -> Album {
        Album {
            id: name.to_lowercase(),
            name: name.to_owned(),
            artists: String::new(),
            artist_refs: Vec::new(),
            cover: None,
            cover_large: None,
            release_type,
            year: 0,
            track_count: 0,
            release_date: date.to_owned(),
            label: String::new(),
            copyrights: Vec::new(),
            added_at: None,
        }
    }

    #[test]
    fn every_edition_survives_but_market_duplicates_do_not() {
        let releases = newest_first(
            [
                release("Meteora (Bonus Edition)", "2003-03-25", ReleaseType::Album),
                release("Meteora", "2003-03-25", ReleaseType::Album),
                release("METEORA", "2003-03-25", ReleaseType::Album),
                release("Meteora", "2003-03-25", ReleaseType::Single),
                release("From Zero", "2024-11-15", ReleaseType::Album),
            ]
            .into_iter(),
        );
        let names: Vec<_> = releases
            .iter()
            .map(|album| (album.name.as_str(), album.release_type))
            .collect();

        assert_eq!(
            names,
            [
                ("From Zero", ReleaseType::Album),
                ("Meteora (Bonus Edition)", ReleaseType::Album),
                ("Meteora", ReleaseType::Album),
                ("Meteora", ReleaseType::Single),
            ]
        );
    }

    #[test]
    fn biography_html_becomes_readable_text() {
        assert_eq!(
            plain_text(
                "Formed by <a href=\"spotify:artist:abc\">Alice &amp; Bob</a>.<br>Based in Paris."
            ),
            "Formed by Alice & Bob. Based in Paris."
        );
    }
}
