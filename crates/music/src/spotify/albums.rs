use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};
use librespot_core::Session;
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::metadata::Album as AlbumMessage;
use librespot_protocol::metadata::album::Type as AlbumType;
use librespot_protocol::metadata::copyright::Type as CopyrightType;
use protobuf::Message as _;

use crate::spotify::{collection, collection2, pathfinder, wire};
use crate::{Album, AlbumDetail, ReleaseType, Track};

const ALBUM_PREFIX: &str = "spotify:album:";
const TRACK_PREFIX: &str = "spotify:track:";
const UNKNOWN: &str = "Unknown";

pub async fn saved_albums(session: &Session) -> Result<Vec<Album>> {
    let items = collection2::saved_items(session, collection2::COLLECTION, ALBUM_PREFIX).await?;
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let uris: Vec<String> = items.iter().map(|item| item.uri.clone()).collect();
    let mut known = metadata(session, &uris).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let mut album = known.remove(&item.uri)?;
            album.added_at = item.added_at;
            Some(album)
        })
        .collect())
}

pub async fn album(session: &Session, album_id: &str) -> Result<AlbumDetail> {
    match pathfinder::album(session, album_id).await {
        Ok(album) => return Ok(album),
        Err(error) => log::warn!("albums: cannot load Pathfinder album: {error:#}"),
    }

    legacy_album(session, album_id).await
}

async fn legacy_album(session: &Session, album_id: &str) -> Result<AlbumDetail> {
    let uri = format!("{ALBUM_PREFIX}{album_id}");
    let message =
        collection::extended(session, std::slice::from_ref(&uri), ExtensionKind::ALBUM_V4)
            .await
            .context("cannot read album metadata")?
            .into_iter()
            .find_map(|entity| AlbumMessage::parse_from_bytes(&entity.extension_data.value).ok())
            .context("album metadata is missing")?;
    let album = album_from(&uri, &message);
    let uris: Vec<_> = track_ids(&message)
        .into_iter()
        .map(|id| format!("{TRACK_PREFIX}{id}"))
        .collect();
    let tracks = match uris.is_empty() {
        true => Vec::new(),
        false => {
            let known = collection::metadata(session, &uris).await?;
            uris.iter()
                .filter_map(|uri| known.get(uri).cloned())
                .collect()
        }
    };
    Ok(AlbumDetail { album, tracks })
}

pub async fn album_tracks(session: &Session, album_id: &str) -> Result<Vec<Track>> {
    Ok(album(session, album_id).await?.tracks)
}

fn track_ids(album: &AlbumMessage) -> Vec<String> {
    album
        .disc
        .iter()
        .flat_map(|disc| disc.track.iter())
        .filter_map(|track| collection::base62(track.gid.as_ref()?))
        .collect()
}

/// An album entity carries its discs, so a batch of them is worth tens of megabytes. They are
/// asked for a batch at a time and turned into models as each batch lands, which is what
/// keeps an artist with a thousand releases from holding the lot at once.
const ALBUM_BATCH: usize = 200;

pub(crate) async fn metadata(session: &Session, uris: &[String]) -> Result<HashMap<String, Album>> {
    let mut albums = HashMap::with_capacity(uris.len());
    collection::batched(
        session,
        uris,
        ExtensionKind::ALBUM_V4,
        ALBUM_BATCH,
        |entities| {
            for entity in entities {
                let Ok(message) = AlbumMessage::parse_from_bytes(&entity.extension_data.value)
                else {
                    continue;
                };
                let album = album_from(&entity.entity_uri, &message);
                albums.insert(entity.entity_uri, album);
            }
            Ok(())
        },
    )
    .await
    .context("cannot read album metadata")?;

    Ok(albums)
}

pub(crate) async fn track_uris(session: &Session, uris: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    collection::batched(
        session,
        uris,
        ExtensionKind::ALBUM_V4,
        ALBUM_BATCH,
        |entities| {
            for entity in entities {
                let Ok(message) = AlbumMessage::parse_from_bytes(&entity.extension_data.value)
                else {
                    continue;
                };
                for id in track_ids(&message) {
                    if seen.insert(id.clone()) {
                        tracks.push(format!("{TRACK_PREFIX}{id}"));
                    }
                }
            }
            Ok(())
        },
    )
    .await
    .context("cannot read album metadata")?;

    Ok(tracks)
}

fn album_from(uri: &str, album: &AlbumMessage) -> Album {
    let (artists, artist_refs) = collection::artists_from(&album.artist);

    Album {
        id: uri.strip_prefix(ALBUM_PREFIX).unwrap_or(uri).to_owned(),
        name: non_empty(album.name.as_deref())
            .unwrap_or(UNKNOWN)
            .to_owned(),
        artists,
        artist_refs,
        cover: cover(album),
        cover_large: cover_large(album),
        release_type: release_type(album.type_()),
        year: album.date.as_ref().map(|date| date.year()).unwrap_or(0),
        track_count: track_ids(album).len() as u32,
        release_date: album
            .date
            .as_ref()
            .map(|date| {
                let year = date.year();
                let month = date.month();
                let day = date.day();
                match (month > 0, day > 0) {
                    (true, true) => format!("{year:04}-{month:02}-{day:02}"),
                    (true, false) => format!("{year:04}-{month:02}"),
                    _ => format!("{year:04}"),
                }
            })
            .unwrap_or_default(),
        label: album.label().to_owned(),
        copyrights: album
            .copyright
            .iter()
            .filter_map(|copyright| {
                let text = non_empty(copyright.text.as_deref())?;
                Some(wire::copyright(copyright.type_() == CopyrightType::P, text))
            })
            .collect(),
        added_at: None,
    }
}

fn release_type(kind: AlbumType) -> ReleaseType {
    match kind {
        AlbumType::ALBUM => ReleaseType::Album,
        AlbumType::SINGLE => ReleaseType::Single,
        AlbumType::COMPILATION => ReleaseType::Compilation,
        AlbumType::EP => ReleaseType::Ep,
        AlbumType::AUDIOBOOK => ReleaseType::Audiobook,
        AlbumType::PODCAST => ReleaseType::Podcast,
    }
}

fn cover(album: &AlbumMessage) -> Option<String> {
    let smallest = album
        .cover_group
        .as_ref()?
        .image
        .iter()
        .filter(|image| image.has_file_id())
        .min_by_key(|image| image.width())?;

    wire::image_url(smallest.file_id())
}

/// The largest cover Spotify lists, which is 640 pixels for nearly every album.
fn cover_large(album: &AlbumMessage) -> Option<String> {
    let largest = album
        .cover_group
        .as_ref()?
        .image
        .iter()
        .filter(|image| image.has_file_id())
        .max_by_key(|image| wire::image_width(image))?;

    wire::image_url(largest.file_id())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}
