//! Whether this machine can decrypt protected tracks.
//!
//! Sonora ships no Widevine module: Google publishes nothing anyone may redistribute. It does
//! what Kodi does instead. A copy a browser on the machine already has is used as it lies.
//! Nothing is looked for until the current provider is one whose tracks need the module, so a
//! Spotify or YouTube run never touches a browser folder. With none found, the user is asked
//! whether to download it from Google, shown Google's terms out of the downloaded archive, and
//! asked again before it is installed into Sonora's own store. Settings offers the same
//! download beside a browser's copy, since that copy can be one the host cannot open. A
//! module accepted that way is preferred over any browser's and kept current without asking
//! again. With nothing accepted, protected providers keep their metadata and refuse to play.

use music::drm::{self, Offer, Origin};

use crate::{Change, Cx, Event, Job, Outcome, Session, SessionEvent, Store, Toasts};

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
    task: Option<Job>,
    /// Whether the machine has been searched this run. The search happens once, the first
    /// time a protected provider is the current one, and later switches only reuse its answer.
    looked: bool,
    /// Where the module in use came from when the user asked for Google's copy beside it. A
    /// download that fails or is declined goes back to that one.
    standing: Option<Origin>,
}

impl Default for Drm {
    fn default() -> Self {
        Self::new()
    }
}

impl Drm {
    pub fn new() -> Self {
        Self {
            state: CdmState::Missing,
            offer: None,
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
    pub fn shown(&self, session: &Session) -> bool {
        drm::supported() && session.wants_drm()
    }

    /// A protected provider just became the current one. The first time, the machine is
    /// searched; afterwards the user is asked again only if nothing was found or decided.
    fn wanted(&mut self, cx: &mut Cx) {
        match self.looked {
            false => self.look(cx),
            true => self.ask(),
        }
    }

    /// Answers from what is already here, off the network. With nothing here and a protected
    /// account present, the user is asked; with a module Sonora fetched before, a newer one is
    /// fetched quietly.
    pub fn look(&mut self, cx: &mut Cx) {
        self.looked = true;
        self.state = CdmState::Looking;

        self.task = Some(cx.spawn(|link| async move {
            let found = tokio::task::spawn_blocking(drm::find).await.ok().flatten();
            link.post(move |store| {
                let Store {
                    drm, session, cx, ..
                } = store;
                drm.task = None;
                drm.state = match &found {
                    Some(found) => CdmState::Ready(found.origin),
                    None => CdmState::Missing,
                };
                if !session.wants_drm() {
                    return;
                }
                match &found {
                    None => drm.ask(),
                    Some(found) if found.origin == Origin::Fetched => drm.refresh(cx),
                    Some(_) => {}
                }
            });
        }));
    }

    /// Puts the question up, unless one is already up, the user answered it this run, or a
    /// module is here.
    fn ask(&mut self) {
        if self.state != CdmState::Missing {
            return;
        }
        self.state = CdmState::Wanted;
    }

    /// The user's no, to either question. A download under way is dropped, and a module that
    /// was already here stays in use.
    pub fn dismiss(&mut self) {
        self.task = None;
        self.offer = None;
        self.state = self.fall_back(CdmState::Declined);
    }

    /// The user's yes to downloading: fetches the archive and puts the terms up. Asked while
    /// a browser's copy is in use, that copy stays in use unless Google's is installed.
    pub fn download(&mut self, cx: &mut Cx) {
        self.standing = match self.state {
            CdmState::Ready(origin) => Some(origin),
            _ => None,
        };
        self.state = CdmState::Offering;

        let io = cx.io.clone();
        self.task = Some(cx.spawn(|link| async move {
            let offered = io.spawn(drm::offer()).await;
            link.post(move |store| {
                let Store { drm, cx, .. } = store;
                drm.task = None;
                match offered {
                    Ok(Ok(offer)) => {
                        drm.state = CdmState::Offered(Terms {
                            version: offer.version().to_string(),
                            license: offer.license().to_string(),
                        });
                        drm.offer = Some(offer);
                    }
                    Ok(Err(error)) => {
                        log::warn!("drm: cannot download the widevine module: {error:#}");
                        drm.state = drm.fall_back(CdmState::Missing);
                        failed("toast-widevine-download-failed", format!("{error:#}"), cx);
                    }
                    Err(error) => {
                        drm.state = drm.fall_back(CdmState::Missing);
                        failed("toast-widevine-download-failed", error.to_string(), cx);
                    }
                }
            });
        }));
    }

    /// The user's yes to the terms: installs what was downloaded.
    pub fn accept(&mut self, cx: &mut Cx) {
        let Some(offer) = self.offer.take() else {
            return;
        };
        self.state = CdmState::Installing;

        self.task = Some(cx.spawn(|link| async move {
            let installed = tokio::task::spawn_blocking(move || offer.install()).await;
            link.post(move |store| {
                let Store { drm, cx, .. } = store;
                drm.task = None;
                drm.state = match installed {
                    Ok(Ok(found)) => {
                        drm.standing = None;
                        CdmState::Ready(found.origin)
                    }
                    Ok(Err(error)) => {
                        log::warn!("drm: cannot install the widevine module: {error:#}");
                        failed("toast-widevine-install-failed", format!("{error:#}"), cx);
                        drm.fall_back(CdmState::Missing)
                    }
                    Err(error) => {
                        failed("toast-widevine-install-failed", error.to_string(), cx);
                        drm.fall_back(CdmState::Missing)
                    }
                };
            });
        }));
    }

    /// Removes the module Sonora fetched from Google and settles on whatever else is here, a
    /// browser's copy or nothing. Nothing is asked again this run.
    pub fn uninstall(&mut self, cx: &mut Cx) {
        self.offer = None;
        self.state = CdmState::Looking;

        self.task = Some(cx.spawn(|link| async move {
            let found = tokio::task::spawn_blocking(|| {
                drm::uninstall()?;
                anyhow::Ok(drm::find())
            })
            .await;
            link.post(move |store| {
                let Store { drm, cx, .. } = store;
                drm.task = None;
                drm.state = match found {
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
            });
        }));
    }

    /// The module that was in use before a download the user asked for, or `otherwise` when
    /// there was none. Either way the download no longer stands in front of it.
    fn fall_back(&mut self, otherwise: CdmState) -> CdmState {
        self.standing.take().map_or(otherwise, CdmState::Ready)
    }

    /// Fetches a newer version of a module the user accepted before, without asking. The one
    /// already open stays in use until the next run.
    fn refresh(&mut self, cx: &mut Cx) {
        let io = cx.io.clone();
        self.task = Some(cx.spawn(|link| async move {
            let fetched = io.spawn(drm::fetch()).await;
            link.post(move |store| {
                store.drm.task = None;
                if let Ok(Err(error)) = fetched {
                    log::warn!("drm: cannot refresh the widevine module: {error:#}");
                }
            });
        }));
    }
}

impl Store {
    pub(crate) fn react_drm(&mut self, change: &Change) {
        if let Change::Event(Event::Session(SessionEvent::SignedIn)) = change
            && drm::supported()
            && self.session.wants_drm()
        {
            self.drm.wanted(&mut self.cx);
        }
    }
}

/// Shows a failed step the user started, with the error below it.
fn failed(key: &'static str, error: String, cx: &mut Cx) {
    Toasts::with_detail(Outcome::Failed, key, None, error, cx);
}
