mod client;
mod id3;
mod index;
mod lyrics;
mod playback;
mod playlists;
mod scan;
mod store;
mod tags;
mod wire;

pub use lyrics::LocalLyrics;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use storage::{Cache, Database};

use crate::{
    Capabilities, InputSource, MusicApi, MusicProvider, PlaybackFactory, PlaylistImportSummary,
    PromptSink, ProviderSession, Shape, SignIn, UserProfile,
};

pub struct LocalProvider {
    cache_dir: PathBuf,
    database: Database,
    index: index::Index,
    last_playlist_import: Mutex<Option<PlaylistImportSummary>>,
}

impl LocalProvider {
    pub fn new(cache_dir: PathBuf, database: Database, cache: Cache) -> Self {
        Self {
            cache_dir,
            database,
            index: index::Index::new(cache),
            last_playlist_import: Mutex::new(None),
        }
    }

    async fn scan_paths(&self, paths: Vec<PathBuf>) -> Result<ProviderSession> {
        let cache_dir = self.cache_dir.clone();
        let index = self.index.clone();
        let database = self.database.clone();
        let scanned = tokio::task::spawn_blocking(move || {
            let scanned = scan::scan(&paths, &cache_dir, &index);
            let store = store::Store::new(database);
            let imported = playlists::import(&scanned.playlists, &scanned.tracks, &store);
            (scanned, imported)
        })
        .await
        .context("local scan task panicked")?;
        let (scanned, imported) = scanned;
        log::debug!(
            "local: found {} playlist files, imported {} ({} tracks, {} unmatched)",
            scanned.playlists.len(),
            imported.playlists,
            imported.tracks,
            imported.unmatched,
        );
        *self
            .last_playlist_import
            .lock()
            .unwrap_or_else(|held| held.into_inner()) = Some(imported);

        let api: Arc<dyn MusicApi> = Arc::new(client::LocalClient::new(
            scanned,
            self.database.clone(),
            self.cache_dir.clone(),
            self.index.clone(),
        ));
        let playback: Arc<dyn PlaybackFactory> = Arc::new(playback::Factory);

        Ok(ProviderSession {
            profile: UserProfile {
                id: "local".to_owned(),
                display_name: "Local Files".to_owned(),
            },
            api,
            playback,
            shape: Shape::Catalog,
            authenticated: false,
            // Files on disk: favorites are kept here, but nothing suggests a station and
            // nothing counts a play.
            capabilities: Capabilities {
                follow_artists: true,
                radio: false,
                playcounts: false,
                library: false,
                pins: false,
            },
        })
    }
}

#[async_trait]
impl MusicProvider for LocalProvider {
    fn name(&self) -> &'static str {
        "Local Files"
    }

    fn slug(&self) -> &'static str {
        "local"
    }

    fn forget_scan(&self) {
        self.index.distrust();
    }

    fn playlists_imported(&self) -> Option<PlaylistImportSummary> {
        self.last_playlist_import
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .take()
    }

    fn listening_to(&self) -> &'static str {
        "Local Music"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        Vec::new()
    }

    fn stored(&self) -> bool {
        false
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        Ok(None)
    }

    async fn sign_in(
        &self,
        method: SignIn,
        _prompt: PromptSink,
        _input: InputSource,
    ) -> Result<ProviderSession> {
        let SignIn::Path(paths) = method else {
            return Err(anyhow!(
                "local files can only be configured with a folder path"
            ));
        };
        self.scan_paths(paths).await
    }

    fn sign_out(&self) {}
}
