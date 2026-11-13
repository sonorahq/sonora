use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context as _, Result};
use librespot_core::{Session, SpotifyUri};
use serde::Deserialize;

use crate::Track;
use crate::spotify::pace::paced;
use crate::spotify::playlists;

const TRACK_PREFIX: &str = "spotify:track:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
/// How many stations are remembered before they are forgotten and fetched again.
const REMEMBERED: usize = 64;

/// The playlist behind each station that `track_radio` fetched, so the id that fetch already
/// received is not asked for again.
#[derive(Default)]
pub struct Stations(Mutex<HashMap<String, String>>);

impl Stations {
    fn remember(&self, track_id: &str, playlist_id: &str) {
        let mut known = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if known.len() >= REMEMBERED {
            known.clear();
        }
        known.insert(track_id.to_owned(), playlist_id.to_owned());
    }

    /// The playlist behind the station last fetched for `track_id`, or `None` when none was.
    pub fn playlist(&self, track_id: &str) -> Option<String> {
        let known = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known.get(track_id).cloned()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RadioResponse {
    media_items: Vec<MediaItem>,
}

#[derive(Deserialize)]
struct MediaItem {
    uri: String,
}

/// The tracks of the station seeded by `track_id`. The playlist they were read from is noted in
/// `stations`.
pub async fn track_radio(
    session: &Session,
    track_id: &str,
    stations: &Stations,
) -> Result<Vec<Track>> {
    let uri =
        SpotifyUri::from_uri(&format!("{TRACK_PREFIX}{track_id}")).context("invalid track ID")?;
    let body = paced(|| session.spclient().get_radio_for_track(&uri))
        .await
        .context("cannot build track radio")?;
    let response: RadioResponse =
        serde_json::from_slice(&body).context("cannot decode track radio response")?;
    let playlist_id =
        radio_playlist_id(&response).context("track radio response did not contain a playlist")?;
    stations.remember(track_id, playlist_id);

    playlists::playlist_tracks(session, playlist_id).await
}

fn radio_playlist_id(response: &RadioResponse) -> Option<&str> {
    response
        .media_items
        .iter()
        .find_map(|item| item.uri.strip_prefix(PLAYLIST_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::{RadioResponse, Stations, radio_playlist_id};

    #[test]
    fn extracts_playlist_from_radio_response() {
        let response: RadioResponse = serde_json::from_value(serde_json::json!({
            "total": 1,
            "mediaItems": [{ "uri": "spotify:playlist:radio-playlist" }]
        }))
        .unwrap();

        assert_eq!(radio_playlist_id(&response), Some("radio-playlist"));
    }

    #[test]
    fn answers_with_the_playlist_the_radio_fetch_received() {
        let response: RadioResponse = serde_json::from_value(serde_json::json!({
            "mediaItems": [{ "uri": "spotify:playlist:radio-playlist" }]
        }))
        .unwrap();
        let stations = Stations::default();
        assert_eq!(stations.playlist("seed"), None);

        stations.remember("seed", radio_playlist_id(&response).unwrap());

        assert_eq!(stations.playlist("seed").as_deref(), Some("radio-playlist"));
        assert_eq!(stations.playlist("other"), None);
    }
}
