use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, Result};
use librespot_core::Session;
use librespot_protocol::entity_extension_data::EntityExtensionData;
use librespot_protocol::extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::metadata::image::Size as ImageSize;
use librespot_protocol::metadata::{
    Album as AlbumMessage, Artist as ArtistMessage, Track as TrackMessage,
};
use protobuf::{EnumOrUnknown, Message as _};

use crate::spotify::{collection2, wire};
use crate::{ArtistRef, Credit, Track};

const TRACK_PREFIX: &str = "spotify:track:";
const UNKNOWN: &str = "Unknown";
const BATCH: usize = 500;

pub async fn saved_tracks(session: &Session, limit: u32) -> Result<Vec<Track>> {
    let items = collection2::saved_items(
        session,
        collection2::COLLECTION,
        TRACK_PREFIX,
        limit as usize,
    )
    .await?;
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let uris: Vec<_> = items.iter().map(|item| item.uri.clone()).collect();
    let mut known = metadata(session, &uris).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let mut track = known.remove(&item.uri)?;
            track.added_at = item.added_at;
            Some(track)
        })
        .collect())
}

pub async fn track(session: &Session, track_id: &str) -> Result<Track> {
    let uri = format!("{TRACK_PREFIX}{track_id}");
    metadata(session, std::slice::from_ref(&uri))
        .await?
        .remove(&uri)
        .context("track metadata is missing")
}

pub(crate) async fn metadata(session: &Session, uris: &[String]) -> Result<HashMap<String, Track>> {
    let entities = extended(session, uris, ExtensionKind::TRACK_V4)
        .await
        .context("cannot read track metadata")?;

    let mut tracks = HashMap::new();
    for entity in entities {
        let Ok(message) = TrackMessage::parse_from_bytes(&entity.extension_data.value) else {
            continue;
        };
        let track = track_from(&entity.entity_uri, &message);
        tracks.insert(entity.entity_uri, track);
    }
    Ok(tracks)
}

pub(crate) async fn extended(
    session: &Session,
    uris: &[String],
    kind: ExtensionKind,
) -> Result<Vec<EntityExtensionData>> {
    let mut entities = Vec::with_capacity(uris.len());
    for batch in uris.chunks(BATCH) {
        let request = BatchedEntityRequest {
            entity_request: batch
                .iter()
                .map(|uri| EntityRequest {
                    entity_uri: uri.clone(),
                    query: vec![ExtensionQuery {
                        extension_kind: EnumOrUnknown::new(kind),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let response = session.spclient().get_extended_metadata(request).await?;
        entities.extend(
            response
                .extended_metadata
                .into_iter()
                .flat_map(|array| array.extension_data),
        );
    }
    Ok(entities)
}

fn track_from(uri: &str, track: &TrackMessage) -> Track {
    let (artists, artist_refs) = artists_from(&track.artist);

    Track {
        id: uri.strip_prefix(TRACK_PREFIX).map(str::to_owned),
        playable: !track.file.is_empty() || !track.alternative.is_empty(),
        name: non_empty(track.name.as_deref())
            .unwrap_or(UNKNOWN)
            .to_owned(),
        artists,
        artist_refs,
        album: track
            .album
            .as_ref()
            .and_then(|album| non_empty(album.name.as_deref()))
            .unwrap_or_default()
            .to_owned(),
        album_id: track.album.as_ref().and_then(|album| base62(album.gid())),
        cover: track.album.as_ref().and_then(cover_url),
        cover_large: track.album.as_ref().and_then(super::albums::cover_large),
        duration: Duration::from_millis(track.duration.unwrap_or_default().max(0) as u64),
        added_at: None,
        added_by: None,
        playcount: None,
        popularity: track.popularity.unwrap_or_default().clamp(0, 100) as u32,
        explicit: track.explicit.unwrap_or_default(),
        track_number: track.number.unwrap_or_default().max(0) as u32,
        disc_number: track.disc_number.unwrap_or_default().max(0) as u32,
        tags: track.tags.clone(),
        languages: track.language_of_performance.clone(),
        credits: track
            .artist_with_role
            .iter()
            .filter_map(|credit| {
                let name = non_empty(credit.artist_name.as_deref())?.to_owned();
                let role = match credit.role.map(|role| role.value()) {
                    Some(1) => "Main artist",
                    Some(2) => "Featured artist",
                    Some(3) => "Remixer",
                    Some(4) => "Actor",
                    Some(5) => "Composer",
                    Some(6) => "Conductor",
                    Some(7) => "Orchestra",
                    _ => "Performer",
                };
                Some(Credit {
                    name,
                    role: role.to_owned(),
                    id: base62(credit.artist_gid()),
                })
            })
            .collect(),
    }
}

pub(crate) fn artists_from(artists: &[ArtistMessage]) -> (String, Vec<ArtistRef>) {
    let refs: Vec<_> = artists
        .iter()
        .filter_map(|artist| {
            let name = non_empty(artist.name.as_deref())?.to_owned();
            Some(ArtistRef {
                name,
                id: base62(artist.gid()),
            })
        })
        .collect();
    let names = match refs.is_empty() {
        true => UNKNOWN.to_owned(),
        false => refs
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    };
    (names, refs)
}

pub(crate) fn base62(gid: &[u8]) -> Option<String> {
    librespot_core::SpotifyId::from_raw(gid)
        .ok()?
        .to_base62()
        .ok()
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn cover_url(album: &AlbumMessage) -> Option<String> {
    let smallest = album
        .cover_group
        .as_ref()?
        .image
        .iter()
        .filter(|image| image.has_file_id())
        .min_by_key(|image| match image.size() {
            ImageSize::SMALL => 0,
            ImageSize::DEFAULT => 1,
            ImageSize::LARGE => 2,
            ImageSize::XLARGE => 3,
        })?;

    wire::image_url(smallest.file_id())
}
