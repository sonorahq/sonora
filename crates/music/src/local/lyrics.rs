use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use async_trait::async_trait;

use crate::lyrics::{LOCAL, lrc, ttml};
use crate::{Lyrics, LyricsHit, LyricsProvider, LyricsQuery};

use super::{tags, wire};

/// Wins a tie against most services without outranking a better kind of sheet.
const TRUST: u32 = 100;

/// Extensions a TTML file beside the track may carry. Linux filesystems tell the two apart.
const TTML: [&str; 2] = ["ttml", "TTML"];

/// Extensions an LRC file beside the track may carry, looked for after the TTML ones.
const LRC: [&str; 2] = ["lrc", "LRC"];

/// Reads the lyrics of a local file: a `.ttml` or `.lrc` file beside it, else what its own tags
/// carry. Any other track gets no answer.
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
        }])
    }
}

/// A `.ttml` file of the same name wins, since it can time each word, then an `.lrc` file; both
/// were put there for this track. The tags are the fallback when neither holds a timed line.
fn read(path: &Path) -> Result<Option<Lyrics>> {
    match from_ttml(path).or_else(|| from_lrc(path)) {
        Some(lyrics) => Ok(Some(lyrics)),
        None => tags::lyrics(path),
    }
}

/// Timed lyrics from `Song.ttml` beside `Song.flac`, word by word when its spans carry times. A
/// file that cannot be parsed or has no timed line is logged and skipped, so the `.lrc` and the
/// tags still get their turn.
fn from_ttml(path: &Path) -> Option<Lyrics> {
    let (file, text) = sidecar(path, &TTML)?;
    let sheet = ttml::parse(&text)
        .inspect_err(|error| log::warn!("lyrics: cannot read {}: {error:#}", file.display()))
        .ok()?;
    if sheet.lyrics.is_empty() || !sheet.lyrics.synced() {
        log::warn!("lyrics: {} holds no timed TTML line", file.display());
        return None;
    }
    Some(sheet.lyrics)
}

/// Timed lyrics from `Song.lrc` beside `Song.flac`. A file with no valid line is logged and
/// skipped, so the tags still get their turn.
fn from_lrc(path: &Path) -> Option<Lyrics> {
    let (file, text) = sidecar(path, &LRC)?;
    let lines = lrc::parse(&text);
    if lines.is_empty() {
        log::warn!("lyrics: {} holds no valid LRC line", file.display());
        return None;
    }
    Some(Lyrics::Synced {
        lines: lines.into(),
    })
}

/// The first file beside the track with one of these extensions that reads, with any BOM
/// stripped. A missing file is quiet; any other read error is logged and that format skipped.
fn sidecar(path: &Path, extensions: &[&str]) -> Option<(PathBuf, String)> {
    extensions.iter().find_map(|extension| {
        let file = path.with_extension(extension);
        match fs::read_to_string(&file) {
            Ok(text) => Some((file, text.trim_start_matches('\u{feff}').to_owned())),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                log::warn!("lyrics: cannot read {}: {error}", file.display());
                None
            }
        }
    })
}
