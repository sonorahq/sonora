//! Sign-in for Tidal: the OAuth device flow a TV app uses, and the tokens it leaves behind,
//! kept in the provider's own cache folder.
//!
//! The listener is shown a short code and a link, approves the device on Tidal's site, and
//! the app polls until the approval lands. The access token lasts four hours and the refresh
//! token far longer, so a stored sign-in is renewed in place rather than asked for again.
//!
//! Since April 2026 the device client is served AAC where it asks for CD-quality FLAC. The
//! refresh token it is handed can be renewed as Tidal's Android app instead, which is served
//! FLAC at every tier, so the first thing a new sign-in does is exactly that, the way tidalapi's
//! `client_enable_hires` does. Tidal may stop allowing it; then the sign-in stays with the
//! device client and plays what that client is given.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::tidal::lock;
use crate::{PromptSink, SignInFailure, SignInProblem, SignInPrompt, credentials};

const DEVICE: &str = "https://auth.tidal.com/v1/oauth2/device_authorization";
const TOKEN: &str = "https://auth.tidal.com/v1/oauth2/token";

/// The limited-input client Tidal's car and TV apps sign in as, the only kind the device flow
/// is open to.
const DEVICE_CLIENT: Client = Client {
    id: "fX2JxdmntZWK0ixT",
    secret: "1Nn9AfDAjxrgJFJbKNWLeAyKGVGmINuXPPLHVXAvxAg=",
};
/// The client Tidal's Android app signs in as, which is served FLAC at every tier.
const APP_CLIENT: Client = Client {
    id: "6BDSRdpK9hqEBTgU",
    secret: "xeuPmY7nbpZ9IIbLAcQ93shka1VNheUAqN6IcszjTG8=",
};
const SCOPE: &str = "r_usr w_usr w_sub";
/// Where a device is approved, as the sign-in screen shows it.
const LINK: &str = "link.tidal.com";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// How long the device flow waits between polls when Tidal names no interval, and how much
/// longer it waits each time Tidal says to slow down, both as RFC 8628 sets them.
const POLL: Duration = Duration::from_secs(5);
/// The OAuth errors that mean the token or the client was refused, rather than the request
/// failing. Only these give up on a session.
const REFUSALS: [&str; 3] = ["invalid_grant", "invalid_client", "unauthorized_client"];

/// How long before it expires an access token is renewed, so a request never leaves with one
/// that runs out on the way.
const MARGIN: Duration = Duration::from_secs(300);

/// Bumped by every sign-out, and held while credentials are written or removed, so a renewal
/// that finishes after a sign-out cannot write the sign-in back.
static SIGN_OUTS: Mutex<u64> = Mutex::new(0);

/// Tidal answering that it is down, busy or slow, rather than refusing anything.
#[derive(Debug)]
pub(crate) struct Unavailable;

/// An OAuth client of Tidal's own: who a token is issued to, which decides the quality it is
/// served.
#[derive(Clone, Copy, Debug)]
struct Client {
    id: &'static str,
    secret: &'static str,
}

/// Which client a session renews as, which decides whether it is served FLAC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Lane {
    /// The device client, moved to the app client at the next renewal that manages it.
    #[default]
    Device,
    /// The app client, served FLAC.
    App,
    /// The device client for good, since Tidal refused to move the session.
    Refused,
}

/// A signed-in session as it is kept on disk: the tokens, who they belong to, and which client
/// renews them. Renewing replaces the whole thing, so a caller never edits one in place.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds since the epoch at which `access_token` stops working.
    pub expires_at: u64,
    pub user_id: String,
    pub country_code: String,
    /// Which client the session renews as. One stored without it renews as the device client
    /// and is moved on the way.
    #[serde(default)]
    pub lane: Lane,
}

/// What the device authorization endpoint hands out.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: Option<String>,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

/// A token answer, after a device approval or a refresh.
#[derive(Deserialize)]
struct Token {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    user: Option<User>,
    user_id: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    user_id: u64,
    country_code: Option<String>,
}

/// A refused token request. `error` is the OAuth error code.
#[derive(Default, Deserialize)]
struct Refusal {
    #[serde(default)]
    error: String,
    sub_status: Option<u64>,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tidal is unavailable for now")
    }
}

impl std::error::Error for Unavailable {}

impl Credentials {
    /// Whether the access token is close enough to its end to renew before using it.
    pub fn stale(&self) -> bool {
        now() + MARGIN.as_secs() >= self.expires_at
    }
}

