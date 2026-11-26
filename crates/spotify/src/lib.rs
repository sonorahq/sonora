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
mod wire;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::playback::Factory;
use music::{
    Capabilities, MusicApi as _, MusicProvider, PlaybackFactory, ProviderSession, RadioSeed, Shape,
};

pub use auth::AuthConfig;
pub use client::SpotifyClient;
pub use lyrics::SpotifyLyrics;

pub struct SpotifyProvider {
    account: Arc<auth::Account>,
    browser_confirmation: fn() -> String,
}

impl SpotifyProvider {
    /// Selects a smaller Spotify album cover when it fits the physical thumbnail size.
    pub fn thumbnail_at(cover: &str, edge: u32) -> Option<String> {
        wire::thumbnail_at(cover, edge)
    }

    pub fn new(config: AuthConfig) -> Self {
        Self {
            account: Arc::new(auth::Account::new(config)),
            browser_confirmation: String::new,
        }
    }

    pub fn from_env() -> Self {
        Self::new(AuthConfig::from_env())
    }

    /// Supplies the browser confirmation in the application's current language.
    pub fn with_browser_confirmation(mut self, message: fn() -> String) -> Self {
        self.browser_confirmation = message;
        self
    }

    /// Looks up lyrics with the same Spotify session as playback and browsing.
    pub fn lyrics(&self) -> SpotifyLyrics {
        SpotifyLyrics::with_account(self.account.clone())
    }

    async fn session(&self, session: session::Session) -> Result<ProviderSession> {
        let client = SpotifyClient::new(session.clone());
        let profile = client.profile().await?;
        let playback = Arc::new(Factory::new(session, client.connect_handle()));
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
        Some(Box::pin(spotifyapis::reachable()))
    }

    fn public_art(&self) -> bool {
        true
    }

    fn soloist(&self) -> bool {
        soloist::supported()
    }

    fn link(&self, link: &str) -> Option<music::link::Linked> {
        links::read(link)
    }

    fn sign_in_options(&self) -> Vec<music::SignIn> {
        vec![music::SignIn::Default]
    }

    fn abandon(&self) {
        self.account.cancel_login();
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
        self.session(session).await.map(Some)
    }

    async fn sign_in(
        &self,
        _method: music::SignIn,
        prompt: music::PromptSink,
        _input: music::InputSource,
    ) -> Result<ProviderSession> {
        let message = (self.browser_confirmation)();
        let session = self.account.login(prompt, &message).await?;
        self.session(session).await
    }

    /// Clears Sonora's Spotify login. Soloist keeps its separately paired login.
    fn sign_out(&self) {
        self.account.forget();
    }
}
