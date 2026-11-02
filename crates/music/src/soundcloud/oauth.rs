//! OAuth 2.1 sign-in for a registered SoundCloud app: the authorization code flow with PKCE that
//! `secure.soundcloud.com` asks for, a loopback callback, and the access and refresh tokens it
//! hands back. Only a grant from here may write, since the `oauth_token` cookie reads alone.
//!
//! Sonora ships no app of its own. SoundCloud signs every token request with the app's secret,
//! which an open binary cannot keep, so the app comes from the environment or from a build that
//! baked one in, and without one the provider offers the cookie sign-in only.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use oauth2::{CsrfToken, PkceCodeChallenge};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::{PromptSink, SignInFailure, SignInProblem, SignInPrompt, escape};

/// Where the browser asks the listener for consent.
const AUTHORIZE: &str = "https://secure.soundcloud.com/authorize";
/// Where a code or a refresh token is exchanged for an access token.
pub(crate) const TOKEN: &str = "https://secure.soundcloud.com/oauth/token";
/// The loopback address the browser comes back on, unless the app registered another. Spotify
/// listens on 8989 and scrobbling on 8990.
const REDIRECT: &str = "http://127.0.0.1:8991/soundcloud";
/// How long the listener has to approve in the browser before the sign-in is given up on.
const WAIT: Duration = Duration::from_secs(300);
/// How long before its expiry an access token is refreshed, so a request never carries one that
/// lapses on the way.
const MARGIN: u64 = 60;

const DONE: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
    <html><body>Sonora is connected to SoundCloud. You can close this window.</body></html>";
const ELSEWHERE: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// A registered SoundCloud app: the pair SoundCloud issued for it and the redirect it was
/// registered with, which must match the one sent here character for character.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct App {
    pub client_id: String,
    pub secret: String,
    pub redirect_uri: String,
}

/// What a finished sign-in or refresh leaves: the token every request carries, the one that
/// renews it, and when the first one lapses, in unix seconds. A token with no expiry is never
/// refreshed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Grant {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: Option<u64>,
}

/// The token endpoint's answer, success and refusal alike.
#[derive(Deserialize)]
struct Answer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

impl App {
    /// The app named by `SONORA_SOUNDCLOUD_CLIENT_ID` and `SONORA_SOUNDCLOUD_CLIENT_SECRET`,
    /// read at run time first and at build time second. `SONORA_SOUNDCLOUD_REDIRECT_URI`
    /// overrides the loopback redirect. None unless both halves of the pair are there.
    pub fn configured() -> Option<Self> {
        let client_id = setting(
            "SONORA_SOUNDCLOUD_CLIENT_ID",
            option_env!("SONORA_SOUNDCLOUD_CLIENT_ID"),
        )?;
        let secret = setting(
            "SONORA_SOUNDCLOUD_CLIENT_SECRET",
            option_env!("SONORA_SOUNDCLOUD_CLIENT_SECRET"),
        )?;
        let redirect_uri = setting(
            "SONORA_SOUNDCLOUD_REDIRECT_URI",
            option_env!("SONORA_SOUNDCLOUD_REDIRECT_URI"),
        )
        .unwrap_or_else(|| REDIRECT.to_owned());
        Some(Self {
            client_id,
            secret,
            redirect_uri,
        })
    }
}

impl Grant {
    /// A token with no refresh and no expiry, such as the browser's `oauth_token` cookie.
    pub fn lasting(token: &str) -> Self {
        Self {
            access: token.to_owned(),
            refresh: None,
            expires_at: None,
        }
    }

    /// Whether the access token lapses within the margin of `now`, in unix seconds.
    pub fn stale(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|at| now + MARGIN >= at)
    }
}

/// Runs the whole round trip: listens on the redirect, sends the browser to SoundCloud's
/// consent page, waits for the code and exchanges it. The url also goes to `prompt`, for a
/// browser that did not open on its own.
pub(crate) async fn authorize(
    http: &reqwest::Client,
    app: &App,
    prompt: PromptSink,
) -> Result<Grant> {
    let address = loopback(&app.redirect_uri).with_context(|| {
        format!(
            "the soundcloud redirect {} is not a loopback http address",
            app.redirect_uri
        )
    })?;
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("cannot listen for the soundcloud sign-in at {address}"))?;
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let state = CsrfToken::new_random();
    let url = authorize_url(app, challenge.as_str(), state.secret());
    prompt(SignInPrompt::Url(url.clone()));
    open::that_in_background(&url);

    let code = wait_for_code(listener, state.secret()).await?;
    exchange(http, TOKEN, app, &code, verifier.secret()).await
}

