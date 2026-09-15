//! Apple Music's JSON turned into Sonora's models.
//!
//! Every resource arrives in the same envelope: an `id`, a `type`, an `attributes` object and
//! sometimes `relationships`. A library resource carries its own id (`i.`, `l.`, `r.`, `p.`)
//! and, when it was asked for with `include=catalog`, the catalog resource it came from. Only
//! the catalog id can be played or opened, so a library row that has none is dropped.
//!
//! Nothing here fails the caller: a row that cannot be read is left out, which is what keeps one
//! odd entry in a library of thousands from emptying a page.

use std::time::Duration;

use serde_json::Value;

use crate::{
    Album, Artist, ArtistProfile, ArtistRef, LibraryItem, LibraryItemKind, Playlist, ReleaseType,
    SavedArtist, Track,
};

/// The size artwork is asked to render at, and the larger one for a page header.
pub const ART: u32 = 600;
pub const HERO: u32 = 1200;

/// The catalog resource behind a library row, when it was included.
pub fn catalog(value: &Value) -> Option<&Value> {
    value.pointer("/relationships/catalog/data/0")
}

/// A non-empty string attribute.
pub fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|found| !found.is_empty())
        .map(str::to_owned)
}

fn number(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32
}

/// Apple hands out artwork as a template with the size left open. The query string of a
/// generated cover carries a signature, so only the placeholders are touched.
pub fn artwork(attributes: &Value, size: u32) -> Option<String> {
    let template = attributes.pointer("/artwork/url")?.as_str()?;
    Some(
        template
            .replace("{w}", &size.to_string())
            .replace("{h}", &size.to_string())
            .replace("{f}", "jpg")
            .replace("{c}", ""),
    )
}

