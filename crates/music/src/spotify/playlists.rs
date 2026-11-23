use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::spotify::session::Session;
use anyhow::{Context as _, Result, anyhow};
use bytes::Bytes;
use futures::{StreamExt as _, stream};
use spotifyapis::proto::list_read::{AttributeKind, Attributes, ListProjection, ListQuery};
use spotifyapis::proto::metadata::ExtensionKind;
use spotifyapis::proto::playlist::{
    Add, ChangeInfo, Delta, Item, ItemAttributes, ItemAttributesPartialState, Kind, ListAttributes,
    ListAttributesPartialState, ListChanges, Mov, Op, RegisterPlaylistImageRequest, Rem,
    SelectedListContent, SourceInfo, UpdateItemAttributes, UpdateListAttributes,
};
use tokio::sync::mpsc;

use crate::cover::{self, CoverLimits};
use crate::order;
use crate::spotify::{auth::sign_in_error, collection, profiles, wire};
use crate::{Contributor, Playlist, PlaylistDetail, PlaylistPreview, Track};

const TRACK_PREFIX: &str = "spotify:track:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const COVER_SLACK: usize = 4;
/// Membership returns every track URI, so keep these batches smaller than cover slices.
const MEMBERSHIP_BATCH: usize = 8;
const CARD_BATCH: usize = 128;
const SOURCE_CLIENT: i32 = 2;
const PICTURE_ATTRIBUTE: i32 = 3;
/// Spotify shows a playlist cover at 640 pixels at most, and its public API caps one at 256 KB.
const COVER_LIMITS: CoverLimits = CoverLimits {
    side: 640,
    bytes: 256 * 1024,
};

