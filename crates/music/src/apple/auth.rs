//! What Apple Music needs to answer for an account: the web player's own bearer token, and the
//! `media-user-token` cookie the sign-in window brings back.
//!
//! The bearer token is the same public one every visitor to music.apple.com gets; it is read
//! off the page rather than registered for, which makes it the brittle half of this file and
//! the reason it is isolated here. The user token is the account, and it is never logged.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context as _, Result, bail};
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

/// The bearer token this process is using. It is the same for every visitor and lasts months,
/// so it is read once.
static BEARER: OnceLock<String> = OnceLock::new();

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
/// header. A bare token is accepted too, so a value pasted by hand works the same way.
pub fn user_token(input: &str) -> Result<String> {
    let input = input.trim();
    let found = input
        .split(';')
        .map(str::trim)
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| PROOF.contains(name))
        .map(|(_, value)| value.trim());
    let token = match found {
        Some(token) => token,
        None if !input.is_empty() && !input.contains(['=', ';', ' ']) => input,
        None => bail!("the cookies carry no media-user-token"),
    };
    if token.is_empty() {
        bail!("the media-user-token is empty");
    }
    Ok(token.to_owned())
}

/// The web player's bearer token: the environment's, the one already read, or one read off the
/// page now.
pub async fn bearer(http: &reqwest::Client) -> Result<String> {
    if let Some(token) = std::env::var(BEARER_ENV).ok().filter(|it| !it.is_empty()) {
        return Ok(token);
    }
    if let Some(token) = BEARER.get() {
        return Ok(token.clone());
    }
    let token = read_bearer(http).await?;
    log::debug!("apple: read a bearer token of {} characters", token.len());
    Ok(BEARER.get_or_init(|| token).clone())
}

/// Reads the token off music.apple.com: out of the page itself, or out of the script bundle the
/// page names. Apple has moved it between the two, so both are tried before giving up.
async fn read_bearer(http: &reqwest::Client) -> Result<String> {
    let html = http
        .get(PLAYER)
        .header(reqwest::header::USER_AGENT, AGENT)
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
