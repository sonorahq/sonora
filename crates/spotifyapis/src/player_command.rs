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
