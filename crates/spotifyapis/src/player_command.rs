//! Compressed JSON commands for Spotify Connect devices.

use std::time::Duration;

use anyhow::Result;
use flate2::{Compression, write::GzEncoder};
use reqwest::{
    Method,
    header::{self, HeaderValue},
};
use serde_json::{Value, json};

use crate::{
    Session, http,
    proto::{between_devices, connect_state::ProvidedTrack, dealer_headers},
};

const PLAYER_COMMAND: &str = "/connect-state/v1/player/command";

/// A target and the Dealer registration used to send it commands.
pub struct ConnectDevice<'a> {
    session: &'a Session,
    from: &'a str,
    to: &'a str,
    connection: &'a str,
}

/// A request to start playback, optionally at a particular occurrence.
pub struct Play<'a> {
    pub context_uri: &'a str,
    pub context_url: &'a str,
    pub target: Option<(&'a str, &'a str)>,
    pub position: Duration,
    pub paused: bool,
    pub shuffle: Option<bool>,
    pub repeat_context: Option<bool>,
    pub repeat_track: Option<bool>,
    pub origin: (&'a str, &'a str),
}

impl Session {
    /// Addresses a Connect device using the caller's registered device and Dealer connection ID.
    pub fn connect_device<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
        connection: &'a str,
    ) -> ConnectDevice<'a> {
        ConnectDevice {
            session: self,
            from,
            to,
            connection,
        }
    }
}

impl ConnectDevice<'_> {
    pub async fn resume(&self) -> Result<()> {
        self.send(json!({ "endpoint": "resume" })).await
    }

    pub async fn pause(&self) -> Result<()> {
        self.send(json!({ "endpoint": "pause" })).await
    }

    pub async fn next(&self) -> Result<()> {
        self.send(json!({ "endpoint": "skip_next" })).await
    }

    pub async fn previous(&self) -> Result<()> {
        self.send(json!({ "endpoint": "skip_prev" })).await
    }

    pub async fn seek(&self, position: Duration) -> Result<()> {
        self.send(json!({ "endpoint": "seek_to", "value": position.as_millis() as u64 }))
            .await
    }

    pub async fn shuffle(&self, enabled: bool) -> Result<()> {
        self.send(json!({ "endpoint": "set_shuffling_context", "value": enabled }))
            .await
    }

    /// Sets the two Spotify repeat flags.
    pub async fn repeat(&self, context: bool, track: bool) -> Result<()> {
        self.send(json!({ "endpoint": "set_options", "repeating_context": context, "repeating_track": track })).await
    }

    pub async fn enqueue(&self, uri: &str) -> Result<()> {
        self.send(json!({
            "endpoint": "add_to_queue",
            "track": { "uri": uri, "metadata": { "is_queued": "true" }, "provider": "queue" },
        }))
        .await
    }

    /// Sets device volume on Spotify's 0–65535 scale.
    pub async fn volume(&self, volume: u16) -> Result<()> {
        self.session
            .set_volume(self.from, self.to, self.connection, volume)
            .await
    }

    /// Transfers playback from the source device, restoring its paused state.
    pub async fn transfer(&self) -> Result<()> {
        self.session
            .transfer_playback(self.from, self.to, self.connection)
            .await
    }

    /// Skips directly to an upcoming queue occurrence.
    pub async fn skip_to(&self, target: &ProvidedTrack) -> Result<()> {
        self.send(json!({
            "endpoint": "skip_next",
            "track": { "uri": target.uri, "uid": target.uid, "metadata": target.metadata },
        }))
        .await
    }

    /// Starts a context, with an optional occurrence and playback options.
    pub async fn play(&self, play: Play<'_>) -> Result<()> {
        let mut options = json!({
            "seek_to": play.position.as_millis() as u64,
            "initially_paused": play.paused,
        });
        for (key, value) in [
            ("shuffling_context", play.shuffle),
            ("repeating_context", play.repeat_context),
            ("repeating_track", play.repeat_track),
        ] {
            if let Some(value) = value {
                options["player_options_override"][key] = value.into();
            }
        }
        if let Some((uri, uid)) = play.target {
            let mut skip_to = json!({ "track_uri": uri });
            if !uid.is_empty() {
                skip_to["track_uid"] = uid.into();
            }
            options["skip_to"] = skip_to;
        }
        self.send(json!({
            "endpoint": "play",
            "context": { "uri": play.context_uri, "url": play.context_url, "metadata": {} },
            "play_origin": { "feature_identifier": play.origin.0, "feature_version": play.origin.1 },
            "options": options,
        })).await
    }

    async fn send(&self, mut command: Value) -> Result<()> {
        let options = if command["endpoint"] == "play" {
            "play_options"
        } else {
            "options"
        };
        for key in [
            "override_restrictions",
            "only_for_local_device",
            "system_initiated",
        ] {
            command[options][key] = false.into();
        }
        command["logging_params"] = json!({});
        let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
        serde_json::to_writer(
            &mut gzip,
            &json!({
                "command": command,
                "connection_type": "wlan",
                "intent_id": format!("{:032x}", fastrand::u128(..)),
            }),
        )?;
        let mut headers = dealer_headers(self.connection)?;
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let path = between_devices(PLAYER_COMMAND, self.from, self.to);
        http::send(
            self.session,
            Method::POST,
            &path,
            headers,
            gzip.finish()?.into(),
        )
        .await?;
        Ok(())
    }
}
