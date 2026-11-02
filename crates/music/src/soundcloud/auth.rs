//! Sign-in credentials for SoundCloud: the `oauth_token` cookie a signed-in browser holds, or
//! the grant a registered app's sign-in left, kept in the provider's own accounts folder and
//! sent as the `Authorization: OAuth …` header both apis expect.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::credentials;
use crate::soundcloud::oauth::Grant;

/// The cookie that proves a signed-in SoundCloud session.
pub(crate) const PROOF: &[&str] = &["oauth_token"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Credentials {
    /// The bare oauth token, without the `OAuth ` prefix the header carries.
    pub token: String,
    /// The token that renews `token`, from a registered app's sign-in. A cookie has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<String>,
    /// When `token` lapses, in unix seconds. A cookie's lapse is not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// The token came from a registered app's sign-in, so it may write. A file an older
    /// Sonora wrote reads as a cookie.
    #[serde(default)]
    pub app: bool,
}

impl Credentials {
    /// The cookie's token, kept as it came.
    pub fn cookie(token: String) -> Self {
        Self {
            token,
            ..Self::default()
        }
    }

    /// What a registered app's sign-in or renewal handed over.
    pub fn from_grant(grant: &Grant) -> Self {
        Self {
            token: grant.access.clone(),
            refresh: grant.refresh.clone(),
            expires_at: grant.expires_at,
            app: true,
        }
    }

    pub fn grant(&self) -> Grant {
        Grant {
            access: self.token.clone(),
            refresh: self.refresh.clone(),
            expires_at: self.expires_at,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_sign_in_an_older_sonora_kept_still_reads() {
        let kept: Credentials = serde_json::from_str(r#"{"token": "2-cookie"}"#).unwrap();
        assert_eq!(kept, Credentials::cookie("2-cookie".to_owned()));
        assert!(!kept.app);
    }

    #[test]
    fn an_app_grant_survives_the_round_trip() {
        let grant = Grant {
            access: "2-access".to_owned(),
            refresh: Some("refresh".to_owned()),
            expires_at: Some(1_900_000_000),
        };
        let kept = Credentials::from_grant(&grant);
        let bytes = serde_json::to_vec(&kept).unwrap();
        let read: Credentials = serde_json::from_slice(&bytes).unwrap();
        assert!(read.app);
        assert_eq!(read.grant(), grant);
    }

    #[test]
    fn takes_the_token_out_of_a_cookie_header() {
        assert_eq!(
            token("sc_anonymous_id=1; oauth_token=2-123-456-abc; other=x").unwrap(),
            "2-123-456-abc"
        );
        assert_eq!(token("OAuth 2-123-456-abc").unwrap(), "2-123-456-abc");
        assert!(token("   ").is_err());
    }
}
