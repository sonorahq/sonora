use std::cell::RefCell;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FocusHandle, Focusable, MouseButton,
    MouseDownEvent, Pixels, Render, ScrollHandle, SharedString, Stateful, Subscription, Task,
    Window, canvas, div, point, px,
};
use i18n::{Language, t};
use input::SEARCH_CONTEXT;
use music::SignInPrompt;
use router::{Destination, SettingsPage, SettingsTab, navigate};
use state::{
    AppSettings, Drm, Equalizer, Io, Outputs, Playback, Scan, Scrobbling, Session, SessionState,
    Sleep, Soloist, Sonora, Updates,
};
use ui::{
    Activate, ActiveTheme as _, Card, Cascade, Deck, Dismiss, Dropdown, Input, Lazy, Popovers,
    Room, Scrollbar, ScrubberState, SelectNext, SelectPrevious, Spin, Submit, Unfold, Vacancy,
    ease_out_quad, paced, snapped,
};

use crate::chrome::{Chrome, OutputPicker};
use crate::shared::part::Part;
use crate::shared::popups::{Credentials, SearchPopup};
use crate::shared::text;

mod about;
mod accounts;
mod appearance;
mod connect;
mod discord;
mod general;
mod header;
mod keybinds;
mod layout;
mod lyrics;
mod now_playing;
mod playback;
mod player_bar;
mod preview;
mod rows;
mod scrobbling;
mod sessions;
mod sound;
mod storage;
mod themes;

pub use header::SettingsHeader;
use layout::section_title;
pub(crate) use layout::{Heading, Layout, frame};
pub(crate) use rows::Row;

/// The space between two groups of a page.
pub(super) const GROUPS: Pixels = px(40.);
/// How wide the column of group names beside the groups stands, in list rows.
const LABEL_COLUMN: f32 = 2.4;
/// How much a query found in a setting's title counts over one found in its detail.
const TITLE_WEIGHT: u32 = 2;
/// How many settings the panel under the search field suggests.
const SUGGESTED: usize = 6;
/// How long the setting a search result opened stays lit, so the eye finds it once the page
/// has moved to it.
const MARK_HOLD: Duration = Duration::from_millis(1400);
/// How long that light takes to fade once it has held, before the listener's pace stretches it.
const MARK_FADE: Duration = Duration::from_millis(900);
/// The picker keys the page's render watches, shared with the rows that open them.
const LANGUAGES: &str = "languages";
const TYPEFACES: &str = "typefaces";
/// The shortcut that puts the caret in the search field, whose key the field shows.
const OPEN_FILTER: &str = "open-filter";
/// How often the storage section measures the cache and the downloads again while its tab shows.
const STORAGE_POLL: Duration = Duration::from_secs(2);

/// One setting, named without building anything. Pages list their settings as slots in groups,
/// a search scores slots by their title and detail, and only visible groups build their rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Accounts,
    AccountsLink,
    LocalFolder,
    SubsonicLibrary,
    Folders,
    Theme,
    Adaptive,
    CommunityThemes,
    Icons,
    Corners,
    /// The typeface and the text size, chosen in one row.
    Font,
    Opacity,
    /// The blur of the window over the desktop and of the menus over the app, as chips.
    Blur,
    /// The ambient background and its motion, as chips.
    Ambient,
    Visualizer,
    VisualizerScreens,
    VisualizerAbsolute,
    PlayerBarLayout,
    PlayerBarTransparent,
    PlayerBarTimeLeft,
    FullscreenControlsAutohide,
    OsFullscreen,
    Lyrics,
    Karaoke,
    BlurLyrics,
    Romanized,
    PanelLyricsSize,
    FullscreenLyricsSize,
    LyricsSources,
    LyricsPriority,
    WordSyncedFirst,
    LyricsForLocal,
    Normalisation,
    Gapless,
    SponsorBlock,
    KeepTracks,
    CacheSize,
    CacheUsage,
    DownloadsUsage,
    OfflineMode,
    OpenDownloads,
    Sleep,
    StayAwake,
    StreamQuality,
    StreamFormat,
    StreamBitrate,
    Widevine,
    Soloist,
    OutputDevice,
    OutputFailure,
    SampleRate,
    Buffer,
    MatchRate,
    Equalizer,
    SpotifyConnect,
    SpotifyConnectName,
    SpotifyConnectCustom,
    SessionsButton,
    SessionsOfficial,
    SessionsServer,
    /// The Discord status drawn as Discord shows it, changed by clicking its parts.
    Discord,
    /// The status's choices the card cannot show, as chips under it.
    DiscordOptions,
    /// Every scrobbling service, as cards.
    Scrobbles,
    ScrobbleProvider,
    /// The shortcut at this place in `input::shortcuts`.
    Keybind(usize),
    KeybindsReset,
    Startup,
    Provider,
    Entries,
    Language,
    Tray,
    TrayIcon,
    NowPlayingOnFileOpen,
    TaskbarProgress,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    ServerSideDecorations,
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    Decorations,
    #[cfg(not(target_os = "macos"))]
    Side,
    #[cfg(not(target_os = "macos"))]
    TrafficLights,
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos"
    ))]
    WindowRounding,
    Motion,
    Pace,
    Saver,
    AdaptiveMenu,
    Version,
    Updates,
    AutoUpdate,
    Channel,
    Log,
    License,
    Source,
}

/// What a tab scrolls to once it is laid out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// A setting a search result opened.
    Slot(Slot),
    /// The title of a section further down a tab, which a route to that section names.
    Section(SettingsPage),
}

/// One group of a page: its name in a column of its own beside it, an optional block of option
/// cards, and the rows below them. A line sets it apart from the group above.
struct Group {
    title: Option<&'static str>,
    cards: Option<Slot>,
    rows: Vec<Slot>,
}

impl Group {
    /// A group named by the Fluent key `title`.
    fn new(title: &'static str) -> Self {
        Self {
            title: Some(title),
            cards: None,
            rows: Vec::new(),
        }
    }

    /// A group with no name, for a page whose first rows need none.
    fn untitled() -> Self {
        Self {
            title: None,
            cards: None,
            rows: Vec::new(),
        }
    }

    /// The setting drawn as option cards between the heading and the rows.
    fn cards(mut self, slot: Slot) -> Self {
        self.cards = Some(slot);
        self
    }

    fn rows(mut self, rows: impl IntoIterator<Item = Slot>) -> Self {
        self.rows.extend(rows);
        self
    }

    /// Every setting the group shows, cards first.
    fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.cards.into_iter().chain(self.rows.iter().copied())
    }
}

