//! Track search. It goes out signed out, as every search of this provider does, because YouTube
//! files each signed-in search in the account's search history and the app searches while the
//! query is still being typed.

use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use ytmusic::nav::Nav as _;
use ytmusic::{Client, Track, YtMusic, parse};

/// How long song results wait on the optional video search before going out without it.
const VIDEO_TIMEOUT: Duration = Duration::from_secs(5);

/// Songs first, then music videos, since YouTube Music indexes both and a video may be the only
/// upload of a track. A failed or slow video search leaves the songs standing.
pub async fn tracks(api: &YtMusic, query: &str) -> Result<Vec<Track>> {
    let (songs, videos) = tokio::join!(
        api.search_songs_with(query, false),
        tokio::time::timeout(VIDEO_TIMEOUT, videos(api, query)),
    );
    let mut tracks = songs?;
    for video in videos.ok().and_then(Result::ok).unwrap_or_default() {
        if !tracks.iter().any(|track| track.video_id == video.video_id) {
            tracks.push(video);
        }
    }
    Ok(tracks)
}

async fn videos(api: &YtMusic, query: &str) -> Result<Vec<Track>> {
    let response = api
        .execute_with(
            "search",
            Client::Music,
            json!({ "query": query, "params": ytmusic::search::VIDEOS }),
            false,
        )
        .await?;
    Ok(parse::find_renderers(&response, "musicShelfRenderer")
        .into_iter()
        .flat_map(|shelf| shelf.items(&["contents"]))
        .filter_map(parse::list_item_track)
        .filter(|track| track.video_id.is_some())
        .collect())
}
