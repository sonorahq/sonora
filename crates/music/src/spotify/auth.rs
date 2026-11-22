use std::path::{Path, PathBuf};
use std::sync::Mutex as SyncMutex;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, anyhow};

use crate::{PromptSink, SignInFailure, SignInProblem, SignInPrompt, credentials};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{StatusCode, Url};
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::{Deserialize, Serialize};

use super::{session::Session, transport};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

pub const DEFAULT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const EXPIRY_MARGIN: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CALLBACK_LIMIT: u64 = 8192;

pub const SCOPES: &[&str] = &[
    "app-remote-control",
    "playlist-read-collaborative",
    "playlist-read-private",
    "streaming",
    "user-follow-read",
    "user-library-read",
    "user-read-email",
    "user-read-playback-state",
    "user-read-private",
    "user-read-recently-played",
    "user-top-read",
];

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub client_id: String,
    pub redirect_uri: String,
    pub cache_dir: PathBuf,
}

/// Shares a restored Spotify session across the provider and lyrics lookups.
pub(super) struct Account {
    pub(super) config: AuthConfig,
    session: Mutex<Option<Session>>,
}

impl Account {
    pub(super) fn new(config: AuthConfig) -> Self {
        Self {
            config,
            session: Mutex::new(None),
        }
    }

    pub(super) async fn login(&self, prompt: PromptSink) -> Result<Session> {
        let token = authorize(&self.config.client_id, &self.config.redirect_uri, prompt).await?;
        let mut held = self.session.lock().await;
        let device = device_id(&self.config.cache_dir)?;
        let oauth = OAuth::login(&self.config, token)?;
        let session = Session::new(oauth, device);
        *held = Some(session.clone());
        Ok(session)
    }

    pub(super) async fn session(&self) -> Result<Option<Session>> {
        let mut held = self.session.lock().await;
        if let Some(session) = held.as_ref()
            && session.has_current_grant()
        {
            return Ok(Some(session.clone()));
        }
        *held = None;
        *held = restore(&self.config).await?;
        Ok(held.clone())
    }

    pub(super) fn forget(&self) {
        OAuth::forget(&self.config);
        if let Ok(mut held) = self.session.try_lock() {
            *held = None;
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.to_owned(),
            redirect_uri: DEFAULT_REDIRECT_URI.to_owned(),
            cache_dir: credentials::dir("spotify"),
        }
    }
}

impl AuthConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(redirect_uri) = std::env::var("SONORA_REDIRECT_URI") {
            config.redirect_uri = redirect_uri;
        }
        config
    }

    /// Whether a Spotify OAuth sign-in has been saved.
    pub fn stored(&self) -> bool {
        OAuth::stored(self)
    }

    pub(super) fn oauth_file(&self) -> PathBuf {
        self.cache_dir.join("spotify-oauth.json")
    }
}

pub fn release(config: &AuthConfig) {
    let Some(address) = Url::parse(&config.redirect_uri)
        .ok()
        .as_ref()
        .and_then(socket_address)
    else {
        log::warn!(
            "auth: cannot read a socket address from {}",
            config.redirect_uri
        );
        return;
    };
    let Ok(mut stream) = std::net::TcpStream::connect(address) else {
        return;
    };
    let _ = std::io::Write::write_all(&mut stream, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
}

fn socket_address(uri: &Url) -> Option<String> {
    if uri.scheme() != "http" {
        return None;
    }
    Some(format!("{}:{}", uri.host()?, uri.port_or_known_default()?))
}

pub async fn restore(config: &AuthConfig) -> Result<Option<Session>> {
    let Some(oauth) = OAuth::restore(config)? else {
        return Ok(None);
    };
    oauth.token().await?;
    Ok(Some(Session::new(oauth, device_id(&config.cache_dir)?)))
}

/// Exchanges a browser callback with a matching OAuth state for an access token.
async fn authorize(
    client_id: &str,
    redirect_uri: &str,
    prompt: PromptSink,
) -> Result<TokenResponse> {
    let redirect = Url::parse(redirect_uri)?;
    let mut url = Url::parse("https://accounts.spotify.com/authorize")?;
    let state = random_secret()?;
    let verifier = random_secret()?;
    let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()));
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("scope", &SCOPES.join(" ")),
        ("state", &state),
        ("code_challenge_method", "S256"),
        ("code_challenge", &challenge),
    ]);
    let address = socket_address(&redirect)
        .with_context(|| format!("redirect URI has no socket address: {redirect_uri}"))?;
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("cannot listen for Spotify login at {address}"))?;
    prompt(SignInPrompt::Url(url.to_string()));
    open::that_in_background(url.as_str());

    loop {
        let (socket, _) = listener.accept().await?;
        let mut stream = BufReader::new(socket);
        let mut request = String::new();
        if !matches!(
            tokio::time::timeout(
                REQUEST_TIMEOUT,
                (&mut stream).take(CALLBACK_LIMIT).read_line(&mut request),
            )
            .await,
            Ok(Ok(_))
        ) {
            continue;
        }
        // release() wakes an abandoned listener with this request.
        if request == "GET / HTTP/1.1\r\n" {
            return Err(SignInFailure(SignInProblem::Cancelled).into());
        }
        let Some(callback) = callback_code(&request, &redirect, &state) else {
            let _ = stream
                .get_mut()
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                )
                .await;
            continue;
        };
        let token = match callback {
            Ok(code) => {
                exchange([
                    ("grant_type", "authorization_code"),
                    ("client_id", client_id),
                    ("redirect_uri", redirect_uri),
                    ("code", &code),
                    ("code_verifier", &verifier),
                ])
                .await
            }
            Err(problem) => Err(SignInFailure(problem).into()),
        };
        let (status, body) = match token.is_ok() {
            true => ("200 OK", "You can return to Sonora."),
            false => ("400 Bad Request", ""),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.get_mut().write_all(response.as_bytes()).await;
        return token;
    }
}

