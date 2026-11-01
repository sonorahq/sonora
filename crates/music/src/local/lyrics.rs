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

/// Extensions a lyrics file beside the track may carry, in the order they are tried. TTML comes
/// first since it can time every word. Linux filesystems tell the cases apart.
const SIDECARS: [&str; 4] = ["ttml", "TTML", "lrc", "LRC"];

/// Reads the lyrics of a local file: a `.ttml` or `.lrc` file beside it, else what its own tags
/// carry.
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

/// A lyrics file of the same name wins, since someone put it there for this track; the tags are
/// the fallback when there is none, or when it holds no line that decodes.
fn read(path: &Path) -> Result<Option<Lyrics>> {
    match sidecar(path) {
        Some(lyrics) => Ok(Some(lyrics)),
        None => tags::lyrics(path),
    }
}

/// Lyrics from `Song.ttml` or `Song.lrc` beside `Song.flac`. A file that cannot be read or has no
/// valid line is logged and skipped rather than failing the lookup, so the next file and then the
/// tags still get their turn.
fn sidecar(path: &Path) -> Option<Lyrics> {
    SIDECARS.iter().find_map(|extension| {
        let sidecar_path = path.with_extension(extension);
        match fs::read_to_string(&sidecar_path) {
            Ok(text) if extension.eq_ignore_ascii_case("ttml") => {
                tags::ttml_lyrics(&sidecar_path, &text)
            }
            Ok(text) => lrc_lyrics(&sidecar_path, &text),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                log::warn!("lyrics: cannot read {}: {error}", sidecar_path.display());
                None
            }
        }
    })
}

/// Timed lyrics from an `.lrc` file, or none when it holds no valid line.
fn lrc_lyrics(lrc_path: &Path, text: &str) -> Option<Lyrics> {
    let lines = lrc::parse(text.trim_start_matches('\u{feff}'));
    if lines.is_empty() {
        log::warn!("lyrics: {} holds no valid LRC line", lrc_path.display());
        return None;
    }
    Some(Lyrics::Synced {
        lines: lines.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::local::tags::tests::flac;

    const TIMED: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="en"><head><metadata><ttm:agent type="person" xml:id="v1"/></metadata></head><body dur="0:10.000"><div begin="1.000" end="6.000"><p begin="1.000" end="3.500" ttm:agent="v1">So here I am</p><p begin="3.500" end="6.000" ttm:agent="v1">Doing it again</p></div></body></tt>"#;

    fn track(name: &str, comments: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Song.flac");
        flac(&path, comments);
        path
    }

    fn assert_timed(lyrics: Option<Lyrics>) {
        let Some(Lyrics::Synced { lines }) = lyrics else {
            panic!("expected timed lyrics, got {lyrics:?}");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "So here I am");
        assert_eq!(lines[0].start, Duration::from_millis(1000));
        assert_eq!(lines[1].text, "Doing it again");
        assert_eq!(lines[1].start, Duration::from_millis(3500));
    }

    #[test]
    fn a_ttml_file_beside_the_track_is_decoded() {
        let path = track("sonora-local-ttml-sidecar", &[]);
        fs::write(path.with_extension("ttml"), TIMED).unwrap();

        assert_timed(read(&path).unwrap());
    }

    #[test]
    fn ttml_in_the_lyrics_tag_is_decoded() {
        let comment = format!("LYRICS={TIMED}");
        let path = track("sonora-local-ttml-tag", &[&comment]);

        assert_timed(read(&path).unwrap());
    }

    #[test]
    fn a_broken_ttml_file_falls_back_to_the_tags() {
        let path = track("sonora-local-ttml-broken", &["LYRICS=From the tags"]);
        fs::write(path.with_extension("ttml"), "<tt><body><p begin=\"1.0\">").unwrap();

        let Some(Lyrics::Plain { text, .. }) = read(&path).unwrap() else {
            panic!("expected the plain lyrics of the tags");
        };
        assert_eq!(text, "From the tags");
    }

    #[test]
    fn a_ttml_file_without_lines_gives_nothing() {
        let path = track("sonora-local-ttml-empty", &[]);
        fs::write(path.with_extension("ttml"), "<tt><body></body></tt>").unwrap();

        assert!(read(&path).unwrap().is_none());
    }
}
