use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::{Album, SavedArtist, Track};

use super::wire;

const SEPARATORS: [char; 8] = ['-', '–', '—', '.', '_', '·', ':', ' '];

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "wv", "ape", "webm", "mka",
];

#[derive(Default)]
pub struct Scanned {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<SavedArtist>,
    pub portraits: HashMap<String, String>,
}

/// Scans every root and merges the results into one library: tracks are collected from all
/// roots before albums/artists are grouped, so the same artist or album spread across more
/// than one folder still merges into a single entry, seamlessly.
pub fn scan(roots: &[PathBuf], cache_dir: &Path) -> Scanned {
    let mut scanned = Scanned::default();

    let mut files = Vec::new();
    for root in roots {
        if root.is_dir() {
            walk_audio_files(root, &mut files);
        }
    }

    let parsed: Vec<(Track, String)> = files
        .into_iter()
        .filter_map(|path| {
            let artist_hint = path.parent().and_then(Path::parent).map(folder_name);
            let (album_hint, _) = path
                .parent()
                .map(|dir| dated(&folder_name(dir)))
                .unwrap_or_default();
            let album_hint = (!album_hint.is_empty()).then_some(album_hint);

            wire::track_from_file(
                &path,
                artist_hint.as_deref(),
                album_hint.as_deref(),
                cache_dir,
            )
        })
        .collect();

    scanned.portraits = collect_portraits(roots, &parsed);
    scanned.albums = group_albums(&parsed);
    scanned.artists = group_artists(&parsed, &scanned.portraits, &scanned.albums);
    scanned.tracks = parsed.into_iter().map(|(track, _)| track).collect();
    scanned
}

fn group_albums(parsed: &[(Track, String)]) -> Vec<Album> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();

    for (index, (track, _)) in parsed.iter().enumerate() {
        let Some(id) = &track.album_id else {
            continue;
        };
        if !groups.contains_key(id) {
            order.push(id.clone());
        }
        groups.entry(id.clone()).or_default().push(index);
    }

    order
        .into_iter()
        .filter_map(|id| {
            let indices = groups.get(&id)?;
            let mut tracks: Vec<Track> = indices.iter().map(|&i| parsed[i].0.clone()).collect();
            tracks.sort_by_key(|track| (track.disc_number, track.track_number, track.name.clone()));

            let album_artist = parsed[indices[0]].1.clone();
            let name = tracks[0].album.clone();
            let year = album_year(indices, parsed);

            Some(wire::album_from_tracks(&name, &album_artist, &tracks, year))
        })
        .collect()
}

fn album_year(indices: &[usize], parsed: &[(Track, String)]) -> i32 {
    indices
        .iter()
        .find_map(|&i| {
            parsed[i]
                .0
                .id
                .as_deref()
                .and_then(wire::path_from_track_id)
                .and_then(wire::tag_year)
        })
        .or_else(|| {
            parsed[indices[0]]
                .0
                .id
                .as_deref()
                .and_then(wire::path_from_track_id)
                .and_then(Path::parent)
                .and_then(|dir| dated(&folder_name(dir)).1)
        })
        .unwrap_or(0)
}

fn group_artists(
    parsed: &[(Track, String)],
    portraits: &HashMap<String, String>,
    albums: &[Album],
) -> Vec<SavedArtist> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();

    for (index, (track, _)) in parsed.iter().enumerate() {
        for artist_ref in &track.artist_refs {
            let id = artist_ref
                .id
                .clone()
                .unwrap_or_else(|| wire::artist_id(&artist_ref.name));
            if !groups.contains_key(&id) {
                order.push(id.clone());
            }
            groups.entry(id).or_default().push(index);
        }
    }

    order
        .into_iter()
        .filter_map(|id| {
            let indices = groups.get(&id)?;
            let name = indices.iter().find_map(|&i| {
                parsed[i].0.artist_refs.iter().find_map(|artist_ref| {
                    (artist_ref.id.as_deref() == Some(id.as_str())).then(|| artist_ref.name.clone())
                })
            })?;
            let cover = portraits
                .get(&id)
                .cloned()
                .or_else(|| {
                    albums
                        .iter()
                        .find(|album| {
                            album
                                .artist_refs
                                .iter()
                                .any(|album_ref| album_ref.id.as_deref() == Some(id.as_str()))
                        })
                        .and_then(|album| album.cover.clone())
                })
                .or_else(|| indices.iter().find_map(|&i| parsed[i].0.cover.clone()));
            Some(SavedArtist {
                id,
                name,
                cover,
                added_at: indices.iter().filter_map(|&i| parsed[i].0.added_at).max(),
            })
        })
        .collect()
}

fn collect_portraits(roots: &[PathBuf], parsed: &[(Track, String)]) -> HashMap<String, String> {
    let mut known: HashSet<String> = HashSet::new();
    for (track, _) in parsed {
        for artist_ref in &track.artist_refs {
            match artist_ref.id.as_deref() {
                Some(id) => {
                    known.insert(id.to_owned());
                }
                None => {
                    known.insert(wire::artist_id(&artist_ref.name));
                }
            }
        }
    }

    let mut dirs = Vec::new();
    for root in roots {
        walk_dirs(root, &mut dirs);
    }

    let mut portraits = HashMap::new();
    for dir in dirs {
        let id = wire::artist_id(&folder_name(&dir));
        if !known.contains(&id) || portraits.contains_key(&id) {
            continue;
        }
        if let Some(portrait) = wire::artist_cover(&dir) {
            portraits.insert(id, portrait);
        }
    }
    portraits
}

fn walk_dirs(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            found.push(path.clone());
            walk_dirs(&path, found);
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
        let scanned = scan(std::slice::from_ref(&dir), &dir);
        assert!(scanned.tracks.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_root_yields_nothing() {
        let dir = std::env::temp_dir().join("sonora-scan-test-missing");
        let _ = fs::remove_dir_all(&dir);
        let scanned = scan(std::slice::from_ref(&dir), &dir);
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
