use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use crate::jellyfin::auth::{self, Credentials};
use crate::jellyfin::{JellyfinClient, NAME, wire};
use crate::lyrics::lrc;
use crate::{Lyrics, LyricsHit, LyricsProvider, LyricsQuery};

const SLUG: &str = "jellyfin";

/// The same tier as lyrics read from a local file, since these are most often the sidecar
/// files and tags of the user's own music.
const TRUST: u32 = 100;

/// Asks the connected Jellyfin server for the lyrics of one of its tracks, word-timed where the
/// file times its words. Any other track, or no stored sign-in, gets no answer.
pub struct JellyfinLyrics {
    client: Mutex<Option<(Credentials, JellyfinClient)>>,
}

impl JellyfinLyrics {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
        }
    }

    /// A client for the stored sign-in, kept until the sign-in changes. This provider has no
    /// session handle, so it reads the same file the session was restored from.
    fn client(&self) -> Option<JellyfinClient> {
        let stored = auth::load()?;
        let mut held = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((known, client)) = held.as_ref()
            && known.token == stored.token
            && known.server == stored.server
        {
            return Some(client.clone());
        }
        let client = JellyfinClient::new(&stored);
        *held = Some((stored, client.clone()));
        Some(client)
    }
}

impl Default for JellyfinLyrics {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsProvider for JellyfinLyrics {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(id) = query.id_for(SLUG) else {
            return Ok(Vec::new());
        };
        let Some(client) = self.client() else {
            return Ok(Vec::new());
        };
        let Some(answer) = client.lyrics(id).await? else {
            return Ok(Vec::new());
        };
        let lyrics = match wire::synced(&answer) {
            Some(mut lines) => {
                lrc::normalize(&mut lines);
                Lyrics::Synced {
                    lines: lines.into(),
                }
            }
            None => Lyrics::plain(
                answer
                    .lyrics
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        };
        if lyrics.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![LyricsHit {
            source: NAME,
            trust: TRUST,
            lyrics,
            instrumental: false,
            title: query.title.clone(),
            artist: query.artist.clone(),
            album: query.album.clone(),
            duration: (!query.duration.is_zero()).then_some(query.duration),
            writers: Vec::new(),
        }])
    }
}
