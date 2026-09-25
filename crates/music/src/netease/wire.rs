//! Conversions from NetEase's wire shapes to the crate's models.
//!
//! Every record the site sends is read through [`Record`]. Its endpoints answer their own
//! dialect of the same record — `ar` or `artists`, `al` or `album`, `dt` or `duration`,
//! milliseconds here and seconds there — so the differences stay in one place rather than
//! being scattered through the client.

use std::time::Duration;

use serde_json::Value;

use crate::{Album, ArtistRef, GenreItem, Playlist, ReleaseType, SavedArtist, Track};

/// One NetEase JSON value with the readers and model conversions for the site's several shapes.
///
/// A record may be an object, a nested object, or one of the bare id values NetEase puts in an
/// id list. The accessors accept all of those forms where the endpoint does.
pub struct Record<'a> {
    value: &'a Value,
}

impl<'a> Record<'a> {
    /// Wraps one wire value without copying it.
    pub fn new(value: &'a Value) -> Self {
        Self { value }
    }

    /// The id this record carries, or the id a bare value is: the site writes a list of ids as
    /// loose numbers and a list of records as objects with an `id` under them. Zero and an empty
    /// string are never a record.
    pub fn id(&self) -> Option<String> {
        Self::read_id(self.value.get("id").unwrap_or(self.value))
    }

