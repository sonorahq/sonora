mod cache;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::Connection;

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
