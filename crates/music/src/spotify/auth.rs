use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};

use crate::spotify::offline;
use crate::vault::Vault;
use crate::{PromptSink, SignInFailure, SignInProblem, SignInPrompt, credentials};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::{FileId, Session, SessionConfig};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge, RedirectUrl, Scope,
    TokenResponse as _, TokenUrl, basic::BasicClient,
};

pub const DEFAULT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

const PRODUCT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const PRODUCT_POLL: std::time::Duration = std::time::Duration::from_millis(100);
/// The bytes of a Spotify file id, which librespot names its kept files by.
const FILE_ID: usize = 20;

pub const SCOPES: &[&str] = &[
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

    /// The stored credential librespot writes after a successful connect.
    pub fn file(&self) -> PathBuf {
        self.cache_dir.join(credentials::FILE)
    }
}

pub fn release(config: &AuthConfig) {
    let Some(address) = socket_address(&config.redirect_uri) else {
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

fn socket_address(uri: &str) -> Option<String> {
    let rest = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))?;
    let authority = rest.split('/').next().filter(|host| !host.is_empty())?;
    match authority.rsplit_once(':') {
        Some((_, port)) if port.chars().all(|digit| digit.is_ascii_digit()) => {
            Some(authority.to_owned())
        }
        _ => Some(format!("{authority}:80")),
    }
}

pub async fn restore(config: &AuthConfig) -> Result<Option<Session>> {
    let session = session(config)?;
    let Some(credentials) = session.cache().and_then(|cache| cache.credentials()) else {
        return Ok(None);
    };

    session.connect(credentials, true).await.map_err(denied)?;
    credentials::secure(&config.file());
    premium(&session).await?;
    Ok(Some(session))
}

pub async fn login(config: &AuthConfig, prompt: PromptSink) -> Result<Session> {
    let client_id = config.client_id.clone();
    let redirect_uri = config.redirect_uri.clone();

    let access_token =
        tokio::task::spawn_blocking(move || authorize(&client_id, &redirect_uri, prompt)).await??;

    let session = session(config)?;
    session
        .connect(Credentials::with_access_token(access_token), true)
        .await
        .map_err(denied)?;
    credentials::secure(&config.file());
    premium(&session).await?;
    Ok(session)
}

fn authorize(client_id: &str, redirect_uri: &str, prompt: PromptSink) -> Result<String> {
    let client = BasicClient::new(ClientId::new(client_id.to_owned()))
        .set_auth_uri(AuthUrl::new(
            "https://accounts.spotify.com/authorize".to_owned(),
        )?)
        .set_token_uri(TokenUrl::new(
            "https://accounts.spotify.com/api/token".to_owned(),
        )?)
        .set_redirect_uri(RedirectUrl::new(redirect_uri.to_owned())?);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let scopes = SCOPES.iter().map(|scope| Scope::new((*scope).to_owned()));
    let (url, _) = client
        .authorize_url(CsrfToken::new_random)
        .add_scopes(scopes)
        .set_pkce_challenge(challenge)
        .url();
    prompt(SignInPrompt::Url(url.to_string()));
    open::that_in_background(url.as_str());

    let address = socket_address(redirect_uri)
        .with_context(|| format!("redirect URI has no socket address: {redirect_uri}"))?;
    let listener = TcpListener::bind(&address)
        .with_context(|| format!("cannot listen for Spotify login at {address}"))?;
    let mut stream = listener
        .incoming()
        .next()
        .context("Spotify login callback did not arrive")??;
    let mut request = String::new();
    BufReader::new(&stream).read_line(&mut request)?;
    let target = request
        .split_whitespace()
        .nth(1)
        .context("Spotify login callback was malformed")?;
    let callback = oauth2::url::Url::parse(&format!("http://localhost{target}"))?;
    let query = callback
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let response = "You can return to Sonora.";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{response}",
        response.len()
    )?;
    if let Some(error) = query.get("error") {
        let problem = match error.as_ref() {
            "access_denied" => SignInProblem::Cancelled,
            _ => SignInProblem::Refused,
        };
        return Err(SignInFailure(problem).into());
    }
    let code = query
        .get("code")
        .context("Spotify login callback had no authorization code")?;
    let response = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .set_pkce_verifier(verifier)
        .request(&exchange)
        .map_err(|error| anyhow!("failed to exchange Spotify authorization code: {error}"))?;
    Ok(response.access_token().secret().to_owned())
}

