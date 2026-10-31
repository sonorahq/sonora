//! Conversions from Yandex Music's wire shapes to the crate's models. The mobile api answers in
//! camelCase, spells ids as numbers or strings depending on the endpoint, and leaves out
//! whatever it has nothing for, so every read goes through tolerant helpers rather than
//! derived structs.

use std::time::Duration;

use serde_json::Value;

use crate::{Album, ArtistRef, Playlist, ReleaseType, SavedArtist, Track, UserProfile};

/// The size every cover is first handed out at. [`resize`] swaps it for a bigger one.
const COVER: &str = "400x400";
/// The size an artist page header and a fullscreen cover ask for.
pub const LARGE: &str = "1000x1000";
/// The original upload.
pub const FULL: &str = "orig";
/// Where every picture of the catalogue is served from.
const AVATARS: &str = "avatars.yandex.net";

/// An id, spelled as a number or as a string. Empty and zero ids are none at all.
pub fn id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .filter(|id| !id.is_empty() && id != "0")
}

/// The first string under any of `keys`.
pub fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(key)?.as_str())
        .filter(|text| !text.is_empty())
}

/// The first number under any of `keys`, spelled as a number or as a string.
pub fn number(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let field = value.get(key)?;
        field
            .as_u64()
            .or_else(|| field.as_str()?.trim().parse().ok())
    })
}

/// The array under `key`, or nothing when there is none.
pub fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// An ISO 8601 time as unix seconds.
fn when(value: &Value, keys: &[&str]) -> Option<i64> {
    crate::iso_8601_to_epoch(text(value, keys))
}

/// The `https` url of a picture the api names by a `%%` template such as
/// `avatars.yandex.net/get-music-content/…/%%`, at `size`.
pub fn picture(template: Option<&str>, size: &str) -> Option<String> {
    let template = template?.trim();
    if template.is_empty() {
        return None;
    }
    let template = template
        .strip_prefix("https://")
        .or_else(|| template.strip_prefix("//"))
        .unwrap_or(template);
    Some(format!("https://{}", template.replace("%%", size)))
}

/// A cover this module handed out, at `size` instead. Only a picture on the avatars host is
/// sized by its last path segment, so anything else comes back as `None`.
pub fn resize(cover: &str, size: &str) -> Option<String> {
    let rest = cover.strip_prefix("https://")?;
    if !rest.starts_with(AVATARS) {
        return None;
    }
    let (head, _) = cover.rsplit_once('/')?;
    Some(format!("{head}/{size}"))
}

/// The signed-in account from the `account` of `account/status`.
pub fn profile(status: &Value) -> UserProfile {
    let account = &status["account"];
    let name = text(account, &["displayName", "fullName", "login"])
        .unwrap_or("Yandex Music")
        .to_owned();
    UserProfile {
        id: id(&account["uid"]).unwrap_or_default(),
        display_name: name,
        avatar: None,
    }
}

/// The name with the version Yandex keeps apart, such as `Song (Remastered 2011)`.
fn titled(value: &Value) -> String {
    let title = text(value, &["title", "name"]).unwrap_or_default();
    match text(value, &["version"]) {
        Some(version) => format!("{title} ({version})"),
        None => title.to_owned(),
    }
}

fn artists_of(value: &Value) -> (String, Vec<ArtistRef>) {
    let refs: Vec<ArtistRef> = list(value, "artists")
        .iter()
        .filter_map(|artist| {
            Some(ArtistRef {
                name: text(artist, &["name"])?.to_owned(),
                // a various-artists entry has an id that leads to no page
                id: (artist["various"] != true)
                    .then(|| id(&artist["id"]))
                    .flatten(),
            })
        })
        .collect();
    let names = refs
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    (names, refs)
}

fn cover(value: &Value) -> Option<String> {
    picture(text(value, &["coverUri", "ogImage"]), COVER)
}

/// Whether a release or a track says it is explicit.
fn explicit(value: &Value) -> bool {
    value["explicit"] == true || text(value, &["contentWarning"]) == Some("explicit")
}

