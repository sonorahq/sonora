//! Links handed to the app from outside, by the system or on the command line, opened on the
//! provider they belong to. Sonora's own `sonora:` links offer a theme to install instead.

use gpui::{Context, Entity, EventEmitter, Task};
use music::link::{self, Linked};

use crate::settings::{is_own_link, read_theme_link};
use crate::{AppSettings, Devices, Io, Outcome, Session, SessionState, ThemeOffer, Toasts};

/// A page a link asked for, for the app to navigate to. It never carries a Jam or a short link.
pub struct Opened(pub Linked);

/// A link waiting for its provider to be the signed-in one.
struct Waiting {
    slug: &'static str,
    linked: Linked,
    /// Whether the session was already asked to switch to the provider, so a switch that ends
    /// somewhere else drops the link instead of asking again.
    switched: bool,
}

/// Opens links to any provider's pages and Jams. A link to a provider other than the current
/// one switches to it when it has an account stored, and opens once that provider is signed in.
/// A page comes out as an `Opened` event and a Jam goes to `Devices` to be asked about. A theme
/// link waits in `theme_offer` until the user installs it or turns it down.
pub struct Links {
    session: Entity<Session>,
    devices: Entity<Devices>,
    settings: Entity<AppSettings>,
    waiting: Option<Waiting>,
    following: Option<Task<()>>,
    theme: Option<ThemeOffer>,
}

impl EventEmitter<Opened> for Links {}

impl Links {
    pub fn new(
        session: Entity<Session>,
        devices: Entity<Devices>,
        settings: Entity<AppSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |this, _, cx| this.advance(cx))
            .detach();
        Self {
            session,
            devices,
            settings,
            waiting: None,
            following: None,
            theme: None,
        }
    }

    /// Opens `text` when it is a provider's link or Sonora's own and says whether it was one. A
    /// short link is followed first, so what it points at opens a moment later. A `sonora:` link
    /// that does not read as a theme is refused with a toast.
    pub fn open(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if is_own_link(text) {
            match read_theme_link(text) {
                Ok(offer) => {
                    self.theme = Some(offer);
                    cx.notify();
                }
                Err(error) => {
                    log::warn!("links: cannot read a theme link: {error:#}");
                    Toasts::show(Outcome::Failed, "toast-theme-link-unreadable", cx);
                }
            }
            return true;
        }
        let Some((slug, linked)) = self.session.read(cx).read_link(text) else {
            return false;
        };
        match linked {
            Linked::Short => self.follow(slug, text.trim().to_owned(), cx),
            linked => self.arrive(slug, linked, cx),
        }
        true
    }

    /// Fetches a short link and opens what its address or its page leads to on `slug`.
    fn follow(&mut self, slug: &'static str, short: String, cx: &mut Context<Self>) {
        let fetch = Io::global(cx).spawn(async move { link::follow(&short).await });
        self.following = Some(cx.spawn(async move |this, cx| {
            let followed = fetch.await.ok().flatten();
            this.update(cx, |this, cx| {
                this.following = None;
                let found = followed.and_then(|followed| {
                    let session = this.session.read(cx);
                    std::iter::once(followed.address.as_str())
                        .chain(link::addresses(&followed.page))
                        .find_map(|address| {
                            let (owner, linked) = session.read_link(address)?;
                            (owner == slug && linked != Linked::Short).then_some(linked)
                        })
                });
                match found {
                    Some(linked) => this.arrive(slug, linked, cx),
                    None => Toasts::show(Outcome::Failed, "toast-link-unreadable", cx),
                }
            })
            .ok();
        }));
    }

    fn arrive(&mut self, slug: &'static str, linked: Linked, cx: &mut Context<Self>) {
        self.waiting = Some(Waiting {
            slug,
            linked,
            switched: false,
        });
        self.advance(cx);
    }

    /// Opens the waiting link once its provider is signed in, asks the session to switch to that
    /// provider when it is not, and drops the link when there is no account to switch to.
    fn advance(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = &self.waiting else {
            return;
        };
        let session = self.session.read(cx);
        if session.is_pending() {
            return;
        }
        let here = session.provider_slug() == Some(waiting.slug);
        let ready = matches!(
            session.state(),
            SessionState::SignedIn(_) | SessionState::Offline(_)
        );
        if here && ready {
            if let Some(waiting) = self.waiting.take() {
                self.deliver(waiting.linked, cx);
            }
            return;
        }
        if waiting.switched {
            self.waiting = None;
            return;
        }
        let slug = waiting.slug;
        match session.account(slug) {
            Some((_, true)) => {
                if let Some(waiting) = &mut self.waiting {
                    waiting.switched = true;
                }
                self.session
                    .update(cx, |session, cx| session.switch(slug, cx));
            }
            Some((name, false)) => {
                self.waiting = None;
                Toasts::about(Outcome::Info, "toast-link-sign-in", name, cx);
            }
            None => self.waiting = None,
        }
    }

    /// The theme a link offered, while the user has not answered it.
    pub fn theme_offer(&self) -> Option<&ThemeOffer> {
        self.theme.as_ref()
    }

    pub fn decline_theme(&mut self, cx: &mut Context<Self>) {
        if self.theme.take().is_some() {
            cx.notify();
        }
    }

    /// Installs and selects the offered theme and says whether it worked. A failure is logged
    /// and shown as a toast.
    pub fn install_theme(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(offer) = self.theme.take() else {
            return false;
        };
        cx.notify();
        let installed = self
            .settings
            .update(cx, |settings, cx| settings.install_theme(&offer, cx));
        match installed {
            Ok(id) => {
                log::info!("links: installed theme {id}");
                true
            }
            Err(error) => {
                log::warn!("links: cannot install theme: {error:#}");
                Toasts::about(
                    Outcome::Failed,
                    "toast-theme-install-failed",
                    offer.family.name,
                    cx,
                );
                false
            }
        }
    }

    fn deliver(&mut self, linked: Linked, cx: &mut Context<Self>) {
        match linked {
            Linked::Jam(jam) => self
                .devices
                .update(cx, |devices, cx| devices.peek(&jam, cx)),
            Linked::Short => {}
            linked => cx.emit(Opened(linked)),
        }
    }
}
