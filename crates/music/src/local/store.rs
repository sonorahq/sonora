use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};
use storage::Database;

use crate::LOCAL_PLAYLIST_PREFIX;

pub struct Stored {
    pub id: String,
    pub name: String,
    pub modified_at: i64,
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

    /// Whether a playlist of exactly this name already exists, however it got there.
    pub fn name_exists(&self, name: &str) -> Result<bool> {
        self.open()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM playlists WHERE name = ?)",
                params![name],
                |row| row.get(0),
            )
            .context("cannot check for a local playlist by name")
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

    /// Places every track in order, in one transaction. For seeding a freshly imported
    /// playlist, where an `add` per track would mean one query per row just to find the next
    /// position.
    pub fn add_all(&self, id: &str, track_ids: &[String]) -> Result<()> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .context("cannot start a local playlist import")?;
        for (position, track_id) in track_ids.iter().enumerate() {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
                     VALUES (?, ?, ?)",
                    params![id, track_id, position as i64],
                )
                .context("cannot add a track to a local playlist")?;
        }
        transaction
            .commit()
            .context("cannot finish a local playlist import")
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
