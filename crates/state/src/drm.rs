//! Whether this machine can decrypt protected tracks.
//!
//! Sonora ships no Widevine module: Google publishes nothing anyone may redistribute. It does
//! what Kodi does instead. A copy a browser on the machine already has is used as it lies.
//! Nothing is looked for until the current provider is one whose tracks need the module, so a
//! YouTube or Spotify OAuth run never touches a browser folder. With none found, the user is asked
//! whether to download it from Google, shown Google's terms out of the downloaded archive, and
//! asked again before it is installed into Sonora's own store. Settings offers the same
//! download beside a browser's copy, since that copy can be one the host cannot open. A
//! module accepted that way is preferred over any browser's and kept current without asking
//! again. With nothing accepted, protected providers keep their metadata and refuse to play.

use gpui::{App, Context, Entity, Task};
use music::drm::{self, Offer, Origin};

use crate::{Io, Outcome, Session, SessionEvent, Toasts};

/// Google's terms for a downloaded module, as the prompt shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terms {
    pub version: String,
    pub license: String,
}

/// Where the module stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CdmState {
    /// Reading the environment, the browsers and the store, which is only the first moment of
    /// a run.
    Looking,
    /// One is here, and where it came from.
    Ready(Origin),
    /// Nothing is here and a protected provider has an account: the user is being asked
    /// whether to download.
    Wanted,
    /// Downloading from Google, to show the terms.
    Offering,
    /// Downloaded; the user is being asked to accept the terms.
    Offered(Terms),
    /// Accepted and going into the store.
    Installing,
    /// The user said no this run. Settings still offers the download.
    Declined,
    /// Nothing is here and nothing is being asked for.
    Missing,
}

/// The Widevine module as app state.
pub struct Drm {
    state: CdmState,
    offer: Option<Offer>,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
    /// Whether the machine has been searched this run. The search happens once, the first
    /// time a protected provider is the current one, and later switches only reuse its answer.
    looked: bool,
    /// Where the module in use came from when the user asked for Google's copy beside it. A
    /// download that fails or is declined goes back to that one.
    standing: Option<Origin>,
}

