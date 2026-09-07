use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use tokio::sync::Mutex;

use crate::lyrics::lrc;
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, LyricsWord, Voice};

const SOURCE: &str = "Musixmatch";
const TOKENS: &str = "https://apic-desktop.musixmatch.com/ws/1.1/token.get";
const SUBTITLES: &str = "https://apic-desktop.musixmatch.com/ws/1.1/macro.subtitles.get";
const APP: &str = "web-desktop-app-v1.0";
const TRUST: u32 = 35;
const SPACING: Duration = Duration::from_millis(1500);
const COOLDOWN: Duration = Duration::from_secs(600);
const AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub struct Musixmatch {
    http: reqwest::Client,
    gate: Mutex<Gate>,
}

#[derive(Default)]
struct Gate {
    token: Option<String>,
    blocked: Option<Instant>,
    latest: Option<Instant>,
}

impl Musixmatch {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            gate: Mutex::new(Gate::default()),
        }
    }

    async fn ready(&self) -> bool {
        let mut gate = self.gate.lock().await;
        if gate.blocked.is_some_and(|until| Instant::now() < until) {
            return false;
        }
        gate.blocked = None;
        if let Some(since) = gate.latest.map(|latest| latest.elapsed())
            && since < SPACING
        {
            tokio::time::sleep(SPACING - since).await;
        }
        gate.latest = Some(Instant::now());
        true
    }

    async fn block(&self) {
        let mut gate = self.gate.lock().await;
        gate.blocked = Some(Instant::now() + COOLDOWN);
    }

    async fn token(&self) -> Result<String> {
        if let Some(token) = self.gate.lock().await.token.clone() {
            return Ok(token);
        }
        if let Some(token) = kept() {
            self.gate.lock().await.token = Some(token.clone());
            return Ok(token);
        }
        self.renew().await
    }

    async fn renew(&self) -> Result<String> {
        self.gate.lock().await.token = None;
        let answer = self
            .ask(TOKENS, &[("format", "json"), ("app_id", APP)])
            .await?;
        let token = match answer.message.header.status_code {
            200 => answer
                .message
                .body
                .and_then(|body| body.user_token)
                .filter(|token| !token.contains("UpgradeOnly"))
                .context("musixmatch handed out no token")?,
            401 | 402 | 403 | 429 => {
                self.block().await;
                anyhow::bail!("musixmatch is throttling this network");
            }
            status => anyhow::bail!("musixmatch answered the token request with status {status}"),
        };
        remember(&token);
        self.gate.lock().await.token = Some(token.clone());
        Ok(token)
    }

    async fn ask(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<Answer> {
        let response = self
            .http
            .get(endpoint)
            .query(params)
            .header("User-Agent", AGENT)
            .header("Cookie", "x-mxm-token-guid=")
            .send()
            .await
            .context("cannot reach musixmatch")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("musixmatch answered with status {status}");
        }
        response
            .json()
            .await
            .context("cannot read the musixmatch response")
    }

    async fn lookup(&self, query: &LyricsQuery) -> Result<Option<Calls>> {
        let duration = query.duration.as_secs().to_string();
        for renewed in [false, true] {
            if !self.ready().await {
                return Ok(None);
            }
            let token = match renewed {
                true => self.renew().await?,
                false => self.token().await?,
            };

            let mut params = vec![
                ("format", "json"),
                ("namespace", "lyrics_richsynched"),
                ("subtitle_format", "mxm"),
                ("optional_calls", "track.richsync"),
                ("app_id", APP),
                ("usertoken", token.as_str()),
            ];
            match query.id_for("spotify") {
                Some(id) => params.push(("track_spotify_id", id)),
                None => {
                    params.push(("q_track", query.title.as_str()));
                    params.push(("q_artist", query.artist.as_str()));
                    if let Some(album) = query.album.as_deref().filter(|album| !album.is_empty()) {
                        params.push(("q_album", album));
                    }
                    if !query.duration.is_zero() {
                        params.push(("q_duration", duration.as_str()));
                    }
                }
            }

            let answer = self.ask(SUBTITLES, &params).await?;
            let header = &answer.message.header;
            match (header.status_code, header.hint.as_deref()) {
                (200, _) => return Ok(answer.message.body.and_then(|body| body.macro_calls)),
                (401, Some("renew")) => continue,
                (401 | 402 | 403 | 429, hint) => {
                    self.block().await;
                    anyhow::bail!(
                        "musixmatch is throttling this network ({})",
                        hint.unwrap_or("no hint")
                    );
                }
                (404, _) => return Ok(None),
                (status, _) => anyhow::bail!("musixmatch answered with status {status}"),
            }
        }
        anyhow::bail!("musixmatch keeps rejecting the token")
    }
}

