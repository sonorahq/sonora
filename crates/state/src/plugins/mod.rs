pub mod auth;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use control::channel::Client;
use gpui::prelude::*;
use gpui::{App, Context, Entity, Global};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub use auth::Auth;

use crate::{AppSettings, Io, Reloaded, Sonora, attach_control};

/// Where control plugins listen: loopback only, never LAN.
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// How long a stopping plugin has to drain before its task is aborted.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// What a plugin serves with: a control client and the session auth. Cloneable across tasks.
#[derive(Clone)]
pub struct PluginContext {
    pub client: Client,
    pub auth: Auth,
    /// Whether this serve requires the session token. False leaves the loopback listener
    /// open to any local client; the user opted out knowing that.
    pub auth_required: bool,
}

/// A built-in control transport. The manager owns the compile-time registry; there is no
/// dynamic loading. `serve` owns all its connection tasks, so aborting the serve task ends
/// everything the plugin started.
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// Stable id, e.g. `rest` or `ws`. Used for status lookups and logs.
    fn id(&self) -> &'static str;

    /// Serves `listener` until `shutdown` fires, then closes its connections and returns.
    /// Only ever touches Sonora through `ctx`.
    async fn serve(
        &self,
        ctx: PluginContext,
        listener: tokio::net::TcpListener,
        shutdown: oneshot::Receiver<()>,
    );
}

/// One registered plugin with its settings wiring.
pub struct Registration {
    /// The transport to serve.
    pub plugin: Arc<dyn Plugin>,
    /// The enable flag this plugin follows.
    pub enabled: fn(&AppSettings) -> bool,
    /// The loopback port this plugin binds. Zero asks the OS for one.
    pub port: fn(&AppSettings) -> u16,
    /// Whether this plugin requires the session token.
    pub auth_required: fn(&AppSettings) -> bool,
}

/// The listener state of one plugin, for settings.
#[derive(Clone, Debug)]
pub enum PluginStatus {
    Stopped,
    Running { addr: SocketAddr },
    Failed { reason: String },
}

struct Attached {
    _plugins: Entity<Plugins>,
}

impl Global for Attached {}

/// Attaches the plugin manager with the built-in transports and reconciles them with
/// settings. Safe to call twice: the second call keeps the existing manager.
pub fn attach_plugins(cx: &mut App, plugins: Vec<Registration>) {
    if cx.has_global::<Attached>() {
        return;
    }
    let sonora = Sonora::global(cx);
    let settings = sonora.settings.clone();
    let plugins =
        cx.new(|cx| Plugins::new(Io::global(cx), settings, attach_control(cx), plugins, cx));
    cx.set_global(Attached { _plugins: plugins });
}

/// The plugin manager: owns the built-in control plugins and their listeners. Observes
/// settings and reconciles running plugins with the enable flags; a bind failure parks one
/// plugin as failed without touching the app or the other plugin.
pub struct Plugins {
    io: Io,
    settings: Entity<AppSettings>,
    client: Client,
    auth: Auth,
    slots: Vec<Slot>,
}

struct Slot {
    plugin: Arc<dyn Plugin>,
    enabled: fn(&AppSettings) -> bool,
    port: fn(&AppSettings) -> u16,
    auth_required: fn(&AppSettings) -> bool,
    state: SlotState,
}

enum SlotState {
    Stopped,
    Running {
        shutdown: oneshot::Sender<()>,
        task: JoinHandle<()>,
        addr: SocketAddr,
        /// The configured port this listener bound for. Compared against settings, so a
        /// zero asking the OS for a port does not read as drift on every reconcile.
        port: u16,
        /// Whether this serve requires the session token. A settings flip rebinds.
        auth_required: bool,
    },
    Failed {
        reason: String,
    },
}

impl Plugins {
    /// The attached manager entity, for the settings UI. Panics when `attach_plugins` has
    /// not run, which only a caller outside the app startup path can hit.
    pub fn entity(cx: &App) -> Entity<Self> {
        cx.global::<Attached>()._plugins.clone()
    }