impl Drm {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, session, event, cx| {
            if matches!(event, SessionEvent::SignedIn)
                && drm::supported()
                && session.read(cx).wants_drm()
            {
                this.wanted(cx);
            }
        })
        .detach();

        Self {
            state: CdmState::Missing,
            offer: None,
            session,
            io,
            task: None,
            looked: false,
            standing: None,
        }
    }

    pub fn state(&self) -> &CdmState {
        &self.state
    }

    /// Whether the module in use is a browser's copy Sonora found on its own. That copy can be
    /// too old for the license server, which is worth telling the user when tracks fail.
    pub fn borrowed(&self) -> bool {
        self.state == CdmState::Ready(Origin::Installed)
    }

    /// Whether the download under way would replace a module already here, a browser's copy the
    /// user asked Google's to stand in for.
    pub fn replacing(&self) -> bool {
        self.standing.is_some()
    }

    /// Whether this build has a host for a module at all. False means the platform has no
    /// host yet, so nothing about a module is worth showing the user.
    pub fn supported(&self) -> bool {
        drm::supported()
    }

    /// Whether the module is worth showing the user right now: this build has a host for one
    /// and the current provider's tracks need it. Settings draws its row on this alone.
    pub fn shown(&self, cx: &App) -> bool {
        drm::supported() && self.session.read(cx).wants_drm()
    }

    /// A protected provider just became the current one. The first time, the machine is
    /// searched; afterwards the user is asked again only if nothing was found or decided.
    fn wanted(&mut self, cx: &mut Context<Self>) {
        match self.looked {
            false => self.look(cx),
            true => self.ask(cx),
        }
    }

    /// Answers from what is already here, off the network. With nothing here and a protected
    /// account present, the user is asked; with a module Sonora fetched before, a newer one is
    /// fetched quietly.
    pub fn look(&mut self, cx: &mut Context<Self>) {
        self.looked = true;
        self.state = CdmState::Looking;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let found = io.spawn_blocking(drm::find).await.ok().flatten();
            this.update(cx, |this, cx| {
                this.task = None;
                this.state = match &found {
                    Some(found) => CdmState::Ready(found.origin),
                    None => CdmState::Missing,
                };
                cx.notify();
                if !this.session.read(cx).wants_drm() {
                    return;
                }
                match &found {
                    None => this.ask(cx),
                    Some(found) if found.origin == Origin::Fetched => this.refresh(cx),
                    Some(_) => {}
                }
            })
            .ok();
        }));
    }

    /// Puts the question up, unless one is already up, the user answered it this run, or a
    /// module is here.
    fn ask(&mut self, cx: &mut Context<Self>) {
        if self.state != CdmState::Missing {
            return;
        }
        self.state = CdmState::Wanted;
        cx.notify();
    }

    /// The user's no, to either question. A download under way is dropped, and a module that
    /// was already here stays in use.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.offer = None;
        self.state = self.fall_back(CdmState::Declined);
        cx.notify();
    }

    /// The user's yes to downloading: fetches the archive and puts the terms up. Asked while
    /// a browser's copy is in use, that copy stays in use unless Google's is installed.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        self.standing = match self.state {
            CdmState::Ready(origin) => Some(origin),
            _ => None,
        };
        self.state = CdmState::Offering;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let offered = io.spawn(drm::offer()).await;
            this.update(cx, |this, cx| {
                this.task = None;
                match offered {
                    Ok(Ok(offer)) => {
                        this.state = CdmState::Offered(Terms {
                            version: offer.version().to_string(),
                            license: offer.license().to_string(),
                        });
                        this.offer = Some(offer);
                    }
                    Ok(Err(error)) => {
                        log::warn!("drm: cannot download the widevine module: {error:#}");
                        this.state = this.fall_back(CdmState::Missing);
                        failed("toast-widevine-download-failed", format!("{error:#}"), cx);
                    }
                    Err(error) => {
                        this.state = this.fall_back(CdmState::Missing);
                        failed("toast-widevine-download-failed", error.to_string(), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The user's yes to the terms: installs what was downloaded.
    pub fn accept(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self.offer.take() else {
            return;
        };
        self.state = CdmState::Installing;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let installed = io.spawn_blocking(move || offer.install()).await;
            this.update(cx, |this, cx| {
                this.task = None;
                this.state = match installed {
                    Ok(Ok(found)) => {
                        this.standing = None;
                        CdmState::Ready(found.origin)
                    }
                    Ok(Err(error)) => {
                        log::warn!("drm: cannot install the widevine module: {error:#}");
                        failed("toast-widevine-install-failed", format!("{error:#}"), cx);
                        this.fall_back(CdmState::Missing)
                    }
                    Err(error) => {
                        failed("toast-widevine-install-failed", error.to_string(), cx);
                        this.fall_back(CdmState::Missing)
                    }
                };
                cx.notify();
            })
            .ok();
        }));
    }

    /// Removes the module Sonora fetched from Google and settles on whatever else is here, a
    /// browser's copy or nothing. Nothing is asked again this run.
    pub fn uninstall(&mut self, cx: &mut Context<Self>) {
        self.offer = None;
        self.state = CdmState::Looking;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let found = io
                .spawn_blocking(|| {
                    drm::uninstall()?;
                    anyhow::Ok(drm::find())
                })
                .await;
            this.update(cx, |this, cx| {
                this.task = None;
                this.state = match found {
                    Ok(Ok(Some(found))) => CdmState::Ready(found.origin),
                    Ok(Ok(None)) => CdmState::Declined,
                    Ok(Err(error)) => {
                        log::warn!("drm: cannot remove the widevine module: {error:#}");
                        failed("toast-widevine-remove-failed", format!("{error:#}"), cx);
                        match drm::find() {
                            Some(found) => CdmState::Ready(found.origin),
                            None => CdmState::Declined,
                        }
                    }
                    Err(error) => {
                        failed("toast-widevine-remove-failed", error.to_string(), cx);
                        CdmState::Declined
                    }
                };
                cx.notify();
            })
            .ok();
        }));
    }

    /// The module that was in use before a download the user asked for, or `otherwise` when
    /// there was none. Either way the download no longer stands in front of it.
    fn fall_back(&mut self, otherwise: CdmState) -> CdmState {
        self.standing.take().map_or(otherwise, CdmState::Ready)
    }

    /// Fetches a newer version of a module the user accepted before, without asking. The one
    /// already open stays in use until the next run.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let fetched = io.spawn(drm::fetch()).await;
            this.update(cx, |this, _| {
                this.task = None;
                if let Ok(Err(error)) = fetched {
                    log::warn!("drm: cannot refresh the widevine module: {error:#}");
                }
            })
            .ok();
        }));
    }
}

/// Shows a failed step the user started, with the error below it.
fn failed(key: &'static str, error: String, cx: &mut App) {
    Toasts::with_detail(Outcome::Failed, key, None, error, cx);
}
