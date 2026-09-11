use std::collections::HashMap;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};
use storage::Database;

use crate::LOCAL_PLAYLIST_PREFIX;

pub struct Stored {
    pub id: String,
    pub name: String,
    pub modified_at: i64,
}

/// A local track's tag-derived fields, cached so a rescan can skip reopening and re-decoding a
/// file whose `modified_at` still matches.
#[derive(Clone)]
pub struct CachedTrack {
    pub modified_at: i64,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub cover: Option<String>,
    pub duration_ms: i64,
    pub track_number: u32,
    pub disc_number: u32,
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
            .prepare("SELECT id, name, modified_at FROM playlists ORDER BY modified_at DESC")
            .context("cannot read local playlists")?;
        let rows = query
            .query_map([], |row| {
                Ok(Stored {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    modified_at: row.get(2)?,
                })
            })
            .context("cannot read local playlists")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot read local playlists")
    }

    pub fn one(&self, id: &str) -> Result<Stored> {
        self.open()?
            .query_row(
                "SELECT id, name, modified_at FROM playlists WHERE id = ?",
                params![id],
                |row| {
                    Ok(Stored {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        modified_at: row.get(2)?,
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

    /// Every cached local track, keyed by its absolute path, for a scan to check its files
    /// against before reopening and re-decoding any of them.
    pub fn cached_tracks(&self) -> Result<HashMap<String, CachedTrack>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(
                "SELECT path, modified_at, name, artist, album, album_artist, cover,
                        duration_ms, track_number, disc_number
                 FROM local_tracks",
            )
            .context("cannot read the local track cache")?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    CachedTrack {
                        modified_at: row.get(1)?,
                        name: row.get(2)?,
                        artist: row.get(3)?,
                        album: row.get(4)?,
                        album_artist: row.get(5)?,
                        cover: row.get(6)?,
                        duration_ms: row.get(7)?,
                        track_number: row.get(8)?,
                        disc_number: row.get(9)?,
                    },
                ))
            })
            .context("cannot read the local track cache")?;

        rows.collect::<rusqlite::Result<_>>()
            .context("cannot read the local track cache")
    }

    /// Replaces the whole local track cache with `rows`: a file missing from `rows` falls out
    /// of the cache, which is how a track removed from disk stops being remembered.
    pub fn set_cached_tracks(&self, rows: &[(String, CachedTrack)]) -> Result<()> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .context("cannot start a local track cache update")?;
        transaction
            .execute("DELETE FROM local_tracks", [])
            .context("cannot clear the local track cache")?;
        {
            // One statement reused for every row, rather than reparsed per insert.
            let mut insert = transaction
                .prepare(
                    "INSERT INTO local_tracks
                         (path, modified_at, name, artist, album, album_artist, cover,
                          duration_ms, track_number, disc_number)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .context("cannot prepare the local track cache write")?;
            for (path, cached) in rows {
                insert
                    .execute(params![
                        path,
                        cached.modified_at,
                        cached.name,
                        cached.artist,
                        cached.album,
                        cached.album_artist,
                        cached.cover,
                        cached.duration_ms,
                        cached.track_number,
                        cached.disc_number,
                    ])
                    .context("cannot write the local track cache")?;
            }
        }
        transaction
            .commit()
            .context("cannot save the local track cache")
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