pub struct SettingsView {
    session: Entity<Session>,
    playback: Entity<Playback>,
    /// Measures the cache and the downloads again every so often while the storage section's
    /// tab is on screen, since tracks are kept and evicted as they play.
    _storage: Task<()>,
    drm: Entity<Drm>,
    soloist: Entity<Soloist>,
    equalizer: Entity<Equalizer>,
    settings: Entity<AppSettings>,
    outputs: Entity<Outputs>,
    output_picker: Entity<OutputPicker>,
    /// The page the route names: a page of the tab on show, or a drill-in.
    page: SettingsPage,
    search: Entity<Input>,
    /// The search field's text, trimmed, which the panel under the field suggests settings for.
    typed: String,
    /// The query whose every match the page lists in place of its own settings, set by Enter in
    /// the search field. Empty means the page shows its own settings.
    query: String,
    /// Whether the panel under the search field is put away. Escape, Enter, a chosen result and
    /// the field losing the keyboard put it away. A click in the field or typing brings it back.
    dismissed: bool,
    /// The panel row the arrow keys are on.
    pick: Option<usize>,
    /// The panel's rows surfacing one after another as the query changes.
    cascade: Cascade,
    /// The panel's box, stretching to fit as rows come and go.
    unfold: Unfold,
    /// What the tab scrolls into view once it is laid out: the setting a search result opened,
    /// or the section a route named.
    reveal: Option<Reveal>,
    /// The setting a search result opened and when, lit for a moment after the page moves to it.
    marked: Option<(Slot, Instant)>,
    /// Whether the next route keeps the search, for the list Enter asks for on the equalizer,
    /// which shares the header but not the page that lists it.
    keep_search: bool,
    /// Where a lyrics source dragged over the priority list would land, as a gap between the
    /// enabled sources. `None` while the drop would leave the order as it is.
    lyrics_drop_gap: Option<usize>,
    scrollbar: Entity<Scrollbar>,
    /// The page that opened a drill-in and where its tab was scrolled, so the way back to that
    /// page or to the drill-in's parent lands there again.
    drilled_from: Option<(SettingsPage, Pixels)>,
    /// Invalidates off-screen heights when data changes, without rebuilding them for each
    /// playback tick, animation frame or scroll update.
    layout_revision: u64,
    /// The column width, text size and rounding the cached heights were measured for.
    geometry: Option<(Pixels, Pixels, Pixels)>,
    opacity: ScrubberState,
    sleep: ScrubberState,
    pending_sleep: Option<Option<Sleep>>,
    cache: ScrubberState,
    /// The cache size a drag of its slider points at, until it lets go.
    pending_cache: Option<u32>,
    popovers: Popovers,
    /// The theme family whose variant menu opened last, which owns that menu's popover.
    theme_variants: Option<String>,
    /// The user's theme family waiting for the user to confirm its deletion.
    deleting_theme: Option<String>,
    /// The fields of a sign-in to a server of the user's own.
    credentials: Credentials,
    credentials_for: Option<&'static str>,
    secret: Entity<Input>,
    manual_secret: Option<(&'static str, &'static str)>,
    scrobbling: Entity<Scrobbling>,
    scrobble_first: Entity<Input>,
    scrobble_second: Entity<Input>,
    /// The field for the custom name Sonora goes by in Spotify's device list.
    connect_name: Entity<Input>,
    /// The address of the listener's own Sonora Sessions server.
    sessions_server: Entity<Input>,
    /// The service whose link dialog is open, by slug.
    scrobble_prompt: Option<&'static str>,
    /// The provider whose sign-in choice is up, by slug and name.
    sign_in_for: Option<(&'static str, &'static str)>,
    /// Holds the key focus while a dialog is up, so escape reaches the page and closes it,
    /// and is where the language and typeface pickers put it back when they close.
    focus: FocusHandle,
    /// Whether the focus has already been taken for the dialog that is up.
    grabbed: bool,
    /// Whether the choice the dialog offered has been taken. The dialog stays up until the
    /// sign-in it started is over, so the veil never blinks away between it and the prompt
    /// that follows.
    sign_in_running: bool,
    /// The card the user just switched to, by slug or `guest`. Its radio fills while the
    /// session tears the old provider down and brings the new one up, which reports nothing
    /// active in between.
    chosen: Option<&'static str>,
    /// Whether the library picker is up.
    library_picker: bool,
    /// The row the picker's keyboard cursor is on. `None` puts it on the choice in effect, which
    /// is where it starts once the libraries arrive.
    library_cursor: Option<usize>,
    /// The option list's scroll position, moved to follow the cursor.
    library_scroll: ScrollHandle,
    /// The library the stored account reads, held so a render never reads the stored account.
    selected_library: Option<music::Library>,
    languages: SearchPopup,
    typefaces: SearchPopup,
    typeface_faced: RefCell<HashSet<SharedString>>,
    installed: Option<Vec<SharedString>>,
    loading_fonts: bool,
    font_task: Option<Task<()>>,
    /// What the pages' previews keep between frames.
    previews: preview::Previews,
    /// Each page's groups as a view of its own, made the first time the page shows, so a preview
    /// that moves renders the page without building them again.
    parts: Vec<(SettingsPage, Entity<Part<SettingsView>>)>,
    /// The shortcut, by id, whose new key the page is waiting for.
    recording: Option<&'static str>,
    /// Catches every key pressed while a shortcut records, before it does anything else.
    recorder: Option<Subscription>,
    updates: Entity<Updates>,
    /// Turns the icon of the button that checks for updates while the check runs.
    update_spin: RefCell<Spin>,
}

