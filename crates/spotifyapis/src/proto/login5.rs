//! Login5, which trades a reusable credential for an access token. Sonora's librespot builds
//! saved that kind of credential, so a sign-in they left keeps working without the browser.

use std::time::Duration;

use anyhow::{Context as _, Result};
use prost::Message as _;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use ring::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};

use crate::auth::AuthError;
use crate::http;

const ENDPOINT: &str = "https://login5.spotify.com/v3/login";
const TIMEOUT: Duration = Duration::from_secs(20);
/// How many challenge rounds one login answers before it gives up.
const ROUNDS: usize = 3;
/// The most zero bits a proof-of-work challenge may ask for. Spotify asks for about ten, and
/// each bit more doubles the work, so a larger ask is refused rather than solved.
const HARDEST: u32 = 24;
/// The most challenges one login answers. Spotify sets one.
const MOST_CHALLENGES: usize = 4;
/// `LoginError` values that mean the credential itself is no longer accepted.
const INVALID_CREDENTIALS: i32 = 1;
const UNKNOWN_IDENTIFIER: i32 = 5;

#[derive(prost::Message)]
struct LoginRequest {
    #[prost(message, optional, tag = "1")]
    client_info: Option<ClientInfo>,
    #[prost(bytes = "vec", tag = "2")]
    login_context: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    challenge_solutions: Option<ChallengeSolutions>,
    #[prost(message, optional, tag = "100")]
    stored_credential: Option<StoredCredential>,
}

#[derive(prost::Message)]
struct ClientInfo {
    #[prost(string, tag = "1")]
    client_id: String,
    #[prost(string, tag = "2")]
    device_id: String,
}

#[derive(prost::Message)]
struct StoredCredential {
    #[prost(string, tag = "1")]
    username: String,
    #[prost(bytes = "vec", tag = "2")]
    data: Vec<u8>,
}

#[derive(prost::Message)]
struct ChallengeSolutions {
    #[prost(message, repeated, tag = "1")]
    solutions: Vec<ChallengeSolution>,
}

#[derive(prost::Message)]
struct ChallengeSolution {
    #[prost(message, optional, tag = "1")]
    hashcash: Option<HashcashSolution>,
}

#[derive(prost::Message)]
struct HashcashSolution {
    #[prost(bytes = "vec", tag = "1")]
    suffix: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    duration: Option<prost_types::Duration>,
}

#[derive(prost::Message)]
struct LoginResponse {
    #[prost(message, optional, tag = "1")]
    ok: Option<LoginOk>,
    #[prost(int32, tag = "2")]
    error: i32,
    #[prost(message, optional, tag = "3")]
    challenges: Option<Challenges>,
    #[prost(bytes = "vec", tag = "5")]
    login_context: Vec<u8>,
}

#[derive(prost::Message)]
struct LoginOk {
    #[prost(string, tag = "1")]
    username: String,
    #[prost(string, tag = "2")]
    access_token: String,
    #[prost(bytes = "vec", tag = "3")]
    stored_credential: Vec<u8>,
    #[prost(int32, tag = "4")]
    access_token_expires_in: i32,
}

#[derive(prost::Message)]
struct Challenges {
    #[prost(message, repeated, tag = "1")]
    challenges: Vec<Challenge>,
}

#[derive(prost::Message)]
struct Challenge {
    #[prost(message, optional, tag = "1")]
    hashcash: Option<HashcashChallenge>,
}

#[derive(prost::Message)]
struct HashcashChallenge {
    #[prost(bytes = "vec", tag = "1")]
    prefix: Vec<u8>,
    #[prost(int32, tag = "2")]
    length: i32,
}

/// What a login hands back: the bearer, how long it lasts, and the credential to use next time,
/// which is empty when Spotify keeps the old one.
pub(crate) struct Granted {
    pub(crate) access_token: String,
    pub(crate) lifetime: Duration,
    pub(crate) credential: Vec<u8>,
}

