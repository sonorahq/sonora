//! Spotify OAuth and refresh grants.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, anyhow};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{StatusCode, Url};
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::http;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const EXPIRY_MARGIN: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CALLBACK_LIMIT: u64 = 8192;

const SCOPES: &[&str] = &[
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

/// Why Spotify could not complete OAuth.
#[derive(Clone, Copy, Debug)]
pub enum AuthError {
    Cancelled,
    Credentials,
    Network,
    Refused,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::Cancelled => "Spotify sign-in was cancelled",
            Self::Credentials => "Spotify rejected the credential",
            Self::Network => "cannot reach Spotify Accounts",
            Self::Refused => "Spotify refused sign-in",
        })
    }
}

impl std::error::Error for AuthError {}

/// Wakes an abandoned OAuth callback listener at the configured redirect.
pub fn release(redirect_uri: &str) {
    let Some(address) = Url::parse(redirect_uri)
        .ok()
        .as_ref()
        .and_then(socket_address)
    else {
        log::warn!("auth: cannot read a socket address from {}", redirect_uri);
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

/// Exchanges a browser callback with a matching OAuth state for an access token.
async fn authorize(
    client_id: &str,
    redirect_uri: &str,
    show_url: impl FnOnce(&str),
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
    show_url(url.as_str());

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
            return Err(AuthError::Cancelled.into());
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
            Err(problem) => Err(problem.into()),
        };
        let (status, body) = match token.is_ok() {
            true => ("200 OK", "Sign-in complete. You can return to the app."),
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
) -> Option<Result<String, AuthError>> {
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
            "access_denied" => AuthError::Cancelled,
            _ => AuthError::Refused,
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

/// Serializes renewals across the provider and the independent lyrics connection.
static REFRESH: Mutex<()> = Mutex::const_new(());
/// Orders credential replacement against sign-out.
static FILE: SyncMutex<()> = SyncMutex::new(());

/// The grant received from Spotify's authorization-code and refresh-token exchanges.
#[derive(Deserialize)]
struct TokenResponse {
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
pub struct OAuth {
    path: PathBuf,
    client_id: String,
    username: String,
    token: Arc<Mutex<Option<Token>>>,
    modified: SyncMutex<SystemTime>,
}

impl OAuth {
    pub fn stored(path: &Path) -> bool {
        saved(path).is_ok_and(|stored| stored.is_some())
    }

    pub fn restore(path: PathBuf, client_id: &str) -> Result<Option<Arc<Self>>> {
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        let Some(stored) = saved(&path)? else {
            return Ok(None);
        };
        if stored.client_id != client_id
            || stored.username.is_empty()
            || stored.refresh_token.is_empty()
        {
            return Err(AuthError::Credentials.into());
        }
        Self::new(path, stored, None).map(Some)
    }

    /// Authorizes in a browser and saves the refresh grant at the caller's credential path.
    pub async fn login(
        path: PathBuf,
        client_id: &str,
        redirect_uri: &str,
        show_url: impl FnOnce(&str),
    ) -> Result<Arc<Self>> {
        let response = authorize(client_id, redirect_uri, show_url).await?;
        let refresh_token = response
            .refresh_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .context("Spotify omitted the refresh token")?;
        let stored = Stored {
            client_id: client_id.to_owned(),
            username: response.username()?.to_owned(),
            refresh_token: refresh_token.to_owned(),
        };
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        write(&path, &stored)?;
        Self::new(path, stored, Some(Token::from(response)))
    }

    fn new(path: PathBuf, grant: Stored, token: Option<Token>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            modified: SyncMutex::new(modified(&path)?),
            path,
            client_id: grant.client_id,
            username: grant.username,
            token: Arc::new(Mutex::new(token)),
        }))
    }

    pub fn forget(path: &Path) {
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        if let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("spotify: cannot remove saved credentials: {error}");
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Detects sign-out and external changes while accepting our own token rotations.
    pub fn has_current_grant(&self) -> bool {
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
    pub async fn token(self: &Arc<Self>) -> Result<String> {
        let auth = self.clone();
        self.token_using(async move { auth.refresh().await }).await
    }

    async fn token_using(
        &self,
        refresh: impl Future<Output = Result<Token>> + Send + 'static,
    ) -> Result<String> {
        let mut cached = self.token.clone().lock_owned().await;
        if let Some(token) = cached
            .as_ref()
            .filter(|token| token.expires_at > Instant::now())
        {
            return Ok(token.access_token.clone());
        }

        // Spotify can rotate the grant after the caller drops its request future.
        tokio::spawn(async move {
            let token = refresh.await?;
            let access_token = token.access_token.clone();
            *cached = Some(token);
            Ok(access_token)
        })
        .await
        .context("Spotify token renewal task failed")?
    }

    async fn refresh(&self) -> Result<Token> {
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
            return Err(AuthError::Credentials.into());
        }
        if let Some(next) = response
            .refresh_token
            .as_deref()
            .filter(|next| !next.is_empty() && *next != stored.refresh_token)
        {
            self.persist_rotated_grant(&mut stored, next)?;
        }
        Ok(Token::from(response))
    }

    fn persist_rotated_grant(&self, stored: &mut Stored, refresh_token: &str) -> Result<()> {
        let _file = FILE.lock().expect("Spotify credential lock poisoned");
        let current = read(&self.path)?;
        anyhow::ensure!(
            current.client_id == stored.client_id
                && current.username == stored.username
                && current.refresh_token == stored.refresh_token,
            "Spotify sign-in changed during token renewal"
        );
        refresh_token.clone_into(&mut stored.refresh_token);
        write(&self.path, stored)?;
        *self
            .modified
            .lock()
            .expect("Spotify grant timestamp lock poisoned") = modified(&self.path)?;
        Ok(())
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
async fn exchange<const N: usize>(form: [(&str, &str); N]) -> Result<TokenResponse> {
    let response = http::request(|client| client.post(TOKEN_URL).form(&form.as_slice()))
        .await
        .map_err(|error| error.without_url())
        .context(AuthError::Network)?;
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
            return Err(AuthError::Credentials.into());
        }
        return Err(AuthError::Refused.into());
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

fn saved(path: &Path) -> Result<Option<Stored>> {
    match std::fs::read(path) {
        Ok(body) => serde_json::from_slice(&body)
            .map(Some)
            .context("cannot decode the Spotify refresh grant"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("cannot read the Spotify refresh grant"),
    }
}

fn write(path: &Path, stored: &Stored) -> Result<()> {
    use std::io::Write as _;

    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut directory = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        directory.mode(0o700);
    }
    directory.recursive(true).create(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec(stored)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    #[cfg(unix)]
    if let Err(error) = std::fs::File::open(parent).and_then(|directory| directory.sync_all()) {
        log::debug!("spotify: cannot sync credential directory: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rotated_grant_survives_cancelled_caller() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("spotify-oauth.json");
        let grant = Stored {
            client_id: "client".into(),
            username: "listener".into(),
            refresh_token: "original".into(),
        };
        write(&path, &grant)?;
        let auth = OAuth::new(path.clone(), grant, None)?;
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (respond, response) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn({
            let auth = auth.clone();
            let writer = auth.clone();
            let mut stored = read(&path)?;
            async move {
                auth.token_using(async move {
                    let _ = started.send(());
                    response.await?;
                    writer.persist_rotated_grant(&mut stored, "rotated")?;
                    Ok(Token {
                        access_token: "access".into(),
                        expires_at: Instant::now() + Duration::from_secs(3600),
                    })
                })
                .await
            }
        });

        tokio::time::timeout(Duration::from_secs(5), async {
            waiting.await?;
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            respond.send(()).expect("refresh survives cancellation");

            let access = auth
                .token_using(async { anyhow::bail!("should reuse the renewed token") })
                .await?;
            assert_eq!(access, "access");
            assert_eq!(read(&path)?.refresh_token, "rotated");
            Ok::<_, anyhow::Error>(())
        })
        .await?
    }

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
