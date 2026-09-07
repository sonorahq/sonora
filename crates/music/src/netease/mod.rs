use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::lyrics::lrc;
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, LyricsWord, Voice};

const SOURCE: &str = "NetEase";
const SEARCH: &str = "https://music.163.com/api/search/get";
const LYRIC: &str = "https://music.163.com/api/song/lyric/v1";
const CANDIDATES: usize = 3;
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/nolight132/sonora)"
);

pub struct NetEase {
    http: reqwest::Client,
}

impl NetEase {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn lyric(&self, id: u64) -> Result<Sheet> {
        let response = self
            .http
            .get(LYRIC)
            .query(&[("id", id.to_string().as_str()), ("cp", "false")])
            .query(&[
                ("lv", "0"),
                ("tv", "0"),
                ("rv", "0"),
                ("kv", "0"),
                ("yv", "0"),
                ("ytv", "0"),
                ("yrv", "0"),
            ])
            .header("User-Agent", AGENT)
            .header("Referer", "https://music.163.com")
            .send()
            .await
            .context("cannot reach netease")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("netease answered with status {status}");
        }
        response
            .json()
            .await
            .context("cannot read the netease lyric response")
    }
}

impl Default for NetEase {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct SearchAnswer {
    result: Option<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    #[serde(default)]
    songs: Vec<Song>,
}

#[derive(Deserialize)]
struct Song {
    id: u64,
    name: String,
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    artists: Vec<Named>,
    album: Option<Named>,
}

#[derive(Deserialize)]
struct Named {
    name: Option<String>,
}

#[derive(Deserialize)]
struct Sheet {
    lrc: Option<Verse>,
    yrc: Option<Verse>,
    #[serde(default, rename = "pureMusic")]
    pure_music: bool,
}

#[derive(Deserialize)]
struct Verse {
    lyric: Option<String>,
}

#[async_trait]
impl LyricsProvider for NetEase {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let wanted = format!("{} {}", query.title, query.artist);
        let response = self
            .http
            .get(SEARCH)
            .query(&[("s", wanted.as_str()), ("type", "1"), ("limit", "5")])
            .header("User-Agent", AGENT)
            .header("Referer", "https://music.163.com")
            .send()
            .await
            .context("cannot reach netease")?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("netease answered with status {status}");
        }
        let answer: SearchAnswer = response
            .json()
            .await
            .context("cannot read the netease search response")?;
        let songs = answer.result.map(|result| result.songs).unwrap_or_default();

        let mut tasks = JoinSet::new();
        for song in shortlist(songs, query.duration) {
            let netease = Self {
                http: self.http.clone(),
            };
            tasks.spawn(async move {
                let sheet = netease
                    .lyric(song.id)
                    .await
                    .inspect_err(|error| {
                        log::warn!("lyrics: netease did not hand over {}: {error:#}", song.id)
                    })
                    .ok()?;
                hit(&song, &sheet)
            });
        }

        let mut hits = Vec::new();
        while let Some(found) = tasks.join_next().await {
            hits.extend(found.ok().flatten());
        }
        Ok(hits)
    }
}

fn shortlist(songs: Vec<Song>, duration: Duration) -> Vec<Song> {
    let mut songs = songs;
    songs.sort_by_key(|song| {
        Duration::from_millis(song.duration)
            .as_secs()
            .abs_diff(duration.as_secs())
    });
    songs.truncate(CANDIDATES);
    songs
}

fn hit(song: &Song, sheet: &Sheet) -> Option<LyricsHit> {
    let lyric = |verse: &Option<Verse>| {
        verse
            .as_ref()
            .and_then(|verse| verse.lyric.clone())
            .filter(|text| !text.trim().is_empty())
    };
    let lines = lyric(&sheet.yrc)
        .map(|yrc| parse_yrc(&yrc))
        .filter(|lines| !lines.is_empty())
        .or_else(|| {
            lyric(&sheet.lrc)
                .map(|text| lrc::parse(&text))
                .filter(|lines| !lines.is_empty())
        });
    let quiet = sheet.pure_music
        || lines
            .as_deref()
            .is_some_and(crate::lyrics::sheet::instrumental);
    let lyrics = match (lines, quiet) {
        (_, true) => Lyrics::plain(""),
        (Some(mut lines), false) => {
            let artists: Vec<String> = song
                .artists
                .iter()
                .filter_map(|artist| artist.name.clone())
                .collect();
            if !crate::lyrics::sheet::headed(&mut lines, &song.name, &artists) {
                return None;
            }
            Lyrics::Synced {
                lines: lines.into(),
            }
        }
        (None, false) => return None,
    };

    Some(LyricsHit {
        source: SOURCE,
        trust: 0,
        lyrics,
        instrumental: quiet,
        fallback: false,
        title: song.name.clone(),
        artist: song
            .artists
            .iter()
            .filter_map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        album: song.album.as_ref().and_then(|album| album.name.clone()),
        duration: (song.duration > 0).then(|| Duration::from_millis(song.duration)),
        writers: [&sheet.yrc, &sheet.lrc]
            .into_iter()
            .filter_map(lyric)
            .flat_map(|text| writers(&text))
            .fold(Vec::new(), |mut writers, name| {
                if !writers.contains(&name) {
                    writers.push(name);
                }
                writers
            }),
    })
}

