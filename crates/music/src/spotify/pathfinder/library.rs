use std::collections::HashSet;

use crate::spotify::session::Session;
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use super::query;
use crate::{PinOutcome, PinTarget, PinTargetKind};

const PAGE_SIZE: usize = 100;

#[derive(Deserialize)]
struct Data {
    me: Me,
}

#[derive(Deserialize)]
struct Me {
    #[serde(rename = "libraryV3")]
    library: Page,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    items: Vec<Value>,
    total_count: usize,
    paging_info: Paging,
}

#[derive(Deserialize)]
struct Paging {
    offset: usize,
}

/// Every row of Your Library, most recently played first, each saying whether it is pinned.
pub(crate) async fn library(session: &Session) -> Result<Vec<PinTarget>> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut offset = 0;
    loop {
        let data: Data = query(
            session,
            "libraryV3",
            json!({
                "filters": [], "order": "Recents", "textFilter": "", "features": ["LIKED_SONGS"],
                "limit": PAGE_SIZE, "offset": offset, "flatten": false, "expandedFolders": [],
                "folderUri": null, "includeFoldersWhenFlattening": true
            }),
        )
        .await?;
        let page = data.me.library;
        ensure!(
            page.paging_info.offset == offset,
            "Spotify returned an unexpected library page"
        );
        let count = page.items.len();
        result.extend(
            page.items
                .iter()
                .filter_map(item)
                .filter(|item| seen.insert(item.uri.clone())),
        );
        offset = offset
            .checked_add(count)
            .context("library offset overflow")?;
        if count == 0 || offset >= page.total_count {
            break;
        }
    }
    Ok(result)
}

pub(crate) async fn set_pinned(session: &Session, uri: &str, pinned: bool) -> Result<PinOutcome> {
    let operation = if pinned {
        "pinLibraryItem"
    } else {
        "unpinLibraryItem"
    };
    let data: Value = query(session, operation, json!({"pinnableItemUri": uri})).await?;
    if pinned {
        pin_result(&data)
    } else {
        Ok(PinOutcome::Updated)
    }
}

fn pin_result(data: &Value) -> Result<PinOutcome> {
    match data
        .pointer("/pinItemInLibrary/pinResult")
        .and_then(Value::as_str)
    {
        Some("SUCCESSFUL") => Ok(PinOutcome::Updated),
        Some("FAILED_ITEM_LIMIT_REACHED") => Ok(PinOutcome::LimitReached),
        Some(reason) => anyhow::bail!("Spotify rejected library pin: {reason}"),
        None => anyhow::bail!("Spotify did not confirm the library pin"),
    }
}

fn string<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)?
        .as_str()
        .filter(|text| !text.is_empty())
}

fn cover(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)?
        .as_array()?
        .iter()
        .filter_map(|source| {
            let url = source.get("url")?.as_str()?;
            (url.starts_with("https://") || url.starts_with("http://")).then_some((
                source.get("width").and_then(Value::as_u64).unwrap_or(300),
                url,
            ))
        })
        .min_by_key(|(width, _)| width.abs_diff(64))
        .map(|(_, url)| url.to_owned())
}

