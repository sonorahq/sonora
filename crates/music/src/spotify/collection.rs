use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::spotify::proto::metadata::ImageSize;
use crate::spotify::proto::metadata::{
    Album as AlbumMessage, Artist as ArtistMessage, Track as TrackMessage,
};
use crate::spotify::session::Session;
use anyhow::{Context as _, Result};
use bytes::Bytes;
use prost::Message;

use super::proto::collection::{CollectionItem, PageRequest, PageResponse, WriteRequest};
use super::proto::metadata::{
    AudioAttributes, BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionKind,
    ExtensionQuery, Reputation, TrackDescriptors,
};
use crate::spotify::{feeds, transport, wire};
use crate::{ArtistRef, Credit, GenreItem, PinTarget, PinTargetKind, Track};

const LIKED_TRACKS: &str = "spotify:collection:tracks";
const TRACK_PREFIX: &str = "spotify:track:";
const UNKNOWN: &str = "Unknown";
pub(super) const BATCH: usize = 500;
pub(super) const TRACK_DETAILS: &[ExtensionKind] = &[
    ExtensionKind::TrackV4,
    ExtensionKind::TrackDescriptor,
    ExtensionKind::OnPlatformReputation,
    ExtensionKind::AudioAttributesV2,
];
const CACHE_ENTRIES: usize = 512;
const CACHE_VALUE_BYTES: usize = 32 * 1024;
const MAX_METADATA_AGE: Duration = Duration::from_secs(300);

const PAGING: &str = "/collection/v2/paging";
const WRITE: &str = "/collection/v2/write";
const CONTENT: &str = "application/vnd.collection-v2.spotify.proto";
pub(crate) const COLLECTION: &str = "collection";
pub(crate) const ARTISTS: &str = "artist";
pub(super) const PINS: &str = "ylpin";
const PAGE: i32 = 300;

static UPDATE: AtomicU64 = AtomicU64::new(0);

/// Recent metadata for this signed-in account, keyed by entity and extension.
#[derive(Default)]
pub(super) struct Cache(Mutex<HashMap<MetadataKey, CachedEntity>>);

/// Keys one protobuf payload by its extension and Spotify URI.
pub(super) type MetadataKey = (ExtensionKind, String);

/// A protobuf payload and the time through which Spotify says it can be reused.
#[derive(Clone)]
struct CachedEntity {
    value: Bytes,
    etag: String,
    fresh_until: Instant,
}

pub async fn saved_tracks(session: &Session) -> Result<Vec<Track>> {
    let items = saved_items(session, COLLECTION, TRACK_PREFIX).await?;
    let uris: Vec<_> = items.iter().map(|item| item.uri.clone()).collect();
    let mut known = metadata(session, &uris).await?;
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let mut track = known.remove(&item.uri)?;
            track.added_at = item.added_at.map(i64::from);
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
    project_batch(session, uris, TRACK_DETAILS, BATCH, take_track)
        .await
        .context("cannot read track metadata")
}

/// Reads artwork without building full tracks. A present entry also confirms track metadata.
pub(super) async fn track_covers(
    session: &Session,
    uris: &[String],
) -> Result<HashMap<String, Option<String>>> {
    project(
        session,
        uris,
        ExtensionKind::TrackV4,
        BATCH,
        |_, track: TrackMessage| Some(track.album.as_ref().and_then(cover_url)),
    )
    .await
}

