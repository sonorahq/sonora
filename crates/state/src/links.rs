//! Links handed to the app from outside, by the system or on the command line, opened on the
//! provider they belong to. Sonora's own `sonora:` links offer a theme to install instead.

use music::link::{self, Linked};

use crate::settings::{is_own_link, read_theme_link};
use crate::{
    AppSettings, Change, Cx, Event, Job, Outcome, Session, SessionState, Store, ThemeOffer, Toasts,
    Topic,
};

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
/// A page comes out as [`Event::LinkOpened`] and a Jam goes to `Devices` to be asked about. A
/// theme link waits in `theme_offer` until the user installs it or turns it down.
#[derive(Default)]
pub struct Links {
    waiting: Option<Waiting>,
    following: Option<Job>,
    theme: Option<ThemeOffer>,
}

impl Links {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recognizes provider links and Sonora links without opening them or fetching short links.
    pub fn recognizes(&self, text: &str, session: &Session) -> bool {
        is_own_link(text) || session.read_link(text).is_some()
    }

    /// The theme a link offered, while the user has not answered it.
    pub fn theme_offer(&self) -> Option<&ThemeOffer> {
        self.theme.as_ref()
    }

    pub fn decline_theme(&mut self) {
        self.theme = None;
    }

    /// Installs and selects the offered theme and says whether it worked. A failure is logged
    /// and shown as a toast.
    pub fn install_theme(&mut self, settings: &mut AppSettings, cx: &mut Cx) -> bool {
        let Some(offer) = self.theme.take() else {
            return false;
        };
        let installed = settings.install_theme(&offer, cx);
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
}

impl Store {
    /// Opens provider links, browser handoffs and theme offers, returning whether the link was handled.
    /// A recognized link cancels the previous resolution and waiting destination before opening.
    pub fn open_link(&mut self, text: &str) -> bool {
        if !self.links.recognizes(text, &self.session) {
            return false;
        }
        self.links.following = None;
        self.links.waiting = None;
        if let Some(address) = link::address(text)
            .filter(|address| address.scheme() == "sonora" && address.host_str() == Some("open"))
        {
            if let Some(target) = link::query(&address, "url")
                && self.session.read_link(&target).is_some()
            {
                return self.open_link(&target);
            }
            Toasts::show(Outcome::Failed, "toast-link-unreadable", &mut self.cx);
            return true;
        }
        if is_own_link(text) {
            match read_theme_link(text) {
                Ok(offer) => self.links.theme = Some(offer),
                Err(error) => {
                    log::warn!("links: cannot read a theme link: {error:#}");
                    Toasts::show(Outcome::Failed, "toast-theme-link-unreadable", &mut self.cx);
                }
            }
            return true;
        }
        let Some((slug, linked)) = self.session.read_link(text) else {
            return false;
        };
        match linked {
            Linked::Short => self.follow_link(slug, text.trim().to_owned()),
            linked => self.arrive_link(slug, linked),
        }
        true
    }

    /// Installs and selects the theme a link offered and says whether it worked.
    pub fn install_theme_offer(&mut self) -> bool {
        self.links.install_theme(&mut self.settings, &mut self.cx)
    }

    pub(crate) fn react_links(&mut self, change: &Change) {
        if let Change::Topic(Topic::Session) = change {
            self.advance_link();
        }
    }

    /// Fetches a short link and opens what its address or its page leads to on `slug`.
    fn follow_link(&mut self, slug: &'static str, short: String) {
        self.links.following = Some(self.cx.spawn(|back| async move {
            let followed = link::follow(&short).await;
            back.post(move |store| {
                store.links.following = None;
                let found = followed.and_then(|followed| {
                    let session = &store.session;
                    std::iter::once(followed.address.as_str())
                        .chain(link::addresses(&followed.page))
                        .find_map(|address| {
                            let (owner, linked) = session.read_link(address)?;
                            (owner == slug && linked != Linked::Short).then_some(linked)
                        })
                });
                match found {
                    Some(linked) => store.arrive_link(slug, linked),
                    None => Toasts::show(Outcome::Failed, "toast-link-unreadable", &mut store.cx),
                }
            });
        }));
    }

    fn arrive_link(&mut self, slug: &'static str, linked: Linked) {
        self.links.waiting = Some(Waiting {
            slug,
            linked,
            switched: false,
        });
        self.advance_link();
    }

    /// Opens the waiting link once its provider is signed in, asks the session to switch to that
    /// provider when it is not, and drops the link when there is no account to switch to.
    fn advance_link(&mut self) {
        let Some(waiting) = &self.links.waiting else {
            return;
        };
        let session = &self.session;
        if session.is_pending() {
            return;
        }
        let here = session.provider_slug() == Some(waiting.slug);
        let ready = matches!(
            session.state(),
            SessionState::SignedIn(_) | SessionState::Offline(_)
        );
        if here && ready {
            if let Some(waiting) = self.links.waiting.take() {
                self.deliver_link(waiting.linked);
            }
            return;
        }
        if waiting.switched {
            self.links.waiting = None;
            return;
        }
        let slug = waiting.slug;
        match session.account(slug) {
            Some((_, true)) => {
                if let Some(waiting) = &mut self.links.waiting {
                    waiting.switched = true;
                }
                self.session.switch(slug, &self.settings, &mut self.cx);
            }
            Some((name, false)) => {
                self.links.waiting = None;
                Toasts::about(Outcome::Info, "toast-link-sign-in", name, &mut self.cx);
            }
            None => self.links.waiting = None,
        }
    }

    fn deliver_link(&mut self, linked: Linked) {
        match linked {
            Linked::Jam(jam) => self.devices.peek(&jam, &mut self.cx),
            Linked::Short => {}
            linked => self.cx.emit(Event::LinkOpened(linked)),
        }
    }
}
