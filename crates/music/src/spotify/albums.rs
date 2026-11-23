use std::collections::HashMap;

use crate::spotify::session::Session;
use anyhow::{Context as _, Result};
use spotifyapis::proto::metadata::Album as AlbumMessage;
use spotifyapis::proto::metadata::AlbumType;
use spotifyapis::proto::metadata::CopyrightType;
use spotifyapis::proto::{collection::CollectionItem, metadata::ExtensionKind};

use crate::spotify::{collection, wire};
use crate::{Album, AlbumDetail, ReleaseType};

pub(super) const ALBUM_PREFIX: &str = "spotify:album:";
const TRACK_PREFIX: &str = "spotify:track:";
const UNKNOWN: &str = "Unknown";

/// Hydrates saved albums in collection order and applies their saved dates.
pub(super) async fn hydrate_saved_albums(
    session: &Session,
    items: Vec<CollectionItem>,
) -> Result<Vec<Album>> {
    let uris: Vec<String> = items.iter().map(|item| item.uri.clone()).collect();
    let mut known = metadata(session, &uris).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let mut album = known.remove(&item.uri)?;
            album.added_at = item.added_at.map(i64::from);
            Some(album)
        })
        .collect())
}

/// Loads the album and its ordered track IDs together, then hydrates uncached tracks in batches.
pub(super) async fn album(session: &Session, album_id: &str) -> Result<AlbumDetail> {
    let (album, uris) = read_album(session, album_id, |uri, message| {
        (
            album_from(uri, message),
            track_uris(message).collect::<Vec<_>>(),
        )
    })
    .await?;
    let mut tracks = collection::ordered_tracks(session, &uris, collection::TRACK_DETAILS).await?;
    album.date_tracks(&mut tracks);
    for track in &mut tracks {
        track.album.clone_from(&album.name);
        track.album_artist.clone_from(&album.artists);
        track.album_id = Some(album.id.clone());
        if album.cover.is_some() {
            track.cover.clone_from(&album.cover);
        }
    }
    Ok(AlbumDetail { album, tracks })
}

/// Lists the track identifiers carried by an album's disc data.
pub(super) fn track_uris(album: &AlbumMessage) -> impl Iterator<Item = String> + '_ {
    album
        .disc
        .iter()
        .flat_map(|disc| &disc.track)
        .filter_map(|track| spotifyapis::id::base62(track.gid.as_ref()?))
        .map(|id| format!("{TRACK_PREFIX}{id}"))
}

/// Bounds decoded album data for large artist catalogues.
pub(super) const ALBUM_BATCH: usize = 200;

pub(crate) async fn metadata(session: &Session, uris: &[String]) -> Result<HashMap<String, Album>> {
    project(session, uris, album_from).await
}

/// Projects each album while its protobuf is available, so playback can reuse its track list.
pub(super) async fn project<T>(
    session: &Session,
    uris: &[String],
    project: impl FnMut(&str, &AlbumMessage) -> T,
) -> Result<HashMap<String, T>> {
    let mut project = project;
    collection::project(
        session,
        uris,
        ExtensionKind::AlbumV4,
        ALBUM_BATCH,
        |uri, album| Some(project(uri, &album)),
    )
    .await
    .context("cannot read album metadata")
}

pub(super) fn album_from(uri: &str, album: &AlbumMessage) -> Album {
    let (artists, artist_refs) = collection::artists_from(&album.artist);

    Album {
        id: uri.strip_prefix(ALBUM_PREFIX).unwrap_or(uri).to_owned(),
        name: non_empty(album.name.as_deref())
            .unwrap_or(UNKNOWN)
            .to_owned(),
        artists,
        artist_refs,
        cover: cover(album, false),
        cover_large: cover(album, true),
        release_type: release_type(album.album_type()),
        year: album.date.as_ref().map(|date| date.year()).unwrap_or(0),
        track_count: album
            .disc
            .iter()
            .flat_map(|disc| &disc.track)
            .filter(|track| track.gid().len() == 16)
            .count() as u32,
        release_date: album
            .date
            .as_ref()
            .and_then(|date| crate::calendar::release_date(date.year(), date.month(), date.day()))
            .unwrap_or_default(),
        label: album.label().to_owned(),
        copyrights: album
            .copyright
            .iter()
            .filter_map(|copyright| {
                let text = non_empty(copyright.text.as_deref())?;
                Some(wire::copyright(
                    copyright.r#type() == CopyrightType::P,
                    text,
                ))
            })
            .collect(),
        added_at: None,
    }
}

/// Projects a single album using the shared metadata cache.
pub(super) async fn read_album<T>(
    session: &Session,
    album_id: &str,
    project_album: impl FnMut(&str, &AlbumMessage) -> T,
) -> Result<T> {
    let uri = format!("{ALBUM_PREFIX}{album_id}");
    project(session, std::slice::from_ref(&uri), project_album)
        .await?
        .remove(&uri)
        .context("Spotify omitted the album metadata")
}

fn release_type(kind: AlbumType) -> ReleaseType {
    match kind {
        AlbumType::Album => ReleaseType::Album,
        AlbumType::Single => ReleaseType::Single,
        AlbumType::Compilation => ReleaseType::Compilation,
        AlbumType::Ep => ReleaseType::Ep,
        AlbumType::Audiobook => ReleaseType::Audiobook,
        AlbumType::Podcast => ReleaseType::Podcast,
    }
}

fn cover(album: &AlbumMessage, large: bool) -> Option<String> {
    let images = album
        .cover_group
        .as_ref()?
        .image
        .iter()
        .filter(|image| image.file_id.is_some());
    let image = match large {
        true => images.max_by_key(|image| wire::image_width(image)),
        false => images.min_by_key(|image| image.width()),
    }?;
    wire::image_url(image.file_id())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}
