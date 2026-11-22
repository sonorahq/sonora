use std::io::Read as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use bytes::Bytes;
use flate2::read::MultiGzDecoder;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url, header};
use serde::Deserialize;
use tokio::sync::OnceCell;

use super::session::Session;

const DISCOVERY: &str =
    "https://apresolve.spotify.com/v2?type=accesspoint&type=dealer&type=spclient";
pub(super) const METADATA: &str = "/extended-metadata/v0/extended-metadata";
pub(super) const PROTOBUF: &str = "application/x-protobuf";
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Adds Dealer's connection ID when Connect is running.
pub(super) fn add_connection_id(session: &Session, headers: &mut header::HeaderMap) -> Result<()> {
    let id = session.connection_id();
    if !id.is_empty() {
        headers.insert("x-spotify-connection-id", id.parse()?);
    }
    Ok(())
}

/// Shares HTTPS connections across Spotify's API and audio CDN requests.
static HTTP: OnceCell<Client> = OnceCell::const_new();

/// Spotify's regional HTTPS and Dealer hosts, ordered by the discovery service.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Endpoints {
    pub(super) accesspoint: Vec<String>,
    pub(super) spclient: ApiHosts,
    pub(super) dealer: Vec<String>,
}

/// Discovery v2 lists regional and global API hosts separately.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct ApiHosts {
    pub(super) regional: Vec<String>,
    pub(super) global: Vec<String>,
}

pub(super) async fn discover() -> Result<Endpoints> {
    Ok(request(|client| client.get(DISCOVERY))
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Accepts Spotify hosts from discovery before sending any account bearer to them.
pub(super) fn regional_url(scheme: &str, address: &str) -> Option<Url> {
    let url = Url::parse(&format!("{scheme}://{address}")).ok()?;
    let host = url.host_str()?;
    (host.ends_with(".spotify.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

/// Posts a protobuf request and decodes the response using the same wire format.
pub(super) async fn rpc<T: prost::Message + Default>(
    session: &Session,
    path: &str,
    request: impl prost::Message,
) -> Result<T> {
    let body = post_proto(session, path, PROTOBUF, false, &request).await?;
    decode(path, body)
}

/// Posts an encoded protobuf message with the route's media type and optional Dealer identity.
pub(super) async fn post_proto(
    session: &Session,
    path: &str,
    mime: &'static str,
    dealer: bool,
    message: &impl prost::Message,
) -> Result<Bytes> {
    let mut headers = proto_headers(mime, None);
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static(mime));
    if dealer {
        add_connection_id(session, &mut headers)?;
    }
    send(
        session,
        Method::POST,
        path,
        headers,
        message.encode_to_vec().into(),
    )
    .await
}

/// Reads a protobuf response with any endpoint-specific headers supplied by the caller.
pub(super) async fn get<T: prost::Message + Default>(
    session: &Session,
    path: &str,
    headers: header::HeaderMap,
) -> Result<T> {
    let body = send(session, Method::GET, path, headers, Bytes::new()).await?;
    decode(path, body)
}

fn decode<T: prost::Message + Default>(path: &str, body: Bytes) -> Result<T> {
    T::decode(body).with_context(|| format!("cannot decode Spotify response from {path}"))
}

/// Advertises the protobuf media type and client platform required by the route.
pub(super) fn proto_headers(
    mime: &'static str,
    platform: Option<&'static str>,
) -> header::HeaderMap {
    let mut headers =
        header::HeaderMap::from_iter([(header::ACCEPT, header::HeaderValue::from_static(mime))]);
    if let Some(platform) = platform {
        headers.insert("app-platform", header::HeaderValue::from_static(platform));
    }
    headers
}

/// Sends a bearer-authenticated request to the regional Spotify API.
pub(super) async fn send(
    session: &Session,
    method: Method,
    path: &str,
    headers: header::HeaderMap,
    body: Bytes,
) -> Result<Bytes> {
    let (base, token) = tokio::try_join!(session.api_url(), session.access_token())?;
    let url = base.join(path)?;
    anyhow::ensure!(url.origin() == base.origin(), "invalid Spotify API path");
    request_at(method, url, &token, headers, body).await
}

pub(super) fn status(error: &anyhow::Error) -> Option<StatusCode> {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
}

/// Posts to a code-defined API endpoint on a dedicated Spotify host.
pub(super) async fn post_service(
    session: &Session,
    endpoint: &'static str,
    headers: header::HeaderMap,
    body: Bytes,
) -> Result<Bytes> {
    let url = Url::parse(endpoint)?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host.ends_with(".spotify.com"))
            && url.username().is_empty()
            && url.password().is_none(),
        "invalid dedicated Spotify API endpoint"
    );
    let token = session.access_token().await?;
    request_at(Method::POST, url, &token, headers, body).await
}

async fn request_at(
    method: Method,
    url: Url,
    token: &str,
    headers: header::HeaderMap,
    body: Bytes,
) -> Result<Bytes> {
    let response = request(|client| {
        client
            .request(method, url)
            .bearer_auth(token)
            .headers(headers)
            .header(header::ACCEPT_ENCODING, "gzip")
            .body(body)
    })
    .await?;
    let response = response.error_for_status()?;
    anyhow::ensure!(
        response.status().is_success(),
        "Spotify returned {}",
        response.status()
    );
    let gzip = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"gzip"));
    let body = response.bytes().await?;
    if gzip {
        let mut decoded = Vec::new();
        MultiGzDecoder::new(body.as_ref()).read_to_end(&mut decoded)?;
        return Ok(decoded.into());
    }
    Ok(body)
}

/// Builds and sends a request using the shared Spotify client.
pub(super) async fn request(
    build: impl FnOnce(&Client) -> RequestBuilder,
) -> reqwest::Result<Response> {
    let client = HTTP
        .get_or_try_init(|| async {
            Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .read_timeout(READ_TIMEOUT)
                .build()
        })
        .await?;
    build(client).send().await
}