    /// The first non-empty string under any of `keys`.
    pub fn text(&self, keys: &[&str]) -> Option<&'a str> {
        keys.iter()
            .find_map(|key| self.value.get(*key))
            .and_then(Value::as_str)
    }

    /// The first number under any of `keys`, whether the site wrote it as a number, a float or
    /// a string. A key the record omits is skipped rather than ending the search.
    pub fn number(&self, keys: &[&str]) -> Option<u64> {
        for key in keys {
            let Some(field) = self.value.get(*key) else {
                continue;
            };
            if let Some(number) = field.as_u64() {
                return Some(number);
            }
            if let Some(number) = field.as_f64() {
                return Some(number.max(0.0) as u64);
            }
            if let Some(text) = field.as_str()
                && let Ok(number) = text.parse::<u64>()
            {
                return Some(number);
            }
        }
        None
    }

    /// A picture url at one size. NetEase hands covers out over plain http and resizes them on
    /// request, so this upgrades the scheme and adds the site's standard size query.
    pub fn picture(&self, keys: &[&str], size: u32) -> Option<String> {
        let url = self.text(keys)?.trim();
        if url.is_empty() {
            return None;
        }
        let mut url = match url.strip_prefix("http://") {
            Some(rest) => format!("https://{rest}"),
            None => url.to_owned(),
        };
        if !url.contains('?') {
            url.push_str(&format!("?param={size}y{size}"));
        }
        Some(url)
    }

    /// One track, from any endpoint that lists tracks.
    ///
    /// `playable` is the catalog's own answer, not this account's: a track the site has lost
    /// rights to is marked here, while one this account's plan cannot play is settled by the
    /// stream call.
    pub fn track(&self) -> Option<Track> {
        let track_id = self.id()?;
        let album = self.value.get("al").or_else(|| self.value.get("album"));
        let (artists, artist_refs) = self.artists();
        Some(Track {
            id: Some(track_id),
            name: self.text(&["name", "title"]).unwrap_or_default().to_owned(),
            playable: self.value.get("noCopyrightRcmd").is_none_or(Value::is_null),
            artists,
            artist_refs,
            album: album
                .and_then(|album| Record::new(album).text(&["name", "title"]))
                .unwrap_or_default()
                .to_owned(),
            album_id: album.and_then(|album| Record::new(album).id()),
            cover: album
                .and_then(|album| Record::new(album).picture(&["picUrl", "blurPicUrl"], 300)),
            duration: Duration::from_millis(self.number(&["dt", "duration"]).unwrap_or(0)),
            added_at: self.seconds(&["subscribedAt", "createTime"]),
            added_by: None,
            playcount: None,
            popularity: self.number(&["pop", "popularity"]).unwrap_or(0).min(100) as u32,
            // None of the endpoints this provider reads marks a track as explicit. The `mark`
            // field is a bitmask of the site's own and no part of it has been shown to mean that.
            explicit: false,
            track_number: self.number(&["no", "trackNumber"]).unwrap_or(0) as u32,
            disc_number: self.number(&["cd", "disc"]).unwrap_or(1).max(1) as u32,
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        })
    }

    /// Every track in a list under any of `keys`, in the order the endpoint listed them.
    pub fn tracks(&self, keys: &[&str]) -> Vec<Track> {
        keys.iter()
            .find_map(|key| self.value.get(*key).and_then(Value::as_array))
            .map(|list| {
                list.iter()
                    .filter_map(|value| Record::new(value).track())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every id in a list under any of `keys`. A playlist's `trackIds` holds records that carry
    /// the id beside their position in the list, while the liked set holds ids bare.
    pub fn ids(&self, keys: &[&str]) -> Vec<String> {
        keys.iter()
            .find_map(|key| self.value.get(*key).and_then(Value::as_array))
            .map(|list| {
                list.iter()
                    .filter_map(|entry| Record::new(entry.get("id").unwrap_or(entry)).id())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One album, from a search hit, the newest list or a collected album.
    pub fn album(&self) -> Option<Album> {
        let album_id = self.id()?;
        let (artists, artist_refs) = self.artists();
        let publish = self.number(&["publishTime"]).unwrap_or(0);
        let (year, _, _) = Self::date_of(publish);
        Some(Album {
            id: album_id,
            name: self.text(&["name"]).unwrap_or_default().to_owned(),
            artists,
            artist_refs,
            cover: self.picture(&["picUrl", "blurPicUrl"], 300),
            cover_large: self.picture(&["picUrl", "blurPicUrl"], 1000),
            release_type: self.release_type(),
            year: year as i32,
            track_count: self.number(&["size", "trackCount"]).unwrap_or(0) as u32,
            release_date: Self::release_date(publish),
            label: self.text(&["company"]).unwrap_or_default().to_owned(),
            copyrights: Vec::new(),
            added_at: self.seconds(&["subTime", "createTime"]),
        })
    }

    /// One playlist, from the account's own list, a search hit or a personalized shelf.
    pub fn playlist(&self, user_id: &str) -> Option<Playlist> {
        let playlist_id = self.id()?;
        let creator = self
            .value
            .get("creator")
            .filter(|creator| !creator.is_null());
        let owner_id = creator
            .and_then(|creator| creator.get("userId"))
            .and_then(Self::read_id)
            .or_else(|| self.value.get("userId").and_then(Self::read_id))
            .unwrap_or_default();
        let owner = creator
            .and_then(|creator| Record::new(creator).text(&["nickname"]))
            .or_else(|| self.text(&["officialPlaylistTitle"]))
            .unwrap_or_default()
            .to_owned();
        Some(Playlist {
            id: playlist_id,
            name: self.text(&["name"]).unwrap_or_default().to_owned(),
            owner,
            owned: !user_id.is_empty() && owner_id == user_id,
            owner_id,
            collaborative: false,
            blend: false,
            // NetEase keeps `privacy` at 0 for a public playlist and 10 for a private one.
            public: self.number(&["privacy"]).unwrap_or(0) == 0,
            cover: self.picture(&["coverImgUrl", "picUrl"], 300),
            track_count: self.number(&["trackCount", "trackNumber"]).unwrap_or(0) as u32,
            modified_at: self.seconds(&["updateTime"]),
        })
    }

    /// One artist the account follows.
    pub fn saved_artist(&self) -> Option<SavedArtist> {
        let artist_id = self.id()?;
        Some(SavedArtist {
            id: artist_id,
            name: self.text(&["name"]).unwrap_or_default().to_owned(),
            cover: self.picture(&["picUrl", "img1v1Url"], 300),
            added_at: self.seconds(&["subTime", "createTime"]),
        })
    }

    /// One card of a home shelf that holds tracks, from a listing that nests the record under
    /// `song` — the shape the new-song shelf answers in.
    pub fn item(&self) -> Option<GenreItem> {
        let song = self.value.get("song").filter(|song| !song.is_null());
        Record::new(song.unwrap_or(self.value))
            .track()
            .map(GenreItem::Track)
    }

    fn read_id(value: &Value) -> Option<String> {
        let id = value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_u64().map(|number| number.to_string()))?;
        (!id.is_empty() && id != "0").then_some(id)
    }

    fn seconds(&self, keys: &[&str]) -> Option<i64> {
        self.number(keys).map(|stamp| (stamp / 1000) as i64)
    }

    fn release_type(&self) -> ReleaseType {
        // The site writes Single and EP in English, and 专辑 and 精选集 in Chinese. `subType`
        // is a different axis (studio, live or remix), so it is deliberately not read here.
        match self.text(&["type"]).unwrap_or_default() {
            "单曲" | "Single" => ReleaseType::Single,
            "EP" | "Ep" => ReleaseType::Ep,
            "精选集" | "Compilation" => ReleaseType::Compilation,
            _ => ReleaseType::Album,
        }
    }

    fn artists(&self) -> (String, Vec<ArtistRef>) {
        let list = self
            .value
            .get("ar")
            .or_else(|| self.value.get("artists"))
            .and_then(Value::as_array);
        if let Some(list) = list {
            let refs: Vec<ArtistRef> = list
                .iter()
                .filter_map(|artist| match artist {
                    Value::String(name) => Some(ArtistRef {
                        name: name.clone(),
                        id: None,
                    }),
                    artist => {
                        let artist = Record::new(artist);
                        let name = artist.text(&["name"])?.to_owned();
                        Some(ArtistRef {
                            name,
                            id: artist.id(),
                        })
                    }
                })
                .collect();
            if !refs.is_empty() {
                let names = refs
                    .iter()
                    .map(|artist| artist.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                return (names, refs);
            }
        }
        let Some(artist) = self.value.get("artist").filter(|artist| !artist.is_null()) else {
            return (String::new(), Vec::new());
        };
        let artist = Record::new(artist);
        let name = artist.text(&["name"]).unwrap_or_default().to_owned();
        match name.is_empty() {
            true => (String::new(), Vec::new()),
            false => (
                name.clone(),
                vec![ArtistRef {
                    name,
                    id: artist.id(),
                }],
            ),
        }
    }
    /// Milliseconds since the epoch as `(year, month, day)`.
    fn date_of(stamp: u64) -> (i64, u32, u32) {
        let days = (stamp / 86_400_000) as i64;
        Self::civil_from_days(days)
    }

    /// Howard Hinnant's days-to-civil conversion.
    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let days = days + 719_468;
        let era = days.div_euclid(146_097);
        let doe = days.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = (mp + if mp < 10 { 3 } else { -9 }) as u32;
        (year + i64::from(month <= 2), month, day)
    }

    fn release_date(stamp: u64) -> String {
        match stamp {
            0 => String::new(),
            stamp => {
                let (year, month, day) = Self::date_of(stamp);
                format!("{year:04}-{month:02}-{day:02}")
            }
        }
    }
}
