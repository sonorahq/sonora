use std::fs::{self, FileTimes, OpenOptions};
use std::future::Future;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use gpui::http_client::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, COOKIE, RANGE};
use gpui::http_client::http::{HeaderValue, Method};
use gpui::http_client::{AsyncBody, HttpClient, Inner, Request, Response, Url};
use sha2::{Digest, Sha256};
use tokio::runtime::Handle;

const USER_AGENT: &str = "sonora";
const CACHE_BYTES: u64 = 128 * 1024 * 1024;
const CACHE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const CACHE_SWEEP: Duration = Duration::from_secs(60 * 60);

static TEMP_FILE: AtomicU64 = AtomicU64::new(0);

type Sent = Pin<Box<dyn Future<Output = Result<Response<AsyncBody>>> + Send>>;

pub struct Client {
    inner: reqwest::Client,
    handle: Handle,
    user_agent: HeaderValue,
    cache: Option<Arc<Mutex<DiskCache>>>,
}

impl Client {
    pub fn new(handle: Handle) -> Self {
        let _guard = handle.enter();
        let cache = dirs::cache_dir()
            .map(|root| root.join("sonora").join("images"))
            .and_then(|root| match DiskCache::new(root, CACHE_BYTES, CACHE_AGE) {
                Ok(cache) => Some(Arc::new(Mutex::new(cache))),
                Err(error) => {
                    log::warn!("artwork: cannot initialize the disk cache: {error}");
                    None
                }
            });
        if let Some(cache) = cache.clone() {
            handle.spawn_blocking(move || with_cache(&cache, |cache| cache.sweep()));
        }
        Self {
            inner: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .unwrap_or_default(),
            handle: handle.clone(),
            user_agent: HeaderValue::from_static(USER_AGENT),
            cache,
        }
    }
}

impl HttpClient for Client {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(&self, request: Request<AsyncBody>) -> Sent {
        let client = self.inner.clone();
        let handle = self.handle.clone();
        let cache = self.cache.clone();

        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let body = read(body)?;

            let fetch = handle.spawn(async move {
                let uri = parts.uri.to_string();
                let cacheable = parts.method == Method::GET
                    && !parts.headers.contains_key(AUTHORIZATION)
                    && !parts.headers.contains_key(COOKIE)
                    && !parts.headers.contains_key(RANGE);
                let cached = match (cacheable, cache.as_ref()) {
                    (true, Some(cache)) => {
                        let cache = cache.clone();
                        let uri = uri.clone();
                        tokio::task::spawn_blocking(move || {
                            with_cache(&cache, |cache| cache.get(&uri)).flatten()
                        })
                        .await
                        .unwrap_or_else(|error| {
                            log::warn!("artwork: cannot read the disk cache: {error}");
                            None
                        })
                    }
                    _ => None,
                };
                if let Some(bytes) = cached {
                    return Ok::<_, anyhow::Error>((reqwest::StatusCode::OK, bytes.into()));
                }

                let mut outgoing = client.request(parts.method, &uri).headers(parts.headers);

                if let Some(body) = body {
                    outgoing = outgoing.body(body);
                }

                let incoming = outgoing.send().await.map_err(reqwest::Error::without_url)?;
                let status = incoming.status();
                let is_image = incoming
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.to_ascii_lowercase().starts_with("image/"));
                let private = incoming
                    .headers()
                    .get(CACHE_CONTROL)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        value.split(',').map(str::trim).any(|value| {
                            value.eq_ignore_ascii_case("no-store")
                                || value.eq_ignore_ascii_case("private")
                        })
                    });
                let bytes = incoming.bytes().await?;
                if cacheable
                    && status == reqwest::StatusCode::OK
                    && is_image
                    && !private
                    && let Some(cache) = cache.as_ref()
                {
                    let cache = cache.clone();
                    let stored = bytes.clone();
                    drop(tokio::task::spawn_blocking(move || {
                        with_cache(&cache, |cache| cache.put(&uri, &stored));
                    }));
                }
                Ok::<_, anyhow::Error>((status, bytes))
            });

            let (status, bytes) = fetch.await??;
            Ok(Response::builder()
                .status(status)
                .body(AsyncBody::from(bytes))?)
        })
    }
}

fn with_cache<T>(
    cache: &Mutex<DiskCache>,
    operation: impl FnOnce(&mut DiskCache) -> T,
) -> Option<T> {
    match cache.lock() {
        Ok(mut cache) => Some(operation(&mut cache)),
        Err(_) => {
            log::warn!("artwork: disk cache lock is poisoned");
            None
        }
    }
}

struct DiskCache {
    root: PathBuf,
    max_bytes: u64,
    max_age: Duration,
    bytes: Option<u64>,
    swept: Instant,
}

