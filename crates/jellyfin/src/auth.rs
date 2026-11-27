use std::path::PathBuf;

use anyhow::{Context as _, Result};
use music::{credentials, escape};
use serde::{Deserialize, Serialize};

/// What Jellyfin's dashboard lists the session under.
const CLIENT: &str = "Sonora";

/// A Jellyfin sign-in as it is kept on disk. The password is never kept: the server hands out
/// an access token for it, and that token is what every later request carries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub server: String,
    pub user_id: String,
    pub username: String,
    pub token: String,
    /// The id this install signs in under. Jellyfin keeps one token per device id, so it is made
    /// once at sign-in and kept with the token it belongs to.
    pub device_id: String,
    /// The name the server goes by in its own dashboard, which tells this account apart from
    /// one on another server. A sign-in kept before it was asked for has none until the account
    /// is next opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
}

/// A fresh device id, made once per sign-in.
pub fn device_id() -> String {
    format!("sonora-{:016x}", fastrand::u64(..))
}

/// The name this machine goes by on the server's devices page.
pub fn device_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    match name.trim().is_empty() {
        true => CLIENT.to_owned(),
        false => name,
    }
}

/// The `Authorization` header Jellyfin reads a client and its token from, in its `MediaBrowser`
/// scheme. The server url-decodes every value, so each one is percent-encoded, which also keeps
/// a quote or comma in the device name from ending a value early. Before sign-in there is no
/// token, and the header only says who is asking.
pub fn header(device: &str, device_id: &str, token: Option<&str>) -> String {
    let mut header = format!(
        "MediaBrowser Client=\"{CLIENT}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
        escape::component(device),
        escape::component(device_id),
        escape::component(env!("CARGO_PKG_VERSION")),
    );
    if let Some(token) = token {
        header.push_str(&format!(", Token=\"{}\"", escape::component(token)));
    }
    header
}

/// The server address as typed, with a scheme and without a trailing slash. A bare host is taken
/// to be plain http, the way a server on the local network usually is.
pub fn normalize_server(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        anyhow::bail!("the server address is empty");
    }
    Ok(match trimmed.contains("://") {
        true => trimmed.to_owned(),
        false => format!("http://{trimmed}"),
    })
}

/// Where the account with slug `slug` keeps its sign-in.
fn path(slug: &str) -> PathBuf {
    credentials::dir(slug).join(credentials::FILE)
}

/// The sign-in kept for the account `slug`, or `None` when there is none or it is missing a part.
pub fn load(slug: &str) -> Option<Credentials> {
    let bytes = std::fs::read(path(slug)).ok()?;
    let mut credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    credentials.server = normalize_server(&credentials.server).ok()?;
    match credentials.user_id.is_empty() || credentials.token.is_empty() {
        true => None,
        false => Some(credentials),
    }
}

pub fn store(slug: &str, stored: &Credentials) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(stored).context("cannot serialize jellyfin credentials")?;
    credentials::write(&path(slug), &bytes).context("cannot store jellyfin credentials")
}

pub fn forget(slug: &str) {
    credentials::remove(&path(slug));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_names_the_client_and_carries_the_token() {
        let header = header("desk top", "sonora-1", Some("abc123"));
        assert_eq!(
            header,
            format!(
                "MediaBrowser Client=\"Sonora\", Device=\"desk%20top\", DeviceId=\"sonora-1\", \
                 Version=\"{}\", Token=\"abc123\"",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn header_before_sign_in_has_no_token() {
        assert!(!header("pc", "id", None).contains("Token="));
    }

    #[test]
    fn header_escapes_quotes_in_the_device_name() {
        assert!(header("a\"b,c", "id", None).contains("Device=\"a%22b%2Cc\""));
    }

    #[test]
    fn server_gets_a_scheme_and_loses_its_trailing_slash() {
        assert_eq!(
            normalize_server(" media.local:8096/ ").unwrap(),
            "http://media.local:8096"
        );
        assert_eq!(
            normalize_server("https://jf.example.org/jellyfin/").unwrap(),
            "https://jf.example.org/jellyfin"
        );
        assert!(normalize_server("  ").is_err());
    }
}
