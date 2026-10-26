//! What Sonora asks of plex.tv rather than of a media server: the sign-in pin, the account, and
//! where the account's servers can be reached.

use std::time::Duration;

use anyhow::{Context as _, Result};
use reqwest::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};

use crate::escape;
use crate::plex::wire::{Account, Connection, Pin, Resource};

const API: &str = "https://plex.tv/api/v2";
const AUTH: &str = "https://app.plex.tv/auth";
const PRODUCT: &str = "Sonora";
/// How long one connection gets to answer before the next is tried.
const PROBE: Duration = Duration::from_secs(4);
const TIMEOUT: Duration = Duration::from_secs(20);

/// The headers that say who is asking. Plex lists a client on the account by what they say, and
/// a media server wants them on every request.
pub fn identity(client_id: &str) -> HeaderMap {
    let device = gethostname::gethostname().to_string_lossy().into_owned();
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    for (name, value) in [
        ("x-plex-client-identifier", client_id),
        ("x-plex-product", PRODUCT),
        ("x-plex-version", env!("CARGO_PKG_VERSION")),
        ("x-plex-platform", std::env::consts::OS),
        ("x-plex-device", PRODUCT),
        ("x-plex-device-name", device.as_str()),
    ] {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    }
    headers
}

/// Asks plex.tv for a pin. The user approves it in a browser, and polling it then hands over the
/// account's token.
pub async fn create_pin(http: &reqwest::Client, client_id: &str) -> Result<Pin> {
    http.post(format!("{API}/pins"))
        .headers(identity(client_id))
        .form(&[("strong", "true")])
        .timeout(TIMEOUT)
        .send()
        .await
        .context("cannot reach plex.tv")?
        .error_for_status()
        .context("plex.tv refused to start a sign-in")?
        .json()
        .await
        .context("cannot read the plex.tv pin")
}

/// The page where the user approves `pin`.
pub fn approval_url(client_id: &str, pin: &Pin) -> String {
    format!(
        "{AUTH}#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D={PRODUCT}",
        escape::component(client_id),
        escape::component(&pin.code),
    )
}

/// The account token once the user has approved `pin`, or `None` while they have not.
pub async fn claimed(http: &reqwest::Client, client_id: &str, pin: &Pin) -> Result<Option<String>> {
    let answer: Pin = http
        .get(format!("{API}/pins/{}", pin.id))
        .headers(identity(client_id))
        .query(&[("code", pin.code.as_str())])
        .timeout(TIMEOUT)
        .send()
        .await
        .context("cannot reach plex.tv")?
        .error_for_status()
        .context("plex.tv no longer knows the sign-in")?
        .json()
        .await
        .context("cannot read the plex.tv pin")?;
    Ok(answer.auth_token.filter(|token| !token.is_empty()))
}

pub async fn account(http: &reqwest::Client, client_id: &str, token: &str) -> Result<Account> {
    let mut headers = identity(client_id);
    headers.insert("x-plex-token", HeaderValue::from_str(token)?);
    http.get(format!("{API}/user"))
        .headers(headers)
        .timeout(TIMEOUT)
        .send()
        .await
        .context("cannot reach plex.tv")?
        .error_for_status()
        .context("plex.tv refused the account")?
        .json()
        .await
        .context("cannot read the plex.tv account")
}

/// The media servers the account can use, its own and the ones shared with it.
pub async fn servers(
    http: &reqwest::Client,
    client_id: &str,
    token: &str,
) -> Result<Vec<Resource>> {
    let mut headers = identity(client_id);
    headers.insert("x-plex-token", HeaderValue::from_str(token)?);
    let resources: Vec<Resource> = http
        .get(format!("{API}/resources"))
        .headers(headers)
        .query(&[("includeHttps", "1"), ("includeRelay", "1")])
        .timeout(TIMEOUT)
        .send()
        .await
        .context("cannot reach plex.tv")?
        .error_for_status()
        .context("plex.tv refused to list the servers")?
        .json()
        .await
        .context("cannot read the plex.tv servers")?;
    Ok(resources
        .into_iter()
        .filter(|resource| resource.provides.split(',').any(|kind| kind == "server"))
        .collect())
}

/// The first of a server's connections that answers, trying the ones on the local network before
/// the ones across the internet and the relay last. The address has no trailing slash.
pub async fn reach(
    http: &reqwest::Client,
    client_id: &str,
    server_token: &str,
    connections: &[Connection],
) -> Result<String> {
    let mut ordered: Vec<&Connection> = connections
        .iter()
        .filter(|connection| !connection.uri.is_empty())
        .collect();
    ordered.sort_by_key(|connection| (connection.relay, !connection.local, connection.ipv6));
    for connection in ordered {
        let uri = connection.uri.trim_end_matches('/');
        let mut headers = identity(client_id);
        headers.insert("x-plex-token", HeaderValue::from_str(server_token)?);
        let answered = http
            .get(format!("{uri}/identity"))
            .headers(headers)
            .timeout(PROBE)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        match answered {
            Ok(_) => return Ok(uri.to_owned()),
            Err(error) => log::info!("plex: {uri} did not answer: {error}"),
        }
    }
    anyhow::bail!("the plex server could not be reached at any of its addresses")
}
