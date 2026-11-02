//! Conversions from Deezer's wire shapes to the crate's models. The gateway answers in
//! SCREAMING_SNAKE and the public REST api in snake_case, and neither is a stable contract,
//! so every read goes through tolerant helpers rather than derived structs.

use std::time::Duration;

use serde_json::Value;

use crate::{
    Album, ArtistRef, Genre, GenreItem, GenreSection, Playlist, ReleaseType, SavedArtist, Track,
    UserProfile, calendar,
};

/// Sets a smart tracklist's id apart from a playlist's, so a later lookup knows to ask for
/// the tracklist. Deezer names these by a word such as `discovery` rather than a number.
pub const SMART: &str = "smarttracklist:";

/// Sets a Flow's id apart from a playlist's. What follows is the Flow's config, such as
/// `default` or `chill`.
pub const FLOW: &str = "flow:";

/// Who a smart tracklist or a Flow is credited to, since Deezer makes them for the listener.
const MAKER: &str = "Deezer";

/// A track id is a numeric string. Zero and negative ids are user uploads, which the
/// streaming endpoints treat differently; the sign stays in the string.
pub fn id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .filter(|id| !id.is_empty() && id != "0")
}

/// The first non-empty string under any of `keys`.
pub fn text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(key))?.as_str()
}

/// The first fractional number under any of `keys`, spelled as a number or as a string.
pub fn decimal(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        let field = value.get(key)?;
        field
            .as_f64()
            .or_else(|| field.as_str()?.trim().parse().ok())
    })
}

/// The first number under any of `keys`. A key the response omits is skipped rather than
/// ending the search, because the two apis spell the same field differently and only one of
/// the spellings is ever present.
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

/// A unix timestamp, or a gateway `YYYY-MM-DD HH:MM:SS` / `YYYY-MM-DD` string as seconds.
fn when(value: &Value, keys: &[&str]) -> Option<i64> {
    if let Some(at) = number(value, keys) {
        return Some(at as i64);
    }
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .and_then(calendar::seconds)
    })
}

/// The signed-in account from the `USER` of `deezer.getUserData`. A refused arl answers user
/// id 0, which comes back as an empty id.
pub fn profile(user: &Value) -> UserProfile {
    UserProfile {
        id: id(&user["USER_ID"]).unwrap_or_default(),
        display_name: text(user, &["BLOG_NAME"]).unwrap_or("Deezer").to_owned(),
        avatar: image("user", text(user, &["USER_PICTURE"]), 300),
    }
}

/// The `https://cdn-images.dzcdn.net/images/<kind>/<md5>/<size>x<size>-000000-80-0-0.jpg`
/// cover url Deezer builds from a picture hash. None when the field holds something other
/// than a hash, which is the caller's cue to fall back to a url field.
pub fn image(kind: &str, md5: Option<&str>, size: u32) -> Option<String> {
    let md5 = md5?.trim();
    if !hashed(md5) {
        return None;
    }
    Some(format!(
        "https://cdn-images.dzcdn.net/images/{kind}/{md5}/{size}x{size}-000000-80-0-0.jpg"
    ))
}

/// Whether a picture field is the hash an image url is built from. The public api answers
/// several of them with a link to the image instead, and a link cannot stand in for a hash.
/// A collage is several hashes joined by `-`, so each part is checked on its own.
fn hashed(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn cover(value: &Value, size: u32) -> Option<String> {
    let md5 = text(value, &["md5_image", "ALB_PICTURE", "picture"]);
    image("cover", md5, size)
        .or_else(|| text(value, &["cover_big", "picture_big"]).map(str::to_owned))
}

fn artist_picture(value: &Value, size: u32) -> Option<String> {
    let md5 = text(value, &["picture", "ART_PICTURE"]);
    image("artist", md5, size).or_else(|| text(value, &["picture_big"]).map(str::to_owned))
}

fn artists_of(value: &Value) -> (String, Vec<ArtistRef>) {
    // a public-api track carries one `artist` object; a gateway track carries ART_NAME plus
    // optionally a SNG_CONTRIBUTORS.mainartist list
    if let Some(list) = value
        .get("SNG_CONTRIBUTORS")
        .and_then(|contributors| contributors.get("mainartist"))
        .and_then(Value::as_array)
        && !list.is_empty()
    {
        let names: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let single = value
            .get("ART_ID")
            .and_then(id)
            .map(|artist_id| (names.clone(), artist_id));
        let refs = match (single, names.len()) {
            (Some((names, artist_id)), 1) => vec![ArtistRef {
                name: names[0].clone(),
                id: Some(artist_id),
            }],
            _ => names
                .iter()
                .map(|name| ArtistRef {
                    name: name.clone(),
                    id: None,
                })
                .collect(),
        };
        return (names.join(", "), refs);
    }
    if let Some(artist) = value.get("artist")
        && let Some(name) = artist.get("name").and_then(Value::as_str)
    {
        return (
            name.to_owned(),
            vec![ArtistRef {
                name: name.to_owned(),
                id: artist.get("id").and_then(id),
            }],
        );
    }
    let name = text(value, &["ART_NAME", "artist_name"]).unwrap_or_default();
    let refs = match name.is_empty() {
        true => Vec::new(),
        false => vec![ArtistRef {
            name: name.to_owned(),
            id: value.get("ART_ID").and_then(id),
        }],
    };
    (name.to_owned(), refs)
}

fn truthy(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| value.get(key))
        .map(|field| match field {
            Value::Bool(flag) => *flag,
            Value::Number(number) => number.as_i64() != Some(0),
            Value::String(text) => matches!(text.as_str(), "1" | "true"),
            _ => false,
        })
        .unwrap_or(false)
}