/// Projects a track with any descriptors and play count returned in its batch.
pub(super) fn take_track(data: &mut HashMap<MetadataKey, Bytes>, uri: &str) -> Option<Track> {
    let mut track = track_from(uri, &take(data, ExtensionKind::TrackV4, uri)?);
    if let Some(descriptors) = take::<TrackDescriptors>(data, ExtensionKind::TrackDescriptor, uri) {
        for descriptor in descriptors.descriptors {
            let label = non_empty(Some(&descriptor.localized_term))
                .unwrap_or(&descriptor.text)
                .trim();
            for (category, values) in [(1, &mut track.genres), (2, &mut track.moods)] {
                if descriptor.categories.contains(&category)
                    && !label.is_empty()
                    && !values.iter().any(|word| word.eq_ignore_ascii_case(label))
                {
                    values.push(label.to_owned());
                }
            }
        }
    }
    track.playcount = take::<Reputation>(data, ExtensionKind::OnPlatformReputation, uri)
        .and_then(|stats| stats.total_plays)
        .and_then(|plays| u64::try_from(plays).ok());
    track.bpm = take::<AudioAttributes>(data, ExtensionKind::AudioAttributesV2, uri)
        .map(|attributes| attributes.bpm as f32)
        .filter(|bpm| bpm.is_finite() && *bpm > 0.);
    Some(track)
}

/// Projects single-extension batches in URI order without retaining raw responses.
pub(super) async fn project<M: Message + Default, T>(
    session: &Session,
    uris: &[String],
    kind: ExtensionKind,
    batch: usize,
    mut map: impl FnMut(&str, M) -> Option<T>,
) -> Result<HashMap<String, T>> {
    project_batch(
        session,
        uris,
        std::slice::from_ref(&kind),
        batch,
        |data, uri| map(uri, take(data, kind, uri)?),
    )
    .await
}

async fn project_batch<T>(
    session: &Session,
    uris: &[String],
    kinds: &[ExtensionKind],
    batch: usize,
    mut map: impl FnMut(&mut HashMap<MetadataKey, Bytes>, &str) -> Option<T>,
) -> Result<HashMap<String, T>> {
    let mut seen = HashSet::new();
    let requests: Vec<_> = uris
        .iter()
        .map(String::as_str)
        .filter(|uri| seen.insert(*uri))
        .map(|uri| (uri, kinds))
        .collect();
    let mut known = HashMap::with_capacity(requests.len());
    for pair in requests.chunks(2 * batch) {
        let (first, second) = pair.split_at(pair.len().min(batch));
        let cache = session.metadata();
        let (head, tail) =
            tokio::try_join!(cache.read(session, first), cache.read(session, second))?;
        for (requests, mut values) in [(first, head), (second, tail)] {
            known.extend(
                requests
                    .iter()
                    .filter_map(|&(uri, _)| Some((uri.to_owned(), map(&mut values, uri)?))),
            );
        }
    }
    Ok(known)
}

/// Hydrates track occurrences in server order, including duplicate occurrences.
pub(super) async fn ordered_tracks(
    session: &Session,
    uris: &[String],
    kinds: &[ExtensionKind],
) -> Result<Vec<Track>> {
    let known = project_batch(session, uris, kinds, BATCH, take_track)
        .await
        .context("cannot read track metadata")?;
    Ok(uris
        .iter()
        .filter_map(|uri| known.get(uri).cloned())
        .collect())
}

/// Fetches different extensions together, reusing cached payloads for each entity and kind.
pub(super) async fn extended_batch(
    session: &Session,
    requests: &[(&str, &[ExtensionKind])],
) -> Result<HashMap<MetadataKey, Bytes>> {
    let mut known = HashMap::new();
    for pair in requests.chunks(2 * BATCH) {
        let (first, second) = pair.split_at(pair.len().min(BATCH));
        let cache = session.metadata();
        let (head, tail) =
            tokio::try_join!(cache.read(session, first), cache.read(session, second))?;
        known.extend(head);
        known.extend(tail);
    }
    Ok(known)
}

/// Decodes one requested extension from a mixed metadata batch.
pub(super) fn take<T: Message + Default>(
    data: &mut HashMap<MetadataKey, Bytes>,
    kind: ExtensionKind,
    uri: &str,
) -> Option<T> {
    T::decode(data.remove(&(kind, uri.to_owned()))?).ok()
}

