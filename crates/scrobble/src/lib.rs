//! Scrobbling to Last.fm, Libre.fm, ListenBrainz, Maloja and teal.fm: linking an account and
//! sending what played, under the rules every one of them shares.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use music::credentials;
use serde::{Deserialize, Serialize};

mod audioscrobbler;
mod listenbrainz;
mod maloja;
mod tealfm;

/// A track shorter than this never counts, and a play this long always does. Last.fm wrote both
/// rules and ListenBrainz, Maloja and teal.fm repeat them, so they belong to the play rather than
/// to any one service.
const MIN_LENGTH: Duration = Duration::from_secs(30);
const FULL_PLAY: Duration = Duration::from_secs(240);

/// Every scrobbling service the app offers, in the order the settings screen lists them.
pub fn services() -> Vec<Arc<dyn Service>> {
    vec![
        Arc::new(audioscrobbler::LASTFM),
        Arc::new(audioscrobbler::LIBREFM),
        Arc::new(listenbrainz::ListenBrainz),
        Arc::new(maloja::Maloja),
        Arc::new(tealfm::TealFm::default()),
    ]
}

/// The service refused the API key itself rather than the request it signed. Last.fm answers
/// this way for an invalid or suspended key, which since 1 December 2026 includes every key whose
/// owner has no Last.fm Pro.
#[derive(Debug)]
pub struct KeyRefused {
    code: u32,
    message: String,
}

impl KeyRefused {
    /// Whether `error`, or the error it wraps under its context, is a refused key.
    pub fn in_error(error: &anyhow::Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}

impl std::fmt::Display for KeyRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the scrobbling service refused the api key ({}): {}",
            self.code, self.message
        )
    }
}

impl std::error::Error for KeyRefused {}

/// One listen: what played and when it started.
#[derive(Clone, Debug)]
pub struct Play {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub duration: Duration,
    pub at: SystemTime,
    /// Counts up with each new play since launch, and a resumed play keeps its number. It orders
    /// plays where `at` cannot, since the wall clock may be set back.
    pub order: u64,
}

impl Play {
    /// Whether `played` has earned the listen a submission. A track under thirty seconds never
    /// earns one, however long it ran.
    pub fn earned(&self, played: Duration) -> bool {
        self.duration >= MIN_LENGTH
            && (played >= FULL_PLAY || played.as_secs_f64() * 2. >= self.duration.as_secs_f64())
    }

    /// The start of the play in seconds since the epoch, which is how every service spells it.
    pub fn timestamp(&self) -> i64 {
        let seconds = self
            .at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        i64::try_from(seconds).unwrap_or(i64::MAX)
    }

    /// The album, dropped when the provider left it empty.
    fn release(&self) -> Option<&str> {
        self.album.as_deref().filter(|album| !album.is_empty())
    }
}

/// How a service is linked, which is also what the settings row draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// Nothing to type. The app opens the browser and catches the approval. Libre.fm, which
    /// issues no API accounts of its own.
    Browser,
    /// An API key and secret the user created, then the same browser approval. Last.fm.
    Keys,
    /// A token copied from the service's settings page, and an optional server address for a
    /// self-hosted instance or a stand-in that speaks the same API. ListenBrainz.
    Token,
    /// A server url and one of that server's API keys. Maloja.
    Server,
    /// An ATProto handle or DID and an app password for that account. teal.fm.
    Password,
}

/// What the settings screen collected for a `Link`. A service refuses a variant it did not ask
/// for.
#[derive(Clone, Debug)]
pub enum Secret {
    None,
    Keys {
        key: String,
        secret: String,
    },
    Token {
        token: String,
        server: String,
    },
    Server {
        url: String,
        key: String,
    },
    Password {
        identifier: String,
        password: String,
    },
}

/// A linked scrobbling account, both as `Service::connect` returns it and as [`store`] keeps it.
/// `session` is whatever authenticates a later submission, so an empty one means the account is
/// not linked. Whether listens go to it is a preference that `settings.json` keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// The API key, for the services that issue one, or the DID of an ATProto account.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// The shared secret that signs requests. Only the audioscrobbler protocol uses it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secret: String,
    /// The session key, user token, server key or app password that authenticates a submission.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session: String,
    /// The server this account lives on, for the services the user hosts themselves, or the PDS
    /// of an ATProto account. Empty means the service's own public instance.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub server: String,
    /// What the settings row calls the account.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Account {
    /// Whether the account carries enough to submit with.
    pub fn linked(&self) -> bool {
        !self.session.is_empty()
    }
}

/// A scrobbling service: how it is linked, and how a linked account submits listens.
#[async_trait]
pub trait Service: Send + Sync {
    /// The slug that keys the account in `settings.json` and names the i18n keys of its row.
    fn id(&self) -> &'static str;

    /// What the settings row has to collect before `connect` can run.
    fn link(&self) -> Link;

    /// Where the user goes to create the credentials `link` asks for.
    fn signup(&self) -> Option<&'static str> {
        None
    }

    /// Turns collected input into a stored account. A `Browser` or `Keys` service opens the
    /// browser itself, once its callback listener is up, and resolves when the user approves or
    /// the wait runs out.
    async fn connect(&self, secret: Secret) -> Result<Account>;

    /// Whether `played` has earned the listen a submission to this service. Most services follow
    /// [`Play::earned`], and one with a stricter rule of its own narrows it.
    fn earned(&self, play: &Play, played: Duration) -> bool {
        play.earned(played)
    }

    /// Tells the service what is playing right now. A service with no such concept keeps the
    /// default and makes no request.
    async fn now_playing(&self, account: &Account, play: &Play) -> Result<()> {
        let _ = (account, play);
        Ok(())
    }

    /// Submits finished listens. Every service takes a batch, so one listen is a slice of one.
    /// The account is one `connect` returned, since a row only submits while its account is
    /// linked and switched on.
    async fn scrobble(&self, account: &Account, plays: &[Play]) -> Result<()>;
}

/// The account linked to a service, blank when it was never linked or its file cannot be read.
pub fn load(service: &str) -> Account {
    let path = file(service);
    let body = match std::fs::read(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Account::default(),
        Err(error) => {
            log::warn!("scrobble: cannot read {}: {error}", path.display());
            return Account::default();
        }
    };
    serde_json::from_slice(&body).unwrap_or_else(|error| {
        log::warn!("scrobble: cannot parse {}: {error}", path.display());
        Account::default()
    })
}

/// Keeps a linked account in a file only its owner can read, beside the providers' sign-ins.
pub fn store(service: &str, account: &Account) -> Result<()> {
    let body = serde_json::to_vec(account).context("cannot encode the scrobbling account")?;
    credentials::write(&file(service), &body)
        .with_context(|| format!("cannot store the {service} scrobbling account"))
}

/// Deletes a service's stored account.
pub fn forget(service: &str) {
    credentials::remove(&file(service));
}

/// Where a service's account lives, `accounts/scrobbling/<service>/credentials.json`.
fn file(service: &str) -> PathBuf {
    credentials::dir("scrobbling")
        .join(service)
        .join(credentials::FILE)
}

/// The one http client every service shares, so they pool connections and reuse TLS sessions
/// between them rather than each holding its own.
fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(reqwest::Client::new)
}

/// The server url without its trailing slash, with `https://` added when the user left the
/// scheme out. An empty url stays empty.
fn root(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    match url.is_empty() || url.contains("://") {
        true => url.to_owned(),
        false => format!("https://{url}"),
    }
}

/// The host and port of a server url, which names an account when the server reports no name.
fn host(url: &str) -> String {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_owned()
}
