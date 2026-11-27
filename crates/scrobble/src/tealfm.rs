use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use super::{Account, Link, Play, Rejected, Secret, Service};

/// The record each listen becomes in the user's repository.
const PLAY: &str = "fm.teal.feed.play";
/// The single record, keyed `self`, that teal.fm reads the current track from.
const STATUS: &str = "fm.teal.actor.status";
const STATUS_KEY: &str = "self";
/// How long a now-playing status counts when the track has no length, the lexicon's default.
const STATUS_FALLBACK: Duration = Duration::from_secs(600);
/// The longest string, in bytes, the play lexicon accepts for a name.
const NAME_LIMIT: usize = 256;
/// The alphabet of the sortable base32 that record keys of the TID form are written in.
const TID_ALPHABET: &[u8; 32] = b"234567abcdefghijklmnopqrstuvwxyz";
const APP_PASSWORDS: &str = "https://bsky.app/settings/app-passwords";
/// The directory that resolves `did:plc` identities to their documents.
const PLC: &str = "https://plc.directory";
/// The Bluesky AppView, which resolves a handle through DNS when its own host serves no
/// `.well-known/atproto-did`.
const RESOLVER: &str = "https://public.api.bsky.app";
/// The client half of `submissionClientAgent`, in the reverse DNS form the lexicon recommends.
const CLIENT: &str = concat!("org.sonorahq.sonora/", env!("CARGO_PKG_VERSION"));
/// How long a handle's own host gets to answer before resolution falls back to the AppView.
const WELL_KNOWN_WAIT: Duration = Duration::from_secs(5);
/// How long any other teal.fm request may take, so a stalled PDS cannot hold a link or a write.
const REQUEST_WAIT: Duration = Duration::from_secs(30);
/// The lexicon counts a track shorter than this only once it has played to the end.
const WHOLE_BELOW: Duration = Duration::from_secs(120);
/// How far short of the end a short track may stop and still count as played to the end, since
/// the last position report lands a moment before the track does.
const END_SLACK: Duration = Duration::from_secs(3);

/// teal.fm reads listens straight from the user's ATProto repository, so Sonora signs in to the
/// account's PDS with an app password and writes the records itself. The app password is what the
/// account stores. The access and refresh tokens it buys live only in memory and are bought again
/// after a restart.
#[derive(Default)]
pub struct TealFm {
    tokens: Mutex<Option<Tokens>>,
}

impl TealFm {
    /// The cached session for this account, or a fresh one when there is none or it belongs to
    /// another DID.
    async fn tokens(&self, account: &Account) -> Result<Tokens> {
        let cached = self.cached(account);
        match cached {
            Some(tokens) => Ok(tokens),
            None => self.sign_in(account).await,
        }
    }

    fn cached(&self, account: &Account) -> Option<Tokens> {
        let guard = self.tokens.lock().ok()?;
        guard
            .as_ref()
            .filter(|tokens| tokens.did == account.key)
            .cloned()
    }

    fn keep(&self, tokens: &Tokens) {
        if let Ok(mut cached) = self.tokens.lock() {
            *cached = Some(tokens.clone());
        }
    }

    /// Buys a session with the stored app password and caches it.
    async fn sign_in(&self, account: &Account) -> Result<Tokens> {
        let created = create_session(&account.server, &account.key, &account.session).await?;
        let tokens = Tokens::from(created);
        self.keep(&tokens);
        Ok(tokens)
    }

    /// Trades the refresh token for a new session, and signs in again with the app password when
    /// the refresh token is spent or revoked too.
    async fn renew(&self, account: &Account, stale: &Tokens) -> Result<Tokens> {
        let refreshed = async {
            let answer = http()?
                .post(xrpc(&account.server, "com.atproto.server.refreshSession"))
                .bearer_auth(&stale.refresh)
                .send()
                .await
                .context("cannot reach the atproto server")?;
            let answer = accepted(answer, "refreshSession").await?;
            answer
                .json::<Created>()
                .await
                .context("cannot read the refreshed atproto session")
        };
        match refreshed.await {
            Ok(created) => {
                let tokens = Tokens::from(created);
                self.keep(&tokens);
                Ok(tokens)
            }
            Err(error) => {
                log::info!("scrobble: teal.fm session refresh failed, signing in again: {error:#}");
                self.sign_in(account).await
            }
        }
    }

