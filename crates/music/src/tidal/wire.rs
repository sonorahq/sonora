//! Conversions from Tidal's wire shapes to the crate's models. The api is not a published
//! contract and fields come and go between endpoints, so every read goes through tolerant
//! helpers rather than derived structs.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::{Album, ArtistRef, Genre, Playlist, ReleaseType, SavedArtist, Track, calendar};

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
/// The size a genre image is asked for at. Tidal cuts genre art at 3:2 and has no square cut.
const GENRE: &str = "640x426";
/// The media tag of an album whose master Tidal serves in hi-res.
const HIRES: &str = "HIRES_LOSSLESS";

/// An album as a catalogue listing reads it, before [`distinct`] folds the masters of one
/// release into one.
pub struct Release {
    album: Album,
    twin: Twin,
    hires: bool,
    popularity: u64,
}

/// What two masters of one release share. The title, version and artist are compared without
/// case or spacing.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Twin {
    title: String,
    version: String,
    explicit: bool,
    main_artist: String,
    release_date: String,
    tracks: u64,
}

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

/// The items of a paged answer. Favorites and playlist items come wrapped in an object with
/// `item` beside `created` or `dateAdded`, and the wrapper is kept so the caller can read when
/// the item was added.
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
        // the album a Tidal track carries has no artists
        album_artist: String::new(),
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

/// A catalogue album with what [`distinct`] needs to tell the masters of one release apart.
pub fn release(wrapper: &Value) -> Option<Release> {
    let value = unwrap(wrapper);
    let album = album(wrapper)?;
    let main_artist = value
        .get("artist")
        .or_else(|| value.get("artists")?.as_array()?.first())
        .and_then(|artist| {
            artist
                .get("id")
                .and_then(id)
                .or_else(|| text(artist, "name").map(normalized))
        })
        .unwrap_or_default();
    let twin = Twin {
        title: normalized(text(value, "title").unwrap_or_default()),
        version: normalized(text(value, "version").unwrap_or_default()),
        explicit: value
            .get("explicit")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        main_artist,
        release_date: text(value, "releaseDate").unwrap_or_default().to_owned(),
        tracks: number(value, "numberOfTracks").unwrap_or(0),
    };
    let hires = value
        .get("mediaMetadata")
        .and_then(|metadata| metadata.get("tags"))
        .and_then(Value::as_array)
        .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some(HIRES)));
    Some(Release {
        album,
        twin,
        hires,
        popularity: number(value, "popularity").unwrap_or(0),
    })
}

pub fn release_list(page: &Value) -> Vec<Release> {
    items(page).iter().filter_map(release).collect()
}

/// The releases without the album `album_id` and every twin of it, for a rail beside that
/// album's own page.
pub fn without(releases: Vec<Release>, album_id: &str) -> Vec<Release> {
    let Some(twin) = releases
        .iter()
        .find(|release| release.album.id == album_id)
        .map(|release| release.twin.clone())
    else {
        return releases;
    };
    releases
        .into_iter()
        .filter(|release| release.twin != twin)
        .collect()
}

/// The albums of a catalogue listing with each release shown once. Tidal lists some releases
/// twice under different ids, one master in hi-res and one not, so of two that agree on every
/// field of [`Twin`] the hi-res one stays, then the more popular. The kept album takes the
/// place where the first of them appeared.
pub fn distinct(releases: Vec<Release>) -> Vec<Album> {
    let mut kept: Vec<Release> = Vec::with_capacity(releases.len());
    let mut seats: HashMap<Twin, usize> = HashMap::new();
    for release in releases {
        match seats.get(&release.twin) {
            Some(&seat) => {
                let held = &kept[seat];
                if (release.hires, release.popularity) > (held.hires, held.popularity) {
                    kept[seat] = release;
                }
            }
            None => {
                seats.insert(release.twin.clone(), kept.len());
                kept.push(release);
            }
        }
    }
    kept.into_iter().map(|release| release.album).collect()
}

