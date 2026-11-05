//! The lyrics sheet a listener picked by hand for a track, kept in the state database with the
//! other things a cache clear must not lose. The sheet itself is stored, so the choice outlives
//! the sources and the evictable sheets cache, along with the search that found it, so the
//! choice can say where it came from.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use music::{Lyrics as Sheet, LyricsHit};
use serde::{Deserialize, Serialize};

/// One manual match: the sheet bound to a track and the typed search it answered.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Choice {
    #[serde(default)]
    searched_title: String,
    #[serde(default)]
    searched_artist: String,
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
}

impl Choice {
    /// The choice binding `hit` to its track, remembering the search it was found with.
    pub(crate) fn of(searched_title: &str, searched_artist: &str, hit: &LyricsHit) -> Self {
        Self {
            searched_title: searched_title.to_owned(),
            searched_artist: searched_artist.to_owned(),
            source: hit.source.to_owned(),
            trust: hit.trust,
            lyrics: hit.lyrics.clone(),
            instrumental: hit.instrumental,
            title: hit.title.clone(),
            artist: hit.artist.clone(),
            album: hit.album.clone(),
            duration: hit.duration,
            writers: hit.writers.clone(),
        }
    }

    /// The stored choice as a hit again, or nothing when its source is gone from this build.
    pub(crate) fn hit(&self, known: &[&'static str]) -> Option<LyricsHit> {
        let source = known.iter().copied().find(|name| *name == self.source)?;
        Some(LyricsHit {
            source,
            trust: self.trust,
            lyrics: self.lyrics.clone(),
            instrumental: self.instrumental,
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration: self.duration,
            writers: self.writers.clone(),
        })
    }

    /// The title and artist the match was searched for with.
    pub(crate) fn searched(&self) -> (&str, &str) {
        (&self.searched_title, &self.searched_artist)
    }
}

/// The manual matches on file. Every method blocks on sqlite and json, so a caller runs it off
/// the main thread. Unlike the cached sheets these are a listener's picks, not answers the
/// sources can give again, so debug builds read and write them like any other build.
#[derive(Clone)]
pub(crate) struct Choices {
    database: storage::Database,
}

impl Choices {
    pub(crate) fn new(database: storage::Database) -> Self {
        Self { database }
    }

    /// Every choice on file, keyed like the cached sheets. A choice whose json cannot be read
    /// is skipped, not fatal.
    pub(crate) fn load(&self) -> HashMap<String, Choice> {
        self.database
            .lyrics_choices()
            .inspect_err(|error| log::warn!("lyrics: cannot read the saved matches: {error:#}"))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| {
                serde_json::from_str(&value)
                    .inspect_err(|error| log::warn!("lyrics: cannot read a saved match: {error}"))
                    .ok()
                    .map(|choice| (key, choice))
            })
            .collect()
    }

    /// Saves `choice` under `key`, stamped with the time it was made.
    pub(crate) fn save(&self, key: &str, choice: &Choice) -> anyhow::Result<()> {
        let value = serde_json::to_string(choice)?;
        self.database.keep_lyrics_choice(key, now(), &value)
    }

    /// Drops the choice under `key`, if there is one.
    pub(crate) fn drop(&self, key: &str) -> anyhow::Result<()> {
        self.database.drop_lyrics_choice(key)
    }
}

fn now() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use music::Lyrics as Sheet;

    use super::{Choice, Choices};

    const KNOWN: &[&str] = &["LrcLib", "NetEase"];

    fn hit(source: &'static str) -> music::LyricsHit {
        music::LyricsHit {
            source,
            lyrics: Sheet::plain("a line"),
            title: "夜に駆ける".to_owned(),
            artist: "YOASOBI".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_choice_survives_a_fresh_store() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let choices = Choices::new(storage::Database::at(dir.path().join("state.sqlite")));
        choices
            .save(
                "subsonic:abc:lyrics",
                &Choice::of("夜に駆ける", "YOASOBI", &hit("LrcLib")),
            )
            .expect("the choice saves");

        // A fresh handle stands in for a restart.
        let again = Choices::new(storage::Database::at(dir.path().join("state.sqlite")));
        let loaded = again.load();

        let choice = loaded
            .get("subsonic:abc:lyrics")
            .expect("the choice is back");
        assert_eq!(choice.searched(), ("夜に駆ける", "YOASOBI"));
        let hit = choice.hit(KNOWN).expect("the source is known");
        assert_eq!(hit.source, "LrcLib");
        assert_eq!(hit.title, "夜に駆ける");
    }

    #[test]
    fn a_choice_from_a_gone_source_is_skipped() {
        let choice = Choice::of("title", "artist", &hit("SomewhereElse"));
        assert!(choice.hit(KNOWN).is_none());
    }

    #[test]
    fn a_failed_drop_is_an_error_not_silence() {
        // A database path that cannot open (a directory) makes every call fail, and the
        // caller must hear it: Lyrics turns this Err into a toast, since the match would
        // otherwise look removed yet come back on the next start.
        let dir = tempfile::tempdir().expect("a temp dir");
        let choices = Choices::new(storage::Database::at(dir.path().to_path_buf()));

        assert!(choices.drop("subsonic:abc:lyrics").is_err());
    }

    #[test]
    fn a_dropped_choice_stays_gone() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let choices = Choices::new(storage::Database::at(dir.path().join("state.sqlite")));
        choices
            .save(
                "local:/tmp/x.mp3:lyrics",
                &Choice::of("t", "a", &hit("NetEase")),
            )
            .expect("the choice saves");
        choices
            .drop("local:/tmp/x.mp3:lyrics")
            .expect("the choice drops");

        let again = Choices::new(storage::Database::at(dir.path().join("state.sqlite")));
        assert!(again.load().is_empty());
    }
}
