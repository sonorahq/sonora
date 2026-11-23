use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as SyncMutex, RwLock, RwLockReadGuard};

use bytes::Bytes;
use tokio::sync::OnceCell;

use spotifyapis::auth::OAuth;

use super::collection;
use spotifyapis::access_point::AudioKeys;

/// An OAuth-authenticated Spotify account shared by API and playback consumers.
#[derive(Clone)]
pub struct Session(Arc<Inner>);

/// Account state, service discovery and the optional native audio connection.
struct Inner {
    api: Arc<spotifyapis::Session>,
    audio_keys: AudioKeys,
    device_id: String,
    session_id: String,
    connection_id: RwLock<String>,
    display_name: OnceCell<String>,
    soloist_playback: Arc<AtomicBool>,
    metadata: collection::Cache,
    rootlist_revision: SyncMutex<Option<Bytes>>,
}

impl Session {
    pub(super) fn new(oauth: Arc<OAuth>, device_id: String) -> Self {
        let api = Arc::new(spotifyapis::Session::new(oauth));
        Self(Arc::new(Inner {
            audio_keys: AudioKeys::new(api.clone(), device_id.clone()),
            api,
            device_id,
            session_id: format!("{:032x}", fastrand::u128(..)),
            connection_id: RwLock::default(),
            display_name: OnceCell::new(),
            soloist_playback: Arc::default(),
            metadata: collection::Cache::default(),
            rootlist_revision: SyncMutex::default(),
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

    /// Records the revision of the complete playlist library, if every page agreed on it.
    pub(super) fn remember_rootlist_revision(&self, revision: Option<Bytes>) {
        *self
            .0
            .rootlist_revision
            .lock()
            .expect("Spotify root-list lock poisoned") =
            revision.filter(|revision| !revision.is_empty());
    }

    pub(super) fn has_rootlist_revision(&self, revision: &[u8]) -> bool {
        self.0
            .rootlist_revision
            .lock()
            .expect("Spotify root-list lock poisoned")
            .as_deref()
            == Some(revision)
    }

    /// The canonical username returned by Spotify Accounts at sign-in.
    pub(super) fn username(&self) -> &str {
        self.api().username()
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

    pub(super) fn api(&self) -> &Arc<spotifyapis::Session> {
        &self.0.api
    }
}
