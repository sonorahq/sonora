use std::fs::{self, FileTimes, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};
use tokio::runtime::Handle;

const USER_AGENT: &str = "sonora";
const CACHE_BYTES: u64 = 128 * 1024 * 1024;
const CACHE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const CACHE_SWEEP: Duration = Duration::from_secs(60 * 60);

static TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Fetches cover art over HTTP, keeping images in a disk cache so a cover seen in an earlier run
/// opens without the network.
#[derive(Clone)]
pub struct Client {
    inner: reqwest::Client,
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
            cache,
        }
    }

    /// The bytes at `url`, from the disk cache when it holds them. Only images are cached, and
    /// never a response the server marked private.
    pub async fn fetch(&self, url: String) -> Result<Vec<u8>> {
        if let Some(cache) = self.cache.clone() {
            let key = url.clone();
            let cached = tokio::task::spawn_blocking(move || {
                with_cache(&cache, |cache| cache.get(&key)).flatten()
            })
            .await
            .unwrap_or_else(|error| {
                log::warn!("artwork: cannot read the disk cache: {error}");
                None
            });
            if let Some(bytes) = cached {
                return Ok(bytes);
            }
        }

        let incoming = self
            .inner
            .get(&url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        let status = incoming.status();
        if !status.is_success() {
            return Err(anyhow!("artwork: the server answered {status}"));
        }
        let header = |name: &str| {
            incoming
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_ascii_lowercase)
        };
        let is_image = header("content-type").is_some_and(|value| value.starts_with("image/"));
        let private = header("cache-control").is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|value| value == "no-store" || value == "private")
        });
        let bytes = incoming.bytes().await?.to_vec();
        if is_image
            && !private
            && let Some(cache) = self.cache.clone()
        {
            let stored = bytes.clone();
            drop(tokio::task::spawn_blocking(move || {
                with_cache(&cache, |cache| cache.put(&url, &stored));
            }));
        }
        Ok(bytes)
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