    /// Registers `plugins` and reconciles them with settings. Production passes the built-in
    /// transports through `attach_plugins`; tests pass fakes.
    pub fn new(
        io: Io,
        settings: Entity<AppSettings>,
        client: Client,
        plugins: Vec<Registration>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&settings, |this, _, cx| this.reconcile(cx))
            .detach();
        cx.subscribe(&settings, |this, _, _: &Reloaded, cx| this.reconcile(cx))
            .detach();
        let mut this = Self {
            io,
            settings,
            client,
            auth: Auth::new(),
            slots: plugins
                .into_iter()
                .map(|registration| Slot {
                    plugin: registration.plugin,
                    enabled: registration.enabled,
                    port: registration.port,
                    auth_required: registration.auth_required,
                    state: SlotState::Stopped,
                })
                .collect(),
        };
        this.reconcile(cx);
        this
    }

    /// The listener state of one plugin, for settings. `None` names no registered plugin.
    pub fn status(&self, id: &str) -> Option<PluginStatus> {
        self.slots
            .iter()
            .find(|slot| slot.plugin.id() == id)
            .map(|slot| match &slot.state {
                SlotState::Stopped => PluginStatus::Stopped,
                SlotState::Running { addr, .. } => PluginStatus::Running { addr: *addr },
                SlotState::Failed { reason } => PluginStatus::Failed {
                    reason: reason.clone(),
                },
            })
    }

    /// The session token, while any plugin serves.
    pub fn token(&self) -> Option<String> {
        self.auth.token()
    }

    /// Rotates the session token, restarting running plugins so old connections drop. `None`
    /// leaves everything untouched when the OS would not hand out randomness.
    pub fn rotate_token(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let token = self.auth.rotate()?;
        let targets: Vec<(u16, bool)> = {
            let settings = self.settings.read(cx);
            self.slots
                .iter()
                .map(|slot| ((slot.port)(settings), (slot.auth_required)(settings)))
                .collect()
        };
        for (slot, (port, auth_required)) in self.slots.iter_mut().zip(targets) {
            if matches!(slot.state, SlotState::Running { .. }) {
                stop(&self.io, slot);
                start(
                    &self.io,
                    &self.client,
                    &self.auth,
                    slot,
                    port,
                    auth_required,
                );
            }
        }
        cx.notify();
        Some(token)
    }

    /// Starts, stops or retries plugins to match the enable flags. Bind failures park one
    /// plugin as failed; everything else keeps running.
    fn reconcile(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<(bool, u16, bool)> = {
            let settings = self.settings.read(cx);
            self.slots
                .iter()
                .map(|slot| {
                    (
                        (slot.enabled)(settings),
                        (slot.port)(settings),
                        (slot.auth_required)(settings),
                    )
                })
                .collect()
        };
        for (slot, (enabled, port, auth_required)) in self.slots.iter_mut().zip(wanted) {
            let running = match &slot.state {
                SlotState::Running {
                    port,
                    auth_required,
                    ..
                } => Some((*port, *auth_required)),
                SlotState::Stopped | SlotState::Failed { .. } => None,
            };
            match (enabled, running) {
                (true, Some((current_port, current_auth)))
                    if current_port != port || current_auth != auth_required =>
                {
                    stop(&self.io, slot);
                    start(
                        &self.io,
                        &self.client,
                        &self.auth,
                        slot,
                        port,
                        auth_required,
                    );
                }
                (true, None) => start(
                    &self.io,
                    &self.client,
                    &self.auth,
                    slot,
                    port,
                    auth_required,
                ),
                (false, Some(_)) => stop(&self.io, slot),
                _ => {}
            }
        }
        // The token lives only while something serves it: forgetting it on the last stop
        // makes the next enable mint a fresh one.
        let idle = self
            .slots
            .iter()
            .all(|slot| !matches!(slot.state, SlotState::Running { .. }));
        if idle {
            self.auth.clear();
        }
        cx.notify();
    }
}

impl Drop for Plugins {
    /// Aborts running plugin tasks. The io runtime dies with the process; this only hurries it.
    fn drop(&mut self) {
        for slot in &mut self.slots {
            if let SlotState::Running { task, .. } = &mut slot.state {
                task.abort();
            }
        }
    }
}

