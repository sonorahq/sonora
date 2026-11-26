//! Spotify OAuth and refresh grants.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as SyncMutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use reqwest::{StatusCode, Url};
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::http;
use crate::proto::login5;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const EXPIRY_MARGIN: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(20);
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

fn socket_address(uri: &Url) -> Option<std::net::SocketAddr> {
    use std::net::IpAddr;

    if uri.scheme() != "http" {
        return None;
    }
    let host = uri.host_str()?;
    let ip: IpAddr = host.trim_matches(['[', ']']).parse().ok()?;
    ip.is_loopback()
        .then_some(std::net::SocketAddr::new(ip, uri.port_or_known_default()?))
}

/// Exchanges a browser callback with a matching OAuth state for an access token.
async fn authorize(
    client_id: &str,
    redirect_uri: &str,
    show_url: impl FnOnce(&str),
) -> Result<(Result<TokenResponse>, tokio::net::TcpStream)> {
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
    let address = socket_address(&redirect).with_context(|| {
        format!("redirect URI must use an HTTP literal loopback IP: {redirect_uri}")
    })?;
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
        return Ok((token, stream.into_inner()));
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
static FILE: SyncMutex<Vec<Weak<PathBuf>>> = SyncMutex::new(Vec::new());

fn lock_file() -> Result<std::sync::MutexGuard<'static, Vec<Weak<PathBuf>>>> {
    FILE.lock()
        .map_err(|_| anyhow!("Spotify credential lock poisoned"))
}

/// The grant received from Spotify's authorization-code and refresh-token exchanges.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    username: Option<String>,
}

/// A refresh grant belongs to the OAuth client that originally obtained it. A grant carried over
/// from librespot holds its reusable credential, base64 encoded, where the refresh token goes.
#[derive(Deserialize, Serialize)]
struct Stored {
    client_id: String,
    username: String,
    refresh_token: String,
    #[serde(default, skip_serializing_if = "Renewal::is_refresh")]
    renewal: Renewal,
    /// The device a login5 grant logs in as. Empty for a refresh grant.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    device_id: String,
}

/// How a saved grant is turned into a fresh access token.
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Renewal {
    /// Spotify Accounts' refresh-token exchange, for a browser sign-in.
    #[default]
    Refresh,
    /// A login5 login with a reusable credential, for a sign-in carried over from librespot.
    Login5,
}

impl Renewal {
    fn is_refresh(&self) -> bool {
        *self == Self::Refresh
    }
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
    renewal: Renewal,
    device_id: String,
    token: Arc<Mutex<Option<Token>>>,
    grant: SyncMutex<Grant>,
}

/// The persisted refresh token and any rotation still waiting to be saved.
struct Grant {
    refresh_token: String,
    pending: Option<String>,
}

impl OAuth {
    pub fn stored(path: &Path) -> bool {
        saved(path).is_ok_and(|stored| stored.is_some())
    }

    pub fn restore(path: PathBuf, client_id: &str) -> Result<Option<Arc<Self>>> {
        let _file = lock_file()?;
        let Some(stored) = saved(&path)? else {
            return Ok(None);
        };
        if stored.client_id != client_id
            || stored.username.is_empty()
            || stored.refresh_token.is_empty()
        {
            return Err(AuthError::Credentials.into());
        }
        Ok(Some(Self::new(path, stored, None)))
    }

