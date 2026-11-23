use std::sync::Arc;
use std::time::Duration;

use crate::spotify::session::Session;
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use futures::future::try_join_all;

use super::{auth, search};
use crate::lyrics::{catalog, lrc};
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, LyricsWord, Voice};
use spotifyapis::proto::lyrics::{Line, Sheet, SyncData, SyncType};

const SOURCE: &str = "Spotify";

/// A lyrics-only connection, independent of the selected playback provider.
pub struct SpotifyLyrics {
    account: Arc<auth::Account>,
}

impl SpotifyLyrics {
    pub fn new(config: auth::AuthConfig) -> Self {
        Self::with_account(Arc::new(auth::Account::new(config)))
    }

    pub fn from_env() -> Self {
        Self::new(auth::AuthConfig::from_env())
    }

    pub(super) fn with_account(account: Arc<auth::Account>) -> Self {
        Self { account }
    }
}

#[async_trait]
impl LyricsProvider for SpotifyLyrics {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(session) = self.account.session().await? else {
            return Ok(Vec::new());
        };
        if let Some(id) = query.id_for("spotify") {
            return Ok(lyrics(&session, id)
                .await?
                .filter(|sheet| !sheet.is_empty())
                .map(|sheet| catalog::hit(SOURCE, query, sheet))
                .into_iter()
                .collect());
        }
        let tracks =
            search::lyric_tracks(&session, &format!("{} {}", query.title, query.artist)).await?;
        let candidates = catalog::candidates(SOURCE, query, tracks);
        let session = &session;
        let hits = try_join_all(candidates.into_iter().map(|(id, mut hit)| async move {
            Ok::<_, anyhow::Error>(
                lyrics(session, &id)
                    .await?
                    .filter(|sheet| !sheet.is_empty())
                    .map(|sheet| {
                        hit.lyrics = sheet;
                        hit
                    }),
            )
        }))
        .await?;
        Ok(hits.into_iter().flatten().collect())
    }
}

pub async fn lyrics(session: &Session, track_id: &str) -> Result<Option<Lyrics>> {
    let answer = session
        .api()
        .lyrics(track_id)
        .await
        .map_err(auth::sign_in_error)
        .context("cannot fetch Spotify lyrics")?;
    Ok(answer.and_then(|answer| answer.lyrics).and_then(sheet))
}

fn sheet(sheet: Sheet) -> Option<Lyrics> {
    if sheet.sync_type() == SyncType::Unsynced {
        let text = sheet
            .lines
            .iter()
            .map(|verse| verse.words.trim())
            .filter(|words| !words.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return (!text.trim().is_empty()).then(|| Lyrics::plain(text));
    }

    let mut lines: Vec<LyricsLine> = sheet.lines.iter().filter_map(line).collect();
    lrc::normalize(&mut lines);
    (!lines.is_empty()).then(|| Lyrics::Synced {
        lines: lines.into(),
    })
}

fn line(line: &Line) -> Option<LyricsLine> {
    let text = line.words.trim();
    if text.is_empty() || text == "♪" {
        return None;
    }
    let start = stamp(line.start_time_ms)?;
    let words = segments(&line.words, &line.sync_data);
    Some(LyricsLine {
        start,
        end: stamp(line.end_time_ms).filter(|end| *end > start),
        text: text.to_owned(),
        romanized: None,
        words,
        secondary: Vec::new(),
        voice: Voice::Lead,
    })
}

/// Turns character counts into text spans, preserving line timing if spans are incomplete.
fn segments(text: &str, data: &[SyncData]) -> Option<Vec<LyricsWord>> {
    if data.is_empty() {
        return None;
    }
    let mut chars = text.char_indices().peekable();
    let mut words = Vec::with_capacity(data.len());
    for segment in data {
        let mut remaining = usize::try_from(segment.num_chars).ok()?;
        if remaining == 0 {
            continue;
        }
        let first = chars.peek()?.0;
        while remaining > 0 {
            remaining = remaining.checked_sub(chars.next()?.1.len_utf16())?;
        }
        let last = chars.peek().map_or(text.len(), |&(offset, _)| offset);
        let start = stamp(segment.start_time_ms)?;
        let end = stamp(segment.end_time_ms).filter(|end| *end > start)?;
        words.push(LyricsWord {
            start,
            end,
            text: text.get(first..last)?.to_owned(),
        });
    }
    (!words.is_empty() && chars.all(|(_, ch)| ch.is_whitespace())).then_some(words)
}

fn stamp(millis: i64) -> Option<Duration> {
    Some(Duration::from_millis(millis.try_into().ok()?))
}