impl Default for Musixmatch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsProvider for Musixmatch {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(calls) = self.lookup(query).await? else {
            return Ok(Vec::new());
        };
        Ok(hit(&calls).into_iter().collect())
    }
}

fn hit(calls: &Calls) -> Option<LyricsHit> {
    let matched = calls.matcher.as_ref()?.answered()?;
    let track = matched.track.as_ref()?;
    let quiet = track.instrumental == 1;

    let sung = match quiet {
        true => Vec::new(),
        false => richsync(calls)
            .or_else(|| subtitles(calls))
            .unwrap_or_default(),
    };
    let mut lines = sung;
    lrc::normalize(&mut lines);
    if lines.is_empty() && !quiet {
        return None;
    }

    Some(LyricsHit {
        source: SOURCE,
        trust: TRUST,
        lyrics: match quiet {
            true => Lyrics::plain(""),
            false => Lyrics::Synced {
                lines: lines.into(),
            },
        },
        instrumental: quiet,
        fallback: false,
        title: track.track_name.clone(),
        artist: track.artist_name.clone(),
        album: (!track.album_name.is_empty()).then(|| track.album_name.clone()),
        duration: (track.track_length > 0).then(|| Duration::from_secs(track.track_length)),
        writers: writers(calls),
    })
}

fn richsync(calls: &Calls) -> Option<Vec<LyricsLine>> {
    let body = calls.richsync.as_ref()?.answered()?.richsync.as_ref()?;
    let sung: Vec<Rich> = serde_json::from_str(&body.richsync_body)
        .inspect_err(|error| log::warn!("lyrics: cannot read a musixmatch richsync: {error}"))
        .ok()?;
    let lines: Vec<LyricsLine> = sung.into_iter().map(verse).collect();
    (!lines.is_empty()).then_some(lines)
}

fn verse(sung: Rich) -> LyricsLine {
    let start = seconds(sung.ts);
    let end = seconds(sung.te).max(start);
    let mut words: Vec<LyricsWord> = Vec::new();
    for (index, token) in sung.tokens.iter().enumerate() {
        let opened = seconds(sung.ts + token.offset);
        let closed = match sung.tokens.get(index + 1) {
            Some(next) => seconds(sung.ts + next.offset),
            None => end,
        }
        .max(opened);
        match words.last_mut() {
            Some(word) if token.text.chars().all(char::is_whitespace) => {
                word.text.push_str(&token.text);
                word.end = word.end.max(closed);
            }
            _ => words.push(LyricsWord {
                start: opened,
                end: closed,
                text: token.text.clone(),
            }),
        }
    }

    LyricsLine {
        start,
        end: Some(end),
        text: sung.text,
        romanized: None,
        words: (!words.is_empty()).then_some(words),
        secondary: Vec::new(),
        voice: Voice::Lead,
    }
}

fn subtitles(calls: &Calls) -> Option<Vec<LyricsLine>> {
    let body = calls
        .subtitles
        .as_ref()?
        .answered()?
        .subtitle_list
        .first()?;
    let cues: Vec<Cue> = serde_json::from_str(&body.subtitle.subtitle_body)
        .inspect_err(|error| log::warn!("lyrics: cannot read a musixmatch subtitle: {error}"))
        .ok()?;
    let starts: Vec<Duration> = cues.iter().map(|cue| seconds(cue.time.total)).collect();
    let lines: Vec<LyricsLine> = cues
        .iter()
        .enumerate()
        .map(|(index, cue)| LyricsLine {
            start: starts[index],
            end: starts.get(index + 1).copied(),
            text: cue.text.clone(),
            romanized: None,
            words: None,
            secondary: Vec::new(),
            voice: Voice::Lead,
        })
        .collect();
    (!lines.is_empty()).then_some(lines)
}

