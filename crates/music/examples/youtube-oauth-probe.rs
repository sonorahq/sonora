use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, RedirectUrl, Scope, TokenResponse as _, TokenUrl, basic::BasicClient,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use ytmusic::Client;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const YOUTUBE_SCOPE: &str = "https://www.googleapis.com/auth/youtube";
const CALLBACK_PATH: &str = "/oauth2/callback";
const CLIENTS: &[Client] = &[Client::Music, Client::Web, Client::Tv];

#[derive(Clone, Copy)]
enum Case {
    Profile,
    LikedSongs,
    OwnedPlaylist,
    Playback,
    NoopMutation,
    BrandAccounts,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::LikedSongs => "liked songs",
            Self::OwnedPlaylist => "owned playlist",
            Self::Playback => "playback",
            Self::NoopMutation => "no-op mutation",
            Self::BrandAccounts => "Brand Accounts",
        }
    }
}

struct ProbeTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<Duration>,
}

struct ProbeOptions<'a> {
    case: Case,
    authuser: usize,
    email: Option<&'a str>,
    page: Option<&'a str>,
}

#[derive(Debug)]
struct Outcome {
    status: reqwest::StatusCode,
    accepted: &'static str,
    identity: &'static str,
    shape: String,
    error: Option<String>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let client_id = std::env::var("SONORA_YOUTUBE_OAUTH_CLIENT_ID")
        .context("set SONORA_YOUTUBE_OAUTH_CLIENT_ID to a Google desktop OAuth client id")?;
    let client_secret = std::env::var("SONORA_YOUTUBE_OAUTH_CLIENT_SECRET")
        .ok()
        .filter(|secret| !secret.trim().is_empty());
    let playlist_id = std::env::var("SONORA_YOUTUBE_PROBE_PLAYLIST_ID")
        .context("set SONORA_YOUTUBE_PROBE_PLAYLIST_ID to an owned playlist id")?;
    let video_id = std::env::var("SONORA_YOUTUBE_PROBE_VIDEO_ID")
        .context("set SONORA_YOUTUBE_PROBE_VIDEO_ID to a playable video id")?;
    let authuser = std::env::var("SONORA_YOUTUBE_PROBE_AUTHUSER")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .context("SONORA_YOUTUBE_PROBE_AUTHUSER must be a non-negative integer")
        })
        .transpose()?
        .unwrap_or(0);
    let expected_email = std::env::var("SONORA_YOUTUBE_PROBE_EMAIL").ok();
    let expected_page = std::env::var("SONORA_YOUTUBE_PROBE_PAGE_ID").ok();

    println!("Starting the Google desktop authorization flow in the system browser.");
    println!("Tokens stay in memory and are never written by this probe.");
    let tokens = authorize(client_id, client_secret).await?;
    println!(
        "Authorization succeeded: refresh token {}, access token expires in {}.",
        match tokens.refresh_token.is_some() {
            true => "received",
            false => "not returned",
        },
        tokens.expires_in.map_or_else(
            || "an unknown interval".to_owned(),
            |value| format!("{value:?}")
        ),
    );

    let http = reqwest::Client::builder()
        .build()
        .context("cannot build the probe HTTP client")?;
    println!();
    println!(
        "{:<14} {:<16} {:<22} {:<8} {:<9} {:<10} shape / error",
        "client", "case", "endpoint", "status", "bearer", "identity"
    );

    for client in CLIENTS {
        for case in [
            Case::Profile,
            Case::LikedSongs,
            Case::OwnedPlaylist,
            Case::Playback,
            Case::NoopMutation,
            Case::BrandAccounts,
        ] {
            let (endpoint, payload) = request(case, *client, &playlist_id, &video_id);
            let started = Instant::now();
            let outcome = send(
                &http,
                &tokens.access_token,
                *client,
                endpoint,
                payload,
                ProbeOptions {
                    case,
                    authuser,
                    email: expected_email.as_deref(),
                    page: expected_page.as_deref(),
                },
            )
            .await;
            match outcome {
                Ok(outcome) => println!(
                    "{:<14} {:<16} {:<22} {:<8} {:<9} {:<10} {} ({:?})",
                    client.name(),
                    case.name(),
                    endpoint,
                    outcome.status,
                    outcome.accepted,
                    outcome.identity,
                    outcome.error.as_deref().unwrap_or(&outcome.shape),
                    started.elapsed(),
                ),
                Err(error) => println!(
                    "{:<14} {:<16} {:<22} {:<8} {:<9} {:<10} {} ({:?})",
                    client.name(),
                    case.name(),
                    endpoint,
                    "network",
                    "unknown",
                    "unknown",
                    error,
                    started.elapsed(),
                ),
            }
        }
    }

    println!();
    println!(
        "A successful response means the bearer was accepted by that request; profile and Brand Account rows are the stronger identity checks."
    );
    println!(
        "No-op mutation sends an empty action list and is intended to test authorization without changing the playlist."
    );
    Ok(())
}

