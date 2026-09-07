mod client;
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

use crate::{
    InputSource, MusicApi, MusicProvider, PlaybackFactory, PromptSink, ProviderSession, SignIn,
    UserProfile,
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

    async fn scan_path(&self, path: PathBuf) -> Result<ProviderSession> {
        let cache_dir = self.cache_dir.clone();
        let scanned = tokio::task::spawn_blocking(move || scan::scan(&path, &cache_dir))
            .await
            .context("local scan task panicked")?;

        let api: Arc<dyn MusicApi> =
            Arc::new(client::LocalClient::new(scanned, self.database.clone()));
        let playback: Arc<dyn PlaybackFactory> = Arc::new(playback::Factory);

        Ok(ProviderSession {
            profile: UserProfile {
                id: "local".to_owned(),
                display_name: "Local Files".to_owned(),
            },
            api,
            playback,
            authenticated: false,
            playcounts: false,
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
        let SignIn::Path(path) = method else {
            return Err(anyhow!(
                "local files can only be configured with a folder path"
            ));
        };
        self.scan_path(path).await
    }

    fn sign_out(&self) {}
}
