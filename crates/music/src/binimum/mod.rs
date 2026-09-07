use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::lyrics::ttml;
use crate::{LyricsHit, LyricsProvider, LyricsQuery};

const SOURCE: &str = "Apple Music";
const SEARCH: &str = "https://lyrics-api.binimum.org/";
const TRUST: u32 = 180;
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/nolight132/sonora)"
);

pub struct Binimum {
    http: reqwest::Client,
}

impl Binimum {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn find(&self, query: &LyricsQuery) -> Result<Vec<Match>> {
        let duration = query.duration.as_secs().to_string();
        let mut params = vec![
            ("track", query.title.as_str()),
            ("artist", query.artist.as_str()),
        ];
        if let Some(album) = query.album.as_deref().filter(|album| !album.is_empty()) {
            params.push(("album", album));
        }
        if !query.duration.is_zero() {
            params.push(("duration", duration.as_str()));
        }

        let response = self
            .http
            .get(SEARCH)
            .query(&params)
            .header("User-Agent", AGENT)
            .send()
            .await
            .context("cannot reach the apple lyrics catalogue")?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            anyhow::bail!("the apple lyrics catalogue answered with status {status}");
        }
        let found: Found = response
            .json()
            .await
            .context("cannot read the apple lyrics catalogue response")?;
        Ok(found.results)
    }

    async fn sheet(&self, url: &str) -> Result<String> {
        if !url.starts_with("https://") {
            anyhow::bail!("the apple lyrics catalogue offered an insecure sheet");
        }
        let response = self
            .http
            .get(url)
            .header("User-Agent", AGENT)
            .send()
            .await
            .context("cannot reach the apple lyrics sheet")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("the apple lyrics sheet answered with status {status}");
        }
        response
            .text()
            .await
            .context("cannot read the apple lyrics sheet")
    }
}

impl Default for Binimum {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct Found {
    #[serde(default)]
    results: Vec<Match>,
}

#[derive(Deserialize)]
struct Match {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    album_name: String,
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    timing_type: String,
    #[serde(rename = "lyricsUrl")]
    lyrics_url: Option<String>,
}

impl Match {
    fn worded(&self) -> bool {
        self.timing_type == "word" || self.timing_type == "syllable"
    }
}

#[async_trait]
impl LyricsProvider for Binimum {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let mut found = self.find(query).await?;
        found.sort_by_key(|found| !found.worded());
        let Some((best, url)) = found
            .iter()
            .find_map(|found| Some((found, found.lyrics_url.as_deref()?)))
        else {
            return Ok(Vec::new());
        };

        let sheet = ttml::parse(&self.sheet(url).await?)?;
        if sheet.lyrics.is_empty() {
            return Ok(Vec::new());
        }

        Ok(vec![LyricsHit {
            source: SOURCE,
            trust: TRUST,
            lyrics: sheet.lyrics,
            instrumental: false,
            fallback: false,
            title: best.track_name.clone(),
            artist: best.artist_name.clone(),
            album: (!best.album_name.is_empty()).then(|| best.album_name.clone()),
            duration: (best.duration > 0).then(|| Duration::from_secs(best.duration)),
            writers: sheet.writers,
        }])
    }
}
