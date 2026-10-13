use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use http::Method;
use librespot_core::{Session, SpotifyId};
use librespot_protocol::playlist4_external::{
    Add, ChangeInfo, CreateListReply, Delta, Item, ItemAttributes, ItemAttributesPartialState,
    ListAttributes, ListAttributesPartialState, ListChanges, Op, Rem, SelectedListContent,
    SourceInfo, UpdateItemAttributes, UpdateListAttributes, op::Kind, source_info::Client,
};
use protobuf::{Message as _, MessageField};
use tokio::task::JoinSet;

use crate::spotify::pace::paced;
use crate::spotify::{collection, profiles, wire};
use crate::{Contributor, GenreItem, GenreSection, Playlist, PlaylistDetail, Track, escape};

const TRACK_PREFIX: &str = "spotify:track:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const ROOTLIST_LIMIT: usize = 10000;
const COVER_SLACK: usize = 4;
const MODIFIED_LIMIT: usize = 300;

pub async fn create(session: &Session, name: &str) -> Result<String> {
    let body = changes(None, rename_op(name));
    let reply = post(session, "/playlist/v2/playlist", &body)
        .await
        .context("cannot create the playlist")?;
    let reply =
        CreateListReply::parse_from_bytes(&reply).context("cannot decode the created playlist")?;

    let id = reply
        .uri()
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
    let rootlist = fetch_rootlist(session).await?;
    let body = changes(rootlist.revision.as_deref(), add_op(&uri));
    rootlist_edit(session, &body)
        .await
        .context("cannot add the playlist to the library")?;
    Ok(())
}

pub async fn rename(session: &Session, playlist_id: &str, name: &str) -> Result<()> {
    let body = changes(None, rename_op(name));
    edit(session, playlist_id, &body)
        .await
        .context("cannot rename the playlist")?;
    Ok(())
}

pub async fn delete(session: &Session, playlist_id: &str) -> Result<()> {
    drop_from_rootlist(session, playlist_id)
        .await
        .context("cannot remove the deleted playlist from the library")?;

    let body = changes(None, delete_op());
    edit(session, playlist_id, &body)
        .await
        .context("cannot delete the playlist")?;
    Ok(())
}

pub async fn remove_from_library(session: &Session, playlist_id: &str) -> Result<()> {
    drop_from_rootlist(session, playlist_id)
        .await
        .context("cannot remove the playlist from the library")?;
    Ok(())
}

async fn drop_from_rootlist(session: &Session, playlist_id: &str) -> Result<()> {
    let uri = format!("{PLAYLIST_PREFIX}{playlist_id}");
    let (rootlist, index) = rootlist(session, &uri).await?;

    let body = changes(rootlist.revision.as_deref(), remove_op(&uri, index));
    rootlist_edit(session, &body).await?;
    Ok(())
}

pub async fn set_public(session: &Session, playlist_id: &str, public: bool) -> Result<()> {
    let uri = format!("{PLAYLIST_PREFIX}{playlist_id}");
    let (rootlist, index) = rootlist(session, &uri).await?;

    let body = changes(rootlist.revision.as_deref(), visibility_op(index, public));
    rootlist_edit(session, &body)
        .await
        .context("cannot change playlist visibility")?;
    Ok(())
}

pub async fn add_track(session: &Session, playlist_id: &str, track_id: &str) -> Result<()> {
    let uri = format!("{TRACK_PREFIX}{track_id}");
    let content = snapshot(session, playlist_id).await?;
    let body = changes(content.revision.as_deref(), add_op(&uri));

    edit(session, playlist_id, &body)
        .await
        .context("cannot add the track to the playlist")?;
    Ok(())
}

pub async fn remove_track(session: &Session, playlist_id: &str, track_id: &str) -> Result<()> {
    let uri = format!("{TRACK_PREFIX}{track_id}");
    let content = snapshot(session, playlist_id).await?;
    let index =
        position(&content, &uri).ok_or_else(|| anyhow!("cannot find the track in the playlist"))?;

    let body = changes(content.revision.as_deref(), remove_op(&uri, index));
    edit(session, playlist_id, &body)
        .await
        .context("cannot remove the track from the playlist")?;
    Ok(())
}