    /// Signs `username` in with the reusable `credential` librespot saved, and saves it at `path`
    /// as a login5 grant so later renewals go the same way. It refuses when a sign-in was saved
    /// there or a sign-out happened meanwhile, and fails with [`AuthError::Credentials`] once
    /// Spotify stops accepting the credential.
    pub async fn adopt(
        path: PathBuf,
        client_id: &str,
        device_id: &str,
        username: &str,
        credential: &[u8],
    ) -> Result<Arc<Self>> {
        anyhow::ensure!(!username.is_empty(), AuthError::Credentials);
        let login = begin_login(path)?;
        let granted = login5::login(client_id, device_id, username, credential).await?;
        let kept = match granted.credential.is_empty() {
            true => credential,
            false => &granted.credential,
        };
        let stored = Stored {
            client_id: client_id.to_owned(),
            username: username.to_owned(),
            refresh_token: STANDARD.encode(kept),
            renewal: Renewal::Login5,
            device_id: device_id.to_owned(),
        };
        let mut file = lock_file()?;
        // a sign-out while login5 ran revoked this attempt, and its grant is not written
        let ticket = Arc::downgrade(&login);
        let index = file
            .iter()
            .position(|active| active.ptr_eq(&ticket))
            .context(AuthError::Cancelled)?;
        file.swap_remove(index);
        anyhow::ensure!(saved(&login)?.is_none(), AuthError::Cancelled);
        write(&login, &stored)?;
        let token = Token::lasting(granted.access_token, granted.lifetime);
        Ok(Self::new(login.as_ref().clone(), stored, Some(token)))
    }