impl Cache {
    /// Reuses fresh entries and sends stale ETags with the remaining queries.
    async fn read(
        &self,
        session: &Session,
        requests: &[(&str, &[ExtensionKind])],
    ) -> Result<HashMap<MetadataKey, Bytes>> {
        if requests.is_empty() {
            return Ok(HashMap::new());
        }
        let now = Instant::now();
        let mut request = BatchedEntityRequest::default();
        let mut ready = HashMap::new();
        let mut pending = HashMap::new();
        {
            let cache = self.0.lock().expect("Spotify metadata cache poisoned");
            for &(uri, kinds) in requests {
                let mut queries = Vec::new();
                for &kind in kinds {
                    let key = (kind, uri.to_owned());
                    if ready.contains_key(&key) || pending.contains_key(&key) {
                        continue;
                    }
                    let stale = match cache.get(&key) {
                        Some(entry) if entry.fresh_until > now => {
                            ready.insert(key, entry.value.clone());
                            continue;
                        }
                        entry => entry.cloned(),
                    };
                    queries.push(ExtensionQuery {
                        extension_kind: kind as i32,
                        etag: stale
                            .as_ref()
                            .map(|entry| entry.etag.clone())
                            .unwrap_or_default(),
                    });
                    pending.insert(key, stale);
                }
                if !queries.is_empty() {
                    request.entity_request.push(EntityRequest {
                        entity_uri: uri.to_owned(),
                        query: queries,
                    });
                }
            }
        }

        if !request.entity_request.is_empty() {
            let response: BatchedExtensionResponse =
                transport::rpc(session, transport::METADATA, request).await?;
            let mut cache = self.0.lock().expect("Spotify metadata cache poisoned");
            for array in response.extended_metadata {
                let Ok(kind) = ExtensionKind::try_from(array.extension_kind) else {
                    continue;
                };
                for mut data in array.extension_data {
                    let key = (kind, std::mem::take(&mut data.entity_uri));
                    let Some(stale) = pending.remove(&key) else {
                        continue;
                    };
                    let header = data.header.get_or_insert_default();
                    let value = if header.status_code == 304 {
                        let Some(CachedEntity { value, etag, .. }) = stale else {
                            continue;
                        };
                        if header.etag.is_empty() {
                            header.etag = etag;
                        }
                        value
                    } else if let Some(payload) = data.extension_data {
                        payload.value
                    } else {
                        cache.remove(&key);
                        continue;
                    };
                    let ttl = Duration::from_secs(header.cache_ttl_in_seconds.max(0) as u64)
                        .min(MAX_METADATA_AGE);
                    if (!ttl.is_zero() || !header.etag.is_empty())
                        && value.len() <= CACHE_VALUE_BYTES
                    {
                        if cache.len() == CACHE_ENTRIES
                            && !cache.contains_key(&key)
                            && let Some(oldest) = cache
                                .iter()
                                .min_by_key(|(_, entry)| entry.fresh_until)
                                .map(|(key, _)| key.clone())
                        {
                            cache.remove(&oldest);
                        }
                        cache.insert(
                            key.clone(),
                            CachedEntity {
                                value: value.clone(),
                                etag: std::mem::take(&mut header.etag),
                                fresh_until: now + ttl,
                            },
                        );
                    } else {
                        cache.remove(&key);
                    }
                    ready.insert(key, value);
                }
            }
        }
        Ok(ready)
    }
}