async fn authorize(client_id: String, client_secret: Option<String>) -> Result<ProbeTokens> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("cannot bind the OAuth loopback callback")?;
    let port = listener
        .local_addr()
        .context("cannot read the OAuth callback address")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let client = oauth_client(client_id, client_secret, &redirect_uri)?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(YOUTUBE_SCOPE.to_owned()))
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .set_pkce_challenge(challenge)
        .url();

    open::that_in_background(url.as_str());
    let code = callback(listener, state.secret()).await?;
    let response = tokio::task::spawn_blocking(move || {
        client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(verifier)
            .request(&exchange)
    })
    .await
    .context("the OAuth token exchange task did not finish")?
    .map_err(|error| anyhow::anyhow!("Google rejected the authorization code: {error}"))?;

    Ok(ProbeTokens {
        access_token: response.access_token().secret().to_owned(),
        refresh_token: response
            .refresh_token()
            .map(|token| token.secret().to_owned()),
        expires_in: response.expires_in(),
    })
}

fn oauth_client(
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: &str,
) -> Result<BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>> {
    let client = BasicClient::new(ClientId::new(client_id))
        .set_auth_uri(AuthUrl::new(AUTH_URL.to_owned())?)
        .set_token_uri(TokenUrl::new(TOKEN_URL.to_owned())?)
        .set_redirect_uri(RedirectUrl::new(redirect_uri.to_owned())?);
    Ok(match client_secret {
        Some(secret) => client.set_client_secret(ClientSecret::new(secret)),
        None => client,
    })
}

async fn callback(listener: tokio::net::TcpListener, expected_state: &str) -> Result<String> {
    let (stream, _) = listener
        .accept()
        .await
        .context("the Google OAuth callback did not arrive")?;
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    reader
        .read_line(&mut request)
        .await
        .context("cannot read the Google OAuth callback")?;
    let target = request
        .split_whitespace()
        .nth(1)
        .context("the Google OAuth callback was malformed")?;
    let callback = oauth2::url::Url::parse(&format!("http://localhost{target}"))?;
    let query: HashMap<_, _> = callback.query_pairs().into_owned().collect();
    let mut stream = reader.into_inner();

    if let Some(error) = query.get("error") {
        write_response(&mut stream, "Authorization was not completed.").await?;
        let description = query.get("error_description").map(String::as_str);
        bail!(
            "Google authorization failed: {error}{}",
            description.map_or_else(String::new, |value| format!(" ({value})"))
        );
    }
    let state = query
        .get("state")
        .context("the Google OAuth callback had no state")?;
    if state != expected_state {
        write_response(&mut stream, "Authorization state did not match.").await?;
        bail!("the Google OAuth callback state did not match");
    }
    let code = query
        .get("code")
        .context("the Google OAuth callback had no authorization code")?
        .to_owned();
    write_response(&mut stream, "You can return to the probe.").await?;
    Ok(code)
}

async fn write_response(stream: &mut tokio::net::TcpStream, body: &str) -> Result<()> {
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: text/plain\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .context("cannot answer the Google OAuth callback")
}

fn request(case: Case, client: Client, playlist_id: &str, video_id: &str) -> (&'static str, Value) {
    match case {
        Case::Profile => ("account/account_menu", json!({})),
        Case::LikedSongs => ("browse", json!({ "browseId": "VLLM" })),
        Case::OwnedPlaylist => (
            "browse",
            json!({ "browseId": format!("VL{}", playlist_id.trim_start_matches("VL")) }),
        ),
        Case::Playback => (
            "player",
            json!({
                "videoId": video_id,
                "racyCheckOk": true,
                "contentCheckOk": true,
                "playbackContext": {
                    "contentPlaybackContext": { "html5Preference": "HTML5_PREF_WANTS" }
                }
            }),
        ),
        Case::NoopMutation => (
            "browse/edit_playlist",
            json!({
                "playlistId": playlist_id.trim_start_matches("VL"),
                "actions": []
            }),
        ),
        Case::BrandAccounts => (
            "account/accounts_list",
            match client {
                Client::Web => {
                    json!({ "requestType": "ACCOUNTS_LIST_REQUEST_TYPE_ACCOUNT_SWITCHER" })
                }
                _ => json!({}),
            },
        ),
    }
}