/// Carries one oauth2 request over reqwest. oauth2 only knows the reqwest it was built against,
/// so the workspace one is handed in as a plain function instead.
fn exchange(request: oauth2::HttpRequest) -> Result<oauth2::HttpResponse, reqwest::Error> {
    let sent = reqwest::blocking::Client::new().execute(request.try_into()?)?;
    let status = sent.status();
    let headers = sent.headers().clone();
    let mut response = http::Response::new(sent.bytes()?.to_vec());
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

async fn premium(session: &Session) -> Result<()> {
    let deadline = tokio::time::Instant::now() + PRODUCT_WAIT;
    loop {
        if let Some(account) = session.user_data().attributes.get("type") {
            match account.as_str() {
                "premium" => return Ok(()),
                _ => {
                    session.shutdown();
                    return Err(anyhow::Error::new(SignInFailure(SignInProblem::Premium)));
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep(PRODUCT_POLL).await;
    }
}

fn denied(error: librespot_core::Error) -> anyhow::Error {
    let problem = classify(&error.to_string());
    anyhow::Error::new(error).context(SignInFailure(problem))
}

fn classify(message: &str) -> SignInProblem {
    let message = message.to_lowercase();
    if message.contains("travel restriction") {
        return SignInProblem::Region;
    }
    if message.contains("bad credentials") || message.contains("invalid credentials") {
        return SignInProblem::Credentials;
    }
    if message.contains("connection")
        || message.contains("timed out")
        || message.contains("dns")
        || message.contains("network")
    {
        return SignInProblem::Network;
    }
    SignInProblem::Refused
}

pub fn forget(config: &AuthConfig) {
    credentials::remove(&config.file());
}

pub(crate) fn session(config: &AuthConfig) -> Result<Session> {
    // librespot keeps each file as the CDN sent it, still under the key Spotify hands out
    // again on every play, so a kept file is no easier to play than the stream.
    let audio = Vault::foreign("spotify");
    let cache = Cache::new(
        Some(config.cache_dir.as_path()),
        None,
        audio.as_ref().map(|(folder, _)| folder.as_path()),
        audio.as_ref().map(|(_, room)| *room),
    )
    .with_context(|| format!("cannot open cache at {}", config.cache_dir.display()))?;

    let session_config = SessionConfig {
        client_id: config.client_id.clone(),
        device_id: device_id(&config.cache_dir),
        ..Default::default()
    };

    let session = Session::new(session_config, Some(cache));
    if let Some(((folder, _), cache)) = audio.zip(session.cache()) {
        let cache = Arc::downgrade(cache);
        Vault::adopt("spotify", move || {
            cache
                .upgrade()
                .map(|cache| clear(&cache, &folder))
                .is_some()
        });
    }
    Ok(session)
}

/// Removes every audio file librespot keeps in `folder` through `cache`, so its count of what
/// it holds stays true while its session runs. librespot files each one under the first two
/// hex digits of its id, and anything else in the folder is removed as it is.
fn clear(cache: &Cache, folder: &Path) {
    for group in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let prefix = group.file_name().to_string_lossy().into_owned();
        for item in std::fs::read_dir(group.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = format!("{prefix}{}", item.file_name().to_string_lossy());
            let removed = match offline::hex(&name).filter(|raw| raw.len() == FILE_ID) {
                Some(raw) => cache
                    .remove_file(FileId::from_raw(&raw))
                    .map_err(anyhow::Error::from),
                None => std::fs::remove_file(item.path()).map_err(anyhow::Error::from),
            };
            if let Err(error) = removed {
                log::warn!("spotify: cannot remove a kept file: {error:#}");
            }
        }
        std::fs::remove_dir(group.path()).ok();
    }
}

/// The id this install goes by on Spotify's device list. A new one each launch would leave a
/// stale Sonora behind every time, so the first one is kept next to the credentials.
fn device_id(dir: &Path) -> String {
    let file = dir.join("device-id");
    let kept = std::fs::read_to_string(&file)
        .map(|id| id.trim().to_owned())
        .ok()
        .filter(|id| !id.is_empty());
    if let Some(id) = kept {
        return id;
    }

    let id = SessionConfig::default().device_id;
    if let Err(error) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, &id)) {
        log::warn!("auth: cannot keep the device id: {error}");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::socket_address;

    #[test]
    fn reads_host_and_port() {
        assert_eq!(
            socket_address("http://127.0.0.1:8989/login").as_deref(),
            Some("127.0.0.1:8989")
        );
    }

    #[test]
    fn defaults_a_missing_port() {
        assert_eq!(
            socket_address("http://localhost/login").as_deref(),
            Some("localhost:80")
        );
    }

    #[test]
    fn rejects_a_uri_without_a_scheme() {
        assert!(socket_address("127.0.0.1:8989/login").is_none());
    }
}
