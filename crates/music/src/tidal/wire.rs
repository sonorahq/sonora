//! Conversions from Tidal's wire shapes to the crate's models. The api is not a published
//! contract and fields come and go between endpoints, so every read goes through tolerant
//! helpers rather than derived structs.

use std::time::Duration;

use serde_json::Value;

use crate::{Album, ArtistRef, Playlist, ReleaseType, SavedArtist, Track, calendar};

/// The cover size used in lists, and the one used on a page header.
pub const COVER: u32 = 320;
pub const COVER_LARGE: u32 = 1280;
/// The square portrait sizes Tidal serves artists at.
pub const PORTRAIT: u32 = 320;
pub const PORTRAIT_LARGE: u32 = 750;
/// The square sizes Tidal serves playlist images at.
pub const SQUARE: u32 = 320;
/// The square size Tidal serves profile pictures at.
pub const AVATAR: u32 = 210;

/// What a mix id carries in front, to tell it from a playlist uuid.
pub const MIX: &str = "mix:";

/// An id, spelled as a number or as a string.
pub fn id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .filter(|id| !id.is_empty() && id != "0")
}

/// The string under `key`, if there is a non-empty one.
pub fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The whole number under `key`, spelled as an integer or as a float.
pub fn number(value: &Value, key: &str) -> Option<u64> {
    let field = value.get(key)?;
    field
        .as_u64()
        .or_else(|| field.as_f64().map(|number| number as u64))
}

/// A count or position, in whatever integer type the model holds it. One that does not fit is
/// treated as missing rather than wrapped.
pub fn count<T: TryFrom<u64>>(value: &Value, key: &str) -> Option<T> {
    T::try_from(number(value, key)?).ok()
}

pub fn decimal(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64()
}

