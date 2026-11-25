use anyhow::{Context as _, Result, anyhow};
use librespot_core::Session;
use serde::Deserialize;

use super::query;

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "trackUnion")]
    track: Option<Track>,
}

#[derive(Deserialize)]
struct Track {
    playcount: Option<String>,
}

pub(crate) async fn track(session: &Session, track_id: &str) -> Result<Option<u64>> {
    let variables = serde_json::json!({ "uri": format!("spotify:track:{track_id}") });
    let data = query::<Data>(session, "getTrack", variables).await?;
    playcount(data)
}

fn playcount(data: Data) -> Result<Option<u64>> {
    let Some(track) = data.track else {
        return Err(anyhow!("track play count response has no track"));
    };
    track
        .playcount
        .map(|count| count.parse().context("invalid track play count"))
        .transpose()
        .map(|count| count.and_then(super::reported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_playcount() {
        let data: Data =
            serde_json::from_slice(br#"{"trackUnion":{"playcount":"1234567"}}"#).unwrap();
        assert_eq!(playcount(data).unwrap(), Some(1_234_567));
    }
}
