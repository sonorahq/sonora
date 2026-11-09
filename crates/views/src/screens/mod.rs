//! The pages the workspace shows, one module each.

pub(crate) mod home;
pub(crate) mod library;
pub(crate) mod search;
pub(crate) mod detail;
pub(crate) mod artist;
pub(crate) mod song;
pub(crate) mod user;
pub(crate) mod genre;
pub(crate) mod history;
pub(crate) mod settings;
pub(crate) mod equalizer;
pub(crate) mod login;

use router::Destination;

/// Every screen's own view state: what a page remembers while it is not on show, such as its
/// text fields, open menus, selection and scroll position.
#[derive(Default)]
pub struct Screens {
    pub home: home::State,
    pub library: library::State,
    pub search: search::State,
    pub detail: detail::State,
    pub artist: artist::State,
    pub song: song::State,
    pub user: user::State,
    pub genre: genre::State,
    pub history: history::State,
    pub settings: settings::State,
    pub equalizer: equalizer::State,
    pub login: login::State,
    pub fullscreen: crate::shells::fullscreen::State,
}

impl Screens {
    /// Lets a screen reset what it keeps per visit as the window arrives at `destination`.
    pub(crate) fn arrived(&mut self, destination: &Destination, settings: &state::AppSettings) {
        self.settings.arrived(destination);
        self.library.arrived(destination, settings);
        self.search.arrived(destination);
        self.detail.arrived(destination, settings);
        self.genre.arrived(destination);
        self.artist.arrived(destination, settings);
    }
}
