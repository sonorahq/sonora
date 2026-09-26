use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};
use storage::Cache;

use super::scan::{Look, Reading};
use crate::{ArtistRef, Track};

/// What the last scan learned about one file: when it was last written, how big it was, and the
/// track its tags produced. A file whose time and size both match is not opened again.
pub struct Known {
    pub mtime: i64,
    pub size: u64,
    pub track: Track,
    /// The album artists the tags name, empty when they name none.
    pub album_artists: Vec<ArtistRef>,
    /// The year the tag carried, which is what an album is dated by.
    pub year: Option<i32>,
}

/// What the last scan learned about one folder. `portrait` is what `wire::artist_cover` answered
/// for it, `None` meaning it had none rather than that nobody looked.
pub struct Seen {
    pub mtime: i64,
    pub portrait: Option<String>,
}

/// Everything the last scan recorded, laid out for a walk: entries by path, and the children of
/// every folder, so a folder whose time has not moved can be listed from memory instead of from
/// the disk.
#[derive(Default)]
pub struct Remembered {
    pub files: HashMap<PathBuf, Known>,
    pub folders: HashMap<PathBuf, Seen>,
    children: HashMap<PathBuf, Children>,
}

#[derive(Default)]
pub struct Children {
    pub folders: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

impl Remembered {
    /// The children a folder had, sorted, or nothing if it was never recorded.
    pub fn children(&self, folder: &Path) -> Option<&Children> {
        self.children.get(folder)
    }

    /// Whether a folder is recorded with exactly this modification time, which is what makes it
    /// safe to list from memory. A folder's time moves when an entry is added, removed or
    /// renamed, so only an edit in place hides from it.
    pub fn unchanged(&self, folder: &Path, mtime: i64) -> bool {
        self.folders
            .get(folder)
            .is_some_and(|seen| seen.mtime == mtime)
    }
}

/// The scan index: what the last scan read, so the next one only reads what changed. It lives in
/// the cache database beside the library snapshots, and losing it costs one slow scan.
#[derive(Clone)]
pub struct Index {
    cache: Cache,
}

impl Index {
    pub fn new(cache: Cache) -> Self {
        Self { cache }
    }

    /// Reads the whole index in one go. A row whose track no longer parses, because the models
    /// moved on, is left out and its file is read again.
    pub fn load(&self) -> Remembered {
        match self.read() {
            Ok(remembered) => remembered,
            Err(error) => {
                log::warn!("local: cannot read the scan index: {error:#}");
                Remembered::default()
            }
        }
    }

    /// Records what a scan changed. A scan that found nothing new writes nothing at all, which
    /// is the point: the steady state costs one read of the index and no write.
    pub fn save(&self, changes: &Changes) {
        if changes.is_empty() {
            return;
        }
        if let Err(error) = self.write(changes) {
            log::warn!("local: cannot record the scan index: {error:#}");
        }
    }

    /// Forgets every folder, so the next scan lists them all again and stats every file. This is
    /// what the Rescan button means: do not trust what you remember.
    pub fn distrust(&self) {
        let forgotten = self
            .cache
            .open()
            .and_then(|connection| {
                connection
                    .execute("DELETE FROM local_folders", [])
                    .context("cannot clear the folder index")
            })
            .context("cannot open the cache");
        if let Err(error) = forgotten {
            log::warn!("local: cannot forget the scanned folders: {error:#}");
        }
    }

    /// Drops what is known about these files and about every folder above them, for a caller
    /// that has just written to them.
    ///
    /// The whole chain has to go. A folder is listed from the index only while its own row is
    /// there, and that listing is built from the rows of its children, so dropping a file row
    /// alone would read as the file having left the library, and dropping its folder alone would
    /// read as the folder having left. Writing a file in place moves no folder's time, so this
    /// is the only thing that tells the next scan to look again.
    pub fn forget(&self, paths: &[PathBuf]) {
        let dropped = (|| -> Result<()> {
            let connection = self.cache.open()?;
            let mut file = connection.prepare("DELETE FROM local_files WHERE path = ?")?;
            let mut folder = connection.prepare("DELETE FROM local_folders WHERE path = ?")?;
            for path in paths {
                file.execute(params![text(path)])
                    .context("cannot drop a file from the index")?;
                for above in path.ancestors().skip(1) {
                    folder
                        .execute(params![text(above)])
                        .context("cannot drop a folder from the index")?;
                }
            }
            Ok(())
        })();
        if let Err(error) = dropped {
            log::warn!("local: cannot forget an edited file: {error:#}");
        }
    }

    fn read(&self) -> Result<Remembered> {
        let connection = self.cache.open()?;
        let mut remembered = Remembered::default();

        let mut files = connection
            .prepare("SELECT path, parent, mtime, size, track FROM local_files")
            .context("cannot read the file index")?;
        let rows = files
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .context("cannot read the file index")?;
        // Reading the rows is quick; turning ten thousand of them back into tracks is not, and
        // it is the whole cost of a scan that changed nothing. So it goes over threads.
        let raw: Vec<(String, String, i64, i64, String)> = rows.filter_map(Result::ok).collect();
        for (path, parent, mtime, size, track, album_artists, year) in parse(raw) {
            let path = PathBuf::from(path);
            remembered
                .children
                .entry(PathBuf::from(parent))
                .or_default()
                .files
                .push(path.clone());
            remembered.files.insert(
                path,
                Known {
                    mtime,
                    size: size as u64,
                    track,
                    album_artists,
                    year,
                },
            );
        }

        let mut folders = connection
            .prepare("SELECT path, parent, mtime, portrait FROM local_folders")
            .context("cannot read the folder index")?;
        let rows = folders
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .context("cannot read the folder index")?;
        for (path, parent, mtime, portrait) in rows.filter_map(Result::ok) {
            let path = PathBuf::from(path);
            remembered
                .children
                .entry(PathBuf::from(parent))
                .or_default()
                .folders
                .push(path.clone());
            remembered.folders.insert(path, Seen { mtime, portrait });
        }

        for children in remembered.children.values_mut() {
            children.folders.sort();
            children.files.sort();
        }
        Ok(remembered)
    }

