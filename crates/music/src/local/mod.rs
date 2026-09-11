mod client;
mod id3;
mod playback;
mod scan;
mod store;
mod tags;
mod wire;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use storage::Database;

use self::store::Store;
use crate::{
    InputSource, MusicApi, MusicProvider, PlaybackFactory, PromptSink, ProviderSession, Shape,
    SignIn, UserProfile,
};

pub struct LocalProvider {
    cache_dir: PathBuf,
    database: Database,
}

impl LocalProvider {
    pub fn new(cache_dir: PathBuf, database: Database) -> Self {
        Self {
            cache_dir,
            database,
        }
    }

    async fn scan_paths(&self, paths: Vec<PathBuf>) -> Result<ProviderSession> {
        let cache_dir = self.cache_dir.clone();
        let cache = Store::new(self.database.clone());
        let scanned = tokio::task::spawn_blocking(move || scan::scan(&paths, &cache_dir, &cache))
            .await
            .context("local scan task panicked")?;
        Ok(self.session_from(scanned))
    }

    /// A session built purely from what was cached for `paths` last time, with no filesystem
    /// access at all — near-instant, though possibly stale until [`Self::scan_paths`]
    /// reconciles it. `None` if nothing has been cached for these paths yet.
    fn restore_paths(&self, paths: &[PathBuf]) -> Option<ProviderSession> {
        let cache = Store::new(self.database.clone());
        let scanned = scan::scan_cached(paths, &cache)?;
        Some(self.session_from(scanned))
    }

    fn session_from(&self, scanned: scan::Scanned) -> ProviderSession {
        let api: Arc<dyn MusicApi> =
            Arc::new(client::LocalClient::new(scanned, self.database.clone()));
        let playback: Arc<dyn PlaybackFactory> = Arc::new(playback::Factory);

        ProviderSession {
            profile: UserProfile {
                id: "local".to_owned(),
                display_name: "Local Files".to_owned(),
            },
            api,
            playback,
            shape: Shape::Catalog,
            authenticated: false,
            playcounts: false,
        }
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

    fn restore_cached(&self, paths: &[PathBuf]) -> Option<ProviderSession> {
        self.restore_paths(paths)
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