#[derive(Deserialize)]
struct Credit {
    #[serde(default)]
    c: Vec<Piece>,
}

#[derive(Deserialize)]
struct Piece {
    tx: Option<String>,
}

fn writers(text: &str) -> Vec<String> {
    let mut writers = Vec::new();
    for line in text.lines().filter(|line| line.starts_with('{')) {
        let Ok(credit) = serde_json::from_str::<Credit>(line) else {
            continue;
        };
        let credit: String = credit.c.into_iter().filter_map(|piece| piece.tx).collect();
        let Some((label, names)) = credit.split_once(':').or_else(|| credit.split_once('：'))
        else {
            continue;
        };
        if !label.contains("作词") && !label.contains("作曲") {
            continue;
        }
        for name in names.split('/') {
            let name = name.trim().to_owned();
            if !name.is_empty() && !writers.contains(&name) {
                writers.push(name);
            }
        }
    }
    writers
}

fn parse_yrc(yrc: &str) -> Vec<LyricsLine> {
    let mut lines: Vec<LyricsLine> = yrc.lines().filter_map(read_yrc).collect();
    lrc::normalize(&mut lines);
    lines
}

fn read_yrc(line: &str) -> Option<LyricsLine> {
    let (header, rest) = line.strip_prefix('[')?.split_once(']')?;
    let (start, span) = pair_of(header)?;

    let mut words = Vec::new();
    let mut text = String::new();
    let mut rest = rest;
    while let Some(open) = rest.find('(') {
        let tail = &rest[open + 1..];
        let Some(shut) = tail.find(')') else { break };
        match stamp_of(&tail[..shut]) {
            Some((at, length)) => {
                grow(&mut words, &mut text, &rest[..open]);
                rest = &tail[shut + 1..];
                let spoken = rest.find('(').map(|next| &rest[..next]).unwrap_or(rest);
                words.push(LyricsWord {
                    start: at,
                    end: at + length,
                    text: spoken.to_owned(),
                });
                text.push_str(spoken);
                rest = &rest[spoken.len()..];
            }
            None => {
                grow(&mut words, &mut text, &rest[..open + shut + 2]);
                rest = &tail[shut + 1..];
            }
        }
    }
    grow(&mut words, &mut text, rest);

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

fn grow(words: &mut [LyricsWord], text: &mut String, tail: &str) {
    if tail.is_empty() {
        return;
    }
    text.push_str(tail);
    if let Some(last) = words.last_mut() {
        last.text.push_str(tail);
    }
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
    let start: u64 = parts.next()?.trim().parse().ok()?;
    let span: u64 = parts.next()?.trim().parse().ok()?;
    parts.next()?;
    Some((Duration::from_millis(start), Duration::from_millis(span)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_yrc_line() {
        let lines = parse_yrc(
            "{\"t\":0,\"c\":[{\"tx\":\"credits\"}]}\n[27360,1290](27360,240,0)I've (27600,90,0)been (27690,360,0)tryna (28050,600,0)call\n",
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start, Duration::from_millis(27_360));
        assert_eq!(lines[0].end, Some(Duration::from_millis(28_650)));
        assert_eq!(lines[0].text, "I've been tryna call");

        let words = lines[0].words.as_ref().expect("the line is worded");
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].text, "I've ");
        assert_eq!(words[1].start, Duration::from_millis(27_600));
        assert_eq!(words[3].end, Duration::from_millis(28_650));
    }

    #[test]
    fn credit_headers_name_the_writers() {
        let text = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"},{\"tx\":\"Abel Tesfaye\"},{\"tx\":\"/\"},{\"tx\":\"Max Martin\"}]}\n{\"t\":1,\"c\":[{\"tx\":\"作曲: \"},{\"tx\":\"Max Martin\"}]}\n{\"t\":2,\"c\":[{\"tx\":\"制作人: \"},{\"tx\":\"Oscar Holter\"}]}\n[1000,2000](1000,500,0)la\n";

        assert_eq!(
            writers(text),
            vec!["Abel Tesfaye".to_owned(), "Max Martin".to_owned()]
        );
    }

    #[test]
    fn untimed_parentheses_become_a_background_lane() {
        let lines = parse_yrc("[1000,2000](1000,500,0)la （la） (1500,500,0)again\n");

        assert_eq!(lines[0].text, "la again");
        let words = lines[0].words.as_ref().expect("the line is worded");
        assert_eq!(words[0].text.trim(), "la");
        assert_eq!(lines[0].secondary[0].text, "(la)");
        assert_eq!(lines[0].secondary[0].start, Duration::from_millis(1000));
    }

    #[test]
    fn the_closest_durations_make_the_shortlist() {
        let song = |id: u64, duration: u64| Song {
            id,
            name: String::new(),
            duration,
            artists: Vec::new(),
            album: None,
        };
        let songs = vec![
            song(1, 100_000),
            song(2, 263_000),
            song(3, 262_000),
            song(4, 500_000),
            song(5, 264_000),
        ];

        let picked = shortlist(songs, Duration::from_secs(263));
        let ids: Vec<u64> = picked.iter().map(|song| song.id).collect();
        assert_eq!(ids, vec![2, 3, 5]);
    }
}