/// The items of a paged answer. Favorites and playlist items come wrapped as
/// `{ "item": …, "created" | "dateAdded": … }`, and the wrapper is kept so the caller can read
/// when the item was added.
pub fn items(page: &Value) -> &[Value] {
    page.get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// An item out of its wrapper, or the value itself when it is not wrapped.
pub fn unwrap(value: &Value) -> &Value {
    value
        .get("item")
        .filter(|item| item.is_object())
        .unwrap_or(value)
}

/// The `resources.tidal.com` url of an image uuid at a square size. A field that already holds
/// a url is passed through as it is.
pub fn image(uuid: Option<&str>, size: u32) -> Option<String> {
    let uuid = uuid?;
    if uuid.starts_with("https://") {
        return Some(uuid.to_owned());
    }
    Some(format!(
        "https://resources.tidal.com/images/{}/{size}x{size}.jpg",
        uuid.replace('-', "/")
    ))
}

/// A track out of a bare or wrapped track object. Videos, which share playlists and mixes
/// with tracks, are left out.
pub fn track(wrapper: &Value) -> Option<Track> {
    let value = unwrap(wrapper);
    if wrapper.get("type").and_then(Value::as_str) == Some("video")
        || value.get("type").and_then(Value::as_str) == Some("Music Video")
    {
        return None;
    }
    let id = value.get("id").and_then(id)?;
    let title = text(value, "title")?;
    let name = match text(value, "version") {
        Some(version) if !title.contains(version) => format!("{title} ({version})"),
        _ => title.to_owned(),
    };
    let (artists, artist_refs) = artists_of(value);
    let album = value.get("album").unwrap_or(&Value::Null);
    // An Atmos-only copy stays listed, so an album keeps its track count, but cannot play.
    let playable = value
        .get("allowStreaming")
        .and_then(Value::as_bool)
        .unwrap_or(true)
        && value
            .get("streamReady")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        && !atmos_only(value);
    Some(Track {
        id: Some(id),
        name,
        playable,
        artists,
        artist_refs,
        album: text(album, "title").unwrap_or_default().to_owned(),
        album_id: album.get("id").and_then(self::id),
        cover: image(text(album, "cover"), COVER),
        duration: Duration::from_secs(number(value, "duration").unwrap_or(0)),
        added_at: added(wrapper),
        added_by: None,
        playcount: None,
        popularity: count(value, "popularity").unwrap_or(0),
        explicit: value
            .get("explicit")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        track_number: count(value, "trackNumber").unwrap_or(0),
        disc_number: count(value, "volumeNumber").unwrap_or(1),
        tags: Vec::new(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

pub fn track_list(page: &Value) -> Vec<Track> {
    items(page).iter().filter_map(track).collect()
}

/// An album out of a bare or wrapped album object.
pub fn album(wrapper: &Value) -> Option<Album> {
    let value = unwrap(wrapper);
    let id = value.get("id").and_then(id)?;
    let (artists, artist_refs) = artists_of(value);
    let release_date = text(value, "releaseDate")
        .or_else(|| text(value, "streamStartDate"))
        .unwrap_or_default();
    let cover = text(value, "cover");
    Some(Album {
        id,
        name: text(value, "title")?.to_owned(),
        artists,
        artist_refs,
        cover: image(cover, COVER),
        cover_large: image(cover, COVER_LARGE),
        release_type: match text(value, "type") {
            Some("SINGLE") => ReleaseType::Single,
            Some("EP") => ReleaseType::Ep,
            Some("COMPILATION") => ReleaseType::Compilation,
            _ => ReleaseType::Album,
        },
        year: release_date
            .get(..4)
            .and_then(|year| year.parse().ok())
            .unwrap_or(0),
        track_count: count(value, "numberOfTracks").unwrap_or(0),
        release_date: release_date.get(..10).unwrap_or(release_date).to_owned(),
        label: String::new(),
        copyrights: text(value, "copyright")
            .map(|copyright| vec![copyright.to_owned()])
            .unwrap_or_default(),
        added_at: added(wrapper),
    })
}

pub fn album_list(page: &Value) -> Vec<Album> {
    items(page).iter().filter_map(album).collect()
}

/// An artist out of a bare or wrapped artist object.
pub fn artist(wrapper: &Value) -> Option<SavedArtist> {
    let value = unwrap(wrapper);
    Some(SavedArtist {
        id: value.get("id").and_then(id)?,
        name: text(value, "name")?.to_owned(),
        cover: image(text(value, "picture"), PORTRAIT),
        added_at: added(wrapper),
    })
}

pub fn artist_list(page: &Value) -> Vec<SavedArtist> {
    items(page).iter().filter_map(artist).collect()
}

/// A playlist out of a bare or wrapped playlist object. `user_id` decides whether the
/// listener owns it.
pub fn playlist(wrapper: &Value, user_id: &str) -> Option<Playlist> {
    let value = unwrap(wrapper);
    let id = text(value, "uuid")?.to_owned();
    let creator = value.get("creator").unwrap_or(&Value::Null);
    let owner_id = creator.get("id").and_then(self::id).unwrap_or_default();
    // Tidal's own playlists name no creator, and a listener's often do not either.
    // `id` reads Tidal's editorial creator id, zero, as no id at all.
    let editorial = owner_id.is_empty();
    let owner = text(creator, "name")
        .or(editorial.then_some("TIDAL"))
        .unwrap_or_default()
        .to_owned();
    let modified_at = text(value, "lastUpdated")
        .or_else(|| text(value, "created"))
        .and_then(calendar::seconds);
    Some(Playlist {
        name: text(value, "title").unwrap_or_default().to_owned(),
        owned: !user_id.is_empty() && owner_id == user_id,
        owner,
        owner_id,
        collaborative: false,
        blend: false,
        public: value
            .get("publicPlaylist")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        cover: image(text(value, "squareImage"), SQUARE)
            .or_else(|| image(text(value, "image"), SQUARE)),
        track_count: count(value, "numberOfTracks").unwrap_or(0),
        modified_at,
        id,
    })
}

/// A mix as a playlist the listener does not own. Its id carries `MIX` in front, so a later
/// lookup knows to ask for a mix rather than a playlist. The `v1` pages and the `v2` feed
/// spell a mix's title and images differently, so both spellings are read.
pub fn mix(value: &Value) -> Option<Playlist> {
    let id = value.get("id").and_then(id)?;
    let title = text(value, "title")
        .or_else(|| {
            value
                .get("titleTextInfo")
                .and_then(|info| text(info, "text"))
        })?
        .to_owned();
    let cover = value
        .get("images")
        .and_then(|images| images.get("MEDIUM").or_else(|| images.get("LARGE")))
        .and_then(|image| text(image, "url"))
        .or_else(|| {
            value
                .get("mixImages")
                .and_then(Value::as_array)
                .and_then(|images| images.iter().find_map(|image| text(image, "url")))
        })
        .map(str::to_owned);
    Some(Playlist {
        id: format!("{MIX}{id}"),
        name: title,
        // Tidal makes every mix. Its subtitles say so in a sentence, or describe the mix.
        owner: "TIDAL".to_owned(),
        owner_id: String::new(),
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover,
        track_count: 0,
        modified_at: None,
    })
}

/// An artist biography without Tidal's link markup: `[wimpLink artistId="…"]Name[/wimpLink]`
/// keeps its name, and the `<br/>` breaks become new lines.
pub fn plain_bio(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        plain.push_str(&rest[..open]);
        let Some(close) = rest[open..].find(']') else {
            rest = &rest[open..];
            break;
        };
        rest = &rest[open + close + 1..];
    }
    plain.push_str(rest);
    plain
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .trim()
        .to_owned()
}

/// When a wrapped item was added, as seconds since the epoch.
fn added(wrapper: &Value) -> Option<i64> {
    text(wrapper, "created")
        .or_else(|| text(wrapper, "dateAdded"))
        .or_else(|| text(unwrap(wrapper), "dateAdded"))
        .and_then(calendar::seconds)
}

/// The artists of a track or an album, as one line to show and as links.
fn artists_of(value: &Value) -> (String, Vec<ArtistRef>) {
    // Most answers list every artist; a few only name the main one.
    let list: &[Value] = match value.get("artists").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list,
        _ => value
            .get("artist")
            .map(std::slice::from_ref)
            .unwrap_or_default(),
    };
    let refs: Vec<ArtistRef> = list
        .iter()
        .filter_map(|artist| {
            Some(ArtistRef {
                name: text(artist, "name")?.to_owned(),
                id: artist.get("id").and_then(id),
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

/// Whether a track is a Dolby Atmos copy with no stereo version behind it. Only a Widevine
/// stream carries one, which Sonora does not play.
fn atmos_only(value: &Value) -> bool {
    value
        .get("audioModes")
        .and_then(Value::as_array)
        .is_some_and(|modes| {
            !modes.is_empty()
                && modes
                    .iter()
                    .all(|mode| mode.as_str() == Some("DOLBY_ATMOS"))
        })
}