/// A catalogue track. An unavailable one still comes back, marked unplayable.
pub fn track(value: &Value) -> Option<Track> {
    let id = id(&value["id"])?;
    let (artists, artist_refs) = artists_of(value);
    let album = list(value, "albums").first();
    let position = album.map(|album| &album["trackPosition"]);
    Some(Track {
        id: Some(id),
        name: titled(value),
        playable: value["available"] != false,
        artists,
        artist_refs,
        album: album
            .and_then(|album| text(album, &["title"]))
            .unwrap_or_default()
            .to_owned(),
        album_id: album.and_then(|album| self::id(&album["id"])),
        cover: cover(value).or_else(|| album.and_then(cover)),
        duration: Duration::from_millis(number(value, &["durationMs"]).unwrap_or_default()),
        added_at: None,
        added_by: None,
        playcount: None,
        popularity: 0,
        explicit: explicit(value),
        track_number: position
            .and_then(|position| number(position, &["index"]))
            .unwrap_or_default() as u32,
        disc_number: position
            .and_then(|position| number(position, &["volume"]))
            .unwrap_or(1) as u32,
        tags: album
            .and_then(|album| text(album, &["genre"]))
            .map(|genre| vec![genre.to_owned()])
            .unwrap_or_default(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

/// The tracks of a list, skipping the entries that are not one.
pub fn tracks(values: &[Value]) -> Vec<Track> {
    values.iter().filter_map(track).collect()
}

/// A playlist entry, which wraps the track with when it was added, or is the track itself.
pub fn entry(value: &Value) -> Option<Track> {
    let mut track = track(value.get("track").unwrap_or(value))?;
    track.added_at = when(value, &["timestamp", "addedAt"]);
    Some(track)
}

fn release_type(value: &Value) -> ReleaseType {
    match text(value, &["type"]).or_else(|| text(value, &["metaType"])) {
        Some("single") => ReleaseType::Single,
        Some("compilation") => ReleaseType::Compilation,
        Some("podcast") => ReleaseType::Podcast,
        Some("audiobook") => ReleaseType::Audiobook,
        _ => ReleaseType::Album,
    }
}

/// A label is named in an object on some endpoints and as a bare string on others.
fn labels(value: &Value) -> String {
    list(value, "labels")
        .iter()
        .filter_map(|label| label.as_str().or_else(|| text(label, &["name"])))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn album(value: &Value) -> Option<Album> {
    let id = id(&value["id"])?;
    let (artists, artist_refs) = artists_of(value);
    let release_date = text(value, &["releaseDate"])
        .map(|date| date.get(..10).unwrap_or(date).to_owned())
        .unwrap_or_default();
    Some(Album {
        id,
        name: titled(value),
        artists,
        artist_refs,
        cover: cover(value),
        cover_large: picture(text(value, &["coverUri", "ogImage"]), LARGE),
        release_type: release_type(value),
        year: number(value, &["year"]).unwrap_or_default() as i32,
        track_count: number(value, &["trackCount"]).unwrap_or_default() as u32,
        release_date,
        label: labels(value),
        copyrights: Vec::new(),
        added_at: None,
    })
}

/// A liked album, which wraps the album with when it was liked.
pub fn liked_album(value: &Value) -> Option<Album> {
    let mut album = album(value.get("album").unwrap_or(value))?;
    album.added_at = when(value, &["timestamp"]);
    Some(album)
}

/// The tracks of `albums/{id}/with-tracks`, a list of discs.
pub fn volumes(value: &Value) -> Vec<Track> {
    list(value, "volumes")
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(track)
        .collect()
}

/// An artist's portrait at `size`.
pub fn portrait(value: &Value, size: &str) -> Option<String> {
    picture(
        value["cover"]["uri"]
            .as_str()
            .or_else(|| text(value, &["ogImage"])),
        size,
    )
}

pub fn saved_artist(value: &Value) -> Option<SavedArtist> {
    let artist = value.get("artist").unwrap_or(value);
    Some(SavedArtist {
        id: id(&artist["id"])?,
        name: text(artist, &["name"])?.to_owned(),
        cover: portrait(artist, COVER),
        added_at: when(value, &["timestamp"]),
    })
}

/// The id the client takes for a playlist: its owner and its number within the owner's
/// playlists, as `owner:kind`.
pub fn playlist_id(owner: &str, kind: &str) -> String {
    format!("{owner}:{kind}")
}

/// Splits a playlist id back into its owner and kind.
pub fn split_playlist(id: &str) -> Option<(&str, &str)> {
    id.split_once(':')
        .filter(|(owner, kind)| !owner.is_empty() && !kind.is_empty())
}

fn playlist_cover(value: &Value) -> Option<String> {
    let cover = &value["cover"];
    cover["uri"]
        .as_str()
        .or_else(|| list(cover, "itemsUri").first().and_then(Value::as_str))
        .or_else(|| text(value, &["ogImage"]))
        .and_then(|template| picture(Some(template), COVER))
}

/// A playlist, owned by `me` when its owner is the signed-in account.
pub fn playlist(value: &Value, me: &str) -> Option<Playlist> {
    let value = value.get("playlist").unwrap_or(value);
    let owner = &value["owner"];
    let owner_id = id(&owner["uid"]).or_else(|| id(&value["uid"]))?;
    let kind = id(&value["kind"])?;
    Some(Playlist {
        id: playlist_id(&owner_id, &kind),
        name: text(value, &["title"]).unwrap_or_default().to_owned(),
        owner: text(owner, &["name", "login"])
            .unwrap_or_default()
            .to_owned(),
        owned: owner_id == me,
        collaborative: value["collective"] == true,
        blend: false,
        public: text(value, &["visibility"]) == Some("public"),
        cover: playlist_cover(value),
        track_count: number(value, &["trackCount"]).unwrap_or_default() as u32,
        modified_at: when(value, &["modified"]),
        owner_id,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn pictures_fill_the_size_in() {
        assert_eq!(
            picture(Some("avatars.yandex.net/get-music-content/1/a/%%"), COVER).as_deref(),
            Some("https://avatars.yandex.net/get-music-content/1/a/400x400")
        );
        assert_eq!(picture(Some(""), COVER), None);
        assert_eq!(
            resize(
                "https://avatars.yandex.net/get-music-content/1/a/400x400",
                LARGE
            )
            .as_deref(),
            Some("https://avatars.yandex.net/get-music-content/1/a/1000x1000")
        );
        assert_eq!(resize("https://example.com/a/400x400", LARGE), None);
    }

    #[test]
    fn a_track_reads_its_album_and_version() {
        let value = json!({
            "id": 12345,
            "title": "Song",
            "version": "Live",
            "durationMs": 180000,
            "available": true,
            "contentWarning": "explicit",
            "artists": [{ "id": 7, "name": "One" }, { "id": 8, "name": "Two" }],
            "albums": [{
                "id": "99",
                "title": "Record",
                "genre": "rock",
                "coverUri": "avatars.yandex.net/x/%%",
                "trackPosition": { "volume": 2, "index": 3 }
            }]
        });
        let track = track(&value).unwrap();
        assert_eq!(track.id.as_deref(), Some("12345"));
        assert_eq!(track.name, "Song (Live)");
        assert_eq!(track.artists, "One, Two");
        assert_eq!(track.album_id.as_deref(), Some("99"));
        assert_eq!(
            track.cover.as_deref(),
            Some("https://avatars.yandex.net/x/400x400")
        );
        assert_eq!((track.disc_number, track.track_number), (2, 3));
        assert_eq!(track.duration, Duration::from_secs(180));
        assert!(track.explicit && track.playable);
    }

    #[test]
    fn a_playlist_is_named_by_owner_and_kind() {
        let value = json!({
            "playlist": {
                "kind": 1003,
                "title": "Mine",
                "owner": { "uid": 42, "login": "me" },
                "visibility": "public",
                "trackCount": 5,
                "cover": { "type": "mosaic", "itemsUri": ["avatars.yandex.net/m/%%"] }
            }
        });
        let playlist = playlist(&value, "42").unwrap();
        assert_eq!(playlist.id, "42:1003");
        assert!(playlist.owned && playlist.public);
        assert_eq!(playlist.owner, "me");
        assert_eq!(split_playlist(&playlist.id), Some(("42", "1003")));
    }
}
