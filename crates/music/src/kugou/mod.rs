use std::io::Read as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::lyrics::sheet;
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, LyricsWord, Voice};

const SOURCE: &str = "Kugou";
const SEARCH: &str = "https://mobiles.kugou.com/api/v3/search/song";
const CANDIDATES: &str = "https://lyrics.kugou.com/search";
const DOWNLOAD: &str = "https://lyrics.kugou.com/download";
const SONGS: usize = 4;
const PAGE: usize = 20;
const SHEETS: usize = 2;
const CIPHER: [u8; 16] = [
    0x40, 0x47, 0x61, 0x77, 0x5e, 0x32, 0x74, 0x47, 0x51, 0x36, 0x31, 0x2d, 0xce, 0xd2, 0x6e, 0x69,
];
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/nolight132/sonora)"
);

pub struct Kugou {
    http: reqwest::Client,
}

impl Kugou {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn songs(&self, wanted: &str) -> Result<Vec<Song>> {
        let keyword = utf8_percent_encode(wanted, NON_ALPHANUMERIC);
        let response = self
            .http
            .get(format!(
                "{SEARCH}?format=json&keyword={keyword}&page=1&pagesize={PAGE}&showtype=1"
            ))
            .header("User-Agent", AGENT)
            .send()
            .await
            .context("cannot reach kugou")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("kugou answered with status {status}");
        }
        let answer: SearchAnswer = response
            .json()
            .await
            .context("cannot read the kugou search response")?;
        Ok(answer.data.map(|data| data.info).unwrap_or_default())
    }

    async fn candidates(&self, song: &Song) -> Result<Vec<Candidate>> {
        let response = self
            .http
            .get(CANDIDATES)
            .query(&[
                ("ver", "1"),
                ("man", "yes"),
                ("client", "mobi"),
                ("hash", song.hash.as_str()),
                ("duration", &(song.duration * 1000).to_string()),
            ])
            .header("User-Agent", AGENT)
            .send()
            .await
            .context("cannot reach the kugou lyric index")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("the kugou lyric index answered with status {status}");
        }
        let answer: CandidateAnswer = response
            .json()
            .await
            .context("cannot read the kugou candidate response")?;
        Ok(answer.candidates)
    }

    async fn krc(&self, candidate: &Candidate) -> Result<String> {
        let response = self
            .http
            .get(DOWNLOAD)
            .query(&[
                ("ver", "1"),
                ("client", "pc"),
                ("id", candidate.id.to_string().as_str()),
                ("accesskey", candidate.accesskey.as_str()),
                ("fmt", "krc"),
                ("charset", "utf8"),
            ])
            .header("User-Agent", AGENT)
            .send()
            .await
            .context("cannot reach the kugou lyric download")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("the kugou lyric download answered with status {status}");
        }
        let answer: Download = response
            .json()
            .await
            .context("cannot read the kugou download response")?;
        let content = answer
            .content
            .filter(|content| !content.is_empty())
            .context("kugou handed over an empty sheet")?;
        decode(&content)
    }
}

impl Default for Kugou {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct SearchAnswer {
    data: Option<SearchData>,
}

#[derive(Deserialize)]
struct SearchData {
    #[serde(default)]
    info: Vec<Song>,
}

#[derive(Clone, Deserialize)]
struct Song {
    hash: String,
    #[serde(default)]
    duration: u64,
    #[serde(default, rename = "songname")]
    name: String,
    #[serde(default, rename = "singername")]
    singer: String,
    #[serde(default, rename = "album_name")]
    album: String,
}

#[derive(Deserialize)]
struct CandidateAnswer {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Clone, Deserialize)]
struct Candidate {
    id: String,
    accesskey: String,
    #[serde(default)]
    krctype: u8,
}

#[derive(Deserialize)]
struct Download {
    content: Option<String>,
}

#[async_trait]
impl LyricsProvider for Kugou {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let wanted = format!("{} {}", query.artist, query.title);
        let songs = shortlist(self.songs(&wanted).await?, query.duration);

        let mut tasks = JoinSet::new();
        for song in songs {
            let kugou = Self {
                http: self.http.clone(),
            };
            let title = query.title.clone();
            tasks.spawn(async move {
                let candidates = kugou
                    .candidates(&song)
                    .await
                    .inspect_err(|error| {
                        log::warn!("lyrics: kugou did not list {}: {error:#}", song.hash)
                    })
                    .ok()?;
                let mut hits = Vec::new();
                for candidate in worded_first(candidates) {
                    let Ok(krc) = kugou.krc(&candidate).await.inspect_err(|error| {
                        log::warn!(
                            "lyrics: kugou did not hand over {}: {error:#}",
                            candidate.id
                        )
                    }) else {
                        continue;
                    };
                    hits.extend(hit(&song, &krc, &title));
                }
                Some(hits)
            });
        }

