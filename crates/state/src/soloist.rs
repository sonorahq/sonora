//! Soloist, Spotify's own headless player, as app state: whether it plays, and the setup that
//! turns it on.
//!
//! Spotify plays through librespot unless the listener sets Soloist up, which nothing does for
//! them. The setup is a dialog of three steps that this entity walks through in order: consent
//! to downloading the program from Spotify, the listener's API key, and pairing the program with
//! the account from one of Spotify's apps. Once all three are done the playback engine is rebuilt
//! on Soloist, and removing the setup rebuilds it on librespot. `music::soloist` does the work,
//! and this only keeps track of where it stands.

use std::time::Duration;

use gpui::{App, Context, Entity, EventEmitter, Task};
use music::soloist::{self, Build, Trouble};
use tokio::task::AbortHandle;

use crate::{Io, Outcome, Session, SessionEvent, SessionState, Toasts};

/// How long pairing waits before it says a firewall may be in the way. Picking the device in a
/// Spotify app takes less when the app can see it.
const BLOCKED_AFTER: Duration = Duration::from_secs(20);

/// A step of the setup dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoloistStep {
    /// Asking whether to download the program from Spotify.
    Consent,
    /// Downloading it.
    Fetching,
    /// Taking the API key.
    Key,
    /// The program is on the account's device list, waiting to be picked in a Spotify app.
    Pairing,
}

/// Where Soloist stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoloistState {
    /// Reading the disk, which is only a moment after sign-in.
    Looking,
    /// Not set up. Spotify plays through librespot.
    Off,
    /// The setup dialog is open on this step.
    Setup(SoloistStep),
    /// Set up, with the build of the program installed. It plays unless the listener switched
    /// it off, which [`Soloist::in_use`] tells.
    Ready(Build),
}

/// Why the last step of the setup did not go through, for the dialog to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoloistProblem {
    /// The program could not be downloaded or does not run here, with the error.
    Download(String),
    /// Spotify turned the API key down.
    Key,
    /// The device was picked from an app signed into another account, the one named here.
    Account(String),
    /// Nobody picked the device in time.
    Waited,
    /// Pairing failed some other way, with the error.
    Pairing(String),
}

/// Sent when Soloist starts or stops being what plays, so the engine is rebuilt.
pub struct SoloistSwitched;

/// Soloist as app state.
pub struct Soloist {
    state: SoloistState,
    problem: Option<SoloistProblem>,
    /// The build of the installed program, when one is installed, set up or not.
    build: Option<Build>,
    /// Whether the engine was last told to play through Soloist.
    in_use: bool,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
    /// The pairing under way. Dropping its task would leave the program listed on the account
    /// and holding its data folder, so a cancelled pairing is aborted through this.
    pairing: Option<AbortHandle>,
    /// The port the pairing program listens on, once pairing has waited `BLOCKED_AFTER`. A
    /// firewall that keeps the Spotify app off it is the usual reason nobody picks the device.
    blocked: Option<u16>,
    /// Waits for that port and that long.
    hint: Option<Task<()>>,
    /// The download the listener asked for, kept apart from `task` so closing the dialog or
    /// looking at the disk again does not lose what it brings.
    download: Option<Task<()>>,
    /// Whether Spotify denied this account an audio key during this run.
    denied: bool,
    /// Whether the setup was offered unasked this run, which happens once.
    asked: bool,
    /// Whether the dialog on show is that offer, so it says why it came up.
    offered: bool,
}

impl EventEmitter<SoloistSwitched> for Soloist {}

