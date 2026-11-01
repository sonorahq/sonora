//! Sign-in credentials for Yandex Music: the OAuth token the Yandex Music app is issued, kept
//! in the provider's own accounts folder.
//!
//! The token comes from Yandex's own OAuth page for the Android app's client. Once the listener
//! has signed in there, the page sends them on to an address that carries the token after
//! `#access_token=`, and either that whole address or the token alone is pasted back.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::credentials;

/// Where the token is handed out: Yandex's OAuth page for the Android app's client, answering
/// with the token in the address it redirects to.
pub const TOKEN_PAGE: &str = "https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabbbdc6cd418abb4b39c32c41195d";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Credentials {
    pub token: String,
}

fn path() -> PathBuf {
    credentials::dir("yandex").join(credentials::FILE)
}

/// Extracts the token from what the listener pastes: the address the OAuth page redirected
/// to, an `access_token=…` pair, or the bare token. Refuses anything without one.
pub fn token(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if let Some((_, rest)) = trimmed.split_once("access_token=") {
        let value = rest.split(['&', '#', ' ']).next().unwrap_or_default();
        if valid(value) {
            return Ok(value.to_owned());
        }
        bail!("the access_token in that address does not look like one");
    }
    let bare = trimmed.strip_prefix("OAuth ").unwrap_or(trimmed).trim();
    if valid(bare) {
        return Ok(bare.to_owned());
    }
    bail!("that is no yandex music token; paste the address the sign-in page ended on");
}

/// A token is a few dozen url-safe characters, `y0_…` on newer accounts.
fn valid(value: &str) -> bool {
    value.len() >= 20
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

pub(crate) fn load() -> Option<Credentials> {
    let bytes = std::fs::read(path()).ok()?;
    let credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    valid(&credentials.token).then_some(credentials)
}

pub(crate) fn store(credentials: &Credentials) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(credentials).context("cannot serialize yandex credentials")?;
    credentials::write(&path(), &bytes).context("cannot store yandex credentials")
}

pub(crate) fn forget() {
    credentials::remove(&path());
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "y0_AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn the_token_reads_from_every_paste() {
        let redirect = format!(
            "https://music.yandex.ru/#access_token={TOKEN}&token_type=bearer&expires_in=31535999"
        );
        assert_eq!(token(&redirect).unwrap(), TOKEN);
        assert_eq!(token(&format!("access_token={TOKEN}")).unwrap(), TOKEN);
        assert_eq!(token(&format!("  {TOKEN}\n")).unwrap(), TOKEN);
        assert_eq!(token(&format!("OAuth {TOKEN}")).unwrap(), TOKEN);
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(token("").is_err());
        assert!(token("short").is_err());
        assert!(token("https://music.yandex.ru/#access_token=&x=1").is_err());
        assert!(token("not a token at all, with spaces").is_err());
    }
}
