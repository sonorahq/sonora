use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, params};
use storage::Database;

use crate::{LOCAL_PLAYLIST_PREFIX, ScanReport, Track};

pub struct Stored {
    pub id: String,
    pub name: String,
    pub modified_at: i64,
    /// The picture the listener gave the playlist, if any.
    pub cover: Option<PathBuf>,
}

/// Which local favorites table a star lands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Starred {
    Tracks,
    Albums,
    Artists,
}

impl Starred {
    fn table(self) -> &'static str {
        match self {
            Self::Tracks => "favorites",
            Self::Albums => "favorite_albums",
            Self::Artists => "favorite_artists",
        }
    }

    fn column(self) -> &'static str {
        match self {
            Self::Tracks => "track_id",
            Self::Albums => "album_id",
            Self::Artists => "artist_id",
        }
    }
}

pub struct Store {
    database: Database,
}

impl Store {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    fn open(&self) -> Result<rusqlite::Connection> {
        self.database.open().context("cannot open local playlists")
    }

    /// Repairs durable references only for exact moves whose old paths are no longer indexed.
    pub fn reconcile(&self, report: &ScanReport, tracks: &[Track]) -> Result<()> {
        if report.moved.is_empty() && report.moved_albums.is_empty() {
            return Ok(());
        }
        let ids: HashSet<&str> = tracks
            .iter()
            .filter_map(|track| track.id.as_deref())
            .collect();
        let albums: HashSet<&str> = tracks
            .iter()
            .filter_map(|track| track.album_id.as_deref())
            .collect();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .context("cannot repair moved music references")?;
        for (from, to) in &report.moved {
            if ids.contains(from.as_str()) || !ids.contains(to.as_str()) {
                continue;
            }
            transaction.execute(
                "UPDATE OR IGNORE playlist_tracks SET track_id = ?2 WHERE track_id = ?1",
                params![from, to],
            )?;
            transaction.execute("DELETE FROM playlist_tracks WHERE track_id = ?1", [from])?;
            transaction.execute("INSERT OR IGNORE INTO favorites (track_id, added_at) SELECT ?2, added_at FROM favorites WHERE track_id = ?1", params![from, to])?;
            transaction.execute("DELETE FROM favorites WHERE track_id = ?1", [from])?;
            transaction.execute("UPDATE OR REPLACE plays SET track_id = ?2 WHERE provider = 'local' AND track_id = ?1", params![from, to])?;
        }
        for (from, to) in &report.moved_albums {
            if albums.contains(from.as_str()) || !albums.contains(to.as_str()) {
                continue;
            }
            transaction.execute("INSERT OR IGNORE INTO favorite_albums (album_id, added_at) SELECT ?2, added_at FROM favorite_albums WHERE album_id = ?1", params![from, to])?;
            transaction.execute("DELETE FROM favorite_albums WHERE album_id = ?1", [from])?;
        }
        transaction
            .commit()
            .context("cannot commit moved music references")
    }

    /// Reports unresolved playlist and favorite references instead of silently hiding missing files.
    pub fn missing_tracks(&self, tracks: &[Track]) -> Result<Vec<String>> {
        let ids: HashSet<&str> = tracks
            .iter()
            .filter_map(|track| track.id.as_deref())
            .collect();
        let connection = self.open()?;
        let mut query = connection
            .prepare("SELECT track_id FROM favorites UNION SELECT track_id FROM playlist_tracks")?;
        let rows = query.query_map([], |row| row.get::<_, String>(0))?;
        let mut missing = Vec::new();
        for id in rows {
            let id = id?;
            if !ids.contains(id.as_str())
                && let Some(path) = super::wire::path_from_track_id(&id)
            {
                missing.push(format!(
                    "cannot find music file referenced by a playlist or favorite: {}",
                    path.display()
                ));
            }
        }
        Ok(missing)
    }

