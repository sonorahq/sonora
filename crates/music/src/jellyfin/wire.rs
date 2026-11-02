//! What Jellyfin answers with, and how it becomes Sonora's models.
//!
//! Every field is optional or defaulted, so an item the server describes in less detail than
//! expected still reads, with the missing parts left empty, rather than failing the whole page.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::escape;
use crate::{
    Album, ArtistRef, Playlist, ReleaseType, SavedArtist, Track, UserProfile, iso_8601_to_epoch,
};

/// Jellyfin counts time in ticks of 100 nanoseconds.
const TICKS_PER_MICRO: u64 = 10;
/// The edge of a list cover, in pixels.
pub const COVER: u32 = 300;
/// The edge of a large cover, wide enough for the fullscreen player on a 2x display.
pub const COVER_LARGE: u32 = 1200;

/// The answer to `POST /Users/AuthenticateByName`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Authentication {
    pub user: Option<User>,
    pub access_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct User {
    pub id: Option<String>,
    pub name: Option<String>,
}

/// One page of `GET /Users/{userId}/Items`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Items {
    pub items: Vec<Item>,
    pub total_record_count: Option<usize>,
}

/// The parts of a `BaseItemDto` that Sonora reads, for tracks, albums, artists and playlists
/// alike.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artist: Option<String>,
    pub album_artists: Vec<NameId>,
    pub artist_items: Vec<NameId>,
    pub artists: Vec<String>,
    pub run_time_ticks: Option<u64>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    pub production_year: Option<i32>,
    pub premiere_date: Option<String>,
    pub date_created: Option<String>,
    pub child_count: Option<u32>,
    pub song_count: Option<u32>,
    pub image_tags: HashMap<String, String>,
    pub album_primary_image_tag: Option<String>,
    pub normalization_gain: Option<f32>,
    pub overview: Option<String>,
    pub user_data: Option<UserData>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct NameId {
    pub name: Option<String>,
    pub id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserData {
    pub play_count: Option<u64>,
}

impl Item {
    /// Whether the server lists this item as an audio track, the only kind Sonora plays.
    pub fn is_audio(&self) -> bool {
        self.kind.as_deref() == Some("Audio")
    }

    fn duration(&self) -> Duration {
        Duration::from_micros(self.run_time_ticks.unwrap_or(0) / TICKS_PER_MICRO)
    }

    fn primary_tag(&self) -> Option<&str> {
        self.image_tags
            .get("Primary")
            .map(String::as_str)
            .filter(|tag| !tag.is_empty())
    }
}

/// Builds the cover urls of one server. Jellyfin serves item images without a token, so a cover
/// url carries nothing secret and stays the same across launches, which lets every image cache
/// keep it.
#[derive(Clone, Debug)]
pub struct Covers {
    server: String,
}

impl Covers {
    pub fn new(server: &str) -> Self {
        Self {
            server: server.trim_end_matches('/').to_owned(),
        }
    }

    /// `GET /Items/{itemId}/Images/Primary`, scaled down to `size`. The tag names the image's
    /// version, so a replaced cover gets a new url.
    pub fn url(&self, id: &str, tag: &str, size: u32) -> String {
        format!(
            "{}/Items/{}/Images/Primary?maxWidth={size}&maxHeight={size}&tag={}",
            self.server,
            escape::component(id),
            escape::component(tag),
        )
    }

    /// The item's own picture, or for a track without one, its album's.
    fn of(&self, item: &Item, size: u32) -> Option<String> {
        if let Some(tag) = item.primary_tag() {
            return Some(self.url(&item.id, tag, size));
        }
        let album = item.album_id.as_deref().filter(|id| !id.is_empty())?;
        let tag = item
            .album_primary_image_tag
            .as_deref()
            .filter(|tag| !tag.is_empty())?;
        Some(self.url(album, tag, size))
    }

    pub fn track(&self, item: Item) -> Track {
        let cover = self.of(&item, COVER);
        let (artists, artist_refs) = artists_of(&item.artist_items, &item.artists);
        Track {
            id: Some(item.id.clone()),
            name: item.name.clone().unwrap_or_default(),
            playable: true,
            artists,
            artist_refs,
            album: item.album.clone().unwrap_or_default(),
            album_id: item.album_id.clone().filter(|id| !id.is_empty()),
            cover,
            duration: item.duration(),
            added_at: iso_8601_to_epoch(item.date_created.as_deref()),
            added_by: None,
            playcount: item.user_data.as_ref().and_then(|data| data.play_count),
            popularity: 0,
            explicit: false,
            track_number: item.index_number.unwrap_or(0),
            disc_number: item.parent_index_number.unwrap_or(1).max(1),
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        }
    }

    pub fn album(&self, item: Item) -> Album {
        let cover = self.of(&item, COVER);
        let cover_large = self.of(&item, COVER_LARGE);
        let names = item.album_artist.iter().cloned().collect::<Vec<_>>();
        let (artists, artist_refs) = match item.album_artists.is_empty() {
            true => artists_of(&item.artist_items, &names),
            false => artists_of(&item.album_artists, &names),
        };
        let year = item.production_year.unwrap_or(0);
        Album {
            id: item.id,
            name: item.name.unwrap_or_default(),
            artists,
            artist_refs,
            cover,
            cover_large,
            release_type: ReleaseType::Album,
            year,
            track_count: item.song_count.or(item.child_count).unwrap_or(0),
            release_date: item
                .premiere_date
                .as_deref()
                .and_then(|date| date.get(..10))
                .map(str::to_owned)
                .unwrap_or_else(|| match year {
                    0 => String::new(),
                    _ => year.to_string(),
                }),
            label: String::new(),
            copyrights: Vec::new(),
            added_at: iso_8601_to_epoch(item.date_created.as_deref()),
        }
    }

    pub fn artist(&self, item: Item) -> SavedArtist {
        SavedArtist {
            cover: self.of(&item, COVER),
            id: item.id,
            name: item.name.unwrap_or_default(),
            added_at: None,
        }
    }

    pub fn artist_large(&self, item: &Item) -> Option<String> {
        self.of(item, COVER_LARGE)
    }

    /// A playlist as the server lists it. Jellyfin lists only the playlists the user may see,
    /// and the first version only reads them, so none counts as the user's own to change.
    pub fn playlist(&self, item: Item) -> Playlist {
        Playlist {
            cover: self.of(&item, COVER),
            id: item.id,
            name: item.name.unwrap_or_default(),
            owner: String::new(),
            owner_id: String::new(),
            owned: false,
            collaborative: false,
            blend: false,
            public: false,
            track_count: item.child_count.or(item.song_count).unwrap_or(0),
            modified_at: None,
        }
    }
}

pub fn profile(user_id: &str, username: &str) -> UserProfile {
    UserProfile {
        id: user_id.to_owned(),
        display_name: username.to_owned(),
        avatar: None,
    }
}

/// The display line and the followable artists of an item. Jellyfin lists artists with ids
/// where it has matched them to an artist item, and as bare names otherwise.
fn artists_of(items: &[NameId], names: &[String]) -> (String, Vec<ArtistRef>) {
    let refs: Vec<ArtistRef> = match items.is_empty() {
        false => items
            .iter()
            .filter_map(|artist| {
                let name = artist.name.clone().filter(|name| !name.is_empty())?;
                Some(ArtistRef {
                    name,
                    id: artist.id.clone().filter(|id| !id.is_empty()),
                })
            })
            .collect(),
        true => names
            .iter()
            .filter(|name| !name.is_empty())
            .map(|name| ArtistRef {
                name: name.clone(),
                id: None,
            })
            .collect(),
    };
    let line = refs
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    (line, refs)
}
