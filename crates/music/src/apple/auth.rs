//! What Apple Music needs to answer for an account: the web player's own bearer token, and the
//! `media-user-token` cookie the sign-in window brings back.
//!
//! The bearer token is the same public one every visitor to music.apple.com gets; it is read
//! off the page rather than registered for, which makes it the brittle half of this file and
//! the reason it is isolated here. The user token is the account, and it is never logged.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use crate::credentials;

/// The cookie that proves an account is signed in, and the name of the value everything else
/// here calls the user token.
pub(crate) const PROOF: &[&str] = &["media-user-token"];

/// An account token supplied by hand, for a headless run with no sign-in window.
pub const USER_TOKEN_ENV: &str = "SONORA_APPLE_MEDIA_USER_TOKEN";

/// A bearer token supplied by hand, which skips reading one off the page. Useful when Apple
/// changes the page and the scrape below stops finding it.
pub const BEARER_ENV: &str = "SONORA_APPLE_BEARER_TOKEN";

/// What a browser calls itself. Apple's web endpoints answer a browser, and a bare reqwest
/// agent gets a different page.
pub(crate) const AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";

/// The page carrying the web player's bearer token, either in its own markup or in the script
/// bundle it names.
const PLAYER: &str = "https://music.apple.com/";

/// The file beside the account's that keeps the last bearer token read off the page.
const KEPT: &str = "bearer.json";

/// How long before it expires a kept bearer token is read again, so a launch never starts on
/// one about to lapse.
const MARGIN: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How long the page or its script bundle may take to arrive. They are megabytes, and a stalled
/// one would otherwise hold the sign-in for minutes.
const PATIENCE: Duration = Duration::from_secs(30);

/// The bearer token this process is using. It is the same for every visitor and lasts months,
/// so it is read once and kept on disk for the next launch.
static BEARER: Mutex<Option<String>> = Mutex::new(None);

/// The bearer token as it is kept on disk.
#[derive(Deserialize, Serialize)]
struct KeptBearer {
    token: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Credentials {
    pub user_token: String,
}

fn file() -> PathBuf {
    credentials::dir("apple").join(credentials::FILE)
}

/// The stored account token, or the one the environment names. Reading the environment here is
/// what lets the probe run without a sign-in.
pub(crate) fn load() -> Option<String> {
    if let Some(token) = std::env::var(USER_TOKEN_ENV)
        .ok()
        .filter(|it| !it.is_empty())
    {
        return Some(token);
    }
    let body = std::fs::read(file()).ok()?;
    match serde_json::from_slice::<Credentials>(&body) {
        Ok(stored) => Some(stored.user_token),
        Err(error) => {
            log::warn!("apple: cannot read the stored credentials: {error}");
            None
        }
    }
}

pub(crate) fn store(token: &str) -> Result<()> {
    let body = serde_json::to_vec_pretty(&Credentials {
        user_token: token.to_owned(),
    })
    .context("cannot encode apple credentials")?;
    credentials::write(&file(), &body)
}

pub(crate) fn forget() {
    credentials::remove(&file());
}

pub(crate) fn stored() -> bool {
    file().exists() || std::env::var_os(USER_TOKEN_ENV).is_some()
}

/// Pulls the account token out of what the sign-in window brought back, which is a `Cookie`
/// header. One value pasted by hand works the same way, bare or still carrying its name, so
/// copying the cookie out of the devtools storage answers as well as the whole header.
pub fn user_token(input: &str) -> Result<String> {
    let input = input.trim();
    if input.contains(';') {
        // A whole header holds many cookies: take the segment ours names.
        let found = input
            .split(';')
            .map(str::trim)
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.trim(), value.trim()))
            .find(|(name, _)| PROOF.contains(name))
            .map(|(_, value)| value);
        return match found {
            Some(token) if !token.is_empty() => Ok(token.to_owned()),
            _ => bail!("the cookies carry no media-user-token"),
        };
    }
    // One value: the token itself. A token can end in `=` padding, so only a leading name
    // is stripped and the rest is kept untouched.
    let token = match input.split_once('=') {
        Some((name, value)) if PROOF.contains(&name.trim()) => value.trim(),
        _ => input,
    };
    if token.is_empty() {
        bail!("the media-user-token is empty");
    }
    Ok(token.to_owned())
}

/// The web player's bearer token: the environment's, the one already read, the one the last
/// launch kept while it has more than `MARGIN` left, or one read off the page now. Reading the
/// page costs megabytes, so a launch with a kept token signs in without it.
pub async fn bearer(http: &reqwest::Client) -> Result<String> {
    if let Some(token) = std::env::var(BEARER_ENV).ok().filter(|it| !it.is_empty()) {
        return Ok(token);
    }
    if let Some(token) = held() {
        return Ok(token);
    }
    if let Some(token) = kept() {
        log::debug!("apple: using the bearer token the last launch kept");
        *BEARER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(token.clone());
        return Ok(token);
    }
    let token = read_bearer(http).await?;
    log::debug!("apple: read a bearer token of {} characters", token.len());
    keep(&token);
    *BEARER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(token.clone());
    Ok(token)
}