    /// The starred ids of one kind, newest first, with the moment each was starred.
    pub fn starred(&self, kind: Starred) -> Result<Vec<(String, i64)>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(&format!(
                "SELECT {}, added_at FROM {} ORDER BY added_at DESC",
                kind.column(),
                kind.table()
            ))
            .context("cannot read local favorites")?;
        let rows = query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("cannot read local favorites")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot read local favorites")
    }

    /// Local track IDs ordered by play count, then most recent play.
    pub fn most_played(&self, limit: usize) -> Result<Vec<String>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(
                "SELECT track_id, COUNT(*) as count
                 FROM plays
                 WHERE track_id LIKE 'local:%'
                 GROUP BY track_id
                 ORDER BY count DESC, MAX(played_at) DESC
                 LIMIT ?",
            )
            .context("cannot read most played local tracks")?;
        let rows = query
            .query_map(params![limit as i64], |row| row.get::<_, String>(0))
            .context("cannot read most played local tracks")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot read most played local tracks")
    }

    pub fn set_starred(&self, kind: Starred, id: &str, saved: bool) -> Result<()> {
        let connection = self.open()?;
        match saved {
            true => connection.execute(
                &format!(
                    "INSERT OR REPLACE INTO {} ({}, added_at) VALUES (?, ?)",
                    kind.table(),
                    kind.column()
                ),
                params![id, stamp()],
            ),
            false => connection.execute(
                &format!("DELETE FROM {} WHERE {} = ?", kind.table(), kind.column()),
                params![id],
            ),
        }
        .context("cannot update a local favorite")?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Stored>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(
                "SELECT id, name, modified_at, path FROM playlists
                 LEFT JOIN playlist_covers ON playlist_id = id
                 ORDER BY modified_at DESC",
            )
            .context("cannot read local playlists")?;
        let rows = query
            .query_map([], |row| {
                Ok(Stored {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    modified_at: row.get(2)?,
                    cover: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                })
            })
            .context("cannot read local playlists")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot read local playlists")
    }

    pub fn one(&self, id: &str) -> Result<Stored> {
        self.open()?
            .query_row(
                "SELECT id, name, modified_at, path FROM playlists
                 LEFT JOIN playlist_covers ON playlist_id = id
                 WHERE id = ?",
                params![id],
                |row| {
                    Ok(Stored {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        modified_at: row.get(2)?,
                        cover: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                    })
                },
            )
            .with_context(|| format!("cannot find local playlist {id}"))
    }

    pub fn create(&self, name: &str) -> Result<String> {
        let id = format!("{LOCAL_PLAYLIST_PREFIX}{}-{}", stamp(), minted());
        self.open()?
            .execute(
                "INSERT INTO playlists (id, name, modified_at) VALUES (?, ?, ?)",
                params![id, name, stamp()],
            )
            .context("cannot create a local playlist")?;
        Ok(id)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        self.open()?
            .execute(
                "UPDATE playlists SET name = ?, modified_at = ? WHERE id = ?",
                params![name, stamp(), id],
            )
            .context("cannot rename a local playlist")?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let connection = self.open()?;
        connection
            .execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?",
                params![id],
            )
            .context("cannot empty a local playlist")?;
        connection
            .execute("DELETE FROM playlists WHERE id = ?", params![id])
            .context("cannot delete a local playlist")?;
        Ok(())
    }

    /// Records `cover` as the playlist's own picture, or forgets it when `None`. Answers the
    /// picture it replaces, so the caller can delete that file.
    pub fn set_cover(&self, id: &str, cover: Option<&Path>) -> Result<Option<PathBuf>> {
        let connection = self.open()?;
        let previous = connection
            .query_row(
                "SELECT path FROM playlist_covers WHERE playlist_id = ?",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("cannot read a local playlist cover")?;
        match cover {
            Some(path) => connection.execute(
                "INSERT OR REPLACE INTO playlist_covers (playlist_id, path) VALUES (?, ?)",
                params![id, path.to_string_lossy()],
            ),
            None => connection.execute(
                "DELETE FROM playlist_covers WHERE playlist_id = ?",
                params![id],
            ),
        }
        .context("cannot change a local playlist cover")?;
        touch(&connection, id)?;
        Ok(previous.map(PathBuf::from))
    }

    pub fn add(&self, id: &str, track_id: &str) -> Result<()> {
        let connection = self.open()?;
        let position: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?",
                params![id],
                |row| row.get(0),
            )
            .context("cannot place a track in a local playlist")?;
        connection
            .execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
                 VALUES (?, ?, ?)",
                params![id, track_id, position],
            )
            .context("cannot add a track to a local playlist")?;
        touch(&connection, id)
    }

    /// Adds `track_ids` to the end of a playlist in order, in one write. A track it holds
    /// already stays where it is.
    pub fn extend(&self, id: &str, track_ids: &[String]) -> Result<()> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .context("cannot add tracks to a local playlist")?;
        let start: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?",
                params![id],
                |row| row.get(0),
            )
            .context("cannot place a track in a local playlist")?;
        for (position, track_id) in (start..).zip(track_ids) {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
                     VALUES (?, ?, ?)",
                    params![id, track_id, position],
                )
                .context("cannot add a track to a local playlist")?;
        }
        touch(&transaction, id)?;
        transaction
            .commit()
            .context("cannot add tracks to a local playlist")
    }

    pub fn remove(&self, id: &str, track_id: &str) -> Result<()> {
        let connection = self.open()?;
        connection
            .execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ? AND track_id = ?",
                params![id, track_id],
            )
            .context("cannot remove a track from a local playlist")?;
        touch(&connection, id)
    }

    pub fn tracks(&self, id: &str) -> Result<Vec<String>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare("SELECT track_id FROM playlist_tracks WHERE playlist_id = ? ORDER BY position")
            .context("cannot read a local playlist")?;
        let rows = query
            .query_map(params![id], |row| row.get::<_, String>(0))
            .context("cannot read a local playlist")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot read a local playlist")
    }
}

fn touch(connection: &Connection, id: &str) -> Result<()> {
    connection
        .execute(
            "UPDATE playlists SET modified_at = ? WHERE id = ?",
            params![stamp(), id],
        )
        .context("cannot stamp a local playlist")?;
    Ok(())
}

fn minted() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn stamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