/// Binds loopback and serves it on the io runtime. A bind failure parks the slot as failed.
fn start(io: &Io, client: &Client, auth: &Auth, slot: &mut Slot, port: u16, auth_required: bool) {
    let socket = SocketAddr::new(LOOPBACK, port);
    let std_listener = match std::net::TcpListener::bind(socket) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!(
                "plugins: cannot listen for {} on {socket}: {error}",
                slot.plugin.id()
            );
            slot.state = SlotState::Failed {
                reason: format!("cannot listen on {socket}: {error}"),
            };
            return;
        }
    };
    if auth.ensure().is_none() {
        log::error!(
            "plugins: no random source for the {} token",
            slot.plugin.id()
        );
        slot.state = SlotState::Failed {
            reason: "cannot mint a session token".to_owned(),
        };
        return;
    }
    if let Err(error) = std_listener.set_nonblocking(true) {
        log::warn!(
            "plugins: cannot serve {} on {socket}: {error}",
            slot.plugin.id()
        );
        slot.state = SlotState::Failed {
            reason: format!("cannot serve on {socket}: {error}"),
        };
        return;
    }
    // from_std registers the socket with a runtime, so the io context is entered for the
    // conversion itself. Nothing here blocks.
    let _entered = io.handle().enter();
    let listener = match tokio::net::TcpListener::from_std(std_listener) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!(
                "plugins: cannot serve {} on {socket}: {error}",
                slot.plugin.id()
            );
            slot.state = SlotState::Failed {
                reason: format!("cannot serve on {socket}: {error}"),
            };
            return;
        }
    };
    let addr = listener.local_addr().unwrap_or(socket);
    let (shutdown, rx) = oneshot::channel();
    let plugin = slot.plugin.clone();
    let ctx = PluginContext {
        client: client.clone(),
        auth: auth.clone(),
        auth_required,
    };
    let task = io.spawn(async move { plugin.serve(ctx, listener, rx).await });
    log::info!("plugins: {} listening on {addr}", slot.plugin.id());
    slot.state = SlotState::Running {
        shutdown,
        task,
        addr,
        port,
        auth_required,
    };
}

