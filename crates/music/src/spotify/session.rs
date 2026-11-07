//! Native AP state with a separate HTTP bearer for cookie sessions.
use super::web::Web;
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, header::CONTENT_TYPE};
use librespot_core::{Error, Session as Native, SpotifyId, SpotifyUri, token::Token};
use librespot_protocol::{
    extended_metadata::{
        BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionQuery,
    },
    extension_kind::ExtensionKind,
};
use protobuf::{EnumOrUnknown, Message, MessageFull};
use serde::Deserialize;
use std::ops::Deref;

#[derive(Clone)]
pub struct Session {
    native: Native,
    web: Option<Web>,
}
impl From<Native> for Session {
    fn from(native: Native) -> Self {
        Self { native, web: None }
    }
}
impl Deref for Session {
    type Target = Native;
    fn deref(&self) -> &Native {
        &self.native
    }
}
impl Session {
    pub(super) fn cookie(native: Native, web: Web) -> Self {
        Self {
            native,
            web: Some(web),
        }
    }
    pub fn native(&self) -> &Native {
        &self.native
    }
    pub(super) fn web(&self) -> Option<&Web> {
        self.web.as_ref()
    }
    pub(super) fn spclient(&self) -> Client<'_> {
        Client(self)
    }
    pub(super) async fn auth_token(&self) -> Result<Token, Error> {
        match &self.web {
            Some(web) => web.auth_token().await.map_err(Error::unavailable),
            None => self.native.login5().auth_token().await,
        }
    }
}
pub(super) struct Client<'a>(&'a Session);
impl Client<'_> {
    pub async fn client_token(self) -> Result<String, Error> {
        match &self.0.web {
            Some(web) => web.client_token().await.map_err(Error::unavailable),
            None => self.0.native.spclient().client_token().await,
        }
    }
    pub async fn request(
        self,
        method: &Method,
        endpoint: &str,
        headers: Option<HeaderMap>,
        body: Option<&[u8]>,
    ) -> Result<Bytes, Error> {
        match &self.0.web {
            Some(web) => {
                if !endpoint.starts_with('/') || endpoint.starts_with("//") {
                    return Err(Error::invalid_argument("invalid Spotify endpoint"));
                }
                let mut headers = headers.unwrap_or_default();
                if endpoint.starts_with("/playlist/v2/")
                    && !headers.contains_key(http::header::ACCEPT)
                {
                    headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("application/x-protobuf"),
                    );
                }
                web.request_url(
                    method.clone(),
                    &format!("https://gae2-spclient.spotify.com{endpoint}"),
                    headers,
                    body.map(<[u8]>::to_vec),
                )
                .await
                .map(Bytes::from)
                .map_err(Error::unavailable)
            }
            None => {
                self.0
                    .native
                    .spclient()
                    .request(method, endpoint, headers, body)
                    .await
            }
        }
    }
    pub async fn request_with_protobuf<M: Message + MessageFull>(
        self,
        method: &Method,
        endpoint: &str,
        headers: Option<HeaderMap>,
        message: &M,
    ) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self
                .0
                .native
                .spclient()
                .request_with_protobuf(method, endpoint, headers, message)
                .await;
        }
        let body = message.write_to_bytes()?;
        let mut headers = headers.unwrap_or_default();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-protobuf"),
        );
        headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static("application/x-protobuf"),
        );
        self.request(method, endpoint, Some(headers), Some(&body))
            .await
    }
    pub async fn get_extended_metadata(
        self,
        request: BatchedEntityRequest,
    ) -> Result<BatchedExtensionResponse, Error> {
        if self.0.web.is_none() {
            return self
                .0
                .native
                .spclient()
                .get_extended_metadata(request)
                .await;
        }
        let body = self
            .request_with_protobuf(
                &Method::POST,
                "/extended-metadata/v0/extended-metadata",
                None,
                &request,
            )
            .await?;
        Ok(BatchedExtensionResponse::parse_from_bytes(&body)?)
    }
    pub async fn get_artist_metadata(self, uri: &SpotifyUri) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self.0.native.spclient().get_artist_metadata(uri).await;
        }
        let response = self
            .get_extended_metadata(BatchedEntityRequest {
                entity_request: vec![EntityRequest {
                    entity_uri: uri.to_uri(),
                    query: vec![ExtensionQuery {
                        extension_kind: EnumOrUnknown::new(ExtensionKind::ARTIST_V4),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await?;
        response
            .extended_metadata
            .into_iter()
            .flat_map(|array| array.extension_data)
            .find_map(|entity| {
                entity
                    .extension_data
                    .into_option()
                    .map(|data| Bytes::from(data.value))
            })
            .ok_or_else(|| Error::unavailable("Spotify artist metadata is missing"))
    }
    pub async fn get_playlist(self, id: &SpotifyId) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self.0.native.spclient().get_playlist(id).await;
        }
        self.request(
            &Method::GET,
            &format!("/playlist/v2/playlist/{}", id.to_base62()),
            None,
            None,
        )
        .await
    }
    pub async fn get_user_profile(
        self,
        username: &str,
        playlists: Option<u32>,
        artists: Option<u32>,
    ) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self
                .0
                .native
                .spclient()
                .get_user_profile(username, playlists, artists)
                .await;
        }
        let mut endpoint = format!("/user-profile-view/v3/profile/{username}?");
        if let Some(limit) = playlists {
            endpoint.push_str(&format!("playlist_limit={limit}&"));
        }
        if let Some(limit) = artists {
            endpoint.push_str(&format!("artist_limit={limit}"));
        }
        self.request(&Method::GET, &endpoint, None, None).await
    }
    pub async fn get_user_followers(self, username: &str) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self.0.native.spclient().get_user_followers(username).await;
        }
        self.request(
            &Method::GET,
            &format!("/user-profile-view/v3/profile/{username}/followers"),
            None,
            None,
        )
        .await
    }
    pub async fn get_user_following(self, username: &str) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self.0.native.spclient().get_user_following(username).await;
        }
        self.request(
            &Method::GET,
            &format!("/user-profile-view/v3/profile/{username}/following"),
            None,
            None,
        )
        .await
    }
    pub async fn get_radio_for_track(self, uri: &SpotifyUri) -> Result<Bytes, Error> {
        if self.0.web.is_none() {
            return self.0.native.spclient().get_radio_for_track(uri).await;
        }
        self.request(
            &Method::GET,
            &format!(
                "/inspiredby-mix/v2/seed_to_playlist/{}?response-format=json",
                uri.to_uri()
            ),
            None,
            None,
        )
        .await
    }
    pub async fn get_context(self, uri: &str) -> Result<SearchContext, Error> {
        if self.0.web.is_none() {
            let context = self.0.native.spclient().get_context(uri).await?;
            return Ok(SearchContext {
                pages: context
                    .pages
                    .into_iter()
                    .map(|page| SearchPage {
                        tracks: page
                            .tracks
                            .into_iter()
                            .map(|track| SearchTrack { uri: track.uri })
                            .collect(),
                    })
                    .collect(),
            });
        }
        let body = self
            .request(
                &Method::GET,
                &format!("/context-resolve/v1/{uri}"),
                None,
                None,
            )
            .await?;
        serde_json::from_slice(&body).map_err(Error::unavailable)
    }
}
// Search uses only the URIs from the context; the full playback context remains native.
#[derive(Deserialize)]
pub(super) struct SearchContext {
    #[serde(default)]
    pub pages: Vec<SearchPage>,
}
#[derive(Deserialize)]
pub(super) struct SearchPage {
    #[serde(default)]
    pub tracks: Vec<SearchTrack>,
}
#[derive(Deserialize)]
pub(super) struct SearchTrack {
    pub uri: Option<String>,
}
