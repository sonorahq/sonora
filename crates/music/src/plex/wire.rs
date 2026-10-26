//! The JSON a Plex Media Server and plex.tv answer with. Plex describes artists, albums, tracks
//! and playlists with one shape, so one [`Item`] holds all four and each reader takes the
//! fields its kind has.

use serde::{Deserialize, Deserializer};

/// Every answer of a media server is one `MediaContainer`.
#[derive(Debug, Default, Deserialize)]
pub struct Envelope {
    #[serde(rename = "MediaContainer", default)]
    pub container: Container,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Container {
    /// How many items the whole listing has, which only a paged listing says.
    pub total_size: Option<usize>,
    pub size: Option<usize>,
    #[serde(rename = "Metadata")]
    pub metadata: Vec<Item>,
    #[serde(rename = "Directory")]
    pub directory: Vec<Directory>,
}

/// A library section, or one entry of a filter listing such as the genres.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Directory {
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// An artist, an album, a track or a playlist.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub rating_key: String,
    pub title: String,
    pub summary: Option<String>,
    pub thumb: Option<String>,
    /// The picture a playlist is drawn as.
    pub composite: Option<String>,
    pub parent_rating_key: Option<String>,
    pub parent_title: Option<String>,
    pub parent_thumb: Option<String>,
    pub grandparent_rating_key: Option<String>,
    pub grandparent_title: Option<String>,
    pub grandparent_thumb: Option<String>,
    /// The artist a track is credited to when that is not its album's artist.
    pub original_title: Option<String>,
    /// The track number of a track, or the position in the library of anything else.
    pub index: Option<u32>,
    /// The disc number of a track.
    pub parent_index: Option<u32>,
    /// Milliseconds.
    pub duration: Option<u64>,
    pub year: Option<i32>,
    pub added_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub last_viewed_at: Option<i64>,
    pub leaf_count: Option<u32>,
    #[serde(deserialize_with = "number")]
    pub user_rating: Option<f32>,
    pub view_count: Option<u64>,
    pub studio: Option<String>,
    pub originally_available_at: Option<String>,
    #[serde(rename = "playlistItemID")]
    pub playlist_item_id: Option<i64>,
    #[serde(rename = "Media")]
    pub media: Vec<Media>,
    #[serde(rename = "Format")]
    pub format: Vec<Tag>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Tag {
    pub tag: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Media {
    #[serde(rename = "Part")]
    pub part: Vec<Part>,
}

/// One file of a track.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Part {
    /// Where the file is served from, on the server.
    pub key: String,
    #[serde(rename = "Stream")]
    pub stream: Vec<Stream>,
}

/// One stream of a file: the audio, or the lyrics that belong to it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stream {
    pub id: Option<u64>,
    /// 2 is audio and 4 is lyrics.
    pub stream_type: Option<u32>,
    /// `lrc` for timed lyrics, `txt` for plain ones.
    pub format: Option<String>,
    #[serde(deserialize_with = "number")]
    pub loudness: Option<f32>,
    #[serde(deserialize_with = "number")]
    pub gain: Option<f32>,
    #[serde(deserialize_with = "number")]
    pub peak: Option<f32>,
    #[serde(deserialize_with = "number")]
    pub album_gain: Option<f32>,
    #[serde(deserialize_with = "number")]
    pub album_peak: Option<f32>,
}

/// What `plex.tv` hands out for a sign-in that is waiting for the user.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub id: u64,
    pub code: String,
    /// Set once the user has approved the pin.
    pub auth_token: Option<String>,
}

/// A server or a player of the account.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resource {
    pub name: String,
    /// What the device is, comma separated, `server` among them for a media server.
    pub provides: String,
    pub client_identifier: String,
    /// The token that opens this one server, which a shared server has apart from the account.
    pub access_token: Option<String>,
    pub owned: bool,
    pub source_title: Option<String>,
    pub connections: Vec<Connection>,
}

/// One address a server can be reached at.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Connection {
    pub uri: String,
    pub local: bool,
    pub relay: bool,
    #[serde(rename = "IPv6")]
    pub ipv6: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Account {
    pub id: u64,
    pub username: String,
    pub title: String,
    pub thumb: Option<String>,
}

/// A number that Plex sometimes writes as a string, as it does for stream measurements.
fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f32>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_f64().map(|number| number as f32),
        Some(serde_json::Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    })
}
