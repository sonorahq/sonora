use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::progress;
use crate::{Album, Track};

use super::index::{Changes, Index, Remembered};
use super::wire;

const SEPARATORS: [char; 8] = ['-', '–', '—', '.', '_', '·', ':', ' '];

/// How many threads read tags at once. Past this a spinning disk seeks more than it reads.
const MAX_READERS: usize = 8;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wav", "wv", "ape", "webm", "mka",
];

/// Playlist files a walk recognizes beside a library's tracks.
const PLAYLIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls", "xspf", "zpl", "wpl", "asx", "b4s"];

#[derive(Default)]
pub struct Scanned {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub portraits: HashMap<String, String>,
    pub playlists: Vec<PathBuf>,
}

/// One file the walk turned up. A file in a folder whose time has not moved is taken on trust:
/// its time and size come from the index and it is never stat'd, let alone opened.
pub(super) struct Found {
    pub path: PathBuf,
    pub mtime: i64,
    pub size: u64,
    /// Whether the index already holds the tags for exactly this time and size.
    pub known: bool,
}

/// One folder the walk entered, with the time it was listed at.
pub(super) struct Folder {
    pub path: PathBuf,
    pub mtime: i64,
    /// Whether the index was trusted for its contents, which also settles its portrait.
    pub known: bool,
}

/// One track as this scan has it, beside what the index should record for its file.
pub(super) struct Reading {
    pub path: PathBuf,
    pub mtime: i64,
    pub size: u64,
    pub track: Track,
    /// The album artist the tags name, `None` when they name none.
    pub album_artist: Option<String>,
    /// What the tag said the year was, kept so dating an album never reopens a file.
    pub year: Option<i32>,
    /// Whether the file was opened this time round, so its row has to be written back.
    pub fresh: bool,
}

/// What a folder answered about an artist portrait.
pub(super) struct Look {
    pub path: PathBuf,
    pub mtime: i64,
    pub portrait: Option<String>,
    pub fresh: bool,
}

/// Scans every root and merges the results into one library: tracks are collected from all
/// roots before albums/artists are grouped, so the same artist or album spread across more
/// than one folder still merges into a single entry, seamlessly.
///
/// Only what changed is read. The index holds every file's time, size and tags, and every
/// folder's time, so a folder that has not been touched is listed from memory and the files in
/// it are never opened. A folder's time moves when an entry is added, removed or renamed, so an
/// edit in place hides from it: Sonora's own tag editor drops the file it wrote from the index,
/// and the Rescan button drops every folder, which is what makes it thorough.
pub fn scan(roots: &[PathBuf], cache_dir: &Path, index: &Index) -> Scanned {
    let started = std::time::Instant::now();
    let progress = progress::start();
    let mut scanned = Scanned::default();
    let remembered = index.load();

    let mut files = Vec::new();
    let mut folders = Vec::new();
    let mut playlists = Vec::new();
    let mut reached = Vec::new();
    for root in roots {
        if root.is_dir() {
            reached.push(root.clone());
            walk(
                root,
                &remembered,
                &mut files,
                &mut folders,
                &mut playlists,
                &progress,
            );
        }
    }
    scanned.playlists = playlists;
    let found = files.len();
    // Every folder is looked at again for an artist portrait, so the count covers both passes
    // and the percentage does not sit at full while the second one runs.
    progress.found(found + folders.len());
    let walked = started.elapsed();
    if !progress.live() {
        return scanned;
    }

    let readings = read_tags(&files, &remembered, roots, cache_dir, &progress);
    let read = started.elapsed();
    if !progress.live() {
        return scanned;
    }

    let looks = look_for_portraits(&folders, &remembered, &readings, &progress);
    if !progress.live() {
        return scanned;
    }

    let changes = Changes::between(&remembered, &readings, &looks, &reached);
    let opened = readings.iter().filter(|reading| reading.fresh).count();
    scanned.portraits = name_portraits(&looks, &readings);
    let parsed: Vec<(Track, Option<String>, Option<i32>)> = readings
        .into_iter()
        .map(|reading| (reading.track, reading.album_artist, reading.year))
        .collect();
    scanned.albums = group_albums(&parsed);
    scanned.tracks = parsed.into_iter().map(|(track, ..)| track).collect();
    index.save(&changes);

    log::debug!(
        "local: scanned {found} files and {} folders in {}ms \
         ({}ms walking, {}ms reading {opened} changed tags, {}ms portraits), \
         {} tracks over {} albums",
        folders.len(),
        started.elapsed().as_millis(),
        walked.as_millis(),
        (read - walked).as_millis(),
        (started.elapsed() - read).as_millis(),
        scanned.tracks.len(),
        scanned.albums.len()
    );
    scanned
}

