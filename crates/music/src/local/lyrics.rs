use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::lyrics::LOCAL;
use crate::{LyricsHit, LyricsProvider, LyricsQuery};

use super::{tags, wire};

/// Wins a tie against most services without outranking a better kind of sheet.
const TRUST: u32 = 100;

/// Reads the lyrics a local file carries in its own tags. Any other track gets no answer.
pub struct LocalLyrics;

#[async_trait]
impl LyricsProvider for LocalLyrics {
    fn name(&self) -> &'static str {
        LOCAL
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        let Some(path) = query
            .track
            .as_ref()
            .and_then(|track| wire::path_from_track_id(&track.id))
        else {
            return Ok(Vec::new());
        };
        let path = path.to_path_buf();
        let read = tokio::task::spawn_blocking(move || tags::lyrics(&path));
        let Some(lyrics) = read.await.context("local lyrics task panicked")?? else {
            return Ok(Vec::new());
        };
        Ok(vec![LyricsHit {
            source: LOCAL,
            trust: TRUST,
            lyrics,
            instrumental: false,
            title: query.title.clone(),
            artist: query.artist.clone(),
            album: query.album.clone(),
            duration: Some(query.duration),
            writers: Vec::new(),
        }])
    }
}
