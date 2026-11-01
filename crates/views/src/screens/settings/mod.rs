use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FocusHandle, Pixels, Render, ScrollHandle,
    SharedString, Task, Window, canvas, div, px, svg,
};
use i18n::{Language, t};
use music::SignInPrompt;
use router::{Destination, SettingsPage, SettingsTab, navigate};
use state::{
    AppSettings, Drm, Equalizer, Io, Outputs, Playback, Scan, Scrobbling, Session, SessionState,
    Sleep, Sonora,
};
use ui::{
    ActiveTheme as _, Card, Deck, Dismiss, Input, Lazy, Popovers, Scrollbar, ScrubberState,
    Vacancy, faint, snapped, upper,
};

use crate::chrome::{Chrome, OutputPicker};
use crate::shared::popups::SearchPopup;
use crate::shared::text;

mod about;
mod accounts;
mod appearance;
mod connect;
mod discord;
mod general;
mod header;
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
pub(crate) use layout::{Layout, frame};
pub(crate) use rows::Row;

/// The space between two groups of a page.
pub(super) const GROUPS: Pixels = px(40.);
/// How much a query found in a setting's title counts over one found in its detail.
const TITLE_WEIGHT: u32 = 2;
/// The picker keys the page's render watches, shared with the rows that open them.
const LANGUAGES: &str = "languages";
const TYPEFACES: &str = "typefaces";
/// How often the storage section measures the cache and the downloads again while its tab shows.
const STORAGE_POLL: Duration = Duration::from_secs(2);

/// One setting, named without building anything. Pages list their settings as slots in groups,
/// a search scores slots by their title and detail, and only visible groups build their rows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Accounts,
    LocalFolder,
    Folders,
    Theme,
    Adaptive,
    CommunityThemes,
    Icons,
    Corners,
    Typeface,
    Font,
    Opacity,
    WindowBlur,
    Blur,
    Ambient,
    AmbientMotion,
    Visualizer,
    VisualizerAbsolute,
    PlayerBarLayout,
    PlayerBarTransparent,
    FullscreenControlsAutohide,
    OsFullscreen,
    Karaoke,
    BlurLyrics,
    Romanized,
    PanelLyricsSize,
    FullscreenLyricsSize,
    LyricsSources,
    LyricsPriority,
    LyricsForLocal,
    Normalisation,
    Gapless,
    KeepTracks,
    CacheSize,
    CacheUsage,
    DownloadsUsage,
    OfflineMode,
    Sleep,
    StayAwake,
    StreamQuality,
    StreamFormat,
    StreamBitrate,
    Widevine,
    OutputDevice,
    OutputFailure,
    SampleRate,
    Buffer,
    Equalizer,
    SpotifyConnect,
    SpotifyConnectName,
    SpotifyConnectCustom,
    SessionsButton,
    SessionsOfficial,
    SessionsServer,
    Discord,
    DiscordName,
    DiscordShowPaused,
    DiscordBadge,
    DiscordAnonymous,
    DiscordHideOnConnect,
    DiscordButtons,
    ArtworkForLocal,
    Scrobble(usize),
    Startup,
    Entries,
    Language,
    Tray,
    TrayIcon,
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

/// One group of a page: an uppercase eyebrow with its icon, an optional block of option cards
/// under it, and the rows below.
struct Group {
    title: Option<&'static str>,
    icon: Option<&'static str>,
    cards: Option<Slot>,
    rows: Vec<Slot>,
}

impl Group {
    /// A group under the heading the Fluent key `title` names, with `icon` before it.
    fn new(title: &'static str, icon: &'static str) -> Self {
        Self {
            title: Some(title),
            icon: Some(icon),
            cards: None,
            rows: Vec::new(),
        }
    }

