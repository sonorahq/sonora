//! Storage resolver replies for native playback and downloads.

use anyhow::Result;
use reqwest::{Url, header::HeaderMap};

use crate::{Session, http};

const ENDPOINT: &str = "/storage-resolve/files/audio/interactive";

#[derive(prost::Message)]
struct StorageResolveResponse {
    #[prost(int32, tag = "1")]
    result: i32,
    #[prost(string, repeated, tag = "2")]
    cdnurl: Vec<String>,
    // #[prost(bytes, tag = "4")]
    // pub fileid: Vec<u8>,
}

impl Session {
    /// Resolves a file's signed audio URLs.
    pub async fn audio_urls(&self, file_id: &str) -> Result<Vec<String>> {
        anyhow::ensure!(
            file_id.len() == 40 && file_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid Spotify audio file ID"
        );
        let resolved: StorageResolveResponse =
            http::get(self, &format!("{}/{file_id}", ENDPOINT), HeaderMap::new()).await?;
        anyhow::ensure!(resolved.result == 0, "Spotify did not grant a CDN download");
        Ok(resolved
            .cdnurl
            .into_iter()
            .filter(|address| {
                Url::parse(address).is_ok_and(|url| {
                    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
                })
            })
            .collect())
    }
}
