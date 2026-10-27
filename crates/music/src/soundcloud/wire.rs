//! Conversions from SoundCloud's wire shapes to the crate's models. The internal api is not a
//! published contract, so every read goes through tolerant helpers rather than derived structs,
//! the way the Deezer conversions do.

use std::time::Duration;

use serde_json::Value;

use crate::{
    Album, ArtistRef, Playlist, ReleaseType, SavedArtist, Track, UserProfile, iso_8601_to_epoch,
};

/// An id as a numeric string. Zero and missing ids answer nothing, since every id the api
/// hands out is a positive number.
pub fn id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .filter(|id| !id.is_empty() && id != "0")
}

/// The first non-empty string under any of `keys`.
pub fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        let field = value.get(key)?;
        field.as_str().filter(|text| !text.is_empty())
    })
}

/// The first number under any of `keys`. A key the response omits is skipped, since the api
/// spells the same field differently across endpoints.
pub fn number(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        let Some(field) = value.get(key) else {
            continue;
        };
        if let Some(number) = field.as_u64() {
            return Some(number);
        }
        if let Some(text) = field.as_str()
            && let Ok(number) = text.parse()
        {
            return Some(number);
        }
    }
    None
}

/// A flag under any of `keys`, read from a boolean, a number or a string.
fn truthy(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| value.get(key))
        .map(|field| match field {
            Value::Bool(flag) => *flag,
            Value::Number(number) => number.as_i64() != Some(0),
            Value::String(text) => matches!(text.as_str(), "1" | "true" | "public"),
            _ => false,
        })
        .unwrap_or(false)
}

/// A unix timestamp, or an ISO8601 date-time string, as seconds.
fn when(value: &Value, keys: &[&str]) -> Option<i64> {
    if let Some(at) = number(value, keys) {
        return Some(at as i64);
    }
    keys.iter()
        .find_map(|key| iso_8601_to_epoch(value.get(*key)?.as_str()))
}

/// The artwork url at `size`, from the `artwork_url` or `avatar_url` field. SoundCloud serves
/// the image at any size by swapping the trailing `-large` for `-t{size}x{size}`.
pub fn image(value: &Value, keys: &[&str], size: u32) -> Option<String> {
    let url = text(value, keys)?;
    Some(resized(url, size))
}

fn resized(url: &str, size: u32) -> String {
    url.replace("-large", &format!("-t{size}x{size}"))
}

/// The signed-in account from `/me`.
pub fn profile(value: &Value) -> UserProfile {
    UserProfile {
        id: value.get("id").and_then(id).unwrap_or_default(),
        display_name: text(value, &["username", "full_name"])
            .unwrap_or("SoundCloud")
            .to_owned(),
        avatar: image(value, &["avatar_url"], 300),
    }
}

/// The user who uploaded a track or owns a playlist, as the `artist` the app shows.
fn artist_of(user: &Value) -> (String, ArtistRef) {
    let name = text(user, &["username", "full_name"]).unwrap_or_default();
    (
        name.to_owned(),
        ArtistRef {
            name: name.to_owned(),
            id: user.get("id").and_then(id),
        },
    )
}