async fn send(
    http: &reqwest::Client,
    access_token: &str,
    client: Client,
    endpoint: &str,
    payload: Value,
    options: ProbeOptions<'_>,
) -> Result<Outcome> {
    let (base, origin) = match client {
        Client::Music => (
            "https://music.youtube.com/youtubei/v1/",
            "https://music.youtube.com",
        ),
        _ => (
            "https://www.youtube.com/youtubei/v1/",
            "https://www.youtube.com",
        ),
    };
    let builder = http
        .post(format!("{base}{endpoint}?prettyPrint=false&alt=json"))
        .header("Accept", "*/*")
        .header("Accept-Language", "*")
        .header("Content-Type", "application/json")
        .header("Origin", origin)
        .header("User-Agent", client.user_agent())
        .header("X-Youtube-Client-Name", client.id().to_string())
        .header("X-Youtube-Client-Version", client.version())
        .header("Authorization", format!("Bearer {access_token}"))
        .header("X-Origin", origin)
        .header("X-Goog-AuthUser", options.authuser.to_string())
        .json(&with_context(payload, client));
    let builder = match options.page {
        Some(page) if !page.trim().is_empty() => builder.header("X-Goog-PageId", page),
        _ => builder,
    };
    let response = builder
        .send()
        .await
        .with_context(|| format!("cannot reach {endpoint}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .with_context(|| format!("cannot read {endpoint} response"))?;
    let value: Value = serde_json::from_slice(&body)
        .with_context(|| format!("{endpoint} returned a non-JSON response"))?;
    let error = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(|message| truncate(message, 160));
    let accepted = match (
        status.is_success(),
        error.is_none(),
        contains_string(&value, "LOGIN_REQUIRED"),
    ) {
        (true, true, false) => "yes",
        _ => "no",
    };
    Ok(Outcome {
        status,
        accepted,
        identity: identity(&value, options),
        shape: format!("{} bytes; {}", body.len(), shape(&value)),
        error,
    })
}

fn with_context(mut payload: Value, client: Client) -> Value {
    payload["context"] = client.context("", "en", "US");
    if client == Client::Music {
        payload["isAudioOnly"] = json!(true);
    }
    payload
}

fn identity(value: &Value, options: ProbeOptions<'_>) -> &'static str {
    if !matches!(options.case, Case::Profile | Case::BrandAccounts)
        || (options.email.is_none() && options.page.is_none())
    {
        return "unchecked";
    }
    let body = value.to_string();
    if options.email.is_some_and(|email| !body.contains(email))
        || options.page.is_some_and(|page| !body.contains(page))
    {
        "mismatch"
    } else {
        "match"
    }
}

fn shape(value: &Value) -> String {
    let keys = value
        .as_object()
        .map(|object| object.keys().take(8).cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| value_type(value).to_owned());
    let mut renderers = BTreeSet::new();
    collect_renderers(value, &mut renderers);
    match renderers.is_empty() {
        true => format!("keys={keys}"),
        false => format!(
            "keys={keys}; renderers={}",
            renderers.into_iter().collect::<Vec<_>>().join(",")
        ),
    }
}

fn collect_renderers(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key.ends_with("Renderer") && found.len() < 8 {
                    found.insert(key.clone());
                }
                collect_renderers(value, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_renderers(value, found);
            }
        }
        _ => {}
    }
}

fn contains_string(value: &Value, wanted: &str) -> bool {
    match value {
        Value::String(text) => text == wanted,
        Value::Array(values) => values.iter().any(|value| contains_string(value, wanted)),
        Value::Object(object) => object.values().any(|value| contains_string(value, wanted)),
        _ => false,
    }
}

fn truncate(text: &str, limit: usize) -> String {
    let mut text = text.to_owned();
    text.truncate(limit);
    text
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn exchange(request: oauth2::HttpRequest) -> Result<oauth2::HttpResponse, reqwest::Error> {
    let sent = reqwest::blocking::Client::new().execute(request.try_into()?)?;
    let status = sent.status();
    let headers = sent.headers().clone();
    let mut response = http::Response::new(sent.bytes()?.to_vec());
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_requires_every_supplied_marker() {
        let value = json!({ "email": "account@example.com", "pageId": "brand-page" });
        assert_eq!(
            identity(
                &value,
                ProbeOptions {
                    case: Case::Profile,
                    authuser: 0,
                    email: Some("account@example.com"),
                    page: Some("brand-page"),
                }
            ),
            "match"
        );
        assert_eq!(
            identity(
                &value,
                ProbeOptions {
                    case: Case::BrandAccounts,
                    authuser: 0,
                    email: Some("account@example.com"),
                    page: Some("other-page"),
                }
            ),
            "mismatch"
        );
    }

    #[test]
    fn identity_is_unchecked_without_a_strong_identity_case() {
        let value = json!({ "email": "account@example.com" });
        assert_eq!(
            identity(
                &value,
                ProbeOptions {
                    case: Case::Playback,
                    authuser: 0,
                    email: Some("account@example.com"),
                    page: None,
                }
            ),
            "unchecked"
        );
    }

    #[test]
    fn request_matrix_uses_the_current_private_operations() {
        let (endpoint, payload) = request(Case::LikedSongs, Client::Music, "playlist", "video");
        assert_eq!(endpoint, "browse");
        assert_eq!(payload["browseId"], "VLLM");

        let (endpoint, payload) = request(Case::NoopMutation, Client::Music, "VLplaylist", "video");
        assert_eq!(endpoint, "browse/edit_playlist");
        assert_eq!(payload["playlistId"], "playlist");
        assert_eq!(payload["actions"], json!([]));
    }
}