/// Accepts only GET requests to the configured path with the outstanding OAuth state.
fn callback_code(
    request: &str,
    redirect: &Url,
    expected_state: &str,
) -> Option<Result<String, SignInProblem>> {
    let mut parts = request.strip_suffix('\n')?.split_ascii_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let (path, query) = parts.next()?.split_once('?')?;
    if path != redirect.path() {
        return None;
    }
    let mut callback = redirect.clone();
    callback.set_query(Some(query));
    let mut state = callback.query_pairs().filter(|(key, _)| key == "state");
    if state.next()?.1 != expected_state || state.next().is_some() {
        return None;
    }
    if let Some((_, error)) = callback.query_pairs().find(|(key, _)| key == "error") {
        return Some(Err(match error.as_ref() {
            "access_denied" => SignInProblem::Cancelled,
            _ => SignInProblem::Refused,
        }));
    }
    callback
        .query_pairs()
        .find(|(key, code)| key == "code" && !code.is_empty())
        .map(|(_, code)| Ok(code.into_owned()))
}

/// Generates a URL-safe PKCE verifier or callback state from the OS random source.
fn random_secret() -> Result<String> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow!("cannot generate Spotify OAuth secret"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The id this install goes by on Spotify's device list. A new one each launch would leave a
/// stale Sonora behind every time, so the first one is kept next to the credentials.
fn device_id(dir: &Path) -> Result<String> {
    let file = dir.join("device-id");
    let kept = std::fs::read_to_string(&file)
        .map(|id| id.trim().to_owned())
        .ok()
        .filter(|id| !id.is_empty());
    if let Some(id) = kept {
        return Ok(id);
    }

    let mut bytes = [0; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow!("cannot generate Spotify device ID"))?;
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let id = format!("{:032x}", u128::from_be_bytes(bytes));
    let id = format!(
        "{}-{}-{}-{}-{}",
        &id[..8],
        &id[8..12],
        &id[12..16],
        &id[16..20],
        &id[20..]
    );
    if let Err(error) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, &id)) {
        log::warn!("auth: cannot keep the device id: {error}");
    }
    Ok(id)
}

/// Serializes renewals across the provider and the independent lyrics connection.
static REFRESH: Mutex<()> = Mutex::const_new(());
/// Orders credential replacement against sign-out.
static FILE: SyncMutex<()> = SyncMutex::new(());

/// The grant received from Spotify's authorization-code and refresh-token exchanges.
#[derive(Deserialize)]
pub(super) struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    username: Option<String>,
}

impl TokenResponse {
    fn username(&self) -> Result<&str> {
        self.username
            .as_deref()
            .filter(|username| !username.is_empty())
            .context("Spotify omitted the account username")
    }
}

/// A refresh grant belongs to the OAuth client that originally obtained it.
#[derive(Deserialize, Serialize)]
struct Stored {
    client_id: String,
    username: String,
    refresh_token: String,
}

/// An access token and the point at which requests should start using a fresh one.
struct Token {
    access_token: String,
    expires_at: Instant,
}

/// Renews the Spotify account's access token through Spotify Accounts.
pub(super) struct OAuth {
    path: PathBuf,
    client_id: String,
    username: String,
    token: Mutex<Option<Token>>,
    modified: SyncMutex<SystemTime>,
}

impl OAuth {
    pub(super) fn stored(config: &AuthConfig) -> bool {
        saved(config).is_ok_and(|stored| stored.is_some())
    }

    pub(super) fn restore(config: &AuthConfig) -> Result<Option<Self>> {
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        let path = config.oauth_file();
        let Some(stored) = saved(config)? else {
            return Ok(None);
        };
        if stored.client_id != config.client_id
            || stored.username.is_empty()
            || stored.refresh_token.is_empty()
        {
            return Err(SignInFailure(SignInProblem::Credentials).into());
        }
        Self::new(path, stored, None).map(Some)
    }

    pub(super) fn login(config: &AuthConfig, response: TokenResponse) -> Result<Self> {
        let path = config.oauth_file();
        let refresh_token = response
            .refresh_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .context("Spotify omitted the refresh token")?;
        let stored = Stored {
            client_id: config.client_id.clone(),
            username: response.username()?.to_owned(),
            refresh_token: refresh_token.to_owned(),
        };
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        write(&path, &stored)?;
        Self::new(path, stored, Some(Token::from(response)))
    }