    /// Calls a repository write, renewing the session once when the server says it expired.
    async fn write(&self, account: &Account, method: &str, body: &impl Serialize) -> Result<()> {
        let tokens = self.tokens(account).await?;
        if post(account, method, &tokens.access, body).await? == Sent::Done {
            return Ok(());
        }
        let tokens = self.renew(account, &tokens).await?;
        match post(account, method, &tokens.access, body).await? {
            Sent::Done => Ok(()),
            Sent::Expired => bail!("the atproto server refused a fresh session for {method}"),
        }
    }
}

#[async_trait]
impl Service for TealFm {
    fn id(&self) -> &'static str {
        "tealfm"
    }

    fn link(&self) -> Link {
        Link::Password
    }

    fn signup(&self) -> Option<&'static str> {
        Some(APP_PASSWORDS)
    }

    fn earned(&self, play: &Play, played: Duration) -> bool {
        play.earned(played) && (play.duration >= WHOLE_BELOW || played + END_SLACK >= play.duration)
    }

    async fn connect(&self, secret: Secret) -> Result<Account> {
        let Secret::Password {
            identifier,
            password,
        } = secret
        else {
            bail!("teal.fm needs a handle and an app password");
        };
        let identifier = identifier.trim().trim_start_matches('@');
        let password = password.trim().to_owned();
        if identifier.is_empty() || password.is_empty() {
            bail!("teal.fm needs a handle and an app password");
        }

        let did = resolve(identifier).await?;
        let server = pds(&did).await?;
        let created = create_session(&server, &did, &password).await?;
        if created.did != did {
            bail!(
                "the atproto server signed in {} instead of {did}",
                created.did
            );
        }

        let name = created.handle.clone();
        self.keep(&Tokens::from(created));
        Ok(Account {
            key: did,
            session: password,
            server,
            name,
            ..Account::default()
        })
    }

    async fn now_playing(&self, account: &Account, play: &Play, position: Duration) -> Result<()> {
        let now = SystemTime::now();
        let lasts = match play.duration.is_zero() {
            true => STATUS_FALLBACK,
            false => play.duration.saturating_sub(position),
        };
        let agent = agent();
        let status = Status {
            kind: STATUS,
            time: datetime(now)?,
            expiry: datetime(now + lasts)?,
            item: View::of(play, &agent, None),
        };
        self.write(
            account,
            "com.atproto.repo.putRecord",
            &Put {
                repo: &account.key,
                collection: STATUS,
                rkey: STATUS_KEY,
                record: &status,
            },
        )
        .await
    }

    /// Deletes the status record, which every teal.fm reader takes as nothing playing. Deleting
    /// a record that is already gone succeeds.
    async fn stopped(&self, account: &Account) -> Result<()> {
        self.write(
            account,
            "com.atproto.repo.deleteRecord",
            &Delete {
                repo: &account.key,
                collection: STATUS,
                rkey: STATUS_KEY,
            },
        )
        .await
    }

    async fn scrobble(&self, account: &Account, plays: &[Play]) -> Result<()> {
        let agent = agent();
        for play in plays {
            let record = Record {
                kind: PLAY,
                view: View::of(play, &agent, Some(datetime(play.at)?)),
            };
            // the key comes from the play, so a retry after a lost answer rewrites the same record
            // rather than adding a second listen
            let rkey = tid(play.at);
            self.write(
                account,
                "com.atproto.repo.putRecord",
                &Put {
                    repo: &account.key,
                    collection: PLAY,
                    rkey: &rkey,
                    record: &record,
                },
            )
            .await?;
        }
        Ok(())
    }
}

/// A signed-in session on the account's PDS.
#[derive(Clone)]
struct Tokens {
    did: String,
    access: String,
    refresh: String,
}

impl From<Created> for Tokens {
    fn from(created: Created) -> Self {
        Self {
            did: created.did,
            access: created.access_jwt,
            refresh: created.refresh_jwt,
        }
    }
}

/// What one repository write came to. Any other refusal is an error.
#[derive(PartialEq, Eq)]
enum Sent {
    Done,
    Expired,
}

/// The answer of `createSession` and `refreshSession`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    did: String,
    handle: String,
    access_jwt: String,
    refresh_jwt: String,
}

