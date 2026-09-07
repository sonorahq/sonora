use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::{Album, Track};

use super::wire;

const SEPARATORS: [char; 8] = ['-', '–', '—', '.', '_', '·', ':', ' '];

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "wv", "ape", "webm", "mka",
];

#[derive(Default)]
pub struct Scanned {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub portraits: HashMap<String, String>,
}

pub fn scan(root: &Path, cache_dir: &Path) -> Scanned {
    let mut scanned = Scanned::default();
    if !root.is_dir() {
        return scanned;
    }

    for entry in entries(root) {
        if entry.is_file() {
            scanned
                .tracks
                .extend(wire::track_from_file(&entry, None, None, cache_dir));
            continue;
        }
        if !entry.is_dir() {
            continue;
        }
        scan_artist(&entry, cache_dir, &mut scanned);
    }

    scan_stragglers(root, cache_dir, &mut scanned);
    scanned
}

fn scan_stragglers(root: &Path, cache_dir: &Path, scanned: &mut Scanned) {
    let known: HashSet<String> = scanned
        .tracks
        .iter()
        .filter_map(|track| track.id.clone())
        .collect();
    let mut found = Vec::new();
    walk_audio_files(root, &mut found);

    for path in found {
        if known.contains(&wire::track_id(&path)) {
            continue;
        }
        let artist_hint = path.parent().map(folder_name);
        if let Some(track) = wire::track_from_file(&path, artist_hint.as_deref(), None, cache_dir) {
            scanned.tracks.push(track);
        }
    }
}

fn walk_audio_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            walk_audio_files(&path, found);
        } else if is_audio_file(&path) {
            found.push(path);
        }
    }
}

fn scan_artist(dir: &Path, cache_dir: &Path, scanned: &mut Scanned) {
    let artist = folder_name(dir);
    if let Some(portrait) = wire::artist_cover(dir) {
        scanned.portraits.insert(artist.clone(), portrait);
    }

    for entry in entries(dir) {
        if entry.is_file() {
            scanned.tracks.extend(wire::track_from_file(
                &entry,
                Some(&artist),
                None,
                cache_dir,
            ));
            continue;
        }
        if !entry.is_dir() {
            continue;
        }
        scan_album(&entry, &artist, cache_dir, scanned);
    }
}

fn scan_album(dir: &Path, artist: &str, cache_dir: &Path, scanned: &mut Scanned) {
    let (name, folder_year) = dated(&folder_name(dir));

    let mut tracks: Vec<Track> = entries(dir)
        .into_iter()
        .filter(|entry| entry.is_file())
        .filter_map(|entry| {
            wire::track_from_file(&entry, Some(artist), Some((&name, dir)), cache_dir)
        })
        .collect();
    if tracks.is_empty() {
        return;
    }
    tracks.sort_by_key(|track| (track.disc_number, track.track_number, track.name.clone()));

    let album_artist = tracks
        .first()
        .map(|track| track.artists.clone())
        .unwrap_or_else(|| artist.to_owned());
    let album_name = tracks
        .first()
        .map(|track| track.album.clone())
        .filter(|album| !album.is_empty())
        .unwrap_or(name);

    let year = tracks
        .first()
        .and_then(|track| track.id.as_deref())
        .and_then(wire::path_from_track_id)
        .and_then(wire::tag_year)
        .or(folder_year)
        .unwrap_or(0);

    scanned.albums.push(wire::album_from_tracks(
        &album_name,
        &album_artist,
        dir,
        &tracks,
        year,
    ));
    scanned.tracks.extend(tracks);
}

fn dated(name: &str) -> (String, Option<i32>) {
    let name = name.trim();
    if let Some(dated) = leading_year(name) {
        return dated;
    }
    if let Some(dated) = trailing_year(name) {
        return dated;
    }

    (name.to_owned(), None)
}

fn plausible(year: i32) -> bool {
    (1000..=2999).contains(&year)
}

fn leading_year(name: &str) -> Option<(String, Option<i32>)> {
    let (open, rest) = match name.strip_prefix(['[', '(']) {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    let (digits, rest) = rest.split_at_checked(4)?;
    let year = digits.parse::<i32>().ok().filter(|year| plausible(*year))?;
    let rest = match open {
        true => rest.strip_prefix([']', ')'])?,
        false => rest,
    };
    let rest = rest.trim_start_matches(SEPARATORS).trim();
    match rest.is_empty() {
        true => None,
        false => Some((rest.to_owned(), Some(year))),
    }
}

fn trailing_year(name: &str) -> Option<(String, Option<i32>)> {
    let rest = name.strip_suffix([']', ')'])?;
    let cut = rest.len().checked_sub(4)?;
    let (rest, digits) = rest.split_at_checked(cut)?;
    let year = digits.parse::<i32>().ok().filter(|year| plausible(*year))?;
    let rest = rest.strip_suffix(['[', '('])?.trim();
    match rest.is_empty() {
        true => None,
        false => Some((rest.to_owned(), Some(year))),
    }
}

fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() || is_audio_file(path))
        .collect();
    entries.sort();
    entries
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, []).unwrap();
    }

    #[test]
    fn ignores_non_audio_files() {
        let dir = std::env::temp_dir().join("sonora-scan-test-ignore");
        let _ = fs::remove_dir_all(&dir);
        touch(&dir.join("notes.txt"));
        let scanned = scan(&dir, &dir);
        assert!(scanned.tracks.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_root_yields_nothing() {
        let dir = std::env::temp_dir().join("sonora-scan-test-missing");
        let _ = fs::remove_dir_all(&dir);
        let scanned = scan(&dir, &dir);
        assert!(scanned.tracks.is_empty());
        assert!(scanned.albums.is_empty());
    }

    #[test]
    fn walks_nested_folders_for_stray_audio_files() {
        let dir = std::env::temp_dir().join("sonora-scan-test-stragglers");
        let _ = fs::remove_dir_all(&dir);
        touch(&dir.join("top.mp3"));
        touch(&dir.join("a/b/c/deep.flac"));
        touch(&dir.join("a/b/c/notes.txt"));

        let mut found = Vec::new();
        walk_audio_files(&dir, &mut found);
        found.sort();

        assert_eq!(
            found,
            vec![dir.join("a/b/c/deep.flac"), dir.join("top.mp3")]
        );
        fs::remove_dir_all(&dir).ok();
    }
}