    /// A group with no heading, for a page whose first rows need none.
    fn untitled() -> Self {
        Self {
            title: None,
            icon: None,
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
    equalizer: Entity<Equalizer>,
    settings: Entity<AppSettings>,
    outputs: Entity<Outputs>,
    output_picker: Entity<OutputPicker>,
    /// The page the route names: a page of the tab on show, or a drill-in.
    page: SettingsPage,
    search: Entity<Input>,
    /// The search field's text, trimmed. Empty means the page shows its own settings.
    query: String,
    /// What the tab scrolls into view once it is laid out: the setting a search result opened,
    /// or the section a route named.
    reveal: Option<Reveal>,
    /// Where a lyrics source dragged over the priority list would land, as a gap between the
    /// enabled sources. `None` while the drop would leave the order as it is.
    lyrics_drop_gap: Option<usize>,
    scrollbar: Entity<Scrollbar>,
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
    server: Entity<Input>,
    username: Entity<Input>,
    password: Entity<Input>,
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
    languages: SearchPopup,
    typefaces: SearchPopup,
    typeface_faced: RefCell<HashSet<SharedString>>,
    installed: Option<Vec<SharedString>>,
    loading_fonts: bool,
    font_task: Option<Task<()>>,
    /// What the pages' previews keep between frames.
    previews: preview::Previews,
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
        cx.observe(&drm, |this, _, cx| this.remeasure(cx)).detach();
        cx.observe(&equalizer, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&outputs, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&session, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&scrobbling, |this, _, cx| this.remeasure(cx))
            .detach();
        cx.observe(&settings, |this, _, cx| this.remeasure(cx))
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
                .shortcut("secondary-f")
                .clearable()
        });
        cx.observe(&search, |this, input, cx| {
            let query = input.read(cx).text().trim().to_owned();
            // A search swaps the page for hits from every page, so wherever the page was
            // scrolled to answers for nothing once one starts.
            if this.query.is_empty() != query.is_empty() {
                this.scrollbar.update(cx, |bar, _| bar.place(Pixels::ZERO));
            }
            if !query.is_empty() {
                this.reveal = None;
            }
            this.query = query;
            cx.notify();
        })
        .detach();

        let connect_name =
            cx.new(|cx| Input::new("settings-spotify-connect-custom-hint", cx).compact());
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

