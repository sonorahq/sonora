use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::spotify::session::Session;
use anyhow::{Context as _, Result};
use futures::{Stream, StreamExt as _, TryStreamExt as _, stream};
use spotifyapis::id::{base62, raw_id};
use spotifyapis::proto::extended_metadata::ImageSize;
use spotifyapis::proto::extended_metadata::{
    Album as AlbumMessage, Artist as ArtistMessage, Track as TrackMessage,
};
use spotifyapis::proto::extended_metadata::{MetadataMessage, MetadataPayload};
use tokio::sync::mpsc;

use super::albums::{ALBUM_PREFIX, hydrate_saved_albums};
use crate::spotify::{auth::sign_in_error, feeds, wire};
use crate::{
    ArtistRef, Credit, GenreItem, Page, PinTarget, PinTargetKind, SavedCollectionPages, Track,
};
use spotifyapis::proto::collection::{CollectionItem, PageRequest, WriteRequest};
use spotifyapis::proto::extended_metadata::{
    AudioAttributes, ExtensionKind, MetadataKey, Reputation, TrackDescriptors,
};

const LIKED_TRACKS: &str = "spotify:collection:tracks";
pub(super) use spotifyapis::id::TRACK_PREFIX;
const UNKNOWN: &str = "Unknown";
pub(super) const BATCH: usize = 500;
pub(super) const TRACK_DETAILS: &[ExtensionKind] = &[
    ExtensionKind::TrackV4,
    ExtensionKind::TrackDescriptor,
    ExtensionKind::OnPlatformReputation,
    ExtensionKind::AudioAttributesV2,
];

pub(crate) const COLLECTION: &str = "collection";
pub(crate) const ARTISTS: &str = "artist";
pub(super) const PINS: &str = "ylpin";
const PAGE: i32 = 300;
// Main collection pages contain both tracks and albums.
const COLLECTION_PAGE: i32 = 2_000;

static UPDATE: AtomicU64 = AtomicU64::new(0);

/// Reads the mixed collection once, then hydrates each list independently.
pub(super) fn saved_collection(session: Session) -> SavedCollectionPages {
    let (tracks_out, tracks) = mpsc::channel(1);
    let (albums_out, albums) = mpsc::channel(1);
    tokio::spawn(async move {
        let items = tokio::select! {
            items = saved_items(&session, COLLECTION, "spotify:") => items,
            () = async { tokio::join!(tracks_out.closed(), albums_out.closed()); } => return,
        };
        let items = match items {
            Ok(items) => items,
            Err(error) => {
                tracks_out.try_send(Err(anyhow::anyhow!("{error:#}"))).ok();
                albums_out.try_send(Err(error)).ok();
                return;
            }
        };
        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        {
            let mut seen = HashSet::new();
            for item in items {
                if item.uri.starts_with(TRACK_PREFIX) && seen.insert(item.uri.clone()) {
                    tracks.push(item);
                } else if item.uri.starts_with(ALBUM_PREFIX) {
                    albums.push(item);
                }
            }
        }
        tokio::join!(
            send_saved_tracks(&session, tracks_out, &tracks),
            send_saved_page(albums_out, hydrate_saved_albums(&session, albums)),
        );
    });
    SavedCollectionPages { tracks, albums }
}

/// Finishes one list independently and stops its hydration if the consumer leaves.
async fn send_saved_page<T>(
    out: mpsc::Sender<Result<Page<T>>>,
    work: impl Future<Output = Result<Vec<T>>>,
) {
    tokio::select! {
        biased;
        () = out.closed() => {}
        result = work => {
            out.try_send(result.map(|items| Page { total: Some(items.len()), items })).ok();
        }
    }
}

/// Streams hydrated tracks in collection order with two catalogue reads in flight.
async fn send_saved_tracks(
    session: &Session,
    out: mpsc::Sender<Result<Page<Track>>>,
    tracks: &[CollectionItem],
) {
    let requests: Vec<_> = tracks
        .chunks(BATCH)
        .map(|batch| hydrate_saved_tracks(session, batch))
        .collect();
    let batches = stream::iter(requests).buffered(2);
    tokio::pin!(batches);
    loop {
        let next = tokio::select! {
            () = out.closed() => return,
            next = batches.next() => next,
        };
        let Some(result) = next else { return };
        let failed = result.is_err();
        if out
            .send(result.map(|items| Page { total: None, items }))
            .await
            .is_err()
            || failed
        {
            return;
        }
    }
}

