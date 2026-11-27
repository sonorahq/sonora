use std::path::PathBuf;

use anyhow::{Context as _, Result};
use music::credentials;
use serde::{Deserialize, Serialize};

/// The file the client identifier is kept in, beside the sign-in.
const CLIENT_ID_FILE: &str = "client-id";

/// What a sign-in leaves behind: the account's token, the one server and music library it was
/// pointed at, and who the account is.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credentials {
    /// The plex.tv token of the account, which finds the server again when its address changes.
    pub token: String,
    pub server: Server,
    pub section: Section,
    pub user: User,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Server {
    /// The server's machine identifier, which stays the same however it is reached.
    pub id: String,
    pub name: String,
    /// The token that opens this server. For a server the account owns it is the account's own.
    pub token: String,
    /// The address that last answered, with no trailing slash.
    pub uri: String,
}

/// The music library of the server that Sonora shows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Section {
    pub key: String,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
}

/// Where the account with slug `slug` keeps its sign-in.
fn path(slug: &str) -> PathBuf {
    credentials::dir(slug).join(credentials::FILE)
}

/// This install's identifier on plex.tv. Plex lists every client that signed in by it, so it is
/// made once, kept in the first account's folder and shared by every account, and a new one
/// would show up as a new device on the account.
pub fn client_id() -> Result<String> {
    let file = credentials::dir(crate::SLUG).join(CLIENT_ID_FILE);
    if let Ok(bytes) = std::fs::read(&file)
        && let Ok(known) = String::from_utf8(bytes)
        && !known.trim().is_empty()
    {
        return Ok(known.trim().to_owned());
    }
    let made = (0..4)
        .map(|_| format!("{:08x}", fastrand::u32(..)))
        .collect::<Vec<_>>()
        .join("");
    credentials::write(&file, made.as_bytes()).context("cannot store the plex client id")?;
    Ok(made)
}

/// The sign-in kept for the account `slug`, or `None` when there is none or it is missing a part.
pub fn load(slug: &str) -> Option<Credentials> {
    let bytes = std::fs::read(path(slug)).ok()?;
    let mut credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    credentials.server.uri = credentials.server.uri.trim_end_matches('/').to_owned();
    match credentials.token.is_empty() || credentials.server.id.is_empty() {
        true => None,
        false => Some(credentials),
    }
}

pub fn store(slug: &str, stored: &Credentials) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(stored).context("cannot serialize plex credentials")?;
    credentials::write(&path(slug), &bytes).context("cannot store plex credentials")
}

pub fn forget(slug: &str) {
    credentials::remove(&path(slug));
}