/// Maps a hydrated catalogue track to the shared model.
pub(super) fn track_from(uri: &str, track: &TrackMessage) -> Track {
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
        album_artist: wire::join_names(
            track
                .album
                .iter()
                .flat_map(|album| &album.artist)
                .filter_map(|artist| non_empty(artist.name.as_deref())),
        ),
        album_id: track.album.as_ref().and_then(|album| base62(album.gid())),
        cover: track.album.as_ref().and_then(cover_url),
        duration: Duration::from_millis(track.duration_ms.unwrap_or_default().max(0) as u64),
        added_at: None,
        added_by: None,
        release_date: track
            .album
            .as_ref()
            .and_then(|album| album.date.as_ref())
            .and_then(|date| crate::calendar::release_date(date.year(), date.month(), date.day())),
        playcount: None,
        popularity: track.popularity.clamp(0, 100) as u32,
        explicit: track.explicit.unwrap_or_default(),
        track_number: track.track_number.max(0) as u32,
        disc_number: track.disc_number.max(0) as u32,
        isrc: track
            .external_id
            .iter()
            .filter(|external| external.kind().eq_ignore_ascii_case("isrc"))
            .find_map(|external| non_empty(external.id.as_deref()).map(str::to_owned)),
        genres: Vec::new(),
        tags: Vec::new(),
        moods: Vec::new(),
        bpm: None,
        languages: track.language_of_performance.clone(),
        credits: track
            .artist_with_role
            .iter()
            .filter_map(|credit| {
                let name = non_empty(credit.artist_name.as_deref())?.to_owned();
                let role = match credit.role {
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
                channel: false,
            })
        })
        .collect();
    let names = match refs.is_empty() {
        true => UNKNOWN.to_owned(),
        false => wire::join_names(refs.iter().map(|artist| artist.name.as_str())),
    };
    (names, refs)
}

pub(crate) fn base62(gid: &[u8]) -> Option<String> {
    String::from_utf8(base62_bytes(gid)?.to_vec()).ok()
}

/// Encodes a 16-byte catalogue ID for callers that only need to compare its text.
pub(super) fn base62_bytes(gid: &[u8]) -> Option<[u8; 22]> {
    const DIGITS: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut value = u128::from_be_bytes(gid.try_into().ok()?);
    let mut encoded = [b'0'; 22];
    for digit in encoded.iter_mut().rev() {
        *digit = DIGITS[(value % 62) as usize];
        value /= 62;
    }
    Some(encoded)
}

/// Decodes the 22-character Spotify ID into the 16 bytes used by the access point.
pub(super) fn raw_id(id: &str) -> Option<[u8; 16]> {
    if id.len() != 22 {
        return None;
    }
    let value = id.bytes().try_fold(0_u128, |value, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'z' => byte - b'a' + 10,
            b'A'..=b'Z' => byte - b'A' + 36,
            _ => return None,
        };
        value.checked_mul(62)?.checked_add(u128::from(digit))
    })?;
    Some(value.to_be_bytes())
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
        .filter(|image| image.file_id.is_some())
        .min_by_key(|image| match image.size() {
            ImageSize::Small => 0,
            ImageSize::Default => 1,
            ImageSize::Large => 2,
            ImageSize::Xlarge => 3,
        })?;

    wire::image_url(smallest.file_id())
}

/// Updates the collection set used by Spotify's native client for library pins.
pub(super) async fn set_pinned(session: &Session, uri: &str, pinned: bool) -> Result<()> {
    let uri = match uri {
        "spotify:collection:tracks" => "spotify:collection",
        uri => uri,
    };
    set_saved(session, PINS, uri, pinned)
        .await
        .context("cannot update the Spotify library pin")
}

/// Writes an entity to its collection set, or removes it.
pub(super) async fn set_saved(session: &Session, set: &str, uri: &str, saved: bool) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("cannot read the current time")?;
    let sequence = UPDATE.fetch_add(1, Ordering::Relaxed);
    let request = WriteRequest {
        username: session.username().to_owned(),
        set: set.to_owned(),
        items: vec![CollectionItem {
            uri: uri.to_owned(),
            added_at: saved
                .then(|| i32::try_from(now.as_secs()))
                .transpose()
                .context("collection timestamp exceeds int32")?,
            is_removed: !saved,
        }],
        update_id: format!(
            "sonora-{}-{}-{}-{sequence}",
            now.as_secs(),
            now.subsec_nanos(),
            std::process::id()
        ),
    };
    transport::post_proto(session, WRITE, CONTENT, true, &request)
        .await
        .context("cannot update the saved collection")?;
    Ok(())
}