/// Hydrates saved tracks in collection order and applies their saved dates.
pub(super) async fn hydrate_saved_tracks(
    session: &Session,
    items: &[CollectionItem],
) -> Result<Vec<Track>> {
    let uris: Vec<_> = items.iter().map(|item| item.uri.clone()).collect();
    let mut known = metadata(session, &uris).await?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let mut track = known.remove(&item.uri)?;
            track.added_at = item.added_at.map(i64::from);
            Some(track)
        })
        .collect())
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
pub(super) fn take_track(
    data: &mut HashMap<MetadataKey, MetadataPayload>,
    uri: &str,
) -> Option<Track> {
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
pub(super) async fn project<M: MetadataMessage, T>(
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
    mut map: impl FnMut(&mut HashMap<MetadataKey, MetadataPayload>, &str) -> Option<T>,
) -> Result<HashMap<String, T>> {
    let mut seen = HashSet::new();
    let requests: Vec<_> = uris
        .iter()
        .map(String::as_str)
        .filter(|uri| seen.insert(*uri))
        .map(|uri| (uri, kinds))
        .collect();
    let mut known = HashMap::with_capacity(requests.len());
    let mut batches = std::pin::pin!(metadata_batches(session, &requests, batch));
    while let Some(result) = batches.next().await {
        let (requests, mut values) = result?;
        known.extend(
            requests
                .iter()
                .filter_map(|&(uri, _)| Some((uri.to_owned(), map(&mut values, uri)?))),
        );
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

/// Fetches different extensions together and indexes their payloads.
pub(super) async fn extended_batch(
    session: &Session,
    requests: &[(&str, &[ExtensionKind])],
) -> Result<HashMap<MetadataKey, MetadataPayload>> {
    metadata_batches(session, requests, BATCH)
        .try_fold(HashMap::new(), |mut known, (_, values)| async move {
            known.extend(values);
            Ok(known)
        })
        .await
}

pub(super) type MetadataBatch<'a> = (
    &'a [(&'a str, &'a [ExtensionKind])],
    HashMap<MetadataKey, MetadataPayload>,
);

/// Reads batches in order with two requests in flight and releases replies as callers consume them.
pub(super) fn metadata_batches<'a>(
    session: &'a Session,
    requests: &'a [(&'a str, &'a [ExtensionKind])],
    size: usize,
) -> impl Stream<Item = Result<MetadataBatch<'a>>> + use<'a> {
    stream::iter(requests.chunks(size))
        .map(move |batch| async move {
            let queries: Vec<_> = batch
                .iter()
                .flat_map(|&(uri, kinds)| kinds.iter().map(move |&kind| (uri, kind)))
                .collect();
            let response = session
                .api()
                .extended_metadata(queries)
                .await
                .map_err(sign_in_error)?;
            Ok((batch, response))
        })
        .buffered(2)
}

/// Decodes one requested extension from a mixed metadata batch.
pub(super) fn take<T: MetadataMessage>(
    data: &mut HashMap<MetadataKey, MetadataPayload>,
    kind: ExtensionKind,
    uri: &str,
) -> Option<T> {
    data.remove(&(kind, uri.to_owned()))?.decode().ok()
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
    let connection = session.connection_id().to_owned();
    session
        .api()
        .update_collection(&request, &connection)
        .await
        .map_err(sign_in_error)
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
            limit: if set == COLLECTION {
                COLLECTION_PAGE
            } else {
                PAGE
            },
        };
        let page = session
            .api()
            .collection_page(&request)
            .await
            .map_err(sign_in_error)
            .context("cannot read the saved collection")?;

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
    let mut cards = feeds::cards(session, uris, HashMap::new(), false).await?;
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
