use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};

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
    CREATE TABLE IF NOT EXISTS favorites (
        track_id TEXT PRIMARY KEY,
        added_at INTEGER NOT NULL
    );";

#[derive(Clone)]
pub struct Database {
    path: PathBuf,
    legacy: Option<Legacy>,
    ready: Arc<Mutex<bool>>,
}

#[derive(Clone)]
struct Legacy {
    config: PathBuf,
    data: PathBuf,
}

impl Database {
    pub fn standard() -> Self {
        let data = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("sonora");
        let config = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("sonora");
        Self {
            path: data.join("state.sqlite"),
            legacy: Some(Legacy { config, data }),
            ready: Arc::new(Mutex::new(false)),
        }
    }

    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            legacy: None,
            ready: Arc::new(Mutex::new(false)),
        }
    }

    pub fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).context("cannot create the state directory")?;
        }
        let connection = Connection::open(&self.path).context("cannot open app state")?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .context("cannot configure app state")?;
        let mut ready = self
            .ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*ready {
            connection
                .execute_batch(SCHEMA)
                .context("cannot prepare app state")?;
            *ready = true;
        }
        Ok(connection)
    }

    /// Imports every pre-state.sqlite database. Each source is independent: a broken legacy
    /// database is retained for diagnosis and does not prevent the other sources from migrating.
    pub fn migrate(&self) {
        let Some(legacy) = &self.legacy else {
            return;
        };
        let sources = [
            (
                legacy.data.join("history.sqlite3"),
                "legacy_history",
                "INSERT OR IGNORE INTO plays (
                    scope, provider, track_id, played_at, name, playable, artists,
                    artist_refs, album, album_id, cover, duration_ms, explicit
                 ) SELECT
                    scope, provider, track_id, played_at, name, playable, artists,
                    artist_refs, album, album_id, cover, duration_ms, explicit
                 FROM legacy_history.plays;",
            ),
            (
                legacy.data.join("flags.sqlite3"),
                "legacy_flags",
                "INSERT OR IGNORE INTO flags (key, value)
                 SELECT key, value FROM legacy_flags.flags;",
            ),
            (
                legacy.config.join("local-playlists.sqlite3"),
                "legacy_local",
                "INSERT OR IGNORE INTO playlists (id, name, modified_at)
                 SELECT id, name, modified_at FROM legacy_local.playlists;
                 INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
                 SELECT playlist_id, track_id, position FROM legacy_local.playlist_tracks;
                 INSERT OR IGNORE INTO favorites (track_id, added_at)
                 SELECT track_id, added_at FROM legacy_local.favorites;",
            ),
        ];

        for (source, alias, copy) in sources {
            if let Err(error) = self.migrate_one(&source, alias, copy) {
                log::warn!(
                    "storage: cannot migrate {} into {}: {error:#}",
                    source.display(),
                    self.path.display()
                );
            }
        }
    }

    fn migrate_one(&self, source: &Path, alias: &str, copy: &str) -> Result<()> {
        if !source.exists() {
            return Ok(());
        }
        let mut connection = self.open()?;
        connection
            .execute(
                &format!("ATTACH DATABASE ? AS {alias}"),
                params![source.to_string_lossy()],
            )
            .context("cannot attach legacy state")?;
        let copied = (|| {
            let transaction = connection.transaction()?;
            transaction.execute_batch(copy)?;
            transaction.commit()
        })()
        .context("cannot copy legacy state");
        connection
            .execute_batch(&format!("DETACH DATABASE {alias}"))
            .context("cannot detach legacy state")?;
        copied?;
        std::fs::remove_file(source).context("cannot remove migrated state")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn scratch() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "sonora-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn all_legacy_databases_are_imported_and_removed() {
        let root = scratch();
        let config = root.join("config");
        let data = root.join("data");
        std::fs::create_dir_all(&config).expect("config directory is created");
        std::fs::create_dir_all(&data).expect("data directory is created");

        let history = data.join("history.sqlite3");
        Connection::open(&history)
            .expect("history opens")
            .execute_batch(
                "CREATE TABLE plays (
                    scope TEXT NOT NULL, provider TEXT NOT NULL, track_id TEXT NOT NULL,
                    played_at INTEGER NOT NULL, name TEXT NOT NULL, playable INTEGER NOT NULL,
                    artists TEXT NOT NULL, artist_refs TEXT NOT NULL, album TEXT NOT NULL,
                    album_id TEXT, cover TEXT, duration_ms INTEGER NOT NULL,
                    explicit INTEGER NOT NULL
                );
                INSERT INTO plays VALUES (
                    'scope', 'provider', 'track', 1, 'Track', 1, 'Artist', '[]',
                    'Album', NULL, NULL, 1000, 0
                );",
            )
            .expect("history is populated");

        let flags = data.join("flags.sqlite3");
        Connection::open(&flags)
            .expect("flags open")
            .execute_batch(
                "CREATE TABLE flags (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
                 INSERT INTO flags VALUES ('flag', 1);",
            )
            .expect("flags are populated");

        let local = config.join("local-playlists.sqlite3");
        Connection::open(&local)
            .expect("local playlists open")
            .execute_batch(
                "CREATE TABLE playlists (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, modified_at INTEGER NOT NULL
                );
                CREATE TABLE playlist_tracks (
                    playlist_id TEXT NOT NULL, track_id TEXT NOT NULL, position INTEGER NOT NULL
                );
                CREATE TABLE favorites (track_id TEXT PRIMARY KEY, added_at INTEGER NOT NULL);
                INSERT INTO playlists VALUES ('list', 'List', 1);
                INSERT INTO playlist_tracks VALUES ('list', 'track', 0);
                INSERT INTO favorites VALUES ('track', 1);",
            )
            .expect("local playlists are populated");

        let database = Database {
            path: data.join("state.sqlite"),
            legacy: Some(Legacy {
                config: config.clone(),
                data: data.clone(),
            }),
            ready: Arc::new(Mutex::new(false)),
        };
        database.migrate();

        assert!(!history.exists());
        assert!(!flags.exists());
        assert!(!local.exists());
        let connection = database.open().expect("state opens");
        for table in [
            "plays",
            "flags",
            "playlists",
            "playlist_tracks",
            "favorites",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("migrated row is counted");
            assert_eq!(count, 1, "{table} was not migrated");
        }
        drop(connection);

        std::fs::remove_file(database.path).expect("state database is removed");
        std::fs::remove_dir(config).expect("config directory is removed");
        std::fs::remove_dir(data).expect("data directory is removed");
        std::fs::remove_dir(root).expect("test directory is removed");
    }
}