/// The consent page for `app`, carrying the PKCE challenge and the state the callback has to
/// echo back.
pub(crate) fn authorize_url(app: &App, challenge: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type=code\
         &code_challenge={}&code_challenge_method=S256&state={}",
        escape::component(&app.client_id),
        escape::component(&app.redirect_uri),
        escape::component(challenge),
        escape::component(state),
    )
}

/// Exchanges an authorization code at `endpoint`, proving the sign-in with the PKCE verifier.
pub(crate) async fn exchange(
    http: &reqwest::Client,
    endpoint: &str,
    app: &App,
    code: &str,
    verifier: &str,
) -> Result<Grant> {
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", app.client_id.as_str()),
        ("client_secret", app.secret.as_str()),
        ("redirect_uri", app.redirect_uri.as_str()),
        ("code_verifier", verifier),
        ("code", code),
    ];
    token(http, endpoint, &form).await
}

/// Renews an access token at `endpoint`. SoundCloud may rotate the refresh token too; when the
/// answer carries none, the one that was sent stays good.
pub(crate) async fn refresh(
    http: &reqwest::Client,
    endpoint: &str,
    app: &App,
    refresh_token: &str,
) -> Result<Grant> {
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", app.client_id.as_str()),
        ("client_secret", app.secret.as_str()),
        ("refresh_token", refresh_token),
    ];
    let mut grant = token(http, endpoint, &form)
        .await
        .context("cannot renew the soundcloud sign-in")?;
    if grant.refresh.is_none() {
        grant.refresh = Some(refresh_token.to_owned());
    }
    Ok(grant)
}

/// The current time in unix seconds.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Waits for the browser to come back with the code. A request that carries neither a code nor
/// an error, such as the favicon, is answered and ignored, so it does not end the wait.
async fn wait_for_code(listener: TcpListener, state: &str) -> Result<String> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let accepted = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| SignInFailure(SignInProblem::Cancelled))
            .context("the browser did not come back from soundcloud in time")?;
        let (mut stream, _) = accepted.context("cannot accept the soundcloud callback")?;
        let mut line = String::new();
        if let Err(error) = BufReader::new(&mut stream).read_line(&mut line).await {
            log::warn!("soundcloud: cannot read the sign-in callback: {error}");
            continue;
        }
        let read = callback(&line, state);
        let page = match read {
            Ok(None) => ELSEWHERE,
            _ => DONE,
        };
        stream.write_all(page.as_bytes()).await.ok();
        if let Some(code) = read? {
            return Ok(code);
        }
    }
}

/// Reads the code out of the callback's request line. A denied consent is a cancelled sign-in,
/// any other error a refused one, and a state other than `state` a callback meant for another
/// sign-in. None for a request that is not the callback at all.
pub(crate) fn callback(request: &str, state: &str) -> Result<Option<String>> {
    let code = parameter(request, "code");
    let error = parameter(request, "error");
    if code.is_none() && error.is_none() {
        return Ok(None);
    }
    if parameter(request, "state").as_deref() != Some(state) {
        bail!("the soundcloud callback does not belong to this sign-in");
    }
    if let Some(error) = error {
        let problem = match error.as_str() {
            "access_denied" => SignInProblem::Cancelled,
            _ => SignInProblem::Refused,
        };
        let description = parameter(request, "error_description").unwrap_or_default();
        return Err(anyhow::Error::new(SignInFailure(problem)).context(format!(
            "soundcloud refused the sign-in ({error}): {description}"
        )));
    }
    Ok(code)
}