#[derive(Default, Deserialize)]
struct Refusal {
    #[serde(default)]
    error: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Resolved {
    did: String,
}

#[derive(Deserialize)]
struct Document {
    #[serde(default)]
    service: Vec<Endpoint>,
}

#[derive(Deserialize)]
struct Endpoint {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "serviceEndpoint")]
    endpoint: serde_json::Value,
}

#[derive(Serialize)]
struct Put<'a, T> {
    repo: &'a str,
    collection: &'a str,
    rkey: &'a str,
    record: &'a T,
}

#[derive(Serialize)]
struct Delete<'a> {
    repo: &'a str,
    collection: &'a str,
    rkey: &'a str,
}

#[derive(Serialize)]
struct Record<'a> {
    #[serde(rename = "$type")]
    kind: &'static str,
    #[serde(flatten)]
    view: View<'a>,
}

#[derive(Serialize)]
struct Status<'a> {
    #[serde(rename = "$type")]
    kind: &'static str,
    time: String,
    expiry: String,
    item: View<'a>,
}

/// A track as both lexicons spell it. The play record and the status item share these fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View<'a> {
    track_name: &'a str,
    artists: [Artist<'a>; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    release_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u64>,
    submission_client_agent: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    played_time: Option<String>,
}

impl<'a> View<'a> {
    /// Spells a play, clipping names to what the lexicon accepts. `played` is the start of the
    /// play, which only the finished listen carries.
    fn of(play: &'a Play, agent: &'a str, played: Option<String>) -> Self {
        Self {
            track_name: clip(&play.title),
            artists: [Artist {
                artist_name: clip(&play.artist),
            }],
            release_name: play.release().map(clip),
            duration: play.seconds(),
            submission_client_agent: agent,
            played_time: played,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Artist<'a> {
    artist_name: &'a str,
}

/// Posts one repository write. An expired or rejected access token comes back as
/// [`Sent::Expired`] so the caller can renew it, and any other refusal is an error.
async fn post(
    account: &Account,
    method: &str,
    access: &str,
    body: &impl Serialize,
) -> Result<Sent> {
    let answer = http()?
        .post(xrpc(&account.server, method))
        .bearer_auth(access)
        .json(body)
        .send()
        .await
        .context("cannot reach the atproto server")?;
    let status = answer.status();
    if status.is_success() {
        return Ok(Sent::Done);
    }
    let refusal: Refusal = answer.json().await.unwrap_or_default();
    if status == StatusCode::UNAUTHORIZED
        || matches!(refusal.error.as_str(), "ExpiredToken" | "InvalidToken")
    {
        return Ok(Sent::Expired);
    }
    if status == StatusCode::BAD_REQUEST {
        return Err(Rejected(format!(
            "{method} ({status}): {} {}",
            refusal.error, refusal.message
        ))
        .into());
    }
    bail!(
        "the atproto server refused {method} ({status}): {} {}",
        refusal.error,
        refusal.message
    );
}

/// Signs in to a PDS with an app password. `identifier` is the account's DID, so a handle that
/// has since moved cannot sign in somebody else.
async fn create_session(pds: &str, identifier: &str, password: &str) -> Result<Created> {
    let answer = http()?
        .post(xrpc(pds, "com.atproto.server.createSession"))
        .json(&serde_json::json!({ "identifier": identifier, "password": password }))
        .send()
        .await
        .context("cannot reach the atproto server")?;
    accepted(answer, "createSession")
        .await?
        .json()
        .await
        .context("cannot read the atproto session")
}

/// Passes a successful answer through and turns a refusal into an error naming the method.
async fn accepted(answer: reqwest::Response, method: &str) -> Result<reqwest::Response> {
    let status = answer.status();
    if status.is_success() {
        return Ok(answer);
    }
    let refusal: Refusal = answer.json().await.unwrap_or_default();
    bail!(
        "the atproto server refused {method} ({status}): {} {}",
        refusal.error,
        refusal.message
    );
}

/// The DID behind a handle. A DID typed in is taken as it is. A handle is asked of its own host
/// first, then of the Bluesky AppView, which also follows the `_atproto` DNS record.
async fn resolve(identifier: &str) -> Result<String> {
    if identifier.starts_with("did:") {
        return Ok(identifier.to_owned());
    }
    let handle = identifier.to_lowercase();
    if let Some(did) = well_known(&handle).await {
        return Ok(did);
    }

    let answer = http()?
        .get(xrpc(RESOLVER, "com.atproto.identity.resolveHandle"))
        .query(&[("handle", &handle)])
        .send()
        .await
        .context("cannot reach the bluesky handle resolver")?;
    let answer = accepted(answer, "resolveHandle")
        .await
        .with_context(|| format!("cannot resolve the handle {handle}"))?;
    let resolved: Resolved = answer
        .json()
        .await
        .context("cannot read the resolved handle")?;
    Ok(resolved.did)
}

/// The DID a handle's host serves at `/.well-known/atproto-did`, if it serves one.
async fn well_known(handle: &str) -> Option<String> {
    let answer = http()
        .ok()?
        .get(format!("https://{handle}/.well-known/atproto-did"))
        .timeout(WELL_KNOWN_WAIT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let did = answer.text().await.ok()?.trim().to_owned();
    (did.starts_with("did:") && !did.contains(char::is_whitespace)).then_some(did)
}

/// The PDS a DID document names, which is where the account signs in and its records live.
async fn pds(did: &str) -> Result<String> {
    let url = match (did.strip_prefix("did:plc:"), did.strip_prefix("did:web:")) {
        (Some(_), _) => format!("{PLC}/{did}"),
        (_, Some(host)) => format!("https://{}/.well-known/did.json", host.replace("%3A", ":")),
        _ => bail!("teal.fm cannot use the identity {did}"),
    };
    let answer = http()?
        .get(url)
        .send()
        .await
        .context("cannot reach the atproto identity directory")?;
    let document: Document = accepted(answer, "the did document")
        .await?
        .json()
        .await
        .context("cannot read the did document")?;

    let endpoint = document
        .service
        .iter()
        .find(|service| {
            service.id.ends_with("#atproto_pds") && service.kind == "AtprotoPersonalDataServer"
        })
        .and_then(|service| service.endpoint.as_str())
        .map(|endpoint| endpoint.trim_end_matches('/').to_owned())
        .with_context(|| format!("the did document of {did} names no pds"))?;
    if !endpoint.starts_with("https://") {
        bail!("the pds of {did} does not use https, so its app password stays unsent");
    }
    Ok(endpoint)
}

fn xrpc(server: &str, method: &str) -> String {
    format!("{server}/xrpc/{method}")
}

/// The client every teal.fm request goes through. Unlike the shared scrobbling client it refuses
/// plain http, redirects included, since it carries the app password and the tokens it buys.
fn http() -> Result<&'static reqwest::Client> {
    static HTTP: OnceLock<reqwest::Result<reqwest::Client>> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .https_only(true)
            .timeout(REQUEST_WAIT)
            .build()
    })
    .as_ref()
    .map_err(|error| anyhow!("cannot build the teal.fm http client: {error}"))
}

