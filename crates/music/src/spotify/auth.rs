use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow};

use crate::{SignInFailure, SignInProblem, credentials};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::{Session, SessionConfig};
use librespot_oauth::OAuthClientBuilder;

pub const DEFAULT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

const PRODUCT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const PRODUCT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

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

/// Moves the credential file releases before 0.31 kept at the cache root into the
/// Spotify folder. Part of the startup migration pass.
pub(crate) fn migrate() {
    let legacy = credentials::root().join(credentials::FILE);
    credentials::adopt(&legacy, &AuthConfig::default().file());
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

pub async fn login(config: &AuthConfig) -> Result<Session> {
    let client_id = config.client_id.clone();
    let redirect_uri = config.redirect_uri.clone();

    let token = tokio::task::spawn_blocking(move || {
        OAuthClientBuilder::new(&client_id, &redirect_uri, SCOPES.to_vec())
            .open_in_browser()
            .build()?
            .get_access_token()
    })
    .await?
    .map_err(explain)?;

    let session = session(config)?;
    session
        .connect(Credentials::with_access_token(token.access_token), true)
        .await
        .map_err(denied)?;
    credentials::secure(&config.file());
    premium(&session).await?;
    Ok(session)
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

fn explain(error: librespot_oauth::OAuthError) -> anyhow::Error {
    let message = error.to_string();
    match callback_error(&message) {
        Some("invalid_scope") => anyhow!("Spotify rejected the requested scopes (invalid_scope)"),
        Some("access_denied") => anyhow::Error::new(SignInFailure(SignInProblem::Cancelled)),
        Some(code) => anyhow!("Spotify refused authorization ({code})"),
        None => anyhow::Error::new(error).context("browser authorization failed"),
    }
}

fn callback_error(message: &str) -> Option<&str> {
    let start = message.find("error=")? + "error=".len();
    let rest = &message[start..];
    Some(rest.split(['&', ' ']).next().unwrap_or(rest))
}

pub fn forget(config: &AuthConfig) {
    credentials::remove(&config.file());
}

fn session(config: &AuthConfig) -> Result<Session> {
    let cache = Cache::new(Some(config.cache_dir.as_path()), None, None, None)
        .with_context(|| format!("cannot open cache at {}", config.cache_dir.display()))?;

    let session_config = SessionConfig {
        client_id: config.client_id.clone(),
        ..Default::default()
    };

    Ok(Session::new(session_config, Some(cache)))
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