/// One track. `playable` reads `streamable` plus the `policy` SoundCloud stamps blocked or
/// private tracks with.
pub fn track(value: &Value) -> Option<Track> {
    let track_id = value.get("id").and_then(id)?;
    // A stub entry some playlist listings carry has no title; only a full track is worth
    // keeping, so anything without one is skipped rather than shown as an empty row.
    text(value, &["title"])?;
    let user = value.get("user").unwrap_or(&Value::Null);
    let (artists, artist_ref) = artist_of(user);
    let album_title = value
        .get("publisher_metadata")
        .and_then(|meta| meta.get("album_title"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let policy = text(value, &["policy"]).unwrap_or_default();
    Some(Track {
        id: Some(track_id),
        name: text(value, &["title"]).unwrap_or_default().to_owned(),
        playable: truthy(value, &["streamable"]) && policy != "BLOCK" && !drm(value),
        artists,
        artist_refs: if artist_ref.name.is_empty() {
            Vec::new()
        } else {
            vec![artist_ref]
        },
        album: album_title.to_owned(),
        album_id: None,
        cover: image(value, &["artwork_url"], 300),
        duration: Duration::from_millis(number(value, &["duration", "full_duration"]).unwrap_or(0)),
        added_at: when(value, &["created_at", "release_date"]),
        added_by: None,
        playcount: number(value, &["playback_count"]),
        popularity: number(value, &["likes_count"])
            .map(|likes| likes.min(100) as u32)
            .unwrap_or(0),
        explicit: value
            .get("publisher_metadata")
            .and_then(|meta| meta.get("explicit"))
            .map(|flag| flag.as_bool() == Some(true))
            .unwrap_or(false),
        track_number: 0,
        disc_number: 1,
        tags: value
            .get("tag_list")
            .and_then(Value::as_str)
            .map(|tags| tags.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

/// Every track a listing holds, in the shape each endpoint uses.
pub fn track_list(value: &Value) -> Vec<Track> {
    collection(value)
        .into_iter()
        .filter_map(|item| track(&item))
        .collect()
}

/// Every playlist a listing holds.
pub fn playlist_list(value: &Value, user_id: &str) -> Vec<Playlist> {
    collection(value)
        .into_iter()
        .filter_map(|item| playlist(&item, user_id))
        .collect()
}

/// Every album a listing holds.
pub fn album_list(value: &Value) -> Vec<Album> {
    collection(value)
        .into_iter()
        .filter_map(|item| album(&item))
        .collect()
}

/// The array a listing endpoint wraps in `collection`, or a bare array, or a single object.
pub(crate) fn collection(value: &Value) -> Vec<Value> {
    value
        .get("collection")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .map(Vec::as_slice)
        .map(|list| list.to_vec())
        .or_else(|| value.as_object().map(|_| vec![value.clone()]))
        .unwrap_or_default()
}

/// One playlist. SoundCloud's albums are playlists with `is_album` set, so an album shares this
/// conversion and `album` layers the release metadata on top.
pub fn playlist(value: &Value, user_id: &str) -> Option<Playlist> {
    let playlist_id = value.get("id").and_then(id)?;
    let user = value.get("user").unwrap_or(&Value::Null);
    let (owner, owner_ref) = artist_of(user);
    let owner_id = owner_ref.id.unwrap_or_default();
    Some(Playlist {
        id: playlist_id,
        // SoundCloud names its "related tracks" mixes `Related tracks: <seed>`, which its own
        // site shows without the prefix, so it is stripped here.
        name: text(value, &["title"])
            .map(|title| title.strip_prefix("Related tracks: ").unwrap_or(title))
            .unwrap_or_default()
            .to_owned(),
        owner,
        owner_id: owner_id.clone(),
        owned: !owner_id.is_empty() && owner_id == user_id,
        collaborative: false,
        blend: false,
        public: matches!(text(value, &["sharing"]), Some("public") | None),
        cover: image(value, &["artwork_url", "calculated_artwork_url"], 300),
        track_count: number(value, &["track_count"]).unwrap_or(0) as u32,
        modified_at: when(value, &["last_modified", "created_at"]),
    })
}

/// One album, from a playlist the api marks `is_album`.
pub fn album(value: &Value) -> Option<Album> {
    let album_id = value.get("id").and_then(id)?;
    let user = value.get("user").unwrap_or(&Value::Null);
    let (artists, artist_ref) = artist_of(user);
    let release = text(value, &["release_date", "display_date", "created_at"]).unwrap_or_default();
    let year = release
        .split('-')
        .next()
        .and_then(|year| year.parse().ok())
        .unwrap_or(0);
    let meta = value.get("publisher_metadata");
    Some(Album {
        id: album_id,
        name: text(value, &["title"]).unwrap_or_default().to_owned(),
        artists,
        artist_refs: if artist_ref.name.is_empty() {
            Vec::new()
        } else {
            vec![artist_ref]
        },
        cover: image(value, &["artwork_url", "calculated_artwork_url"], 300),
        cover_large: image(value, &["artwork_url", "calculated_artwork_url"], 1000),
        release_type: ReleaseType::Album,
        year,
        track_count: number(value, &["track_count"]).unwrap_or(0) as u32,
        release_date: release.to_owned(),
        label: text(value, &["label_name"])
            .or_else(|| meta.and_then(|meta| text(meta, &["publisher"])))
            .unwrap_or_default()
            .to_owned(),
        copyrights: meta
            .map(|meta| {
                ["p_line", "c_line"]
                    .iter()
                    .filter_map(|key| meta.get(*key))
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        added_at: when(value, &["created_at", "release_date"]),
    })
}

/// One saved (followed) user, as the artist the library lists.
pub fn saved_artist(value: &Value) -> Option<SavedArtist> {
    let artist_id = value.get("id").and_then(id)?;
    Some(SavedArtist {
        id: artist_id,
        name: text(value, &["username", "full_name"])
            .unwrap_or_default()
            .to_owned(),
        cover: image(value, &["avatar_url"], 300),
        added_at: when(value, &["created_at"]),
    })
}

/// Whether a track's transcodings are encrypted, which SoundCloud only allows its own apps to
/// play. Such a track still advertises plain transcodings that answer 404, so it is marked
/// unplayable rather than queued to a stream that cannot open.
fn drm(value: &Value) -> bool {
    value
        .get("media")
        .and_then(|media| media.get("transcodings"))
        .and_then(Value::as_array)
        .is_some_and(|transcodings| {
            transcodings.iter().any(|entry| {
                entry
                    .get("format")
                    .and_then(|format| format.get("protocol"))
                    .and_then(Value::as_str)
                    .is_some_and(|protocol| protocol.contains("encrypted"))
            })
        })
}
