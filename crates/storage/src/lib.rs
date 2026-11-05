mod cache;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};

pub use cache::Cache;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS app_state (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS plays (
        scope TEXT NOT NULL,
        provider TEXT NOT NULL,
        track_id TEXT NOT NULL,
        played_at INTEGER NOT NULL,
        name TEXT NOT NULL,
        playable INTEGER NOT NULL,
        artists TEXT NOT NULL,
        artist_refs TEXT NOT NULL,
        album TEXT NOT NULL,
        album_id TEXT,
        cover TEXT,
        duration_ms INTEGER NOT NULL,
        explicit INTEGER NOT NULL,
        PRIMARY KEY (scope, provider, track_id, played_at)
    );
    CREATE INDEX IF NOT EXISTS plays_scope_time
        ON plays (scope, played_at DESC);
    CREATE TABLE IF NOT EXISTS flags (
        key TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS playlists (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        modified_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS playlist_tracks (
        playlist_id TEXT NOT NULL,
        track_id TEXT NOT NULL,
        position INTEGER NOT NULL,
        PRIMARY KEY (playlist_id, track_id)
    );
    CREATE INDEX IF NOT EXISTS playlist_tracks_order
        ON playlist_tracks (playlist_id, position);
    CREATE TABLE IF NOT EXISTS playlist_covers (
        playlist_id TEXT PRIMARY KEY,
        path TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS favorites (
        track_id TEXT PRIMARY KEY,
        added_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS favorite_albums (
        album_id TEXT PRIMARY KEY,
        added_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS favorite_artists (
        artist_id TEXT PRIMARY KEY,
        added_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS downloads (
        provider TEXT NOT NULL,
        track_id TEXT NOT NULL,
        single INTEGER NOT NULL,
        done INTEGER NOT NULL,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (provider, track_id)
    );
    CREATE TABLE IF NOT EXISTS downloaded_sets (
        provider TEXT NOT NULL,
        kind TEXT NOT NULL,
        set_id TEXT NOT NULL,
        value TEXT NOT NULL,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (provider, kind, set_id)
    );
    CREATE TABLE IF NOT EXISTS lyrics_choices (
        key TEXT PRIMARY KEY,
        saved_at INTEGER NOT NULL,
        value TEXT NOT NULL
    );";

#[derive(Clone)]
pub struct Database {
    path: PathBuf,
    ready: Arc<Mutex<bool>>,
}

impl Database {
    pub fn standard() -> Self {
        let data = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("sonora");
        Self {
            path: data.join("state.sqlite"),
            ready: Arc::new(Mutex::new(false)),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            ready: Arc::new(Mutex::new(false)),
        }
    }

    pub fn open(&self) -> Result<Connection> {
        connect(&self.path, SCHEMA, &self.ready, "app state")
    }

    /// Every lyrics sheet a listener picked by hand, as `(key, json)`.
    pub fn lyrics_choices(&self) -> Result<Vec<(String, String)>> {
        let connection = self.open()?;
        let mut read = connection
            .prepare("SELECT key, value FROM lyrics_choices")
            .context("cannot prepare a lyrics choices read")?;
        let rows = read
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .context("cannot read lyrics choices")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot read lyrics choices")?;
        Ok(rows)
    }

    /// Saves the lyrics sheet a listener picked by hand for the track under `key`.
    pub fn keep_lyrics_choice(&self, key: &str, saved_at: i64, value: &str) -> Result<()> {
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO lyrics_choices (key, saved_at, value) VALUES (?, ?, ?)
                 ON CONFLICT(key) DO UPDATE SET saved_at = excluded.saved_at,
                     value = excluded.value",
                params![key, saved_at, value],
            )
            .context("cannot save a lyrics choice")?;
        Ok(())
    }

    /// Drops the lyrics sheet picked by hand under `key`, if there is one.
    pub fn drop_lyrics_choice(&self, key: &str) -> Result<()> {
        let connection = self.open()?;
        connection
            .execute("DELETE FROM lyrics_choices WHERE key = ?", params![key])
            .context("cannot drop a lyrics choice")?;
        Ok(())
    }
}

/// Opens `path`, applying `schema` the first time this process opens it. `what` names the
/// database in the errors a caller sees.
pub(crate) fn connect(
    path: &Path,
    schema: &str,
    ready: &Mutex<bool>,
    what: &str,
) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create the {what} directory"))?;
    }
    let connection = Connection::open(path).with_context(|| format!("cannot open {what}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .with_context(|| format!("cannot configure {what}"))?;
    let mut ready = ready
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !*ready {
        connection
            .execute_batch(schema)
            .with_context(|| format!("cannot prepare {what}"))?;
        *ready = true;
    }
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::Database;

    #[test]
    fn a_lyrics_choice_is_kept_until_dropped() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let database = Database::at(dir.path().join("state.sqlite"));

        database
            .keep_lyrics_choice("subsonic:abc:lyrics", 10, "{\"first\":true}")
            .expect("the choice saves");
        database
            .keep_lyrics_choice("subsonic:def:lyrics", 11, "{\"second\":true}")
            .expect("the choice saves");

        let mut choices = database.lyrics_choices().expect("the choices read back");
        choices.sort();
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].0, "subsonic:abc:lyrics");

        // Saving the same key again replaces what was there.
        database
            .keep_lyrics_choice("subsonic:abc:lyrics", 12, "{\"again\":true}")
            .expect("the choice saves again");
        let choices = database.lyrics_choices().expect("the choices read back");
        assert_eq!(choices.len(), 2);
        assert!(choices.iter().any(|(_, value)| value.contains("again")));
        assert!(!choices.iter().any(|(_, value)| value.contains("first")));

        database
            .drop_lyrics_choice("subsonic:abc:lyrics")
            .expect("the choice drops");
        let choices = database.lyrics_choices().expect("the choices read back");
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].0, "subsonic:def:lyrics");

        // Dropping what was never saved is not an error.
        database
            .drop_lyrics_choice("subsonic:abc:lyrics")
            .expect("dropping a missing choice is fine");
    }
}