        Self {
            session,
            _storage: storage,
            playback,
            drm,
            equalizer,
            settings,
            outputs,
            output_picker,
            page: SettingsPage::General,
            search,
            query: String::new(),
            reveal: None,
            lyrics_drop_gap: None,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
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
            server: cx.new(|cx| Input::new("login-server-hint", cx)),
            username: cx.new(|cx| Input::new("login-username-hint", cx)),
            password: cx.new(|cx| Input::new("login-password-hint", cx).masked()),
            credentials_for: None,
            secret: cx.new(|cx| Input::new("login-cookie-hint", cx)),
            manual_secret: None,
            sign_in_for: None,
            focus,
            grabbed: false,
            sign_in_running: false,
            chosen: None,
            scrobbling,
            scrobble_first: cx.new(|cx| Input::new("settings-scrobble-key", cx)),
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
        }
    }

    /// Shows the tab a page belongs to and ends any search, so a route always lands on a plain
    /// tab. A new route starts at the top of its tab, or at its section when that lies further
    /// down, and a route to the page already on show keeps the tab where it was scrolled.
    pub(crate) fn select(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        if self.page != page {
            self.scrollbar.update(cx, |bar, _| bar.place(Pixels::ZERO));
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
                _ => (page.parent().is_none() && page != page.tab().lead())
                    .then_some(Reveal::Section(page)),
            };
        }
        self.page = page;
        self.popovers.close();
        self.previews.shown(page.tab());
        // a device plugged in since the list was read is only found by asking again
        if page.tab() == SettingsTab::Playback {
            self.outputs.update(cx, |outputs, cx| outputs.refresh(cx));
        }
        self.clear_search(cx);
        cx.notify();
    }

    /// Tells the previews whether settings are open, so a new opening picks a new track and a
    /// closing stops what they keep running.
    pub(crate) fn set_open(&mut self, open: bool) {
        match open {
            true => self.previews.opened(),
            false => self.previews.closed(),
        }
    }

    pub(crate) fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// The search field, which the header draws.
    pub(crate) fn search(&self) -> Entity<Input> {
        self.search.clone()
    }

    pub(crate) fn clear_search(&mut self, cx: &mut Context<Self>) {
        if !self.search.read(cx).text().is_empty() {
            self.search.update(cx, |input, cx| input.set_text("", cx));
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
        if self.searching() {
            self.clear_search(cx);
            return true;
        }
        false
    }

    /// The groups a page shows, in order.
    fn groups(&self, page: SettingsPage, cx: &App) -> Vec<Group> {
        match page {
            SettingsPage::Providers => self.accounts_groups(),
            SettingsPage::Appearance => self.appearance_groups(),
            SettingsPage::PlayerBar => self.player_bar_groups(),
            SettingsPage::NowPlaying => self.now_playing_groups(cx),
            SettingsPage::Lyrics => self.lyrics_groups(),
            SettingsPage::Playback => self.playback_groups(cx),
            SettingsPage::Sound => self.sound_groups(cx),
            SettingsPage::Storage => self.storage_groups(cx),
            SettingsPage::Discord => self.discord_groups(cx),
            SettingsPage::Scrobbling => self.scrobbling_groups(cx),
            SettingsPage::General => self.general_groups(cx),
            SettingsPage::About => self.about_groups(),
            SettingsPage::LyricsSources => self.lyrics_sources_groups(),
            SettingsPage::MusicFolders => self.music_folders_groups(),
        }
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
            SettingsPage::Providers | SettingsPage::About => Some(
                self.lazy((page.id(), 0_usize), cx, move |this, _, cx| match page {
                    SettingsPage::Providers => this.profile(cx).into_any_element(),
                    _ => this.mark(cx).into_any_element(),
                })
                .into_any_element(),
            ),
            _ => None,
        }
    }

    /// What a page shows under its groups.
    fn footer(&self, page: SettingsPage, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match page {
            SettingsPage::About => vec![
                self.lazy((page.id(), 2_usize), cx, |this, _, cx| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(GROUPS)
                        .child(this.team(cx))
                        .child(this.notice(cx))
                        .into_any_element()
                })
                .into_any_element(),
            ],
            _ => Vec::new(),
        }
    }

    /// The live preview above a page's first group, built and animated only while visible.
    fn preview(&self, page: SettingsPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !matches!(
            page,
            SettingsPage::NowPlaying
                | SettingsPage::Lyrics
                | SettingsPage::Sound
                | SettingsPage::Discord
        ) {
            return None;
        }
        let view = cx.entity();
        Some(
            self.lazy((page.id(), 1_usize), cx, move |this, window, cx| {
                let previews = &this.previews;
                match page {
                    SettingsPage::NowPlaying => preview::now_playing::preview(this, window, cx),
                    SettingsPage::Lyrics => {
                        preview::lyrics::preview(&previews.lyrics, this, window, cx)
                    }
                    SettingsPage::Sound => {
                        preview::sound::preview(&previews.sound, this, window, cx)
                    }
                    SettingsPage::Discord => {
                        preview::discord::preview(&previews.discord, this, window, cx)
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
    fn found(&self, cx: &App) -> Vec<(SettingsPage, Slot)> {
        let mut hits: Vec<(u32, SettingsPage, Slot)> = SettingsPage::ALL
            .into_iter()
            .flat_map(|page| {
                self.groups(page, cx)
                    .into_iter()
                    .flat_map(|group| group.slots().collect::<Vec<_>>())
                    .map(move |slot| (page, slot))
            })
            .filter_map(|(page, slot)| {
                self.slot_score(slot, &self.query, cx)
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
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per row keeps the keys beside the rows"
    )]
    fn slot_meta(&self, slot: Slot, cx: &App) -> Option<(SharedString, SharedString)> {
        let meta = match slot {
            Slot::Folders => return None,
            Slot::Startup => (t!("settings-startup"), t!("settings-startup-detail")),
            Slot::Entries => (t!("settings-entries"), t!("settings-entries-detail")),
            Slot::Language => (t!("settings-language"), t!("settings-language-detail")),
            Slot::Tray => (
                t!("settings-close-to-tray"),
                t!("settings-close-to-tray-detail"),
            ),
            Slot::TrayIcon => (t!("settings-tray-icon"), t!("settings-tray-icon-detail")),
            Slot::Accounts => {
                let detail = t!("settings-accounts-detail");
                let names = self.account_words(cx);
                (t!("settings-accounts"), format!("{detail} {names}").into())
            }
            Slot::LocalFolder => (
                t!("settings-local-folder"),
                match self.session.read(cx).local_paths().is_empty() {
                    true => t!("settings-local-folder-empty"),
                    false => SharedString::default(),
                },
            ),
            Slot::Theme => (t!("settings-theme"), t!("settings-theme-detail")),
            Slot::Adaptive => (t!("settings-adaptive"), t!("settings-adaptive-detail")),
            Slot::CommunityThemes => (
                t!("settings-theme-explore"),
                t!("settings-theme-explore-detail"),
            ),
            Slot::Ambient => (t!("settings-ambient"), t!("settings-ambient-detail")),
            Slot::AmbientMotion => (
                t!("settings-ambient-motion"),
                t!("settings-ambient-motion-detail"),
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
            Slot::VisualizerAbsolute => (
                t!("settings-visualizer-absolute"),
                t!("settings-visualizer-absolute-detail"),
            ),
            Slot::Icons => (t!("settings-icons"), t!("settings-icons-detail")),
            Slot::Opacity => (t!("settings-opacity"), t!("settings-opacity-detail")),
            Slot::WindowBlur => (
                t!("settings-blur-window"),
                t!("settings-blur-window-detail"),
            ),
            Slot::Blur => (t!("settings-blur"), t!("settings-blur-detail")),
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
            Slot::Font => (t!("settings-font"), t!("settings-font-detail")),
            Slot::Typeface => (t!("settings-typeface"), t!("settings-typeface-detail")),
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
            Slot::KeepTracks => (
                t!("settings-keep-tracks"),
                t!("settings-keep-tracks-detail"),
            ),
            Slot::CacheSize => (t!("settings-cache-size"), t!("settings-cache-size-detail")),
            Slot::CacheUsage => (t!("settings-cache-used"), t!("settings-cache-used-detail")),
            Slot::DownloadsUsage => (
                t!("settings-downloads-used"),
                t!("settings-downloads-used-detail"),
            ),
            Slot::OfflineMode => (
                t!("settings-offline-mode"),
                t!("settings-offline-mode-detail"),
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
            Slot::SpotifyConnect => (
                t!("settings-spotify-connect"),
                t!("settings-spotify-connect-detail"),
            ),
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
            Slot::SessionsServer => (t!("settings-sessions-server"), SharedString::default()),
            Slot::Discord => (t!("settings-discord"), t!("settings-discord-detail")),
            Slot::DiscordName => (
                t!("settings-discord-name"),
                t!("settings-discord-name-detail"),
            ),
            Slot::DiscordShowPaused => (
                t!("settings-discord-show-paused"),
                t!("settings-discord-show-paused-detail"),
            ),
            Slot::DiscordBadge => (
                t!("settings-discord-badge"),
                t!("settings-discord-badge-detail"),
            ),
            Slot::ArtworkForLocal => (
                t!("settings-artwork-for-local-files"),
                t!("settings-artwork-for-local-files-detail"),
            ),
            Slot::DiscordAnonymous => (
                t!("settings-discord-anonymous"),
                t!("settings-discord-anonymous-detail"),
            ),
            Slot::DiscordHideOnConnect => (
                t!("settings-discord-hide-on-connect"),
                t!("settings-discord-hide-on-connect-detail"),
            ),
            Slot::DiscordButtons => (
                t!("settings-discord-buttons"),
                t!("settings-discord-buttons-detail"),
            ),
            Slot::Scrobble(index) => self.scrobble_meta(index, cx)?,
            Slot::Version => (t!("settings-version"), t!("settings-version-detail")),
            Slot::Updates => (
                t!("settings-check-updates"),
                t!("settings-check-updates-detail"),
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
            Slot::Entries => self.entries_row(cx),
            Slot::Language => self.language_row(cx),
            Slot::Tray => self.tray_row(cx),
            Slot::TrayIcon => self.tray_icon_row(cx),
            Slot::Accounts => div().into_any_element(),
            Slot::LocalFolder => self.local_folder_row(cx),
            Slot::Folders => self.folders(cx),
            Slot::Theme | Slot::Icons | Slot::Corners => div().into_any_element(),
            Slot::Adaptive => self.adaptive_row(cx),
            Slot::CommunityThemes => self.community_themes_row(cx),
            Slot::Ambient => self.ambient_row(cx),
            Slot::AmbientMotion => self.ambient_motion_row(cx),
            Slot::Visualizer => div().into_any_element(),
            Slot::PlayerBarLayout => div().into_any_element(),
            Slot::PlayerBarTransparent => self.player_bar_transparent_row(cx),
            Slot::VisualizerAbsolute => self.visualizer_absolute_row(cx),
            Slot::Opacity => self.opacity_row(cx),
            Slot::WindowBlur => self.blur_window_row(cx),
            Slot::Blur => self.blur_row(cx),
            Slot::FullscreenControlsAutohide => self.fullscreen_controls_autohide_row(cx),
            Slot::OsFullscreen => self.os_fullscreen_row(cx),
            Slot::PanelLyricsSize => self.panel_lyrics_size_row(cx),
            Slot::FullscreenLyricsSize => self.fullscreen_lyrics_size_row(cx),
            Slot::BlurLyrics => self.blur_lyrics_row(cx),
            Slot::Font => self.font_row(cx),
            Slot::Typeface => self.typeface_row(cx),
            Slot::Motion => self.motion_row(cx),
            Slot::Pace => self.pace_row(cx),
            Slot::Saver => self.saver_row(cx),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Slot::ServerSideDecorations => self.server_side_decorations_row(cx),
            #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
            Slot::Decorations => self.decorations_row(cx),
            #[cfg(not(target_os = "macos"))]
            Slot::Side => self.side_row(cx),
            #[cfg(not(target_os = "macos"))]
            Slot::TrafficLights => self.traffic_light_controls_row(cx),
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
            Slot::KeepTracks => self.keep_tracks_row(cx),
            Slot::CacheSize => self.cache_size_row(cx),
            Slot::CacheUsage => self.cache_usage_row(cx),
            Slot::DownloadsUsage => self.downloads_usage_row(cx),
            Slot::OfflineMode => self.offline_mode_row(cx),
            Slot::StreamQuality => self.stream_quality_row(cx),
            Slot::StreamFormat => self.stream_format_row(cx),
            Slot::StreamBitrate => self.stream_bitrate_row(cx),
            Slot::Sleep => self.sleep_row(cx),
            Slot::StayAwake => self.stay_awake_row(cx),
            Slot::OutputDevice => self.output_device_row(),
            Slot::OutputFailure => self.output_failure_row(cx),
            Slot::SampleRate => self.sample_rate_row(cx),
            Slot::Buffer => self.buffer_row(cx),
            Slot::Widevine => self.widevine_row(cx),
            Slot::Equalizer => self.equalizer_row(cx),
            Slot::LyricsSources => self.lyrics_sources_row(cx),
            Slot::LyricsPriority => self.lyrics_priority(cx),
            Slot::Karaoke => self.karaoke_lyrics_row(cx),
            Slot::Romanized => self.romanized_lyrics_row(cx),
            Slot::LyricsForLocal => self.lyrics_for_local_files_row(cx),
            Slot::SpotifyConnect => self.spotify_connect_row(cx),
            Slot::SpotifyConnectName => self.spotify_connect_name_row(cx),
            Slot::SpotifyConnectCustom => self.spotify_connect_custom_row(),
            Slot::SessionsButton => self.sessions_button_row(cx),
            Slot::SessionsOfficial => self.sessions_official_row(cx),
            Slot::SessionsServer => self.sessions_server_row(cx),
            Slot::Discord => self.discord_row(cx),
            Slot::DiscordName => self.discord_name_row(cx),
            Slot::DiscordShowPaused => self.discord_show_paused_row(cx),
            Slot::DiscordBadge => self.discord_badge_row(cx),
            Slot::ArtworkForLocal => self.artwork_for_local_files_row(cx),
            Slot::DiscordAnonymous => self.discord_anonymous_row(cx),
            Slot::DiscordHideOnConnect => self.discord_hide_on_connect_row(cx),
            Slot::DiscordButtons => self.discord_buttons_row(cx),
            Slot::Scrobble(index) => match index < self.scrobbling.read(cx).rows().len() {
                true => self.scrobble_row(index, cx),
                false => div().into_any_element(),
            },
            Slot::Version => self.version_row(),
            Slot::Updates => self.updates_row(cx),
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
            Slot::Accounts => self.accounts_rows(cx),
            Slot::Theme => self.theme_cards(cx),
            Slot::Icons => self.icon_cards(window, cx),
            Slot::Corners => self.corner_cards(window, cx),
            Slot::Visualizer => self.visualizer_cards(window, cx),
            Slot::PlayerBarLayout => self.player_bar_cards(window, cx),
            Slot::StreamQuality => self.quality_cards(window, cx),
            slot => self.slot_element(slot, window, cx),
        }
    }

    /// One group: the eyebrow with its icon, its option cards, then its rows, all on the page
    /// itself with no surface around them and nothing between the rows.
    fn group(&self, group: Group, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Group {
            title,
            icon,
            cards,
            rows,
        } = group;
        let heading = title.map(|key| {
            let label = i18n::lookup(key, None);
            group_heading(label.clone(), icon, cx)
                .id(SharedString::from(format!("settings-group-{key}")))
                .role(Role::Heading)
                .aria_level(2)
                .aria_label(label)
        });
        let carded = cards.is_some();
        let cards = cards.map(|slot| {
            let cards = self.cards(slot, window, cx);
            self.revealing(Reveal::Slot(slot), cards, cx)
        });
        let rows = rows
            .into_iter()
            .map(|slot| {
                let row = self.slot_element(slot, window, cx);
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

        div()
            .flex()
            .flex_col()
            .children(heading)
            .children(cards)
            .children(rows)
            .into_any_element()
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
    fn tab(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let pages = match self.page.parent() {
            Some(_) => vec![self.page],
            None => self.page.tab().sections().to_vec(),
        };
        // a section's title stands as close over its groups as the page's own title does
        let inset = cx.theme().metrics.inset;
        let pages = pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                let title = (index > 0).then(|| {
                    let title = section_title(page, cx).into_any_element();
                    div().mt(GROUPS).mb(inset).child(self.revealing(
                        Reveal::Section(page),
                        title,
                        cx,
                    ))
                });
                let groups = self
                    .groups(page, cx)
                    .into_iter()
                    .enumerate()
                    .map(|(index, group)| {
                        // A search target must lay out even off screen so its reveal marker
                        // can position the scroll. The other groups keep their cached heights.
                        if self.reveal.is_some_and(|reveal| {
                            group.slots().any(|slot| reveal == Reveal::Slot(slot))
                        }) {
                            return self.group(group, window, cx);
                        }
                        self.lazy((page.id(), index + 3), cx, move |this, window, cx| {
                            this.group(group, window, cx)
                        })
                        .into_any_element()
                    })
                    .collect::<Vec<_>>();

                div().flex().flex_col().children(title).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(GROUPS)
                        .children(self.header(page, cx))
                        .children(self.preview(page, cx))
                        .children(groups)
                        .children(self.footer(page, cx)),
                )
            })
            .collect::<Vec<_>>();

        div().flex().flex_col().gap(GROUPS).children(pages)
    }

    /// What a search shows instead of the page: every setting that answers it, each naming the
    /// page it lives on and opening it there.
    pub(crate) fn results(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let hits = self.found(cx);
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
                            Some((page, slot)) => this.result((index, count), *page, *slot, cx),
                            None => div().into_any_element(),
                        }
                    })),
            ),
        )
    }

    /// One search result, placed by its rank and the number of results: the setting's name over
    /// the page it lives on.
    fn result(
        &self,
        (index, count): (usize, usize),
        page: SettingsPage,
        slot: Slot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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

        Card::new(
            SharedString::from(format!("settings-result-{}-{title}", page.id())),
            title,
        )
        .aria_description(place.clone())
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
        .fallback(tab.icon())
        .meta(place)
        .press(cx.listener(move |this, _, _, cx| this.open_result(page, slot, cx)))
        .into_any_element()
    }

    /// Opens the page a result lives on and scrolls its setting into view.
    fn open_result(&mut self, page: SettingsPage, slot: Slot, cx: &mut Context<Self>) {
        self.reveal = Some(Reveal::Slot(slot));
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
            || theme_delete.is_some();
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
        let named = self.searching().then(|| t!("settings-search-results"));

        layout::frame(
            &destination,
            named,
            "settings",
            &self.scrollbar,
            body,
            window,
            cx,
        )
        .track_focus(&self.focus)
        // the user taking the page in hand lets go of whatever it was holding in view
        .on_scroll_wheel(cx.listener(|this, _, _, _| this.reveal = None))
        .capture_any_mouse_down(cx.listener(|this, _, _, _| this.reveal = None))
        // an action stops where it is handled unless told to go on, so an escape with nothing
        // to close here goes on to the workspace
        .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
            if !this.escape(cx) {
                cx.propagate();
            }
        }))
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
    }
}

/// A group's heading: its icon, when it has one, before its name as an uppercase eyebrow.
pub(super) fn group_heading(label: SharedString, icon: Option<&'static str>, cx: &App) -> Div {
    let theme = cx.theme();

    faint(cx)
        .flex()
        .items_center()
        .gap_2()
        .mb(theme.metrics.pad)
        .children(icon.map(|icon| {
            svg()
                .path(icons::path(icon))
                .size(theme.text(ui::Text::Small))
                .flex_none()
                .text_color(theme.muted_foreground)
        }))
        .child(upper(label))
}