pub async fn playlist(session: &Session, playlist_id: &str) -> Result<PlaylistDetail> {
    let content = snapshot(session, playlist_id).await?;
    let playlist = wire::playlist_from(playlist_id, &content, &session.username());
    let mut tracks = tracks_from(session, &content).await?;
    credit(session, &playlist, &mut tracks).await;

    Ok(PlaylistDetail {
        playlist,
        tracks,
        continuation: None,
    })
}

async fn credit(session: &Session, playlist: &Playlist, tracks: &mut [Track]) {
    let ids: HashSet<String> = tracks
        .iter()
        .filter_map(|track| track.added_by.as_ref())
        .map(|added| added.id.clone())
        .collect();

    if ids.len() < 2 && !playlist.blend {
        for track in tracks.iter_mut() {
            track.added_by = None;
        }
        return;
    }

    let known = profiles::contributors(session, ids).await;
    for track in tracks {
        let Some(added) = track.added_by.as_mut() else {
            continue;
        };
        if let Some(found) = known.get(&added.id) {
            *added = Arc::new(found.clone());
        }
    }
}

pub async fn header(session: &Session, playlist_id: &str) -> Result<Playlist> {
    let content = snapshot(session, playlist_id).await?;

    Ok(wire::playlist_from(
        playlist_id,
        &content,
        &session.username(),
    ))
}

pub async fn name_blanks(session: &Session, sections: &mut Vec<GenreSection>) {
    let blanks: Vec<String> = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .filter_map(|item| match item {
            GenreItem::Playlist(playlist) if playlist.name.is_empty() => Some(playlist.id.clone()),
            _ => None,
        })
        .collect();
    if blanks.is_empty() {
        return;
    }

    let mut pending = JoinSet::new();
    for id in blanks {
        let session = session.clone();
        pending.spawn(async move { (id.clone(), header(&session, &id).await.ok()) });
    }

    let mut found = HashMap::new();
    while let Some(joined) = pending.join_next().await {
        if let Ok((id, Some(playlist))) = joined {
            found.insert(id, playlist);
        }
    }

    for section in sections.iter_mut() {
        for item in &mut section.items {
            let GenreItem::Playlist(playlist) = item else {
                continue;
            };
            if !playlist.name.is_empty() {
                continue;
            }
            let Some(named) = found.get(&playlist.id) else {
                continue;
            };
            playlist.name = named.name.clone();
            playlist.cover = named.cover.clone();
        }
        section.items.retain(|item| match item {
            GenreItem::Playlist(playlist) => !playlist.name.is_empty(),
            _ => true,
        });
    }
    sections.retain(|section| !section.items.is_empty());
}

pub async fn playlist_tracks(session: &Session, playlist_id: &str) -> Result<Vec<Track>> {
    let content = snapshot(session, playlist_id).await?;
    tracks_from(session, &content).await
}

pub async fn covers(session: &Session, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
    let content = snapshot(session, playlist_id).await?;
    let uris: Vec<String> = content
        .contents
        .items
        .iter()
        .map(|item| item.uri())
        .filter(|uri| uri.starts_with(TRACK_PREFIX))
        .take(wanted * COVER_SLACK)
        .map(str::to_owned)
        .collect();
    if uris.is_empty() {
        return Ok(Vec::new());
    }

    let known = collection::metadata(session, &uris).await?;
    let tracks: Vec<Track> = uris
        .iter()
        .filter_map(|uri| known.get(uri).cloned())
        .collect();

    Ok(crate::distinct_covers(&tracks, wanted))
}