    /// Authorizes in a browser and saves the refresh grant at the caller's credential path.
    pub fn login<'a>(
        path: PathBuf,
        client_id: &'a str,
        redirect_uri: &'a str,
        success_message: &'a str,
        show_url: impl FnOnce(&str) + 'a,
    ) -> impl Future<Output = Result<Arc<Self>>> + 'a {
        let login = begin_login(path);
        async move {
            let login = login?;
            let (exchange, mut browser) = authorize(client_id, redirect_uri, show_url).await?;
            let result = match exchange {
                Ok(response) => Self::save_login(&login, client_id, response).await,
                Err(error) => Err(error),
            };
            let (status, body) = if result.is_ok() {
                ("200 OK", success_message)
            } else {
                ("400 Bad Request", "")
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ =
                tokio::time::timeout(REQUEST_TIMEOUT, browser.write_all(response.as_bytes())).await;
            result
        }
    }

    async fn save_login(
        login: &Arc<PathBuf>,
        client_id: &str,
        mut response: TokenResponse,
    ) -> Result<Arc<Self>> {
        let refresh_token = response
            .refresh_token
            .take()
            .filter(|token| !token.is_empty())
            .context("Spotify omitted the refresh token")?;
        let username = response.username.take().filter(|name| !name.is_empty());
        let token = Token::from(response);
        let username = match username {
            Some(username) => username,
            None => tokio::time::timeout(
                TOKEN_TIMEOUT,
                crate::access_point::account_username(&token.access_token),
            )
            .await
            .context("Spotify account identity lookup timed out")?
            .context("cannot establish the Spotify account identity")?,
        };
        let stored = Stored {
            client_id: client_id.to_owned(),
            username,
            refresh_token,
            renewal: Renewal::Refresh,
            device_id: String::new(),
        };
        let mut file = lock_file()?;
        let ticket = Arc::downgrade(login);
        let index = file
            .iter()
            .position(|active| active.ptr_eq(&ticket))
            .context(AuthError::Cancelled)?;
        file.swap_remove(index);
        write(login, &stored)?;
        Ok(Self::new(login.as_ref().clone(), stored, Some(token)))
    }

    fn new(path: PathBuf, grant: Stored, token: Option<Token>) -> Arc<Self> {
        Arc::new(Self {
            grant: SyncMutex::new(Grant {
                refresh_token: grant.refresh_token,
                pending: None,
            }),
            path,
            client_id: grant.client_id,
            username: grant.username,
            renewal: grant.renewal,
            device_id: grant.device_id,
            token: Arc::new(Mutex::new(token)),
        })
    }

    pub fn forget(path: &Path) {
        let mut file = match lock_file() {
            Ok(guard) => guard,
            Err(error) => {
                log::warn!("spotify: cannot remove saved credentials: {error}");
                return;
            }
        };
        cancel_logins(&mut file, path);
        if let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("spotify: cannot remove saved credentials: {error}");
        }
    }

    /// Revokes pending browser attempts for this file while keeping the saved grant.
    pub fn cancel_login(path: &Path) {
        if let Ok(mut file) = lock_file() {
            cancel_logins(&mut file, path);
        }
    }

    pub(crate) fn username(&self) -> &str {
        &self.username
    }

    pub(crate) fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Detects sign-out and external changes while accepting our own token rotations.
    pub fn has_current_grant(&self) -> bool {
        self.with_grant(|_| Ok(())).is_ok()
    }

    /// Uses the in-memory grant only while its backing file still belongs to this session.
    fn with_grant<T>(&self, use_grant: impl FnOnce(&mut Grant) -> Result<T>) -> Result<T> {
        let _file = lock_file()?;
        let mut grant = self
            .grant
            .lock()
            .map_err(|_| anyhow!("Spotify grant lock poisoned"))?;
        self.check_grant(&grant)?;
        use_grant(&mut grant)
    }

    /// Checks the stored account and token under the credential lock.
    fn check_grant(&self, grant: &Grant) -> Result<Stored> {
        let stored = saved(&self.path)?.context(AuthError::Credentials)?;
        anyhow::ensure!(
            stored.client_id == self.client_id
                && stored.username == self.username
                && stored.refresh_token == grant.refresh_token,
            AuthError::Credentials
        );
        Ok(stored)
    }

    /// Validates the grant with Spotify before the app treats the account as signed in.
    pub async fn validate(self: &Arc<Self>) -> Result<()> {
        self.token().await.map(drop)
    }

    /// Uses the cached bearer until it expires. Rotated refresh grants are persisted atomically.
    pub(crate) async fn token(self: &Arc<Self>) -> Result<String> {
        let auth = self.clone();
        self.token_using(async move { auth.refresh().await }).await
    }

    async fn token_using(
        self: &Arc<Self>,
        refresh: impl Future<Output = Result<Token>> + Send + 'static,
    ) -> Result<String> {
        let mut cached = self.token.clone().lock_owned().await;
        if let Some(access_token) = self.with_grant(|_| {
            Ok(cached
                .as_ref()
                .filter(|token| token.expires_at > Instant::now())
                .map(|token| token.access_token.clone()))
        })? {
            return Ok(access_token);
        }

        // Spotify can rotate the grant after the caller drops its request future.
        let auth = self.clone();
        tokio::spawn(async move {
            let token = refresh.await?;
            auth.with_grant(|_| {
                let access_token = token.access_token.clone();
                *cached = Some(token);
                Ok(access_token)
            })
        })
        .await
        .context("Spotify token renewal task failed")?
    }

    async fn refresh(&self) -> Result<Token> {
        let _refresh = REFRESH.lock().await;
        let refresh_token = self.with_grant(|grant| {
            Ok(grant
                .pending
                .as_ref()
                .unwrap_or(&grant.refresh_token)
                .clone())
        })?;
        if self.renewal == Renewal::Login5 {
            let credential = STANDARD
                .decode(&refresh_token)
                .context(AuthError::Credentials)?;
            let granted = login5::login(
                &self.client_id,
                &self.device_id,
                &self.username,
                &credential,
            )
            .await?;
            let next =
                (!granted.credential.is_empty()).then(|| STANDARD.encode(&granted.credential));
            self.keep_grant(next.as_deref())?;
            return Ok(Token::lasting(granted.access_token, granted.lifetime));
        }
        let response = exchange([
            ("grant_type", "refresh_token"),
            ("client_id", &self.client_id),
            ("refresh_token", &refresh_token),
        ])
        .await?;
        anyhow::ensure!(
            response
                .username
                .as_deref()
                .is_none_or(|username| username == self.username),
            AuthError::Credentials
        );
        self.keep_grant(
            response
                .refresh_token
                .as_deref()
                .filter(|token| !token.is_empty()),
        )?;
        Ok(Token::from(response))
    }

    /// Keeps a rotation even if the filesystem is temporarily unavailable.
    fn keep_grant(&self, next: Option<&str>) -> Result<()> {
        let _file = lock_file()?;
        let mut grant = self
            .grant
            .lock()
            .map_err(|_| anyhow!("Spotify grant lock poisoned"))?;
        let pending = next.map(str::to_owned).or_else(|| grant.pending.clone());
        let mut stored = match self.check_grant(&grant) {
            Ok(stored) => stored,
            Err(error) if error.is::<std::io::Error>() => {
                grant.pending = pending;
                log::warn!("spotify: cannot read grant to save renewal: {error:#}");
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        grant.pending = pending.filter(|next| *next != grant.refresh_token);
        if let Some(next) = &grant.pending {
            stored.refresh_token.clone_from(next);
            match write(&self.path, &stored) {
                Ok(()) => {
                    grant.refresh_token = stored.refresh_token;
                    grant.pending = None;
                }
                Err(error) => log::warn!("spotify: cannot save renewed grant: {error:#}"),
            }
        }
        Ok(())
    }
}

impl Token {
    /// A token Spotify says lasts `lifetime`, renewed a little before it runs out.
    fn lasting(access_token: String, lifetime: Duration) -> Self {
        let lifetime = lifetime.saturating_sub(EXPIRY_MARGIN);
        let now = Instant::now();
        Self {
            access_token,
            expires_at: now.checked_add(lifetime).unwrap_or(now),
        }
    }
}

impl From<TokenResponse> for Token {
    fn from(response: TokenResponse) -> Self {
        Self::lasting(
            response.access_token,
            Duration::from_secs(response.expires_in),
        )
    }
}

/// Exchanges an authorization code or renews a refresh grant at Spotify Accounts.
async fn exchange<const N: usize>(form: [(&str, &str); N]) -> Result<TokenResponse> {
    let response = http::request(|client| {
        client
            .post(TOKEN_URL)
            .timeout(TOKEN_TIMEOUT)
            .form(&form.as_slice())
    })
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

/// Registers a browser attempt before its future runs, so sign-out can revoke its write.
fn begin_login(path: PathBuf) -> Result<Arc<PathBuf>> {
    let login = Arc::new(path);
    let mut file = lock_file()?;
    file.retain(|attempt| attempt.strong_count() > 0);
    file.push(Arc::downgrade(&login));
    Ok(login)
}

fn cancel_logins(attempts: &mut Vec<Weak<PathBuf>>, path: &Path) {
    attempts.retain(|attempt| attempt.upgrade().is_some_and(|file| file.as_path() != path));
}

fn saved(path: &Path) -> Result<Option<Stored>> {
    match std::fs::read(path) {
        Ok(body) => serde_json::from_slice(&body)
            .map(Some)
            .context(AuthError::Credentials),
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
    async fn cancellation_keeps_the_rotated_grant_and_shares_the_new_token() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("grant.json");
        let grant = Stored {
            client_id: "client".into(),
            username: "listener".into(),
            refresh_token: "old".into(),
            renewal: Renewal::Refresh,
            device_id: String::new(),
        };
        write(&path, &grant)?;
        let auth = OAuth::restore(path.clone(), "client")?.unwrap();
        let (started, wait) = tokio::sync::oneshot::channel();
        let (finish, response) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn({
            let auth = auth.clone();
            async move {
                let writer = auth.clone();
                auth.token_using(async move {
                    let _ = started.send(());
                    response.await?;
                    writer.keep_grant(Some("new"))?;
                    Ok(Token {
                        access_token: "bearer".into(),
                        expires_at: Instant::now() + Duration::from_secs(60),
                    })
                })
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            wait.await?;
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            finish.send(()).unwrap();
            assert_eq!(
                auth.token_using(async { anyhow::bail!("unexpected refresh") })
                    .await?,
                "bearer"
            );
            assert_eq!(saved(&path)?.unwrap().refresh_token, "new");
            assert!(auth.has_current_grant());
            Ok::<_, anyhow::Error>(())
        })
        .await?
    }
}
