//! A track's station, in the order YouTube ranks it and as far as YouTube goes on serving it.
//! `ytmusic::track_radio` asks for the first panel alone, and the continuation that keeps a
//! station running past it is only in the raw answer, so the request is made here. The radios
//! of a playlist, an album or an artist come through the same panel.

use anyhow::{Context as _, Result};
use music::Track;
use serde_json::{Value, json};
use ytmusic::nav::Nav as _;
use ytmusic::{Client, YtMusic, parse};

use crate::wire;

/// The `params` YouTube Music sends to ask for a playlist's radio rather than the playlist.
const RADIO_PARAMS: &str = "wAEB";

/// The station YouTube Music opens when a track is played on its own: the seed first, then what
/// YouTube's automix picks after it. With `from` it is the next stretch instead, which arrives
/// under a different renderer key. The seed goes with the continuation either way, since on its
/// own the token answers with most of the panel before it again.
pub(crate) async fn station(
    api: &YtMusic,
    video_id: &str,
    from: Option<&str>,
) -> Result<(Vec<Track>, Option<String>)> {
    let body = json!({
        "videoId": video_id,
        "playlistId": format!("RDAMVM{video_id}"),
        "enablePersistentPlaylistPanel": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
    });
    next(api, body, from).await
}

/// The radio YouTube Music starts from a whole collection, paged like `station`. `seed` is the
/// radio's playlist id: `RDAMPL` before a playlist's or an album's own id, or the `RDEM…` id an
/// artist page offers.
pub(crate) async fn collection(
    api: &YtMusic,
    seed: &str,
    from: Option<&str>,
) -> Result<(Vec<Track>, Option<String>)> {
    let body = json!({
        "playlistId": seed,
        "params": RADIO_PARAMS,
        "enablePersistentPlaylistPanel": true,
        "isAudioOnly": true,
        "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
    });
    next(api, body, from).await
}

/// The radio id behind an artist page's Start radio button.
pub(crate) async fn artist_seed(api: &YtMusic, channel_id: &str) -> Result<String> {
    let response = api
        .execute("browse", Client::Music, json!({ "browseId": channel_id }))
        .await?;
    parse::find_renderers(&response, "watchPlaylistEndpoint")
        .into_iter()
        .filter_map(|endpoint| endpoint.str_at(&["playlistId"]))
        .find(|id| id.starts_with("RDEM"))
        .map(str::to_string)
        .context("artist page offers no radio")
}

/// Asks `next` with `body`, at its start or at the continuation `from`.
async fn next(
    api: &YtMusic,
    mut body: Value,
    from: Option<&str>,
) -> Result<(Vec<Track>, Option<String>)> {
    let key = match from {
        Some(_) => "playlistPanelContinuation",
        None => "playlistPanelRenderer",
    };
    if let Some(from) = from {
        body["continuation"] = json!(from);
    }
    let response = api.execute("next", Client::Music, body).await?;
    Ok(panel(&response, key))
}

/// The tracks of the first panel under `key`, and the continuation that follows them. A panel
/// with no radio continuation is the end of the station.
fn panel(response: &Value, key: &str) -> (Vec<Track>, Option<String>) {
    let Some(panel) = parse::find_renderers(response, key).into_iter().next() else {
        return (Vec::new(), None);
    };
    let tracks = panel
        .items(&["contents"])
        .iter()
        .filter_map(parse::panel_track)
        .enumerate()
        .map(|(index, track)| wire::track(track, index as u32))
        .collect();
    let next = panel
        .items(&["continuations"])
        .iter()
        .find_map(|item| item.str_at(&["nextRadioContinuationData", "continuation"]))
        .map(str::to_string);
    (tracks, next)
}
