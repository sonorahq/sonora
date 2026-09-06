use std::time::Duration;

use serde_json::Value;

use crate::models::{
    Album, AlbumDetail, Artist, ArtistProfile, ArtistRef, Playlist, PlaylistDetail, ReleaseType,
    SavedArtist, Track,
};

pub fn id(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(|id| id.as_u64().map(|id| id.to_string()).or_else(|| id.as_str().map(str::to_string)))
}

pub fn track(value: &Value, index: u32) -> Track {
    let artist = value.get("artist");
    let album = value.get("album");
    let name = artist
        .and_then(|artist| artist.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let artist_id = artist.and_then(id);
    Track {
        id: id(value),
        name: text(value, "title"),
        playable: value
            .get("readable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        artists: name.clone(),
        artist_refs: vec![ArtistRef {
            name,
            id: artist_id,
        }],
        album: album
            .and_then(|album| album.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        album_id: album.and_then(id),
        cover: cover(album.unwrap_or(value)),
        duration: Duration::from_secs(value.get("duration").and_then(Value::as_u64).unwrap_or(0)),
        added_at: value.get("time_add").and_then(Value::as_i64),
        added_by: None,
        playcount: value.get("rank").and_then(Value::as_u64),
        popularity: value
            .get("rank")
            .and_then(Value::as_u64)
            .map(|rank| (rank / 100_000) as u32)
            .unwrap_or(0),
        explicit: value
            .get("explicit_lyrics")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        track_number: value
            .get("track_position")
            .and_then(Value::as_u64)
            .map(|number| number as u32)
            .unwrap_or(index + 1),
        disc_number: value
            .get("disk_number")
            .and_then(Value::as_u64)
            .map(|number| number as u32)
            .unwrap_or(1),
        tags: Vec::new(),
        languages: Vec::new(),
        credits: Vec::new(),
    }
}

pub fn album(value: &Value) -> Album {
    let artist = value.get("artist");
    let name = artist
        .and_then(|artist| artist.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let date = text(value, "release_date");
    let year = date
        .get(..4)
        .and_then(|year| year.parse().ok())
        .unwrap_or(0);
    Album {
        id: id(value).unwrap_or_default(),
        name: text(value, "title"),
        artists: name.clone(),
        artist_refs: vec![ArtistRef {
            name,
            id: artist.and_then(id),
        }],
        cover: cover(value),
        cover_large: cover_large(value),
        release_type: release_type(value.get("record_type").and_then(Value::as_str)),
        year,
        track_count: value
            .get("nb_tracks")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        release_date: date,
        label: text(value, "label"),
        copyrights: Vec::new(),
        added_at: value.get("time_add").and_then(Value::as_i64),
    }
}

pub fn album_detail(value: &Value) -> AlbumDetail {
    let tracks = list(value.get("tracks"))
        .into_iter()
        .enumerate()
        .map(|(index, item)| track(item, index as u32))
        .collect();
    AlbumDetail {
        album: album(value),
        tracks,
    }
}

pub fn artist(value: &Value, top: Vec<Track>, albums: Vec<Album>) -> Artist {
    Artist {
        name: text(value, "name"),
        cover_large: picture_large(value),
        biography: None,
        monthly_listeners: value.get("nb_fan").and_then(Value::as_u64),
        top_tracks: top,
        albums,
    }
}

pub fn artist_profile(value: &Value) -> ArtistProfile {
    ArtistProfile {
        name: text(value, "name"),
        cover_large: picture_large(value),
        biography: None,
    }
}

pub fn saved_artist(value: &Value) -> SavedArtist {
    SavedArtist {
        id: id(value).unwrap_or_default(),
        name: text(value, "name"),
        cover: picture(value),
        added_at: value.get("time_add").and_then(Value::as_i64),
    }
}

pub fn playlist(value: &Value, user_id: &str) -> Playlist {
    let creator = value.get("creator").or_else(|| value.get("user"));
    let owner_id = creator.and_then(id).unwrap_or_default();
    Playlist {
        id: id(value).unwrap_or_default(),
        name: text(value, "title"),
        owner: creator
            .and_then(|creator| creator.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        owner_id: owner_id.clone(),
        owned: !user_id.is_empty() && owner_id == user_id,
        collaborative: value
            .get("collaborative")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        blend: false,
        public: value.get("public").and_then(Value::as_bool).unwrap_or(true),
        cover: cover(value).or_else(|| picture(value)),
        track_count: value
            .get("nb_tracks")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        modified_at: value
            .get("time_mod")
            .or_else(|| value.get("timestamp"))
            .and_then(Value::as_i64),
    }
}

pub fn playlist_detail(value: &Value, user_id: &str) -> PlaylistDetail {
    let tracks = list(value.get("tracks"))
        .into_iter()
        .enumerate()
        .map(|(index, item)| track(item, index as u32))
        .collect();
    PlaylistDetail {
        playlist: playlist(value, user_id),
        tracks,
    }
}

pub fn cover(value: &Value) -> Option<String> {
    url(value, &["cover_medium", "cover_big", "cover_xl", "cover"])
}

pub fn cover_large(value: &Value) -> Option<String> {
    url(value, &["cover_xl", "cover_big", "cover_medium"])
}

pub fn picture(value: &Value) -> Option<String> {
    url(value, &["picture_medium", "picture_big", "picture_xl", "picture"])
}

pub fn picture_large(value: &Value) -> Option<String> {
    url(value, &["picture_xl", "picture_big", "picture_medium"])
}

pub fn preview(value: &Value) -> Option<String> {
    value
        .get("preview")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}

pub fn list(value: Option<&Value>) -> Vec<&Value> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn url(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
    })
}

fn release_type(kind: Option<&str>) -> ReleaseType {
    match kind {
        Some("ep") => ReleaseType::Ep,
        Some("single") => ReleaseType::Single,
        Some("compile") | Some("compilation") => ReleaseType::Compilation,
        _ => ReleaseType::Album,
    }
}
