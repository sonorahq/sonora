use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as SyncMutex, RwLock, RwLockReadGuard};

use anyhow::{Context as _, Result};
use reqwest::Url;
use tokio::sync::OnceCell;

use super::auth::OAuth;
use super::collection;
use super::playback::AudioKeys;
use super::playlists::Summary;
use super::transport::{self, Endpoints};

/// An OAuth-authenticated Spotify account shared by API and playback consumers.
#[derive(Clone)]
pub struct Session(Arc<Inner>);

/// Account state, service discovery and the optional native audio connection.
struct Inner {
    oauth: OAuth,
    audio_keys: AudioKeys,
    device_id: String,
    session_id: String,
    connection_id: RwLock<String>,
    endpoints: OnceCell<Endpoints>,
    display_name: OnceCell<String>,
    soloist_playback: Arc<AtomicBool>,
    metadata: collection::Cache,
    playlist_summaries: SyncMutex<HashMap<String, Summary>>,
}

impl Session {
    pub(super) fn new(oauth: OAuth, device_id: String) -> Self {
        Self(Arc::new(Inner {
            oauth,
            audio_keys: AudioKeys::default(),
            device_id,
            session_id: format!("{:032x}", fastrand::u128(..)),
            connection_id: RwLock::default(),
            endpoints: OnceCell::new(),
            display_name: OnceCell::new(),
            soloist_playback: Arc::default(),
            metadata: collection::Cache::default(),
            playlist_summaries: SyncMutex::default(),
        }))
    }

    pub(super) fn audio_keys(&self) -> &AudioKeys {
        &self.0.audio_keys
    }

    /// Shares whether Sonora queued the playback on its Soloist daemon.
    pub(super) fn soloist_playback(&self) -> &Arc<AtomicBool> {
        &self.0.soloist_playback
    }

    pub(super) fn metadata(&self) -> &collection::Cache {
        &self.0.metadata
    }

    pub(super) fn playlist_summaries(&self) -> &SyncMutex<HashMap<String, Summary>> {
        &self.0.playlist_summaries
    }

    /// The canonical username returned by Spotify Accounts at sign-in.
    pub(super) fn username(&self) -> &str {
        self.0.oauth.username()
    }

    /// Reuses the account's sign-in name when rendering its playlist cards.
    pub(super) fn display_name(&self) -> Option<&str> {
        self.0.display_name.get().map(String::as_str)
    }

    pub(super) fn remember_display_name(&self, name: &str) {
        if name != self.username() && self.display_name().is_none() {
            let _ = self.0.display_name.set(name.to_owned());
        }
    }

    pub(super) fn device_id(&self) -> &str {
        &self.0.device_id
    }

    pub(super) fn session_id(&self) -> &str {
        &self.0.session_id
    }

    pub(super) fn client_id(&self) -> &str {
        self.0.oauth.client_id()
    }

    pub(super) fn connection_id(&self) -> RwLockReadGuard<'_, String> {
        self.0
            .connection_id
            .read()
            .expect("Spotify Dealer identity lock poisoned")
    }

    pub(super) fn set_connection_id(&self, id: &str) {
        id.clone_into(
            &mut self
                .0
                .connection_id
                .write()
                .expect("Spotify Dealer identity lock poisoned"),
        );
    }

    /// Picks a regional host or the global host supplied in the discovery response.
    pub(super) async fn api_url(&self) -> Result<Url> {
        let hosts = &self.endpoints().await?.spclient;
        hosts
            .regional
            .iter()
            .chain(&hosts.global)
            .find_map(|address| transport::regional_url("https", address))
            .context("Spotify discovery returned no API host")
    }

    /// Returns the Dealer hosts supplied in the discovery response.
    pub(super) async fn dealer_urls(&self) -> Result<Vec<Url>> {
        Ok(self
            .endpoints()
            .await?
            .dealer
            .iter()
            .filter_map(|address| transport::regional_url("wss", address))
            .collect())
    }

    pub(super) async fn endpoints(&self) -> Result<&Endpoints> {
        self.0
            .endpoints
            .get_or_try_init(transport::discover)
            .await
            .context("Spotify endpoint discovery failed")
    }

    pub(super) fn has_current_grant(&self) -> bool {
        self.0.oauth.has_current_grant()
    }

    /// Gets a bearer for direct Spotify requests from the account's OAuth grant.
    pub(super) async fn access_token(&self) -> Result<String> {
        self.0.oauth.token().await
    }
}
