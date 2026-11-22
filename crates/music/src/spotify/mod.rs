pub mod auth;

mod albums;
mod artists;
mod client;
mod collection;
mod connect;
mod feeds;
mod links;
mod lyrics;
mod playback;
mod playlists;
mod profiles;
mod search;
mod session;
mod transport;
mod wire;

mod proto;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::spotify::playback::Factory;
use crate::{
    Capabilities, MusicApi as _, MusicProvider, PlaybackFactory, ProviderSession, RadioSeed, Shape,
};

pub use auth::AuthConfig;
pub use client::SpotifyClient;
pub use lyrics::SpotifyLyrics;

pub struct SpotifyProvider {
    account: Arc<auth::Account>,
}

impl SpotifyProvider {
    pub fn new(config: AuthConfig) -> Self {
        Self {
            account: Arc::new(auth::Account::new(config)),
        }
    }

    pub fn from_env() -> Self {
        Self::new(AuthConfig::from_env())
    }

    /// Looks up lyrics with the same Spotify session as playback and browsing.
    pub fn lyrics(&self) -> SpotifyLyrics {
        SpotifyLyrics::with_account(self.account.clone())
    }

    async fn session(&self, client: SpotifyClient) -> Result<ProviderSession> {
        let profile = client.profile().await?;
        let playback = Arc::new(Factory::new(client.session().clone()));
        Ok(ProviderSession {
            profile,
            api: Arc::new(client),
            playback,
            shape: Shape::Saved,
            authenticated: true,
            capabilities: Capabilities {
                collection_radio: &[RadioSeed::Album, RadioSeed::Playlist, RadioSeed::Artist],
                pins: true,
                playlist_covers: true,
                playlist_order: true,
                download_gap: playback::DOWNLOAD_GAP,
                qualities: playback::QUALITIES,
                ..Capabilities::ALL
            },
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

    fn network_probe(&self) -> Option<futures::future::BoxFuture<'static, bool>> {
        Some(Box::pin(async { transport::discover().await.is_ok() }))
    }

    fn public_art(&self) -> bool {
        true
    }

    fn soloist(&self) -> bool {
        crate::soloist::supported()
    }

    fn link(&self, link: &str) -> Option<crate::link::Linked> {
        links::read(link)
    }

    fn sign_in_options(&self) -> Vec<crate::SignIn> {
        vec![crate::SignIn::Default]
    }

    fn abandon(&self) {
        auth::release(&self.account.config);
    }

    fn stored(&self) -> bool {
        self.account.config.stored()
    }

    /// Plays vault downloads directly through Sonora's shared audio engine.
    fn offline_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        if !self.stored() {
            return None;
        }
        Some(Arc::new(playback::OfflineFactory))
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        let Some(session) = self.account.session().await? else {
            return Ok(None);
        };
        self.session(SpotifyClient::new(session)).await.map(Some)
    }

    async fn sign_in(
        &self,
        _method: crate::SignIn,
        prompt: crate::PromptSink,
        _input: crate::InputSource,
    ) -> Result<ProviderSession> {
        let session = self.account.login(prompt).await?;
        self.session(SpotifyClient::new(session)).await
    }

    /// Forgets the account and the Soloist pairing with it, since that pairing is this
    /// account's sign-in too.
    fn sign_out(&self) {
        self.account.forget();
        if let Err(error) = crate::soloist::forget() {
            log::warn!("spotify: cannot remove the soloist setup: {error:#}");
        }
    }
}