/// Posts `form` to the token endpoint and reads the grant out of the answer. A refused code,
/// refresh token or app is a credentials failure, with SoundCloud's own reason beside it.
async fn token(http: &reqwest::Client, endpoint: &str, form: &[(&str, &str)]) -> Result<Grant> {
    let response = http
        .post(endpoint)
        .header("Accept", "application/json; charset=utf-8")
        .form(form)
        .send()
        .await
        .context("cannot reach soundcloud's sign-in")?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("cannot read soundcloud's sign-in answer")?;
    let answer: Option<Answer> = serde_json::from_str(&text).ok();
    if let Some(Answer {
        error: Some(error),
        error_description,
        ..
    }) = &answer
    {
        let problem = match error.as_str() {
            "invalid_grant" | "invalid_client" | "unauthorized_client" => {
                SignInProblem::Credentials
            }
            _ => SignInProblem::Refused,
        };
        let description = error_description.as_deref().unwrap_or("no reason given");
        return Err(anyhow::Error::new(SignInFailure(problem)).context(format!(
            "soundcloud refused the sign-in ({error}): {description}"
        )));
    }
    if !status.is_success() {
        let problem = match status.as_u16() {
            400 | 401 | 403 => SignInProblem::Credentials,
            _ => SignInProblem::Refused,
        };
        return Err(anyhow::Error::new(SignInFailure(problem))
            .context(format!("soundcloud refused the sign-in: http {status}")));
    }
    let answer = answer.context("soundcloud's sign-in answered with no json")?;
    let access = answer
        .access_token
        .filter(|token| !token.is_empty())
        .context("soundcloud's sign-in answer carries no access token")?;
    Ok(Grant {
        access,
        refresh: answer.refresh_token.filter(|token| !token.is_empty()),
        expires_at: answer.expires_in.map(|seconds| now() + seconds),
    })
}

/// A run-time setting, else the build-time one, trimmed. Empty counts as unset.
fn setting(name: &str, built: Option<&'static str>) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| built.map(str::to_owned))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The `host:port` an `http://` loopback redirect listens on. Anything else cannot be answered
/// from here, so it answers none.
fn loopback(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("http://")?;
    let authority = rest.split(['/', '?']).next()?;
    let (host, port) = authority.rsplit_once(':')?;
    let local = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
    let numeric = !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit());
    (local && numeric).then(|| authority.to_owned())
}

