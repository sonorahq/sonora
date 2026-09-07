use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::lyrics::lrc;
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, LyricsWord, Voice};

const SOURCE: &str = "LrcLib";
const ENDPOINT: &str = "https://lrclib.net/api/search";
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/nolight132/sonora)"
);

pub struct LrcLib {
    http: reqwest::Client,
}

impl LrcLib {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

impl Default for LrcLib {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    #[serde(rename = "albumName")]
    album_name: Option<String>,
    duration: Option<f64>,
    #[serde(rename = "plainLyrics")]
    plain: Option<String>,
    #[serde(rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(default)]
    instrumental: bool,
    lyricsfile: Option<String>,
}

#[derive(Deserialize)]
struct LyricsFile {
    #[serde(default)]
    lines: Vec<FileLine>,
}

#[derive(Deserialize)]
struct FileLine {
    text: String,
    start_ms: u64,
    end_ms: Option<u64>,
    #[serde(default)]
    words: Vec<FileWord>,
}

#[derive(Deserialize)]
struct FileWord {
    text: String,
    start_ms: u64,
    end_ms: Option<u64>,
}

#[async_trait]
impl LyricsProvider for LrcLib {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let response = self
            .http
            .get(ENDPOINT)
            .query(&[
                ("track_name", query.title.as_str()),
                ("artist_name", query.artist.as_str()),
            ])
            .header("User-Agent", AGENT)
            .send()
            .await
            .context("cannot reach lrclib")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("lrclib answered with status {status}");
        }
        let found: Vec<Found> = response
            .json()
            .await
            .context("cannot read the lrclib response")?;
        Ok(found.into_iter().filter_map(hit).collect())
    }
}

fn hit(found: Found) -> Option<LyricsHit> {
    let sheet = found
        .lyricsfile
        .as_deref()
        .and_then(filed)
        .or_else(|| {
            found
                .synced
                .as_deref()
                .map(lrc::parse)
                .filter(|lines| !lines.is_empty())
        })
        .map(|lines| Lyrics::Synced {
            lines: lines.into(),
        })
        .or_else(|| {
            found
                .plain
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(Lyrics::plain)
        });
    let lyrics = match (sheet, found.instrumental) {
        (Some(lyrics), _) => lyrics,
        (None, true) => Lyrics::plain(""),
        (None, false) => return None,
    };

    Some(LyricsHit {
        source: SOURCE,
        trust: 0,
        lyrics,
        instrumental: found.instrumental,
        fallback: false,
        title: found.track_name.unwrap_or_default(),
        artist: found.artist_name.unwrap_or_default(),
        album: found.album_name.filter(|name| !name.is_empty()),
        duration: found.duration.map(Duration::from_secs_f64),
        writers: Vec::new(),
    })
}

fn filed(text: &str) -> Option<Vec<LyricsLine>> {
    let file: LyricsFile = serde_norway::from_str(text)
        .inspect_err(|error| log::warn!("lyrics: cannot read a lrclib lyricsfile: {error}"))
        .ok()?;
    let mut lines: Vec<LyricsLine> = file
        .lines
        .into_iter()
        .map(|line| LyricsLine {
            start: Duration::from_millis(line.start_ms),
            end: line.end_ms.map(Duration::from_millis),
            words: worded(&line.words),
            text: line.text,
            romanized: None,
            secondary: Vec::new(),
            voice: Voice::Lead,
        })
        .collect();
    lrc::normalize(&mut lines);
    (!lines.is_empty()).then_some(lines)
}

fn worded(words: &[FileWord]) -> Option<Vec<LyricsWord>> {
    let words: Vec<LyricsWord> = words
        .iter()
        .map(|word| {
            let start = Duration::from_millis(word.start_ms);
            LyricsWord {
                start,
                end: word.end_ms.map(Duration::from_millis).unwrap_or(start),
                text: word.text.clone(),
            }
        })
        .collect();
    (!words.is_empty()).then_some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_synced_over_plain() {
        let found = Found {
            track_name: Some("Jaded".to_owned()),
            artist_name: Some("Spiritbox".to_owned()),
            album_name: Some("Jaded".to_owned()),
            duration: Some(263.),
            plain: Some("plain".to_owned()),
            synced: Some("[00:01.00] synced".to_owned()),
            lyricsfile: None,
            instrumental: false,
        };

        assert!(hit(found).unwrap().lyrics.synced());
    }

    #[test]
    fn skips_an_entry_without_any_lyrics() {
        let found = Found {
            track_name: Some("Jaded".to_owned()),
            artist_name: Some("Spiritbox".to_owned()),
            album_name: None,
            duration: Some(263.),
            plain: Some("   ".to_owned()),
            synced: None,
            lyricsfile: None,
            instrumental: false,
        };

        assert!(hit(found).is_none());
    }

    #[test]
    fn a_lyricsfile_with_words_beats_the_lrc() {
        let found = Found {
            track_name: Some("Jaded".to_owned()),
            artist_name: Some("Spiritbox".to_owned()),
            album_name: None,
            duration: Some(263.),
            plain: None,
            synced: Some("[00:01.00] line only".to_owned()),
            instrumental: false,
            lyricsfile: Some(
                "version: '1.0'\nmetadata:\n  title: Jaded\n  artist: Spiritbox\nlines:\n- text: 'Hello world'\n  start_ms: 1200\n  end_ms: 2800\n  words:\n  - text: 'Hello '\n    start_ms: 1200\n    end_ms: 1900\n  - text: 'world'\n    start_ms: 1900\n    end_ms: 2800\n"
                    .to_owned(),
            ),
        };

        let lyrics = hit(found).unwrap().lyrics;
        assert!(lyrics.worded());
        let Lyrics::Synced { lines } = &lyrics else {
            unreachable!()
        };
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].end, Some(Duration::from_millis(2800)));
    }

    #[test]
    fn an_empty_lyricsfile_line_preserves_an_instrumental_pause() {
        let lines = filed(
            "lines:\n- text: sung\n  start_ms: 9360\n  end_ms: 11970\n- text: ''\n  start_ms: 11970\n  end_ms: 24160\n- text: next\n  start_ms: 24160\n  end_ms: 27050\n",
        )
        .expect("the lyricsfile is valid");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].end, Some(Duration::from_millis(11_970)));
        assert_eq!(lines[1].start, Duration::from_millis(24_160));
    }

    #[test]
    fn a_broken_lyricsfile_falls_back_to_the_lrc() {
        let found = Found {
            track_name: Some("Jaded".to_owned()),
            artist_name: Some("Spiritbox".to_owned()),
            album_name: None,
            duration: Some(263.),
            plain: None,
            synced: Some("[00:01.00] line only".to_owned()),
            lyricsfile: Some(": not yaml [".to_owned()),
            instrumental: false,
        };

        let lyrics = hit(found).unwrap().lyrics;
        assert!(lyrics.synced());
        assert!(!lyrics.worded());
    }
}