impl SettingsView {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let scrobbling = Sonora::global(cx).scrobbling.clone();
        let drm = Sonora::global(cx).drm.clone();
        let equalizer = Sonora::global(cx).equalizer.clone();
        let outputs = Sonora::global(cx).outputs.clone();
        let soloist = Sonora::global(cx).soloist.clone();
        let updates = Sonora::global(cx).updates.clone();
        cx.observe(&updates, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&drm, |this, _, cx| this.remeasure(cx)).detach();
        cx.observe(&soloist, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&equalizer, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&outputs, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&session, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&scrobbling, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&settings, |this, _, cx| {
            this.show_filter_shortcut(cx);
            this.remeasure(cx);
        })
        .detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        playback.update(cx, |playback, cx| playback.measure_storage(cx));
        let storage = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STORAGE_POLL).await;
                let measured = this.update(cx, |this, cx| {
                    let shown = router::trail(cx).read(cx).current();
                    if matches!(shown, Destination::Settings(page) if page.tab() == SettingsTab::Playback) {
                        this.playback
                            .update(cx, |playback, cx| playback.measure_storage(cx));
                    }
                });
                if measured.is_err() {
                    return;
                }
            }
        });
        cx.observe(&Scan::global(cx), |this, _, cx| this.remeasure(cx))
            .detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<ui::WindowBlur>(|this, cx| this.remeasure(cx))
            .detach();
        let me = cx.entity_id();
        let focus = cx.focus_handle();
        let languages = SearchPopup::new("settings-language-search", me, focus.clone(), cx);
        cx.observe(&languages.input(), |this, _, cx| {
            this.languages.changed(cx);
            cx.notify();
        })
        .detach();
        let typefaces = SearchPopup::new("settings-typeface-search", me, focus.clone(), cx);
        cx.observe(&typefaces.input(), |this, _, cx| {
            this.typefaces.changed(cx);
            cx.notify();
        })
        .detach();
        let search = cx.new(|cx| {
            Input::new("settings-search", cx)
                .icon("icons/search.svg")
                .blurred()
                .clearable()
        });
        let filter =
            input::shown(OPEN_FILTER, settings.read(cx).keybinds()).map(SharedString::from);
        search.update(cx, |input, cx| input.set_shortcut(filter, cx));
        cx.observe(&search, |this, input, cx| {
            let typed = input.read(cx).text().trim().to_owned();
            if typed == this.typed {
                return;
            }
            this.typed = typed;
            this.dismissed = false;
            this.pick = None;
            // An emptied field goes back to the page rather than keeping the last results.
            if this.typed.is_empty() {
                this.list(String::new(), cx);
            }
            cx.notify();
        })
        .detach();

        let connect_name = cx.new(|cx| Input::new("settings-spotify-connect-name", cx).compact());
        let custom = settings.read(cx).spotify_connect_custom_name().to_owned();
        connect_name.update(cx, |input, cx| input.set_text(custom, cx));
        cx.observe(&connect_name, |this, input, cx| {
            let text = input.read(cx).text().to_owned();
            this.settings.update(cx, |settings, cx| {
                settings.set_spotify_connect_custom_name(text, cx)
            });
        })
        .detach();

        let sessions_server =
            cx.new(|cx| Input::new("settings-sessions-server-hint", cx).compact());
        let typed = settings.read(cx).sessions_server().to_owned();
        sessions_server.update(cx, |input, cx| input.set_text(typed, cx));
        // the Save button stands held while the server it saves is checked, and the field wears a
        // red border from a save that was turned down until one goes through
        let listening = Sonora::global(cx).listening.clone();
        cx.observe(&listening, |this: &mut Self, listening, cx| {
            let rejected = listening.read(cx).rejected();
            this.sessions_server
                .update(cx, |input, cx| input.set_invalid(rejected, cx));
            cx.notify();
        })
        .detach();
        // what is typed is kept only once the Save button finds its server answers
        cx.observe(&sessions_server, |_, _, cx| cx.notify())
            .detach();

        let popovers = Popovers::default();
        let output_picker = cx.new(|cx| OutputPicker::new(outputs.clone(), popovers.clone(), cx));
        let previews = preview::Previews::new(cx);

        let selected_library = session.read(cx).library();
        Self {
            session,
            updates,
            update_spin: RefCell::default(),
            _storage: storage,
            playback,
            drm,
            soloist,
            equalizer,
            settings,
            outputs,
            output_picker,
            page: SettingsTab::General.lead(),
            search,
            typed: String::new(),
            query: String::new(),
            dismissed: false,
            pick: None,
            cascade: Cascade::default(),
            unfold: Unfold::default(),
            reveal: None,
            marked: None,
            keep_search: false,
            lyrics_drop_gap: None,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            drilled_from: None,
            layout_revision: 0,
            geometry: None,
            opacity: ScrubberState::new("opacity"),
            sleep: ScrubberState::new("sleep"),
            pending_sleep: None,
            cache: ScrubberState::new("cache-size"),
            pending_cache: None,
            popovers,
            theme_variants: None,
            deleting_theme: None,
            credentials: Credentials::new(cx),
            credentials_for: None,
            secret: cx.new(|cx| Input::new("login-cookie-hint", cx)),
            manual_secret: None,
            sign_in_for: None,
            focus,
            grabbed: false,
            sign_in_running: false,
            chosen: None,
            library_picker: false,
            library_cursor: None,
            library_scroll: ScrollHandle::new(),
            selected_library,
            scrobbling,
            scrobble_first: cx.new(|cx| Input::new("common-api-key", cx)),
            scrobble_second: cx.new(|cx| Input::new("settings-scrobble-secret", cx)),
            connect_name,
            sessions_server,
            scrobble_prompt: None,
            languages,
            typefaces,
            typeface_faced: RefCell::new(HashSet::new()),
            installed: None,
            loading_fonts: false,
            font_task: None,
            previews,
            parts: Vec::new(),
            recording: None,
            recorder: None,
        }
    }

    /// Shows the tab a page belongs to and ends any search, so a route always lands on a plain
    /// tab, unless the route is the one that carries a list asked for on the equalizer. A new route starts at the top of its tab, or at its section when that lies further
    /// down, and a route to the page already on show keeps the tab where it was scrolled. A
    /// drill-in going back to its tab returns to where the tab was scrolled when it opened.
    pub(crate) fn select(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        if self.page != page {
            let back = self
                .drilled_from
                .take()
                .filter(|(from, _)| {
                    self.page.parent().is_some()
                        && (page == *from || self.page.parent() == Some(page))
                })
                .map(|(_, at)| at);
            let drilling = page.parent().is_some()
                && self.page.parent().is_none()
                && self.page.tab() == page.tab()
                && !self.searching();
            if drilling {
                let at = self.scrollbar.read(cx).scroll().offset().y;
                self.drilled_from = Some((self.page, at));
            }
            self.scrollbar.update(cx, |bar, _| {
                bar.place(Pixels::ZERO);
                // the drill-in on show is shorter than the tab, so the offset is handed to the
                // handle as is and clamped against the tab once it lays out
                if let Some(at) = back {
                    bar.scroll().set_offset(point(Pixels::ZERO, at));
                }
            });
            // a search result names a setting on the page it opens, and any other route to a
            // later page of a tab reveals that page's title
            self.reveal = match self.reveal {
                Some(Reveal::Slot(slot))
                    if self
                        .groups(page, cx)
                        .iter()
                        .any(|group| group.slots().any(|shown| shown == slot)) =>
                {
                    Some(Reveal::Slot(slot))
                }
                _ => (back.is_none() && page.parent().is_none() && page != page.tab().lead())
                    .then_some(Reveal::Section(page)),
            };
        }
        if self.page != page {
            self.stop_recording(cx);
        }
        self.page = page;
        self.popovers.close();
        self.previews.shown(page.tab());
        // a device plugged in since the list was read is only found by asking again
        if page.tab() == SettingsTab::Playback {
            self.outputs.update(cx, |outputs, cx| outputs.refresh(cx));
        }
        if page.tab() == SettingsPage::Providers.tab() {
            self.selected_library = self.session.read(cx).library();
        }
        if !std::mem::take(&mut self.keep_search) {
            self.clear_search(cx);
        }
        cx.notify();
    }

    /// Tells the previews whether settings are open, so a new opening picks a new track and a
    /// closing stops what they keep running.
    pub(crate) fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        match open {
            true => self.previews.opened(),
            false => {
                self.previews.closed();
                self.stop_recording(cx);
            }
        }
    }

    pub(crate) fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// The search field, which the keyboard shortcut for the page's filter focuses.
    pub(crate) fn search(&self) -> Entity<Input> {
        self.search.clone()
    }

    /// The search field with the panel of the settings that best answer what is typed, which the
    /// header draws. Enter opens the row the arrow keys are on, or else lists every match in
    /// place of the page. Building it never redraws the view, since the header draws it.
    pub(crate) fn search_field(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // The field losing the keyboard puts the panel away, so focusing it again leaves the panel
        // shut until a click or a key asks for it. A change of focus redraws every view, so the
        // header always sees it. A blur listener would not do: the panel opening under the cached
        // header reads to it as a blur while the field keeps the keyboard.
        if !self.typing(window, cx) {
            self.dismissed = true;
            self.pick = None;
        }
        let open = self.panel_open(window, cx);
        // A closed panel forgets its rows and height, so the next one unfolds from nothing.
        if !open {
            self.cascade.clear();
            self.unfold.reset();
        }
        let panel = open.then(|| self.panel(window, cx));
        let field = div()
            // A click in the field asks for the panel. It listens in the capture phase because the
            // field stops the press.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left && this.dismissed {
                    this.dismissed = false;
                    cx.notify();
                }
            }))
            .child(self.search.clone());
        let me = cx.entity().downgrade();

        div()
            .w_full()
            .key_context(SEARCH_CONTEXT)
            .on_action(cx.listener(Self::suggest_next))
            .on_action(cx.listener(Self::suggest_previous))
            .on_action(cx.listener(Self::submit_search))
            .child(
                Dropdown::new("settings-search-field", field)
                    .w_full()
                    .open(open)
                    .on_close(move |_, cx| {
                        me.update(cx, |this, cx| this.close_panel(cx)).ok();
                    })
                    .children(panel),
            )
            .into_any_element()
    }

    pub(crate) fn clear_search(&mut self, cx: &mut Context<Self>) {
        if !self.search.read(cx).text().is_empty() {
            self.search.update(cx, |input, cx| input.set_text("", cx));
        }
    }

    /// Lists every setting that answers `query` in place of the page, or brings the page back for
    /// an empty one. A new list starts at the top, since where the page was scrolled to answers
    /// for nothing there.
    fn list(&mut self, query: String, cx: &mut Context<Self>) {
        if self.query == query {
            return;
        }
        self.scrollbar.update(cx, |bar, _| bar.place(Pixels::ZERO));
        if !query.is_empty() {
            self.reveal = None;
        }
        self.query = query;
        cx.notify();
    }

    /// Whether the panel under the search field shows: the field has the keyboard and holds
    /// something, and nothing put the panel away.
    fn panel_open(&self, window: &Window, cx: &App) -> bool {
        !self.dismissed && !self.typed.is_empty() && self.typing(window, cx)
    }

    /// Whether the search field has the keyboard.
    fn typing(&self, window: &Window, cx: &App) -> bool {
        self.search.read(cx).focus_handle(cx).is_focused(window)
    }

    fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.dismissed = true;
        self.pick = None;
        cx.notify();
    }

    /// The settings the panel under the search field suggests for what is typed, best first.
    fn offers(&self, cx: &App) -> Vec<(SettingsPage, Slot)> {
        let mut offers = self.found(&self.typed, cx);
        offers.truncate(SUGGESTED);
        offers
    }

    /// Down in the search field: the next row of the panel, or the panel back once it was put
    /// away.
    fn suggest_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panel_open(window, cx) {
            if self.dismissed && !self.typed.is_empty() {
                self.dismissed = false;
                cx.notify();
            }
            return;
        }
        let Some(last) = self.offers(cx).len().checked_sub(1) else {
            return;
        };
        self.pick = Some(self.pick.map_or(0, |at| (at + 1).min(last)));
        cx.notify();
    }

    /// Up in the search field: the row above in the panel, or back to the text from the first.
    fn suggest_previous(
        &mut self,
        _: &SelectPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.panel_open(window, cx) {
            return;
        }
        if let Some(at) = self.pick {
            self.pick = at.checked_sub(1);
            cx.notify();
        }
    }

    /// Enter in the search field: opens the panel row the arrow keys are on, or else lists every
    /// setting that answers what is typed in place of the page.
    fn submit_search(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        let picked = self
            .pick
            .filter(|_| self.panel_open(window, cx))
            .and_then(|at| self.offers(cx).get(at).copied());
        match picked {
            Some((page, slot)) => self.open_result(page, slot, window, cx),
            None => {
                self.list(self.typed.clone(), cx);
                self.close_panel(cx);
                // the equalizer draws the header over a page of its own, so the list opens on
                // the settings page it came from
                let shown = router::trail(cx).read(cx).current();
                if self.searching() && !matches!(shown, Destination::Settings(_)) {
                    self.keep_search = true;
                    navigate(Destination::Settings(self.page), cx);
                }
            }
        }
    }

    /// Takes one step back out of whatever is open on the page: a dialog first, then a menu,
    /// then the search. Returns false when nothing was open.
    pub(crate) fn escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.close_dialog(cx) {
            return true;
        }
        if self.popovers.any() {
            self.popovers.close();
            cx.notify();
            return true;
        }
        if !self.search.read(cx).text().is_empty() {
            self.clear_search(cx);
            return true;
        }
        false
    }

    /// The groups a page shows, in order.
    fn groups(&self, page: SettingsPage, cx: &App) -> Vec<Group> {
        match page {
            SettingsPage::Providers => self.accounts_groups(),
            SettingsPage::Appearance => self.appearance_groups(cx),
            SettingsPage::PlayerBar => self.player_bar_groups(),
            SettingsPage::NowPlaying => self.now_playing_groups(cx),
            SettingsPage::Lyrics => self.lyrics_groups(cx),
            SettingsPage::Playback => self.playback_groups(cx),
            SettingsPage::Sound => self.sound_groups(cx),
            SettingsPage::Storage => self.storage_groups(cx),
            SettingsPage::Discord => self.discord_groups(cx),
            SettingsPage::Scrobbling => self.scrobbling_groups(cx),
            SettingsPage::General => self.general_groups(),
            SettingsPage::About => self.about_groups(),
            SettingsPage::Keybinds => self.keybinds_groups(),
            SettingsPage::LyricsSources => self.lyrics_sources_groups(),
            SettingsPage::MusicFolders => self.music_folders_groups(),
            SettingsPage::Accounts => self.accounts_page_groups(cx),
        }
    }

    /// Draws the key that opens the search in its field, which the user may have rebound.
    fn show_filter_shortcut(&self, cx: &mut Context<Self>) {
        let filter =
            input::shown(OPEN_FILTER, self.settings.read(cx).keybinds()).map(SharedString::from);
        self.search
            .update(cx, |input, cx| input.set_shortcut(filter, cx));
    }

    /// Invalidates cached block heights after data or settings change their layout.
    fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.layout_revision = self.layout_revision.wrapping_add(1);
        cx.notify();
    }

    /// Keeps a block's measured height while it is off screen and builds its content only
    /// when visible, or once after its layout changes.
    fn lazy(
        &self,
        id: impl Into<ElementId>,
        cx: &Context<Self>,
        build: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> AnyElement + 'static,
    ) -> Lazy {
        let view = cx.entity();
        Lazy::new(id, move |window, cx| {
            view.update(cx, |this, cx| build(this, window, cx))
        })
        .revision(self.layout_revision)
    }

    /// What a page shows between its title and its groups, built only while visible.
    fn header(&self, page: SettingsPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        match page {
            SettingsPage::Accounts | SettingsPage::About => Some(
                self.lazy((page.id(), 0_usize), cx, move |this, _, cx| match page {
                    SettingsPage::Accounts => this.profile(cx).into_any_element(),
                    _ => this.mark(cx).into_any_element(),
                })
                .into_any_element(),
            ),
            _ => None,
        }
    }

    /// The switch at the end of a page's title that turns the page's feature on, for a page
    /// whose every other setting shapes that feature.
    fn title_control(&self, page: SettingsPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        match page {
            SettingsPage::Discord => Some(self.discord_switch(cx)),
            _ => None,
        }
    }

    /// What a page shows under its groups, each part set apart by a line as a group is.
    fn footer(
        &self,
        page: SettingsPage,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        match page {
            SettingsPage::About => vec![
                self.lazy((page.id(), 2_usize), cx, |this, window, cx| {
                    this.team(window, cx).into_any_element()
                })
                .into_any_element(),
                beside(None, true, false, self.notice(cx), window, cx),
            ],
            _ => Vec::new(),
        }
    }

    /// The live preview above a page's first group, built and animated only while visible.
    fn preview(&self, page: SettingsPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !matches!(
            page,
            SettingsPage::NowPlaying | SettingsPage::Lyrics | SettingsPage::Sound
        ) {
            return None;
        }
        let view = cx.entity();
        Some(
            self.lazy((page.id(), 1_usize), cx, move |this, window, cx| {
                let previews = &this.previews;
                match page {
                    SettingsPage::NowPlaying => {
                        Some(previews.now_playing.clone().into_any_element())
                    }
                    SettingsPage::Lyrics => Some(previews.lyrics.clone().into_any_element()),
                    SettingsPage::Sound => {
                        preview::sound::preview(&previews.sound, this, window, cx)
                    }
                    _ => None,
                }
                .unwrap_or_else(|| div().into_any_element())
            })
            .on_visibility(move |shown, _, cx| {
                if !shown && page == SettingsPage::Sound {
                    view.update(cx, |this, _| this.previews.sound.hide());
                }
            })
            .into_any_element(),
        )
    }

    /// Every setting of every page that answers the query, best match first, with the page it
    /// lives on. Ties keep the order of the pages. Only titles and details are read and no row
    /// is built.
    fn found(&self, query: &str, cx: &App) -> Vec<(SettingsPage, Slot)> {
        let mut hits: Vec<(u32, SettingsPage, Slot)> = SettingsPage::ALL
            .into_iter()
            .flat_map(|page| {
                self.groups(page, cx)
                    .into_iter()
                    .flat_map(|group| group.slots().collect::<Vec<_>>())
                    .map(move |slot| (page, slot))
            })
            .filter_map(|(page, slot)| {
                self.slot_score(slot, query, cx)
                    .map(|score| (score, page, slot))
            })
            .collect();
        hits.sort_by_key(|(score, ..)| std::cmp::Reverse(*score));
        hits.into_iter()
            .map(|(_, page, slot)| (page, slot))
            .collect()
    }

    /// How well the slot answers `query`, or `None` when it does not. The query may span
    /// the title and the detail, and a hit in the title alone counts on top.
    fn slot_score(&self, slot: Slot, query: &str, cx: &App) -> Option<u32> {
        let (title, detail) = self.slot_meta(slot, cx)?;
        let both = format!("{} {}", title, detail);
        let whole = text::fuzzy(&both, query)?;
        let titled = text::fuzzy(&title, query).unwrap_or(0) * TITLE_WEIGHT;
        Some(whole + titled)
    }

    /// The title and detail a search is matched against, without building the row. Kept next
    /// to the slots: a new row needs its pair here to be found. A slot with no pair is left out
    /// of searches.
    fn slot_meta(&self, slot: Slot, cx: &App) -> Option<(SharedString, SharedString)> {
        let meta = match slot {
            Slot::Folders | Slot::AccountsLink => return None,
            Slot::Startup => (t!("settings-startup"), t!("settings-startup-detail")),
            Slot::Provider => (t!("settings-provider"), t!("settings-provider-detail")),
            Slot::Entries => (t!("settings-entries"), t!("settings-entries-detail")),
            Slot::Language => (t!("settings-language"), t!("settings-language-detail")),
            Slot::Tray => (
                t!("settings-close-to-tray"),
                t!("settings-close-to-tray-detail"),
            ),
            Slot::TrayIcon => (t!("settings-tray-icon"), t!("settings-tray-icon-detail")),
            Slot::NowPlayingOnFileOpen => (
                t!("settings-now-playing-on-file-open"),
                t!("settings-now-playing-on-file-open-detail"),
            ),
            Slot::TaskbarProgress => (
                t!("settings-taskbar-progress"),
                t!("settings-taskbar-progress-detail"),
            ),
            Slot::Accounts => {
                let detail = t!("settings-accounts-detail");
                let names = self.account_words(cx);
                (t!("settings-accounts"), format!("{detail} {names}").into())
            }
            Slot::LocalFolder => (
                t!("settings-music-folders"),
                match self.session.read(cx).local_paths().is_empty() {
                    true => t!("settings-local-folder-empty"),
                    false => SharedString::default(),
                },
            ),
            Slot::SubsonicLibrary => (t!("settings-subsonic-library"), SharedString::default()),
            Slot::Theme => (t!("settings-theme"), t!("settings-theme-detail")),
            Slot::Adaptive => (t!("settings-adaptive"), t!("settings-adaptive-detail")),
            Slot::CommunityThemes => (
                t!("settings-theme-explore"),
                t!("settings-theme-explore-detail"),
            ),
            Slot::Ambient => (
                t!("settings-group-background"),
                format!(
                    "{} {} {} {}",
                    t!("settings-ambient"),
                    t!("settings-ambient-detail"),
                    t!("settings-ambient-motion"),
                    t!("settings-ambient-motion-detail")
                )
                .into(),
            ),
            Slot::Visualizer => (t!("settings-visualizer"), t!("settings-visualizer-detail")),
            Slot::PlayerBarLayout => (
                t!("settings-player-bar-layout"),
                t!("settings-player-bar-layout-detail"),
            ),
            Slot::PlayerBarTransparent => (
                t!("settings-player-bar-transparent"),
                t!("settings-player-bar-transparent-detail"),
            ),
            Slot::PlayerBarTimeLeft => (
                t!("settings-player-bar-time-left"),
                t!("settings-player-bar-time-left-detail"),
            ),
            Slot::VisualizerScreens => (
                t!("settings-visualizer-screens"),
                t!("settings-visualizer-screens-detail"),
            ),
            Slot::VisualizerAbsolute => (
                t!("settings-visualizer-absolute"),
                t!("settings-visualizer-absolute-detail"),
            ),
            Slot::Icons => (t!("settings-icons"), t!("settings-icons-detail")),
            Slot::Opacity => (t!("settings-opacity"), t!("settings-opacity-detail")),
            Slot::Blur => (
                t!("settings-blur-title"),
                format!(
                    "{} {} {} {}",
                    t!("settings-blur-window"),
                    match ui::COMPOSITOR_OPACITY {
                        true => t!("settings-blur-window-compositor-detail"),
                        false => t!("settings-blur-window-detail"),
                    },
                    t!("settings-blur"),
                    t!("settings-blur-detail")
                )
                .into(),
            ),
            Slot::Corners => (t!("settings-corners"), t!("settings-corners-detail")),
            Slot::FullscreenControlsAutohide => (
                t!("settings-fullscreen-controls-autohide"),
                t!("settings-fullscreen-controls-autohide-detail"),
            ),
            Slot::OsFullscreen => (
                t!("settings-os-fullscreen"),
                t!("settings-os-fullscreen-detail"),
            ),
            Slot::PanelLyricsSize => (
                t!("settings-panel-lyrics-size"),
                t!("settings-panel-lyrics-size-detail"),
            ),
            Slot::FullscreenLyricsSize => (
                t!("settings-fullscreen-lyrics-size"),
                t!("settings-fullscreen-lyrics-size-detail"),
            ),
            Slot::BlurLyrics => (
                t!("settings-blur-lyrics"),
                t!("settings-blur-lyrics-detail"),
            ),
            Slot::Font => (
                t!("settings-typeface"),
                format!(
                    "{} {} {}",
                    t!("settings-typeface-detail"),
                    t!("settings-font"),
                    t!("settings-font-detail")
                )
                .into(),
            ),
            Slot::Motion => (t!("settings-motion"), t!("settings-motion-detail")),
            Slot::Pace => (t!("settings-pace"), t!("settings-pace-detail")),
            Slot::Saver => (t!("settings-saver"), t!("settings-saver-detail")),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Slot::ServerSideDecorations => (
                t!("settings-server-side-decorations"),
                t!("settings-server-side-decorations-detail"),
            ),
            #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
            Slot::Decorations => (
                t!("settings-window-controls"),
                t!("settings-window-controls-detail"),
            ),
            #[cfg(not(target_os = "macos"))]
            Slot::Side => (
                t!("settings-controls-side"),
                t!("settings-controls-side-detail"),
            ),
            #[cfg(not(target_os = "macos"))]
            Slot::TrafficLights => (
                t!("settings-traffic-light-controls"),
                t!("settings-traffic-light-controls-detail"),
            ),
            #[cfg(any(
                target_os = "windows",
                target_os = "linux",
                target_os = "freebsd",
                target_os = "macos"
            ))]
            Slot::WindowRounding => (
                t!("settings-window-rounding"),
                t!("settings-window-rounding-detail"),
            ),
            Slot::AdaptiveMenu => (
                t!("settings-adaptive-menu"),
                t!("settings-adaptive-menu-detail"),
            ),
            Slot::Normalisation => (
                t!("settings-normalisation"),
                t!("settings-normalisation-detail"),
            ),
            Slot::Gapless => (t!("settings-gapless"), t!("settings-gapless-detail")),
            Slot::SponsorBlock => (
                t!("settings-sponsorblock"),
                t!("settings-sponsorblock-detail"),
            ),
            Slot::KeepTracks => (
                t!("settings-keep-tracks"),
                t!("settings-keep-tracks-detail"),
            ),
            Slot::CacheSize => (t!("settings-cache-size"), t!("settings-cache-size-detail")),
            Slot::CacheUsage => (t!("settings-space-used"), t!("settings-cache-used-detail")),
            Slot::DownloadsUsage => (
                t!("settings-space-used"),
                t!("settings-downloads-used-detail"),
            ),
            Slot::OfflineMode => (
                t!("settings-offline-mode"),
                t!("settings-offline-mode-detail"),
            ),
            Slot::OpenDownloads => (
                t!("settings-downloads-open"),
                t!("settings-downloads-open-detail"),
            ),
            Slot::StreamQuality => (
                t!("settings-stream-quality"),
                t!("settings-stream-quality-detail"),
            ),
            Slot::StreamFormat => (
                t!("settings-stream-format"),
                t!("settings-stream-format-detail"),
            ),
            Slot::StreamBitrate => (
                t!("settings-stream-bitrate"),
                t!("settings-stream-bitrate-detail"),
            ),
            Slot::Sleep => (t!("settings-sleep"), t!("settings-sleep-detail")),
            Slot::StayAwake => (t!("settings-stay-awake"), t!("settings-stay-awake-detail")),
            Slot::Widevine => (t!("settings-widevine"), self.widevine_detail(cx)),
            Slot::Soloist => (t!("settings-soloist"), t!("settings-soloist-detail")),
            Slot::OutputDevice => (
                t!("settings-output-device"),
                t!("settings-output-device-detail"),
            ),
            Slot::OutputFailure => self.output_failure(cx),
            Slot::SampleRate => (
                t!("settings-sample-rate"),
                t!("settings-sample-rate-detail"),
            ),
            Slot::Buffer => (t!("settings-buffer"), t!("settings-buffer-detail")),
            Slot::MatchRate => (t!("settings-match-rate"), t!("settings-match-rate-detail")),
            Slot::Equalizer => (
                t!("settings-equalizer"),
                format!(
                    "{} {}",
                    t!("settings-equalizer-detail"),
                    t!("settings-equalizer-preset-detail")
                )
                .into(),
            ),
            Slot::LyricsSources => (
                t!("settings-lyrics-sources"),
                t!("settings-lyrics-providers-detail"),
            ),
            Slot::LyricsPriority => (
                t!("settings-lyrics-priority"),
                t!("settings-lyrics-priority-detail"),
            ),
            Slot::WordSyncedFirst => (
                t!("settings-lyrics-word-synced-first"),
                t!("settings-lyrics-word-synced-first-detail"),
            ),
            Slot::Lyrics => (t!("settings-lyrics"), t!("settings-lyrics-detail")),
            Slot::Karaoke => (
                t!("settings-karaoke-lyrics"),
                t!("settings-karaoke-lyrics-detail"),
            ),
            Slot::Romanized => (
                t!("settings-romanized-lyrics"),
                t!("settings-romanized-lyrics-detail"),
            ),
            Slot::LyricsForLocal => (
                t!("settings-lyrics-for-local-files"),
                t!("settings-lyrics-for-local-files-detail"),
            ),
            Slot::SpotifyConnect => match () {
                () if connect::jellyfin(cx) => (
                    t!("settings-jellyfin-remote"),
                    t!("settings-jellyfin-remote-detail"),
                ),
                () if connect::youtube(cx) => (
                    t!("settings-youtube-cast"),
                    t!("settings-youtube-cast-detail"),
                ),
                () => (
                    t!("settings-spotify-connect"),
                    t!("settings-spotify-connect-detail"),
                ),
            },
            Slot::SpotifyConnectName => (
                t!("settings-spotify-connect-name"),
                t!("settings-spotify-connect-name-detail"),
            ),
            Slot::SpotifyConnectCustom => (
                t!("settings-spotify-connect-custom"),
                t!("settings-spotify-connect-custom-detail"),
            ),
            Slot::SessionsButton => (t!("settings-sessions-button"), SharedString::default()),
            Slot::SessionsOfficial => (t!("settings-sessions-official"), SharedString::default()),
            Slot::SessionsServer => (t!("common-server-address"), SharedString::default()),
            Slot::Discord => (
                t!("settings-discord"),
                [
                    "settings-discord-detail",
                    "settings-discord-name",
                    "settings-discord-name-detail",
                    "settings-discord-badge",
                    "settings-discord-badge-detail",
                    "settings-discord-buttons",
                    "settings-discord-buttons-detail",
                ]
                .map(|key| i18n::lookup(key, None).to_string())
                .join(" ")
                .into(),
            ),
            Slot::DiscordOptions => (
                t!("settings-group-status"),
                [
                    "settings-discord-show-paused",
                    "settings-discord-show-paused-detail",
                    "settings-discord-anonymous",
                    "settings-discord-anonymous-detail",
                    "settings-discord-upload-covers",
                    "settings-discord-upload-covers-detail",
                    "settings-artwork-for-local-files",
                    "settings-artwork-for-local-files-detail",
                    "settings-discord-hide-on-connect",
                    "settings-discord-hide-on-connect-detail",
                ]
                .map(|key| i18n::lookup(key, None).to_string())
                .join(" ")
                .into(),
            ),
            Slot::Scrobbles => self.scrobbles_meta(cx),
            Slot::ScrobbleProvider => self.scrobble_provider_meta(cx)?,
            Slot::Keybind(index) => self.keybind_meta(index, cx)?,
            Slot::KeybindsReset => (t!("keybinds-reset-all"), SharedString::default()),
            Slot::Version => (t!("settings-version"), t!("settings-version-detail")),
            Slot::Updates => (
                t!("settings-auto-check"),
                t!("settings-check-updates-detail"),
            ),
            Slot::AutoUpdate => (
                t!("settings-auto-update"),
                t!("settings-auto-update-detail"),
            ),
            Slot::Channel => (
                t!("settings-update-channel"),
                t!("settings-update-channel-detail"),
            ),
            Slot::Log => (t!("settings-log"), t!("settings-log-detail")),
            Slot::License => (t!("settings-license"), t!("settings-license-detail")),
            Slot::Source => (t!("settings-source"), t!("settings-source-detail")),
        };
        Some(meta)
    }

    /// Builds the slot's row or cards when its group is visible or needs measuring.
    fn slot_element(&self, slot: Slot, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match slot {
            Slot::Startup => self.startup_row(cx),
            Slot::Provider => self.provider_row(cx),
            Slot::Entries => self.entries_row(cx),
            Slot::Language => self.language_row(cx),
            Slot::Tray => self.tray_row(cx),
            Slot::TrayIcon => self.tray_icon_row(cx),
            Slot::NowPlayingOnFileOpen => self.now_playing_on_file_open_row(cx),
            Slot::TaskbarProgress => self.taskbar_progress_row(cx),
            Slot::Accounts => div().into_any_element(),
            Slot::AccountsLink => self.accounts_link_row(cx),
            Slot::LocalFolder => self.local_folder_row(cx),
            Slot::SubsonicLibrary => self.subsonic_library_row(cx),
            Slot::Folders => self.folders(cx),
            Slot::Theme | Slot::Icons | Slot::Corners => div().into_any_element(),
            Slot::Adaptive => self.adaptive_row(cx),
            Slot::CommunityThemes => self.community_themes_row(cx),
            Slot::Ambient => self.ambient_row(cx),
            Slot::Visualizer => self.visualizer_row(cx),
            Slot::PlayerBarLayout => div().into_any_element(),
            Slot::PlayerBarTransparent => self.player_bar_transparent_row(cx),
            Slot::PlayerBarTimeLeft => self.player_bar_time_left_row(cx),
            Slot::VisualizerScreens => self.visualizer_screens_row(cx),
            Slot::VisualizerAbsolute => self.visualizer_absolute_row(cx),
            Slot::Opacity => self.opacity_row(cx),
            Slot::Blur => self.blur_row(cx),
            Slot::FullscreenControlsAutohide => self.fullscreen_controls_autohide_row(cx),
            Slot::OsFullscreen => self.os_fullscreen_row(cx),
            Slot::PanelLyricsSize => self.panel_lyrics_size_row(cx),
            Slot::FullscreenLyricsSize => self.fullscreen_lyrics_size_row(cx),
            Slot::BlurLyrics => self.blur_lyrics_row(cx),
            Slot::Font => self.font_row(cx),
            Slot::Motion => self.motion_row(cx),
            Slot::Pace => self.pace_row(cx),
            Slot::Saver => self.saver_row(cx),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Slot::ServerSideDecorations => self.server_side_decorations_row(cx),
            #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
            Slot::Decorations => self.decorations_row(cx),
            #[cfg(not(target_os = "macos"))]
            Slot::Side => self.side_row(window, cx),
            #[cfg(not(target_os = "macos"))]
            Slot::TrafficLights => self.traffic_light_controls_row(window, cx),
            #[cfg(any(
                target_os = "windows",
                target_os = "linux",
                target_os = "freebsd",
                target_os = "macos"
            ))]
            Slot::WindowRounding => self.window_rounding_row(window, cx),
            Slot::AdaptiveMenu => self.adaptive_menu_row(cx),
            Slot::Normalisation => self.playback_row(cx),
            Slot::Gapless => self.gapless_row(cx),
            Slot::SponsorBlock => self.sponsorblock_row(cx),
            Slot::KeepTracks => self.keep_tracks_row(cx),
            Slot::CacheSize => self.cache_size_row(cx),
            Slot::CacheUsage => self.cache_usage_row(cx),
            Slot::DownloadsUsage => self.downloads_usage_row(cx),
            Slot::OfflineMode => self.offline_mode_row(cx),
            Slot::OpenDownloads => self.open_downloads_row(cx),
            Slot::StreamQuality => self.stream_quality_row(cx),
            Slot::StreamFormat => self.stream_format_row(cx),
            Slot::StreamBitrate => self.stream_bitrate_row(cx),
            Slot::Sleep => self.sleep_row(cx),
            Slot::StayAwake => self.stay_awake_row(cx),
            Slot::OutputDevice => self.output_device_row(cx),
            Slot::OutputFailure => self.output_failure_row(cx),
            Slot::SampleRate => self.sample_rate_row(cx),
            Slot::Buffer => self.buffer_row(cx),
            Slot::MatchRate => self.match_rate_row(cx),
            Slot::Widevine => self.widevine_row(cx),
            Slot::Soloist => self.soloist_row(cx),
            Slot::Equalizer => self.equalizer_row(cx),
            Slot::LyricsSources => self.lyrics_sources_row(cx),
            Slot::LyricsPriority => self.lyrics_priority(cx),
            Slot::WordSyncedFirst => self.word_synced_first_row(cx),
            Slot::Lyrics => self.lyrics_row(cx),
            Slot::Karaoke => self.karaoke_lyrics_row(cx),
            Slot::Romanized => self.romanized_lyrics_row(cx),
            Slot::LyricsForLocal => self.lyrics_for_local_files_row(cx),
            Slot::SpotifyConnect => self.spotify_connect_row(cx),
            Slot::SpotifyConnectName => self.spotify_connect_name_row(cx),
            Slot::SpotifyConnectCustom => self.spotify_connect_custom_row(cx),
            Slot::SessionsButton => self.sessions_button_row(cx),
            Slot::SessionsOfficial => self.sessions_official_row(cx),
            Slot::SessionsServer => self.sessions_server_row(cx),
            Slot::Discord => self.discord_card(window, cx),
            Slot::DiscordOptions => self.discord_options(cx),
            Slot::Scrobbles => div().into_any_element(),
            Slot::ScrobbleProvider => self.scrobble_provider_row(cx),
            Slot::Keybind(index) => self.keybind_row(index, cx),
            Slot::KeybindsReset => self.keybinds_reset_row(cx),
            Slot::Version => self.version_row(window, cx),
            Slot::Updates => self.updates_row(cx),
            Slot::AutoUpdate => self.auto_update_row(cx),
            Slot::Channel => self.channel_row(cx),
            Slot::Log => self.log_row(cx),
            Slot::License => self.license_row(cx),
            Slot::Source => self.source_row(cx),
        }
    }

    /// Builds the option cards a group shows above its rows. A slot with no cards of its own
    /// draws its row instead.
    fn cards(&self, slot: Slot, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match slot {
            Slot::Accounts => self.accounts_cards(window, cx),
            Slot::Scrobbles => self.scrobble_cards(window, cx),
            Slot::Theme => self.theme_cards(cx),
            Slot::Icons => self.icon_cards(window, cx),
            Slot::Corners => self.corner_cards(window, cx),
            Slot::PlayerBarLayout => self.player_bar_cards(window, cx),
            Slot::StreamQuality => self.quality_cards(window, cx),
            slot => self.slot_element(slot, window, cx),
        }
    }

    /// One group: its name beside it, its option cards, then its rows, all on the page itself
    /// with no surface around them and nothing between the rows. A group with no name leaves
    /// the name's column empty while `indent`, so its rows line up with the named groups of
    /// the page.
    fn group(
        &self,
        group: Group,
        indent: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Group { title, cards, rows } = group;
        let heading = title.map(|key| {
            group_label(
                SharedString::from(format!("settings-group-{key}")),
                i18n::lookup(key, None),
                cx,
            )
            .into_any_element()
        });
        let carded = cards.is_some();
        let cards = cards.map(|slot| {
            let cards = self.cards(slot, window, cx);
            let cards = self.marking(slot, cards, window, cx);
            self.revealing(Reveal::Slot(slot), cards, cx)
        });
        let rows = rows
            .into_iter()
            .map(|slot| {
                let row = self.slot_element(slot, window, cx);
                let row = self.marking(slot, row, window, cx);
                self.revealing(Reveal::Slot(slot), row, cx)
            })
            .collect::<Vec<_>>();
        let rows = (!rows.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .when(carded, |rows| rows.mt_3())
                .children(rows)
        });

        let body = div().flex().flex_col().children(cards).children(rows);
        beside(heading, indent, !carded, body, window, cx)
    }

    /// Lights `element` while it is the setting a search result just opened, in the fill the
    /// result had while picked in the panel, and lets the light fade once it has held. The light
    /// reaches a little past the column on both sides, as a drill row's hover does.
    fn marking(&self, slot: Slot, element: AnyElement, window: &Window, cx: &App) -> AnyElement {
        let level = self.mark_level(slot, window, cx);
        if level <= 0. {
            return element;
        }
        let theme = cx.theme();

        div()
            .relative()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .mx_neg_3()
                    .rounded(theme.radius)
                    .bg(theme.table_active.opacity(level)),
            )
            .child(element)
            .into_any_element()
    }

    /// How lit `slot` stands: 1 while a search result has just opened it, easing to 0 once the
    /// light has held, and 0 for any other setting. It asks for frames until the light is out.
    /// Under reduced motion the light goes out at once rather than fading.
    fn mark_level(&self, slot: Slot, window: &Window, cx: &App) -> f32 {
        let Some((marked, since)) = self.marked else {
            return 0.;
        };
        if marked != slot {
            return 0.;
        }
        let level = match since.elapsed().checked_sub(MARK_HOLD) {
            None => 1.,
            Some(_) if cx.reduce_motion() => 0.,
            Some(fading) => {
                let span = paced(MARK_FADE).as_secs_f32().max(f32::EPSILON);
                1. - ease_out_quad((fading.as_secs_f32() / span).min(1.))
            }
        };
        if level > 0. {
            window.request_animation_frame();
        }
        level
    }

    /// Wraps `element` so that, while it is what the tab waits to reveal, the tab keeps it just
    /// below the header, clear of the fade. The tab holds it there through content above it arriving late, until
    /// the user scrolls or leaves.
    fn revealing(&self, reveal: Reveal, element: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        if self.reveal != Some(reveal) {
            return element;
        }
        let bar = self.scrollbar.clone();
        let view = cx.entity().downgrade();

        div()
            .relative()
            .child(element)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let lead = Layout::of(window, cx).clear();
                        let scroll = bar.read(cx).scroll().clone();
                        let at = scroll.offset().y;
                        let top = bounds.top() - scroll.bounds().top() - at;
                        let to = (-(top - lead).max(Pixels::ZERO)).max(-scroll.max_offset().y);
                        if (to - at).abs() < px(0.5) {
                            return;
                        }
                        window.on_next_frame(move |_, cx| {
                            bar.update(cx, |bar, _| bar.place(to));
                            view.update(cx, |_, cx| cx.notify()).ok();
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    /// The tab on show, or the drill-in on its own: each of its pages in turn. The frame draws
    /// the first page's title, and every later page opens on its own title, set well apart from
    /// the page above it.
    fn tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let pages = match self.page.parent() {
            Some(_) => vec![self.page],
            None => self.page.tab().sections().to_vec(),
        };
        let parts = pages
            .iter()
            .map(|page| self.part(*page, cx))
            .collect::<Vec<_>>();
        // a section's title stands as close over its groups as the page's own title does, and
        // only as far below the section above as the title's own size sets it apart
        let inset = cx.theme().metrics.inset;
        let pages = pages
            .into_iter()
            .zip(parts)
            .enumerate()
            .map(|(index, (page, part))| {
                let title = (index > 0).then(|| {
                    let control = self.title_control(page, cx);
                    let title = section_title(page, control, cx).into_any_element();
                    div()
                        .mb(inset)
                        .child(self.revealing(Reveal::Section(page), title, cx))
                });

                // the Now playing preview stands beside its settings where the column has room,
                // and every other preview over them
                let groups = div().flex().flex_col().child(Part::shown(&part, cx));
                let preview = self.preview(page, cx);
                let beside = (page == SettingsPage::NowPlaying)
                    .then(|| preview::now_playing::beside(window, cx))
                    .flatten();
                let (above, body) = match (preview, beside) {
                    (Some(preview), Some(width)) => (
                        None,
                        div()
                            .flex()
                            .items_start()
                            .gap(inset)
                            .child(div().flex_none().w(width).child(preview))
                            .child(groups.flex_1().min_w_0())
                            .into_any_element(),
                    ),
                    (preview, _) => (preview, groups.into_any_element()),
                };

                div().flex().flex_col().children(title).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(GROUPS)
                        .children(self.header(page, cx))
                        .children(above)
                        .child(body),
                )
            })
            .collect::<Vec<_>>();

        div().flex().flex_col().gap(GROUPS).children(pages)
    }

    /// The view that draws `page`'s groups, made the first time the page shows.
    fn part(&mut self, page: SettingsPage, cx: &mut Context<Self>) -> Entity<Part<Self>> {
        if let Some((_, part)) = self.parts.iter().find(|(shown, _)| *shown == page) {
            return part.clone();
        }
        let view = cx.entity();
        let part = cx.new(|cx| {
            Part::new(
                &view,
                move |this: &mut Self, window, cx| this.sections(page, window, cx),
                cx,
            )
        });
        self.parts.push((page, part.clone()));
        part
    }

    /// The groups of `page` and what it shows under them, each set apart from the one above by a
    /// line, as the page's part draws them.
    fn sections(&self, page: SettingsPage, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let groups = self.groups(page, cx);
        let indent = groups.iter().any(|group| group.title.is_some());
        let mut blocks = groups
            .into_iter()
            .enumerate()
            .map(|(index, group)| {
                // A search target must lay out even off screen so its reveal marker
                // can position the scroll. The other groups keep their cached heights.
                match self
                    .reveal
                    .is_some_and(|reveal| group.slots().any(|slot| reveal == Reveal::Slot(slot)))
                {
                    true => self.group(group, indent, window, cx),
                    false => self
                        .lazy((page.id(), index + 3), cx, move |this, window, cx| {
                            this.group(group, indent, window, cx)
                        })
                        .into_any_element(),
                }
            })
            .collect::<Vec<_>>();
        blocks.extend(self.footer(page, window, cx));
        let blocks = blocks
            .into_iter()
            .enumerate()
            .map(|(index, shown)| {
                div()
                    .when(index > 0, |this| {
                        this.mt(theme.metrics.pad)
                            .pt(theme.metrics.pad)
                            .border_t_1()
                            .border_color(theme.border)
                    })
                    .child(shown)
            })
            .collect::<Vec<_>>();

        div().flex().flex_col().children(blocks).into_any_element()
    }

    /// What a search shows instead of the page: every setting that answers it, each naming the
    /// page it lives on and opening it there.
    pub(crate) fn results(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let hits = self.found(&self.query, cx);
        if hits.is_empty() {
            let empty = t!("search-no-matches");
            return div().child(
                div()
                    .id("settings-results-empty")
                    .role(Role::Label)
                    .aria_value(empty.clone())
                    .child(Vacancy::new(empty).icon("icons/search.svg").py_6()),
            );
        }
        let row = snapped(theme.metrics.list_row, window);
        let count = hits.len();

        // the cards pad their art in from their own edge, so the list reaches out by as much to
        // put the art in line with the column
        div().child(
            div().mx(-theme.metrics.pad).child(
                Deck::new("settings-results")
                    .rows(hits.iter().map(|_| row))
                    .draw(cx.processor(move |this: &mut Self, index: usize, _, cx| {
                        match hits.get(index) {
                            Some((page, slot)) => this
                                .result((index, count), *page, *slot, cx)
                                .into_any_element(),
                            None => div().into_any_element(),
                        }
                    })),
            ),
        )
    }

    /// The panel under the search field: the settings that best answer what is typed, or a line
    /// saying none do.
    fn panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let offers = self.offers(cx);
        let heading = t!("settings-search-results");
        let none = offers
            .is_empty()
            .then(|| Vacancy::new(t!("search-no-matches")).spoken("settings-search-none"));

        // New rows surface one after another and the panel stretches to fit them, the way the
        // suggestions under the search page's field fill.
        self.cascade.show(
            offers
                .iter()
                .map(|(page, slot)| SharedString::from(result_key(*page, *slot))),
        );
        let count = offers.len();
        let rows = offers
            .iter()
            .enumerate()
            .map(|(at, (page, slot))| {
                let level = self.cascade.level(&result_key(*page, *slot), window, cx);
                let chosen = self.pick == Some(at);
                let row = self
                    .result((at, count), *page, *slot, cx)
                    .tucked()
                    .chosen(chosen)
                    // The keyboard stays in the field, so the chosen card claims the active
                    // descendant while every card reports its seat in the list.
                    .when(chosen, |card| card.aria_active_descendant());
                Cascade::surface(row, level)
            })
            .collect::<Vec<_>>();
        let list = div()
            .id("settings-search-offers")
            .role(Role::ListBox)
            .aria_label(heading)
            .flex()
            .flex_col()
            .gap_px()
            .children(none)
            .children(rows);
        self.unfold.wrap(list, false, window, cx).into_any_element()
    }

    /// One search result, placed by its rank and the number of results: the setting's name over
    /// the page it lives on.
    fn result(
        &self,
        (index, count): (usize, usize),
        page: SettingsPage,
        slot: Slot,
        cx: &mut Context<Self>,
    ) -> Card {
        let title = self
            .slot_meta(slot, cx)
            .map(|(title, _)| title)
            .unwrap_or_default();
        let tab = page.tab();
        let place = match page.parent() {
            Some(_) => t!(
                "settings-result-place",
                parent = i18n::lookup(tab.key(), None).to_string(),
                page = i18n::lookup(page.key(), None).to_string()
            ),
            None => i18n::lookup(tab.key(), None),
        };

        Card::new(SharedString::from(result_key(page, slot)), title)
            .aria_description(place.clone())
            .aria_position_in_set(index + 1)
            .aria_size_of_set(count)
            .fallback(tab.icon())
            .meta(place)
            .press(cx.listener(move |this, _, window, cx| this.open_result(page, slot, window, cx)))
    }

    /// Opens the page a result lives on and scrolls its setting into view. The search field lets
    /// go of the keyboard, so the keys reach the page the result opened.
    fn open_result(
        &mut self,
        page: SettingsPage,
        slot: Slot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reveal = Some(Reveal::Slot(slot));
        self.marked = Some((slot, Instant::now()));
        self.close_panel(cx);
        if self.typing(window, cx) {
            window.focus(&self.focus, cx);
        }
        self.clear_search(cx);
        navigate(Destination::Settings(page), cx);
        cx.notify();
    }

    /// Starts reading the installed fonts the first time the typeface picker could need them.
    fn load_fonts(&mut self, cx: &mut Context<Self>) {
        let picking = self.popovers.shows(TYPEFACES);
        let wanted = self.page.tab() == SettingsTab::Appearance || picking;
        if !wanted || self.installed.is_some() || self.loading_fonts {
            return;
        }
        self.loading_fonts = true;
        let text_system = cx.text_system().clone();
        let io = Io::global(cx);
        self.font_task = Some(cx.spawn(async move |this, cx| {
            let names = io
                .spawn_blocking(move || appearance::usable_fonts(text_system))
                .await
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                this.installed = Some(names);
                this.loading_fonts = false;
                this.font_task = None;
                // the picker may already be open on an empty list: put the
                // cursor on the chosen face now that it can be found
                if this.popovers.shows(TYPEFACES) {
                    let chosen = this.settings.read(cx).font();
                    let place = this
                        .typeface_entries()
                        .iter()
                        .position(|name| name.as_ref() == chosen);
                    this.typefaces.place(place, cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let geometry = (Layout::of(window, cx).width, theme.font_size, theme.radius);
        if self.geometry.replace(geometry) != Some(geometry) {
            self.layout_revision = self.layout_revision.wrapping_add(1);
            for (_, part) in &self.parts {
                part.update(cx, |part, _| part.unsettle());
            }
        }
        if !cx.has_active_drag() {
            self.lyrics_drop_gap = None;
        }
        self.load_fonts(cx);

        let picking_languages = self.popovers.shows(LANGUAGES);
        if picking_languages {
            let chosen_language = self.settings.read(cx).language();
            let language_selected = Language::ALL
                .into_iter()
                .position(|language| language.id() == chosen_language)
                .map(|place| place + 1)
                .or(Some(0));
            self.languages.sync(true, language_selected, window, cx);
        } else {
            self.languages.sync(false, None, window, cx);
        }

        if self.popovers.shows(TYPEFACES) {
            let chosen_typeface = self.settings.read(cx).font();
            let typeface_selected = self
                .typeface_entries()
                .iter()
                .position(|name| name.as_ref() == chosen_typeface);
            self.typefaces.sync(true, typeface_selected, window, cx);
        } else {
            self.typefaces.sync(false, None, window, cx);
        }

        let accounts = match self.session.read(cx).state() {
            SessionState::Authorizing(Some(SignInPrompt::Accounts(accounts))) => {
                Some(accounts.clone())
            }
            _ => None,
        };
        let code = match self.session.read(cx).state() {
            SessionState::Authorizing(Some(SignInPrompt::Code { code, url })) => {
                Some((code.clone(), url.clone()))
            }
            _ => None,
        };
        if self.sign_in_running && !self.session.read(cx).is_pending() {
            self.sign_in_running = false;
            self.sign_in_for = None;
        }
        let manual_secret = self.manual_secret.filter(|_| {
            matches!(
                self.session.read(cx).state(),
                SessionState::Authorizing(Some(SignInPrompt::Secret))
            )
        });

        // only one of these is ever up: a prompt the sign-in raised hides the choice behind
        // it, and the choice holds the veil until that prompt arrives
        let taken = accounts.is_some()
            || code.is_some()
            || manual_secret.is_some()
            || self.credentials_for.is_some();
        let sign_in_for = self.sign_in_for.filter(|_| !taken);

        // a dialog takes the key focus, since escape only reaches the page from inside it
        let theme_delete = self.theme_delete_modal(cx);
        let dialog = taken
            || sign_in_for.is_some()
            || self.scrobble_prompt.is_some()
            || theme_delete.is_some()
            || self.library_picker;
        match (dialog, self.grabbed) {
            (true, false) => {
                window.focus(&self.focus, cx);
                self.grabbed = true;
            }
            (false, true) => self.grabbed = false,
            _ => {}
        }

        let body = match self.searching() {
            true => self.results(window, cx),
            false => self.tab(window, cx),
        };
        let destination = Destination::Settings(self.page);
        let heading = match self.searching() {
            true => Heading {
                named: Some(t!("settings-search-results")),
                trailing: None,
            },
            false => Heading {
                named: None,
                trailing: match self.page.parent() {
                    Some(_) => None,
                    None => self.title_control(self.page.tab().lead(), cx),
                },
            },
        };

        layout::frame(
            &destination,
            heading,
            "settings",
            &self.scrollbar,
            body,
            window,
            cx,
        )
        .track_focus(&self.focus)
        // the user taking the page in hand lets go of whatever it was holding in view
        .on_scroll(cx.listener(|this, _, _, _| this.reveal = None))
        .capture_any_mouse_down(cx.listener(|this, _, _, _| this.reveal = None))
        // an action stops where it is handled unless told to go on, so an escape with nothing
        // to close here goes on to the workspace
        .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
            if !this.escape(cx) {
                cx.propagate();
            }
        }))
        // the library picker's keyboard: arrows walk the cursor, enter chooses; a closed
        // picker passes every one of these on untouched
        .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.library_walk(1, cx)))
        .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.library_walk(-1, cx)))
        .on_action(cx.listener(|this, _: &Activate, _, cx| this.library_activate(cx)))
        .when_some(accounts, |this, accounts| {
            this.child(self.account_modal(accounts, cx).into_any_element())
        })
        .when_some(code, |this, (code, url)| {
            this.child(self.code_prompt(code, url, cx).into_any_element())
        })
        .when_some(manual_secret, |this, (slug, provider)| {
            this.child(self.secret_prompt(slug, provider, cx).into_any_element())
        })
        .when(self.credentials_for.is_some(), |this| {
            this.child(self.credentials_prompt(cx).into_any_element())
        })
        .when_some(self.scrobble_prompt, |this, service| {
            this.child(self.scrobble_modal(service, cx).into_any_element())
        })
        .when_some(sign_in_for, |this, (slug, provider)| {
            this.child(self.sign_in_prompt(slug, provider, cx).into_any_element())
        })
        .when_some(theme_delete, |this, modal| this.child(modal))
        .when(self.library_picker, |this| {
            this.child(self.library_modal(cx).into_any_element())
        })
    }
}