/// Reads the tags of every file the index does not already hold. Each one is a file open and a
/// tag parse, as much waiting on the disk as parsing, which is why it is spread over threads.
fn read_tags(
    files: &[Found],
    remembered: &Remembered,
    roots: &[PathBuf],
    cache_dir: &Path,
    progress: &progress::Scan,
) -> Vec<Reading> {
    spread(files, progress, |found| {
        let reading = match found.known {
            true => remembered.files.get(&found.path).map(|known| Reading {
                path: found.path.clone(),
                mtime: found.mtime,
                size: found.size,
                track: known.track.clone(),
                album_artist: known.album_artist.clone(),
                year: known.year,
                fresh: false,
            }),
            false => {
                read_one(&found.path, roots, cache_dir).map(|(track, album_artist, year)| Reading {
                    path: found.path.clone(),
                    mtime: found.mtime,
                    size: found.size,
                    track,
                    album_artist,
                    year,
                    fresh: true,
                })
            }
        };
        progress.read();
        reading
    })
}

/// One file's tags, with the folders above it standing in for whatever the tag does not say.
fn read_one(
    path: &Path,
    roots: &[PathBuf],
    cache_dir: &Path,
) -> Option<(Track, Option<String>, Option<i32>)> {
    let artist_hint = path
        .parent()
        .and_then(Path::parent)
        .filter(|dir| roots.iter().any(|root| dir.starts_with(root)))
        .map(folder_name);
    let (album_hint, _) = path
        .parent()
        .map(|dir| dated(&folder_name(dir)))
        .unwrap_or_default();
    let album_hint = (!album_hint.is_empty()).then_some(album_hint);

    wire::track_from_file(
        path,
        artist_hint.as_deref(),
        album_hint.as_deref(),
        cache_dir,
    )
}

/// Runs `work` over every item, a contiguous chunk to a thread, and returns what it kept in the
/// order it was given. Order is what the album grouping, the songs list and the first portrait
/// found all go by, so the chunks are joined in turn rather than raced together.
fn spread<T, R>(
    items: &[T],
    progress: &progress::Scan,
    work: impl Fn(&T) -> Option<R> + Sync,
) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(MAX_READERS);
    let chunk = items.len().div_ceil(threads).max(1);

    std::thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .take_while(|_| progress.live())
                        .filter_map(&work)
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        workers
            .into_iter()
            .filter_map(|worker| worker.join().ok())
            .flatten()
            .collect()
    })
}

/// Asks every folder named after an artist whether it holds a portrait. A folder the index was
/// trusted for answers from the index; the rest are stats on disk, so they are spread over
/// threads like the tag reads. A folder matching no artist is recorded without one, so a folder
/// that only becomes an artist's later needs a thorough Rescan to be looked at again.
fn look_for_portraits(
    folders: &[Folder],
    remembered: &Remembered,
    readings: &[Reading],
    progress: &progress::Scan,
) -> Vec<Look> {
    let named: HashSet<String> = readings
        .iter()
        .map(|reading| wire::normalize(&reading.track.artists))
        .collect();

    spread(folders, progress, |folder| {
        let known = folder
            .known
            .then(|| remembered.folders.get(&folder.path))
            .flatten();
        let portrait = match known {
            Some(seen) => seen.portrait.clone(),
            None => named
                .contains(&wire::normalize(&folder_name(&folder.path)))
                .then(|| wire::artist_cover(&folder.path))
                .flatten(),
        };
        progress.read();
        Some(Look {
            path: folder.path.clone(),
            mtime: folder.mtime,
            portrait,
            fresh: known.is_none(),
        })
    })
}

/// Files each portrait under the artist its folder is named after, first one in walk order.
fn name_portraits(looks: &[Look], readings: &[Reading]) -> HashMap<String, String> {
    let mut by_normalized: HashMap<String, String> = HashMap::new();
    for reading in readings {
        by_normalized
            .entry(wire::normalize(&reading.track.artists))
            .or_insert_with(|| reading.track.artists.clone());
    }

    let mut portraits = HashMap::new();
    for look in looks {
        let Some(portrait) = &look.portrait else {
            continue;
        };
        let Some(artist) = by_normalized.get(&wire::normalize(&folder_name(&look.path))) else {
            continue;
        };
        portraits
            .entry(artist.clone())
            .or_insert_with(|| portrait.clone());
    }
    portraits
}