        let mut hits = Vec::new();
        while let Some(found) = tasks.join_next().await {
            hits.extend(found.ok().flatten().unwrap_or_default());
        }
        Ok(hits)
    }
}

fn shortlist(songs: Vec<Song>, duration: Duration) -> Vec<Song> {
    let mut songs = songs;
    songs.sort_by_key(|song| {
        Duration::from_secs(song.duration)
            .as_secs()
            .abs_diff(duration.as_secs())
    });
    songs.truncate(SONGS);
    songs
}

fn worded_first(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut candidates = candidates;
    candidates.sort_by_key(|candidate| u8::from(candidate.krctype != 2));
    candidates.truncate(SHEETS);
    candidates
}

fn decode(content: &str) -> Result<String> {
    let packed = STANDARD
        .decode(content)
        .context("cannot decode the kugou sheet")?;
    let body = packed
        .get(4..)
        .context("the kugou sheet is missing its header")?;
    let unmasked: Vec<u8> = body
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ CIPHER[index % CIPHER.len()])
        .collect();
    let mut text = String::new();
    flate2::read::ZlibDecoder::new(unmasked.as_slice())
        .read_to_string(&mut text)
        .context("cannot inflate the kugou sheet")?;
    Ok(text)
}

fn hit(song: &Song, krc: &str, title: &str) -> Option<LyricsHit> {
    let mut lines = parse(krc);
    let artists: Vec<String> = song
        .singer
        .split(['、', ',', '&'])
        .map(str::trim)
        .filter(|artist| !artist.is_empty())
        .map(str::to_owned)
        .collect();
    if !sheet::headed(&mut lines, title, &artists) {
        return None;
    }
    let quiet = sheet::instrumental(&lines);
    crate::lyrics::lrc::normalize(&mut lines);
    if lines.is_empty() && !quiet {
        return None;
    }

    Some(LyricsHit {
        source: SOURCE,
        trust: 0,
        lyrics: match quiet {
            true => Lyrics::plain(""),
            false => Lyrics::Synced {
                lines: lines.into(),
            },
        },
        instrumental: quiet,
        fallback: false,
        title: song.name.clone(),
        artist: song.singer.clone(),
        album: (!song.album.is_empty()).then(|| song.album.clone()),
        duration: (song.duration > 0).then(|| Duration::from_secs(song.duration)),
        writers: Vec::new(),
    })
}

fn parse(krc: &str) -> Vec<LyricsLine> {
    krc.lines().filter_map(read).collect()
}

fn read(line: &str) -> Option<LyricsLine> {
    let (header, rest) = line.strip_prefix('[')?.split_once(']')?;
    let (start, span) = pair_of(header)?;

    let mut words = Vec::new();
    let mut text = String::new();
    let mut rest = rest;
    while let Some(open) = rest.find('<') {
        let tail = &rest[open + 1..];
        let Some(shut) = tail.find('>') else { break };
        if !text.is_empty() || open > 0 {
            text.push_str(&rest[..open]);
        }
        let stamp = stamp_of(&tail[..shut]);
        rest = &tail[shut + 1..];
        let spoken = rest.find('<').map(|next| &rest[..next]).unwrap_or(rest);
        if let Some((at, length)) = stamp {
            words.push(LyricsWord {
                start: start + at,
                end: start + at + length,
                text: spoken.to_owned(),
            });
        }
        text.push_str(spoken);
        rest = &rest[spoken.len()..];
    }
    text.push_str(rest);

    let text = text.trim().to_owned();
    if text.is_empty() {
        return None;
    }
    Some(LyricsLine {
        start,
        end: Some(start + span),
        words: (!words.is_empty()).then_some(words),
        text,
        romanized: None,
        secondary: Vec::new(),
        voice: Voice::Lead,
    })
}

fn pair_of(header: &str) -> Option<(Duration, Duration)> {
    let (start, span) = header.split_once(',')?;
    Some((
        Duration::from_millis(start.trim().parse().ok()?),
        Duration::from_millis(span.trim().parse().ok()?),
    ))
}

fn stamp_of(stamp: &str) -> Option<(Duration, Duration)> {
    let mut parts = stamp.split(',');
    let at = parts.next()?.trim().parse().ok()?;
    let length = parts.next()?.trim().parse().ok()?;
    Some((Duration::from_millis(at), Duration::from_millis(length)))
}