/// A track's full title. The gateway names a track without its version and hands the version
/// over in a field of its own, so the two are joined here and a track keeps one name whichever
/// api listed it.
fn versioned(title: &str, version: Option<&str>) -> String {
    let version = version.map(str::trim).filter(|version| !version.is_empty());
    match version {
        Some(version) if !title.contains(version) => format!("{title} {version}"),
        _ => title.to_owned(),
    }
}

/// One track from either api. Anything the response omits falls back to a neutral default;
/// `duration` is the one field every listing answers.
pub fn track(value: &Value) -> Option<Track> {
    let track_id = value
        .get("SNG_ID")
        .or_else(|| value.get("id"))
        .and_then(id)?;
    let name = versioned(
        text(value, &["SNG_TITLE", "title", "TITLE"]).unwrap_or_default(),
        text(value, &["VERSION"]),
    );
    let (artists, artist_refs) = artists_of(value);
    let album = value.get("album").cloned().unwrap_or(Value::Null);
    Some(Track {
        id: Some(track_id),
        name,
        playable: truthy(value, &["readable"])
            || !value
                .get("readable")
                .map(|flag| flag.is_boolean())
                .unwrap_or(false),
        artists,
        artist_refs,
        album: text(&album, &["title"])
            .or_else(|| text(value, &["ALB_TITLE"]))
            .unwrap_or_default()
            .to_owned(),
        album_id: album
            .get("id")
            .and_then(id)
            .or_else(|| value.get("ALB_ID").and_then(id)),
        cover: cover(&album, 300).or_else(|| cover(value, 300)),
        duration: Duration::from_secs(number(value, &["DURATION", "duration"]).unwrap_or(0)),
        added_at: when(value, &["ADDED_AT", "time_add", "DATE_ADD"]),
        added_by: None,
        playcount: None,
        popularity: number(value, &["RANK", "rank"])
            .map(|rank| (rank / 10_000).min(100) as u32)
            .unwrap_or(0),
        explicit: truthy(value, &["explicit_lyrics", "EXPLICIT_LYRICS"]),
        track_number: number(value, &["TRACK_NUMBER", "track_position"]).unwrap_or(0) as u32,
        disc_number: number(value, &["DISK_NUMBER"]).unwrap_or(1) as u32,
        tags: Vec::new(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

pub fn track_list(value: &Value) -> Vec<Track> {
    value
        .get("data")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(track).collect())
        .unwrap_or_default()
}

/// One album from either api.
pub fn album(value: &Value) -> Option<Album> {
    let album_id = value
        .get("ALB_ID")
        .or_else(|| value.get("id"))
        .and_then(id)?;
    let (artists, artist_refs) = artists_of(value);
    let release = text(
        value,
        &[
            "release_date",
            "PHYSICAL_RELEASE_DATE",
            "DIGITAL_RELEASE_DATE",
        ],
    )
    .unwrap_or_default();
    let year = release
        .split('-')
        .next()
        .and_then(|year| year.parse().ok())
        .unwrap_or(0);
    Some(Album {
        id: album_id,
        name: text(value, &["ALB_TITLE", "title"])
            .unwrap_or_default()
            .to_owned(),
        artists,
        artist_refs,
        cover: cover(value, 300),
        cover_large: cover(value, 1200),
        release_type: match text(value, &["record_type"]) {
            Some("single") => ReleaseType::Single,
            Some("ep") => ReleaseType::Ep,
            Some("compile") => ReleaseType::Compilation,
            _ => ReleaseType::Album,
        },
        year,
        track_count: number(value, &["NB_TRAK", "nb_tracks"]).unwrap_or(0) as u32,
        release_date: release.to_owned(),
        label: text(value, &["label", "LABEL"])
            .unwrap_or_default()
            .to_owned(),
        copyrights: Vec::new(),
        added_at: when(value, &["ADDED_AT", "time_add", "DATE_ADD"]),
    })
}

/// One playlist, from `deezer.pageProfile`, `deezer.pagePlaylist` or the public api.
pub fn playlist(value: &Value, user_id: &str) -> Option<Playlist> {
    let playlist_id = value
        .get("PLAYLIST_ID")
        .or_else(|| value.get("id"))
        .and_then(id)?;
    // a fetched playlist names its owner `creator`, one found by search names it `user`
    let creator = value.get("creator").or_else(|| value.get("user"));
    let owner = text(value, &["PARENT_USERNAME", "CREATOR_NAME"])
        .or_else(|| {
            creator
                .and_then(|creator| creator.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let owner_id = text(value, &["PARENT_USER_ID"])
        .map(str::to_owned)
        .or_else(|| creator.and_then(|creator| creator.get("id")).and_then(id))
        .unwrap_or_default();
    // a playlist with no picture of its own answers with a collage of four album hashes, and
    // PICTURE_TYPE is what says the url is built under `cover` rather than `playlist`
    let md5 = text(value, &["PLAYLIST_PICTURE", "picture"]);
    let kind = text(value, &["PICTURE_TYPE"]).unwrap_or("playlist");
    Some(Playlist {
        id: playlist_id,
        name: text(value, &["TITLE", "title"])
            .unwrap_or_default()
            .to_owned(),
        owner: owner.to_owned(),
        owner_id: owner_id.clone(),
        owned: !owner_id.is_empty() && owner_id == user_id,
        collaborative: false,
        blend: false,
        public: number(value, &["STATUS"])
            .map(|status| status == 1)
            .unwrap_or_else(|| truthy(value, &["public"])),
        cover: image(kind, md5, 300)
            .or_else(|| text(value, &["picture_medium"]).map(str::to_owned)),
        track_count: number(value, &["NB_SONG", "nb_tracks"]).unwrap_or(0) as u32,
        modified_at: when(value, &["DATE_MOD"]),
    })
}

/// One saved (favorite) artist.
pub fn saved_artist(value: &Value) -> Option<SavedArtist> {
    let artist_id = value
        .get("ART_ID")
        .or_else(|| value.get("id"))
        .and_then(id)?;
    Some(SavedArtist {
        id: artist_id,
        name: text(value, &["ART_NAME", "name"])
            .unwrap_or_default()
            .to_owned(),
        cover: artist_picture(value, 300),
        added_at: when(value, &["ADDED_AT", "time_add", "DATE_ADD"]),
    })
}

/// The sections of a `page.get` answer that hold at least one card Sonora can draw, in the
/// order Deezer lays them out. A section without a title takes `fallback`, which is how a
/// module page names its one untitled grid.
pub fn sections(page: &Value, user_id: &str, fallback: &str) -> Vec<GenreSection> {
    page.get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|section| {
            let items: Vec<GenreItem> = section
                .get("items")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|item| card(item, user_id))
                .collect();
            let title = text(section, &["title"])
                .filter(|title| !title.is_empty())
                .unwrap_or(fallback)
                .to_owned();
            (!items.is_empty()).then_some(GenreSection { title, items })
        })
        .collect()
}

/// One card of a `page.get` section. The `data` under it is the same gateway record the rest
/// of the gateway answers with, except for a Flow and a channel, which only the card describes
/// in full.
fn card(item: &Value, user_id: &str) -> Option<GenreItem> {
    let data = item.get("data")?;
    match text(item, &["type"])? {
        "album" => album(data).map(GenreItem::Album),
        "playlist" => playlist(data, user_id).map(GenreItem::Playlist),
        "artist" => saved_artist(data).map(GenreItem::Artist),
        "track" => track(data).map(GenreItem::Track),
        "smarttracklist" => smart_tracklist(data).map(GenreItem::Playlist),
        "flow" => flow(item).map(GenreItem::Playlist),
        "channel" => channel(item).map(GenreItem::Genre),
        _ => None,
    }
}

/// A smart tracklist as a playlist, from a home card or the `DATA` of
/// `deezer.pageSmartTracklist`. Every daily mix is titled `daily`, so the generated title
/// tells them apart, with the date it ends in left out.
pub fn smart_tracklist(value: &Value) -> Option<Playlist> {
    let smart_id = text(value, &["SMARTTRACKLIST_ID"]).filter(|id| !id.is_empty())?;
    let name = text(value, &["AUTO_GENERATED_TITLE"])
        .map(undated)
        .filter(|name| !name.is_empty())
        .or_else(|| text(value, &["TITLE"]))
        .unwrap_or(smart_id);
    let cover = value.get("COVER").and_then(|cover| {
        image(
            text(cover, &["TYPE"]).unwrap_or("cover"),
            text(cover, &["MD5"]),
            300,
        )
    });
    Some(made(
        format!("{SMART}{smart_id}"),
        name,
        cover,
        number(value, &["NB_SONG"]).unwrap_or(0) as u32,
    ))
}

/// A Flow card as a playlist. The card's `data` names the Flow but leaves its picture to the
/// card itself.
pub fn flow(item: &Value) -> Option<Playlist> {
    let data = item.get("data").unwrap_or(&Value::Null);
    let config = text(item, &["id"])
        .or_else(|| text(data, &["id"]))
        .filter(|id| !id.is_empty())?;
    let name = text(item, &["title"])
        .or_else(|| text(data, &["title"]))
        .unwrap_or("Flow");
    Some(made(format!("{FLOW}{config}"), name, picture(item, 300), 0))
}

/// A channel card as a genre whose id is the page path `page.get` opens it by, such as
/// `channels/metal`. A card that leads anywhere else, such as the concerts page, is left out.
fn channel(item: &Value) -> Option<Genre> {
    let path = text(item, &["target"])?.trim_start_matches('/');
    if !path.starts_with("channels/") {
        return None;
    }
    let data = item.get("data").unwrap_or(&Value::Null);
    let name = text(item, &["title"]).or_else(|| text(data, &["title", "name"]))?;
    let cover = picture(item, 300).or_else(|| {
        let linked = item.get("image_linked_item")?;
        image(
            text(linked, &["type"]).unwrap_or("playlist"),
            text(linked, &["md5"]),
            300,
        )
    });
    Some(Genre {
        id: path.to_owned(),
        name: name.to_owned(),
        cover,
    })
}

/// The first picture a card lists.
fn picture(item: &Value, size: u32) -> Option<String> {
    let first = item.get("pictures")?.as_array()?.first()?;
    image(
        text(first, &["type"]).unwrap_or("cover"),
        text(first, &["md5"]),
        size,
    )
}

/// A playlist Deezer makes for the listener rather than one a user owns.
fn made(id: String, name: &str, cover: Option<String>, track_count: u32) -> Playlist {
    Playlist {
        id,
        name: name.to_owned(),
        owner: MAKER.to_owned(),
        owner_id: String::new(),
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover,
        track_count,
        modified_at: None,
    }
}

/// A generated title without the ` - 31.10.2026` it ends in, whatever the date's format.
fn undated(title: &str) -> &str {
    match title.rsplit_once(" - ") {
        Some((name, date))
            if date.bytes().any(|byte| byte.is_ascii_digit())
                && date
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || b"./-".contains(&byte)) =>
        {
            name.trim()
        }
        _ => title.trim(),
    }
}

/// The music channels of the `channels/explore` page. Podcasts are not something Sonora
/// plays, so the podcasts channel goes, and so does any section where a channel's picture
/// comes from a podcast, which is how the podcast categories show.
pub fn explore(page: &Value) -> Vec<Genre> {
    page.get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|section| section.get("items")?.as_array())
        .filter(|items| {
            !items.iter().any(|item| {
                item.get("image_linked_item")
                    .and_then(|linked| text(linked, &["type"]))
                    == Some("talk")
            })
        })
        .flatten()
        .filter(|item| text(item, &["type"]) == Some("channel"))
        .filter_map(channel)
        .filter(|genre| genre.id != "channels/podcasts")
        .collect()
}
