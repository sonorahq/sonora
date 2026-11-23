//! Shared HTTP clients and protobuf requests.

use std::future::Future;
use std::io::Read as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use bytes::Bytes;
use flate2::read::MultiGzDecoder;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url, header};
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::Session;

const DISCOVERY: &str =
    "https://apresolve.spotify.com/v2?type=accesspoint&type=dealer&type=spclient";
pub(crate) const PROTOBUF: &str = "application/x-protobuf";
const API_TIMEOUT: Duration = Duration::from_secs(20);

/// Shares HTTPS connections for Spotify Accounts and the regional API.
static HTTP: OnceCell<Client> = OnceCell::const_new();

/// Spotify's regional HTTPS and Dealer hosts, ordered by the discovery service.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct Endpoints {
    pub(crate) accesspoint: Vec<String>,
    pub(crate) spclient: ApiHosts,
    pub(crate) dealer: Vec<String>,
}

/// Discovery v2 lists regional and global API hosts separately.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct ApiHosts {
    pub(crate) regional: Vec<String>,
    pub(crate) global: Vec<String>,
}

pub(crate) async fn discover() -> Result<Endpoints> {
    Ok(request(|client| client.get(DISCOVERY))
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Accepts Spotify hosts from discovery before sending any account bearer to them.
pub(crate) fn regional_url(scheme: &str, address: &str) -> Option<Url> {
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

/// Calls a read-only protobuf RPC, which may be retried on a transient failure.
pub(crate) async fn read_rpc<T: prost::Message + Default>(
    session: &Session,
    path: &str,
    request: impl prost::Message,
) -> Result<T> {
    let body =
        retry_read(|| post_proto(session, path, PROTOBUF, header::HeaderMap::new(), &request))
            .await?;
    decode(path, body)
}

/// Posts an encoded protobuf message with the route’s media type and extra headers.
pub(crate) async fn post_proto(
    session: &Session,
    path: &str,
    mime: &'static str,
    mut headers: header::HeaderMap,
    message: &impl prost::Message,
) -> Result<Bytes> {
    headers.insert(header::ACCEPT, header::HeaderValue::from_static(mime));
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static(mime));
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
pub(crate) async fn get<T: prost::Message + Default>(
    session: &Session,
    path: &str,
    headers: header::HeaderMap,
) -> Result<T> {
    let body =
        retry_read(|| send(session, Method::GET, path, headers.clone(), Bytes::new())).await?;
    decode(path, body)
}

/// Repeats a read once after a transport failure or a transient server error.
async fn retry_read<T, F: Future<Output = Result<T>>>(mut read: impl FnMut() -> F) -> Result<T> {
    match read().await {
        Err(error)
            if error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                match error.status() {
                    Some(status) => matches!(
                        status,
                        StatusCode::REQUEST_TIMEOUT
                            | StatusCode::INTERNAL_SERVER_ERROR
                            | StatusCode::BAD_GATEWAY
                            | StatusCode::SERVICE_UNAVAILABLE
                            | StatusCode::GATEWAY_TIMEOUT
                    ),
                    None => error.is_timeout() || error.is_request() || error.is_body(),
                }
            }) =>
        {
            read().await
        }
        result => result,
    }
}

fn decode<T: prost::Message + Default>(path: &str, body: Bytes) -> Result<T> {
    T::decode(body).with_context(|| format!("cannot decode Spotify response from {path}"))
}

/// Requests a protobuf response using the route's supported media type.
pub(crate) fn proto_headers(mime: &'static str) -> header::HeaderMap {
    header::HeaderMap::from_iter([(header::ACCEPT, header::HeaderValue::from_static(mime))])
}

/// Sends a bearer-authenticated request to the regional Spotify API.
pub(crate) async fn send(
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

pub fn status(error: &anyhow::Error) -> Option<StatusCode> {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
}

/// Posts to a code-defined API endpoint on a dedicated Spotify host.
pub(crate) async fn post_service(
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
    let metadata =
        method == Method::POST && url.path() == crate::proto::extended_metadata::ENDPOINT;
    let build = |client: &Client| {
        client
            .request(method.clone(), url.clone())
            .bearer_auth(token)
            .headers(headers.clone())
            .header(header::ACCEPT_ENCODING, "gzip")
            .body(body.clone())
    };
    let response = request(&build).await?;
    let retry_body = metadata && response.version() == reqwest::Version::HTTP_2;
    match response_bytes(response).await {
        Err(error)
            if retry_body
                && error
                    .chain()
                    .any(|cause| cause.to_string().contains("too_many_data_frames")) =>
        {
            // Repeating the same metadata read on HTTP/2 can hit the same frame budget.
            static HTTP1: OnceCell<Client> = OnceCell::const_new();
            let client = HTTP1
                .get_or_try_init(|| async { api_client().http1_only().build() })
                .await?;
            response_bytes(build(client).send().await?).await
        }
        result => result,
    }
}

async fn response_bytes(response: Response) -> Result<Bytes> {
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

fn api_client() -> reqwest::ClientBuilder {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(API_TIMEOUT)
        .read_timeout(API_TIMEOUT)
}

/// Builds and sends a request using the shared Spotify client.
pub(crate) async fn request(
    build: impl FnOnce(&Client) -> RequestBuilder,
) -> reqwest::Result<Response> {
    initialize_tls();
    let client = HTTP
        .get_or_try_init(|| async { api_client().build() })
        .await?;
    build(client).send().await
}

/// Whether Spotify's discovery service is reachable without account credentials.
pub async fn reachable() -> bool {
    discover().await.is_ok()
}

/// Shared client for signed CDN requests, with shorter stall timeouts than the API.
pub fn media_client() -> Result<&'static Client> {
    static MEDIA: std::sync::OnceLock<std::result::Result<Client, reqwest::Error>> =
        std::sync::OnceLock::new();
    initialize_tls();
    MEDIA
        .get_or_init(|| {
            Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(Duration::from_secs(8))
                .build()
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("cannot build the Spotify media client: {error}"))
}

fn initialize_tls() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}