/// The `submissionClientAgent` string, `org.sonorahq.sonora/<version> (<os>)`.
fn agent() -> String {
    format!("{CLIENT} ({})", std::env::consts::OS)
}

/// A time as the ATProto datetime format spells it, in UTC with milliseconds.
fn datetime(time: SystemTime) -> Result<String> {
    let timestamp = jiff::Timestamp::try_from(time).context("cannot date the play")?;
    Ok(timestamp.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

/// The longest prefix of `text` that fits the lexicon's byte limit without splitting a character.
fn clip(text: &str) -> &str {
    if text.len() <= NAME_LIMIT {
        return text;
    }
    let end = (0..=NAME_LIMIT)
        .rev()
        .find(|&end| text.is_char_boundary(end))
        .unwrap_or(0);
    &text[..end]
}

/// The record key of a play, a TID made from the moment the play started. The microseconds give
/// its time and the nanoseconds within that microsecond its clock id, so the same play always
/// gets the same key.
fn tid(at: SystemTime) -> String {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let micros = u64::try_from(since.as_micros()).unwrap_or(u64::MAX) & ((1 << 53) - 1);
    let mut value = (micros << 10) | u64::from(since.subsec_nanos() % 1000);
    let mut key = [TID_ALPHABET[0]; 13];
    for slot in key.iter_mut().rev() {
        *slot = TID_ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    key.iter().map(|&byte| char::from(byte)).collect()
}