impl Soloist {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, session, event, cx| match event {
            SessionEvent::SignedIn if session.read(cx).offers_soloist() => this.look(cx),
            SessionEvent::SignedOut => this.close(cx),
            _ => {}
        })
        .detach();

        Self {
            state: SoloistState::Off,
            problem: None,
            build: None,
            in_use: soloist::ready(),
            session,
            io,
            task: None,
            pairing: None,
            blocked: None,
            hint: None,
            download: None,
            denied: false,
            asked: false,
            offered: false,
        }
    }

    pub fn state(&self) -> &SoloistState {
        &self.state
    }

    /// What went wrong in the step before the one on show.
    pub fn problem(&self) -> Option<&SoloistProblem> {
        self.problem.as_ref()
    }

    /// Whether Soloist is worth showing: the current provider can play through it on this
    /// computer and has an account. Settings draws its row on this alone.
    pub fn shown(&self, cx: &App) -> bool {
        self.session.read(cx).offers_soloist()
    }

    /// Whether the program is on disk, set up or not, so there is something to remove.
    pub fn installed(&self) -> bool {
        self.build.is_some()
    }

    /// The TCP port a firewall has to let in for pairing, along with mDNS, once pairing has
    /// waited long enough for that to be worth saying.
    pub fn blocked_port(&self) -> Option<u16> {
        self.blocked
    }

    /// Whether the setup dialog came up by itself, for an account Spotify denies audio keys.
    pub fn offered(&self) -> bool {
        self.offered
    }

    /// Whether Soloist is set up and switched on.
    pub fn in_use(&self) -> bool {
        self.in_use
    }

    /// Whether Soloist is what plays the current provider right now: it is set up, switched on
    /// and the provider on show is the one it plays.
    pub fn plays(&self, cx: &App) -> bool {
        self.in_use && self.shown(cx)
    }

    /// Reads what is on disk: whether the setup is complete and which build is installed. A
    /// build close to its end is replaced quietly, since the listener agreed to the download
    /// when they set it up.
    pub fn look(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, SoloistState::Setup(_)) {
            return;
        }
        self.state = SoloistState::Looking;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let looked = io
                .spawn_blocking(|| (soloist::installed(), soloist::wanted()))
                .await;
            let (build, wanted) = looked.unwrap_or((None, false));
            this.update(cx, |this, cx| {
                this.task = None;
                let stale = build.as_ref().is_some_and(Build::stale);
                this.build = build;
                this.settle(cx);
                if stale && this.in_use {
                    this.refresh(cx);
                }
                if wanted || this.denied {
                    this.offer(cx);
                }
            })
            .ok();
        }));
    }

    /// Spotify denied the account an audio key, so nothing plays through librespot. The setup
    /// is offered for it, once a run.
    pub(crate) fn refused(&mut self, cx: &mut Context<Self>) {
        self.denied = true;
        self.offer(cx);
    }

    /// Opens the setup unasked, the way the Widevine module is offered: once a run, for an
    /// account Spotify denies keys, while Soloist is not set up. Closing it leaves the setup to
    /// the settings until the next run.
    fn offer(&mut self, cx: &mut Context<Self>) {
        if self.asked || self.state != SoloistState::Off || !self.shown(cx) {
            return;
        }
        self.asked = true;
        self.begin(cx);
        self.offered = true;
    }

    /// Switches Soloist on or off while keeping its setup. The engine is rebuilt on whichever
    /// player that leaves.
    pub fn set_used(&mut self, on: bool, cx: &mut Context<Self>) {
        if !matches!(self.state, SoloistState::Ready(_)) || on == self.in_use {
            return;
        }
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let switched = io.spawn_blocking(move || soloist::enable(on)).await;
            this.update(cx, |this, cx| {
                this.task = None;
                match switched {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => log::warn!("soloist: cannot switch: {error:#}"),
                    Err(error) => log::warn!("soloist: cannot switch: {error}"),
                }
                this.settle(cx);
            })
            .ok();
        }));
    }

    /// Opens the setup dialog: on the download when the program is not here yet, on its progress
    /// when it is on its way, and on the key when it is here. A setup that is only switched off
    /// is switched back on instead.
    pub fn begin(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, SoloistState::Setup(_)) {
            return;
        }
        if matches!(self.state, SoloistState::Ready(_)) && !self.in_use {
            return self.set_used(true, cx);
        }
        self.task = None;
        self.problem = None;
        self.offered = false;
        // a download the listener started before closing the dialog is still on its way
        self.state = SoloistState::Setup(match (&self.download, &self.build) {
            (Some(_), _) => SoloistStep::Fetching,
            (None, Some(_)) => SoloistStep::Key,
            (None, None) => SoloistStep::Consent,
        });
        cx.notify();
    }

    /// The listener's yes to the download: fetches the program from Spotify and moves on to
    /// the key.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        self.problem = None;
        self.state = SoloistState::Setup(SoloistStep::Fetching);
        cx.notify();

        let io = self.io.clone();
        self.download = Some(cx.spawn(async move |this, cx| {
            let fetched = io.spawn(soloist::fetch()).await;
            this.update(cx, |this, cx| {
                this.download = None;
                let fetched = match fetched {
                    Ok(fetched) => fetched.map_err(|error| format!("{error:#}")),
                    Err(error) => Err(error.to_string()),
                };
                // a dialog closed meanwhile stays closed, and the program is kept all the same
                let waiting = this.state == SoloistState::Setup(SoloistStep::Fetching);
                match fetched {
                    Ok(build) => {
                        this.build = Some(build);
                        if waiting {
                            this.state = SoloistState::Setup(SoloistStep::Key);
                        }
                    }
                    Err(error) => {
                        log::warn!("soloist: cannot download the program: {error}");
                        if waiting {
                            this.problem = Some(SoloistProblem::Download(error));
                            this.state = SoloistState::Setup(SoloistStep::Consent);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Takes the API key and starts pairing. The key is only stored once Spotify has taken it,
    /// which it does when the listener picks the device.
    pub fn submit(&mut self, key: String, cx: &mut Context<Self>) {
        if self.state != SoloistState::Setup(SoloistStep::Key) || key.trim().is_empty() {
            return;
        }
        self.problem = None;
        self.state = SoloistState::Setup(SoloistStep::Pairing);
        cx.notify();

        let account = match self.session.read(cx).state() {
            SessionState::SignedIn(profile) => Some(profile.id.clone()),
            _ => None,
        };
        let (listening, port) = tokio::sync::oneshot::channel();
        let pairing = self.io.spawn(soloist::pair(key, account, listening));
        self.pairing = Some(pairing.abort_handle());
        self.blocked = None;
        self.hint = Some(cx.spawn(async move |this, cx| {
            let Ok(port) = port.await else {
                return;
            };
            cx.background_executor().timer(BLOCKED_AFTER).await;
            this.update(cx, |this, cx| {
                if this.state == SoloistState::Setup(SoloistStep::Pairing) {
                    this.blocked = Some(port);
                    cx.notify();
                }
            })
            .ok();
        }));
        self.task = Some(cx.spawn(async move |this, cx| {
            let paired = pairing.await;
            this.update(cx, |this, cx| {
                this.task = None;
                this.pairing = None;
                this.hint = None;
                this.blocked = None;
                match paired {
                    Ok(Ok(())) => {
                        // A pairing that replaces one already in use changes nothing `settle`
                        // can see, and the engine still has to start over on the new sign-in.
                        if !this.settle(cx) {
                            cx.emit(SoloistSwitched);
                        }
                        Toasts::show(Outcome::Done, "toast-soloist-ready", cx);
                    }
                    Ok(Err(trouble)) => {
                        log::warn!("soloist: pairing failed: {trouble:?}");
                        this.problem = Some(match trouble {
                            Trouble::Key => SoloistProblem::Key,
                            Trouble::Account(account) => SoloistProblem::Account(account),
                            Trouble::Waited => SoloistProblem::Waited,
                            Trouble::Failed(error) => SoloistProblem::Pairing(error),
                        });
                        this.state = SoloistState::Setup(SoloistStep::Key);
                        cx.notify();
                    }
                    // aborted by `close`, which has settled the state already
                    Err(_) => {}
                }
            })
            .ok();
        }));
    }

    /// Goes back from pairing to the key, as when the listener wants to paste another one.
    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.state != SoloistState::Setup(SoloistStep::Pairing) {
            return;
        }
        self.stop_pairing();
        self.state = SoloistState::Setup(SoloistStep::Key);
        cx.notify();
    }

    /// Closes the setup dialog wherever it stands. A pairing under way is stopped. A download
    /// under way carries on, so the program is there the next time the setup opens.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.stop_pairing();
        self.problem = None;
        self.offered = false;
        self.settle(cx);
    }

    /// Removes the setup and the program, which puts Spotify back on librespot. Soloist is
    /// switched off and its program stopped first, since a running program holds its own files.
    /// A removal that fails leaves whatever is still on disk, with Remove still on offer.
    pub fn remove(&mut self, cx: &mut Context<Self>) {
        self.stop_pairing();
        self.download = None;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            if let Ok(Err(error)) = io.spawn_blocking(|| soloist::enable(false)).await {
                log::warn!("soloist: cannot switch off before removing: {error:#}");
            }
            // the engine lets go of Soloist here, and the program stops with it
            this.update(cx, |this, cx| {
                this.settle(cx);
                this.state = SoloistState::Looking;
            })
            .ok();
            io.spawn(soloist::stopped()).await.ok();
            let removed = io
                .spawn_blocking(|| {
                    let removed = soloist::remove();
                    if removed.is_err() {
                        soloist::reload();
                    }
                    (removed, soloist::installed())
                })
                .await;
            this.update(cx, |this, cx| {
                this.task = None;
                let (removed, build) = match removed {
                    Ok((removed, build)) => (removed.map_err(|error| format!("{error:#}")), build),
                    Err(error) => (Err(error.to_string()), None),
                };
                let error = removed.err();
                if let Some(error) = error {
                    log::warn!("soloist: cannot remove the setup: {error}");
                    Toasts::with_detail(
                        Outcome::Failed,
                        "toast-soloist-remove-failed",
                        None,
                        error,
                        cx,
                    );
                }
                this.build = build;
                this.settle(cx);
            })
            .ok();
        }));
    }

    /// Fetches the current build over one close to its end, without asking. The engine in use
    /// keeps the build it started and the next one starts the new.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let fetched = io.spawn(soloist::fetch()).await;
            this.update(cx, |this, cx| {
                this.task = None;
                match fetched {
                    Ok(Ok(build)) => {
                        this.build = Some(build);
                        this.settle(cx);
                    }
                    Ok(Err(error)) => log::warn!("soloist: cannot refresh the program: {error:#}"),
                    Err(error) => log::warn!("soloist: cannot refresh the program: {error}"),
                }
            })
            .ok();
        }));
    }

    fn stop_pairing(&mut self) {
        self.hint = None;
        self.blocked = None;
        if let Some(pairing) = self.pairing.take() {
            pairing.abort();
            self.task = None;
        }
    }

    /// Takes the state from what is on disk and tells the engine when Soloist started or
    /// stopped being what plays. Answers whether it told the engine.
    fn settle(&mut self, cx: &mut Context<Self>) -> bool {
        let ready = soloist::ready();
        self.state = match (&self.build, soloist::set_up()) {
            (Some(build), true) => SoloistState::Ready(build.clone()),
            _ => SoloistState::Off,
        };
        let switched = std::mem::replace(&mut self.in_use, ready) != ready;
        if switched {
            cx.emit(SoloistSwitched);
        }
        cx.notify();
        switched
    }
}