async fn tracks_from(session: &Session, content: &SelectedListContent) -> Result<Vec<Track>> {
    let added: Vec<(String, Option<i64>, Option<String>)> = content
        .contents
        .items
        .iter()
        .filter(|item| item.uri().starts_with(TRACK_PREFIX))
        .map(|item| {
            (
                item.uri().to_owned(),
                wire::seconds(item.attributes.timestamp()),
                contributor(item),
            )
        })
        .collect();
    if added.is_empty() {
        return Ok(Vec::new());
    }

    let uris: Vec<String> = added.iter().map(|(uri, _, _)| uri.clone()).collect();
    let known = collection::metadata(session, &uris).await?;
    Ok(added
        .iter()
        .filter_map(|(uri, added_at, added_by)| {
            let mut track = known.get(uri).cloned()?;
            track.added_at = *added_at;
            track.added_by = added_by.clone().map(Contributor::unnamed).map(Arc::new);
            Some(track)
        })
        .collect())
}

fn contributor(item: &Item) -> Option<String> {
    let added_by = profiles::username(item.attributes.added_by());
    (!added_by.is_empty()).then_some(added_by)
}

pub async fn modified(session: &Session, ids: Vec<String>) -> HashMap<String, i64> {
    let wanted = ids.len().min(MODIFIED_LIMIT);
    if wanted < ids.len() {
        log::debug!(
            "playlists: reading the modification date of {wanted} of {} playlists",
            ids.len()
        );
    }

    let mut pending = JoinSet::new();
    for id in ids.into_iter().take(wanted) {
        let session = session.clone();
        pending.spawn(async move {
            let stamp = stamp(&session, &id).await;
            (id, stamp)
        });
    }

    let mut stamps = HashMap::new();
    while let Some(joined) = pending.join_next().await {
        if let Ok((id, Some(stamp))) = joined {
            stamps.insert(id, stamp);
        }
    }
    stamps
}

async fn stamp(session: &Session, playlist_id: &str) -> Option<i64> {
    let endpoint = format!(
        "/playlist/v2/playlist/{}?from=0&length=0",
        escape::component(playlist_id)
    );
    let body = session
        .spclient()
        .request(&Method::GET, &endpoint, None, None)
        .await
        .inspect_err(|error| {
            log::debug!("playlists: cannot read the metadata of {playlist_id}: {error}")
        })
        .ok()?;

    let content = SelectedListContent::parse_from_bytes(&body).ok()?;
    wire::seconds(content.timestamp())
}

async fn snapshot(session: &Session, playlist_id: &str) -> Result<SelectedListContent> {
    let id = SpotifyId::from_base62(playlist_id).context("cannot read the playlist id")?;
    let body = paced(|| session.spclient().get_playlist(&id))
        .await
        .context("cannot read the playlist")?;

    SelectedListContent::parse_from_bytes(&body).context("cannot decode the playlist")
}

async fn edit(session: &Session, playlist_id: &str, body: &ListChanges) -> Result<Vec<u8>> {
    let endpoint = format!(
        "/playlist/v2/playlist/{}/changes",
        escape::component(playlist_id)
    );
    post(session, &endpoint, body).await
}

/// Reads one page of the signed-in account's rootlist. This stands in for librespot's
/// `get_rootlist`, which pastes the username into the path unescaped, so an account whose name
/// holds an ö gets a 400 back.
pub async fn rootlist_page(session: &Session, from: usize, length: usize) -> Result<Vec<u8>> {
    let endpoint = format!(
        "/playlist/v2/user/{}/rootlist?decorate=revision,attributes,length,owner,capabilities,status_code&from={from}&length={length}",
        escape::component(&session.username())
    );
    let reply = paced(|| {
        session
            .spclient()
            .request(&Method::GET, &endpoint, None, None)
    })
    .await?;
    Ok(reply.to_vec())
}

async fn fetch_rootlist(session: &Session) -> Result<SelectedListContent> {
    let body = rootlist_page(session, 0, ROOTLIST_LIMIT)
        .await
        .context("cannot read the rootlist")?;
    SelectedListContent::parse_from_bytes(&body).context("cannot decode the rootlist")
}

async fn rootlist(session: &Session, uri: &str) -> Result<(SelectedListContent, i32)> {
    let rootlist = fetch_rootlist(session).await?;
    let index = position(&rootlist, uri)
        .ok_or_else(|| anyhow!("cannot find the playlist in the rootlist"))?;

    Ok((rootlist, index))
}