/// What tells one search result from another, as its element id and across the panel's frames.
/// Two settings of a page may share a title, so the key names the setting itself.
fn result_key(page: SettingsPage, slot: Slot) -> String {
    format!("settings-result-{}-{slot:?}", page.id())
}

/// A group's heading: its icon, when it has one, before its name as an uppercase eyebrow.
pub(super) fn group_heading(label: SharedString, icon: Option<&'static str>, cx: &App) -> Div {
    ui::eyebrow_with(icon, label, cx).mb(cx.theme().metrics.pad)
}

/// A group's name as it stands in the column beside the group, read as a level two heading.
pub(super) fn group_label(id: SharedString, label: SharedString, cx: &App) -> Stateful<Div> {
    ui::eyebrow_with(None, label.clone(), cx)
        .flex_1()
        .min_w_0()
        .id(id)
        .role(Role::Heading)
        .aria_level(2)
        .aria_label(label)
}

/// Lays `body` out beside its `label` where the column has the room, and under it where it does
/// not. With `centred` the label stands level with the body's first row, otherwise at its top,
/// for a body that opens with cards. With no label the body takes the whole width, or keeps the
/// label's column empty while `indent`.
pub(super) fn beside(
    label: Option<AnyElement>,
    indent: bool,
    centred: bool,
    body: impl IntoElement,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let side = Layout::of(window, cx).room().fits(Room::Roomy);
    if !side {
        return div()
            .flex()
            .flex_col()
            .children(label.map(|label| div().mb(theme.metrics.pad).child(label)))
            .child(body)
            .into_any_element();
    }
    if label.is_none() && !indent {
        return body.into_any_element();
    }

    div()
        .flex()
        .items_start()
        .gap(theme.metrics.pad * 2.)
        .child(
            div()
                .flex_none()
                .w(theme.metrics.list_row * LABEL_COLUMN)
                .flex()
                .map(|column| match centred {
                    true => column.min_h(theme.metrics.list_row).items_center(),
                    false => column.pt(theme.metrics.pad),
                })
                .children(label),
        )
        .child(div().flex_1().min_w_0().child(body))
        .into_any_element()
}