/// Signals a running plugin to stop, aborting its task past the drain grace.
fn stop(io: &Io, slot: &mut Slot) {
    let SlotState::Running { shutdown, task, .. } =
        std::mem::replace(&mut slot.state, SlotState::Stopped)
    else {
        return;
    };
    log::info!("plugins: stopping {}", slot.plugin.id());
    let _ = shutdown.send(());
    // The reaper is fire-and-forget by design: it ends the serve task past the grace, then
    // ends itself.
    drop(io.spawn(async move {
        let mut task = task;
        if tokio::time::timeout(STOP_GRACE, &mut task).await.is_err() {
            task.abort();
        }
    }));
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use control::{
        AppSnapshot, PlaybackSnapshot, PlaybackStatus, QueueSnapshot, RepeatMode, SessionSnapshot,
        SessionStatus,
    };
    use storage::Database;

    use super::*;
    use crate::AppSettings;

    struct Fake {
        id: &'static str,
        served: AtomicUsize,
        stopped: AtomicUsize,
        open: AtomicUsize,
    }

    impl Fake {
        fn new(id: &'static str) -> Arc<Self> {
            Arc::new(Self {
                id,
                served: AtomicUsize::new(0),
                stopped: AtomicUsize::new(0),
                open: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait::async_trait]
    impl Plugin for Fake {
        fn id(&self) -> &'static str {
            self.id
        }

        async fn serve(
            &self,
            ctx: PluginContext,
            listener: tokio::net::TcpListener,
            mut shutdown: oneshot::Receiver<()>,
        ) {
            self.served.fetch_add(1, Ordering::Relaxed);
            if !ctx.auth_required {
                self.open.fetch_add(1, Ordering::Relaxed);
            }
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    accepted = listener.accept() => {
                        if accepted.is_err() {
                            break;
                        }
                    }
                }
            }
            self.stopped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn pictured() -> AppSnapshot {
        AppSnapshot {
            snapshot_revision: 0,
            playback: PlaybackSnapshot {
                status: PlaybackStatus::Idle,
                track: None,
                position_ms: 0,
                duration_ms: None,
                volume: 0.7,
                repeat: RepeatMode::Off,
                shuffle: false,
            },
            queue: QueueSnapshot {
                revision: 0,
                past: Vec::new(),
                current: None,
                upcoming: Vec::new(),
                suggested: Vec::new(),
                manual_count: 0,
                shuffle: false,
            },
            session: SessionSnapshot {
                status: SessionStatus::SignedOut,
                provider: None,
            },
        }
    }

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "sonora-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    struct Rig {
        settings: Entity<AppSettings>,
        plugins: Entity<Plugins>,
        remote: Arc<Fake>,
        root: PathBuf,
    }

    impl Rig {
        fn new(cx: &mut gpui::TestAppContext) -> Self {
            Self::with_settings(cx, None)
        }

        /// Builds the rig, optionally from a `settings.json` written first.
        fn with_settings(cx: &mut gpui::TestAppContext, settings_json: Option<&str>) -> Self {
            let root = scratch("plugins");
            if let Some(json) = settings_json {
                std::fs::create_dir_all(&root).expect("scratch dir exists");
                std::fs::write(root.join("settings.json"), json).expect("settings are written");
            }
            let database = Database::at(root.join("state.sqlite"));
            let io = Io::new().expect("io runtime starts");
            let settings = cx.new(|_| AppSettings::load_at(root.join("settings.json"), database));
            let (_, client) = control::channel::pair(pictured());
            let remote = Fake::new("remote");
            let plugins = cx.new(|cx| {
                Plugins::new(
                    io,
                    settings.clone(),
                    client,
                    vec![Registration {
                        plugin: remote.clone(),
                        enabled: AppSettings::remote_api,
                        port: AppSettings::remote_port,
                        auth_required: AppSettings::remote_auth,
                    }],
                    cx,
                )
            });
            Self {
                settings,
                plugins,
                remote,
                root,
            }
        }

        fn enable_remote(&self, cx: &mut gpui::TestAppContext) {
            cx.update(|cx| {
                self.settings.update(cx, |settings, cx| {
                    settings.set_remote_port(0, cx);
                    settings.set_remote_api(true, cx);
                });
            });
            cx.run_until_parked();
        }

        fn status(&self, id: &str, cx: &mut gpui::TestAppContext) -> Option<PluginStatus> {
            cx.update(|cx| self.plugins.read(cx).status(id))
        }

        fn addr(&self, id: &str, cx: &mut gpui::TestAppContext) -> SocketAddr {
            match self.status(id, cx) {
                Some(PluginStatus::Running { addr }) => addr,
                status => panic!("expected {id} running, saw {status:?}"),
            }
        }

        /// Waits for a fake counter on the io threads, which the gpui dispatcher does
        /// not drive.
        fn await_count(label: &str, counter: &AtomicUsize, count: usize) {
            let landed = Instant::now();
            while counter.load(Ordering::Relaxed) != count {
                assert!(
                    landed.elapsed() < Duration::from_secs(5),
                    "the fake {label}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn await_served(fake: &Fake, count: usize) {
            Self::await_count("serves", &fake.served, count);
        }

        fn await_stopped(fake: &Fake, count: usize) {
            Self::await_count("stops", &fake.stopped, count);
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[gpui::test]
    async fn stays_quiet_until_opt_in(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);

        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Stopped)
        ));
        assert!(rig.status("other", cx).is_none());
        assert_eq!(rig.remote.served.load(Ordering::Relaxed), 0);
        let token = cx.update(|cx| rig.plugins.read(cx).token());
        assert_eq!(token, None);
    }

    #[gpui::test]
    async fn enable_starts_listening_on_loopback(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);

        let addr = rig.addr("remote", cx);
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0);
        Rig::await_served(&rig.remote, 1);
        let token = cx.update(|cx| rig.plugins.read(cx).token());
        assert_eq!(token.as_deref().map(str::len), Some(43));

        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("the port is open");
    }

    #[gpui::test]
    async fn disable_stops_and_frees_the_port(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);
        let addr = rig.addr("remote", cx);

        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_api(false, cx);
            });
        });
        cx.run_until_parked();

        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Stopped)
        ));
        Rig::await_stopped(&rig.remote, 1);
        assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_err());
    }

    #[gpui::test]
    async fn port_change_rebinds(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);
        Rig::await_served(&rig.remote, 1);
        let token = cx.update(|cx| rig.plugins.read(cx).token());

        // A reconcile with the same port leaves the listener alone.
        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_port(0, cx);
            });
        });
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(rig.remote.served.load(Ordering::Relaxed), 1);

        let held = std::net::TcpListener::bind(SocketAddr::new(LOOPBACK, 0)).expect("a free port");
        let port = held.local_addr().expect("a bound port").port();
        drop(held);
        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_port(port, cx);
            });
        });
        cx.run_until_parked();

        Rig::await_served(&rig.remote, 2);
        Rig::await_stopped(&rig.remote, 1);
        assert_eq!(rig.remote.served.load(Ordering::Relaxed), 2);
        assert_eq!(rig.addr("remote", cx).port(), port);
        assert_eq!(cx.update(|cx| rig.plugins.read(cx).token()), token);
    }

    #[gpui::test]
    async fn auth_change_restarts_open(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);
        Rig::await_served(&rig.remote, 1);
        assert_eq!(rig.remote.open.load(Ordering::Relaxed), 0);

        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_auth(false, cx);
            });
        });
        cx.run_until_parked();

        Rig::await_served(&rig.remote, 2);
        Rig::await_stopped(&rig.remote, 1);
        assert_eq!(rig.remote.open.load(Ordering::Relaxed), 1);
        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Running { .. })
        ));
    }

    #[gpui::test]
    async fn disable_forgets_the_token(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);
        let before = cx.update(|cx| rig.plugins.read(cx).token());
        assert!(before.is_some());

        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_api(false, cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| rig.plugins.read(cx).token()), None);

        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_api(true, cx);
            });
        });
        cx.run_until_parked();
        let after = cx.update(|cx| rig.plugins.read(cx).token());
        assert!(after.is_some());
        assert_ne!(before, after);
    }

    #[gpui::test]
    async fn collision_is_recorded_not_fatal(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        let held =
            std::net::TcpListener::bind(SocketAddr::new(LOOPBACK, 0)).expect("a port to hold");
        let port = held.local_addr().expect("a held port").port();

        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_port(port, cx);
                settings.set_remote_api(true, cx);
            });
        });
        cx.run_until_parked();

        match rig.status("remote", cx) {
            Some(PluginStatus::Failed { reason }) => assert!(!reason.is_empty()),
            status => panic!("expected remote failed, saw {status:?}"),
        }
        assert_eq!(rig.remote.served.load(Ordering::Relaxed), 0);

        // Asking the OS for a port retries the bind.
        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_port(0, cx);
            });
        });
        cx.run_until_parked();

        Rig::await_served(&rig.remote, 1);
        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Running { .. })
        ));
    }

    #[gpui::test]
    async fn double_enable_starts_once(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);
        cx.update(|cx| {
            rig.settings.update(cx, |settings, cx| {
                settings.set_remote_api(true, cx);
            });
        });
        cx.run_until_parked();

        Rig::await_served(&rig.remote, 1);
        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Running { .. })
        ));
    }

    #[gpui::test]
    async fn rotate_restarts_and_changes_the_token(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        rig.enable_remote(cx);
        let before = cx.update(|cx| rig.plugins.read(cx).token());

        let after = cx.update(|cx| {
            rig.plugins
                .update(cx, |plugins, cx| plugins.rotate_token(cx))
        });

        assert!(before.is_some());
        assert!(after.is_some());
        assert_ne!(before, after);
        Rig::await_served(&rig.remote, 2);
        Rig::await_stopped(&rig.remote, 1);
        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Running { .. })
        ));
    }

    #[gpui::test]
    async fn saved_enablement_listens_on_startup(cx: &mut gpui::TestAppContext) {
        let rig = Rig::with_settings(cx, Some(r#"{"remote_api":true,"remote_port":0}"#));

        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Running { .. })
        ));
        cx.update(|cx| {
            rig.settings.update(cx, |_, cx| cx.emit(Reloaded));
        });
        cx.run_until_parked();
        Rig::await_served(&rig.remote, 1);
    }

    #[gpui::test]
    async fn retired_flags_listen_on_startup(cx: &mut gpui::TestAppContext) {
        let rig = Rig::with_settings(cx, Some(r#"{"ws_api":true,"ws_port":0}"#));

        Rig::await_served(&rig.remote, 1);
        assert!(matches!(
            rig.status("remote", cx),
            Some(PluginStatus::Running { .. })
        ));
    }
}
