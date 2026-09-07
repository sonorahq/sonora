use std::time::SystemTime;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Scrobbler, Track};

const SUBMIT_ENDPOINT: &str = "https://api.listenbrainz.org/1/submit-listens";
const VALIDATE_ENDPOINT: &str = "https://api.listenbrainz.org/1/validate-token";

pub struct ListenBrainzClient {
    http: reqwest::Client,
    token: String,
}

impl ListenBrainzClient {
    pub fn new(token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            token,
        }
    }

    async fn submit(&self, body: Value) -> Result<()> {
        let response = self
            .http
            .post(SUBMIT_ENDPOINT)
            .header("Authorization", format!("Token {}", self.token))
            .json(&body)
            .send()
            .await
            .context("cannot reach listenbrainz")?;
        if !response.status().is_success() {
            anyhow::bail!("listenbrainz answered with status {}", response.status());
        }
        Ok(())
    }
}

fn artist_of(track: &Track) -> String {
    track
        .artist_refs
        .first()
        .map(|artist| artist.name.clone())
        .unwrap_or_else(|| track.artists.clone())
}

fn listen_payload(track: &Track, listen_type: &str, listened_at: Option<u64>) -> Value {
    let mut entry = json!({
        "track_metadata": {
            "artist_name": artist_of(track),
            "track_name": track.name,
            "release_name": track.album,
            "additional_info": {
                "duration_ms": track.duration.as_millis() as u64,
            },
        },
    });
    if let Some(listened_at) = listened_at {
        entry["listened_at"] = json!(listened_at);
    }
    json!({
        "listen_type": listen_type,
        "payload": [entry],
    })
}

pub async fn validate_token(token: &str) -> Result<bool> {
    #[derive(Deserialize)]
    struct Response {
        valid: bool,
    }
    let response: Response = reqwest::Client::new()
        .get(VALIDATE_ENDPOINT)
        .header("Authorization", format!("Token {token}"))
        .send()
        .await
        .context("cannot reach listenbrainz")?
        .json()
        .await
        .context("cannot read the listenbrainz response")?;
    Ok(response.valid)
}

#[async_trait]
impl Scrobbler for ListenBrainzClient {
    async fn now_playing(&self, track: &Track) -> Result<()> {
        self.submit(listen_payload(track, "playing_now", None))
            .await
    }

    async fn scrobble(&self, track: &Track, started_at: SystemTime) -> Result<()> {
        let listened_at = started_at
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.submit(listen_payload(track, "single", Some(listened_at)))
            .await
    }
}
