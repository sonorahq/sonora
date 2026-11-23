//! Typed calls to Spotify services.

mod connect;
mod jam;
mod playlist;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use prost::Message as _;
use reqwest::{
    StatusCode,
    header::{self, HeaderMap, HeaderValue},
};

use crate::{
    Session, http,
    proto::{browse, collection, home, lyrics, metadata, profile, search, sequence},
};

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The catalogue type to request from SearchView.
#[derive(Clone, Copy)]
pub enum SearchKind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl Session {
    /// Reads the Browse landing feed.
    pub async fn browse(&self) -> Result<browse::Browse> {
        http::get(self, browse::ENDPOINT, HeaderMap::new()).await
    }

    /// Resolves a file's signed audio URLs.
    pub async fn resolve_audio(
        &self,
        file_id: &str,
    ) -> Result<crate::proto::storage::StorageResolveResponse> {
        use crate::proto::storage;
        anyhow::ensure!(
            file_id.len() == 40 && file_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid Spotify audio file ID"
        );
        http::get(
            self,
            &format!("{}/{file_id}", storage::ENDPOINT),
            HeaderMap::new(),
        )
        .await
    }

    /// Searches one catalogue type with its own result limit.
    pub async fn search(
        &self,
        query: &str,
        kind: SearchKind,
        limit: usize,
    ) -> Result<search::MainViewResponse> {
        let kind = match kind {
            SearchKind::Track => "track",
            SearchKind::Album => "album",
            SearchKind::Artist => "artist",
            SearchKind::Playlist => "playlist",
        };
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let id = (fastrand::u128(..) & !0x00000000_0000_f000_c000_000000000000)
            | 0x00000000_0000_4000_8000_000000000000;
        let request_id = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            id >> 96,
            (id >> 80) as u16,
            (id >> 64) as u16,
            (id >> 48) as u16,
            id & 0xffff_ffff_ffff,
        );
        let query = component(query);
        let path = format!(
            "{}?query={query}&entity_types={kind}&limit={limit}&timestamp={timestamp}&request_id={request_id}&show_type=podcast",
            search::ENDPOINT
        );
        http::get(self, &path, linux_headers()).await
    }

    /// Loads personalized shelves for a timezone.
    pub async fn home(&self, timezone: &str) -> Result<home::ResolvedHome> {
        http::get(
            self,
            &format!("{}?timezone={}", home::ENDPOINT, component(timezone)),
            HeaderMap::new(),
        )
        .await
    }

    /// Reads one profile and its public playlists.
    pub async fn profile(&self, username: &str, playlist_limit: u32) -> Result<profile::Profile> {
        let path = format!(
            "{}/{}?playlist_limit={playlist_limit}&artist_limit=0",
            profile::ENDPOINT,
            component(username)
        );
        http::get(self, &path, http::proto_headers(http::PROTOBUF)).await
    }

    /// Reads timed lyrics, with an empty answer for tracks Spotify has no lyrics for.
    pub async fn lyrics(&self, track_id: &str) -> Result<Option<lyrics::Answer>> {
        let path = format!("{}/{}", lyrics::ENDPOINT, component(track_id));
        let mut headers = linux_headers();
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static(concat!("spotifyapis/", env!("CARGO_PKG_VERSION"))),
        );
        match http::get(self, &path, headers).await {
            Ok(answer) => Ok(Some(answer)),
            Err(error)
                if matches!(
                    http::status(&error),
                    Some(StatusCode::NOT_FOUND | StatusCode::GONE)
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Resolves playback contexts and their first sequence pages.
    pub async fn resolve_context(
        &self,
        request: sequence::ContextRequest,
    ) -> Result<sequence::ContextResponse> {
        http::rpc(self, sequence::RESOLVE_CONTEXT, request).await
    }

    /// Reads a batch of ordered sequence pages.
    pub async fn resolve_pages(
        &self,
        request: sequence::PageRequest,
    ) -> Result<sequence::PageResponse> {
        http::rpc(self, sequence::RESOLVE_PAGE, request).await
    }

    /// Loads the requested catalogue extensions.
    pub async fn extended_metadata(
        &self,
        request: metadata::BatchedEntityRequest,
    ) -> Result<metadata::BatchedExtensionResponse> {
        http::rpc(self, metadata::ENDPOINT, request).await
    }

    /// Reads a page of saved collection items.
    pub async fn collection_page(
        &self,
        request: &collection::PageRequest,
    ) -> Result<collection::PageResponse> {
        let bytes = http::post_proto(
            self,
            collection::PAGING,
            collection::CONTENT,
            HeaderMap::new(),
            request,
        )
        .await?;
        Ok(collection::PageResponse::decode(bytes)?)
    }

    /// Applies updates to the saved collection or pins.
    pub async fn update_collection(
        &self,
        request: &collection::WriteRequest,
        connection_id: &str,
    ) -> Result<()> {
        http::post_proto(
            self,
            collection::WRITE,
            collection::CONTENT,
            dealer_headers(connection_id)?,
            request,
        )
        .await?;
        Ok(())
    }
}

pub(crate) fn component(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

pub(crate) fn dealer_headers(connection_id: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if !connection_id.is_empty() {
        headers.insert("x-spotify-connection-id", connection_id.parse()?);
    }
    Ok(headers)
}

// Search and Lyrics reject requests without a platform.
fn linux_headers() -> HeaderMap {
    HeaderMap::from_iter([(
        header::HeaderName::from_static("app-platform"),
        HeaderValue::from_static("Linux"),
    )])
}
