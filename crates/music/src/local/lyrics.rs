use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::lyrics::{LOCAL, lrc};
use crate::{Lyrics, LyricsHit, LyricsProvider, LyricsQuery};

use super::{tags, wire};

/// Wins a tie against most services without outranking a better kind of sheet.
const TRUST: u32 = 100;

/// Extensions a lyrics file beside the track may carry. Linux filesystems tell the two apart.
const SIDECARS: [&str; 2] = ["lrc", "LRC"];

/// Reads the lyrics of a local file: an `.lrc` file beside it, else what its own tags carry.
/// Any other track gets no answer.
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
        let read = tokio::task::spawn_blocking(move || read(&path));
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
            origin: None,
            contributors: Vec::new(),
            expires: None,
        }])
    }
}

/// The `.lrc` file of the same name wins, since someone put it there for this track; the tags are
/// the fallback when there is none, or when it holds no timed line.
fn read(path: &Path) -> Result<Option<Lyrics>> {
    match sidecar(path) {
        Some(lyrics) => Ok(Some(lyrics)),
        None => tags::lyrics(path),
    }
}

/// Timed lyrics from `Song.lrc` beside `Song.flac`. A file that cannot be read or has no valid
/// line is logged and skipped rather than failing the lookup, so the tags still get their turn.
fn sidecar(path: &Path) -> Option<Lyrics> {
    let (lrc_path, text) = SIDECARS.iter().find_map(|extension| {
        let lrc_path = path.with_extension(extension);
        match fs::read_to_string(&lrc_path) {
            Ok(text) => Some((lrc_path, text)),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                log::warn!("lyrics: cannot read {}: {error}", lrc_path.display());
                None
            }
        }
    })?;
    let lines = lrc::parse(text.trim_start_matches('\u{feff}'));
    if lines.is_empty() {
        log::warn!("lyrics: {} holds no valid LRC line", lrc_path.display());
        return None;
    }
    Some(Lyrics::Synced {
        lines: lines.into(),
    })
}
