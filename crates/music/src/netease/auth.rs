//! Sign-in credentials for NetEase: the session cookies the web player holds, kept in the
//! provider's own cache folder.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::credentials;

/// The cookie that proves a signed-in NetEase session. Everything the account can see is
/// scoped to it, and the site sets it on `music.163.com` alone.
pub(crate) const PROOF: &[&str] = &["MUSIC_U"];

/// A NetEase session: the token, and the CSRF pair the site checks on every write.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Credentials {
    /// The `MUSIC_U` value alone, not the whole header.
    pub music_u: String,
    /// The `__csrf` value the browser stored beside it. Empty is accepted, and is what a
    /// paste of the token alone leaves; NetEase only asks it back on a write.
    pub csrf: String,
}

impl Credentials {
    /// The `Cookie` header every call carries. `os=pc` is what the web player sends and what
    /// makes the endpoints answer in the shape its own pages were written against.
    pub(crate) fn header(&self) -> String {
        format!("os=pc; MUSIC_U={}; __csrf={}", self.music_u, self.csrf)
    }
}

fn path() -> PathBuf {
    credentials::dir("netease").join(credentials::FILE)
}

/// Reads the session out of what the user pastes: a whole `Cookie` header, the `MUSIC_U=…`
/// pair alone, or the bare token. Refuses anything without a token.
pub(crate) fn cookies(input: &str) -> Result<Credentials> {
    let trimmed = input.trim();
    let mut music_u = None;
    let mut csrf = String::new();
    let mut paired = false;
    for pair in trimmed.split(';') {
        let pair = pair.trim();
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        paired = true;
        match name.trim() {
            "MUSIC_U" => music_u = Some(value.trim().to_owned()),
            "__csrf" => csrf = value.trim().to_owned(),
            _ => {}
        }
    }
    // a lone token has no `=` in it, so nothing above matched and the paste is the token
    let music_u = music_u
        .or_else(|| (!paired).then(|| trimmed.to_owned()))
        .unwrap_or_default();
    if !valid(&music_u) {
        bail!("the cookies carry no MUSIC_U; sign in to music.163.com first");
    }
    Ok(Credentials { music_u, csrf })
}

/// Whether a value could be a `MUSIC_U`. Deliberately loose: the token is an opaque string
/// and only the site can say whether it is a live one, so this only rejects a paste that is
/// plainly the wrong cookie, or nothing at all.
fn valid(value: &str) -> bool {
    value.len() >= 20 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

pub(crate) fn load() -> Option<Credentials> {
    let bytes = std::fs::read(path()).ok()?;
    let credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    valid(&credentials.music_u).then_some(credentials)
}

pub(crate) fn store(credentials: &Credentials) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(credentials).context("cannot serialize netease credentials")?;
    credentials::write(&path(), &bytes).context("cannot store netease credentials")
}

pub(crate) fn forget() {
    credentials::remove(&path());
}