/// Drops the bearer token in use and the kept one, after Apple turned it down, so the next
/// `bearer` reads the page again.
pub(crate) fn forget_bearer() {
    *BEARER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    credentials::remove(&credentials::dir("apple").join(KEPT));
}

fn held() -> Option<String> {
    BEARER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The token the last launch kept, while it is still good for longer than `MARGIN`.
fn kept() -> Option<String> {
    let body = std::fs::read(credentials::dir("apple").join(KEPT)).ok()?;
    let token = serde_json::from_slice::<KeptBearer>(&body).ok()?.token;
    let left = expiry(&token)?.duration_since(SystemTime::now()).ok()?;
    (left > MARGIN).then_some(token)
}

fn keep(token: &str) {
    let body = match serde_json::to_vec(&KeptBearer {
        token: token.to_owned(),
    }) {
        Ok(body) => body,
        Err(error) => return log::warn!("apple: cannot encode the bearer token: {error}"),
    };
    if let Err(error) = credentials::write(&credentials::dir("apple").join(KEPT), &body) {
        log::warn!("apple: cannot keep the bearer token: {error:#}");
    }
}

/// When a JWT stops being accepted, read from the `exp` claim of its payload.
fn expiry(token: &str) -> Option<SystemTime> {
    let payload = URL_SAFE_NO_PAD.decode(token.split('.').nth(1)?).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(claims.get("exp")?.as_u64()?))
}

/// Reads the token off music.apple.com: out of the page itself, or out of the script bundle the
/// page names. Apple has moved it between the two, so both are tried before giving up.
async fn read_bearer(http: &reqwest::Client) -> Result<String> {
    let html = http
        .get(PLAYER)
        .header(reqwest::header::USER_AGENT, AGENT)
        .timeout(PATIENCE)
        .send()
        .await
        .context("cannot reach music.apple.com")?
        .error_for_status()
        .context("music.apple.com refused the request")?
        .text()
        .await
        .context("cannot read music.apple.com")?;

    if let Some(token) = jwt(&html) {
        return Ok(token.to_owned());
    }
    let path = bundle(&html).context("music.apple.com names no script bundle")?;
    let script = http
        .get(format!("https://music.apple.com{path}"))
        .header(reqwest::header::USER_AGENT, AGENT)
        .timeout(PATIENCE)
        .send()
        .await
        .context("cannot fetch the apple music script bundle")?
        .error_for_status()
        .context("apple refused the script bundle")?
        .text()
        .await
        .context("cannot read the apple music script bundle")?;
    jwt(&script)
        .map(str::to_owned)
        .context("the apple music script bundle carries no bearer token")
}

/// The path of the script bundle the page names, `/assets/index~<hash>.js`.
fn bundle(html: &str) -> Option<&str> {
    let at = html.find("/assets/index~")?;
    let rest = &html[at..];
    let end = rest.find(".js")? + ".js".len();
    let path = rest.get(..end)?;
    (!path["/assets/".len()..].contains('/')).then_some(path)
}

/// The first JWT in a body: three dot-separated base64url runs starting with the `eyJ` that a
/// JSON header always encodes to. Length is what tells a token from an incidental match.
fn jwt(body: &str) -> Option<&str> {
    let mut from = 0usize;
    while let Some(at) = body[from..].find("eyJ") {
        let start = from + at;
        let len = body[start..]
            .chars()
            .take_while(|letter| {
                letter.is_ascii_alphanumeric() || matches!(letter, '-' | '_' | '.')
            })
            .count();
        let token = &body[start..start + len];
        if token.matches('.').count() == 2 && token.len() >= 100 {
            return Some(token);
        }
        from = start + 3;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_user_token_out_of_a_cookie_header() {
        let header = "geo=PL; media-user-token=AbCd123==; s_vi=x";
        assert_eq!(user_token(header).unwrap(), "AbCd123==");
    }

    #[test]
    fn accepts_a_bare_token() {
        assert_eq!(user_token("  AbCd123  ").unwrap(), "AbCd123");
    }

    #[test]
    fn refuses_cookies_without_the_account() {
        assert!(user_token("geo=PL; s_vi=x").is_err());
        assert!(user_token("media-user-token=").is_err());
        assert!(user_token("").is_err());
    }

    #[test]
    fn finds_the_script_bundle_the_page_names() {
        let html = r#"<script src="/assets/index~abc123.js" type="module">"#;
        assert_eq!(bundle(html), Some("/assets/index~abc123.js"));
        assert_eq!(bundle("<html></html>"), None);
    }

    #[test]
    fn finds_a_jwt_and_ignores_a_short_match() {
        let token = format!(
            "eyJ{}.{}.{}",
            "a".repeat(60),
            "b".repeat(60),
            "c".repeat(60)
        );
        let body = format!("var t=\"eyJshort.a.b\",u=\"{token}\";");
        assert_eq!(jwt(&body), Some(token.as_str()));
        assert_eq!(jwt("nothing here"), None);
    }
}