async fn rootlist_edit(session: &Session, body: &ListChanges) -> Result<Vec<u8>> {
    let endpoint = format!(
        "/playlist/v2/user/{}/rootlist/changes",
        escape::component(&session.username())
    );
    post(session, &endpoint, body).await
}

async fn post(session: &Session, endpoint: &str, body: &ListChanges) -> Result<Vec<u8>> {
    let reply = paced(|| {
        session
            .spclient()
            .request_with_protobuf(&Method::POST, endpoint, None, body)
    })
    .await?;
    Ok(reply.to_vec())
}

fn changes(revision: Option<&[u8]>, op: Op) -> ListChanges {
    let mut source = SourceInfo::new();
    source.set_client(Client::CLIENT);

    let mut info = ChangeInfo::new();
    info.source = MessageField::some(source);

    let mut delta = Delta::new();
    delta.ops.push(op);
    delta.info = MessageField::some(info);

    let mut changes = ListChanges::new();
    changes.base_revision = revision.map(<[u8]>::to_vec);
    changes.deltas.push(delta);
    changes.want_resulting_revisions = Some(true);
    changes
}

fn item(uri: &str) -> Item {
    let mut item = Item::new();
    item.set_uri(uri.to_owned());
    item
}

fn add_op(uri: &str) -> Op {
    let mut add = Add::new();
    add.items.push(item(uri));
    add.set_add_last(true);

    let mut op = Op::new();
    op.set_kind(Kind::ADD);
    op.add = MessageField::some(add);
    op
}

fn remove_op(uri: &str, index: i32) -> Op {
    let mut rem = Rem::new();
    rem.set_from_index(index);
    rem.set_length(1);
    rem.items.push(item(uri));
    rem.set_items_as_key(true);

    let mut op = Op::new();
    op.set_kind(Kind::REM);
    op.rem = MessageField::some(rem);
    op
}

fn rename_op(name: &str) -> Op {
    let mut values = ListAttributes::new();
    values.set_name(name.to_owned());
    list_attributes_op(values)
}

fn delete_op() -> Op {
    let mut values = ListAttributes::new();
    values.set_deleted_by_owner(true);
    list_attributes_op(values)
}

fn list_attributes_op(values: ListAttributes) -> Op {
    let mut state = ListAttributesPartialState::new();
    state.values = MessageField::some(values);

    let mut update = UpdateListAttributes::new();
    update.new_attributes = MessageField::some(state);

    let mut op = Op::new();
    op.set_kind(Kind::UPDATE_LIST_ATTRIBUTES);
    op.update_list_attributes = MessageField::some(update);
    op
}

fn visibility_op(index: i32, public: bool) -> Op {
    let mut values = ItemAttributes::new();
    values.set_public(public);

    let mut state = ItemAttributesPartialState::new();
    state.values = MessageField::some(values);

    let mut update = UpdateItemAttributes::new();
    update.set_index(index);
    update.new_attributes = MessageField::some(state);

    let mut op = Op::new();
    op.set_kind(Kind::UPDATE_ITEM_ATTRIBUTES);
    op.update_item_attributes = MessageField::some(update);
    op
}

