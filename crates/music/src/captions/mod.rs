use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use serde_json::Value;
use ytmusic::{Client, YtMusic};

use crate::lyrics::lrc;
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, Voice};

const SOURCE: &str = "YouTube Captions";
const PROVIDER: &str = "youtube";
const CLIENT: Client = Client::VisionOs;
const TRUST: u32 = 0;
const ASR: &str = "asr";
const TRACKS: &str = "/captions/playerCaptionsTracklistRenderer/captionTracks";
const AUDIO: &str = "/captions/playerCaptionsTracklistRenderer/audioTracks";
const SPOKEN: &str = "/captions/playerCaptionsTracklistRenderer/defaultAudioTrackIndex";

pub struct Captions {
    api: Arc<YtMusic>,
    http: reqwest::Client,
}

struct Written {
    generated: bool,
    language: String,
    url: String,
}

impl Captions {
    pub fn new() -> Self {
        Self {
            api: Arc::new(YtMusic::anonymous()),
            http: reqwest::Client::new(),
        }
    }

    async fn read(&self, written: &Written) -> Result<Vec<LyricsLine>> {
        let body = self
            .http
            .get(format!("{}&fmt=json3", written.url))
            .send()
            .await
            .context("cannot reach the timed text endpoint")?
            .error_for_status()
            .context("the timed text endpoint refused")?
            .text()
            .await
            .context("cannot read the timed text body")?;
        if body.trim().is_empty() {
            bail!("the timed text endpoint served nothing");
        }
        let timed: Value = serde_json::from_str(&body).context("cannot parse the timed text")?;
        let mut lines: Vec<LyricsLine> = timed
            .get("events")
            .and_then(Value::as_array)
            .map(|events| events.iter().filter_map(line).collect())
            .unwrap_or_default();
        lrc::normalize(&mut lines);
        Ok(lines)
    }
}

impl Default for Captions {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsProvider for Captions {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(id) = query.id_for(PROVIDER) else {
            return Ok(Vec::new());
        };
        let response = self
            .api
            .player_response(id, CLIENT)
            .await
            .context("cannot ask for the player response")?;
        let Some(written) = pick(&response, query.language.as_deref()) else {
            return Ok(Vec::new());
        };
        let lines = self.read(&written).await?;
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![LyricsHit {
            source: SOURCE,
            trust: TRUST,
            lyrics: Lyrics::Synced {
                lines: lines.into(),
            },
            instrumental: false,
            fallback: true,
            title: query.title.clone(),
            artist: query.artist.clone(),
            album: query.album.clone(),
            duration: (!query.duration.is_zero()).then_some(query.duration),
            writers: Vec::new(),
        }])
    }
}

fn pick(response: &Value, language: Option<&str>) -> Option<Written> {
    let found: Vec<Written> = response
        .pointer(TRACKS)?
        .as_array()?
        .iter()
        .filter_map(written)
        .collect();
    let authored = |written: &Written| !written.generated;
    let read = |written: &Written| language.is_some_and(|wanted| alike(&written.language, wanted));
    let index = found
        .iter()
        .position(|written| authored(written) && read(written))
        .or_else(|| spoken(response).filter(|index| found.get(*index).is_some_and(authored)))
        .or_else(|| found.iter().position(authored))
        .or_else(|| found.iter().position(read))
        .unwrap_or_default();
    found.into_iter().nth(index)
}

fn alike(code: &str, language: &str) -> bool {
    primary(code).eq_ignore_ascii_case(primary(language))
}

fn primary(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or(tag)
}

fn spoken(response: &Value) -> Option<usize> {
    let chosen = response
        .pointer(SPOKEN)
        .and_then(Value::as_u64)
        .unwrap_or(0);
    response
        .pointer(AUDIO)?
        .as_array()?
        .get(chosen as usize)?
        .get("defaultCaptionTrackIndex")?
        .as_u64()
        .map(|index| index as usize)
}

fn written(found: &Value) -> Option<Written> {
    Some(Written {
        generated: found.get("kind").and_then(Value::as_str) == Some(ASR),
        language: found
            .get("languageCode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        url: found.get("baseUrl")?.as_str()?.to_owned(),
    })
}

fn line(event: &Value) -> Option<LyricsLine> {
    let start = event.get("tStartMs")?.as_u64()?;
    let text = event
        .get("segs")?
        .as_array()?
        .iter()
        .filter_map(|seg| seg.get("utf8").and_then(Value::as_str))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    Some(LyricsLine {
        start: Duration::from_millis(start),
        end: event
            .get("dDurationMs")
            .and_then(Value::as_u64)
            .map(|span| Duration::from_millis(start + span)),
        text,
        romanized: None,
        words: None,
        secondary: Vec::new(),
        voice: Voice::Lead,
    })
}