/// Groups tracks into albums by the id each one carries, in scan order. An album is credited to
/// the first album artist its tags name, or to the artists all of its tracks share.
fn group_albums(parsed: &[(Track, Option<String>, Option<i32>)]) -> Vec<Album> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();

    for (index, (track, ..)) in parsed.iter().enumerate() {
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

            let album_artist = indices
                .iter()
                .find_map(|&i| parsed[i].1.clone())
                .unwrap_or_else(|| wire::shared_artists(&tracks));
            let name = tracks[0].album.clone();
            let year = album_year(indices, parsed);

            Some(wire::album_from_tracks(
                &id,
                &name,
                &album_artist,
                &tracks,
                year,
            ))
        })
        .collect()
}

/// The year an album is dated by: the first year any of its tracks carries, and the year in its
/// folder's name when none of them does. The years come from the scan's own reads, so dating an
/// album of a hundred tracks costs nothing on top.
fn album_year(indices: &[usize], parsed: &[(Track, Option<String>, Option<i32>)]) -> i32 {
    indices
        .iter()
        .find_map(|&i| parsed[i].2)
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

/// One pass over a folder tree, collecting the audio files to read and the folders to look for
/// portraits in. A folder whose time matches the index is listed from memory: no listing of it,
/// and no stat for any file in it. Children come out sorted either way, so the library holds the
/// same order whichever path a folder took.
fn walk(
    dir: &Path,
    remembered: &Remembered,
    files: &mut Vec<Found>,
    folders: &mut Vec<Folder>,
    playlists: &mut Vec<PathBuf>,
    progress: &progress::Scan,
) {
    let Some((mtime, _)) = stat(dir) else {
        return;
    };
    let known = remembered.unchanged(dir, mtime);
    folders.push(Folder {
        path: dir.to_path_buf(),
        mtime,
        known,
    });

    match known {
        true => {
            // Playlists aren't part of the index, so a folder whose mtime says nothing changed
            // still needs a fresh readdir here, or an edit to a playlist file already on disk -
            // which never moves the folder's own mtime - would never be seen again. This runs
            // even for a folder with no cached children at all (one holding only playlists, say),
            // which is why it comes before that lookup can return early.
            if let Ok(read) = std::fs::read_dir(dir) {
                playlists.extend(
                    read.filter_map(|entry| entry.ok())
                        .map(|entry| entry.path())
                        .filter(|path| is_playlist_file(path)),
                );
            }
            let Some(children) = remembered.children(dir) else {
                return;
            };
            for path in &children.files {
                let Some(known) = remembered.files.get(path) else {
                    continue;
                };
                files.push(Found {
                    path: path.clone(),
                    mtime: known.mtime,
                    size: known.size,
                    known: true,
                });
            }
            for child in &children.folders {
                if !progress.live() {
                    return;
                }
                walk(child, remembered, files, folders, playlists, progress);
            }
        }
        false => {
            let Ok(read) = std::fs::read_dir(dir) else {
                return;
            };
            let mut here: Vec<(PathBuf, bool)> = read
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    let path = entry.path();
                    let folder = is_folder(&entry, &path);
                    (path, folder)
                })
                .collect();
            here.sort();
            for (path, folder) in here {
                if !progress.live() {
                    return;
                }
                if folder {
                    walk(&path, remembered, files, folders, playlists, progress);
                } else if is_audio_file(&path) {
                    let Some((mtime, size)) = stat(&path) else {
                        continue;
                    };
                    let known = remembered
                        .files
                        .get(&path)
                        .is_some_and(|known| known.mtime == mtime && known.size == size);
                    files.push(Found {
                        path,
                        mtime,
                        size,
                        known,
                    });
                } else if is_playlist_file(&path) {
                    playlists.push(path);
                }
            }
        }
    }
    progress.walking(files.len() + folders.len());
}

/// Whether a listed entry is a folder, without a stat where the listing already said so. A
/// share charges for every stat, and a big library has one entry per track; a symlink is the one
/// case that still has to be followed to find out.
fn is_folder(entry: &std::fs::DirEntry, path: &Path) -> bool {
    match entry.file_type() {
        Ok(kind) if !kind.is_symlink() => kind.is_dir(),
        _ => path.is_dir(),
    }
}

