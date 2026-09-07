use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use music::{Lyrics as Sheet, LyricsHit};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 3;
const PASSING: bool = cfg!(debug_assertions);
const CAPACITY: usize = 500;

#[derive(Serialize, Deserialize)]
struct Vault {
    version: u32,
    #[serde(default)]
    entries: HashMap<String, Kept>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Kept {
    stored: u64,
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    hits: Vec<Held>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Held {
    source: String,
    #[serde(default)]
    trust: u32,
    lyrics: Sheet,
    #[serde(default)]
    instrumental: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    fallback: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration: Option<Duration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    writers: Vec<String>,
}

pub(crate) struct Loaded(HashMap<String, Kept>);

pub(crate) struct Chore {
    path: PathBuf,
    entries: HashMap<String, Kept>,
}

pub(crate) struct Sheets {
    path: PathBuf,
    entries: HashMap<String, Kept>,
    dirty: bool,
    warmed: bool,
}

impl Sheets {
    pub(crate) fn new() -> Self {
        Self {
            path: path(),
            entries: HashMap::new(),
            dirty: false,
            warmed: PASSING,
        }
    }

    pub(crate) fn read() -> Loaded {
        if PASSING {
            return Loaded(HashMap::new());
        }
        let path = path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded(HashMap::new());
            }
            Err(error) => {
                log::warn!("lyrics: cannot read {}: {error}", path.display());
                return Loaded(HashMap::new());
            }
        };
        let vault: Vault = match serde_json::from_slice(&bytes) {
            Ok(vault) => vault,
            Err(error) => {
                log::warn!("lyrics: cannot read the cached sheets: {error}");
                return Loaded(HashMap::new());
            }
        };
        match vault.version == VERSION {
            true => Loaded(vault.entries),
            false => Loaded(HashMap::new()),
        }
    }

    pub(crate) fn absorb(&mut self, loaded: Loaded) {
        self.warmed = true;
        for (key, kept) in loaded.0 {
            self.entries.entry(key).or_insert(kept);
        }
    }

    pub(crate) fn holds(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub(crate) fn get(&self, key: &str, known: &[&'static str]) -> Option<(Vec<LyricsHit>, bool)> {
        let kept = self.entries.get(key)?;
        let hits: Vec<LyricsHit> = kept
            .hits
            .iter()
            .filter_map(|held| held.hit(known))
            .collect();
        let instrumental = kept.instrumental;
        (!hits.is_empty() || instrumental).then_some((hits, instrumental))
    }

    pub(crate) fn put(&mut self, key: String, hits: &[LyricsHit], instrumental: bool) {
        self.entries.insert(
            key,
            Kept {
                stored: now(),
                instrumental,
                hits: hits.iter().map(Held::of).collect(),
            },
        );
        self.evict();
        self.dirty = true;
    }

    pub(crate) fn chore(&mut self) -> Option<Chore> {
        (self.dirty && self.warmed && !PASSING).then(|| {
            self.dirty = false;
            Chore {
                path: self.path.clone(),
                entries: self.entries.clone(),
            }
        })
    }

    fn evict(&mut self) {
        while self.entries.len() > CAPACITY {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, kept)| kept.stored)
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            self.entries.remove(&oldest);
        }
    }
}

impl Chore {
    pub(crate) fn write(self) {
        let Some(parent) = self.path.parent() else {
            return;
        };
        if let Err(error) = fs::create_dir_all(parent) {
            log::warn!("lyrics: cannot create {}: {error}", parent.display());
            return;
        }
        let vault = Vault {
            version: VERSION,
            entries: self.entries,
        };
        let bytes = match serde_json::to_vec(&vault) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("lyrics: cannot serialize the cached sheets: {error}");
                return;
            }
        };
        if let Err(error) = fs::write(&self.path, bytes) {
            log::warn!("lyrics: cannot write {}: {error}", self.path.display());
        }
    }
}

impl Held {
    fn of(hit: &LyricsHit) -> Self {
        Self {
            source: hit.source.to_owned(),
            trust: hit.trust,
            lyrics: hit.lyrics.clone(),
            instrumental: hit.instrumental,
            fallback: hit.fallback,
            title: hit.title.clone(),
            artist: hit.artist.clone(),
            album: hit.album.clone(),
            duration: hit.duration,
            writers: hit.writers.clone(),
        }
    }

    fn hit(&self, known: &[&'static str]) -> Option<LyricsHit> {
        let source = known.iter().copied().find(|name| *name == self.source)?;
        Some(LyricsHit {
            source,
            trust: self.trust,
            lyrics: self.lyrics.clone(),
            instrumental: self.instrumental,
            fallback: self.fallback,
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration: self.duration,
            writers: self.writers.clone(),
        })
    }
}

fn path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sonora")
        .join("lyrics.json")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