/// Runs the device flow: shows the code, opens the approval page, and polls until the
/// listener approves, refuses or lets the code expire. Cancelling is the caller's to watch for,
/// by dropping this future.
pub(crate) async fn device(http: &reqwest::Client, prompt: &PromptSink) -> Result<Credentials> {
    let device: Device = http
        .post(DEVICE)
        .form(&[("client_id", DEVICE_CLIENT.id), ("scope", SCOPE)])
        .send()
        .await
        .context("cannot reach tidal to start the sign-in")?
        .error_for_status()
        .context("tidal refused to start the sign-in")?
        .json()
        .await
        .context("cannot read the tidal sign-in code")?;
    // Tidal answers both links without a scheme, and may leave either out.
    let shown = device
        .verification_uri
        .as_deref()
        .map_or(LINK, |uri| uri.trim_start_matches("https://"));
    let approve = device
        .verification_uri_complete
        .as_deref()
        .map_or_else(|| format!("{LINK}/{}", device.user_code), str::to_owned);
    prompt(SignInPrompt::Code {
        code: device.user_code.clone(),
        url: shown.to_owned(),
    });
    open::that_in_background(match approve.starts_with("http") {
        true => approve,
        false => format!("https://{approve}"),
    });

    let mut interval = device.interval.map_or(POLL, Duration::from_secs);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in);
    loop {
        tokio::time::sleep(interval).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(SignInFailure(SignInProblem::Cancelled))
                .context("the tidal sign-in code expired");
        }
        let sent = http
            .post(TOKEN)
            .form(&[
                ("client_id", DEVICE_CLIENT.id),
                ("client_secret", DEVICE_CLIENT.secret),
                ("device_code", device.device_code.as_str()),
                ("grant_type", DEVICE_GRANT),
                ("scope", SCOPE),
            ])
            .send()
            .await;
        // One poll failing says nothing about the approval, so the wait carries on until the
        // code expires.
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                log::debug!("tidal: cannot poll the sign-in, trying again: {error}");
                continue;
            }
        };
        if response.status().is_success() {
            let token: Token = response
                .json()
                .await
                .context("cannot read the tidal sign-in answer")?;
            let approved = credentials_from(token, None)?;
            return Ok(upgrade(http, approved).await);
        }
        let status = response.status();
        let refusal: Refusal = response.json().await.unwrap_or_default();
        match refusal.error.as_str() {
            "authorization_pending" => {}
            "" => log::debug!("tidal: the sign-in poll answered {status}, trying again"),
            "slow_down" => interval += POLL,
            "expired_token" => {
                return Err(SignInFailure(SignInProblem::Cancelled))
                    .context("the tidal sign-in code expired");
            }
            "access_denied" => {
                return Err(SignInFailure(SignInProblem::Cancelled))
                    .context("the tidal sign-in was refused");
            }
            other => bail!(
                "tidal refused the sign-in: {other} ({})",
                refusal.sub_status.unwrap_or_default()
            ),
        }
    }
}

/// Moves a device sign-in over to the app client, so it is served FLAC. Failing to is not a
/// failed sign-in: the device session still plays, at the quality it is given, and unless
/// Tidal refused the move it is tried again at the next renewal.
async fn upgrade(http: &reqwest::Client, device: Credentials) -> Credentials {
    match renew(http, &device, APP_CLIENT).await {
        Ok(upgraded) => {
            log::info!("tidal: the session is served lossless");
            Credentials {
                lane: Lane::App,
                ..upgraded
            }
        }
        Err(error) => {
            log::warn!(
                "tidal: cannot move the session to the app client yet, so lossless tracks play as aac: {error:#}"
            );
            let lane = match refused(&error) {
                true => Lane::Refused,
                false => Lane::Device,
            };
            Credentials { lane, ..device }
        }
    }
}

/// Swaps the refresh token for a new access token. A session renews as the app client unless
/// Tidal has refused it there, and a refusal from the app client falls back to the device
/// client for good. Any other failure is the caller's, except for a device session still
/// waiting to move, which renews as it is and tries again next time. A refusal from the client
/// the session ends up on means the sign-in is gone.
pub(crate) async fn refresh(http: &reqwest::Client, stale: &Credentials) -> Result<Credentials> {
    let (renewed, lane) = match stale.lane {
        Lane::Refused => (renew(http, stale, DEVICE_CLIENT).await?, Lane::Refused),
        Lane::App | Lane::Device => match renew(http, stale, APP_CLIENT).await {
            Ok(renewed) => (renewed, Lane::App),
            Err(error) if refused(&error) => {
                log::warn!("tidal: the app client refused the session, falling back: {error:#}");
                (renew(http, stale, DEVICE_CLIENT).await?, Lane::Refused)
            }
            Err(error) if stale.lane == Lane::Device => {
                log::debug!("tidal: cannot move the session to the app client yet: {error:#}");
                (renew(http, stale, DEVICE_CLIENT).await?, Lane::Device)
            }
            Err(error) => return Err(error),
        },
    };
    if stale.lane == Lane::Device && lane == Lane::App {
        log::info!("tidal: the session is served lossless");
    }
    Ok(Credentials { lane, ..renewed })
}

