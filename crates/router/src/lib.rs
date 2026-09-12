mod link;
mod navigation;
mod uri;

pub use link::Link;
pub use navigation::{Navigation, NavigationEvent};
pub use uri::destination;

use gpui::{App, AppContext as _, Entity, Global, SharedString};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryTab {
    Songs,
    Albums,
    Playlists,
    Artists,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavEntry {
    Home,
    Search,
    Library,
    History,
    Local,
    Pins,
}

impl NavEntry {
    pub const ALL: [Self; 6] = [
        Self::Home,
        Self::Search,
        Self::Library,
        Self::History,
        Self::Local,
        Self::Pins,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Search => "search",
            Self::Library => "library",
            Self::History => "history",
            Self::Local => "local",
            Self::Pins => "sidebar-pins",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Home => "nav-home",
            Self::Search => "nav-search",
            Self::Library => "nav-library",
            Self::History => "nav-history",
            Self::Local => "nav-local",
            Self::Pins => "nav-pinned",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Home,
    Search,
    History,
    Songs,
    Albums,
    Playlists,
    Artists,
    Imported,
}

impl Screen {
    pub const ALL: [Self; 8] = [
        Self::Home,
        Self::Search,
        Self::Songs,
        Self::Albums,
        Self::Artists,
        Self::Playlists,
        Self::Imported,
        Self::History,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Search => "search",
            Self::History => "history",
            Self::Songs => "songs",
            Self::Albums => "albums",
            Self::Playlists => "playlists",
            Self::Artists => "artists",
            Self::Imported => "imported",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Home => "nav-home",
            Self::Search => "nav-search",
            Self::History => "nav-history",
            Self::Songs => "nav-songs",
            Self::Albums => "nav-albums",
            Self::Playlists => "nav-playlists",
            Self::Artists => "nav-artists",
            Self::Imported => "nav-local",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|screen| screen.id() == id)
    }

    pub fn destination(self) -> Destination {
        match self {
            Self::Home => Destination::Home,
            Self::Search => Destination::Search,
            Self::History => Destination::History,
            Self::Songs => Destination::Library(LibraryTab::Songs),
            Self::Albums => Destination::Library(LibraryTab::Albums),
            Self::Playlists => Destination::Library(LibraryTab::Playlists),
            Self::Artists => Destination::Library(LibraryTab::Artists),
            Self::Imported => Destination::Local(LibraryTab::Songs),
        }
    }
}

/// One category of the settings page. The page shows one at a time, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    Playback,
    Privacy,
    Integrations,
    About,
}

impl SettingsTab {
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Appearance,
        Self::Playback,
        Self::Privacy,
        Self::Integrations,
        Self::About,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Playback => "playback",
            Self::Privacy => "privacy",
            Self::Integrations => "integrations",
            Self::About => "about",
        }
    }

    /// The Fluent key of the category's name.
    pub fn key(self) -> &'static str {
        match self {
            Self::General => "settings-tab-general",
            Self::Appearance => "settings-tab-appearance",
            Self::Playback => "settings-tab-playback",
            Self::Privacy => "settings-tab-privacy",
            Self::Integrations => "settings-tab-integrations",
            Self::About => "settings-tab-about",
        }
    }

    /// The icon shown before the category's name in the category bar.
    pub fn icon(self) -> &'static str {
        match self {
            Self::General => "icons/settings.svg",
            Self::Appearance => "icons/palette.svg",
            Self::Playback => "icons/play.svg",
            Self::Privacy => "icons/lock.svg",
            Self::Integrations => "icons/link.svg",
            Self::About => "icons/info.svg",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Home,
    History,
    Library(LibraryTab),
    Local(LibraryTab),
    Album(SharedString),
    Song(SharedString),
    Playlist(SharedString),
    /// A folder of playlists, by the id its provider's library gives it.
    Folder(SharedString),
    Artist(SharedString),
    User(SharedString),
    Genre(SharedString),
    Search,
    Settings(SettingsTab),
    Fullscreen,
}

impl From<&ui::Pin> for Destination {
    fn from(pin: &ui::Pin) -> Self {
        let id = SharedString::from(pin.id.clone());
        match pin.kind {
            ui::PinKind::Album => Destination::Album(id),
            ui::PinKind::Artist => Destination::Artist(id),
            ui::PinKind::Playlist => Destination::Playlist(id),
            ui::PinKind::Folder => Destination::Folder(id),
            ui::PinKind::Song => Destination::Song(id),
        }
    }
}

impl Destination {
    pub fn same_section(&self, other: &Destination) -> bool {
        match (self, other) {
            (Destination::Library(_), Destination::Library(_))
            | (Destination::Local(_), Destination::Local(_))
            | (Destination::Settings(_), Destination::Settings(_)) => true,
            _ => self == other,
        }
    }
}

#[derive(Clone)]
struct Router(Entity<Navigation>);

impl Global for Router {}

pub fn init(start: Destination, cx: &mut App) {
    let navigation = cx.new(|_| Navigation::new(start));
    cx.set_global(Router(navigation));
}

pub fn trail(cx: &App) -> Entity<Navigation> {
    cx.global::<Router>().0.clone()
}

pub fn navigate(destination: Destination, cx: &mut App) {
    trail(cx).update(cx, |navigation, cx| navigation.go(destination, cx));
}

pub fn back(cx: &mut App) {
    trail(cx).update(cx, |navigation, cx| navigation.back(cx));
}

pub fn forward(cx: &mut App) {
    trail(cx).update(cx, |navigation, cx| navigation.forward(cx));
}
