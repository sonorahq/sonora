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

const WORKER: &str = "https://billowing-resonance-da83.johnwatson.workers.dev/hashes";
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const FILE: &str = "pathfinder.json";

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
        Some(stale) => format!("{WORKER}?stale={stale}"),
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
    parsed(&body)
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

#[cfg(test)]
mod tests {
    use super::*;

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
