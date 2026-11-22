use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use super::proto::search::{AlbumType, Entity, MainViewResponse};
use super::{collection, session::Session, transport, wire};
use crate::{Album, ArtistRef, CoverSize, Found, Playlist, ReleaseType, SavedArtist, Track};

const LIMIT: usize = 20;
const PATH: &str = "/searchview/v3/search";

/// Runs one ranked search for a catalogue type. SearchView's limit is shared
/// across types in a blended response.
async fn hits(session: &Session, term: &str, kind: &str) -> Result<Vec<Entity>> {
    let term = term.trim();
    if term.is_empty() {
        return Ok(Vec::new());
    }

    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let request_id = request_id();
    let query = crate::escape::component(term);
    let path = format!(
        "{PATH}?query={query}&entity_types={kind}&limit={LIMIT}&timestamp={timestamp}&request_id={request_id}&show_type=podcast"
    );

    let headers = transport::proto_headers("application/protobuf", Some("Linux"));
    let response: MainViewResponse = transport::get(session, &path, headers).await?;
    Ok(response.hits.into_iter().take(LIMIT).collect())
}

/// Hydrates ranked track URIs with core catalogue metadata in one batch.
pub(super) async fn tracks(session: &Session, term: &str) -> Result<Vec<Track>> {
    let uris: Vec<_> = hits(session, term, "track")
        .await?
        .into_iter()
        .map(|hit| hit.uri)
        .filter(|uri| uri.starts_with("spotify:track:"))
        .collect();
    collection::ordered_tracks(
        session,
        &uris,
        &[super::proto::metadata::ExtensionKind::TrackV4],
    )
    .await
}

/// Reads recording identifiers and credits for lyric matching directly from SearchView.
pub(super) async fn lyric_tracks(session: &Session, term: &str) -> Result<Vec<Track>> {
    Ok(hits(session, term, "track")
        .await?
        .into_iter()
        .filter_map(|hit| {
            let track = hit.track?;
            let duration = track.duration.and_then(|duration| {
                let seconds = u64::try_from(duration.seconds).ok()?;
                let nanos = u32::try_from(duration.nanos).ok()?;
                (nanos < 1_000_000_000).then(|| Duration::new(seconds, nanos))
            });
            Some(Track {
                id: Some(id(&hit.uri, "spotify:track:")?),
                name: hit.name,
                artists: wire::join_names(
                    track
                        .track_artists
                        .iter()
                        .map(|artist| artist.name.as_str()),
                ),
                album: track
                    .track_album
                    .map(|album| album.name)
                    .unwrap_or_default(),
                duration: duration.unwrap_or_default(),
                ..Default::default()
            })
        })
        .collect())
}

pub(super) async fn albums(session: &Session, term: &str) -> Result<Vec<Album>> {
    Ok(hits(session, term, "album")
        .await?
        .into_iter()
        .filter_map(album)
        .collect())
}

pub(super) async fn artists(session: &Session, term: &str) -> Result<Vec<SavedArtist>> {
    Ok(hits(session, term, "artist")
        .await?
        .into_iter()
        .filter_map(|hit| {
            Some(SavedArtist {
                id: id(&hit.uri, "spotify:artist:")?,
                name: hit.name,
                cover: wire::playlist_image(&hit.image_uri),
                added_at: None,
            })
        })
        .collect())
}

pub(super) async fn playlists(session: &Session, term: &str) -> Result<Vec<Playlist>> {
    Ok(hits(session, term, "playlist")
        .await?
        .into_iter()
        .filter_map(playlist)
        .collect())
}

/// Searches each catalogue type concurrently so each section has its own result limit.
pub(super) async fn all(session: &Session, term: &str) -> Found {
    let (tracks, albums, artists, playlists) = tokio::join!(
        tracks(session, term),
        albums(session, term),
        artists(session, term),
        playlists(session, term),
    );
    Found {
        tracks,
        albums,
        artists,
        playlists,
    }
}

fn album(hit: Entity) -> Option<Album> {
    let album = hit.album?;
    let release_type = match album.r#type() {
        AlbumType::Undefined | AlbumType::Album => ReleaseType::Album,
        AlbumType::Single => ReleaseType::Single,
        AlbumType::Compilation => ReleaseType::Compilation,
        AlbumType::Ep => ReleaseType::Ep,
        AlbumType::Audiobook => ReleaseType::Audiobook,
        AlbumType::Podcast => ReleaseType::Podcast,
    };
    let release_date = crate::calendar::parse_release_date(&album.user_country_release_iso_time)
        .or_else(|| crate::calendar::release_date(album.release_year, 0, 0))
        .unwrap_or_default();
    let cover = wire::playlist_image(&hit.image_uri);
    let cover_large = cover
        .as_deref()
        .and_then(|cover| wire::cover_at(cover, CoverSize::Large));
    Some(Album {
        id: id(&hit.uri, "spotify:album:")?,
        name: hit.name,
        artists: album.artist_names,
        artist_refs: album
            .album_artists
            .into_iter()
            .map(|artist| ArtistRef {
                id: id(&artist.uri, "spotify:artist:"),
                name: artist.name,
                channel: false,
            })
            .collect(),
        cover,
        cover_large,
        release_type,
        year: album.release_year,
        release_date,
        ..Default::default()
    })
}

fn playlist(hit: Entity) -> Option<Playlist> {
    let playlist = hit.playlist?;
    Some(Playlist {
        id: id(&hit.uri, "spotify:playlist:")?,
        name: hit.name,
        owner: playlist.owner_name,
        public: true,
        cover: wire::playlist_image(&hit.image_uri),
        track_count: playlist.number_of_tracks.try_into().unwrap_or_default(),
        ..Default::default()
    })
}

fn id(uri: &str, prefix: &str) -> Option<String> {
    uri.strip_prefix(prefix)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// Formats the per-request identifier as a UUIDv4.
fn request_id() -> String {
    let [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p] = fastrand::u128(..).to_be_bytes();
    let c0 = (g & 0x0f) | 0x40;
    let d0 = (i & 0x3f) | 0x80;
    format!(
        "{a:02x}{b:02x}{c:02x}{d:02x}-{e:02x}{f:02x}-{c0:02x}{h:02x}-{d0:02x}{j:02x}-{k:02x}{l:02x}{m:02x}{n:02x}{o:02x}{p:02x}"
    )
}