pub(crate) async fn saved_items(
    session: &Session,
    set: &str,
    prefix: &str,
) -> Result<Vec<CollectionItem>> {
    let username = session.username();
    let mut found = Vec::new();
    let mut token = String::new();

    loop {
        let request = PageRequest {
            username: username.to_owned(),
            set: set.to_owned(),
            token,
            limit: PAGE,
        };
        let raw = transport::post_proto(session, PAGING, CONTENT, false, &request)
            .await
            .context("cannot read the saved collection")?;
        let page = PageResponse::decode(raw).context("cannot decode the collection page")?;

        found.extend(
            page.items
                .into_iter()
                .filter(|item| !item.is_removed && item.uri.starts_with(prefix)),
        );

        token = page.next;
        if token.is_empty() {
            break;
        }
    }

    Ok(found)
}

/// Builds Spotify's pin URI for an item the sidebar can open.
pub(super) fn pin_uri(kind: PinTargetKind, id: &str) -> Option<String> {
    let prefix = match kind {
        PinTargetKind::LikedSongs => return Some(LIKED_TRACKS.to_owned()),
        PinTargetKind::Playlist => "spotify:playlist:",
        PinTargetKind::Album => "spotify:album:",
        PinTargetKind::Artist => "spotify:artist:",
        _ => return None,
    };
    raw_id(id)?;
    Some(format!("{prefix}{id}"))
}

/// Reads the dedicated pin set and decorates supported pins with catalogue cards.
pub(super) async fn pins(session: &Session) -> Result<Vec<PinTarget>> {
    let mut seen = HashSet::new();
    let mut items = saved_items(session, PINS, "spotify:").await?;
    items.sort_by_key(|item| std::cmp::Reverse(item.added_at));
    let pins: Vec<_> = items
        .into_iter()
        .filter_map(|item| match item.uri.as_str() {
            "spotify:collection" | LIKED_TRACKS => {
                Some((PinTargetKind::LikedSongs, LIKED_TRACKS.to_owned()))
            }
            uri if uri.starts_with("spotify:playlist:") => {
                Some((PinTargetKind::Playlist, item.uri))
            }
            uri if uri.starts_with("spotify:album:") => Some((PinTargetKind::Album, item.uri)),
            uri if uri.starts_with("spotify:artist:") => Some((PinTargetKind::Artist, item.uri)),
            _ => None,
        })
        .filter(|(_, uri)| seen.insert(uri.clone()))
        .collect();
    let uris = pins.iter().map(|(_, uri)| uri.as_str()).collect();
    let mut cards = feeds::cards(session, uris, false).await?;
    Ok(pins
        .into_iter()
        .map(|(kind, uri)| {
            let id = uri.rsplit_once(':').unwrap_or_default().1;
            let card = match cards.remove(&uri) {
                Some(GenreItem::Playlist(item)) => Some((item.name, String::new(), item.cover)),
                Some(GenreItem::Album(item)) => Some((item.name, item.artists, item.cover)),
                Some(GenreItem::Artist(item)) => Some((item.name, String::new(), item.cover)),
                _ => None,
            };
            let (name, subtitle, cover) =
                card.unwrap_or_else(|| (id.to_owned(), String::new(), None));
            PinTarget {
                uri,
                name,
                subtitle,
                cover,
                kind,
                pinned: true,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_collection_items_and_preserves_absent_timestamps() {
        // Two nested items, followed by a continuation token.
        let page = PageResponse::decode(
            &b"\x0a\x06\x0a\x01a\x10\xac\x02\x0a\x05\x0a\x01b\x18\x01\x12\x04next"[..],
        )
        .unwrap();
        assert_eq!(page.next, "next");
        let [saved, removed] = page.items.as_slice() else {
            panic!("expected two items")
        };
        assert_eq!(
            (saved.uri.as_str(), saved.added_at, saved.is_removed),
            ("a", Some(300), false)
        );
        assert_eq!(
            (removed.uri.as_str(), removed.added_at, removed.is_removed),
            ("b", None, true)
        );
    }
}
