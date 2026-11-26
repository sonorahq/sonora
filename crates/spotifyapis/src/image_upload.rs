//! Spotify playlist JPEG uploads.

use anyhow::Result;
use bytes::Bytes;
use reqwest::header::{self, HeaderMap, HeaderValue};

use crate::{Session, http};

const IMAGE_UPLOAD: &str = "https://image-upload.spotify.com/v4/playlist";

impl Session {
    /// Uploads a JPEG and returns the token for playlist cover registration.
    pub async fn upload_playlist_image(&self, jpeg: Bytes) -> Result<String> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Uploaded {
            upload_token: String,
        }
        let headers = HeaderMap::from_iter([
            (header::ACCEPT, HeaderValue::from_static("application/json")),
            (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
        ]);
        let bytes = http::post_service(self, IMAGE_UPLOAD, headers, jpeg).await?;
        Ok(serde_json::from_slice::<Uploaded>(&bytes)?.upload_token)
    }
}
