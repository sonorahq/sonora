use std::time::Duration;

use gpui::{App, Context, Entity, EventEmitter, Task};

use crate::library::Shelf;
use crate::session::Session;
use crate::{AppSettings, Io, Sonora};

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

/// Raised when the network came back, so a screen that gave up can load again.
pub struct Reconnected;

/// Whether the app can reach the network at all, as one answer for the whole app rather than
/// one per screen. It goes offline on the first failure that reads as a lost connection and
/// comes back either when something reaches the network or when its own check does. Offline
/// mode reads as a network that is gone for as long as the listener keeps it on.
pub struct Network {
    session: Entity<Session>,
    settings: Entity<AppSettings>,
    io: Io,
    lost: bool,
    /// Whether the listener turned offline mode on.
    offline: bool,
    probe: Option<Task<()>>,
}

impl EventEmitter<Reconnected> for Network {}

impl Network {
    pub fn new(session: Entity<Session>, settings: Entity<AppSettings>, io: Io, cx: &App) -> Self {
        let offline = settings.read(cx).offline_mode();
        Self {
            session,
            settings,
            io,
            lost: false,
            offline,
            probe: None,
        }
    }

    pub fn global(cx: &App) -> Entity<Self> {
        Sonora::global(cx).network.clone()
    }

    /// Whether the network is gone, or offline mode is on. Every screen that needs one asks this
    /// rather than waiting for its own load to fail.
    pub fn lost(cx: &App) -> bool {
        match cx.try_global::<Sonora>() {
            Some(sonora) => {
                let network = sonora.network.read(cx);
                network.lost || network.offline
            }
            None => false,
        }
    }

    /// Whether the listener turned offline mode on, which a screen tells apart from a lost
    /// connection by offering to go online rather than to try again.
    pub fn offline(cx: &App) -> bool {
        cx.try_global::<Sonora>()
            .is_some_and(|sonora| sonora.network.read(cx).offline)
    }

    /// Whether offline mode keeps the lists of `shelf` to what was downloaded. Local files need
    /// no network, so their shelf keeps everything.
    pub fn downloads_only(shelf: Shelf, cx: &App) -> bool {
        !shelf.local() && Self::offline(cx)
    }

    /// Turns offline mode on or off and remembers it for the next launch. Going offline lets go of
    /// the live account. Going online with the network there raises [`Reconnected`], so every
    /// screen and the account load again.
    pub fn set_offline(&mut self, offline: bool, cx: &mut Context<Self>) {
        if self.offline == offline {
            return;
        }
        log::info!(
            "network: offline mode is {}",
            match offline {
                true => "on",
                false => "off",
            }
        );
        self.offline = offline;
        self.settings
            .update(cx, |settings, cx| settings.set_offline_mode(offline, cx));
        cx.notify();
        cx.refresh_windows();
        match offline {
            true => self
                .session
                .update(cx, |session, cx| session.go_offline(cx)),
            false if !self.lost => cx.emit(Reconnected),
            false => {}
        }
    }

    /// Notes a failed call. Only a reason that reads as a lost connection puts the app offline;
    /// anything the provider refused leaves it alone.
    pub fn failed(reason: &str, cx: &mut App) {
        if !music::trouble::offline(reason) {
            return;
        }
        let Some(network) = cx
            .try_global::<Sonora>()
            .map(|sonora| sonora.network.clone())
        else {
            return;
        };
        network.update(cx, |this, cx| {
            if this.lost {
                return;
            }
            log::warn!("network: the connection is gone: {reason}");
            this.lost = true;
            this.watch(cx);
            cx.notify();
            cx.refresh_windows();
        });
    }

    /// Notes that something reached the network, which is the quickest way back online.
    pub fn reached(cx: &mut App) {
        let Some(network) = cx
            .try_global::<Sonora>()
            .map(|sonora| sonora.network.clone())
        else {
            return;
        };
        network.update(cx, |this, cx| this.found(cx));
    }

    fn found(&mut self, cx: &mut Context<Self>) {
        if !self.lost {
            return;
        }
        log::info!("network: the connection is back");
        self.lost = false;
        self.probe = None;
        cx.notify();
        cx.refresh_windows();
        // In offline mode nothing is let back on the network until the listener goes online.
        if !self.offline {
            cx.emit(Reconnected);
        }
    }

    /// Opens a connection to the active provider's host until one succeeds, waiting longer
    /// between tries as they keep failing. Nothing is asked of the host but the connection, and
    /// it is one the app talks to anyway, so no other service ever learns Sonora is running. A
    /// provider that needs no network hands back no host, and then nothing is tried at all.
    fn watch(&mut self, cx: &mut Context<Self>) {
        let io = self.io.clone();

        self.probe = Some(cx.spawn(async move |this, cx| {
            for tried in 0.. {
                let wait = PROBE[tried.min(PROBE.len() - 1)];
                cx.background_executor().timer(wait).await;

                let Ok(host) = this.update(cx, |this, cx| this.session.read(cx).reach()) else {
                    return;
                };
                let Some(host) = host else {
                    continue;
                };
                let reached = io
                    .spawn(async move {
                        let connect = tokio::net::TcpStream::connect((host.as_str(), PORT));
                        match tokio::time::timeout(PATIENCE, connect).await {
                            Ok(answer) => answer.is_ok(),
                            Err(_) => false,
                        }
                    })
                    .await;
                if matches!(reached, Ok(true)) {
                    this.update(cx, |this, cx| this.found(cx)).ok();
                    return;
                }
            }
        }));
    }
}