impl DiskCache {
    fn new(root: PathBuf, max_bytes: u64, max_age: Duration) -> std::io::Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            max_bytes,
            max_age,
            bytes: None,
            swept: Instant::now(),
        })
    }

    fn get(&mut self, url: &str) -> Option<Vec<u8>> {
        let path = self.path(url);
        let metadata = fs::metadata(&path).ok()?;
        let used = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if used.elapsed().is_ok_and(|age| age > self.max_age) {
            self.remove(&path, metadata.len());
            return None;
        }

        match fs::read(&path) {
            Ok(bytes) => {
                if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                    let now = SystemTime::now();
                    file.set_times(FileTimes::new().set_accessed(now).set_modified(now))
                        .ok();
                }
                Some(bytes)
            }
            Err(_) => {
                self.remove(&path, metadata.len());
                None
            }
        }
    }

    fn put(&mut self, url: &str, bytes: &[u8]) {
        if bytes.len() as u64 > self.max_bytes {
            return;
        }
        if self.bytes.is_none() && self.sweep().is_none() {
            return;
        }

        let path = self.path(url);
        let previous = fs::metadata(&path).map_or(0, |metadata| metadata.len());
        let key = key(url);
        let temporary = self.root.join(format!(
            ".{key}.tmp-{}-{}",
            std::process::id(),
            TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let written = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut file| std::io::Write::write_all(&mut file, bytes))
            .and_then(|()| fs::rename(&temporary, &path));
        if let Err(error) = written {
            fs::remove_file(&temporary).ok();
            log::debug!("artwork: cannot write a disk cache entry: {error}");
            return;
        }

        self.bytes = self.bytes.map(|total| {
            total
                .saturating_sub(previous)
                .saturating_add(bytes.len() as u64)
        });
        if self.bytes.is_some_and(|total| total > self.max_bytes)
            || self.swept.elapsed() >= CACHE_SWEEP
        {
            self.sweep();
        }
    }

    fn sweep(&mut self) -> Option<()> {
        let now = SystemTime::now();
        let mut entries = Vec::new();
        for item in fs::read_dir(&self.root).ok()?.flatten() {
            let path = item.path();
            if !cache_entry(&path) {
                continue;
            }
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            let used = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(used).is_ok_and(|age| age > self.max_age) {
                fs::remove_file(path).ok();
            } else {
                entries.push((path, used, metadata.len()));
            }
        }

        entries.sort_unstable_by_key(|(_, used, _)| *used);
        let mut bytes = entries.iter().map(|(_, _, size)| size).sum::<u64>();
        for (path, _, size) in entries {
            if bytes <= self.max_bytes {
                break;
            }
            if fs::remove_file(path).is_ok() {
                bytes = bytes.saturating_sub(size);
            }
        }
        self.bytes = Some(bytes);
        self.swept = Instant::now();
        Some(())
    }

    fn path(&self, url: &str) -> PathBuf {
        self.root.join(key(url))
    }

    fn remove(&mut self, path: &Path, size: u64) {
        if fs::remove_file(path).is_ok() {
            self.bytes = self.bytes.map(|bytes| bytes.saturating_sub(size));
        }
    }
}

fn key(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
}

fn cache_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn read(body: AsyncBody) -> Result<Option<Vec<u8>>> {
    match body.0 {
        Inner::Empty => Ok(None),
        Inner::Bytes(mut cursor) => {
            let mut bytes = Vec::new();
            cursor.read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        Inner::AsyncReader(_) => Err(anyhow!("streaming request bodies are not supported")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temporary(PathBuf);

    impl Temporary {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sonora-image-cache-test-{}-{}",
                std::process::id(),
                TEMP_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Temporary {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn reads_a_cached_image_and_refreshes_its_age() {
        let root = Temporary::new();
        let mut cache = DiskCache::new(root.0.clone(), 32, Duration::from_secs(60)).unwrap();
        cache.put("https://example.com/cover", b"image");

        let path = cache.path("https://example.com/cover");
        let old = SystemTime::now() - Duration::from_secs(30);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old))
            .unwrap();

        assert_eq!(
            cache.get("https://example.com/cover"),
            Some(b"image".to_vec())
        );
        assert!(fs::metadata(path).unwrap().modified().unwrap() > old);
    }

    #[test]
    fn removes_expired_images() {
        let root = Temporary::new();
        let mut cache = DiskCache::new(root.0.clone(), 32, Duration::from_secs(60)).unwrap();
        cache.put("expired", b"old");
        let expired = cache.path("expired");
        OpenOptions::new()
            .write(true)
            .open(&expired)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(120)))
            .unwrap();
        cache.sweep();

        assert!(!expired.exists());
    }

    #[test]
    fn evicts_the_least_recently_used_image_at_the_size_limit() {
        let root = Temporary::new();
        let mut cache = DiskCache::new(root.0.clone(), 5, Duration::from_secs(60)).unwrap();
        cache.put("old", b"old");
        let old = cache.path("old");
        OpenOptions::new()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(30)))
            .unwrap();
        cache.put("new", b"fresh");

        assert!(!old.exists());
        assert_eq!(cache.get("new"), Some(b"fresh".to_vec()));
        assert!(cache.bytes.unwrap() <= 5);
    }
}
