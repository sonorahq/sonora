use std::time::Duration;

use crate::library::Shelf;
use crate::{AppSettings, Change, Cx, Event, Job, Store, Topic};

/// How long to wait before each check of whether the network is back. The last one repeats
/// for as long as the network stays gone.
const PROBE: [Duration; 4] = [
    Duration::from_secs(3),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];
/// How long a check may take before it counts as a failure.
const PATIENCE: Duration = Duration::from_secs(4);
/// The port a check connects on, which is the one every provider is reached over.
const PORT: u16 = 443;

/// Whether the app can reach the network at all, as one answer for the whole app rather than
/// one per screen. It goes offline on the first failure that reads as a lost connection and
/// comes back either when something reaches the network or when its own check does. Offline
/// mode reads as a network that is gone for as long as the listener keeps it on.
pub struct Network {
    lost: bool,
    /// Whether the listener turned offline mode on.
    offline: bool,
    probe: Option<Job>,
}

impl Network {
    pub fn new(settings: &AppSettings) -> Self {
        Self {
            lost: false,
            offline: settings.offline_mode(),
            probe: None,
        }
    }

    /// Whether the network is gone, or offline mode is on. Every screen that needs one asks this
    /// rather than waiting for its own load to fail.
    pub fn lost(&self) -> bool {
        self.lost || self.offline
    }

    /// Whether the listener turned offline mode on, which a screen tells apart from a lost
    /// connection by offering to go online rather than to try again.
    pub fn offline(&self) -> bool {
        self.offline
    }

    /// Whether offline mode keeps the lists of `shelf` to what was downloaded. Local files need
    /// no network, so their shelf keeps everything.
    pub fn downloads_only(&self, shelf: Shelf) -> bool {
        !shelf.local() && self.offline
    }

    /// Notes a failed call. Only a reason that reads as a lost connection puts the app offline;
    /// anything the provider refused leaves it alone.
    fn failed(&mut self, reason: &str, cx: &mut Cx) {
        if !music::trouble::offline(reason) || self.lost {
            return;
        }
        log::warn!("network: the connection is gone: {reason}");
        self.lost = true;
        self.watch(cx);
        cx.changed(Topic::Network);
    }

    fn found(&mut self, cx: &mut Cx) {
        if !self.lost {
            return;
        }
        log::info!("network: the connection is back");
        self.lost = false;
        self.probe = None;
        cx.changed(Topic::Network);
        // In offline mode nothing is let back on the network until the listener goes online.
        if !self.offline {
            cx.emit(Event::Reconnected);
        }
    }

    /// Opens a connection to the active provider's host until one succeeds, waiting longer
    /// between tries as they keep failing. Nothing is asked of the host but the connection, and
    /// it is one the app talks to anyway, so no other service ever learns Sonora is running. A
    /// provider that needs no network hands back no host, and then nothing is tried at all.
    fn watch(&mut self, cx: &mut Cx) {
        self.probe = Some(cx.spawn(|link| async move {
            for tried in 0.. {
                let wait = PROBE[tried.min(PROBE.len() - 1)];
                tokio::time::sleep(wait).await;

                let Some(host) = link.read(|store| store.session.reach()).await else {
                    return;
                };
                let Some(host) = host else {
                    continue;
                };
                let connect = tokio::net::TcpStream::connect((host.as_str(), PORT));
                let reached = matches!(tokio::time::timeout(PATIENCE, connect).await, Ok(Ok(_)));
                if reached {
                    link.post(|store| store.network.found(&mut store.cx));
                    return;
                }
            }
        }));
    }
}

impl Store {
    /// Turns offline mode on or off and remembers it for the next launch. Going offline lets go of
    /// the live account. Going online with the network there raises [`Event::Reconnected`], so
    /// every screen and the account load again.
    pub fn set_offline(&mut self, offline: bool) {
        let network = &mut self.network;
        if network.offline == offline {
            return;
        }
        log::info!(
            "network: offline mode is {}",
            match offline {
                true => "on",
                false => "off",
            }
        );
        network.offline = offline;
        self.settings.set_offline_mode(offline, &mut self.cx);
        self.cx.changed(Topic::Network);
        match offline {
            true => self.session.go_offline(&mut self.cx),
            false if !self.network.lost => self.cx.emit(Event::Reconnected),
            false => {}
        }
    }

    pub(crate) fn react_network(&mut self, change: &Change) {
        match change {
            Change::Event(Event::NetworkFailed(reason)) => {
                self.network.failed(reason, &mut self.cx)
            }
            Change::Event(Event::NetworkReached) => self.network.found(&mut self.cx),
            _ => {}
        }
    }
}
