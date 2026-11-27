//! Spotify profile rankings and discovery of the web player's persisted query.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use reqwest::header;
use serde::{Deserialize, de::DeserializeOwned};
use tokio::sync::Mutex;

use crate::{Session, http};

const TOP_CONTENT: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const DESKTOP_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The current web player's persisted query hash for personal rankings.
static HASH: Mutex<Option<(Instant, String)>> = Mutex::const_new(None);

impl Session {
    /// Reads personal rankings, refreshing a stale web-player hash once after a failed request.
    pub async fn top_rankings<T: DeserializeOwned>(&self, range: &str) -> Result<T> {
        let hash = resolve(None).await?;
        match self.send_rankings(&hash, range).await {
            Ok(data) => Ok(data),
            Err(error) => match resolve(Some(&hash)).await {
                Ok(updated) if updated != hash => self.send_rankings(&updated, range).await,
                _ => Err(error),
            },
        }
    }

    /// Reads the account's personal rankings for one Spotify time range.
    async fn send_rankings<T: DeserializeOwned>(&self, hash: &str, range: &str) -> Result<T> {
        let input = serde_json::json!({
            "offset": 0, "limit": 50, "sortBy": "AFFINITY", "timeRange": range,
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "operationName": "userTopContent",
            "variables": {
                "includeTopArtists": true, "topArtistsInput": input,
                "includeTopTracks": true, "topTracksInput": input,
            },
            "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}},
        }))?;
        let headers = header::HeaderMap::from_iter([
            (header::ACCEPT, "application/json".parse()?),
            (header::CONTENT_TYPE, "application/json".parse()?),
            (
                header::HeaderName::from_static("app-platform"),
                "WebPlayer".parse()?,
            ),
            (
                header::HeaderName::from_static("spotify-app-version"),
                "896000000".parse()?,
            ),
        ]);
        let body = http::post_service(self, TOP_CONTENT, headers, body.into()).await?;
        #[derive(Deserialize)]
        struct Response<T> {
            data: Option<T>,
            #[serde(default)]
            errors: Vec<serde_json::Value>,
        }
        let response: Response<T> = serde_json::from_slice(&body)?;
        ensure!(
            response.errors.is_empty(),
            "Spotify rejected personal rankings: {:?}",
            response.errors
        );
        response
            .data
            .context("Spotify personal rankings have no data")
    }
}

/// Discovers the query only once when several ranking windows are requested together.
async fn resolve(rejected: Option<&str>) -> Result<String> {
    let mut cached = HASH.lock().await;
    if let Some((at, hash)) = cached.as_ref()
        && at.elapsed() < MAX_AGE
        && rejected != Some(hash.as_str())
    {
        return Ok(hash.clone());
    }
    match discover().await {
        Ok(hash) => {
            *cached = Some((Instant::now(), hash.clone()));
            Ok(hash)
        }
        Err(error) => cached.as_ref().map(|(_, hash)| hash.clone()).ok_or(error),
    }
}

/// Reads the personal-ranking operation from the public desktop web-player profile chunk.
async fn discover() -> Result<String> {
    let html = public_text("https://open.spotify.com").await?;
    let bundle = desktop_bundle(&html).context("Spotify page has no desktop player bundle")?;
    let javascript = public_text(bundle).await?;
    let chunk = profile_bundle(&javascript).context("Spotify bundle has no profile chunk")?;
    let url = format!("https://open.spotifycdn.com/cdn/build/web-player/{chunk}");
    let profile = public_text(&url).await?;
    operation_hash(&profile, "userTopContent", "query")
        .map(str::to_owned)
        .context("Spotify profile bundle has no userTopContent operation")
}

fn sane(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}
async fn public_text(url: &str) -> Result<String> {
    use std::io::Read as _;
    let body = http::request(|client| {
        client
            .get(url)
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::USER_AGENT, DESKTOP_USER_AGENT)
    })
    .await?
    .error_for_status()?
    .bytes()
    .await?;
    let bytes = if body.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(body.as_ref()).read_to_end(&mut decoded)?;
        decoded
    } else {
        body.to_vec()
    };
    String::from_utf8(bytes).context("Spotify bundle is not UTF-8")
}

fn desktop_bundle(html: &str) -> Option<&str> {
    html.split('"').find(|url| {
        url.starts_with("https://open.spotifycdn.com/cdn/build/web-player/web-player.")
            && url.ends_with(".js")
    })
}

/// Resolves the profile chunk from the client's own JavaScript filename map.
fn profile_bundle(javascript: &str) -> Option<String> {
    let name = "xpui-routes-profile";
    let before = javascript.split_once(&format!(":\"{name}\""))?.0;
    let id = before.rsplit([',', '{']).next()?;
    // An entry of the map follows a comma or the opening brace, so a longer id that merely ends
    // in these digits is not taken for it.
    let hash = [',', '{'].into_iter().find_map(|before| {
        let marker = format!("{before}{id}:\"");
        javascript
            .split(&marker)
            .skip(1)
            .filter_map(|tail| tail.split_once('"').map(|p| p.0))
            .find(|value| value.len() == 8 && value.bytes().all(|b| b.is_ascii_hexdigit()))
    })?;
    Some(format!("{name}.{hash}.js"))
}

fn operation_hash<'a>(javascript: &'a str, operation: &str, kind: &str) -> Option<&'a str> {
    let marker = format!("\"{operation}\",\"{kind}\",\"");
    let (_, after) = javascript.split_once(&marker)?;
    let (hash, _) = after.split_once('"')?;
    sane(hash).then_some(hash)
}