    fn new(path: PathBuf, grant: Stored, token: Option<Token>) -> Result<Self> {
        Ok(Self {
            modified: SyncMutex::new(modified(&path)?),
            path,
            client_id: grant.client_id,
            username: grant.username,
            token: Mutex::new(token),
        })
    }

    pub(super) fn forget(config: &AuthConfig) {
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        credentials::remove(&config.oauth_file());
    }

    pub(super) fn username(&self) -> &str {
        &self.username
    }

    pub(super) fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Detects sign-out and external changes while accepting our own token rotations.
    pub(super) fn has_current_grant(&self) -> bool {
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        modified(&self.path).is_ok_and(|current| {
            current
                == *self
                    .modified
                    .lock()
                    .expect("Spotify grant timestamp lock poisoned")
        })
    }

    /// Uses the cached bearer until it expires. Rotated refresh grants are persisted atomically.
    pub(super) async fn token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached
            .as_ref()
            .filter(|token| token.expires_at > Instant::now())
        {
            return Ok(token.access_token.clone());
        }

        let _refresh = REFRESH.lock().await;
        let mut stored = read(&self.path)?;
        anyhow::ensure!(
            stored.client_id == self.client_id && stored.username == self.username,
            "Spotify sign-in changed"
        );
        let response = exchange([
            ("grant_type", "refresh_token"),
            ("client_id", &self.client_id),
            ("refresh_token", &stored.refresh_token),
        ])
        .await?;
        if response
            .username
            .as_deref()
            .is_some_and(|username| username != self.username)
        {
            return Err(SignInFailure(SignInProblem::Credentials).into());
        }
        if let Some(next) = response
            .refresh_token
            .as_deref()
            .filter(|next| !next.is_empty() && *next != stored.refresh_token)
        {
            let _file = FILE.lock().expect("Spotify credential lock poisoned");
            let current = read(&self.path)?;
            anyhow::ensure!(
                current.client_id == stored.client_id
                    && current.username == stored.username
                    && current.refresh_token == stored.refresh_token,
                "Spotify sign-in changed during token renewal"
            );
            next.clone_into(&mut stored.refresh_token);
            write(&self.path, &stored)?;
            *self
                .modified
                .lock()
                .expect("Spotify grant timestamp lock poisoned") = modified(&self.path)?;
        }
        let token = Token::from(response);
        let access_token = token.access_token.clone();
        *cached = Some(token);
        Ok(access_token)
    }
}

impl From<TokenResponse> for Token {
    fn from(response: TokenResponse) -> Self {
        let lifetime = Duration::from_secs(response.expires_in).saturating_sub(EXPIRY_MARGIN);
        let now = Instant::now();
        Self {
            access_token: response.access_token,
            expires_at: now.checked_add(lifetime).unwrap_or(now),
        }
    }
}

/// Exchanges an authorization code or renews a refresh grant at Spotify Accounts.
pub(super) async fn exchange<const N: usize>(form: [(&str, &str); N]) -> Result<TokenResponse> {
    let response = transport::request(|client| client.post(TOKEN_URL).form(&form.as_slice()))
        .await
        .map_err(|error| error.without_url())
        .context(SignInFailure(SignInProblem::Network))?;
    if response.status() == StatusCode::BAD_REQUEST {
        #[derive(Deserialize)]
        struct Rejection {
            error: String,
        }

        if response
            .json::<Rejection>()
            .await
            .is_ok_and(|body| body.error == "invalid_grant")
        {
            return Err(SignInFailure(SignInProblem::Credentials).into());
        }
        return Err(SignInFailure(SignInProblem::Refused).into());
    }
    anyhow::ensure!(
        response.status().is_success(),
        "Spotify token exchange returned {}",
        response.status()
    );
    let token = response.json::<TokenResponse>().await?;
    anyhow::ensure!(
        !token.access_token.is_empty(),
        "Spotify omitted the access token"
    );
    Ok(token)
}

fn modified(path: &Path) -> Result<SystemTime> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .context("cannot read the Spotify refresh grant timestamp")
}

fn read(path: &Path) -> Result<Stored> {
    let body = std::fs::read(path).context("cannot read the Spotify refresh grant")?;
    serde_json::from_slice(&body).context("cannot decode the Spotify refresh grant")
}

fn saved(config: &AuthConfig) -> Result<Option<Stored>> {
    match std::fs::read(config.oauth_file()) {
        Ok(body) => serde_json::from_slice(&body)
            .map(Some)
            .context("cannot decode the Spotify refresh grant"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("cannot read the Spotify refresh grant"),
    }
}

fn write(path: &Path, stored: &Stored) -> Result<()> {
    credentials::write(path, &serde_json::to_vec(stored)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_http_callback_addresses() {
        for (uri, expected) in [
            ("http://127.0.0.1:8989/login", Some("127.0.0.1:8989")),
            ("http://localhost/login", Some("localhost:80")),
            ("https://localhost/login", None),
        ] {
            assert_eq!(
                socket_address(&Url::parse(uri).unwrap()).as_deref(),
                expected
            );
        }
    }
}
