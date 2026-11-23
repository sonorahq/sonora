//! Playlist snapshots, projections, edits, and artwork registration.

use anyhow::Result;
use bytes::Bytes;
use prost::Message as _;
use reqwest::header::{self, HeaderMap, HeaderValue};

use crate::{
    Session, http,
    proto::{list_read, playlist},
};

use super::{component, dealer_headers};

const IMAGE_UPLOAD: &str = "https://image-upload.spotify.com/v4/playlist";

impl Session {
    /// Reads typed projections for playlists and root lists.
    pub async fn get_lists(
        &self,
        request: list_read::GetListsRequest,
    ) -> Result<list_read::GetListsResponse> {
        http::rpc(self, list_read::GET_LISTS, request).await
    }

    /// Reads a playlist snapshot or just its summary.
    pub async fn playlist(&self, id: &str, summary: bool) -> Result<playlist::SelectedListContent> {
        let query = if summary { "?from=0&length=0" } else { "" };
        let path = format!("{}/{}{query}", playlist::PLAYLIST_ENDPOINT, component(id));
        http::get(self, &path, HeaderMap::new()).await
    }

    /// Reads a page of the signed-in account's playlist library.
    pub async fn rootlist_page(
        &self,
        from: usize,
        length: usize,
    ) -> Result<playlist::SelectedListContent> {
        let path = format!(
            "{}/{}/rootlist?decorate=revision,attributes,length,owner,capabilities,status_code&from={from}&length={length}",
            playlist::USER_ENDPOINT,
            component(self.username())
        );
        http::get(self, &path, HeaderMap::new()).await
    }

    /// Creates a playlist from the supplied changes.
    pub async fn create_playlist(
        &self,
        changes: &playlist::ListChanges,
        connection_id: &str,
    ) -> Result<playlist::CreateListReply> {
        let bytes = http::post_proto(
            self,
            playlist::PLAYLIST_ENDPOINT,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            changes,
        )
        .await?;
        Ok(playlist::CreateListReply::decode(bytes)?)
    }

    /// Applies changes to an existing playlist.
    pub async fn edit_playlist(
        &self,
        id: &str,
        changes: &playlist::ListChanges,
        connection_id: &str,
    ) -> Result<()> {
        let path = format!("{}/{}/changes", playlist::PLAYLIST_ENDPOINT, component(id));
        http::post_proto(
            self,
            &path,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            changes,
        )
        .await?;
        Ok(())
    }

    /// Applies changes to the signed-in account's playlist library.
    pub async fn edit_rootlist(
        &self,
        changes: &playlist::ListChanges,
        connection_id: &str,
    ) -> Result<()> {
        let path = format!(
            "{}/{}/rootlist/changes",
            playlist::USER_ENDPOINT,
            component(self.username())
        );
        http::post_proto(
            self,
            &path,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            changes,
        )
        .await?;
        Ok(())
    }

    /// Registers a cover already uploaded to Spotify.
    pub async fn register_playlist_image(
        &self,
        id: &str,
        request: &playlist::RegisterPlaylistImageRequest,
        connection_id: &str,
    ) -> Result<playlist::RegisterPlaylistImageResponse> {
        let path = format!(
            "{}/{}/register-image",
            playlist::PLAYLIST_ENDPOINT,
            component(id)
        );
        let bytes = http::post_proto(
            self,
            &path,
            http::PROTOBUF,
            dealer_headers(connection_id)?,
            request,
        )
        .await?;
        Ok(playlist::RegisterPlaylistImageResponse::decode(bytes)?)
    }

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