/// An ISO-8601 stamp as seconds since the epoch. Apple writes them two ways, `2026-09-15` and
/// `2026-09-15T12:03:39Z`, and only the date part matters for sorting a library.
pub fn moment(attributes: &Value, key: &str) -> Option<i64> {
    let stamp = attributes.get(key)?.as_str()?;
    let (date, time) = match stamp.split_once('T') {
        Some((date, time)) => (date, time),
        None => (stamp, ""),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let day: i64 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let mut clock = time.trim_end_matches('Z').split(':');
    let hour: i64 = clock.next().unwrap_or("0").parse().unwrap_or(0);
    let minute: i64 = clock.next().unwrap_or("0").parse().unwrap_or(0);
    let second: i64 = clock
        .next()
        .and_then(|second| second.split('.').next())
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    Some(days(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days from the epoch to a civil date, by Howard Hinnant's algorithm.
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The artists of a resource. Apple only names them in one string unless the relationship was
/// asked for, and without an id an artist is not somewhere the app can go.
pub fn artist_refs(value: &Value, attributes: &Value) -> Vec<ArtistRef> {
    let named = text(attributes, "artistName").unwrap_or_default();
    let listed: Vec<ArtistRef> = value
        .pointer("/relationships/artists/data")
        .and_then(Value::as_array)
        .map(|artists| {
            artists
                .iter()
                .filter_map(|artist| {
                    Some(ArtistRef {
                        name: text(artist.get("attributes")?, "name")?,
                        id: artist.get("id").and_then(Value::as_str).map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    match listed.is_empty() {
        true => vec![ArtistRef {
            name: named,
            id: None,
        }],
        false => listed,
    }
}

/// One catalog song.
pub fn song(value: &Value) -> Option<Track> {
    let id = value.get("id")?.as_str()?;
    let attributes = value.get("attributes")?;
    Some(Track {
        id: Some(id.to_owned()),
        name: text(attributes, "name")?,
        // Without play parameters a song is listed but not streamable here.
        playable: attributes.get("playParams").is_some(),
        artists: text(attributes, "artistName").unwrap_or_default(),
        artist_refs: artist_refs(value, attributes),
        album: text(attributes, "albumName").unwrap_or_default(),
        album_id: value
            .pointer("/relationships/albums/data/0/id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        cover: artwork(attributes, ART),
        duration: Duration::from_millis(
            attributes
                .get("durationInMillis")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        added_at: moment(attributes, "dateAdded"),
        added_by: None,
        playcount: None,
        popularity: 0,
        explicit: text(attributes, "contentRating").as_deref() == Some("explicit"),
        track_number: number(attributes, "trackNumber"),
        disc_number: number(attributes, "discNumber"),
        tags: attributes
            .get("genreNames")
            .and_then(Value::as_array)
            .map(|genres| {
                genres
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

/// One library song, which plays through the catalog id its play parameters name. A library row
/// without one is an upload, and this path has no way to play it.
pub fn library_song(value: &Value) -> Option<Track> {
    let id = value
        .pointer("/attributes/playParams/catalogId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            catalog(value)?
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })?;
    // The catalog copy is richer when it came along, and the library copy is what carries the
    // date it was added.
    let mut track = match catalog(value) {
        Some(found) => song(found)?,
        None => song(value)?,
    };
    track.id = Some(id);
    track.added_at = value
        .pointer("/attributes")
        .and_then(|attributes| moment(attributes, "dateAdded"))
        .or(track.added_at);
    Some(track)
}

/// One catalog album.
pub fn album(value: &Value) -> Option<Album> {
    let id = value.get("id")?.as_str()?;
    let attributes = value.get("attributes")?;
    let release = text(attributes, "releaseDate").unwrap_or_default();
    Some(Album {
        id: id.to_owned(),
        name: text(attributes, "name")?,
        artists: text(attributes, "artistName").unwrap_or_default(),
        artist_refs: artist_refs(value, attributes),
        cover: artwork(attributes, ART),
        cover_large: artwork(attributes, HERO),
        release_type: release_type(attributes),
        year: release
            .get(..4)
            .and_then(|year| year.parse().ok())
            .unwrap_or(0),
        track_count: number(attributes, "trackCount"),
        release_date: release,
        label: text(attributes, "recordLabel").unwrap_or_default(),
        copyrights: text(attributes, "copyright").into_iter().collect(),
        added_at: moment(attributes, "dateAdded"),
    })
}

/// One library album, which opens through the catalog release it came from.
pub fn library_album(value: &Value) -> Option<Album> {
    let id = catalog(value)?.get("id")?.as_str()?.to_owned();
    let mut album = album(catalog(value)?).or_else(|| album(value))?;
    album.id = id;
    album.added_at = value
        .pointer("/attributes")
        .and_then(|attributes| moment(attributes, "dateAdded"))
        .or(album.added_at);
    Some(album)
}

/// What Apple calls the release, as far as the model has a name for it.
fn release_type(attributes: &Value) -> ReleaseType {
    let single = attributes
        .get("isSingle")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let compilation = attributes
        .get("isCompilation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let tracks = number(attributes, "trackCount");
    match (single, compilation, tracks) {
        (true, _, _) => ReleaseType::Single,
        (_, true, _) => ReleaseType::Compilation,
        (_, _, 2..=6) => ReleaseType::Ep,
        _ => ReleaseType::Album,
    }
}

/// One catalog playlist, which is Apple's or a curator's rather than the listener's.
pub fn playlist(value: &Value) -> Option<Playlist> {
    let id = value.get("id")?.as_str()?;
    let attributes = value.get("attributes")?;
    let curator = text(attributes, "curatorName").unwrap_or_else(|| "Apple Music".to_owned());
    Some(Playlist {
        id: id.to_owned(),
        name: text(attributes, "name")?,
        owner: curator.clone(),
        owner_id: curator,
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover: artwork(attributes, ART),
        track_count: value
            .pointer("/relationships/tracks/data")
            .and_then(Value::as_array)
            .map(|tracks| tracks.len() as u32)
            .unwrap_or(0),
        modified_at: moment(attributes, "lastModifiedDate"),
    })
}

/// One of the listener's own playlists.
pub fn library_playlist(value: &Value, owner: &str) -> Option<Playlist> {
    let id = value.get("id")?.as_str()?;
    let attributes = value.get("attributes")?;
    Some(Playlist {
        id: id.to_owned(),
        name: text(attributes, "name")?,
        owner: owner.to_owned(),
        owner_id: owner.to_owned(),
        owned: attributes
            .get("canEdit")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        collaborative: attributes
            .get("hasCollaboration")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        blend: false,
        public: attributes
            .get("isPublic")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        cover: artwork(attributes, ART),
        track_count: 0,
        modified_at: moment(attributes, "lastModifiedDate").or(moment(attributes, "dateAdded")),
    })
}

/// One library artist. Only the catalog counterpart has a picture, and only its id can open a
/// page, so an artist without one is left out.
pub fn saved_artist(value: &Value) -> Option<SavedArtist> {
    let found = catalog(value)?;
    let attributes = found.get("attributes")?;
    Some(SavedArtist {
        id: found.get("id")?.as_str()?.to_owned(),
        name: text(attributes, "name").or_else(|| text(value.get("attributes")?, "name"))?,
        cover: artwork(attributes, ART),
        added_at: value
            .get("attributes")
            .and_then(|attributes| moment(attributes, "dateAdded")),
    })
}

/// A catalog artist page, with whatever its views brought along.
pub fn artist(value: &Value) -> Option<Artist> {
    let attributes = value.get("attributes")?;
    Some(Artist {
        name: text(attributes, "name")?,
        cover_large: artwork(attributes, HERO),
        biography: text(attributes, "editorialNotes")
            .or_else(|| text(attributes.get("editorialNotes")?, "standard")),
        monthly_listeners: None,
        top_tracks: view(value, "top-songs").iter().filter_map(song).collect(),
        albums: view(value, "full-albums")
            .iter()
            .chain(view(value, "singles").iter())
            .filter_map(album)
            .collect(),
    })
}

pub fn artist_profile(value: &Value) -> Option<ArtistProfile> {
    let attributes = value.get("attributes")?;
    Some(ArtistProfile {
        name: text(attributes, "name")?,
        cover_large: artwork(attributes, HERO),
        biography: attributes
            .get("editorialNotes")
            .and_then(|notes| text(notes, "standard").or_else(|| text(notes, "short"))),
    })
}

/// The rows of one named view of a resource, which is how an artist page carries its songs and
/// releases.
pub fn view<'a>(value: &'a Value, name: &str) -> &'a [Value] {
    value
        .pointer("/views")
        .and_then(|views| views.get(name))
        .and_then(|view| view.get("data"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// One row of the mixed library landing, whatever kind of thing it is.
///
/// The uri is Spotify-shaped on purpose: a sidebar pin is built by taking what follows the last
/// colon, so an id on its own could never become one.
pub fn library_item(value: &Value, owner: &str) -> Option<LibraryItem> {
    let kind = value.get("type")?.as_str()?;
    let attributes = value.get("attributes")?;
    let (kind, uri, subtitle) = match kind {
        "library-albums" | "albums" => (
            LibraryItemKind::Album,
            catalog(value)
                .and_then(|found| found.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            text(attributes, "artistName").unwrap_or_default(),
        ),
        "library-playlists" | "playlists" => (
            LibraryItemKind::Playlist,
            value.get("id").and_then(Value::as_str).map(str::to_owned),
            text(attributes, "curatorName").unwrap_or_else(|| owner.to_owned()),
        ),
        "library-artists" | "artists" => (
            LibraryItemKind::Artist,
            catalog(value)
                .and_then(|found| found.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            String::new(),
        ),
        _ => return None,
    };
    let part = match kind {
        LibraryItemKind::Album => "album",
        LibraryItemKind::Artist => "artist",
        _ => "playlist",
    };
    Some(LibraryItem {
        uri: format!("apple:{part}:{}", uri?),
        name: text(attributes, "name")?,
        subtitle,
        cover: artwork(attributes, ART)
            .or_else(|| artwork(catalog(value)?.get("attributes")?, ART)),
        kind,
        pinned: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_song() -> Value {
        serde_json::json!({
            "id": "1440857781",
            "type": "songs",
            "attributes": {
                "name": "Blue Monday",
                "artistName": "New Order",
                "albumName": "Substance",
                "durationInMillis": 447_000u64,
                "trackNumber": 3,
                "discNumber": 1,
                "contentRating": "explicit",
                "genreNames": ["Alternative", "Music"],
                "playParams": { "id": "1440857781", "kind": "song" },
                "artwork": { "url": "https://is1.mzstatic.com/a/{w}x{h}bb.{f}" }
            },
            "relationships": {
                "artists": { "data": [{ "id": "555", "attributes": { "name": "New Order" } }] },
                "albums": { "data": [{ "id": "888" }] }
            }
        })
    }

    #[test]
    fn reads_a_catalog_song() {
        let track = song(&catalog_song()).unwrap();
        assert_eq!(track.id.as_deref(), Some("1440857781"));
        assert_eq!(track.name, "Blue Monday");
        assert_eq!(track.album_id.as_deref(), Some("888"));
        assert_eq!(track.duration, Duration::from_millis(447_000));
        assert!(track.explicit);
        assert!(track.playable);
        assert_eq!(track.tags, vec!["Alternative", "Music"]);
        assert_eq!(
            track.cover.as_deref(),
            Some("https://is1.mzstatic.com/a/600x600bb.jpg")
        );
        // The relationship is what makes an artist clickable.
        assert_eq!(track.artist_refs[0].id.as_deref(), Some("555"));
    }

    #[test]
    fn a_song_with_no_play_parameters_is_not_playable() {
        let mut row = catalog_song();
        row["attributes"]["playParams"] = Value::Null;
        row["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("playParams");
        assert!(!song(&row).unwrap().playable);
    }

    #[test]
    fn a_library_song_plays_through_its_catalog_id() {
        let row = serde_json::json!({
            "id": "i.abc123",
            "type": "library-songs",
            "attributes": {
                "name": "Blue Monday",
                "artistName": "New Order",
                "dateAdded": "2025-02-13T10:04:40Z",
                "playParams": { "catalogId": "1440857781", "isLibrary": true }
            }
        });
        let track = library_song(&row).unwrap();
        assert_eq!(track.id.as_deref(), Some("1440857781"));
        assert_eq!(track.added_at, Some(1_739_441_080));
    }

    #[test]
    fn an_upload_with_no_catalog_id_is_dropped() {
        let row = serde_json::json!({
            "id": "i.abc123",
            "type": "library-songs",
            "attributes": { "name": "Home Recording" }
        });
        assert!(library_song(&row).is_none());
    }

    /// The library row carries the date, the catalog row everything else.
    #[test]
    fn a_library_album_takes_the_catalog_id_and_keeps_its_date() {
        let row = serde_json::json!({
            "id": "l.xyz",
            "type": "library-albums",
            "attributes": { "name": "Substance", "artistName": "New Order", "dateAdded": "2025-10-14" },
            "relationships": { "catalog": { "data": [{
                "id": "617154241",
                "attributes": { "name": "Substance", "artistName": "New Order", "trackCount": 24, "releaseDate": "1987-08-17" }
            }] } }
        });
        let album = library_album(&row).unwrap();
        assert_eq!(album.id, "617154241");
        assert_eq!(album.year, 1987);
        assert_eq!(album.track_count, 24);
        assert_eq!(album.added_at, Some(1_760_400_000));
    }

    #[test]
    fn dates_come_back_as_seconds() {
        let attributes = serde_json::json!({
            "day": "1970-01-01",
            "plain": "2026-09-15",
            "stamped": "2026-09-15T12:03:39Z",
            "fractional": "2026-09-15T12:03:39.456Z",
            "rubbish": "not a date"
        });
        assert_eq!(moment(&attributes, "day"), Some(0));
        assert_eq!(moment(&attributes, "plain"), Some(1_789_430_400));
        assert_eq!(moment(&attributes, "stamped"), Some(1_789_473_819));
        assert_eq!(moment(&attributes, "fractional"), Some(1_789_473_819));
        assert_eq!(moment(&attributes, "rubbish"), None);
        assert_eq!(moment(&attributes, "missing"), None);
    }

    #[test]
    fn the_track_count_decides_an_ep_from_an_album() {
        let ep = serde_json::json!({ "trackCount": 4 });
        let album = serde_json::json!({ "trackCount": 12 });
        let single = serde_json::json!({ "trackCount": 1, "isSingle": true });
        assert_eq!(release_type(&ep), ReleaseType::Ep);
        assert_eq!(release_type(&album), ReleaseType::Album);
        assert_eq!(release_type(&single), ReleaseType::Single);
    }

    #[test]
    fn a_library_playlist_is_the_listeners_own() {
        let row = serde_json::json!({
            "id": "p.abc",
            "attributes": {
                "name": "Classical",
                "canEdit": true,
                "isPublic": false,
                "lastModifiedDate": "2026-05-12T22:57:25Z"
            }
        });
        let playlist = library_playlist(&row, "Me").unwrap();
        assert_eq!(playlist.id, "p.abc");
        assert!(playlist.owned);
        assert!(!playlist.public);
        assert_eq!(playlist.owner, "Me");
    }

    #[test]
    fn an_artist_page_reads_its_views() {
        let row = serde_json::json!({
            "id": "5468295",
            "attributes": {
                "name": "Daft Punk",
                "artwork": { "url": "https://is1.mzstatic.com/a/{w}x{h}bb.jpg" },
                "editorialNotes": { "standard": "Robots." }
            },
            "views": {
                "top-songs": { "data": [catalog_song()] },
                "full-albums": { "data": [{
                    "id": "617154241",
                    "attributes": { "name": "Random Access Memories", "artistName": "Daft Punk", "trackCount": 13 }
                }] }
            }
        });
        let artist = artist(&row).unwrap();
        assert_eq!(artist.name, "Daft Punk");
        assert_eq!(artist.biography.as_deref(), Some("Robots."));
        assert_eq!(artist.top_tracks.len(), 1);
        assert_eq!(artist.albums.len(), 1);
        assert_eq!(
            artist.cover_large.as_deref(),
            Some("https://is1.mzstatic.com/a/1200x1200bb.jpg")
        );
    }

    #[test]
    fn a_library_row_becomes_a_landing_item() {
        let row = serde_json::json!({
            "id": "l.xyz",
            "type": "library-albums",
            "attributes": { "name": "Omen", "artistName": "Of Virtue",
                            "artwork": { "url": "https://is1.mzstatic.com/a/{w}x{h}bb.jpg" } },
            "relationships": { "catalog": { "data": [{ "id": "1691419979", "attributes": {} }] } }
        });
        let item = library_item(&row, "Me").unwrap();
        assert_eq!(item.uri, "apple:album:1691419979");
        assert_eq!(item.kind, LibraryItemKind::Album);
        assert_eq!(item.subtitle, "Of Virtue");
    }

    #[test]
    fn a_kind_the_model_has_no_place_for_is_dropped() {
        let row = serde_json::json!({
            "type": "library-music-videos",
            "attributes": { "name": "A Video" }
        });
        assert!(library_item(&row, "Me").is_none());
    }
}