fn position(content: &SelectedListContent, uri: &str) -> Option<i32> {
    let contents = &content.contents;
    let offset = contents.items.iter().position(|item| item.uri() == uri)? as i32;
    Some(contents.pos() + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(uris: &[&str], pos: i32) -> SelectedListContent {
        let mut content = SelectedListContent::new();
        let contents = content.contents.mut_or_insert_default();
        contents.set_pos(pos);
        contents.set_truncated(false);
        for uri in uris {
            contents.items.push(item(uri));
        }
        content
    }

    #[test]
    fn removal_carries_the_index_and_the_revision() {
        let content = listing(&["spotify:track:a", "spotify:track:b"], 0);
        let index = position(&content, "spotify:track:b").expect("index");
        let body = changes(Some(b"rev"), remove_op("spotify:track:b", index));

        assert_eq!(body.base_revision(), b"rev");
        let rem = &body.deltas[0].ops[0].rem;
        assert_eq!(rem.from_index(), 1);
        assert_eq!(rem.length(), 1);
        assert!(rem.items_as_key());
        assert_eq!(rem.items[0].uri(), "spotify:track:b");
    }

    #[test]
    fn position_is_absolute_within_a_page() {
        let content = listing(&["spotify:track:a", "spotify:track:b"], 100);

        assert_eq!(position(&content, "spotify:track:a"), Some(100));
        assert_eq!(position(&content, "spotify:track:b"), Some(101));
        assert_eq!(position(&content, "spotify:track:c"), None);
    }

    #[test]
    fn addition_appends_without_an_index() {
        let body = changes(None, add_op("spotify:track:a"));

        assert!(body.base_revision.is_none());
        let add = &body.deltas[0].ops[0].add;
        assert!(add.add_last());
        assert_eq!(add.items[0].uri(), "spotify:track:a");
    }

    #[test]
    fn attribute_ops_set_one_value_each() {
        let renamed = rename_op("Road trip");
        let renamed = &renamed.update_list_attributes.new_attributes.values;
        assert_eq!(renamed.name(), "Road trip");
        assert!(!renamed.deleted_by_owner());

        let deleted = delete_op();
        let deleted = &deleted.update_list_attributes.new_attributes.values;
        assert!(deleted.deleted_by_owner());
        assert_eq!(deleted.name(), "");
    }

    #[test]
    fn leaving_a_playlist_rems_the_rootlist_item() {
        let rootlist = listing(&["spotify:playlist:a", "spotify:playlist:b"], 0);
        let index = position(&rootlist, "spotify:playlist:b").expect("index");
        let op = remove_op("spotify:playlist:b", index);

        assert_eq!(op.kind(), Kind::REM);
        assert_eq!(op.rem.from_index(), 1);
        assert_eq!(op.rem.length(), 1);
        assert_eq!(op.rem.items[0].uri(), "spotify:playlist:b");
        assert!(op.update_list_attributes.is_none());
    }

    #[test]
    fn a_name_survives_the_wire_in_any_script() {
        for name in ["Playlist", "Плейлист", "Плейліст", "日本語", "hi 🎧"] {
            let body = changes(None, rename_op(name));
            let bytes = body.write_to_bytes().expect("encode");
            let decoded = ListChanges::parse_from_bytes(&bytes).expect("decode");
            let values = &decoded.deltas[0].ops[0]
                .update_list_attributes
                .new_attributes
                .values;

            assert_eq!(values.name(), name);
        }
    }

    #[test]
    fn a_created_playlist_is_appended_to_the_rootlist() {
        let mut rootlist = listing(&["spotify:playlist:a"], 0);
        rootlist.revision = Some(vec![9, 9, 9]);

        let body = changes(rootlist.revision.as_deref(), add_op("spotify:playlist:new"));
        let op = &body.deltas[0].ops[0];

        assert_eq!(body.base_revision, Some(vec![9, 9, 9]));
        assert_eq!(op.kind(), Kind::ADD);
        assert!(op.add.add_last());
        assert_eq!(op.add.items[0].uri(), "spotify:playlist:new");
    }

    #[test]
    fn visibility_targets_the_rootlist_index() {
        let op = visibility_op(3, true);
        let update = &op.update_item_attributes;

        assert_eq!(op.kind(), Kind::UPDATE_ITEM_ATTRIBUTES);
        assert_eq!(update.index(), 3);
        assert!(update.item.is_none());
        assert!(update.new_attributes.values.public());
    }

    #[test]
    fn every_op_encodes() {
        for op in [
            add_op("spotify:track:a"),
            remove_op("spotify:track:a", 0),
            rename_op("name"),
            delete_op(),
            visibility_op(0, false),
        ] {
            let body = changes(Some(b"rev"), op);
            body.write_to_bytes().expect("encodes");
        }
    }
}
