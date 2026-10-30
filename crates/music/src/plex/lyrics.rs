use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use crate::lyrics::lrc;
use crate::plex::auth::{self, Credentials};
use crate::plex::{NAME, PlexClient};
use crate::{Lyrics, LyricsHit, LyricsProvider, LyricsQuery};

const SLUG: &str = "plex";

/// The same tier as lyrics read from a local file, since these are most often the tags of the
/// user's own files or what Plex fetched for them.
const TRUST: u32 = 100;

type Identity = (String, String, String);

/// Asks the connected Plex server for the lyrics of one of its tracks. Any other track, or no
/// stored sign-in, gets no answer.
pub struct PlexLyrics {
    client: Mutex<Option<(Identity, PlexClient)>>,
}

impl PlexLyrics {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
        }
    }

    /// A client for the stored sign-in, kept until the credentials change. This provider has no
    /// session handle, so it reads the same file the session was restored from.
    fn client(&self) -> Result<Option<PlexClient>> {
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
        let client = PlexClient::new(stored, auth::client_id()?)?;
        *held = Some((identity, client.clone()));
        Ok(Some(client))
    }
}

impl Default for PlexLyrics {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsProvider for PlexLyrics {
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
        let Some(lyrics) = client
            .lyrics_text(id)
            .await?
            .and_then(|text| sheet(&text))
            .filter(|lyrics| !lyrics.is_empty())
        else {
            return Ok(Vec::new());
        };
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

fn identity(stored: &Credentials) -> Identity {
    (
        stored.server.id.clone(),
        stored.user.id.clone(),
        stored.server.token.clone(),
    )
}

/// The lyrics a text holds: timed when it reads as LRC, plain otherwise.
fn sheet(text: &str) -> Option<Lyrics> {
    let text = text.trim_start_matches('\u{feff}');
    let lines = lrc::parse(text);
    match lines.is_empty() {
        false => Some(Lyrics::Synced {
            lines: lines.into(),
        }),
        true => (!text.trim().is_empty()).then(|| Lyrics::plain(text)),
    }
}