/// An artist's portrait, or the album cover Tidal names in its place for an artist it keeps no
/// portrait of, which is what its own players draw then. Tidal serves a cover at both portrait
/// sizes.
pub fn portrait(artist: &Value, size: u32) -> Option<String> {
    let named = |key| text(artist, key).filter(|uuid| !uuid.is_empty());
    image(
        named("picture").or_else(|| named("selectedAlbumCoverFallback")),
        size,
    )
}

/// An artist out of a bare or wrapped artist object.
pub fn artist(wrapper: &Value) -> Option<SavedArtist> {
    let value = unwrap(wrapper);
    Some(SavedArtist {
        id: value.get("id").and_then(id)?,
        name: text(value, "name")?.to_owned(),
        cover: portrait(value, PORTRAIT),
        added_at: added(wrapper),
    })
}

pub fn artist_list(page: &Value) -> Vec<SavedArtist> {
    items(page).iter().filter_map(artist).collect()
}

/// A genre out of `/genres`. Its id is the path Tidal lists the genre's music under.
pub fn genre(value: &Value) -> Option<Genre> {
    Some(Genre {
        id: text(value, "path")?.to_owned(),
        name: text(value, "name")?.to_owned(),
        cover: text(value, "image").map(|uuid| {
            format!(
                "https://resources.tidal.com/images/{}/{GENRE}.jpg",
                uuid.replace('-', "/")
            )
        }),
    })
}

/// A playlist out of a bare or wrapped playlist object. `user_id` decides whether the
/// listener owns it. One of the listener's comes with no owner name, which [`credit`] fills.
pub fn playlist(wrapper: &Value, user_id: &str) -> Option<Playlist> {
    let value = unwrap(wrapper);
    let id = text(value, "uuid")?.to_owned();
    let creator = value.get("creator").unwrap_or(&Value::Null);
    let owner_id = creator.get("id").and_then(self::id).unwrap_or_default();
    // Tidal's own playlists name no creator, and a listener's often do not either.
    // `id` reads Tidal's editorial creator id, zero, as no id at all.
    let editorial = owner_id.is_empty();
    let owner = text(creator, "name")
        .or_else(|| editorial.then_some("TIDAL"))
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

/// Names the listener as the owner of a playlist of theirs whose creator Tidal left unnamed,
/// which it does for every playlist a listener makes.
pub fn credit(playlist: &mut Playlist, listener: &str) {
    if playlist.owned && playlist.owner.is_empty() {
        playlist.owner = listener.to_owned();
    }
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
        // A mix's listing carries no count, and only its page says how many tracks it holds.
        track_count: 0,
        modified_at: None,
    })
}

/// An artist biography without Tidal's link markup. A `[wimpLink artistId="…"]Name[/wimpLink]`
/// link keeps its name, and the `<br/>` breaks become new lines.
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
    // Most answers list every artist, and a few only name the main one.
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
                channel: false,
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

/// Text lowercased with its runs of whitespace made single spaces, for comparing names.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{PORTRAIT, PORTRAIT_LARGE, artist, portrait};

    #[test]
    fn an_artist_without_a_portrait_is_drawn_with_the_cover_tidal_names() {
        let both = json!({"id": 1, "name": "A", "picture": "aa-bb", "selectedAlbumCoverFallback": "cc-dd"});
        assert_eq!(
            portrait(&both, PORTRAIT).as_deref(),
            Some("https://resources.tidal.com/images/aa/bb/320x320.jpg")
        );
        let unpictured =
            json!({"id": 1, "name": "A", "picture": null, "selectedAlbumCoverFallback": "cc-dd"});
        assert_eq!(
            portrait(&unpictured, PORTRAIT_LARGE).as_deref(),
            Some("https://resources.tidal.com/images/cc/dd/750x750.jpg")
        );
        assert_eq!(
            artist(&unpictured)
                .and_then(|artist| artist.cover)
                .as_deref(),
            Some("https://resources.tidal.com/images/cc/dd/320x320.jpg")
        );
        assert_eq!(portrait(&json!({"id": 1, "picture": ""}), PORTRAIT), None);
    }
}
