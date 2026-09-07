use std::fmt::Write as _;

use librespot_protocol::playlist4_external::{
    Item, ListAttributes, MetaItem, SelectedListContent as RootList,
};
use percent_encoding::percent_decode_str;
use serde::Deserialize;

use crate::models;

pub const UNKNOWN: &str = "Unknown";
const IMAGE_CDN: &str = "https://i.scdn.co/image/";
const BLEND: &str = "blend";
const BY_SIZE: [&str; 4] = ["xlarge", "large", "default", "small"];
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const GROUP_START: &str = "spotify:start-group:";
const GROUP_END: &str = "spotify:end-group:";

pub fn image_url(file_id: &[u8]) -> Option<String> {
    if file_id.is_empty() {
        return None;
    }

    let hex = file_id.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    Some(format!("{IMAGE_CDN}{hex}"))
}

pub fn blend(attributes: &ListAttributes) -> bool {
    if attributes.format().to_ascii_lowercase().contains(BLEND) {
        return true;
    }

    attributes
        .format_attributes
        .iter()
        .any(|attribute| attribute.key().to_ascii_lowercase().starts_with(BLEND))
}

pub fn fetchable(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn playlist_cover(attributes: &ListAttributes) -> Option<String> {
    for target in BY_SIZE {
        if let Some(size) = attributes
            .picture_size
            .iter()
            .find(|size| size.target_name() == target)
            .filter(|size| fetchable(size.url()))
        {
            return Some(size.url().to_owned());
        }
    }

    attributes
        .picture_size
        .first()
        .map(|size| size.url())
        .filter(|url| fetchable(url))
        .map(str::to_owned)
        .or_else(|| image_url(attributes.picture()))
}

#[derive(Debug, Default, Deserialize)]
pub struct Named {
    pub display_name: Option<String>,
    pub name: Option<String>,
}

impl Named {
    pub fn label(&self) -> Option<&str> {
        self.display_name
            .as_deref()
            .or(self.name.as_deref())
            .filter(|label| !label.is_empty())
    }
}

pub fn playlist_from(id: &str, content: &RootList, username: &str) -> models::Playlist {
    let owner = match content.owner_username() {
        "" => UNKNOWN,
        owner => owner,
    };
    let name = match content.attributes.name() {
        "" => UNKNOWN,
        name => name,
    };

    models::Playlist {
        id: id.to_owned(),
        name: name.to_owned(),
        owner: owner.to_owned(),
        owner_id: content.owner_username().to_owned(),
        owned: owner == username,
        collaborative: content.attributes.collaborative(),
        blend: blend(&content.attributes),
        public: false,
        cover: playlist_cover(&content.attributes),
        track_count: content.length().max(0) as u32,
        modified_at: seconds(content.timestamp()),
    }
}

pub fn playlists_from(rootlist: &RootList) -> Vec<models::PlaylistEntry> {
    let contents = &rootlist.contents;
    let meta = &contents.meta_items;
    let mut open: Vec<models::PlaylistFolder> = Vec::new();
    let mut root = Vec::new();

    for (index, item) in contents.items.iter().enumerate() {
        let uri = item.uri();
        if let Some(group) = uri.strip_prefix(GROUP_START) {
            let (id, name) = group.split_once(':').unwrap_or((group, ""));
            open.push(models::PlaylistFolder {
                id: id.to_owned(),
                name: folder_name(name),
                entries: Vec::new(),
            });
            continue;
        }
        if let Some(id) = uri.strip_prefix(GROUP_END) {
            if let Some(depth) = open.iter().rposition(|folder| folder.id == id) {
                while open.len() > depth {
                    close(&mut open, &mut root);
                }
            }
            continue;
        }
        let Some(id) = uri.strip_prefix(PLAYLIST_PREFIX) else {
            continue;
        };
        let playlist = models::PlaylistEntry::Playlist(playlist_item(id, item, meta.get(index)));
        shelve(&mut open, &mut root, playlist);
    }
    while !open.is_empty() {
        close(&mut open, &mut root);
    }

    root
}

fn close(open: &mut Vec<models::PlaylistFolder>, root: &mut Vec<models::PlaylistEntry>) {
    let Some(folder) = open.pop() else {
        return;
    };
    shelve(open, root, models::PlaylistEntry::Folder(folder));
}

fn shelve(
    open: &mut [models::PlaylistFolder],
    root: &mut Vec<models::PlaylistEntry>,
    entry: models::PlaylistEntry,
) {
    match open.last_mut() {
        Some(folder) => folder.entries.push(entry),
        None => root.push(entry),
    }
}

fn folder_name(encoded: &str) -> String {
    let spaced = encoded.replace('+', " ");
    let name = percent_decode_str(&spaced).decode_utf8_lossy();
    match name.trim() {
        "" => UNKNOWN.to_owned(),
        name => name.to_owned(),
    }
}

fn playlist_item(id: &str, item: &Item, meta: Option<&MetaItem>) -> models::Playlist {
    let name = meta
        .map(|meta| meta.attributes.name())
        .filter(|name| !name.is_empty())
        .unwrap_or(UNKNOWN);
    let owner = meta
        .map(|meta| meta.owner_username())
        .filter(|owner| !owner.is_empty())
        .unwrap_or(UNKNOWN);

    models::Playlist {
        id: id.to_owned(),
        name: name.to_owned(),
        owner: owner.to_owned(),
        owner_id: meta
            .map(|meta| meta.owner_username())
            .unwrap_or_default()
            .to_owned(),
        owned: false,
        collaborative: meta.is_some_and(|meta| meta.attributes.collaborative()),
        blend: meta.is_some_and(|meta| blend(&meta.attributes)),
        public: item.attributes.public(),
        cover: meta.and_then(|meta| playlist_cover(&meta.attributes)),
        track_count: meta.map(|meta| meta.length()).unwrap_or_default().max(0) as u32,
        modified_at: None,
    }
}

pub fn seconds(millis: i64) -> Option<i64> {
    (millis > 0).then_some(millis / 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlaylistEntry;

    fn item(uri: &str) -> Item {
        let mut item = Item::new();
        item.set_uri(uri.to_owned());
        item
    }

    fn named(name: &str) -> MetaItem {
        let mut meta = MetaItem::new();
        meta.attributes
            .mut_or_insert_default()
            .set_name(name.to_owned());
        meta.set_owner_username("owner".to_owned());
        meta.set_length(3);
        meta
    }

    fn rootlist(uris: &[&str]) -> RootList {
        let mut content = RootList::new();
        let contents = content.contents.mut_or_insert_default();
        contents.set_pos(0);
        contents.set_truncated(false);
        for uri in uris {
            contents.items.push(item(uri));
            contents
                .meta_items
                .push(match uri.strip_prefix(PLAYLIST_PREFIX) {
                    Some(id) => named(&id.to_uppercase()),
                    None => MetaItem::new(),
                });
        }
        content
    }

    fn outline(entries: &[PlaylistEntry]) -> Vec<String> {
        let mut lines = Vec::new();
        for entry in entries {
            match entry {
                PlaylistEntry::Playlist(playlist) => {
                    lines.push(format!("{}={}", playlist.id, playlist.name))
                }
                PlaylistEntry::Folder(folder) => {
                    lines.push(format!("[{}]", folder.name));
                    lines.extend(
                        outline(&folder.entries)
                            .into_iter()
                            .map(|line| format!("  {line}")),
                    );
                }
            }
        }
        lines
    }

    #[test]
    fn a_rootlist_without_groups_stays_flat() {
        let entries = playlists_from(&rootlist(&["spotify:playlist:a", "spotify:playlist:b"]));

        assert_eq!(outline(&entries), ["a=A", "b=B"]);
    }

    #[test]
    fn group_markers_become_folders_in_listing_order() {
        let entries = playlists_from(&rootlist(&[
            "spotify:playlist:a",
            "spotify:start-group:f1:Road+trip",
            "spotify:playlist:b",
            "spotify:playlist:c",
            "spotify:end-group:f1",
            "spotify:playlist:d",
        ]));

        assert_eq!(
            outline(&entries),
            ["a=A", "[Road trip]", "  b=B", "  c=C", "d=D"]
        );
    }

    #[test]
    fn folders_nest() {
        let entries = playlists_from(&rootlist(&[
            "spotify:start-group:outer:Outer",
            "spotify:playlist:a",
            "spotify:start-group:inner:Inner",
            "spotify:playlist:b",
            "spotify:end-group:inner",
            "spotify:playlist:c",
            "spotify:end-group:outer",
        ]));

        assert_eq!(
            outline(&entries),
            ["[Outer]", "  a=A", "  [Inner]", "    b=B", "  c=C"]
        );
    }

    #[test]
    fn playlist_metadata_keeps_its_index_past_a_marker() {
        let entries = playlists_from(&rootlist(&[
            "spotify:start-group:f1:Folder",
            "spotify:playlist:a",
            "spotify:end-group:f1",
        ]));
        let playlists = PlaylistEntry::playlists(&entries);

        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].name, "A");
        assert_eq!(playlists[0].owner, "owner");
        assert_eq!(playlists[0].track_count, 3);
    }

    #[test]
    fn folder_names_are_form_decoded() {
        assert_eq!(folder_name("Road+trip"), "Road trip");
        assert_eq!(folder_name("Rock%2FMetal"), "Rock/Metal");
        assert_eq!(folder_name("A%2BB"), "A+B");
        assert_eq!(
            folder_name("%D0%9F%D0%BB%D0%B5%D0%B9%D0%BB%D0%B8%D1%81%D1%82%D1%8B"),
            "Плейлисты"
        );
        assert_eq!(folder_name(""), UNKNOWN);
    }

    #[test]
    fn an_unterminated_group_closes_at_the_end() {
        let entries = playlists_from(&rootlist(&[
            "spotify:start-group:f1:Open",
            "spotify:playlist:a",
            "spotify:start-group:f2:Inner",
            "spotify:playlist:b",
        ]));

        assert_eq!(
            outline(&entries),
            ["[Open]", "  a=A", "  [Inner]", "    b=B"]
        );
    }

    #[test]
    fn a_stray_end_marker_is_ignored_and_a_skipped_one_closes_through() {
        let entries = playlists_from(&rootlist(&[
            "spotify:end-group:nobody",
            "spotify:start-group:outer:Outer",
            "spotify:start-group:inner:Inner",
            "spotify:playlist:a",
            "spotify:end-group:outer",
            "spotify:playlist:b",
        ]));

        assert_eq!(
            outline(&entries),
            ["[Outer]", "  [Inner]", "    a=A", "b=B"]
        );
    }

    #[test]
    fn other_uris_are_dropped() {
        let entries = playlists_from(&rootlist(&[
            "spotify:playlist:a",
            "spotify:show:podcast",
            "spotify:user:someone:collection",
        ]));

        assert_eq!(outline(&entries), ["a=A"]);
    }
}
