mod client;
mod id3;
mod index;
mod lyrics;
mod m3u;
mod playback;
mod scan;
mod store;
mod tags;
mod wire;

pub use lyrics::LocalLyrics;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use storage::{Cache, Database};

use crate::{
    Capabilities, Catalog, InputSource, MusicApi, MusicProvider, PlaybackFactory, PromptSink,
    ProviderSession, Shape, SignIn, Track, UserProfile,
};

pub struct LocalProvider {
    cache_dir: PathBuf,
    database: Database,
    index: index::Index,
}

impl LocalProvider {
    pub fn new(cache_dir: PathBuf, database: Database, cache: Cache) -> Self {
        Self {
            cache_dir,
            database,
            index: index::Index::new(cache),
        }
    }

    async fn scan_paths(
        &self,
        paths: Vec<PathBuf>,
        ticket: crate::progress::Ticket,
        thorough: bool,
    ) -> Result<ProviderSession> {
        let cache_dir = self.cache_dir.clone();
        let index = self.index.clone();
        let roots = paths.clone();
        let mut scanned = tokio::task::spawn_blocking(move || {
            scan::reserved(&paths, &cache_dir, &index, ticket, thorough)
        })
        .await
        .context("local scan task panicked")?
        .context("local scan was cancelled")?;
        anyhow::ensure!(ticket.live(), "local scan was cancelled");

        let active = |id: &String| {
            wire::path_from_track_id(id)
                .is_some_and(|path| roots.iter().any(|root| path.starts_with(root)))
        };
        scanned
            .report
            .moved
            .retain(|(from, to)| active(from) && active(to));
        scanned.report.moved_albums.retain(|(from, to)| {
            scanned
                .report
                .album_move_paths
                .get(from)
                .is_some_and(|paths| !paths.is_empty() && paths.iter().all(&active))
                && scanned.albums.iter().any(|album| album.id == *to)
        });
        let store = store::Store::new(self.database.clone());
        store.reconcile(&scanned.report, &scanned.tracks)?;
        scanned
            .report
            .issues
            .extend(store.missing_tracks(&scanned.tracks, &roots)?);
        let api: Arc<dyn MusicApi> = Arc::new(client::LocalClient::new(
            scanned,
            self.database.clone(),
            self.index.clone(),
        ));
        let playback: Arc<dyn PlaybackFactory> = Arc::new(playback::Factory);

        Ok(ProviderSession {
            profile: UserProfile {
                id: "local".to_owned(),
                display_name: "Local Files".to_owned(),
                avatar: None,
            },
            api,
            playback,
            shape: Shape::Catalog(Catalog::Fixed),
            authenticated: false,
            // Files on disk: favorites are kept here, but nothing suggests a station and
            // nothing counts a play.
            capabilities: Capabilities {
                follow_artists: true,
                radio: false,
                playcounts: false,
                pins: false,
                transcode: false,
                playlist_covers: true,
                keeps: false,
                downloads: false,
                qualities: &[],
                download_gap: Duration::ZERO,
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

    fn playback_factory(&self) -> Option<Arc<dyn PlaybackFactory>> {
        Some(Arc::new(playback::Factory))
    }

    fn track_from_path(&self, path: &Path) -> Option<Track> {
        wire::track_from_file(path, None, None, &self.cache_dir).map(|tagged| tagged.track)
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
        self.scan_paths(paths, crate::progress::reserve(), false)
            .await
    }

    async fn scan(
        &self,
        paths: Vec<PathBuf>,
        ticket: crate::progress::Ticket,
        thorough: bool,
    ) -> Result<ProviderSession> {
        self.scan_paths(paths, ticket, thorough).await
    }

    fn sign_out(&self) {}
}