/// Logs `username` in with a reusable `credential`, answering the proof-of-work challenges
/// Spotify may set. A credential Spotify no longer accepts fails with [`AuthError::Credentials`].
pub(crate) async fn login(
    client_id: &str,
    device_id: &str,
    username: &str,
    credential: &[u8],
) -> Result<Granted> {
    let mut request = LoginRequest {
        client_info: Some(ClientInfo {
            client_id: client_id.to_owned(),
            device_id: device_id.to_owned(),
        }),
        stored_credential: Some(StoredCredential {
            username: username.to_owned(),
            data: credential.to_vec(),
        }),
        ..Default::default()
    };
    for _ in 0..ROUNDS {
        let body = request.encode_to_vec();
        let response = http::request(|client| {
            client
                .post(ENDPOINT)
                .timeout(TIMEOUT)
                .header(CONTENT_TYPE, http::PROTOBUF)
                .header(ACCEPT, http::PROTOBUF)
                .body(body)
        })
        .await
        .map_err(|error| error.without_url())
        .context(AuthError::Network)?;
        anyhow::ensure!(response.status().is_success(), AuthError::Refused);
        let bytes = response.bytes().await.context(AuthError::Network)?;
        let reply = LoginResponse::decode(bytes).context("cannot read Spotify's login answer")?;
        if let Some(ok) = reply.ok {
            anyhow::ensure!(
                !ok.access_token.is_empty() && (ok.username.is_empty() || ok.username == username),
                AuthError::Credentials
            );
            let lifetime = u64::try_from(ok.access_token_expires_in).unwrap_or_default();
            return Ok(Granted {
                access_token: ok.access_token,
                lifetime: Duration::from_secs(lifetime),
                credential: ok.stored_credential,
            });
        }
        if let Some(challenges) = reply.challenges {
            let context = reply.login_context.clone();
            let solutions = tokio::task::spawn_blocking(move || solve(&context, &challenges))
                .await
                .context("the login challenge stopped")??;
            request.login_context = reply.login_context;
            request.challenge_solutions = Some(solutions);
            continue;
        }
        return Err(match reply.error {
            INVALID_CREDENTIALS | UNKNOWN_IDENTIFIER => AuthError::Credentials,
            _ => AuthError::Refused,
        }
        .into());
    }
    Err(AuthError::Refused.into())
}

/// Answers every proof-of-work challenge of a login, refusing one that asks for more work than
/// [`HARDEST`] allows.
fn solve(context: &[u8], challenges: &Challenges) -> Result<ChallengeSolutions> {
    anyhow::ensure!(
        challenges.challenges.len() <= MOST_CHALLENGES,
        AuthError::Refused
    );
    let solutions = challenges
        .challenges
        .iter()
        .map(|challenge| {
            let hashcash = match &challenge.hashcash {
                Some(hashcash) => Some(HashcashSolution {
                    suffix: hashcash_suffix(context, hashcash)?,
                    duration: None,
                }),
                None => None,
            };
            Ok(ChallengeSolution { hashcash })
        })
        .collect::<Result<_>>()?;
    Ok(ChallengeSolutions { solutions })
}

/// Finds the suffix whose SHA-1 after the challenge's prefix ends in as many zero bits as it
/// asks for, counting up from a start the login context fixes, as Spotify's own clients do.
fn hashcash_suffix(context: &[u8], challenge: &HashcashChallenge) -> Result<Vec<u8>> {
    let wanted = u32::try_from(challenge.length)
        .ok()
        .filter(|wanted| *wanted <= HARDEST)
        .ok_or(AuthError::Refused)?;
    let start = tail(digest(&SHA1_FOR_LEGACY_USE_ONLY, context).as_ref());
    // a suffix turns up after about 2^wanted tries, so 256 times that is never reached in practice
    for counter in 0..1_i64 << (wanted + 8) {
        let mut suffix = start.wrapping_add(counter).to_be_bytes().to_vec();
        suffix.extend_from_slice(&counter.to_be_bytes());
        let mut input = challenge.prefix.clone();
        input.extend_from_slice(&suffix);
        if tail(digest(&SHA1_FOR_LEGACY_USE_ONLY, &input).as_ref()).trailing_zeros() >= wanted {
            return Ok(suffix);
        }
    }
    Err(AuthError::Refused.into())
}

/// Bytes 12 to 20 of a SHA-1 digest as a big-endian number.
fn tail(hash: &[u8]) -> i64 {
    hash.get(12..20)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map_or(0, i64::from_be_bytes)
}
