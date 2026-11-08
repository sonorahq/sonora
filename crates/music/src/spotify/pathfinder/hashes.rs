use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use bytes::Bytes;
use http::{Method, Request, header};
use librespot_core::Session;
use serde::{Deserialize, Serialize};

use crate::escape;

const WORKER: &str = "https://billowing-resonance-da83.johnwatson.workers.dev/hashes";
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const FILE: &str = "pathfinder.json";

/// Operations the shared hash service does not list, read from the web player bundle instead.
const DESKTOP_OPERATIONS: [(&str, &str); 5] = [
    ("home", "query"),
    ("userTopContent", "query"),
    ("libraryV3", "query"),
    ("pinLibraryItem", "mutation"),
    ("unpinLibraryItem", "mutation"),
];

pub(super) struct Hash {
    pub(super) value: String,
    pub(super) tried: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct Registry {
    fetched: u64,
    operations: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Answer {
    operations: HashMap<String, String>,
}

pub(super) async fn resolve(session: &Session, operation: &str) -> Result<Hash> {
    let cached = registry();
    if let Some(value) = cached
        .as_ref()
        .filter(|registry| aged(registry) < MAX_AGE)
        .and_then(|registry| registry.operations.get(operation).cloned())
    {
        return Ok(Hash {
            value,
            tried: false,
        });
    }
    if DESKTOP_OPERATIONS
        .iter()
        .any(|(name, _)| *name == operation)
    {
        return match desktop_hash(operation).await {
            Ok(value) => Ok(Hash {
                value,
                tried: false,
            }),
            Err(error) => cached
                .and_then(|mut registry| registry.operations.remove(operation))
                .map(|value| Hash { value, tried: true })
                .ok_or(error),
        };
    }
    let latest = match fetched(session).await {
        Ok(operations) => operations,
        Err(error) => {
            return cached
                .and_then(|mut registry| registry.operations.remove(operation))
                .map(|value| Hash { value, tried: true })
                .ok_or(error);
        }
    };
    latest
        .get(operation)
        .cloned()
        .map(|value| Hash { value, tried: true })
        .with_context(|| format!("the hash registry has no {operation} query"))
}

pub(super) async fn refetch(session: &Session, operation: &str, stale: &str) -> Option<String> {
    if DESKTOP_OPERATIONS
        .iter()
        .any(|(name, _)| *name == operation)
    {
        return desktop_hash(operation).await.ok();
    }
    refreshed(session, Some(stale))
        .await
        .ok()?
        .get(operation)
        .cloned()
        .or_else(|| {
            log::warn!("pathfinder: the hash registry has no {operation} query");
            None
        })
}

async fn fetched(session: &Session) -> Result<HashMap<String, String>> {
    refreshed(session, None).await
}

async fn refreshed(session: &Session, stale: Option<&str>) -> Result<HashMap<String, String>> {
    let operations = match download(session, stale).await {
        Ok(operations) => operations,
        Err(error) => {
            log::warn!("pathfinder: cannot refresh the query hashes: {error:#}");
            return Err(error);
        }
    };
    store(&operations);
    Ok(operations)
}

async fn download(session: &Session, stale: Option<&str>) -> Result<HashMap<String, String>> {
    let uri = match stale {
        Some(stale) => format!("{WORKER}?stale={}", escape::component(stale)),
        None => WORKER.to_owned(),
    };
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::ACCEPT, "application/json")
        .body(Bytes::new())
        .context("cannot build the query hash request")?;
    let body = session
        .http_client()
        .request_body(request)
        .await
        .context("cannot request the query hashes")?;
    let mut operations = parsed(&body)?;
    if let Some(cached) = registry() {
        for (operation, _) in DESKTOP_OPERATIONS {
            if let Some(hash) = cached.operations.get(operation) {
                operations
                    .entry(operation.to_owned())
                    .or_insert_with(|| hash.clone());
            }
        }
    }
    Ok(operations)
}

fn parsed(body: &[u8]) -> Result<HashMap<String, String>> {
    let answer: Answer =
        serde_json::from_slice(body).context("cannot decode the query hash registry")?;
    if answer.operations.is_empty() {
        bail!("the query hash registry is empty");
    }
    if let Some((operation, _)) = answer
        .operations
        .iter()
        .find(|(_, hash)| !sane(hash.as_str()))
    {
        bail!("the {operation} query hash is malformed");
    }
    Ok(answer.operations)
}

fn sane(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn aged(registry: &Registry) -> Duration {
    Duration::from_secs(now().saturating_sub(registry.fetched))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn registry() -> Option<Registry> {
    cache().lock().ok()?.clone()
}

fn store(operations: &HashMap<String, String>) {
    let registry = Registry {
        fetched: now(),
        operations: operations.clone(),
    };
    write(&registry);
    if let Ok(mut cache) = cache().lock() {
        *cache = Some(registry);
    }
}

fn cache() -> &'static Mutex<Option<Registry>> {
    static CACHE: OnceLock<Mutex<Option<Registry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(read()))
}

fn read() -> Option<Registry> {
    let body = std::fs::read(path()).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write(registry: &Registry) {
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let written = serde_json::to_vec_pretty(registry)
        .context("cannot encode")
        .and_then(|body| std::fs::write(&path, body).context("cannot save"));
    if let Err(error) = written {
        log::warn!("pathfinder: {error:#} {}", path.display());
    }
}

fn path() -> PathBuf {
    crate::credentials::root().join(FILE)
}

// Discover the operations missing from the shared hash service.
async fn desktop_hash(operation: &str) -> Result<String> {
    // librespot overwrites User-Agent on every request, which makes this page serve
    // the mobile bundle. This unauthenticated client reads only public web assets.
    let public = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(20)).build()?;
    let html = public_text(&public, "https://open.spotify.com").await?;
    let bundle = desktop_bundle(&html).context("Spotify page has no desktop player bundle")?;
    let javascript = public_text(&public, bundle).await?;
    let javascript = match operation {
        "userTopContent" => {
            let chunk =
                profile_bundle(&javascript).context("Spotify bundle has no profile chunk")?;
            let url = format!("https://open.spotifycdn.com/cdn/build/web-player/{chunk}");
            public_text(&public, &url).await?
        }
        _ => javascript,
    };
    let mut operations = registry()
        .map(|registry| registry.operations)
        .unwrap_or_default();
    let mut requested = None;
    for (name, kind) in DESKTOP_OPERATIONS {
        if let Some(hash) = operation_hash(&javascript, name, kind) {
            operations.insert(name.to_owned(), hash.to_owned());
            if name == operation {
                requested = Some(hash.to_owned());
            }
        }
    }
    let requested =
        requested.with_context(|| format!("Spotify bundle has no {operation} operation"))?;
    store(&operations);
    Ok(requested)
}

async fn public_text(client: &reqwest::Client, url: &str) -> Result<String> {
    use std::io::Read as _;
    let body = client
        .get(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
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
    let marker = format!("{id}:\"");
    let hash = javascript
        .split(&marker)
        .skip(1)
        .filter_map(|tail| tail.split_once('"').map(|p| p.0))
        .find(|value| value.len() == 8 && value.bytes().all(|b| b.is_ascii_hexdigit()))?;
    Some(format!("{name}.{hash}.js"))
}

fn operation_hash<'a>(javascript: &'a str, operation: &str, kind: &str) -> Option<&'a str> {
    let marker = format!("\"{operation}\",\"{kind}\",\"");
    let (_, after) = javascript.split_once(&marker)?;
    let (hash, _) = after.split_once('"')?;
    sane(hash).then_some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_only_the_desktop_bundle_and_a_valid_query_hash() {
        let url = "https://open.spotifycdn.com/cdn/build/web-player/web-player.abc.js";
        assert_eq!(
            desktop_bundle(&format!(r#"<script src="{url}"></script>"#)),
            Some(url)
        );
        assert!(
            desktop_bundle(r#"<script src="https://example.com/web-player.js"></script>"#)
                .is_none()
        );
        let hash = "0123456789abcdef".repeat(4);
        assert_eq!(
            operation_hash(
                &format!(r#"new Query("libraryV3","query","{hash}",null)"#),
                "libraryV3",
                "query"
            ),
            Some(hash.as_str())
        );
        assert!(
            operation_hash(
                r#"new Query("libraryV3","query","broken",null)"#,
                "libraryV3",
                "query"
            )
            .is_none()
        );
    }

    #[test]
    fn discovers_each_library_mutation_without_confusing_pin_and_unpin() {
        let pin = "a".repeat(64);
        let unpin = "b".repeat(64);
        let javascript = format!(
            r#"new Query("unpinLibraryItem","mutation","{unpin}",null);new Query("pinLibraryItem","mutation","{pin}",null)"#
        );
        assert_eq!(
            operation_hash(&javascript, "pinLibraryItem", "mutation"),
            Some(pin.as_str())
        );
        assert_eq!(
            operation_hash(&javascript, "unpinLibraryItem", "mutation"),
            Some(unpin.as_str())
        );
        assert!(operation_hash(&javascript, "pinLibraryItem", "query").is_none());
    }

    fn payload(hash: &str) -> Vec<u8> {
        format!(
            r#"{{"version":1,"updated_at":"2026-08-09T00:00:00.000Z",
               "bundle":"web-player.abc.js","operations":{{"getAlbum":"{hash}"}}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn keeps_the_operations_of_a_registry() {
        let hash = "0123456789abcdef".repeat(4);
        let operations = parsed(&payload(&hash)).unwrap();
        assert_eq!(operations["getAlbum"], hash);
    }

    #[test]
    fn rejects_a_malformed_hash() {
        assert!(parsed(&payload("")).is_err());
        assert!(parsed(&payload(&"z".repeat(64))).is_err());
    }

    #[test]
    fn rejects_an_empty_registry() {
        assert!(parsed(br#"{"version":1,"operations":{}}"#).is_err());
    }
}