    fn write(&self, changes: &Changes) -> Result<()> {
        let mut connection = self.cache.open()?;
        let transaction = connection
            .transaction()
            .context("cannot start the index write")?;
        forget_rows(&transaction, "local_files", &changes.gone_files)?;
        forget_rows(&transaction, "local_folders", &changes.gone_folders)?;
        write_files(&transaction, &changes.files)?;
        write_folders(&transaction, &changes.folders)?;
        transaction.commit().context("cannot commit the index")?;
        Ok(())
    }
}

/// What one scan has to write back: the files and folders it read this time, and the ones that
/// have left the disk since the last scan.
#[derive(Default)]
pub struct Changes {
    files: Vec<(PathBuf, Known)>,
    folders: Vec<(PathBuf, Seen)>,
    gone_files: Vec<PathBuf>,
    gone_folders: Vec<PathBuf>,
}

impl Changes {
    /// Works out what moved between the index and this scan. Deletions are limited to the roots
    /// the walk actually reached, so a folder that was offline this time keeps its rows instead
    /// of costing a full read once it is back.
    pub(super) fn between(
        remembered: &Remembered,
        readings: &[Reading],
        looks: &[Look],
        reached: &[PathBuf],
    ) -> Self {
        let mut changes = Self::default();
        let here: HashMap<&Path, ()> = readings
            .iter()
            .map(|reading| (reading.path.as_path(), ()))
            .collect();
        let listed: HashMap<&Path, ()> =
            looks.iter().map(|look| (look.path.as_path(), ())).collect();

        for reading in readings.iter().filter(|reading| reading.fresh) {
            changes.files.push((
                reading.path.clone(),
                Known {
                    mtime: reading.mtime,
                    size: reading.size,
                    track: reading.track.clone(),
                    album_artists: reading.album_artists.clone(),
                    year: reading.year,
                },
            ));
        }
        for look in looks.iter().filter(|look| look.fresh) {
            changes.folders.push((
                look.path.clone(),
                Seen {
                    mtime: look.mtime,
                    portrait: look.portrait.clone(),
                },
            ));
        }

        let under = |path: &Path| reached.iter().any(|root| path.starts_with(root));
        changes.gone_files = remembered
            .files
            .keys()
            .filter(|path| under(path) && !here.contains_key(path.as_path()))
            .cloned()
            .collect();
        changes.gone_folders = remembered
            .folders
            .keys()
            .filter(|path| under(path) && !listed.contains_key(path.as_path()))
            .cloned()
            .collect();
        changes
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.folders.is_empty()
            && self.gone_files.is_empty()
            && self.gone_folders.is_empty()
    }
}

fn forget_rows(connection: &Connection, table: &str, paths: &[PathBuf]) -> Result<()> {
    let mut delete = connection
        .prepare(&format!("DELETE FROM {table} WHERE path = ?"))
        .context("cannot drop an index row")?;
    for path in paths {
        delete
            .execute(params![text(path)])
            .context("cannot drop an index row")?;
    }
    Ok(())
}

/// Records each file as its track, the tag's year and the album artist, in that order. A row in
/// any other shape fails to parse on the next scan and its file is read again.
fn write_files(connection: &Connection, files: &[(PathBuf, Known)]) -> Result<()> {
    let mut insert = connection
        .prepare("INSERT OR REPLACE INTO local_files (path, parent, mtime, size, track) VALUES (?, ?, ?, ?, ?)")
        .context("cannot record a file")?;
    for (path, known) in files {
        let Ok(track) = serde_json::to_string(&(&known.track, known.year, &known.album_artists))
        else {
            continue;
        };
        insert
            .execute(params![
                text(path),
                text(parent(path)),
                known.mtime,
                known.size as i64,
                track
            ])
            .context("cannot record a file")?;
    }
    Ok(())
}

fn write_folders(connection: &Connection, folders: &[(PathBuf, Seen)]) -> Result<()> {
    let mut insert = connection
        .prepare(
            "INSERT OR REPLACE INTO local_folders (path, parent, mtime, portrait) \
             VALUES (?, ?, ?, ?)",
        )
        .context("cannot record a folder")?;
    for (path, seen) in folders {
        insert
            .execute(params![
                text(path),
                text(parent(path)),
                seen.mtime,
                seen.portrait
            ])
            .context("cannot record a folder")?;
    }
    Ok(())
}

/// Turns the stored tracks back into models, a chunk of rows to a thread. A row that no longer
/// parses is dropped, which costs one file read on the next scan and nothing else.
type Parsed = (String, String, i64, i64, Track, Vec<ArtistRef>, Option<i32>);

fn parse(rows: Vec<(String, String, i64, i64, String)>) -> Vec<Parsed> {
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk = rows.len().div_ceil(threads).max(1);

    std::thread::scope(|scope| {
        let workers: Vec<_> = rows
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .filter_map(|(path, parent, mtime, size, track)| {
                            let (track, year, album_artists) =
                                serde_json::from_str::<(Track, Option<i32>, Vec<ArtistRef>)>(track)
                                    .ok()?;
                            Some((
                                path.clone(),
                                parent.clone(),
                                *mtime,
                                *size,
                                track,
                                album_artists,
                                year,
                            ))
                        })
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

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(path)
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
