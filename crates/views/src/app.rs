//! The window's state: the store, the navigation, the resolved theme and every screen's own view
//! state. Rune hands it to every view as `&Ui` and to every handler as `&mut Ui`.

use std::time::Duration;

use router::{Destination, Navigation, Screen};
use rune::{LiveText, Tx};
use state::Store;
use ui::{Artwork, Look, ScrubberState, Theme, ThemeOverrides};

use crate::chrome::ChromeState;
use crate::screens::Screens;

/// How often the elapsed-time labels follow playback while a track plays. The progress bar
/// follows every frame through its live value; the labels only change once a second.
const TICK: Duration = Duration::from_millis(250);

/// Values that change every frame or every second, bound to elements through Rune's live
/// values so that following them never runs the view.
pub struct Lives {
    /// Where playback is, as a fraction of the track, for every seek bar.
    pub seek: ScrubberState,
    /// The elapsed time, `m:ss`.
    pub elapsed: LiveText,
    /// The track's length, `m:ss`.
    pub total: LiveText,
    /// The last position fed in, so a still player costs nothing.
    last: Option<(u64, u64)>,
}

impl Lives {
    fn new() -> Self {
        Self {
            seek: ScrubberState::new(),
            elapsed: LiveText::new("0:00"),
            total: LiveText::new("0:00"),
            last: None,
        }
    }
}

/// The whole window state.
pub struct Ui {
    pub store: Store,
    pub nav: Navigation,
    pub theme: Theme,
    /// Cover art: fetched, decoded and faded in off the UI thread.
    pub artwork: Option<Artwork>,
    pub live: Lives,
    pub chrome: ChromeState,
    pub screens: Screens,
    /// The overlays any screen opens: the context menu and the dialogs.
    pub dialogs: crate::shared::Dialogs,
    /// What the theme was last resolved from, so a change anywhere re-resolves it once.
    resolved: Option<(Look, ThemeOverrides, bool)>,
    /// Whether the platform reports a dark appearance.
    pub system_dark: bool,
    /// The language and icon pack last applied.
    applied: (String, String),
    bindings: Vec<input::Binding>,
}

impl Ui {
    pub fn new(store: Store) -> Self {
        let startup = store.settings.startup().to_owned();
        let start = Screen::from_id(&startup)
            .unwrap_or(Screen::Home)
            .destination();
        let chrome = ChromeState::new(&store.settings);
        let mut ui = Self {
            store,
            nav: Navigation::new(start.clone()),
            theme: Theme::dark(),
            artwork: None,
            live: Lives::new(),
            chrome,
            screens: Screens::default(),
            dialogs: crate::shared::Dialogs::default(),
            resolved: None,
            system_dark: true,
            applied: (String::new(), String::new()),
            bindings: input::bindings(),
        };
        ui.sync();
        ui.arrive(&start);
        ui
    }

    /// The artwork service. It exists from the window's setup on, before the first frame.
    pub fn artwork(&self) -> &Artwork {
        self.artwork
            .as_ref()
            .expect("the artwork service is installed at setup")
    }

    /// Brings everything derived from settings up to date: the theme, the language, the icon
    /// pack and the motion pace. Cheap when nothing changed.
    pub fn sync(&mut self) {
        let settings = &self.store.settings;
        let wanted = (settings.look(), settings.theme_overrides(), self.system_dark);
        if self.resolved.as_ref() != Some(&wanted) {
            self.theme = Theme::resolve(wanted.0, &wanted.1, wanted.2);
            self.resolved = Some(wanted);
        }
        let applied = (settings.language().to_owned(), settings.icons().to_owned());
        if applied != self.applied {
            i18n::set(i18n::resolve(&applied.0));
            icons::set(&applied.1);
            self.applied = applied;
        }
        ui::motion::set_pace(settings.pace());
        crate::shared::adaptive::wear(self);
    }

    /// Settles what a change raised and re-derives what follows the settings.
    pub fn settle(&mut self) {
        self.store.settle();
        for linked in self.store.take_opened() {
            if let Some(destination) = linked_page(&linked) {
                self.go(destination);
            }
        }
        self.sync();
    }

    /// Goes to `destination`, opening its page.
    pub fn go(&mut self, destination: Destination) {
        if let Some(arrived) = self.nav.go(destination) {
            self.arrive(&arrived);
        }
    }

