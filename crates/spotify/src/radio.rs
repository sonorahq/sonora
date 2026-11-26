use anyhow::{Context as _, Result};
use librespot_core::{Session, SpotifyUri};
use music::{RadioSeed, Track};
use serde::Deserialize;

use crate::pace::paced;
use crate::playlists;

const TRACK_PREFIX: &str = "spotify:track:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RadioResponse {
    media_items: Vec<MediaItem>,
}

#[derive(Deserialize)]
struct MediaItem {
    uri: String,
}

pub async fn track_radio(session: &Session, track_id: &str) -> Result<Vec<Track>> {
    seeded(session, &format!("{TRACK_PREFIX}{track_id}")).await
}

/// The radio of a whole album, playlist or artist. Spotify builds it through the same endpoint
/// as a track's, seeded by the collection's uri.
pub async fn collection_radio(session: &Session, seed: RadioSeed, id: &str) -> Result<Vec<Track>> {
    let kind = match seed {
        RadioSeed::Album => "album",
        RadioSeed::Playlist => "playlist",
        RadioSeed::Artist => "artist",
    };
    seeded(session, &format!("spotify:{kind}:{id}")).await
}

/// The tracks of the radio playlist Spotify builds from the item at `uri`.
async fn seeded(session: &Session, uri: &str) -> Result<Vec<Track>> {
    let uri = SpotifyUri::from_uri(uri).context("invalid radio seed")?;
    let body = paced(|| session.spclient().get_radio_for_track(&uri))
        .await
        .context("cannot build radio")?;
    let response: RadioResponse =
        serde_json::from_slice(&body).context("cannot decode radio response")?;
    let playlist_id =
        radio_playlist_id(&response).context("radio response did not contain a playlist")?;

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
    use super::{RadioResponse, radio_playlist_id};

    #[test]
    fn extracts_playlist_from_radio_response() {
        let response: RadioResponse = serde_json::from_value(serde_json::json!({
            "total": 1,
            "mediaItems": [{ "uri": "spotify:playlist:radio-playlist" }]
        }))
        .unwrap();

        assert_eq!(radio_playlist_id(&response), Some("radio-playlist"));
    }
}
