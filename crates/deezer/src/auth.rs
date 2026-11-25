//! Sign-in credentials for Deezer: the `arl` session cookie from a browser, kept in the
//! provider's own cache folder.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use music::credentials;
use serde::{Deserialize, Serialize};

/// The cookie that proves a signed-in Deezer session.
pub(crate) const PROOF: &[&str] = &["arl"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Credentials {
    /// The `arl` cookie value alone, not the whole header.
    pub arl: String,
    /// The user id of the family profile picked at sign-in, which every connect switches to.
    /// None for an account without family profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

fn path() -> PathBuf {
    credentials::dir("deezer").join(credentials::FILE)
}

/// Extracts the `arl` value from what the user pastes: a full `Cookie` header, a lone
/// `arl=…` pair, or the bare token. Refuses anything without one.
pub fn arl(input: &str) -> Result<String> {
    let trimmed = input.trim();
    for pair in trimmed.split(';') {
        let pair = pair.trim();
        if let Some((name, value)) = pair.split_once('=')
            && name.trim() == "arl"
        {
            let value = value.trim();
            if valid(value) {
                return Ok(value.to_owned());
            }
            bail!("the arl cookie does not look like one");
        }
    }
    if valid(trimmed) {
        return Ok(trimmed.to_owned());
    }
    bail!("the cookies carry no arl; sign in to deezer.com first");
}

/// An arl is a hex token in either case, around 192 characters.
fn valid(value: &str) -> bool {
    value.len() >= 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn load() -> Option<Credentials> {
    let bytes = std::fs::read(path()).ok()?;
    let credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    valid(&credentials.arl).then_some(credentials)
}

pub(crate) fn store(credentials: &Credentials) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(credentials).context("cannot serialize deezer credentials")?;
    credentials::write(&path(), &bytes).context("cannot store deezer credentials")
}

pub(crate) fn forget() {
    credentials::remove(&path());
}