fn writers(calls: &Calls) -> Vec<String> {
    let credit = calls
        .richsync
        .as_ref()
        .and_then(Wrapped::answered)
        .and_then(|body| body.richsync.as_ref())
        .and_then(|rich| rich.lyrics_copyright.as_deref())
        .or_else(|| {
            calls
                .subtitles
                .as_ref()
                .and_then(Wrapped::answered)
                .and_then(|body| body.subtitle_list.first())
                .and_then(|entry| entry.subtitle.lyrics_copyright.as_deref())
        })
        .unwrap_or_default();
    credited(credit)
}

fn credited(copyright: &str) -> Vec<String> {
    copyright
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Writer(s):"))
        .flat_map(|names| names.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value.max(0.))
}

fn path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join("musixmatch.json")
}

fn kept() -> Option<String> {
    let held: Held = serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()?;
    Some(held.user_token).filter(|token| !token.is_empty())
}

fn remember(token: &str) {
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let held = Held {
        user_token: token.to_owned(),
    };
    if let Ok(text) = serde_json::to_string(&held)
        && let Err(error) = std::fs::write(&path, text)
    {
        log::warn!("lyrics: cannot keep the musixmatch token: {error}");
    }
}

#[derive(Deserialize, serde::Serialize)]
struct Held {
    user_token: String,
}

fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

#[derive(Deserialize)]
struct Answer {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    header: Header,
    #[serde(default, deserialize_with = "lenient")]
    body: Option<Body>,
}

#[derive(Deserialize)]
struct Header {
    status_code: i64,
    #[serde(default)]
    hint: Option<String>,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    user_token: Option<String>,
    #[serde(default)]
    macro_calls: Option<Calls>,
}

#[derive(Deserialize)]
struct Calls {
    #[serde(rename = "matcher.track.get")]
    matcher: Option<Wrapped<Matched>>,
    #[serde(rename = "track.richsync.get")]
    richsync: Option<Wrapped<RichBody>>,
    #[serde(rename = "track.subtitles.get")]
    subtitles: Option<Wrapped<SubtitleBody>>,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
struct Wrapped<T> {
    message: Envelope<T>,
}

impl<T> Wrapped<T> {
    fn answered(&self) -> Option<&T> {
        (self.message.header.status_code == 200)
            .then_some(self.message.body.as_ref())
            .flatten()
    }
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
struct Envelope<T> {
    header: Header,
    #[serde(default, deserialize_with = "lenient")]
    body: Option<T>,
}

#[derive(Deserialize)]
struct Matched {
    track: Option<Track>,
}

#[derive(Deserialize)]
struct Track {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    album_name: String,
    #[serde(default)]
    track_length: u64,
    #[serde(default)]
    instrumental: u8,
}

#[derive(Deserialize)]
struct RichBody {
    richsync: Option<RichSync>,
}

#[derive(Deserialize)]
struct RichSync {
    richsync_body: String,
    #[serde(default)]
    lyrics_copyright: Option<String>,
}

#[derive(Deserialize)]
struct SubtitleBody {
    #[serde(default)]
    subtitle_list: Vec<SubtitleEntry>,
}

#[derive(Deserialize)]
struct SubtitleEntry {
    subtitle: Subtitle,
}

#[derive(Deserialize)]
struct Subtitle {
    subtitle_body: String,
    #[serde(default)]
    lyrics_copyright: Option<String>,
}

#[derive(Deserialize)]
struct Rich {
    ts: f64,
    te: f64,
    #[serde(rename = "x")]
    text: String,
    #[serde(default, rename = "l")]
    tokens: Vec<RichToken>,
}

#[derive(Deserialize)]
struct RichToken {
    #[serde(rename = "c")]
    text: String,
    #[serde(rename = "o")]
    offset: f64,
}

#[derive(Deserialize)]
struct Cue {
    text: String,
    time: Stamp,
}

#[derive(Deserialize)]
struct Stamp {
    total: f64,
}
