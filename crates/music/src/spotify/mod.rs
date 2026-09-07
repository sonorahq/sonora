pub mod auth;

mod albums;
mod artists;
mod client;
mod collection;
mod collection2;
mod lyrics;
mod pathfinder;
mod pb;
mod playback;
mod playlists;
mod profiles;
mod radio;
mod search;
mod sink;
mod wire;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::hybrid::HybridFactory;
use crate::spotify::playback::Factory;
use crate::{MusicApi, MusicProvider, PlaybackFactory, ProviderSession};

pub use auth::AuthConfig;
pub use client::LibrespotClient;

pub struct SpotifyProvider {
    config: AuthConfig,
    youtube: Arc<ytmusic::YtMusic>,
}

impl SpotifyProvider {
    pub fn new(config: AuthConfig, youtube: Arc<ytmusic::YtMusic>) -> Self {
        Self { config, youtube }
    }

    pub fn from_env(youtube: Arc<ytmusic::YtMusic>) -> Self {
        Self::new(AuthConfig::from_env(), youtube)
    }

    async fn session(&self, client: LibrespotClient, premium: bool) -> Result<ProviderSession> {
        let profile = client.profile().await?;
        let session = client.session().clone();
        let api: Arc<dyn MusicApi> = Arc::new(client);

        let playback: Arc<dyn PlaybackFactory> = if premium {
            Arc::new(Factory::new(session))
        } else {
            log::info!(
                "spotify: this account has no Premium; playing its tracks through YouTube Music"
            );
            Arc::new(HybridFactory::new(api.clone(), self.youtube.clone()))
        };

        Ok(ProviderSession {
            profile,
            api,
            playback,
            authenticated: true,
            playcounts: true,
        })
    }
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
    fn name(&self) -> &'static str {
        "Spotify"
    }

    fn slug(&self) -> &'static str {
        "spotify"
    }

    fn sign_in_options(&self) -> Vec<crate::SignIn> {
        vec![crate::SignIn::Default]
    }

    fn abandon(&self) {
        auth::release(&self.config);
    }

    fn stored(&self) -> bool {
        self.config.file().exists()
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        let Some(auth::Connected { session, premium }) = auth::restore(&self.config).await? else {
            return Ok(None);
        };
        self.session(LibrespotClient::new(session), premium)
            .await
            .map(Some)
    }

    async fn sign_in(
        &self,
        _method: crate::SignIn,
        prompt: crate::PromptSink,
        _input: crate::InputSource,
    ) -> Result<ProviderSession> {
        let auth::Connected { session, premium } = auth::login(&self.config, prompt).await?;
        self.session(LibrespotClient::new(session), premium).await
    }

    fn sign_out(&self) {
        auth::forget(&self.config);
    }
}
