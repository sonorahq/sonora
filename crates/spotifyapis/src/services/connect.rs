//! Connect state and device commands.

use anyhow::Result;
use bytes::Bytes;
use flate2::{Compression, write::GzEncoder};
use prost::Message as _;
use reqwest::{
    Method,
    header::{self, HeaderValue},
};
use serde_json::{Value, json};

use super::{component, dealer_headers};
use crate::{
    Session, http,
    proto::connect::{self, Cluster, PutStateRequest, SetVolumeRequest},
};

const VOLUME: &str = "/connect-state/v1/connect/volume";
const TRANSFER: &str = "/connect-state/v1/connect/transfer";
const PLAYER_COMMAND: &str = "/connect-state/v1/player/command";

impl Session {
    /// Publishes device state and returns the current account cluster.
    pub async fn put_device_state(
        &self,
        device: &str,
        connection: &str,
        request: &PutStateRequest,
    ) -> Result<Cluster> {
        let body = self
            .device_state(
                device,
                connection,
                Method::PUT,
                "",
                request.encode_to_vec().into(),
            )
            .await?;
        Ok(Cluster::decode(body)?)
    }

    /// Marks a registered device inactive without notifying its peers.
    pub async fn deactivate_device(&self, device: &str, connection: &str) -> Result<()> {
        self.device_state(
            device,
            connection,
            Method::PUT,
            "/inactive?notify=false",
            Bytes::new(),
        )
        .await?;
        Ok(())
    }

    /// Unregisters a device from Spotify Connect.
    pub async fn unregister_device(&self, device: &str) -> Result<()> {
        self.device_state(device, "", Method::DELETE, "", Bytes::new())
            .await?;
        Ok(())
    }

    /// Sets the volume of another Spotify Connect device.
    pub async fn set_volume(
        &self,
        from: &str,
        to: &str,
        connection: &str,
        request: &SetVolumeRequest,
    ) -> Result<()> {
        self.device_command(
            Method::PUT,
            &between_devices(VOLUME, from, to),
            connection,
            http::PROTOBUF,
            request.encode_to_vec().into(),
        )
        .await
    }

    /// Transfers playback between devices and restores the paused state.
    pub async fn transfer_playback(&self, from: &str, to: &str, connection: &str) -> Result<()> {
        let body = json!({"transfer_options": {"restore_paused": "restore"}});
        self.device_command(
            Method::POST,
            &between_devices(TRANSFER, from, to),
            connection,
            "application/json",
            serde_json::to_vec(&body)?.into(),
        )
        .await
    }

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

    async fn device_state(
        &self,
        device: &str,
        connection: &str,
        method: Method,
        suffix: &str,
        body: Bytes,
    ) -> Result<Bytes> {
        let mut headers = http::proto_headers(http::PROTOBUF);
        if method == Method::PUT {
            headers.insert("x-spotify-connection-id", connection.parse()?);
            if !body.is_empty() {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(http::PROTOBUF),
                );
            }
        }
        let path = format!("{}/{}{suffix}", connect::DEVICES, component(device));
        http::send(self, method, &path, headers, body).await
    }

    async fn device_command(
        &self,
        method: Method,
        path: &str,
        connection: &str,
        mime: &'static str,
        body: Bytes,
    ) -> Result<()> {
        let mut headers = dealer_headers(connection)?;
        headers.extend(http::proto_headers(mime));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
        http::send(self, method, path, headers, body).await?;
        Ok(())
    }
}

fn between_devices(prefix: &str, from: &str, to: &str) -> String {
    format!("{prefix}/from/{}/to/{}", component(from), component(to))
}