/// When a path was last written, as whole seconds since the epoch, and how big it is. Seconds
/// are enough: a file written twice inside one second is rare, and its size moves with it.
fn stat(path: &Path) -> Option<(i64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((mtime, metadata.len()))
}

fn dated(name: &str) -> (String, Option<i32>) {
    let name = name.trim();
    if let Some(dated) = leading_year(name) {
        return dated;
    }
    if let Some(dated) = trailing_year(name) {
        return dated;
    }
    if let Some(dated) = trailing_plain_year(name) {
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
    let (digits, rest) = rest
        .split_at_checked(4)
        .filter(|(_, rest)| !rest.starts_with(|letter: char| letter.is_ascii_digit()))?;
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

fn trailing_plain_year(name: &str) -> Option<(String, Option<i32>)> {
    let cut = name.len().checked_sub(4)?;
    let (rest, digits) = name.split_at_checked(cut)?;
    let year = digits.parse::<i32>().ok().filter(|year| plausible(*year))?;
    let rest = rest.trim_end_matches(SEPARATORS).trim();
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

fn is_playlist_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PLAYLIST_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard};

    /// The scan generation is process-wide: whichever test starts a scan last cancels every other
    /// one in flight, so the tests that start one take this in turn.
    static SCANNING: Mutex<()> = Mutex::new(());

    /// The lock without the poisoning, so one failing test does not take the rest with it.
    fn alone() -> MutexGuard<'static, ()> {
        SCANNING.lock().unwrap_or_else(|held| held.into_inner())
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, []).unwrap();
    }

    /// An empty folder and an index of its own, so one test never sees another's rows.
    fn scratch(name: &str) -> (PathBuf, Index) {
        let dir = std::env::temp_dir().join(name);
        let database = std::env::temp_dir().join(format!("{name}-index.sqlite"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&database);
        (dir, Index::new(storage::Cache::at(database)))
    }

    #[test]
    fn ignores_non_audio_files() {
        let _scanning = alone();
        let (dir, index) = scratch("sonora-scan-test-ignore");
        touch(&dir.join("notes.txt"));
        let scanned = scan(std::slice::from_ref(&dir), &dir, &index);
        assert!(scanned.tracks.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_root_yields_nothing() {
        let _scanning = alone();
        let (dir, index) = scratch("sonora-scan-test-missing");
        let scanned = scan(std::slice::from_ref(&dir), &dir, &index);
        assert!(scanned.tracks.is_empty());
        assert!(scanned.albums.is_empty());
    }

    #[test]
    fn a_folder_opening_with_a_year_gives_the_album_and_the_year() {
        assert_eq!(dated("1999 - Album"), ("Album".to_owned(), Some(1999)));
        assert_eq!(dated("[2004] Album"), ("Album".to_owned(), Some(2004)));
    }

    #[test]
    fn a_longer_number_opening_a_folder_is_not_a_year() {
        assert_eq!(dated("10000 Days"), ("10000 Days".to_owned(), None));
        assert_eq!(dated("20000 Leagues"), ("20000 Leagues".to_owned(), None));
    }

    #[test]
    fn walks_nested_folders_for_stray_audio_files() {
        let _scanning = alone();
        let (dir, _) = scratch("sonora-scan-test-stragglers");
        touch(&dir.join("top.mp3"));
        touch(&dir.join("a/b/c/deep.flac"));
        touch(&dir.join("a/b/c/notes.txt"));

        let mut found = Vec::new();
        let mut folders = Vec::new();
        let mut playlists = Vec::new();
        walk(
            &dir,
            &Remembered::default(),
            &mut found,
            &mut folders,
            &mut playlists,
            &crate::progress::start(),
        );

        let paths: Vec<PathBuf> = found.into_iter().map(|file| file.path).collect();
        assert_eq!(
            paths,
            vec![dir.join("a/b/c/deep.flac"), dir.join("top.mp3")]
        );
        let mut walked: Vec<PathBuf> = folders.into_iter().map(|folder| folder.path).collect();
        walked.sort();
        assert_eq!(
            walked,
            vec![
                dir.clone(),
                dir.join("a"),
                dir.join("a/b"),
                dir.join("a/b/c")
            ]
        );
        fs::remove_dir_all(&dir).ok();
    }
}