pub async fn create(session: &Session, name: &str) -> Result<String> {
    let body = changes(None, rename_op(name));
    let connection = session.connection_id().to_owned();
    let reply = session
        .api()
        .create_playlist(&body, &connection)
        .await
        .map_err(sign_in_error)
        .context("cannot create the playlist")?;

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
    let body = changes(None, add_op([item(uri)]));
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

/// Registers a cover, then updates the cover and any new name in the same playlist write.
pub async fn edit_appearance(
    session: &Session,
    playlist_id: &str,
    name: Option<&str>,
    image: Option<&[u8]>,
) -> Result<Option<String>> {
    let picture = match image {
        Some(image) => {
            let jpeg = cover::square_jpeg(image, COVER_LIMITS)?;
            let token = upload_image(session, jpeg)
                .await
                .context("cannot upload the cover")?;
            let picture = register_image(session, playlist_id, &token)
                .await
                .context("cannot register the cover")?;
            Some(picture)
        }
        None => None,
    };
    let cover = picture.as_deref().and_then(wire::image_url);
    let cleared = if picture.is_none() {
        vec![PICTURE_ATTRIBUTE]
    } else {
        Vec::new()
    };
    let op = list_attributes_op(
        ListAttributes {
            name: name.map(str::to_owned),
            picture,
            ..Default::default()
        },
        cleared,
    );
    edit(session, playlist_id, &changes(None, op))
        .await
        .context("cannot change the playlist appearance")?;
    Ok(cover)
}

/// Sends a JPEG to Spotify's image upload host and answers the token that names it.
async fn upload_image(session: &Session, jpeg: Vec<u8>) -> Result<String> {
    session
        .api()
        .upload_playlist_image(jpeg.into())
        .await
        .map_err(sign_in_error)
}

/// Turns an upload token into the picture id a playlist attribute holds.
async fn register_image(session: &Session, playlist_id: &str, token: &str) -> Result<Bytes> {
    let connection = session.connection_id().to_owned();
    let request = RegisterPlaylistImageRequest {
        upload_token: token.to_owned(),
    };
    let registered = session
        .api()
        .register_playlist_image(playlist_id, &request, &connection)
        .await
        .map_err(sign_in_error)
        .context("cannot register the playlist picture")?;
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

pub async fn add_tracks(session: &Session, playlist_id: &str, track_ids: &[String]) -> Result<()> {
    if track_ids.is_empty() {
        return Ok(());
    }
    let items = track_ids
        .iter()
        .map(|id| item(format!("{TRACK_PREFIX}{id}")));
    let body = changes(None, add_op(items));

    edit(session, playlist_id, &body)
        .await
        .context("cannot add tracks to the playlist")
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

/// Loads the complete snapshot and publishes ordered previews while hydrating it.
pub(super) async fn playlist(
    session: &Session,
    playlist_id: &str,
    preview: Option<mpsc::Sender<PlaylistPreview>>,
) -> Result<PlaylistDetail> {
    let content = snapshot(session, playlist_id).await?;
    let mut playlist = wire::playlist_from(playlist_id, &content, session.username());
    let tracks = playlist_tracks_and_profiles(session, &content, &mut playlist, preview).await?;

    Ok(PlaylistDetail {
        playlist,
        tracks,
        continuation: None,
    })
}

/// Reads current playlist summaries without fetching their entries.
pub(super) async fn summaries(session: &Session, mut ids: Vec<String>) -> Vec<Playlist> {
    let username = session.username();
    ids.sort_unstable();
    ids.dedup();
    let mut playlists = stream::iter(ids)
        .map(|id| async move {
            match session
                .api()
                .playlist(&id, true)
                .await
                .map_err(sign_in_error)
            {
                Ok(content) => Some(wire::playlist_from(&id, &content, username)),
                Err(error) => {
                    log::warn!("playlists: cannot refresh Spotify playlist {id}: {error:#}");
                    None
                }
            }
        })
        .buffer_unordered(4)
        .filter_map(std::future::ready)
        .collect::<Vec<_>>()
        .await;
    profiles::playlist_owners(session, &mut playlists).await;
    playlists
}

/// Fetches track metadata and credits together from the URIs in the playlist snapshot.
async fn playlist_tracks_and_profiles(
    session: &Session,
    content: &SelectedListContent,
    playlist: &mut Playlist,
    preview: Option<mpsc::Sender<PlaylistPreview>>,
) -> Result<Vec<Track>> {
    let items = track_items(content);
    let mut ids: HashSet<_> = items
        .clone()
        .filter_map(|item| item.attributes.as_ref())
        .map(|attrs| profiles::username(attrs.added_by()))
        .filter(|id| !id.is_empty())
        .collect();
    let show_contributors = ids.len() > 1 || playlist.blend;
    if !show_contributors {
        ids.clear();
    }
    let own_name = session
        .display_name()
        .filter(|_| playlist.owner_id == session.username());
    if let Some(name) = own_name {
        name.clone_into(&mut playlist.owner);
    } else if !playlist.owner_id.is_empty() {
        ids.insert(playlist.owner_id.clone());
    }
    let profile_uris: Vec<_> = ids
        .into_iter()
        .map(|id| format!("spotify:user:{id}"))
        .collect();
    let mut pending = HashSet::new();
    let mut requests: Vec<_> = profile_uris
        .iter()
        .map(|uri| (uri.as_str(), &[ExtensionKind::UserProfile][..]))
        .collect();
    requests.extend(
        items
            .clone()
            .map(|item| item.uri.as_str())
            .filter(|uri| pending.insert(*uri))
            .map(|uri| (uri, collection::TRACK_DETAILS)),
    );
    let batches: Vec<_> = requests
        .chunks(collection::BATCH)
        .map(|batch| async move {
            Ok::<_, anyhow::Error>((batch, collection::extended_batch(session, batch).await?))
        })
        .collect();
    let mut batches = stream::iter(batches).buffered(2);
    let mut items = items.peekable();
    let mut known = HashMap::new();
    let mut profiles = HashMap::new();
    let mut tracks = Vec::new();
    while let Some(batch) = batches.next().await {
        let (requests, mut data) = batch?;
        for &(uri, kinds) in requests {
            if kinds == [ExtensionKind::UserProfile] {
                if let Some(data) = collection::take(&mut data, ExtensionKind::UserProfile, uri) {
                    let card = profiles::contributor(uri, data);
                    if card.id == playlist.owner_id && own_name.is_none() {
                        playlist.owner.clone_from(&card.name);
                    }
                    profiles.insert(card.id.clone(), Arc::new(card));
                }
            } else {
                pending.remove(uri);
                if let Some(track) = collection::take_track(&mut data, uri) {
                    known.insert(uri, track);
                }
            }
        }
        let start = tracks.len();
        while let Some(item) = items.next_if(|item| !pending.contains(item.uri.as_str())) {
            let Some(track) = known.get(item.uri.as_str()) else {
                continue;
            };
            let mut track = listed_track(track, item);
            if !show_contributors {
                track.added_by = None;
            } else if let Some(added) = track.added_by.as_mut()
                && let Some(found) = profiles.get(&added.id)
            {
                *added = found.clone();
            }
            tracks.push(track);
        }
        if !pending.is_empty()
            && start < tracks.len()
            && let Some(out) = &preview
        {
            out.send(PlaylistPreview {
                playlist: playlist.clone(),
                tracks: tracks[start..].to_vec(),
            })
            .await
            .ok();
        }
    }
    if !playlist.blend && show_contributors {
        let mut contributors = tracks.iter().filter_map(|track| track.added_by.as_ref());
        if let Some(first) = contributors.next()
            && contributors.all(|added| added.id == first.id)
        {
            for track in &mut tracks {
                track.added_by = None;
            }
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
            .map(|(id, wanted)| (ListQuery::playlist(id, None), *wanted));
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
            .map(|id| (ListQuery::playlist(id, Some(length)), wanted));
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
    requests: impl Iterator<Item = (ListQuery, usize)>,
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
    session
        .api()
        .playlist(playlist_id, false)
        .await
        .map_err(sign_in_error)
        .context("cannot read the playlist")
}

/// Reads the revision and ordered items used by indexed playlist edits.
async fn edit_snapshot(session: &Session, playlist_id: &str) -> Result<Snapshot> {
    projected_snapshot(session, format!("{PLAYLIST_PREFIX}{playlist_id}")).await
}

/// Keeps each list slot aligned with its index in the returned revision.
async fn projected_snapshot(session: &Session, uri: String) -> Result<Snapshot> {
    let snapshot = items(session, &[ListQuery { uri, page: None }])
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
    let connection = session.connection_id().to_owned();
    session
        .api()
        .edit_playlist(playlist_id, body, &connection)
        .await
        .map_err(sign_in_error)
}

async fn rootlist(session: &Session, uri: &str) -> Result<(Snapshot, i32)> {
    let rootlist_uri = format!("spotify:user:{}:rootlist", session.username());
    let rootlist = projected_snapshot(session, rootlist_uri).await?;
    let index = position(&rootlist, uri)
        .ok_or_else(|| anyhow!("cannot find the playlist in the rootlist"))?;

    Ok((rootlist, index))
}

async fn rootlist_edit(session: &Session, body: &ListChanges) -> Result<()> {
    let connection = session.connection_id().to_owned();
    session
        .api()
        .edit_rootlist(body, &connection)
        .await
        .map_err(sign_in_error)
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

fn item(uri: impl Into<String>) -> Item {
    Item {
        uri: uri.into(),
        attributes: None,
    }
}

fn add_op(items: impl IntoIterator<Item = Item>) -> Op {
    Op {
        kind: Kind::Add as i32,
        add: Some(Add {
            items: items.into_iter().collect(),
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
    order::placements(items.len(), held, &steps)?
        .into_iter()
        .map(|placement| {
            Ok(Op {
                kind: Kind::Mov as i32,
                mov: Some(Mov {
                    from_index: Some(i32::try_from(placement.from)?),
                    length: Some(1),
                    to_index: Some(i32::try_from(placement.before)?),
                    ..Default::default()
                }),
                ..Default::default()
            })
        })
        .collect()
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

const BATCH: usize = 100;

/// An ordered playlist snapshot with the revision used to submit an edit.
struct Snapshot {
    items: Vec<String>,
    revision: Bytes,
}

/// Reads ordered item URIs for several playlists, with independent page windows.
async fn items(session: &Session, queries: &[ListQuery]) -> Vec<Result<Snapshot>> {
    let projection = ListProjection {
        items: &[AttributeKind::Uri],
        ..Default::default()
    };
    let replies = match session
        .api()
        .get_lists(queries, projection)
        .await
        .map_err(sign_in_error)
    {
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
    pub(super) playlists: HashMap<String, Playlist>,
}

const CARD_FIELDS: [AttributeKind; 4] = [
    AttributeKind::Name,
    AttributeKind::OwnerUsername,
    AttributeKind::PictureFileId,
    AttributeKind::PictureSizes,
];

/// Reads the requested Browse sections together, preserving their order and full directories.
pub(super) async fn browse_sections(
    session: &Session,
    uris: Vec<String>,
    with_cards: bool,
) -> Result<Vec<BrowseSection>> {
    if uris.is_empty() {
        return Ok(Vec::new());
    }
    let queries: Vec<_> = uris
        .into_iter()
        .map(|uri| ListQuery { uri, page: None })
        .collect();
    let projection = ListProjection {
        list: &[AttributeKind::Name],
        items: &[AttributeKind::Uri, AttributeKind::FormatListAttributes],
        referenced: if with_cards { &CARD_FIELDS } else { &[] },
    };
    session
        .api()
        .get_lists(&queries, projection)
        .await
        .map_err(sign_in_error)?
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
            let playlists = entity
                .meta_items
                .into_iter()
                .filter_map(|(uri, meta)| {
                    let card = playlist_card(uri.strip_prefix(PLAYLIST_PREFIX)?, meta.attributes?)?;
                    // Incomplete projections still get the same direct read as other cards.
                    (!card.owner_id.is_empty() && card.cover.is_some()).then_some((uri, card))
                })
                .collect();
            Ok(BrowseSection {
                title,
                items,
                playlists,
            })
        })
        .collect()
}

/// Reads the names, owners, and covers needed for playlist cards in one projection.
pub(super) async fn cards(session: &Session, ids: &[String]) -> HashMap<String, Playlist> {
    let projection = ListProjection {
        list: &CARD_FIELDS,
        ..Default::default()
    };
    let mut cards = HashMap::new();
    for ids in ids.chunks(CARD_BATCH) {
        let queries: Vec<_> = ids.iter().map(|id| ListQuery::playlist(id, None)).collect();
        let replies = match session
            .api()
            .get_lists(&queries, projection)
            .await
            .map_err(sign_in_error)
        {
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
            if let Some(card) = entity.attributes.and_then(|attrs| playlist_card(id, attrs)) {
                cards.insert(format!("{PLAYLIST_PREFIX}{id}"), card);
            }
        }
    }
    cards
}

fn playlist_card(id: &str, attributes: Attributes) -> Option<Playlist> {
    if attributes.name.is_empty() {
        return None;
    }
    let cover = attributes
        .picture_sizes
        .as_ref()
        .and_then(|sizes| {
            let sizes = &sizes.picture_size;
            ["default", "large", "xlarge", "small"]
                .into_iter()
                .find_map(|target| {
                    sizes
                        .iter()
                        .filter(|size| size.target_name == target)
                        .find_map(|size| wire::playlist_image(&size.url))
                })
                .or_else(|| {
                    sizes
                        .iter()
                        .rev()
                        .find_map(|size| wire::playlist_image(&size.url))
                })
        })
        .or_else(|| match std::str::from_utf8(&attributes.picture_file_id) {
            Ok(link) if link.starts_with("spotify:image:") => wire::playlist_image(link),
            _ if attributes.picture_file_id.len() == 20 => {
                wire::image_url(&attributes.picture_file_id)
            }
            _ => None,
        });
    Some(Playlist {
        id: id.to_owned(),
        name: attributes.name,
        owner_id: attributes.owner_username,
        cover,
        public: true,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_edits_use_original_positions_and_revision() {
        let content = Snapshot {
            items: ["spotify:track:a", "spotify:episode:e", "spotify:track:b"]
                .map(str::to_owned)
                .into(),
            revision: Bytes::from_static(b"revision"),
        };
        let shown = ["a", "b"].map(str::to_owned);
        let reversed = ["b", "a"].map(str::to_owned);
        let held = [0, 2];
        assert!(move_ops(&content, &held, &reversed, &shown).is_err());
        let mut ops = move_ops(&content, &held, &shown, &reversed).unwrap();
        let move_op = ops[0].mov.as_ref().unwrap();
        assert_eq!((move_op.from_index(), move_op.to_index()), (2, 0));
        ops.push(remove_op("spotify:track:b", 2));
        let body = changes_of(Some(&content.revision), ops);
        assert_eq!(body.base_revision.as_ref(), Some(&content.revision));
        assert_eq!(body.deltas[0].ops[1].rem.as_ref().unwrap().from_index(), 2);

        let update = visibility_op(7, true).update_item_attributes.unwrap();
        assert_eq!(update.index(), 7);
        assert!(update.new_attributes.values.public());
    }
}
