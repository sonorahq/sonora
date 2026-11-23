//! Compressed JSON commands for Spotify Connect devices.

use anyhow::Result;
use flate2::{Compression, write::GzEncoder};
use reqwest::{
    Method,
    header::{self, HeaderValue},
};
use serde_json::{Value, json};

use crate::{
    Session, http,
    proto::{between_devices, dealer_headers},
};

const PLAYER_COMMAND: &str = "/connect-state/v1/player/command";

impl Session {
    /// Sends Spotify's compressed command envelope to another Connect device.
    pub async fn player_command(
        &self,
        from: &str,
        to: &str,
        connection: &str,
        mut command: Value,
    ) -> Result<()> {
        let options = if command["endpoint"] == "play" {
            "play_options"
        } else {
            "options"
        };
        anyhow::ensure!(
            command.is_object()
                && command
                    .get(options)
                    .is_none_or(|value| value.is_null() || value.is_object()),
            "invalid Spotify player command"
        );
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
        let mut headers = dealer_headers(connection)?;
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let path = between_devices(PLAYER_COMMAND, from, to);
        http::send(self, Method::POST, &path, headers, gzip.finish()?.into()).await?;
        Ok(())
    }
}

/// Builds the same occurrence selector for ordinary play and targeted history jumps.
pub fn play(
    uri: &str,
    url: &str,
    target: Option<(&str, &str)>,
    mut options: serde_json::Value,
    origin: (&str, &str),
) -> serde_json::Value {
    if let Some((uri, uid)) = target {
        let mut skip_to = serde_json::json!({ "track_uri": uri });
        if !uid.is_empty() {
            skip_to["track_uid"] = uid.into();
        }
        options["skip_to"] = skip_to;
    }
    serde_json::json!({
        "endpoint": "play",
        "context": { "uri": uri, "url": url, "metadata": {} },
        "play_origin": { "feature_identifier": origin.0, "feature_version": origin.1 },
        "options": options,
        "logging_params": {},
    })
}

/// Targets a specific upcoming occurrence rather than skipping to the next row.
pub fn skip_next_to(target: &crate::proto::connect_state::ProvidedTrack) -> Value {
    json!({
        "endpoint": "skip_next",
        "track": { "uri": target.uri, "uid": target.uid, "metadata": target.metadata },
    })
}
