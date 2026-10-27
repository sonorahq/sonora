//! Sign-in credentials for SoundCloud: the `oauth_token` cookie a signed-in browser holds,
//! kept in the provider's own accounts folder and sent as the `Authorization: OAuth …` header
//! the internal api expects.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::credentials;

/// The cookie that proves a signed-in SoundCloud session.
pub(crate) const PROOF: &[&str] = &["oauth_token"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Credentials {
    /// The bare oauth token, without the `OAuth ` prefix the header carries.
    pub token: String,
}

fn path() -> PathBuf {
    credentials::dir("soundcloud").join(credentials::FILE)
}

/// Extracts the token from what the user pastes or the browser window captured: a full
/// `Cookie` header, a lone `oauth_token=…` pair, an `OAuth …` authorization header, or the
/// bare token. Refuses anything without one.
pub fn token(input: &str) -> Result<String> {
    let trimmed = input.trim();
    // A full Cookie header: find the oauth_token pair. SoundCloud has also spelled the same
    // cookie `sc_oauth_token`, so both names are accepted.
    for pair in trimmed.split(';') {
        let pair = pair.trim();
        if let Some((name, value)) = pair.split_once('=')
            && matches!(name.trim(), "oauth_token" | "sc_oauth_token")
        {
            let value = value.trim();
            if valid(value) {
                return Ok(value.to_owned());
            }
            bail!("the oauth token does not look like one");
        }
    }
    // An `Authorization: OAuth …` header, as SoundCloud's own requests carry it.
    if let Some(rest) = trimmed
        .strip_prefix("OAuth ")
        .or_else(|| trimmed.strip_prefix("oauth "))
    {
        let value = rest.trim();
        if valid(value) {
            return Ok(value.to_owned());
        }
        bail!("the oauth token does not look like one");
    }
    if valid(trimmed) {
        return Ok(trimmed.to_owned());
    }
    bail!("the cookies carry no oauth_token; sign in to soundcloud.com first");
}

/// A token is whatever the browser handed over. SoundCloud signs it with dashes and base64url
/// characters, so the only check is that it is one value: not empty, and free of the whitespace
/// that would break the header it is sent in.
fn valid(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| !byte.is_ascii_whitespace())
}

pub(crate) fn load() -> Option<Credentials> {
    let bytes = std::fs::read(path()).ok()?;
    let credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    valid(&credentials.token).then_some(credentials)
}

pub(crate) fn store(credentials: &Credentials) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(credentials)
        .context("cannot serialize soundcloud credentials")?;
    credentials::write(&path(), &bytes).context("cannot store soundcloud credentials")
}

pub(crate) fn forget() {
    credentials::remove(&path());
}
