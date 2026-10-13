use anyhow::{Context as _, Result, bail};
use bytes::Bytes;
use http::{Method, Request, header};
use librespot_core::{Session, spclient::CLIENT_TOKEN};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::spotify::pace;

mod album;
mod artist;
mod browse;
mod hashes;
mod library;
mod plays;
mod search;

pub(crate) use album::album;
pub(crate) use artist::{Overview, artist};
pub(crate) use browse::{all as genres, home, page as genre};
pub(crate) use library::{library, set_pinned};
pub(crate) use plays::track;
pub(crate) use search::{albums as search_albums, playlists as search_playlists};

const ENDPOINT: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const APP_PLATFORM: &str = "WebPlayer";
const APP_VERSION: &str = "896000000";

#[derive(Deserialize)]
struct Response<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

async fn query<T: DeserializeOwned>(
    session: &Session,
    operation: &str,
    variables: serde_json::Value,
) -> Result<T> {
    let hash = hashes::resolve(session, operation).await?;
    let rejected = match send(session, operation, &hash.value, &variables).await {
        Ok(data) => return Ok(data),
        Err(error) => error,
    };
    if hash.tried {
        return Err(rejected);
    }
    let Some(latest) = hashes::refetch(session, operation, &hash.value).await else {
        return Err(rejected);
    };
    if latest == hash.value {
        return Err(rejected);
    }
    send(session, operation, &latest, &variables).await
}

async fn send<T: DeserializeOwned>(
    session: &Session,
    operation: &str,
    hash: &str,
    variables: &serde_json::Value,
) -> Result<T> {
    let body = serde_json::to_vec(&serde_json::json!({
        "operationName": operation,
        "variables": variables,
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": hash,
            }
        },
    }))
    .with_context(|| format!("cannot encode {operation} Pathfinder request"))?;
    let token = session
        .login5()
        .auth_token()
        .await
        .context("cannot obtain Spotify access token")?;
    let client_token = session
        .spclient()
        .client_token()
        .await
        .context("cannot obtain Spotify client token")?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(ENDPOINT)
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .header("app-platform", APP_PLATFORM)
        .header("spotify-app-version", APP_VERSION)
        .header(
            header::AUTHORIZATION,
            format!("{} {}", token.token_type, token.access_token),
        )
        .header(CLIENT_TOKEN, client_token)
        .body(Bytes::from(body))
        .with_context(|| format!("cannot build {operation} Pathfinder request"))?;
    let body = pace::send(session, request)
        .await
        .with_context(|| format!("cannot request {operation} from Pathfinder"))?;
    decoded(&body, operation)
}

fn decoded<T: DeserializeOwned>(bytes: &[u8], operation: &str) -> Result<T> {
    let response: Response<T> = serde_json::from_slice(bytes)
        .with_context(|| format!("cannot decode {operation} Pathfinder response"))?;
    if !response.errors.is_empty() {
        let messages = response
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        bail!("Spotify rejected {operation} Pathfinder query: {messages}");
    }
    response
        .data
        .with_context(|| format!("{operation} Pathfinder response has no data"))
}

fn reported(count: u64) -> Option<u64> {
    (count > 0).then_some(count)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::decoded;

    #[test]
    fn decodes_data() {
        let data: Value = decoded(br#"{"data":{"value":42}}"#, "test").unwrap();
        assert_eq!(data["value"], 42);
    }

    #[test]
    fn reports_graphql_error() {
        let error = decoded::<Value>(
            br#"{"data":null,"errors":[{"message":"bad hash"}]}"#,
            "test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("bad hash"));
    }
}
