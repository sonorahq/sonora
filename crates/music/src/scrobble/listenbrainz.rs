use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

use super::{Account, Link, Play, Secret, Service, host, root};

/// The public instance, used whenever the account names no `server` of its own.
const API: &str = "https://api.listenbrainz.org";
const SETTINGS: &str = "https://listenbrainz.org/settings/";
/// How many listens the endpoint takes in one call.
const BATCH: usize = 50;
const CLIENT: &str = "Sonora";

pub struct ListenBrainz;

#[async_trait]
impl Service for ListenBrainz {
    fn id(&self) -> &'static str {
        "listenbrainz"
    }

    fn link(&self) -> Link {
        Link::Token
    }

    fn signup(&self) -> Option<&'static str> {
        Some(SETTINGS)
    }

    async fn connect(&self, secret: Secret) -> Result<Account> {
        let Secret::Token { token, server } = secret else {
            bail!("listenbrainz needs a user token");
        };
        let token = token.trim().to_owned();
        if token.is_empty() {
            bail!("listenbrainz needs a user token");
        }
        let server = base(&server);
        let api = match server.is_empty() {
            true => API,
            false => server.as_str(),
        };

        let answer: Validation = super::http()
            .get(format!("{api}/1/validate-token"))
            .header(AUTHORIZATION, format!("Token {token}"))
            .send()
            .await
            .context("cannot reach listenbrainz")?
            .json()
            .await
            .context("cannot read the listenbrainz answer")?;

        if !answer.valid {
            let message = answer.message.unwrap_or_default();
            bail!("listenbrainz refused the token: {message}");
        }

        let name = answer
            .user_name
            .filter(|name| !name.is_empty())
            .or_else(|| (!server.is_empty()).then(|| host(&server)))
            .unwrap_or_default();
        Ok(Account {
            session: token,
            server,
            name,
            enabled: true,
            ..Account::default()
        })
    }

    async fn now_playing(&self, account: &Account, play: &Play) -> Result<()> {
        submit(account, "playing_now", &[listen(play, false)]).await
    }

    async fn scrobble(&self, account: &Account, plays: &[Play]) -> Result<()> {
        for batch in plays.chunks(BATCH) {
            let listens: Vec<Listen<'_>> = batch.iter().map(|play| listen(play, true)).collect();
            submit(account, "single", &listens).await?;
        }
        Ok(())
    }
}

/// Where this account's instance lives. An empty `server` means the public one.
fn api(account: &Account) -> &str {
    match account.server.is_empty() {
        true => API,
        false => account.server.trim_end_matches('/'),
    }
}

/// The root an instance's `/1/` endpoints hang off. Users paste either the root or the API path
/// that tools such as multi-scrobbler print, so a trailing `/1` is dropped.
fn base(url: &str) -> String {
    let url = root(url);
    match url.strip_suffix("/1") {
        Some(rest) => rest.trim_end_matches('/').to_owned(),
        None => url,
    }
}

async fn submit(account: &Account, kind: &str, payload: &[Listen<'_>]) -> Result<()> {
    let answer = super::http()
        .post(format!("{}/1/submit-listens", api(account)))
        .header(AUTHORIZATION, format!("Token {}", account.session))
        .json(&Submission {
            listen_type: kind,
            payload,
        })
        .send()
        .await
        .context("cannot reach listenbrainz")?;

    if answer.status().is_success() {
        return Ok(());
    }
    let status = answer.status();
    let body = answer.text().await.unwrap_or_default();
    bail!("listenbrainz refused the request ({status}): {body}");
}

/// Builds one listen. `timed` is false for the now-playing report, which carries no start time.
fn listen(play: &Play, timed: bool) -> Listen<'_> {
    Listen {
        listened_at: timed.then(|| play.timestamp()),
        track_metadata: Metadata {
            artist_name: &play.artist,
            track_name: &play.title,
            release_name: play.release(),
            additional_info: Extra {
                duration_ms: play.duration.as_millis().min(u64::MAX as u128) as u64,
                media_player: CLIENT,
                submission_client: CLIENT,
            },
        },
    }
}

#[derive(Deserialize)]
struct Validation {
    valid: bool,
    user_name: Option<String>,
    message: Option<String>,
}

#[derive(Serialize)]
struct Submission<'a> {
    listen_type: &'a str,
    payload: &'a [Listen<'a>],
}

#[derive(Serialize)]
struct Listen<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    listened_at: Option<i64>,
    track_metadata: Metadata<'a>,
}

#[derive(Serialize)]
struct Metadata<'a> {
    artist_name: &'a str,
    track_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_name: Option<&'a str>,
    additional_info: Extra,
}

#[derive(Serialize)]
struct Extra {
    duration_ms: u64,
    media_player: &'static str,
    submission_client: &'static str,
}
