use std::time::Duration;

use anyhow::Result;

use super::{auth::sign_in_error, collection, session::Session, wire};
use music::{
    Album, ArtistRef, CoverSize, Found, Playlist, ReleaseType, SavedArtist, SearchPart, Track,
};
use spotifyapis::proto::searchview::{AlbumType, Entity, SearchKind};

const LIMIT: usize = 20;

/// Searches one catalogue type with its own result limit.
async fn hits<T>(
    session: &Session,
    term: &str,
    kind: SearchKind,
    convert: impl FnMut(Entity) -> Option<T>,
) -> Result<Vec<T>> {
    let term = term.trim();
    if term.is_empty() {
        return Ok(Vec::new());
    }

    let response = session
        .api()
        .search(term, kind, LIMIT)
        .await
        .map_err(sign_in_error)?;
    Ok(response
        .hits
        .into_iter()
        .take(LIMIT)
        .filter_map(convert)
        .collect())
}

/// Hydrates ranked track URIs with core catalogue metadata in one batch.
pub(super) async fn tracks(session: &Session, term: &str) -> Result<Vec<Track>> {
    let uris = hits(session, term, SearchKind::Track, |hit| {
        hit.uri.starts_with("spotify:track:").then_some(hit.uri)
    })
    .await?;
    collection::ordered_tracks(
        session,
        &uris,
        &[spotifyapis::proto::extended_metadata::ExtensionKind::TrackV4],
    )
    .await
}

/// Reads recording identifiers and credits for lyric matching directly from SearchView.
pub(super) async fn lyric_tracks(session: &Session, term: &str) -> Result<Vec<Track>> {
    hits(session, term, SearchKind::Track, |hit| {
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
    .await
}

pub(super) async fn albums(session: &Session, term: &str) -> Result<Vec<Album>> {
    hits(session, term, SearchKind::Album, album).await
}

pub(super) async fn artists(session: &Session, term: &str) -> Result<Vec<SavedArtist>> {
    hits(session, term, SearchKind::Artist, |hit| {
        Some(SavedArtist {
            id: id(&hit.uri, "spotify:artist:")?,
            name: hit.name,
            cover: wire::playlist_image(&hit.image_uri),
            added_at: None,
        })
    })
    .await
}

pub(super) async fn playlists(session: &Session, term: &str) -> Result<Vec<Playlist>> {
    hits(session, term, SearchKind::Playlist, playlist).await
}

/// Searches each catalogue type concurrently so each section has its own result limit.
pub(super) async fn with_preview(
    session: &Session,
    term: &str,
    preview: &tokio::sync::mpsc::Sender<SearchPart>,
) -> Found {
    async fn publish<T: Clone>(
        work: impl Future<Output = Result<Vec<T>>>,
        preview: &tokio::sync::mpsc::Sender<SearchPart>,
        wrap: impl FnOnce(Vec<T>) -> SearchPart,
    ) -> Result<Vec<T>> {
        let rows = work.await?;
        if !preview.is_closed() {
            let _ = preview.try_send(wrap(rows.clone()));
        }
        Ok(rows)
    }
    let (tracks, albums, artists, playlists) = tokio::join!(
        publish(Box::pin(tracks(session, term)), preview, SearchPart::Tracks),
        publish(albums(session, term), preview, SearchPart::Albums),
        publish(artists(session, term), preview, SearchPart::Artists),
        publish(playlists(session, term), preview, SearchPart::Playlists),
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
        AlbumType::Audiobook | AlbumType::Podcast => return None,
    };
    let release_date = music::ReleaseDate::parse(&album.user_country_release_iso_time)
        .or_else(|| {
            let stamp = album.release_time?;
            let date = jiff::Timestamp::new(stamp.seconds, stamp.nanos)
                .ok()?
                .to_zoned_datetime(jiff::tz::TimeZone::UTC)
                .date();
            music::ReleaseDate::from_parts(
                date.year().into(),
                date.month().into(),
                date.day().into(),
            )
        })
        .or_else(|| music::ReleaseDate::from_parts(album.release_year, 0, 0));
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
