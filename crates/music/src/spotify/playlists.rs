use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::spotify::proto::metadata::ExtensionKind;
use crate::spotify::proto::playlist::{
    Add, AttributeKind, AttributesRequestedFields, ChangeInfo, CreateListReply, Delta,
    GetListRequest, GetListResponse, GetListsRequest, GetListsResponse, Item, ItemAttributes,
    ItemAttributesPartialState, ItemRequestedFields, Kind, ListAttributes,
    ListAttributesPartialState, ListChanges, ListEntity, ListEntityRequestedFields, ListIdentifier,
    ListResponseRequestedFields, Mov, Op, Pagination, RegisterPlaylistImageRequest,
    RegisterPlaylistImageResponse, Rem, RequestOptions, RequestedFields, SelectedListContent,
    SourceInfo, UpdateItemAttributes, UpdateListAttributes,
};
use crate::spotify::session::Session;
use anyhow::{Context as _, Result, anyhow};
use bytes::Bytes;
use futures::{StreamExt as _, stream};
use http::header::{ACCEPT, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use prost::Message as _;
use serde::Deserialize;

use crate::cover::{self, CoverLimits};
use crate::order;
use crate::spotify::{collection, profiles, transport, wire};
use crate::{Contributor, Playlist, PlaylistDetail, Track, escape};

const TRACK_PREFIX: &str = "spotify:track:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const COVER_SLACK: usize = 4;
/// Membership returns every track URI, so keep these batches smaller than cover slices.
const MEMBERSHIP_BATCH: usize = 8;
/// Revalidate bursts of change notifications, periodically rereading permissions and ownership.
const SUMMARY_AGE: Duration = Duration::from_secs(30);
const SOURCE_CLIENT: i32 = 2;
const PICTURE_ATTRIBUTE: i32 = 3;
/// Where the Spotify clients upload a playlist picture before they attach it.
const IMAGE_UPLOAD: &str = "https://image-upload.spotify.com/v4/playlist";
/// Spotify shows a playlist cover at 640 pixels at most, and its public API caps one at 256 KB.
const COVER_LIMITS: CoverLimits = CoverLimits {
    side: 640,
    bytes: 256 * 1024,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Uploaded {
    upload_token: String,
}

pub async fn create(session: &Session, name: &str) -> Result<String> {
    let body = changes(None, rename_op(name));
    let reply = post(session, "/playlist/v2/playlist", &body)
        .await
        .context("cannot create the playlist")?;
    let reply = CreateListReply::decode(reply).context("cannot decode the created playlist")?;

    let id = reply
        .uri
        .strip_prefix(PLAYLIST_PREFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("cannot read the created playlist id"))?;

    rename(session, &id, name).await?;

    add_to_library(session, &id).await?;

    Ok(id)
}

pub async fn add_to_library(session: &Session, playlist_id: &str) -> Result<()> {
    let uri = format!("{PLAYLIST_PREFIX}{playlist_id}");
    let revision = rootlist_revision(session, rootlist_uri(session)).await?;
    let body = changes(Some(&revision), add_op(&uri));
    rootlist_edit(session, &body)
        .await
        .context("cannot add the playlist to the library")
}

pub async fn rename(session: &Session, playlist_id: &str, name: &str) -> Result<()> {
    let body = changes(None, rename_op(name));
    edit(session, playlist_id, &body)
        .await
        .context("cannot rename the playlist")
}

/// Uploads `image` and makes it the playlist's picture, as the Spotify clients do, or takes the
/// picture off when it is `None` so Spotify shows its mosaic again.
pub async fn set_cover(session: &Session, playlist_id: &str, image: Option<&[u8]>) -> Result<()> {
    let op = match image {
        Some(image) => {
            let jpeg = cover::square_jpeg(image, COVER_LIMITS)?;
            let token = upload_image(session, jpeg)
                .await
                .context("cannot upload the cover")?;
            let picture = register_image(session, playlist_id, &token)
                .await
                .context("cannot register the cover")?;
            picture_op(picture)
        }
        None => no_picture_op(),
    };
    edit(session, playlist_id, &changes(None, op))
        .await
        .context("cannot change the playlist cover")
}

/// Sends a JPEG to Spotify's image upload host and answers the token that names it.
async fn upload_image(session: &Session, jpeg: Vec<u8>) -> Result<String> {
    let headers = HeaderMap::from_iter([
        (ACCEPT, HeaderValue::from_static("application/json")),
        (CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
    ]);
    let reply = transport::post_service(session, IMAGE_UPLOAD, headers, jpeg.into()).await?;
    let uploaded: Uploaded =
        serde_json::from_slice(&reply).context("cannot decode the upload reply")?;
    Ok(uploaded.upload_token)
}

/// Turns an upload token into the picture id a playlist attribute holds.
async fn register_image(session: &Session, playlist_id: &str, token: &str) -> Result<Bytes> {
    let endpoint = format!(
        "/playlist/v2/playlist/{}/register-image",
        escape::component(playlist_id)
    );
    let request = RegisterPlaylistImageRequest {
        upload_token: token.to_owned(),
    };
    let reply = post(session, &endpoint, &request).await?;
    let registered = RegisterPlaylistImageResponse::decode(reply)
        .context("cannot decode the registered picture")?;
    anyhow::ensure!(
        !registered.picture.is_empty(),
        "Spotify omitted the registered picture ID"
    );
    Ok(registered.picture)
}

/// Removes the rootlist entry at the revision and index Spotify returned.
pub async fn remove_from_library(session: &Session, playlist_id: &str) -> Result<()> {
    let uri = format!("{PLAYLIST_PREFIX}{playlist_id}");
    let (rootlist, index) = rootlist(session, &uri).await?;

    let body = changes(Some(&rootlist.revision), remove_op(&uri, index));
    rootlist_edit(session, &body)
        .await
        .context("cannot remove the playlist from the library")
}

pub async fn set_public(session: &Session, playlist_id: &str, public: bool) -> Result<()> {
    let uri = format!("{PLAYLIST_PREFIX}{playlist_id}");
    let (rootlist, index) = rootlist(session, &uri).await?;

    let body = changes(Some(&rootlist.revision), visibility_op(index, public));
    rootlist_edit(session, &body)
        .await
        .context("cannot change playlist visibility")
}

pub async fn add_track(session: &Session, playlist_id: &str, track_id: &str) -> Result<()> {
    let uri = format!("{TRACK_PREFIX}{track_id}");
    let content = edit_snapshot(session, playlist_id).await?;
    let body = changes(Some(&content.revision), add_op(&uri));

    edit(session, playlist_id, &body)
        .await
        .context("cannot add the track to the playlist")
}

pub async fn remove_track(session: &Session, playlist_id: &str, track_id: &str) -> Result<()> {
    let uri = format!("{TRACK_PREFIX}{track_id}");
    let content = edit_snapshot(session, playlist_id).await?;
    let index =
        position(&content, &uri).ok_or_else(|| anyhow!("cannot find the track in the playlist"))?;

    let body = changes(Some(&content.revision), remove_op(&uri, index));
    edit(session, playlist_id, &body)
        .await
        .context("cannot remove the track from the playlist")
}

/// Puts the listed tracks in the order `wanted` with one change on the revision it was read at,
/// once that listing is confirmed to be the one the listener saw, so an edit made elsewhere
/// before or after the read fails rather than moving the wrong rows. The listing leaves out
/// episodes and tracks with no metadata, as `tracks_from` does, so the moves are counted among
/// the listed tracks and placed among all the items.
pub async fn reorder(
    session: &Session,
    playlist_id: &str,
    seen: &[String],
    wanted: &[String],
) -> Result<()> {
    let content = edit_snapshot(session, playlist_id).await?;
    let uris: Vec<String> = content
        .items
        .iter()
        .filter(|uri| uri.starts_with(TRACK_PREFIX))
        .cloned()
        .collect();
    let known = collection::track_covers(session, &uris).await?;
    let held: Vec<usize> = content
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.starts_with(TRACK_PREFIX) && known.contains_key(*item))
        .map(|(index, _)| index)
        .collect();

    let ops = move_ops(&content, &held, seen, wanted)?;
    if ops.is_empty() {
        return Ok(());
    }
    let body = changes_of(Some(&content.revision), ops);
    edit(session, playlist_id, &body)
        .await
        .context("cannot reorder the playlist")
}

pub async fn playlist(session: &Session, playlist_id: &str) -> Result<PlaylistDetail> {
    let content = snapshot(session, playlist_id).await?;
    let mut playlist = wire::playlist_from(playlist_id, &content, session.username());
    let tracks = playlist_tracks_and_profiles(session, &content, &mut playlist).await?;

    Ok(PlaylistDetail {
        playlist,
        tracks,
        continuation: None,
    })
}

/// The last full summary and the revision it describes.
#[derive(Clone)]
pub(super) struct Summary {
    playlist: Playlist,
    revision: Bytes,
    read_at: Instant,
}

/// A Dealer revision can identify an outdated summary without a DiffLists request.
pub(super) fn invalidate_summary(session: &Session, id: &str, revision: &[u8]) {
    if revision.is_empty() {
        return;
    }
    let mut cache = session
        .playlist_summaries()
        .lock()
        .expect("Spotify playlist summary cache poisoned");
    if cache
        .get(id)
        .is_some_and(|entry| entry.revision != revision)
    {
        cache.remove(id);
    }
}

/// Revalidates recent summaries together, rereading full snapshots for changed playlists.
pub(super) async fn summaries(session: &Session, ids: Vec<String>) -> Vec<Playlist> {
    let mut pending: HashSet<_> = ids.into_iter().collect();
    let cache = session.playlist_summaries();
    let recent: Vec<_> = {
        let mut cache = cache
            .lock()
            .expect("Spotify playlist summary cache poisoned");
        cache.retain(|_, summary| summary.read_at.elapsed() < SUMMARY_AGE);
        pending
            .iter()
            .filter_map(|id| cache.get(id).cloned())
            .collect()
    };
    for summary in &recent {
        pending.remove(&summary.playlist.id);
    }
    // Cold entries do not need a revision check before their snapshot can start.
    let ((mut updated, changed), cold) = tokio::join!(
        refresh_recent_summaries(session, &recent),
        fetch_summaries(session, pending),
    );
    {
        let mut cache = cache
            .lock()
            .expect("Spotify playlist summary cache poisoned");
        for summary in cold.into_iter().chain(changed) {
            updated.push(summary.playlist.clone());
            if !summary.revision.is_empty() {
                cache.insert(summary.playlist.id.clone(), summary);
            }
        }
    }
    profiles::playlist_owners(session, &mut updated).await;
    updated
}

/// Reuses unchanged summaries and reads fresh snapshots for the rest.
async fn refresh_recent_summaries(
    session: &Session,
    recent: &[Summary],
) -> (Vec<Playlist>, Vec<Summary>) {
    let mut unchanged_playlists = Vec::new();
    let mut changed = Vec::new();
    for batch in recent.chunks(BATCH) {
        let revisions: Vec<_> = batch
            .iter()
            .map(|summary| (summary.playlist.id.as_str(), &summary.revision))
            .collect();
        match unchanged(session, &revisions).await {
            Ok(unchanged) => {
                for (summary, unchanged) in batch.iter().zip(unchanged) {
                    if unchanged {
                        unchanged_playlists.push(summary.playlist.clone());
                    } else {
                        changed.push(summary.playlist.id.clone());
                    }
                }
            }
            Err(error) => {
                log::debug!("playlists: cannot revalidate playlist summaries: {error:#}");
                changed.extend(batch.iter().map(|summary| summary.playlist.id.clone()));
            }
        }
    }
    (unchanged_playlists, fetch_summaries(session, changed).await)
}

async fn fetch_summaries(session: &Session, ids: impl IntoIterator<Item = String>) -> Vec<Summary> {
    let username = session.username();
    stream::iter(ids)
        .map(|id| async move {
            let endpoint = format!(
                "/playlist/v2/playlist/{}?from=0&length=0",
                escape::component(&id),
            );
            match read_snapshot(session, &endpoint).await {
                Ok(content) => Some(Summary {
                    playlist: wire::playlist_from(&id, &content, username),
                    // Keep the revision without retaining the response buffer in the cache.
                    revision: content
                        .revision
                        .as_deref()
                        .map(Bytes::copy_from_slice)
                        .unwrap_or_default(),
                    read_at: Instant::now(),
                }),
                Err(error) => {
                    log::warn!("playlists: cannot refresh Spotify playlist {id}: {error:#}");
                    None
                }
            }
        })
        .buffer_unordered(4)
        .filter_map(std::future::ready)
        .collect()
        .await
}

/// Fetches track metadata and credits together from the URIs in the playlist snapshot.
async fn playlist_tracks_and_profiles(
    session: &Session,
    content: &SelectedListContent,
    playlist: &mut Playlist,
) -> Result<Vec<Track>> {
    let items = track_items(content);
    let uris: HashSet<_> = items.clone().map(|item| item.uri.as_str()).collect();
    let mut ids: HashSet<_> = items
        .clone()
        .filter_map(|item| item.attributes.as_ref())
        .map(|attrs| profiles::username(attrs.added_by()))
        .filter(|id| !id.is_empty())
        .collect();
    if ids.len() < 2 && !playlist.blend {
        ids.clear();
    }
    let own_name = session
        .display_name()
        .filter(|_| playlist.owner_id == session.username());
    if own_name.is_none() && !playlist.owner_id.is_empty() {
        ids.insert(playlist.owner_id.clone());
    }
    let profiles: Vec<_> = ids
        .into_iter()
        .map(|id| format!("spotify:user:{id}"))
        .collect();
    let requests: Vec<_> = uris
        .iter()
        .map(|&uri| (uri, collection::TRACK_DETAILS))
        .chain(
            profiles
                .iter()
                .map(|uri| (uri.as_str(), &[ExtensionKind::UserProfile][..])),
        )
        .collect();
    let mut data = collection::extended_batch(session, &requests).await?;
    let known: HashMap<_, _> = uris
        .into_iter()
        .filter_map(|uri| Some((uri, collection::take_track(&mut data, uri)?)))
        .collect();
    let known_profiles: HashMap<_, _> = profiles
        .into_iter()
        .filter_map(|uri| {
            let profile = collection::take(&mut data, ExtensionKind::UserProfile, &uri)?;
            let card = profiles::contributor(&uri, profile);
            Some((card.id.clone(), Arc::new(card)))
        })
        .collect();
    let mut tracks: Vec<_> = items
        .filter_map(|item| Some(listed_track(known.get(item.uri.as_str())?, item)))
        .collect();
    if let Some(owner) = known_profiles.get(&playlist.owner_id) {
        playlist.owner.clone_from(&owner.name);
    } else if let Some(name) = own_name {
        name.clone_into(&mut playlist.owner);
    }
    let many = tracks
        .iter()
        .filter_map(|track| track.added_by.as_ref().map(|user| &user.id))
        .collect::<HashSet<_>>()
        .len()
        > 1;
    for track in &mut tracks {
        if !many && !playlist.blend {
            track.added_by = None;
        } else if let Some(added) = track.added_by.as_mut()
            && let Some(found) = known_profiles.get(&added.id)
        {
            *added = found.clone();
        }
    }
    Ok(tracks)
}

pub async fn playlist_tracks(session: &Session, playlist_id: &str) -> Result<Vec<Track>> {
    let content = snapshot(session, playlist_id).await?;
    tracks_from(session, &content).await
}

/// Reads every track, including lists whose covers are already known.
pub(super) async fn memberships(
    session: &Session,
    requests: &[(String, usize)],
) -> Vec<Result<crate::PlaylistMembership>> {
    let mut results = Vec::with_capacity(requests.len());
    for batch in requests.chunks(MEMBERSHIP_BATCH) {
        let queries = batch
            .iter()
            .map(|(id, wanted)| (Query::playlist(id, None), *wanted));
        results.extend(track_lists(session, queries, None).await);
    }
    results
}

/// Reads a bounded prefix for artwork previews.
pub(super) async fn covers(
    session: &Session,
    ids: &[String],
    wanted: usize,
) -> Vec<Result<Vec<String>>> {
    if wanted == 0 {
        return ids.iter().map(|_| Ok(Vec::new())).collect();
    }
    let limit = wanted.saturating_mul(COVER_SLACK);
    let length = limit.clamp(16, 256) as i32;
    let mut results = Vec::with_capacity(ids.len());
    for batch in ids.chunks(BATCH) {
        let queries = batch
            .iter()
            .map(|id| (Query::playlist(id, Some(length)), wanted));
        results.extend(
            track_lists(session, queries, Some(limit))
                .await
                .into_iter()
                .map(|row| Ok(row?.covers)),
        );
    }
    results
}

/// Resolves artwork once per batch. Full memberships remain usable when artwork fails.
async fn track_lists(
    session: &Session,
    requests: impl Iterator<Item = (Query, usize)>,
    preview_limit: Option<usize>,
) -> Vec<Result<crate::PlaylistMembership>> {
    let (queries, counts): (Vec<_>, Vec<_>) = requests.unzip();
    let rows = items(session, &queries).await;
    let tracks = |row: Snapshot| {
        row.items
            .into_iter()
            .filter(|uri| uri.starts_with(TRACK_PREFIX))
            .take(preview_limit.unwrap_or(usize::MAX))
            .collect::<Vec<_>>()
    };
    let rows: Vec<_> = rows.into_iter().map(|row| row.map(tracks)).collect();
    let candidates = rows
        .iter()
        .zip(&counts)
        .filter(|(_, count)| **count > 0)
        .filter_map(|(row, _)| row.as_ref().ok())
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    let known = collection::track_covers(session, &candidates).await;
    if let Err(error) = &known {
        log::warn!("playlists: cannot read covers: {error:#}");
    }
    rows.into_iter()
        .zip(counts)
        .map(|(row, wanted)| {
            let uris = row?;
            let covers = match &known {
                Ok(known) => crate::distinct_cover_urls(
                    uris.iter().filter_map(|uri| known.get(uri)?.as_deref()),
                    wanted,
                ),
                Err(error) if preview_limit.is_some() && !uris.is_empty() => {
                    return Err(anyhow!(
                        "cannot hydrate Spotify cover candidates: {error:#}"
                    ));
                }
                Err(_) => Vec::new(),
            };
            let tracks = if preview_limit.is_none() {
                uris.iter()
                    .filter_map(|uri| {
                        uri.strip_prefix(TRACK_PREFIX)
                            .filter(|id| !id.is_empty())
                            .map(str::to_owned)
                    })
                    .collect()
            } else {
                HashSet::new()
            };
            Ok(crate::PlaylistMembership { tracks, covers })
        })
        .collect()
}

async fn tracks_from(session: &Session, content: &SelectedListContent) -> Result<Vec<Track>> {
    let items = track_items(content);
    let uris: Vec<_> = items.clone().map(|item| item.uri.clone()).collect();
    let known = collection::metadata(session, &uris).await?;
    Ok(items
        .filter_map(|item| Some(listed_track(known.get(&item.uri)?, item)))
        .collect())
}

/// Only catalogue track entries can be hydrated into a playlist's track model.
fn track_items(content: &SelectedListContent) -> impl Iterator<Item = &Item> + Clone {
    content
        .contents
        .iter()
        .flat_map(|contents| &contents.items)
        .filter(|item| item.uri.starts_with(TRACK_PREFIX))
}

/// Copies a catalogue track into a specific occurrence in a playlist.
fn listed_track(track: &Track, item: &Item) -> Track {
    let mut track = track.clone();
    if let Some(attrs) = &item.attributes {
        track.added_at = wire::seconds(attrs.timestamp());
        let added_by = profiles::username(attrs.added_by());
        track.added_by = (!added_by.is_empty()).then(|| Arc::new(Contributor::unnamed(added_by)));
    }
    track
}

async fn snapshot(session: &Session, playlist_id: &str) -> Result<SelectedListContent> {
    let endpoint = format!("/playlist/v2/playlist/{}", escape::component(playlist_id));
    read_snapshot(session, &endpoint)
        .await
        .context("cannot read the playlist")
}

async fn read_snapshot(session: &Session, endpoint: &str) -> Result<SelectedListContent> {
    transport::get(session, endpoint, HeaderMap::new()).await
}

/// Reads the revision and ordered items used by indexed playlist edits.
async fn edit_snapshot(session: &Session, playlist_id: &str) -> Result<Snapshot> {
    projected_snapshot(session, format!("{PLAYLIST_PREFIX}{playlist_id}")).await
}

/// Keeps each list slot aligned with its index in the returned revision.
async fn projected_snapshot(session: &Session, uri: String) -> Result<Snapshot> {
    let snapshot = items(session, &[Query { uri, page: None }])
        .await
        .pop()
        .context("Spotify omitted the playlist")??;
    anyhow::ensure!(
        !snapshot.revision.is_empty(),
        "Spotify omitted the list revision"
    );
    anyhow::ensure!(
        snapshot.items.iter().all(|uri| !uri.is_empty()),
        "Spotify omitted an item URI"
    );
    Ok(snapshot)
}

async fn edit(session: &Session, playlist_id: &str, body: &ListChanges) -> Result<()> {
    let endpoint = format!(
        "/playlist/v2/playlist/{}/changes",
        escape::component(playlist_id)
    );
    post(session, &endpoint, body).await.map(|_| ())
}

/// Reads a rootlist page with ownership, visibility, permissions and other library metadata.
pub(super) async fn rootlist_page(
    session: &Session,
    from: usize,
    length: usize,
) -> Result<SelectedListContent> {
    let endpoint = format!(
        "/playlist/v2/user/{}/rootlist?decorate=revision,attributes,length,owner,capabilities,status_code&from={from}&length={length}",
        escape::component(session.username())
    );
    read_snapshot(session, &endpoint)
        .await
        .context("cannot read the rootlist")
}

fn rootlist_uri(session: &Session) -> String {
    format!("spotify:user:{}:rootlist", session.username())
}

async fn rootlist(session: &Session, uri: &str) -> Result<(Snapshot, i32)> {
    let rootlist = projected_snapshot(session, rootlist_uri(session)).await?;
    let index = position(&rootlist, uri)
        .ok_or_else(|| anyhow!("cannot find the playlist in the rootlist"))?;

    Ok((rootlist, index))
}

async fn rootlist_edit(session: &Session, body: &ListChanges) -> Result<()> {
    let endpoint = format!(
        "/playlist/v2/user/{}/rootlist/changes",
        escape::component(session.username())
    );
    post(session, &endpoint, body).await.map(|_| ())
}

async fn post(session: &Session, endpoint: &str, body: &impl prost::Message) -> Result<Bytes> {
    transport::post_proto(session, endpoint, transport::PROTOBUF, true, body).await
}

fn changes(revision: Option<&Bytes>, op: Op) -> ListChanges {
    changes_of(revision, vec![op])
}

/// One change that applies `ops` in turn on `revision`, or on whatever the list holds when
/// there is none.
fn changes_of(revision: Option<&Bytes>, ops: Vec<Op>) -> ListChanges {
    ListChanges {
        base_revision: revision.cloned(),
        deltas: vec![Delta {
            ops,
            info: Some(ChangeInfo {
                source: Some(SourceInfo {
                    client: Some(SOURCE_CLIENT),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }],
        want_resulting_revisions: Some(true),
    }
}

fn item(uri: &str) -> Item {
    Item {
        uri: uri.to_owned(),
        attributes: None,
    }
}

fn add_op(uri: &str) -> Op {
    Op {
        kind: Kind::Add as i32,
        add: Some(Add {
            items: vec![item(uri)],
            add_last: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn remove_op(uri: &str, index: i32) -> Op {
    Op {
        kind: Kind::Rem as i32,
        rem: Some(Rem {
            from_index: Some(index),
            length: Some(1),
            items: vec![item(uri)],
            items_as_key: Some(true),
        }),
        ..Default::default()
    }
}

/// The moves that put the listed rows in the order `wanted`, where `held` gives each listed
/// row's index among the items. Spotify applies them in turn and counts each `to_index` in the
/// items before that move, as the place the moved item goes in front of.
fn move_ops(
    content: &Snapshot,
    held: &[usize],
    seen: &[String],
    wanted: &[String],
) -> Result<Vec<Op>> {
    let items = &content.items;
    let listed: Vec<Option<&str>> = held
        .iter()
        .map(|&index| {
            items
                .get(index)
                .and_then(|item| item.strip_prefix(TRACK_PREFIX))
        })
        .collect();
    order::confirm(&listed, seen)?;
    let steps = order::moves(seen, wanted)?;
    let ops = order::placements(items.len(), held, &steps)?
        .into_iter()
        .map(|placement| Op {
            kind: Kind::Mov as i32,
            mov: Some(Mov {
                from_index: Some(placement.from as i32),
                length: Some(1),
                to_index: Some(placement.before as i32),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    Ok(ops)
}

fn rename_op(name: &str) -> Op {
    list_attributes_op(
        ListAttributes {
            name: Some(name.to_owned()),
            ..Default::default()
        },
        Vec::new(),
    )
}

fn picture_op(picture: Bytes) -> Op {
    list_attributes_op(
        ListAttributes {
            picture: Some(picture),
            ..Default::default()
        },
        Vec::new(),
    )
}

/// Clears the playlist picture using Spotify's attribute ID.
fn no_picture_op() -> Op {
    list_attributes_op(ListAttributes::default(), vec![PICTURE_ATTRIBUTE])
}

fn list_attributes_op(values: ListAttributes, no_value: Vec<i32>) -> Op {
    Op {
        kind: Kind::UpdateListAttributes as i32,
        update_list_attributes: Some(UpdateListAttributes {
            new_attributes: ListAttributesPartialState { values, no_value },
        }),
        ..Default::default()
    }
}

fn visibility_op(index: i32, public: bool) -> Op {
    Op {
        kind: Kind::UpdateItemAttributes as i32,
        update_item_attributes: Some(UpdateItemAttributes {
            index: Some(index),
            new_attributes: ItemAttributesPartialState {
                values: ItemAttributes {
                    public: Some(public),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn position(content: &Snapshot, uri: &str) -> Option<i32> {
    i32::try_from(content.items.iter().position(|item| item == uri)?).ok()
}

const GET_LISTS: &str = "/playlist/spotify.list.v1.ListReadService/GetLists";
const DIFF_LISTS: &str = "/playlist/spotify.list.v1.ListReadService/DiffLists";
const BATCH: usize = 100;

/// An ordered playlist snapshot with the revision used to submit an edit.
struct Snapshot {
    items: Vec<String>,
    revision: Bytes,
}

/// A list URI and an optional limit on entries from its start.
struct Query {
    uri: String,
    page: Option<i32>,
}

impl Query {
    fn playlist(id: &str, page: Option<i32>) -> Self {
        Self {
            uri: format!("spotify:playlist:{id}"),
            page,
        }
    }
}

/// Reads the root-list revision for appends, which do not need existing entries.
async fn rootlist_revision(session: &Session, uri: String) -> Result<Bytes> {
    let (_, revision) = read(session, &[Query { uri, page: None }], || fields([], []))
        .await?
        .pop()
        .context("Spotify omitted the root list")??;
    anyhow::ensure!(
        !revision.is_empty(),
        "Spotify omitted the root-list revision"
    );
    Ok(revision)
}

/// Reads ordered item URIs for several playlists, with independent page windows.
async fn items(session: &Session, queries: &[Query]) -> Vec<Result<Snapshot>> {
    let fields = || fields([], [AttributeKind::Uri]);
    let replies = match read(session, queries, fields).await {
        Ok(replies) => replies,
        Err(error) => {
            return queries
                .iter()
                .map(|query| Err(anyhow::anyhow!("cannot read {}: {error:#}", query.uri)))
                .collect();
        }
    };
    replies
        .into_iter()
        .map(|reply| {
            let (entity, revision) = reply?;
            let items = entity
                .items
                .into_iter()
                .map(|item| {
                    item.attributes
                        .map(|attributes| attributes.uri)
                        .unwrap_or_default()
                })
                .collect();
            Ok(Snapshot { items, revision })
        })
        .collect()
}

/// A Browse section with its ordered entity and optional navigation URIs.
pub(super) struct BrowseSection {
    pub(super) title: String,
    pub(super) items: Vec<(String, Option<String>)>,
}

/// Reads the requested Browse sections together, preserving their order and full directories.
pub(super) async fn browse_sections(
    session: &Session,
    uris: Vec<String>,
) -> Result<Vec<BrowseSection>> {
    if uris.is_empty() {
        return Ok(Vec::new());
    }
    let queries: Vec<_> = uris
        .into_iter()
        .map(|uri| Query { uri, page: None })
        .collect();
    read(session, &queries, || {
        fields(
            [AttributeKind::Name],
            [AttributeKind::Uri, AttributeKind::FormatListAttributes],
        )
    })
    .await?
    .into_iter()
    .map(|reply| {
        let (entity, _) = reply?;
        let title = entity
            .attributes
            .map(|attributes| attributes.name)
            .unwrap_or_default();
        let items = entity
            .items
            .into_iter()
            .filter_map(|item| {
                let attributes = item.attributes?;
                let target = attributes
                    .format_attributes
                    .and_then(|mut attributes| attributes.entries.remove("target_uri"));
                Some((attributes.uri, target))
            })
            .collect();
        Ok(BrowseSection { title, items })
    })
    .collect()
}

/// Reads the names, owners, and covers needed for playlist cards in one projection.
pub(super) async fn cards(session: &Session, ids: &[String]) -> HashMap<String, Playlist> {
    let fields = || {
        fields(
            [
                AttributeKind::Name,
                AttributeKind::OwnerUsername,
                AttributeKind::PictureFileId,
                AttributeKind::PictureSizes,
            ],
            [],
        )
    };
    let mut cards = HashMap::new();
    for ids in ids.chunks(BATCH) {
        let queries: Vec<_> = ids.iter().map(|id| Query::playlist(id, None)).collect();
        let replies = match read(session, &queries, fields).await {
            Ok(replies) => replies,
            Err(error) => {
                log::debug!("playlists: cannot read playlist cards: {error:#}");
                continue;
            }
        };
        for (id, reply) in ids.iter().zip(replies) {
            let Ok((entity, _)) = reply else {
                continue;
            };
            let Some(attributes) = entity.attributes else {
                continue;
            };
            if attributes.name.is_empty() {
                continue;
            }
            let cover = attributes
                .picture_sizes
                .iter()
                .flat_map(|sizes| &sizes.picture_size)
                .rev()
                .find_map(|size| wire::playlist_image(&size.url))
                .or_else(|| match std::str::from_utf8(&attributes.picture_file_id) {
                    Ok(link) if link.starts_with("spotify:image:") => wire::playlist_image(link),
                    _ if attributes.picture_file_id.len() == 20 => {
                        wire::image_url(&attributes.picture_file_id)
                    }
                    _ => None,
                });
            cards.insert(
                format!("spotify:playlist:{id}"),
                Playlist {
                    id: id.clone(),
                    name: attributes.name,
                    owner_id: attributes.owner_username,
                    cover,
                    public: true,
                    ..Default::default()
                },
            );
        }
    }
    cards
}

/// Selects list and item attributes independently for GetLists and DiffLists.
fn fields<const L: usize, const I: usize>(
    list: [AttributeKind; L],
    items: [AttributeKind; I],
) -> RequestedFields {
    fn attributes<const N: usize>(kinds: [AttributeKind; N]) -> Option<AttributesRequestedFields> {
        (N > 0).then(|| AttributesRequestedFields {
            requested_field: kinds.into_iter().map(|kind| kind as i32).collect(),
        })
    }
    let entity = ListEntityRequestedFields {
        attributes: attributes(list),
        items: attributes(items).map(|attributes| ItemRequestedFields {
            attributes: Some(attributes),
        }),
    };
    RequestedFields {
        list_response: ListResponseRequestedFields { entity },
    }
}

type ListResult = Result<(ListEntity, Bytes)>;

/// An empty per-list reply confirms the supplied revision. Any other reply needs a fresh read.
async fn unchanged(session: &Session, revisions: &[(&str, &Bytes)]) -> Result<Vec<bool>> {
    let requests = revisions
        .iter()
        .map(|(id, revision)| GetListRequest {
            identifier: ListIdentifier {
                uri: format!("spotify:playlist:{id}"),
                revision: (*revision).clone(),
            },
            options: RequestOptions {
                requested_fields: fields([AttributeKind::Name], []),
                pagination: None,
            },
        })
        .collect();
    let response: GetListsResponse =
        transport::rpc(session, DIFF_LISTS, GetListsRequest { requests }).await?;
    anyhow::ensure!(
        response.responses.len() == revisions.len(),
        "Spotify returned the wrong number of playlist revision checks"
    );
    Ok(response
        .responses
        .iter()
        .map(bytes::Bytes::is_empty)
        .collect())
}

/// Preserves independent successes and errors in the request order.
async fn read(
    session: &Session,
    queries: &[Query],
    fields: impl Fn() -> RequestedFields,
) -> Result<Vec<ListResult>> {
    // Duplicate list projections can fail the entire request.
    let mut slots = HashMap::new();
    let mut requests = Vec::new();
    let order: Vec<_> = queries
        .iter()
        .map(|query| {
            *slots.entry((&query.uri, query.page)).or_insert_with(|| {
                let index = requests.len();
                requests.push(GetListRequest {
                    identifier: ListIdentifier {
                        uri: query.uri.clone(),
                        revision: Bytes::new(),
                    },
                    options: RequestOptions {
                        requested_fields: fields(),
                        pagination: query.page.map(|length| Pagination { length }),
                    },
                });
                index
            })
        })
        .collect();
    let response: GetListsResponse =
        transport::rpc(session, GET_LISTS, GetListsRequest { requests }).await?;
    anyhow::ensure!(
        response.responses.len() == slots.len(),
        "Spotify returned the wrong number of lists"
    );
    Ok(queries
        .iter()
        .zip(order)
        .map(|(query, index)| {
            GetListResponse::decode(response.responses[index].clone())
                .map_err(anyhow::Error::from)
                .and_then(decode)
                .with_context(|| format!("cannot read {}", query.uri))
        })
        .collect())
}

fn decode(response: GetListResponse) -> ListResult {
    let info = response.info.unwrap_or_default();
    if let Some(error) = info.request_error {
        anyhow::bail!("Spotify could not read the playlist: {}", error.message);
    }
    let entity = response
        .entity
        .context("Spotify omitted the playlist items")?;
    Ok((entity, info.revision))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(uris: &[&str]) -> Snapshot {
        Snapshot {
            items: uris.iter().map(|uri| (*uri).to_owned()).collect(),
            revision: Bytes::from_static(b"rev"),
        }
    }

    #[test]
    fn reorders_track_positions_around_episodes_and_rejects_stale_order() {
        let content = listing(&[
            "spotify:track:a",
            "spotify:episode:e",
            "spotify:track:b",
            "spotify:track:c",
        ]);
        let held = [0, 2, 3];
        let shown = ["a", "b", "c"].map(str::to_owned);
        for (wanted, from, to) in [(["b", "c", "a"], 0, 4), (["c", "a", "b"], 3, 0)] {
            let ops = move_ops(&content, &held, &shown, &wanted.map(str::to_owned)).unwrap();
            let [op] = ops.as_slice() else {
                panic!("expected one move")
            };
            assert_eq!(op.kind(), Kind::Mov);
            let movement = op.mov.as_ref().unwrap();
            assert_eq!(
                (
                    movement.from_index(),
                    movement.length(),
                    movement.to_index()
                ),
                (from, 1, to)
            );
        }
        let stale = ["b", "a", "c"].map(str::to_owned);
        assert!(move_ops(&content, &held, &stale, &shown).is_err());
    }

    #[test]
    fn writes_append_and_indexed_removal_on_the_current_revision() {
        for uri in ["spotify:track:b", "spotify:playlist:b"] {
            let content = listing(&["spotify:track:a", uri]);
            let index = position(&content, uri).unwrap();
            let body = changes_of(
                Some(&content.revision),
                vec![add_op(uri), remove_op(uri, index)],
            );
            assert_eq!(body.base_revision.as_ref(), Some(&content.revision));
            let [add, rem] = body.deltas[0].ops.as_slice() else {
                panic!("expected add and remove")
            };
            assert_eq!((add.kind(), rem.kind()), (Kind::Add, Kind::Rem));
            let add = add.add.as_ref().unwrap();
            let rem = rem.rem.as_ref().unwrap();
            assert!(add.add_last());
            assert_eq!(add.items[0].uri, uri);
            assert_eq!((rem.from_index(), rem.length()), (1, 1));
            assert!(rem.items_as_key());
            assert_eq!(rem.items[0].uri, uri);
        }
    }

    #[test]
    fn visibility_targets_the_rootlist_index() {
        let op = visibility_op(3, true);
        let update = op.update_item_attributes.as_ref().unwrap();

        assert_eq!(op.kind(), Kind::UpdateItemAttributes);
        assert_eq!(update.index(), 3);
        assert!(update.new_attributes.values.public());
    }
}
