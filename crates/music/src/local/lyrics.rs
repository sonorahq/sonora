use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};

use crate::{Lyrics, lyrics::lrc};

use super::wire;

pub async fn read(track_id: &str) -> Result<Option<Lyrics>> {
    let path = lyrics_path(track_id)?;
    tokio::task::spawn_blocking(move || read_file(&path))
        .await
        .context("local lyrics task panicked")?
}

fn lyrics_path(track_id: &str) -> Result<PathBuf> {
    let track = wire::path_from_track_id(track_id)
        .ok_or_else(|| anyhow!("{track_id} is not a local track id"))?;
    Ok(track.with_extension("lrc"))
}

fn read_file(path: &Path) -> Result<Option<Lyrics>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read local lyrics {}", path.display()));
        }
    };

    let lines = lrc::parse(text.trim_start_matches('\u{feff}'));
    if lines.is_empty() {
        bail!("{} does not contain valid LRC lyrics", path.display());
    }

    Ok(Some(Lyrics::Synced {
        lines: lines.into(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let path = std::env::temp_dir().join(format!(
            "sonora-local-lyrics-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        fs::create_dir_all(&path).expect("scratch directory is created");
        path
    }

    #[test]
    fn derives_lrc_path_from_track_path() {
        let track = Path::new("Music").join("Album").join("Song.flac");
        let id = wire::track_id(&track);

        assert_eq!(
            lyrics_path(&id).unwrap(),
            Path::new("Music").join("Album").join("Song.lrc")
        );
    }

    #[tokio::test]
    async fn loads_same_name_lrc_file() {
        let root = scratch("valid");
        let track = root.join("Song.flac");
        let id = wire::track_id(&track);

        fs::write(
            track.with_extension("lrc"),
            "[00:01.00]First test line\n[00:02.50]Second test line\n",
        )
        .expect("lyrics file is written");

        let found = read(&id)
            .await
            .expect("local lyrics lookup succeeds")
            .expect("local lyrics are found");

        let Lyrics::Synced { lines } = found else {
            panic!("LRC file produces synced lyrics");
        };

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "First test line");
        assert_eq!(lines[1].text, "Second test line");

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }

    #[tokio::test]
    async fn loads_utf8_bom_lrc_file() {
        let root = scratch("bom");
        let track = root.join("Song.flac");
        let id = wire::track_id(&track);

        fs::write(track.with_extension("lrc"), "\u{feff}[00:01.00]BOM test\n")
            .expect("lyrics file is written");

        let found = read(&id)
            .await
            .expect("local lyrics lookup succeeds")
            .expect("local lyrics are found");

        let Lyrics::Synced { lines } = found else {
            panic!("LRC file produces synced lyrics");
        };

        assert_eq!(lines[0].text, "BOM test");

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }

    #[tokio::test]
    async fn missing_lrc_file_returns_none() {
        let root = scratch("missing");
        let track = root.join("Song.flac");
        let id = wire::track_id(&track);

        let found = read(&id)
            .await
            .expect("missing local lyrics are not an error");

        assert!(found.is_none());

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }

    #[tokio::test]
    async fn empty_lrc_file_is_rejected() {
        let root = scratch("empty");
        let track = root.join("Song.flac");
        let id = wire::track_id(&track);

        fs::write(track.with_extension("lrc"), "").expect("lyrics file is written");

        let error = read(&id)
            .await
            .expect_err("empty local lyrics are rejected");

        assert!(
            error
                .to_string()
                .contains("does not contain valid LRC lyrics")
        );

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }

    #[tokio::test]
    async fn lrc_file_without_valid_lines_is_rejected() {
        let root = scratch("malformed");
        let track = root.join("Song.flac");
        let id = wire::track_id(&track);

        fs::write(
            track.with_extension("lrc"),
            "[not-a-time]This is not valid timed LRC\n",
        )
        .expect("lyrics file is written");

        let error = read(&id)
            .await
            .expect_err("malformed local lyrics are rejected");

        assert!(
            error
                .to_string()
                .contains("does not contain valid LRC lyrics")
        );

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }

    #[tokio::test]
    async fn unreadable_lrc_file_is_rejected() {
        let root = scratch("unreadable");
        let track = root.join("Song.flac");
        let id = wire::track_id(&track);

        fs::create_dir(track.with_extension("lrc")).expect("lyrics path is created as a directory");

        let error = read(&id)
            .await
            .expect_err("unreadable local lyrics are rejected");

        assert!(error.to_string().contains("cannot read local lyrics"));

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }

    #[tokio::test]
    async fn updated_lrc_file_is_read_again() {
        let root = scratch("updated");
        let track = root.join("Song.flac");
        let lyrics = track.with_extension("lrc");
        let id = wire::track_id(&track);

        fs::write(&lyrics, "[00:01.00]Before\n").expect("lyrics file is written");

        let first = read(&id)
            .await
            .expect("first lookup succeeds")
            .expect("first lyrics file exists");

        fs::write(&lyrics, "[00:01.00]After\n").expect("lyrics file is updated");

        let second = read(&id)
            .await
            .expect("second lookup succeeds")
            .expect("second lyrics file exists");

        let Lyrics::Synced { lines: first } = first else {
            panic!("LRC file produces synced lyrics");
        };
        let Lyrics::Synced { lines: second } = second else {
            panic!("LRC file produces synced lyrics");
        };

        assert_eq!(first[0].text, "Before");
        assert_eq!(second[0].text, "After");

        fs::remove_dir_all(root).expect("scratch directory is removed");
    }
}
