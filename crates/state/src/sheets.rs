//! The lyrics the services answered with, kept in the cache database one track at a time, so
//! only the sheets something asks for are ever read into memory.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use music::{Lyrics as Sheet, LyricsContributor, LyricsHit};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 5;
const PASSING: bool = cfg!(debug_assertions);
const CAPACITY: usize = 500;

/// Everything the services said about one track. A service in `answered` replied, even if it had
/// nothing, so a lookup asks it again only while nothing usable is stored. One that failed is
/// left out and gets asked next time.
#[derive(Serialize, Deserialize)]
struct Kept {
    stored: u64,
    #[serde(default)]
    answered: Vec<String>,
    #[serde(default)]
    hits: Vec<Held>,
}

#[derive(Serialize, Deserialize)]
struct Held {
    source: String,
    #[serde(default)]
    trust: u32,
    lyrics: Sheet,
    #[serde(default)]
    instrumental: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    contributors: Vec<LyricsContributor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires: Option<SystemTime>,
}

/// One track's answer on its way to the database.
pub(crate) struct Unsaved {
    key: String,
    kept: Kept,
}

/// The stored sheets. Every method blocks on sqlite and json, so a caller runs it off the main
/// thread. Debug builds neither read nor write them, so a lookup always reaches the services.
#[derive(Clone)]
pub(crate) struct Sheets {
    cache: storage::Cache,
}

impl Sheets {
    pub(crate) fn new(cache: storage::Cache) -> Self {
        Self { cache }
    }

    /// The hits stored for `key` and the services that answered, limited to the sources in
    /// `known`. `None` when nothing is stored.
    pub(crate) fn get(
        &self,
        key: &str,
        known: &[&'static str],
    ) -> Option<(Vec<LyricsHit>, Vec<&'static str>)> {
        if PASSING {
            return None;
        }
        let value = self
            .cache
            .lyrics(key, VERSION)
            .inspect_err(|error| log::warn!("lyrics: cannot read the cached sheet: {error:#}"))
            .ok()??;
        let kept: Kept = serde_json::from_str(&value)
            .inspect_err(|error| log::warn!("lyrics: cannot read the cached sheet: {error}"))
            .ok()?;
        // An expired hit is dropped and its source asked again, as the catalogue requires.
        let now = SystemTime::now();
        let (hits, expired): (Vec<Held>, Vec<Held>) = kept
            .hits
            .into_iter()
            .partition(|held| held.expires.is_none_or(|expires| expires > now));
        let answered = kept
            .answered
            .iter()
            .filter(|source| !expired.iter().any(|held| held.source == **source))
            .filter_map(|source| known.iter().copied().find(|name| name == source))
            .collect();
        let hits = hits
            .into_iter()
            .filter_map(|held| held.hit(known))
            .collect();
        Some((hits, answered))
    }

    /// An answer to store under `key`, stamped with the time it was found.
    pub(crate) fn unsaved(key: String, hits: &[LyricsHit], answered: &[&'static str]) -> Unsaved {
        Unsaved {
            key,
            kept: Kept {
                stored: now(),
                answered: answered.iter().map(|name| (*name).to_owned()).collect(),
                hits: hits.iter().map(Held::of).collect(),
            },
        }
    }

    /// Writes `unsaved` and drops the oldest sheets past the capacity.
    pub(crate) fn save(&self, unsaved: Vec<Unsaved>) {
        if PASSING || unsaved.is_empty() {
            return;
        }
        if let Err(error) = self.keep(unsaved.into_iter().map(|entry| (entry.key, entry.kept))) {
            log::warn!("lyrics: cannot save the cached sheets: {error:#}");
        }
    }

    /// Deletes the old `lyrics.json`. Its sheets were filed by which services were enabled, so
    /// none of them can be read as one track's answer.
    pub(crate) fn migrate(&self) {
        if PASSING {
            return;
        }
        let path = legacy_path();
        match fs::remove_file(&path) {
            Ok(()) => log::info!("lyrics: removed the old cached sheets"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("lyrics: cannot remove {}: {error}", path.display()),
        }
    }

    fn keep(&self, entries: impl IntoIterator<Item = (String, Kept)>) -> anyhow::Result<()> {
        let rows = entries.into_iter().filter_map(|(key, kept)| {
            let value = serde_json::to_string(&kept)
                .inspect_err(|error| log::warn!("lyrics: cannot serialize a sheet: {error}"))
                .ok()?;
            Some((key, i64::try_from(kept.stored).unwrap_or(i64::MAX), value))
        });
        self.cache.keep_lyrics(VERSION, rows, CAPACITY)
    }
}

impl Held {
    fn of(hit: &LyricsHit) -> Self {
        Self {
            source: hit.source.to_owned(),
            trust: hit.trust,
            lyrics: hit.lyrics.clone(),
            instrumental: hit.instrumental,
            title: hit.title.clone(),
            artist: hit.artist.clone(),
            album: hit.album.clone(),
            duration: hit.duration,
            writers: hit.writers.clone(),
            origin: hit.origin.clone(),
            contributors: hit.contributors.clone(),
            expires: hit.expires,
        }
    }

    fn hit(self, known: &[&'static str]) -> Option<LyricsHit> {
        let source = known.iter().copied().find(|name| *name == self.source)?;
        Some(LyricsHit {
            source,
            trust: self.trust,
            lyrics: self.lyrics,
            instrumental: self.instrumental,
            title: self.title,
            artist: self.artist,
            album: self.album,
            duration: self.duration,
            writers: self.writers,
            origin: self.origin,
            contributors: self.contributors,
            expires: self.expires,
        })
    }
}

fn legacy_path() -> PathBuf {
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