/// Whether an answer is Tidal being down, busy or slow rather than refusing anything, which
/// clears on its own.
pub(crate) fn transient(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
}

/// Whether a failure is Tidal being down or busy, which is worth waiting out rather than
/// signing out over.
pub(crate) fn unavailable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<Unavailable>())
}

/// Whether a failure is Tidal refusing the credentials, as opposed to the network or Tidal
/// itself failing. Only a refusal is a reason to give up on a stored sign-in.
pub(crate) fn refused(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<SignInFailure>()
            .is_some_and(|failure| failure.0 == SignInProblem::Credentials)
    })
}

/// One refresh round trip as `client`.
async fn renew(http: &reqwest::Client, stale: &Credentials, client: Client) -> Result<Credentials> {
    let response = http
        .post(TOKEN)
        .form(&[
            ("client_id", client.id),
            ("client_secret", client.secret),
            ("refresh_token", stale.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
            ("scope", SCOPE),
        ])
        .send()
        .await
        .context("cannot reach tidal to renew the session")?;
    let status = response.status();
    if !status.is_success() {
        let refusal: Refusal = response.json().await.unwrap_or_default();
        let reason = format!(
            "tidal would not renew the session: {} ({status}, {})",
            refusal.error,
            refusal.sub_status.unwrap_or_default()
        );
        // Anything else, a timeout or being told to slow down among them, says nothing about
        // the credentials themselves.
        if REFUSALS.contains(&refusal.error.as_str()) {
            return Err(SignInFailure(SignInProblem::Credentials)).context(reason);
        }
        return match transient(status) {
            true => Err(Unavailable).context(reason),
            false => Err(anyhow::anyhow!(reason)),
        };
    }
    let token: Token = response
        .json()
        .await
        .context("cannot read the renewed tidal session")?;
    credentials_from(token, Some(stale))
}

/// Builds stored credentials from a token answer. A refresh answer leaves out what does not
/// change, which is read from the credentials it renews; which client the session renews as is
/// the caller's to settle.
fn credentials_from(token: Token, previous: Option<&Credentials>) -> Result<Credentials> {
    let user_id = token
        .user
        .as_ref()
        .map(|user| user.user_id)
        .or(token.user_id)
        .map(|id| id.to_string())
        .or_else(|| previous.map(|previous| previous.user_id.clone()))
        .context("tidal named no account for the session")?;
    let country_code = token
        .user
        .as_ref()
        .and_then(|user| user.country_code.clone())
        .or_else(|| previous.map(|previous| previous.country_code.clone()))
        .unwrap_or_else(|| "US".to_owned());
    let refresh_token = token
        .refresh_token
        .or_else(|| previous.map(|previous| previous.refresh_token.clone()))
        .context("tidal handed out no refresh token")?;
    Ok(Credentials {
        access_token: token.access_token,
        refresh_token,
        expires_at: now() + token.expires_in,
        user_id,
        country_code,
        lane: previous.map_or(Lane::Device, |previous| previous.lane),
    })
}

fn path() -> PathBuf {
    credentials::dir("tidal").join(credentials::FILE)
}

pub(crate) fn load() -> Option<Credentials> {
    let bytes = std::fs::read(path()).ok()?;
    let credentials: Credentials = serde_json::from_slice(&bytes)
        .inspect_err(|error| log::warn!("tidal: the stored sign-in cannot be read: {error}"))
        .ok()?;
    (!credentials.refresh_token.is_empty()).then_some(credentials)
}

/// How many sign-outs this process has seen. A session remembers the count it began at.
pub(crate) fn generation() -> u64 {
    *lock(&SIGN_OUTS)
}

/// The stored sign-in and the sign-out count it was read at, read together so a sign-out cannot
/// fall between them and leave the old sign-in counted as current.
pub(crate) fn restore() -> Option<(Credentials, u64)> {
    let held = lock(&SIGN_OUTS);
    Some((load()?, *held))
}

/// Stores a sign-in that has just been made.
pub(crate) fn store(credentials: &Credentials) -> Result<()> {
    let _held = lock(&SIGN_OUTS);
    write(credentials)
}

/// Stores a renewed sign-in, unless the listener has signed out since the session renewing it
/// began, which would otherwise bring back the sign-in they just removed.
pub(crate) fn keep(generation: u64, credentials: &Credentials) -> Result<()> {
    let held = lock(&SIGN_OUTS);
    match *held == generation {
        true => write(credentials),
        false => {
            log::debug!("tidal: not keeping a renewal from a session that was signed out");
            Ok(())
        }
    }
}

pub(crate) fn forget() {
    let mut held = lock(&SIGN_OUTS);
    *held += 1;
    credentials::remove(&path());
}

fn write(credentials: &Credentials) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(credentials).context("cannot serialize tidal credentials")?;
    credentials::write(&path(), &bytes).context("cannot store tidal credentials")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}
