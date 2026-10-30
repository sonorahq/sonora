use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opensubsonic::data::{Cue, CueLine, StructuredLyrics};

use crate::lyrics::lrc;
use crate::subsonic::auth::{self, Credentials};
use crate::subsonic::{NAME, SubsonicClient};
use crate::{Lyrics, LyricsHit, LyricsLine, LyricsProvider, LyricsQuery, LyricsWord, Voice};

const SLUG: &str = "subsonic";

/// The same tier as lyrics read from a local file, since these are most often the tags of the
/// user's own files.
const TRUST: u32 = 100;

type Identity = (String, String, String);

/// Asks the connected Subsonic server for the lyrics of one of its tracks. Any other track, or no
/// stored sign-in, gets no answer.
pub struct SubsonicLyrics {
    client: Mutex<Option<(Identity, SubsonicClient)>>,
}

impl SubsonicLyrics {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
        }
    }

    /// A client for the stored sign-in, kept until the credentials change. This provider has no
    /// session handle, so it reads the same file the session was restored from.
    fn client(&self) -> Result<Option<SubsonicClient>> {
        let Some(stored) = auth::load() else {
            return Ok(None);
        };
        let identity = identity(&stored);
        let mut held = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((known, client)) = held.as_ref()
            && *known == identity
        {
            return Ok(Some(client.clone()));
        }
        let signature = stored
            .signature
            .clone()
            .unwrap_or_else(|| auth::sign(&stored.username, &stored.password));
        let client =
            SubsonicClient::new(stored.server, stored.username, stored.password, &signature)?;
        *held = Some((identity, client.clone()));
        Ok(Some(client))
    }
}

impl Default for SubsonicLyrics {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsProvider for SubsonicLyrics {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(id) = query.id_for(SLUG) else {
            return Ok(Vec::new());
        };
        let Some(client) = self.client()? else {
            return Ok(Vec::new());
        };
        let sheets = match client.lyrics_by_song(id).await {
            Ok(list) => structured(list.structured_lyrics),
            Err(error) => {
                log::info!("subsonic: no structured lyrics, asking by name: {error:#}");
                let legacy = client.lyrics_by_name(&query.artist, &query.title).await?;
                legacy
                    .value
                    .and_then(|text| legacy_sheet(&text))
                    .into_iter()
                    .collect()
            }
        };
        Ok(sheets
            .into_iter()
            .map(|lyrics| LyricsHit {
                source: NAME,
                trust: TRUST,
                lyrics,
                instrumental: false,
                title: query.title.clone(),
                artist: query.artist.clone(),
                album: query.album.clone(),
                duration: (!query.duration.is_zero()).then_some(query.duration),
                writers: Vec::new(),
            })
            .collect())
    }
}

fn identity(stored: &Credentials) -> Identity {
    (
        stored.server.clone(),
        stored.username.clone(),
        stored.password.clone(),
    )
}

/// The main-lane entries of a structured answer, each as the lyrics it holds. Translations and
/// pronunciations are not shown.
fn structured(entries: Vec<StructuredLyrics>) -> Vec<Lyrics> {
    entries
        .into_iter()
        .filter(|entry| matches!(entry.kind.as_deref(), None | Some("main")))
        .filter_map(sheet)
        .filter(|lyrics| !lyrics.is_empty())
        .collect()
}

fn sheet(entry: StructuredLyrics) -> Option<Lyrics> {
    let offset = entry.offset.unwrap_or(0.);
    if entry.synced {
        let mut lines: Vec<LyricsLine>;
        let cue_lines: Option<&Vec<CueLine>> = entry.cue_line.as_ref();

        if cue_lines.is_some_and(|v| !v.is_empty()) {
            let agents = entry.agents.as_ref();
            let agents_exists: bool = agents.is_some_and(|a| !a.is_empty());

            lines = cue_lines?
                .iter()
                .filter_map(|cue| {
                    let voice: Voice;
                    if agents_exists && cue.agent_id.is_some() {
                        let agent = agents?
                            .iter()
                            .find(|&agent| cue.agent_id.to_owned().unwrap() == agent.id);

                        voice = if agent?.role == "main" {
                            Voice::Lead
                        } else {
                            Voice::Counter
                        };
                    } else {
                        voice = Voice::Lead;
                    }

                    Some(LyricsLine {
                        start: shifted(cue.start?, offset),
                        end: cue.end.map(|value| shifted(value, offset)),
                        text: cue.value.clone()?.trim().to_owned(),
                        romanized: None,
                        words: cues(&cue.cue, offset),
                        secondary: Vec::new(),
                        voice,
                    })
                })
                .collect();
        } else {
            lines = entry
                .line
                .iter()
                .filter_map(|line| {
                    Some(LyricsLine {
                        start: shifted(line.start?, offset),
                        end: None,
                        text: line.value.trim().to_owned(),
                        romanized: None,
                        words: None,
                        secondary: Vec::new(),
                        voice: Voice::Lead,
                    })
                })
                .collect();
        }
        lrc::normalize(&mut lines);
        if !lines.is_empty() {
            return Some(Lyrics::Synced {
                lines: lines.into(),
            });
        }
    }
    let text = entry
        .line
        .iter()
        .map(|line| line.value.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Lyrics::plain(text))
}

/// A line's start in the player's time. A positive offset shows the lyrics sooner.
fn shifted(start: f64, offset: f64) -> Duration {
    Duration::try_from_secs_f64((start - offset).max(0.) / 1_000.).unwrap_or_default()
}

fn legacy_sheet(text: &str) -> Option<Lyrics> {
    let lines = lrc::parse(text.trim_start_matches('\u{feff}'));
    match lines.is_empty() {
        false => Some(Lyrics::Synced {
            lines: lines.into(),
        }),
        true => (!text.trim().is_empty()).then(|| Lyrics::plain(text)),
    }
}

fn cues(cues: &[Cue], offset: f64) -> Option<Vec<LyricsWord>> {
    if cues.is_empty() {
        return None;
    }

    Some(
        cues.iter()
            .filter_map(|m| {
                Some(LyricsWord {
                    start: shifted(m.start?, offset),
                    end: shifted(m.end?, offset),
                    text: m.value.clone()?,
                })
            })
            .collect(),
    )
}