fn item(row: &Value) -> Option<PinTarget> {
    let data = row.pointer("/item/data")?;
    let typename = string(data, "/__typename")?;
    let uri = string(data, "/uri").or_else(|| string(row, "/item/_uri"))?;
    let (kind, name, subtitle, artwork) = match typename {
        "Playlist" => (
            PinTargetKind::Playlist,
            string(data, "/name")?,
            string(data, "/ownerV2/data/name")
                .unwrap_or_default()
                .to_owned(),
            cover(data, "/images/items/0/sources"),
        ),
        "Album" => (
            PinTargetKind::Album,
            string(data, "/name")?,
            data.pointer("/artists/items")?
                .as_array()?
                .iter()
                .filter_map(|artist| string(artist, "/profile/name"))
                .collect::<Vec<_>>()
                .join(", "),
            cover(data, "/coverArt/sources"),
        ),
        "Artist" => (
            PinTargetKind::Artist,
            string(data, "/profile/name")?,
            String::new(),
            cover(data, "/visuals/avatarImage/sources"),
        ),
        "PseudoPlaylist" if uri == "spotify:collection:tracks" => (
            PinTargetKind::LikedSongs,
            string(data, "/name")?,
            String::new(),
            cover(data, "/image/sources"),
        ),
        "Audiobook" => (
            PinTargetKind::Audiobook,
            string(data, "/name")?,
            data.get("authorsV2")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|author| string(author, "/name"))
                .collect::<Vec<_>>()
                .join(", "),
            cover(data, "/coverArt/sources"),
        ),
        "Podcast" => (
            PinTargetKind::Show,
            string(data, "/name")?,
            string(data, "/publisher/name")
                .unwrap_or_default()
                .to_owned(),
            cover(data, "/coverArt/sources"),
        ),
        "Folder" => (
            PinTargetKind::Folder,
            string(data, "/name")?,
            String::new(),
            None,
        ),
        _ => return None,
    };
    Some(PinTarget {
        uri: uri.to_owned(),
        name: name.to_owned(),
        subtitle,
        cover: artwork,
        kind,
        pinned: row.get("pinned").and_then(Value::as_bool).unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_response_requires_confirmation_and_reports_the_server_limit() {
        assert_eq!(
            pin_result(&json!({"pinItemInLibrary":{"pinResult":"SUCCESSFUL"}})).unwrap(),
            PinOutcome::Updated
        );
        assert_eq!(
            pin_result(
                &json!({"pinItemInLibrary":{"pinResult":"FAILED_ITEM_LIMIT_REACHED","pinLimit":4}})
            )
            .unwrap(),
            PinOutcome::LimitReached
        );
        for reason in [
            "FAILED_ITEM_IN_FOLDER",
            "FAILED_ITEM_NOT_SUPPORTED",
            "FAILED_NOT_IN_YOUR_LIBRARY",
            "NEW_FAILURE",
        ] {
            assert!(pin_result(&json!({"pinItemInLibrary":{"pinResult":reason}})).is_err());
        }
        assert!(pin_result(&json!({})).is_err());
        assert!(pin_result(&json!({"pinItemInLibrary":null})).is_err());
    }

    #[test]
    fn unavailable_items_do_not_become_unknown_playlists() {
        assert!(
            item(
                &json!({"item":{"_uri":"spotify:playlist:gone","data":{"__typename":"NotFound"}}})
            )
            .is_none()
        );
    }

    #[test]
    fn liked_songs_keeps_its_pin_and_cover() {
        let row = item(&json!({"pinned":true,"item":{"data":{"__typename":"PseudoPlaylist",
            "uri":"spotify:collection:tracks","name":"Liked Songs","image":{"sources":[
                {"width":640,"url":"https://example.com/large"},{"width":64,"url":"https://example.com/small"}]}}}})).unwrap();
        assert_eq!(row.kind, PinTargetKind::LikedSongs);
        assert!(row.pinned);
        assert_eq!(row.cover.as_deref(), Some("https://example.com/small"));
    }

    #[test]
    fn artists_and_books_keep_their_actual_types() {
        let artist = item(&json!({"item":{"_uri":"spotify:artist:a","data":{"__typename":"Artist","profile":{"name":"Artist"}}}})).unwrap();
        let book = item(&json!({"item":{"_uri":"spotify:show:b","data":{"__typename":"Audiobook","name":"Book","authorsV2":[{"name":"Author"}]}}})).unwrap();
        assert_eq!(artist.kind, PinTargetKind::Artist);
        assert_eq!(book.kind, PinTargetKind::Audiobook);
        assert_eq!(book.subtitle, "Author");
    }
}