/// One query parameter of an HTTP request line, decoded.
fn parameter(request: &str, name: &str) -> Option<String> {
    let (_, query) = request.split_whitespace().nth(1)?.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| {
            percent_decode_str(&value.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundcloud::mock::Mock;

    fn app() -> App {
        App {
            client_id: "app-id".to_owned(),
            secret: "app-secret".to_owned(),
            redirect_uri: REDIRECT.to_owned(),
        }
    }

    fn failure(error: &anyhow::Error) -> Option<SignInProblem> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<SignInFailure>())
            .map(|failure| failure.0)
    }

    #[tokio::test]
    async fn signs_in_against_a_mocked_token_endpoint() {
        let mock = Mock::start(|_| {
            (
                200,
                r#"{"access_token":"2-access","refresh_token":"refresh-1","expires_in":3599,"scope":""}"#
                    .to_owned(),
            )
        })
        .await;
        let before = now();
        let grant = exchange(
            &Mock::http(),
            &mock.url("/oauth/token"),
            &app(),
            "the-code",
            "the-verifier",
        )
        .await
        .unwrap();

        assert_eq!(grant.access, "2-access");
        assert_eq!(grant.refresh.as_deref(), Some("refresh-1"));
        let expires_at = grant.expires_at.unwrap();
        assert!(expires_at >= before + 3599 && expires_at <= now() + 3599);

        let sent = mock.requests();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].line.starts_with("POST /oauth/token "));
        for pair in [
            "grant_type=authorization_code",
            "client_id=app-id",
            "client_secret=app-secret",
            "code=the-code",
            "code_verifier=the-verifier",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A8991%2Fsoundcloud",
        ] {
            assert!(
                sent[0].body.contains(pair),
                "{pair} missing from {}",
                sent[0].body
            );
        }
    }

    #[tokio::test]
    async fn a_refused_sign_in_says_why() {
        let mock = Mock::start(|_| {
            (
                401,
                r#"{"error":"invalid_grant","error_description":"The authorization code has expired"}"#
                    .to_owned(),
            )
        })
        .await;
        let error = exchange(&Mock::http(), &mock.url("/oauth/token"), &app(), "old", "v")
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "soundcloud refused the sign-in (invalid_grant): The authorization code has expired"
        );
        assert_eq!(failure(&error), Some(SignInProblem::Credentials));
    }

    #[tokio::test]
    async fn a_sign_in_answer_without_a_token_is_an_error() {
        let mock = Mock::start(|_| (200, r#"{"token_type":"bearer"}"#.to_owned())).await;
        let error = exchange(&Mock::http(), &mock.url("/oauth/token"), &app(), "c", "v")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("carries no access token"));
    }

    #[tokio::test]
    async fn a_refresh_keeps_the_refresh_token_it_was_not_given_a_new_one_for() {
        let mock = Mock::start(|_| {
            (
                200,
                r#"{"access_token":"2-fresh","expires_in":3600}"#.to_owned(),
            )
        })
        .await;
        let grant = refresh(
            &Mock::http(),
            &mock.url("/oauth/token"),
            &app(),
            "refresh-1",
        )
        .await
        .unwrap();

        assert_eq!(grant.access, "2-fresh");
        assert_eq!(grant.refresh.as_deref(), Some("refresh-1"));
        let body = &mock.requests()[0].body;
        assert!(body.contains("grant_type=refresh_token"));
        assert!(body.contains("refresh_token=refresh-1"));
        assert!(body.contains("client_secret=app-secret"));
    }

    #[test]
    fn the_callback_hands_over_the_code() {
        let line = "GET /soundcloud?code=1-abc%2Bdef&state=xyz HTTP/1.1\r\n";
        assert_eq!(callback(line, "xyz").unwrap().as_deref(), Some("1-abc+def"));
    }

    #[test]
    fn a_denied_consent_is_a_cancelled_sign_in() {
        let line = "GET /soundcloud?error=access_denied&error_description=The+user+denied&state=xyz HTTP/1.1";
        let error = callback(line, "xyz").unwrap_err();
        assert_eq!(failure(&error), Some(SignInProblem::Cancelled));
        assert_eq!(
            error.to_string(),
            "soundcloud refused the sign-in (access_denied): The user denied"
        );
    }

    #[test]
    fn a_callback_for_another_sign_in_is_refused() {
        let line = "GET /soundcloud?code=1-abc&state=other HTTP/1.1";
        let error = callback(line, "xyz").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not belong to this sign-in")
        );
    }

    #[test]
    fn a_stray_request_does_not_end_the_wait() {
        assert!(
            callback("GET /favicon.ico HTTP/1.1", "xyz")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_consent_page_carries_the_pkce_challenge() {
        let url = authorize_url(&app(), "the-challenge", "the-state");
        assert!(url.starts_with("https://secure.soundcloud.com/authorize?"));
        for pair in [
            "client_id=app-id",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A8991%2Fsoundcloud",
            "response_type=code",
            "code_challenge=the-challenge",
            "code_challenge_method=S256",
            "state=the-state",
        ] {
            assert!(url.contains(pair), "{pair} missing from {url}");
        }
        assert!(!url.contains("app-secret"));
    }

    #[test]
    fn a_grant_goes_stale_a_minute_early() {
        let grant = Grant {
            access: "a".to_owned(),
            refresh: None,
            expires_at: Some(1_000),
        };
        assert!(!grant.stale(1_000 - MARGIN - 1));
        assert!(grant.stale(1_000 - MARGIN));
        let lasting = Grant {
            expires_at: None,
            ..grant
        };
        assert!(!lasting.stale(u64::MAX - MARGIN));
    }

    #[test]
    fn only_a_loopback_redirect_is_listened_on() {
        assert_eq!(loopback(REDIRECT).as_deref(), Some("127.0.0.1:8991"));
        assert_eq!(
            loopback("http://localhost:9000/cb?x=1").as_deref(),
            Some("localhost:9000")
        );
        assert!(loopback("https://127.0.0.1:8991/soundcloud").is_none());
        assert!(loopback("http://example.com:80/soundcloud").is_none());
        assert!(loopback("http://127.0.0.1/soundcloud").is_none());
        assert!(loopback("sonora://soundcloud").is_none());
    }
}