    pub fn replace(&mut self, destination: Destination) {
        if let Some(arrived) = self.nav.replace(destination) {
            self.arrive(&arrived);
        }
    }

    pub fn back(&mut self) {
        if let Some(arrived) = self.nav.back() {
            self.arrive(&arrived);
        }
    }

    pub fn forward(&mut self) {
        if let Some(arrived) = self.nav.forward() {
            self.arrive(&arrived);
        }
    }

    pub fn up(&mut self, parent: Destination) {
        if let Some(arrived) = self.nav.up(parent) {
            self.arrive(&arrived);
        }
    }

    pub fn open_settings(&mut self) {
        if let Some(arrived) = self.nav.open_settings() {
            self.arrive(&arrived);
        }
    }

    /// Opens the model behind a page that shows one item, so it loads while the page enters.
    fn arrive(&mut self, destination: &Destination) {
        let store = &mut self.store;
        match destination {
            Destination::Album(id) => store.open_album(id),
            Destination::Playlist(id) => store.open_playlist(id),
            Destination::Artist(id) => store.open_artist(id),
            Destination::Song(id) => store.open_song(id),
            Destination::User(id) => store.open_user(id),
            Destination::Genre(id) => store.open_genre(id),
            _ => {}
        }
        self.screens.arrived(destination, &self.store.settings);
        self.settle();
    }

    /// The shortcut a key press asks for, run against the window.
    pub(crate) fn key(&mut self, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
        let typing = tx.focused().is_some_and(|focused| focused.contains("input"));
        let Some(action) = input::action(&self.bindings, event, tx, typing) else {
            return false;
        };
        crate::actions::run(self, action, tx);
        true
    }

    /// Feeds the live values from playback. Called before every frame; returns whether anything
    /// the view reads changed outside a commit.
    pub fn poll(&mut self, tx: &mut Tx) -> bool {
        let playback = &self.store.playback;
        let (elapsed, total) = playback.shown_time(&self.store.devices);
        let fraction = match total.is_zero() {
            true => 0.,
            false => (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.),
        };
        // a drag on a seek bar holds it where the pointer is until the seek lands
        if self.chrome.player_bar.seeking.is_none() {
            self.live.seek.set(fraction);
        }
        let seconds = (elapsed.as_secs(), total.as_secs());
        if self.live.last != Some(seconds) {
            self.live.elapsed.set(ui::clock(elapsed).as_ref());
            self.live.total.set(ui::clock(total).as_ref());
            self.live.last = Some(seconds);
        }
        if playback.wants_playing() {
            tx.wake_after(TICK);
        }
        let artwork = self.artwork.as_ref().is_some_and(Artwork::take_news);
        // the right sidebar's lyrics follow the sung line, and its queue the playing song
        let aside = crate::chrome::sidebar_right::poll(self, tx);
        let tinted = crate::shared::adaptive::wear(self);
        self.store.take_dirty() | artwork | aside | tinted
    }
}

/// A handler that changes the store: runs `f`, then settles what it raised and re-derives the
/// theme and language. Every press that acts on the app goes through it.
pub fn act(f: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut Ui) + 'static {
    move |ui: &mut Ui| {
        f(ui);
        ui.settle();
    }
}

/// [`act`] for a handler that also needs Rune's transaction (scrolling, focus, menus).
pub fn act_tx(f: impl Fn(&mut Ui, &mut Tx) + 'static) -> impl Fn(&mut Ui, &mut Tx) + 'static {
    move |ui: &mut Ui, tx: &mut Tx| {
        f(ui, tx);
        ui.settle();
    }
}

/// The page a link opens on, for the kinds of link that open a page.
fn linked_page(linked: &music::link::Linked) -> Option<Destination> {
    use music::link::Linked;
    match linked {
        Linked::Track(id) => Some(Destination::Song(id.as_str().into())),
        Linked::Album(id) => Some(Destination::Album(id.as_str().into())),
        Linked::Artist(id) => Some(Destination::Artist(id.as_str().into())),
        Linked::Playlist(id) => Some(Destination::Playlist(id.as_str().into())),
        Linked::User(id) => Some(Destination::User(id.as_str().into())),
        Linked::Jam(_) | Linked::Short => None,
    }
}
