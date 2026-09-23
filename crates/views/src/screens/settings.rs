use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::shared::confirm::{Confirm, Kind};
use crate::shared::effects;
use crate::shared::local;
use crate::shared::popups::{AccountPicker, CookiePrompt, SearchPopup, matches_query};
use crate::shared::text;
use crate::shared::veil::{Edge, veil};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, FontWeight, MouseButton,
    MouseUpEvent, Pixels, Render, SharedString, Task, Window, div, px, relative,
};
use gpui::{ScrollHandle, prelude::*, svg};
use i18n::{Language, t};
use music::drm::Origin;
use music::equalizer::{self, Preset};
use music::scrobble::{Link, Secret};
use music::{AccountChoice, SignIn, SignInPrompt, WritingSystem};
use router::{Destination, NavEntry, Screen, SettingsTab, navigate};
use state::{
    AppSettings, CdmState, DiscordName, Drm, Failure, FullscreenControlsAutohide, Io,
    MAX_PARTICLES, Playback, SYSTEM_FONT, Scan, ScrobbleState, Scrobbling, Session, SessionState,
    Sleep, Sonora,
};
use ui::{ActiveTheme as _, Deck, LEADING, Scrollbar, Scroller, eyebrow, snapped};
use ui::{
    Avatar, Button, Dismiss, InfoCard, Initials, Input, Look, MAX_FONT, MAX_LYRICS_SCALE,
    MAX_TRANSPARENCY, MIN_FONT, MIN_LYRICS_SCALE, MenuItem, Modal, Pace, Picker, Popovers, Radio,
    Rounding, Saver, Scrubber, ScrubberState, Separator, Skeleton, StageStyle, Stillness, Switch,
    TabBar, Text, Theme, ThemeKind, Vacancy, VisualizerStyle,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How wide the settings column, the search field and the category bar grow at most.
const WIDTH: Pixels = px(640.);
/// How much a query found in a setting's title counts over one found in its detail.
const TITLE_WEIGHT: u32 = 2;
/// How far the rows are blurred where they pass under the header. One blur pass serves every
/// strip, so this is also the widest the haze gets. The renderer cuts a kernel off at 24 taps
/// on a quarter-resolution frame, so past about 32px a wider radius only flattens the curve.
const HEADER_BLUR: Pixels = px(1.);
/// How far past the header the rows keep dissolving, so the handoff under the blur has
/// no hard edge where sharp content emerges from the haze.
const HEADER_FADE_TAIL: Pixels = px(48.);
const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
const SOURCE_URL: &str = "https://github.com/sonorahq/sonora";

const THEMES: &str = "themes";
const PACKS: &str = "packs";
const CORNERS: &str = "corners";
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
const WINDOW_ROUNDING: &str = "window-rounding";
const FULLSCREEN_CONTROLS_AUTOHIDE: &str = "fullscreen-controls-autohide";
const STAGE_STYLE: &str = "stage-style";
const VISUALIZER_STYLE: &str = "visualizer-style";
const LANGUAGES: &str = "languages";
const TYPEFACES: &str = "typefaces";
const TYPEFACE_LIMIT: usize = 200;
const TYPEFACE_LEAD: usize = 2;
// faces previewed before a measurement
const TYPEFACE_GUESS: usize = 24;
// faces loaded per frame
const TYPEFACE_BATCH: usize = 3;
const STARTUP: &str = "startup";
const ENTRIES: &str = "entries";
const DISCORD_NAME: &str = "discord-name";
const DISCORD_BUTTONS: &str = "discord-buttons";
const MOTION: &str = "motion";
const PACE: &str = "pace";
const SAVER: &str = "saver";
const SLEEP: &str = "sleep";
const EQUALIZER_PRESETS: &str = "equalizer-presets";
// the step a dragged band snaps to, in decibels
const EQUALIZER_STEP: f32 = 0.5;
const SLEEP_MAX_MINUTES: u64 = 120;
const SLEEP_MAGNETS: [u64; 4] = [15, 30, 45, 60];
const SLEEP_MAGNET_WEIGHT: usize = 4;
const SLEEP_LAST: usize =
    SLEEP_MAX_MINUTES as usize + SLEEP_MAGNETS.len() * (SLEEP_MAGNET_WEIGHT - 1) + 1;

/// How tall one line of `step` text stands. The deck needs every row's height before it
/// builds anything, so wherever a height is summed from text, that text pins itself to this
/// leading and truncates to one line.
fn line(theme: &Theme, step: Text) -> Pixels {
    px((theme.text(step) / px(1.) * LEADING).round())
}

/// How tall a standard row draws: its padding over one title line and one detail line, the
/// layout `SettingsView::row` builds.
fn standard_height(theme: &Theme) -> Pixels {
    SECTION_GAP + line(theme, Text::Body) + ROW_GAP + line(theme, Text::Small) + SECTION_GAP
}

/// The deck draws only the rows in view, so every row's height is fixed by construction:
/// standard rows and titles keep one height each, separators are one pixel, and the three
/// composite rows sum theirs from fixed parts in `SettingsView::slot_height`.
const SEPARATOR_HEIGHT: Pixels = px(1.);
/// Slack kept under the accounts block, so a border the formula counts differently still
/// cannot clip the last card.
const ACCOUNTS_SLACK: Pixels = px(2.);
/// `gap_1`, the breathing room inside a summed row.
const ROW_GAP: Pixels = px(4.);
/// `gap_2`, the step between stacked buttons and band labels.
const BLOCK_GAP: Pixels = px(8.);
/// `gap_3` and `py_3`, the step between blocks of a composite row.
const SECTION_GAP: Pixels = px(12.);
/// `gap_0p5`, the step between a provider's name and its status.
const HALF_GAP: Pixels = px(2.);
/// The icon beside a provider's sign-in error.
const ERROR_ICON: Pixels = px(14.);
/// The arrow at the end of a card that signs in when it is clicked.
const ARROW: Pixels = px(14.);
/// How many buttons tall the sign-in choice stands, however few it lists.
const CHOICES: f32 = 3.;

/// One row of the settings page, described without building anything. The deck scores and
/// measures slots and only builds the ones in view, so typing a search or scrolling never
/// constructs the rows it does not show.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Title(&'static str),
    Sep,
    Startup,
    Entries,
    Language,
    Tray,
    Accounts,
    LocalFolder,
    Theme,
    Adaptive,
    Ambient,
    AmbientMotion,
    Stage,
    Particles,
    Visualizer,
    Icons,
    Opacity,
    WindowBlur,
    Blur,
    Corners,
    FullscreenControlsAutohide,
    PanelLyricsSize,
    FullscreenLyricsSize,
    BlurLyrics,
    Font,
    Typeface,
    Motion,
    Pace,
    Saver,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    ServerSideDecorations,
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
    Decorations,
    #[cfg(not(target_os = "macos"))]
    Side,
    #[cfg(not(target_os = "macos"))]
    TrafficLights,
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
    WindowRounding,
    AdaptiveMenu,
    Normalisation,
    Gapless,
    Sleep,
    Widevine,
    Equalizer,
    EqualizerPreset,
    EqualizerBands,
    LyricsProviders,
    PreferLocalLyrics,
    Karaoke,
    Romanized,
    LyricsForLocal,
    Discord,
    DiscordName,
    DiscordShowPaused,
    DiscordBadge,
    DiscordAnonymous,
    DiscordButtons,
    Scrobble(usize),
    Version,
    Updates,
    Log,
    License,
    Source,
}

/// One setting on the page: the row that draws it, and the title and detail a search is
/// matched against.
struct Setting {
    title: SharedString,
    detail: SharedString,
    element: AnyElement,
}

/// One field of the scrobbling link dialog: which hint it carries, what it starts with, and
/// whether it holds a secret and so is drawn as dots.
struct Field {
    hint: &'static str,
    value: String,
    masked: bool,
}

/// What the guest card answers to, where a provider card answers to its slug.
const GUEST: &str = "guest";

/// What a whole account card does when it is clicked.
type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

struct Account {
    slug: &'static str,
    name: &'static str,
    options: Vec<SignIn>,
    web_sign_in: bool,
    stored: bool,
    active: bool,
    cancel: bool,
    error: Option<Failure>,
}

/// The guest entry the accounts block draws under the providers. It is a card of the app's
/// own, not a provider: it stands for the anonymous session `slug` offers, so signing out of
/// it signs that provider out.
#[derive(Clone, Copy)]
struct Guest {
    slug: &'static str,
    stored: bool,
    active: bool,
}

/// Which sign-in methods a provider card lists. Anonymous never appears there, because guest
/// mode has a card of its own, and a stored account is asked for nothing.
fn offered(method: &SignIn, stored: bool) -> bool {
    match method {
        SignIn::Default | SignIn::Secret | SignIn::Credentials { .. } => !stored,
        SignIn::Anonymous | SignIn::Path(_) => false,
    }
}

#[derive(Clone, Copy)]
struct Member {
    login: &'static str,
    avatar: &'static str,
    profile: &'static str,
    role: Role,
}

#[derive(Clone, Copy)]
enum Role {
    LeadMaintainer,
    Maintainer,
    Contributor,
}

impl Role {
    fn label(self) -> SharedString {
        match self {
            Self::LeadMaintainer => t!("settings-role-lead-maintainer"),
            Self::Maintainer => t!("settings-role-maintainer"),
            Self::Contributor => t!("settings-role-contributor"),
        }
    }
}

macro_rules! member {
    ($login:literal, $role:expr) => {
        Member {
            login: $login,
            avatar: concat!("https://github.com/", $login, ".png"),
            profile: concat!("https://github.com/", $login),
            role: $role,
        }
    };
}

const MEMBERS: [Member; 5] = [
    member!("nolight132", Role::LeadMaintainer),
    member!("zxsleebu", Role::Maintainer),
    member!("fx-got", Role::Maintainer),
    member!("Makakashan", Role::Contributor),
    member!("imizgun", Role::Contributor),
];

pub struct SettingsView {
    session: Entity<Session>,
    playback: Entity<Playback>,
    drm: Entity<Drm>,
    settings: Entity<AppSettings>,
    tab: SettingsTab,
    search: Entity<Input>,
    /// The search field's text, trimmed. Empty means the page shows one category.
    query: String,
    /// How tall the header floating over the page measured last, so the rows start beneath it.
    header_height: Pixels,
    /// Whether the header has measured itself at least once. Until then the height is a
    /// zero stand-in, and the page stays hidden rather than flashing unpadded for a frame.
    header_measured: bool,
    scrollbar: Entity<Scrollbar>,
    opacity: ScrubberState,
    sleep: ScrubberState,
    /// One slider per equalizer band, lowest first.
    bands: Vec<ScrubberState>,
    pending_sleep: Option<Option<Sleep>>,
    popovers: Popovers,
    server: Entity<Input>,
    username: Entity<Input>,
    password: Entity<Input>,
    credentials_for: Option<&'static str>,
    secret: Entity<Input>,
    manual_secret: Option<(&'static str, &'static str)>,
    scrobbling: Entity<Scrobbling>,
    scrobble_first: Entity<Input>,
    scrobble_second: Entity<Input>,
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
        cx.observe(&drm, |_, _, cx| cx.notify()).detach();
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        cx.observe(&scrobbling, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        cx.observe(&Scan::global(cx), |_, _, cx| cx.notify())
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
                .clearable()
                // the field floats over the rows the page scrolls beneath it, so it frosts
                // them the way the category bar under it does
                .blurred()
        });
        cx.observe(&search, |this, input, cx| {
            let query = input.read(cx).text().trim().to_owned();
            // A search swaps the rows for hits from every category, so wherever the page was
            // scrolled to answers for nothing once one starts.
            if this.query.is_empty() && !query.is_empty() {
                this.scrollbar.update(cx, |bar, _| bar.place(Pixels::ZERO));
            }
            this.query = query;
            cx.notify();
        })
        .detach();

        Self {
            session,
            playback,
            drm,
            settings,
            tab: SettingsTab::General,
            search,
            query: String::new(),
            header_height: Pixels::ZERO,
            header_measured: false,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            opacity: ScrubberState::new("opacity"),
            sleep: ScrubberState::new("sleep"),
            bands: (0..equalizer::BANDS)
                .map(|band| ScrubberState::new(format!("equalizer-band-{band}")))
                .collect(),
            pending_sleep: None,
            popovers: Popovers::default(),
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
            scrobble_prompt: None,
            languages,
            typefaces,
            typeface_faced: RefCell::new(HashSet::new()),
            installed: None,
            loading_fonts: false,
            font_task: None,
        }
    }

    /// Shows one category and ends any search, so a route always lands on a plain page.
    pub(crate) fn select(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.popovers.close();
        if !self.search.read(cx).text().is_empty() {
            self.search.update(cx, |input, cx| input.set_text("", cx));
        }
        cx.notify();
    }

    fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// Takes the header's measured height. The header floats over the page, so the rows pad
    /// themselves by this much and scroll beneath it, and the scrollbar starts below it so
    /// the bar never slides under the blur.
    fn set_header_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        if self.header_height == height {
            return;
        }
        self.header_height = height;
        self.header_measured = true;
        self.scrollbar.update(cx, |bar, cx| {
            bar.set_track_top(height, cx);
        });
        cx.notify();
    }

    /// The rows of the page as a deck: the current category's, or, while a search is on,
    /// every category's that answers it, best match first and without the group titles. Only
    /// the rows in view are ever built.
    fn panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let bare = match self.searching() {
            false => self.tab_slots(self.tab, cx),
            true => self.found(cx),
        };
        if bare.is_empty() {
            return Vacancy::new(t!("search-no-matches"))
                .icon("icons/search.svg")
                .py_6()
                .into_any_element();
        }

        let mut slots = Vec::with_capacity(bare.len() * 2);
        let mut parted = false;
        for slot in bare {
            let titled = matches!(slot, Slot::Title(_));
            if parted && !titled {
                slots.push(Slot::Sep);
            }
            parted = !titled;
            slots.push(slot);
        }

        let heights = slots
            .iter()
            .map(|slot| self.slot_height(*slot, window, cx))
            .collect::<Vec<_>>();
        Deck::new("settings-deck")
            .rows(heights)
            .draw(cx.processor(
                move |this: &mut Self, index: usize, _, cx| match slots.get(index) {
                    Some(slot) => this.slot_element(*slot, cx),
                    None => div().into_any_element(),
                },
            ))
            .into_any_element()
    }

    /// Every setting of every category that answers the query, best match first. Ties keep
    /// the order of the categories. Only titles and details are read; no row is built.
    fn found(&self, cx: &App) -> Vec<Slot> {
        let mut hits: Vec<(u32, Slot)> = SettingsTab::ALL
            .into_iter()
            .flat_map(|tab| self.tab_slots(tab, cx))
            .filter_map(|slot| {
                self.slot_score(slot, &self.query, cx)
                    .map(|score| (score, slot))
            })
            .collect();
        hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        hits.into_iter().map(|(_, slot)| slot).collect()
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

    fn tab_slots(&self, tab: SettingsTab, cx: &App) -> Vec<Slot> {
        match tab {
            SettingsTab::General => vec![
                Slot::Startup,
                Slot::Entries,
                Slot::Language,
                Slot::Title("settings-group-window"),
                Slot::Tray,
                Slot::Title("settings-group-accounts"),
                Slot::Accounts,
                Slot::Title("settings-group-library"),
                Slot::LocalFolder,
            ],
            SettingsTab::Appearance => vec![
                Slot::Title("settings-tab-general"),
                Slot::Theme,
                Slot::Adaptive,
                Slot::Icons,
                Slot::Opacity,
            ]
            .into_iter()
            .chain(ui::WINDOW_BLUR.then_some(Slot::WindowBlur))
            .chain([
                Slot::Blur,
                Slot::Corners,
                Slot::Title("settings-group-fullscreen"),
                Slot::Ambient,
            ])
            .chain(
                self.settings
                    .read(cx)
                    .ambient()
                    .then_some(Slot::AmbientMotion),
            )
            .chain([Slot::Stage])
            .chain(
                self.settings
                    .read(cx)
                    .stage_style()
                    .fielded()
                    .then_some(Slot::Particles),
            )
            .chain([
                Slot::Visualizer,
                Slot::FullscreenControlsAutohide,
                Slot::Title("settings-group-lyrics"),
                Slot::PanelLyricsSize,
                Slot::FullscreenLyricsSize,
                Slot::BlurLyrics,
                Slot::Title("settings-group-text"),
                Slot::Font,
                Slot::Typeface,
                Slot::Title("settings-group-motion"),
                Slot::Motion,
                Slot::Pace,
                Slot::Saver,
            ])
            .chain(self.decoration_slots(cx))
            .chain([Slot::Title("settings-advanced"), Slot::AdaptiveMenu])
            .collect(),
            SettingsTab::Playback => {
                let mut slots = vec![
                    Slot::Title("settings-tab-general"),
                    Slot::Normalisation,
                    Slot::Gapless,
                    Slot::Sleep,
                ];
                if self.drm.read(cx).shown(cx) {
                    slots.push(Slot::Widevine);
                }
                slots.extend([Slot::Title("settings-group-equalizer"), Slot::Equalizer]);
                if self.playback.read(cx).equalizer() {
                    slots.push(Slot::EqualizerPreset);
                    slots.push(Slot::EqualizerBands);
                }
                slots.extend([Slot::Title("settings-group-lyrics"), Slot::LyricsProviders]);
                if self
                    .settings
                    .read(cx)
                    .lyrics_provider_enabled(music::lyrics::LOCAL)
                {
                    slots.push(Slot::PreferLocalLyrics);
                }
                slots.extend([Slot::Karaoke, Slot::Romanized]);
                slots
            }
            SettingsTab::Privacy => {
                vec![Slot::Title("settings-group-lyrics"), Slot::LyricsForLocal]
            }
            SettingsTab::Integrations => self
                .discord_slots(cx)
                .into_iter()
                .chain([Slot::Title("settings-group-scrobbling")])
                .chain(self.scrobble_slots(cx))
                .collect(),
            SettingsTab::About => vec![
                Slot::Title("settings-tab-general"),
                Slot::Version,
                Slot::Updates,
                Slot::Log,
                Slot::Title("settings-group-project"),
                Slot::License,
                Slot::Source,
            ],
        }
    }

    /// The title and detail a search is matched against, without building the row. Kept next
    /// to the builders by key: a new row needs its pair here to be found.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per row keeps the keys beside the rows"
    )]
    fn slot_meta(&self, slot: Slot, cx: &App) -> Option<(SharedString, SharedString)> {
        let meta = match slot {
            Slot::Title(_) | Slot::Sep => return None,
            Slot::Startup => (t!("settings-startup"), t!("settings-startup-detail")),
            Slot::Entries => (t!("settings-entries"), t!("settings-entries-detail")),
            Slot::Language => (t!("settings-language"), t!("settings-language-detail")),
            Slot::Tray => (
                t!("settings-close-to-tray"),
                t!("settings-close-to-tray-detail"),
            ),
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
            Slot::Ambient => (t!("settings-ambient"), t!("settings-ambient-detail")),
            Slot::AmbientMotion => (
                t!("settings-ambient-motion"),
                t!("settings-ambient-motion-detail"),
            ),
            Slot::Stage => (t!("settings-stage"), t!("settings-stage-detail")),
            Slot::Particles => (t!("settings-particles"), t!("settings-particles-detail")),
            Slot::Visualizer => (t!("settings-visualizer"), t!("settings-visualizer-detail")),
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
            Slot::PanelLyricsSize => (
                i18n::lookup("settings-panel-lyrics-size", None),
                i18n::lookup("settings-panel-lyrics-size-detail", None),
            ),
            Slot::FullscreenLyricsSize => (
                i18n::lookup("settings-fullscreen-lyrics-size", None),
                i18n::lookup("settings-fullscreen-lyrics-size-detail", None),
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
            #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
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
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
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
            Slot::Sleep => (t!("settings-sleep"), t!("settings-sleep-detail")),
            Slot::Widevine => {
                let (detail, _) = widevine_copy(self.drm.read(cx).state());
                (t!("settings-widevine"), i18n::lookup(detail, None))
            }
            Slot::Equalizer => (t!("settings-equalizer"), t!("settings-equalizer-detail")),
            Slot::EqualizerPreset => (
                t!("settings-equalizer-preset"),
                t!("settings-equalizer-preset-detail"),
            ),
            Slot::EqualizerBands => (t!("settings-group-equalizer"), SharedString::default()),
            Slot::LyricsProviders => (
                t!("settings-lyrics-providers"),
                t!("settings-lyrics-providers-detail"),
            ),
            Slot::PreferLocalLyrics => (
                t!("settings-prefer-local-lyrics"),
                t!("settings-prefer-local-lyrics-detail"),
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
            Slot::DiscordAnonymous => (
                t!("settings-discord-anonymous"),
                t!("settings-discord-anonymous-detail"),
            ),
            Slot::DiscordButtons => (
                t!("settings-discord-buttons"),
                t!("settings-discord-buttons-detail"),
            ),
            Slot::Scrobble(index) => {
                let rows = self.scrobbling.read(cx).rows();
                let row = rows.get(index)?;
                let service = row.id();
                let detail = match row.state() {
                    ScrobbleState::Off => t!("settings-scrobble-off"),
                    ScrobbleState::Linking => t!("settings-scrobble-waiting"),
                    ScrobbleState::On(name) => match name.is_empty() {
                        true => t!("settings-scrobble-on"),
                        false => t!("settings-scrobble-as", name = name.as_ref()),
                    },
                    ScrobbleState::Failed(key) => i18n::lookup(key, None),
                };
                (i18n::lookup(&format!("settings-{service}"), None), detail)
            }
            Slot::Version => (t!("settings-version"), t!("settings-version-detail")),
            Slot::Updates => (
                t!("settings-check-updates"),
                t!("settings-check-updates-detail"),
            ),
            Slot::Log => (t!("settings-log"), t!("settings-log-detail")),
            Slot::License => (t!("settings-license"), t!("settings-license-detail")),
            Slot::Source => (t!("settings-source"), t!("settings-source-detail")),
        };
        Some(meta)
    }

    /// How tall the slot draws. Standard rows share one height with the row builder, and
    /// the composite rows sum theirs from the same fixed parts their elements are built of.
    fn slot_height(&self, slot: Slot, window: &Window, cx: &App) -> Pixels {
        let theme = *cx.theme();
        match slot {
            Slot::Title(_) => snapped(theme.metrics.row, window),
            Slot::Sep => SEPARATOR_HEIGHT,
            Slot::Accounts => snapped(self.accounts_height(&theme, cx), window),
            Slot::LocalFolder => snapped(self.local_height(&theme, cx), window),
            Slot::EqualizerBands => snapped(
                SECTION_GAP
                    + line(&theme, Text::Tiny)
                    + BLOCK_GAP
                    + theme.metrics.cover
                    + BLOCK_GAP
                    + line(&theme, Text::Tiny)
                    + SECTION_GAP,
                window,
            ),
            _ => snapped(standard_height(&theme), window),
        }
    }

    /// The accounts block: the header over one card per provider. Summed from the same fixed
    /// parts the element is built of, so the deck never clips a card.
    fn accounts_height(&self, theme: &Theme, cx: &App) -> Pixels {
        let head = line(theme, Text::Body) + ROW_GAP + line(theme, Text::Small);
        let mut total = SECTION_GAP + head + SECTION_GAP;
        for account in self.providers(cx) {
            total += SECTION_GAP + card_height(theme, account.error.is_some());
        }
        if self.guest(cx).is_some() {
            total += SECTION_GAP + card_height(theme, false);
        }
        total + ACCOUNTS_SLACK
    }

    /// The local folder block: the header over one line per watched folder.
    fn local_height(&self, theme: &Theme, cx: &App) -> Pixels {
        let mut total = standard_height(theme);
        let paths = self.session.read(cx).local_paths().len();
        if paths > 0 {
            total += ROW_GAP
                + theme.metrics.control_small * paths as f32
                + ROW_GAP * paths.saturating_sub(1) as f32
                + BLOCK_GAP;
        }
        total
    }

    /// Builds the slot's row. Asked only for the rows in view, every frame they are.
    fn slot_element(&self, slot: Slot, cx: &mut Context<Self>) -> AnyElement {
        match slot {
            Slot::Title(key) => div()
                .h(cx.theme().metrics.row)
                .flex()
                .flex_col()
                .justify_end()
                .pb_1()
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(eyebrow(i18n::lookup(key, None), cx)),
                )
                .into_any_element(),
            Slot::Sep => Separator::horizontal().w_full().into_any_element(),
            Slot::Startup => self.startup_row(cx).element,
            Slot::Entries => self.entries_row(cx).element,
            Slot::Language => self.language_row(cx).element,
            Slot::Tray => self.tray_row(cx).element,
            Slot::Accounts => self.accounts_row(cx).element,
            Slot::LocalFolder => self.local_folder_row(cx).element,
            Slot::Theme => self.theme_row(cx).element,
            Slot::Adaptive => self.adaptive_row(cx).element,
            Slot::Ambient => self.ambient_row(cx).element,
            Slot::AmbientMotion => self.ambient_motion_row(cx).element,
            Slot::Stage => self.stage_row(cx).element,
            Slot::Particles => self.particles_row(cx).element,
            Slot::Visualizer => self.visualizer_style_row(cx).element,
            Slot::Icons => self.icons_row(cx).element,
            Slot::Opacity => self.opacity_row(cx).element,
            Slot::WindowBlur => self.blur_window_row(cx).element,
            Slot::Blur => self.blur_row(cx).element,
            Slot::Corners => self.corners_row(cx).element,
            Slot::FullscreenControlsAutohide => self.fullscreen_controls_autohide_row(cx).element,
            Slot::PanelLyricsSize => self.panel_lyrics_size_row(cx).element,
            Slot::FullscreenLyricsSize => self.fullscreen_lyrics_size_row(cx).element,
            Slot::BlurLyrics => self.blur_lyrics_row(cx).element,
            Slot::Font => self.font_row(cx).element,
            Slot::Typeface => self.typeface_row(cx).element,
            Slot::Motion => self.motion_row(cx).element,
            Slot::Pace => self.pace_row(cx).element,
            Slot::Saver => self.saver_row(cx).element,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Slot::ServerSideDecorations => self.server_side_decorations_row(cx).element,
            #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
            Slot::Decorations => self.decorations_row(cx).element,
            #[cfg(not(target_os = "macos"))]
            Slot::Side => self.side_row(cx).element,
            #[cfg(not(target_os = "macos"))]
            Slot::TrafficLights => self.traffic_light_controls_row(cx).element,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
            Slot::WindowRounding => self.window_rounding_row(cx).element,
            Slot::AdaptiveMenu => self.adaptive_menu_row(cx).element,
            Slot::Normalisation => self.playback_row(cx).element,
            Slot::Gapless => self.gapless_row(cx).element,
            Slot::Sleep => self.sleep_row(cx).element,
            Slot::Widevine => self.widevine_row(cx).element,
            Slot::Equalizer => self.equalizer_row(cx).element,
            Slot::EqualizerPreset => self.equalizer_preset_row(cx).element,
            Slot::EqualizerBands => self.equalizer_bands_row(cx).element,
            Slot::LyricsProviders => self.lyrics_providers_row(cx).element,
            Slot::PreferLocalLyrics => self.prefer_local_lyrics_row(cx).element,
            Slot::Karaoke => self.karaoke_lyrics_row(cx).element,
            Slot::Romanized => self.romanized_lyrics_row(cx).element,
            Slot::LyricsForLocal => self.lyrics_for_local_files_row(cx).element,
            Slot::Discord => self.discord_row(cx).element,
            Slot::DiscordName => self.discord_name_row(cx).element,
            Slot::DiscordShowPaused => self.discord_show_paused_row(cx).element,
            Slot::DiscordBadge => self.discord_badge_row(cx).element,
            Slot::DiscordAnonymous => self.discord_anonymous_row(cx).element,
            Slot::DiscordButtons => self.discord_buttons_row(cx).element,
            Slot::Scrobble(index) => match index < self.scrobbling.read(cx).rows().len() {
                true => self.scrobble_row(index, cx).element,
                false => div().into_any_element(),
            },
            Slot::Version => self.version_row(cx).element,
            Slot::Updates => self.updates_row(cx).element,
            Slot::Log => self.log_row(cx).element,
            Slot::License => self.license_row(cx).element,
            Slot::Source => self.source_row(cx).element,
        }
    }

    fn look(&self, cx: &Context<Self>) -> Look {
        Look {
            tint: cx.theme().tint,
            tint_secondary: cx.theme().tint_secondary,
            ..self.settings.read(cx).look()
        }
    }

    #[allow(
        unused_variables,
        reason = "cx is unused on macOS, no slots are listed there"
    )]
    fn decoration_slots(&self, cx: &App) -> Vec<Slot> {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let mut slots = vec![
            Slot::Title("settings-group-window-style"),
            Slot::ServerSideDecorations,
            Slot::Side,
            Slot::TrafficLights,
        ];
        // Server-side decorations put the compositor in charge of the frame, so window
        // rounding is only ever this app's call with client-side ones.
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if !self.settings.read(cx).server_side_decorations() {
            slots.push(Slot::WindowRounding);
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
        let slots = vec![
            Slot::Title("settings-group-title-bar"),
            Slot::Decorations,
            Slot::Side,
            Slot::TrafficLights,
            Slot::WindowRounding,
        ];
        #[cfg(target_os = "macos")]
        let slots = Vec::<Slot>::new();
        slots
    }

    fn startup_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = Screen::from_id(self.settings.read(cx).startup()).unwrap_or(Screen::Home);
        let current = i18n::lookup(chosen.key(), None);

        let picker = Picker::new(STARTUP, &self.popovers, current)
            .width(Picker::NARROW)
            .items(Screen::ALL.map(|screen| {
                MenuItem::new(screen.id(), i18n::lookup(screen.key(), None))
                    .selected(screen == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_startup(screen.id(), cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-startup"),
            t!("settings-startup-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn entries_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let picker = Picker::new(ENTRIES, &self.popovers, t!("settings-entries-pick"))
            .width(Picker::REGULAR)
            .sticky()
            .items(NavEntry::ALL.map(|entry| {
                let shown = self.settings.read(cx).nav_shown(entry.id());

                MenuItem::new(entry.id(), i18n::lookup(entry.key(), None))
                    .selected(shown)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_nav_shown(entry.id(), !shown, cx)
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-entries"),
            t!("settings-entries-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn language_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).language().to_owned();
        let current = match Language::from_id(&chosen) {
            Some(language) => SharedString::from(language.label()),
            None => t!("settings-language-system"),
        };

        let asked = self.languages.query();
        let entries = std::iter::once((i18n::AUTO, t!("settings-language-system")))
            .chain(
                Language::ALL
                    .into_iter()
                    .map(|language| (language.id(), SharedString::from(language.label()))),
            )
            .filter(|(id, label)| matches_query(id, label, &asked))
            .collect::<Vec<_>>();
        let barren = entries.is_empty();
        let count = entries.len();
        let cursor = self.languages.cursor(count);
        let submitted = entries.clone();

        let picker = Picker::new(LANGUAGES, &self.popovers, current)
            .width(Picker::WIDE)
            .menu(self.languages.menu("languages-menu", Picker::WIDE))
            .items(entries.into_iter().enumerate().map(|(place, (id, label))| {
                MenuItem::new(id, label)
                    .selected(place == cursor)
                    .checked(chosen == id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_language(id, cx));
                        this.popovers.close();
                        cx.notify();
                    }))
            }))
            .when(barren, |picker| {
                picker
                    .item(MenuItem::new("language-empty", t!("settings-language-none")).disabled())
            });
        let picker = self.languages.controls(
            picker,
            count,
            move |this, place, _, cx| {
                let Some((id, _)) = submitted.get(place) else {
                    return;
                };
                this.settings
                    .update(cx, |settings, cx| settings.set_language(*id, cx));
                this.popovers.close();
                cx.notify();
            },
            cx,
        );

        self.row(
            t!("settings-language"),
            t!("settings-language-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn typeface_entries(&self) -> Vec<SharedString> {
        let asked = self.typefaces.query();
        let installed = self.installed.as_deref().unwrap_or_default();

        std::iter::once(SharedString::from(SYSTEM_FONT))
            .chain(installed.iter().cloned())
            .filter(|name| {
                let label = match name.as_ref() == SYSTEM_FONT {
                    true => t!("settings-typeface-system"),
                    false => name.clone(),
                };
                matches_query(name, &label, &asked)
            })
            .take(TYPEFACE_LIMIT)
            .collect()
    }

    fn typeface_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).font().to_owned();
        let bundled = chosen == SYSTEM_FONT;
        let current = match bundled {
            true => t!("settings-typeface-system"),
            false => SharedString::from(chosen.clone()),
        };

        let picking = self.popovers.shows(TYPEFACES);
        let asked = self.typefaces.query();
        let installed = self.installed.as_deref().unwrap_or_default();

        let mut entries = Vec::new();
        if picking {
            entries.push((
                SharedString::from(SYSTEM_FONT),
                t!("settings-typeface-system"),
            ));
            entries.extend(installed.iter().map(|name| (name.clone(), name.clone())));
            entries = entries
                .into_iter()
                .filter(|(id, label)| matches_query(id, label, &asked))
                .take(TYPEFACE_LIMIT)
                .collect();
        }

        let barren = picking && entries.is_empty();
        let count = entries.len();
        let cursor = self.typefaces.cursor(count);
        let submitted = entries
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();

        let mut items = Vec::new();
        let mut waiting = false;

        if picking {
            let scroll = self.typefaces.scroll(cx);
            let row = scroll
                .bounds_for_item(0)
                .map(|item| item.size.height)
                .filter(|height| *height > px(0.));
            let first = row.map_or(cursor, |row| {
                ((-scroll.offset().y) / row).floor().max(0.) as usize
            });
            let shown = row.map_or(TYPEFACE_GUESS, |row| {
                (self.typefaces.height() / row).ceil() as usize
            });
            let previewed = first.saturating_sub(TYPEFACE_LEAD)..first + shown + TYPEFACE_LEAD;

            let mut budget = TYPEFACE_BATCH;
            let mut faced = self.typeface_faced.borrow_mut();

            // forget the faces that scrolled out of view, so scrolling back spends
            // the per-frame budget on them again rather than facing them all at once
            faced.retain(|name| {
                entries
                    .iter()
                    .enumerate()
                    .any(|(place, (id, _))| id == name && previewed.contains(&place))
            });

            items = entries
                .into_iter()
                .enumerate()
                .map(|(place, (id, label))| {
                    let name = id.clone();
                    let preview = name.clone();
                    let wanted = name.as_ref() != SYSTEM_FONT && previewed.contains(&place);
                    let shows = match (wanted, faced.contains(&name)) {
                        (false, _) => false,
                        (true, true) => true,
                        (true, false) => match budget {
                            0 => {
                                waiting = true;
                                false
                            }
                            _ => {
                                budget -= 1;
                                faced.insert(name.clone());
                                true
                            }
                        },
                    };
                    MenuItem::new(id, label)
                        .selected(place == cursor)
                        .checked(chosen == name.as_ref())
                        .when(shows, |item| item.face(preview))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let name = name.to_string();
                            this.settings
                                .update(cx, |settings, cx| settings.set_font(name, cx));
                            this.popovers.close();
                            cx.notify();
                        }))
                })
                .collect::<Vec<_>>();
            drop(faced);
        }

        if waiting {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }

        let picker = Picker::new(TYPEFACES, &self.popovers, current)
            .width(Picker::WIDE)
            .menu(self.typefaces.menu("typefaces-menu", Picker::WIDE))
            .items(items)
            .when(self.loading_fonts, |picker| {
                picker.item(
                    MenuItem::new("typeface-loading", t!("settings-typeface-loading")).disabled(),
                )
            })
            .when(barren && !self.loading_fonts, |picker| {
                picker
                    .item(MenuItem::new("typeface-empty", t!("settings-typeface-none")).disabled())
            });

        let keys = self.typefaces.controls(
            picker,
            count,
            move |this, place, _, cx| {
                let Some(name) = submitted.get(place) else {
                    return;
                };
                this.settings
                    .update(cx, |settings, cx| settings.set_font(name.to_string(), cx));
                this.popovers.close();
                cx.notify();
            },
            cx,
        );

        self.row(
            t!("settings-typeface"),
            t!("settings-typeface-detail"),
            muted,
            small,
            keys.into_any_element(),
        )
    }

    fn corners_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        let picker = Picker::new(CORNERS, &self.popovers, look.rounding.label())
            .width(Picker::NARROW)
            .items(Rounding::ALL.into_iter().map(|rounding| {
                let overrides = overrides.clone();
                MenuItem::new(rounding.id(), rounding.label())
                    .selected(look.rounding == rounding)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_rounding(rounding.id(), cx);
                        });
                        Theme::set(Look { rounding, ..look }, &overrides, cx);
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-corners"),
            t!("settings-corners-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    /// Turns every frosted treatment in the app on or off at once: the menus, the fields, the
    /// floating panels and the bands the chrome lays over the page. A backdrop blur is the
    /// priciest thing the renderer does per frame, which is the whole reason it is a choice.
    fn blur_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        self.row(
            t!("settings-blur"),
            t!("settings-blur-detail"),
            muted,
            small,
            Switch::new("blur", look.blur)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let blur = !look.blur;
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur(blur, cx));
                    Theme::set(Look { blur, ..look }, &overrides, cx);
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// Asks the platform to blur the desktop behind a see-through window. It needs something to
    /// show through, so the switch is off and disabled while the window is opaque.
    fn blur_window_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();
        let opaque = !look.transparent;

        self.row(
            t!("settings-blur-window"),
            t!("settings-blur-window-detail"),
            muted,
            small,
            Switch::new("blur-window", look.blur_window && !opaque)
                .disabled(opaque)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let blur_window = !look.blur_window;
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur_window(blur_window, cx));
                    Theme::set(
                        Look {
                            blur_window,
                            ..look
                        },
                        &overrides,
                        cx,
                    );
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn font_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        let step = move |id: &'static str, label: &'static str, delta: f32| {
            let overrides = overrides.clone();
            let wanted = (look.font + delta).clamp(MIN_FONT, MAX_FONT);

            Button::new(id)
                .label(label)
                .small()
                .outline()
                .disabled(wanted == look.font)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_font_size(wanted, cx));
                    Theme::set(
                        Look {
                            font: wanted,
                            ..look
                        },
                        &overrides,
                        cx,
                    );
                    cx.notify();
                }))
        };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("font-smaller", "−", -1.))
            .child(div().child(t!("settings-font-value", size = look.font.round() as i64)))
            .child(step("font-larger", "+", 1.));

        self.row(
            t!("settings-font"),
            t!("settings-font-detail"),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn server_side_decorations_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let enabled = self.settings.read(cx).server_side_decorations();

        self.row(
            t!("settings-server-side-decorations"),
            t!("settings-server-side-decorations-detail"),
            muted,
            small,
            Switch::new("server-side-decorations", enabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let decorations = this.settings.update(cx, |settings, cx| {
                        settings.set_server_side_decorations(!enabled, cx);
                        settings.window_decorations()
                    });
                    window.request_decorations(decorations);
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
    fn decorations_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).window_controls();

        self.row(
            t!("settings-window-controls"),
            t!("settings-window-controls-detail"),
            muted,
            small,
            Switch::new("window-controls", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_window_controls(!on, cx));
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(target_os = "macos"))]
    fn traffic_light_controls_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).traffic_light_controls();

        self.row(
            t!("settings-traffic-light-controls"),
            t!("settings-traffic-light-controls-detail"),
            muted,
            small,
            Switch::new("traffic-light-controls", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_traffic_light_controls(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd"))]
    fn window_rounding_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).window_rounding();

        let picker = Picker::new(WINDOW_ROUNDING, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Rounding::ALL.into_iter().map(|rounding| {
                MenuItem::new(rounding.id(), rounding.label())
                    .selected(current == rounding)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_window_rounding(rounding, cx)
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-window-rounding"),
            t!("settings-window-rounding-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    #[cfg(not(target_os = "macos"))]
    fn side_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let settings = self.settings.read(cx);
        let left = settings.controls_on_left();
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let shown = !settings.server_side_decorations();
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let shown = settings.window_controls();

        self.row(
            t!("settings-controls-side"),
            t!("settings-controls-side-detail"),
            muted,
            small,
            Button::new("controls-side")
                .label(match left {
                    true => t!("common-left"),
                    false => t!("common-right"),
                })
                .small()
                .outline()
                .disabled(!shown)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_controls_on_left(!left, cx));
                }))
                .into_any_element(),
        )
    }

    fn profile(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        div()
            .flex()
            .items_center()
            .gap_4()
            .child(match self.session.read(cx).state() {
                SessionState::SignedIn(profile) => {
                    Initials::new(profile.display_name.clone(), px(64.)).into_any_element()
                }
                _ => Skeleton::new().size(px(64.)).circle().into_any_element(),
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(match self.session.read(cx).state() {
                        SessionState::SignedIn(profile) => div()
                            .child(profile.display_name.clone())
                            .text_size(theme.text(Text::Large))
                            .font_weight(FontWeight::SEMIBOLD)
                            .into_any_element(),
                        _ => Skeleton::new().w(px(140.)).h(px(14.)).into_any_element(),
                    })
                    .child(match self.session.read(cx).state() {
                        SessionState::SignedIn(profile) => div()
                            .child(profile.id.clone())
                            .text_color(muted)
                            .text_size(theme.text(Text::Small))
                            .into_any_element(),
                        _ => Skeleton::new().w(px(90.)).h(px(10.)).into_any_element(),
                    }),
            )
    }

    fn theme_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let current = look.kind;
        let adaptive = self.settings.read(cx).adaptive_theme();
        let overrides = self.settings.read(cx).theme_overrides().clone();

        let picker = Picker::new(THEMES, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(ThemeKind::ALL.into_iter().map(|kind| {
                let item = MenuItem::new(kind.id(), kind.label()).selected(current == kind);
                match adaptive
                    && !matches!(kind, ThemeKind::System | ThemeKind::Dark | ThemeKind::Light)
                {
                    true => item.disabled(),
                    false => {
                        let overrides = overrides.clone();
                        item.on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.update(cx, |settings, cx| {
                                settings.set_theme(kind.id(), cx);
                            });
                            Theme::fade(Look { kind, ..look }, &overrides, cx);
                            cx.notify();
                        }))
                    }
                }
            }));

        let settings = self.settings.clone();
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("open-theme-config")
                    .label(t!("settings-theme-config"))
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| {
                        let path = settings.update(cx, |settings, _| settings.ensure_file());
                        if let Err(error) = open_path(&path) {
                            log::warn!("settings: cannot open {}: {error}", path.display());
                        }
                    }),
            )
            .child(picker);

        self.row(
            t!("settings-theme"),
            t!("settings-theme-detail"),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    fn icons_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).icons().to_owned();
        let current = icons::pack(&chosen).unwrap_or_else(icons::active);

        let picker = Picker::new(PACKS, &self.popovers, current.title())
            .width(Picker::REGULAR)
            .items(icons::packs().map(|pack| {
                MenuItem::new(pack.id, pack.title())
                    .selected(pack.id == current.id)
                    .detail(samples(pack, muted))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_icons(pack.id, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-icons"),
            t!("settings-icons-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn opacity_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();
        let transparency = match look.transparent {
            true => look.transparency,
            false => 0.,
        };
        let value = 1. - transparency / MAX_TRANSPARENCY;
        let percent = ((1. - transparency) * 100.).round() as i64;

        let control = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().w(theme.metrics.cover).child(
                    Scrubber::new(&self.opacity, value)
                        .colors(theme.progress_bar, theme.muted, theme.foreground)
                        .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                            let transparency = (1. - *fraction) * MAX_TRANSPARENCY;
                            let transparent = transparency > 0.;
                            this.settings.update(cx, |settings, cx| {
                                settings.set_transparent(transparent, cx);
                                settings.set_transparency(transparency, cx);
                            });
                            Theme::set(
                                Look {
                                    transparent,
                                    transparency,
                                    ..look
                                },
                                &overrides,
                                cx,
                            );
                        })),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w(theme.metrics.control * 1.5)
                    .whitespace_nowrap()
                    .text_right()
                    .child(t!("settings-opacity-value", percent = percent)),
            );

        self.row(
            t!("settings-opacity"),
            t!("settings-opacity-detail"),
            muted,
            small,
            control.into_any_element(),
        )
    }

    fn adaptive_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).adaptive_theme();
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides().clone();

        self.row(
            t!("settings-adaptive"),
            t!("settings-adaptive-detail"),
            muted,
            small,
            Switch::new("adaptive-theme", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let adaptive = !on;
                    let kind = match adaptive
                        && !matches!(
                            look.kind,
                            ThemeKind::System | ThemeKind::Dark | ThemeKind::Light
                        ) {
                        true => ThemeKind::Dark,
                        false => look.kind,
                    };
                    this.settings.update(cx, |settings, cx| {
                        settings.set_adaptive_theme(adaptive, cx);
                        if kind != look.kind {
                            settings.set_theme(kind.id(), cx);
                        }
                    });
                    if kind != look.kind {
                        Theme::fade(Look { kind, ..look }, &overrides, cx);
                    }
                }))
                .into_any_element(),
        )
    }

    /// The fullscreen background sampled from the cover. It carries the cover's hues itself,
    /// so fullscreen tints from the artwork with the adaptive theme off as well.
    fn ambient_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).ambient();

        self.row(
            t!("settings-ambient"),
            t!("settings-ambient-detail"),
            muted,
            small,
            Switch::new("ambient", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_ambient(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn ambient_motion_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).ambient_motion();

        self.row(
            t!("settings-ambient-motion"),
            t!("settings-ambient-motion-detail"),
            muted,
            small,
            Switch::new("ambient-motion", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_ambient_motion(!on, cx));
                }))
                .into_any_element(),
        )
    }

    /// How fullscreen stages the cover: bare, haloed in drifting particles,
    /// riding a turning record as its label, or slid out of its sleeve. One
    /// choice covers what used to be the starry and the vinyl switches.
    fn stage_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).stage_style();

        let picker = Picker::new(STAGE_STYLE, &self.popovers, chosen.label())
            .width(Picker::NARROW)
            .items(StageStyle::ALL.map(|style| {
                MenuItem::new(style.id(), style.label())
                    .selected(style == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_stage_style(style, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-stage"),
            t!("settings-stage-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    /// How many particles drift off the cover, stepped like the lyrics sizes.
    fn particles_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let count = self.settings.read(cx).particles();

        let step = move |suffix: &'static str, label: &'static str, delta: i64| {
            let wanted = (count as i64 + delta).clamp(0, MAX_PARTICLES as i64) as usize;

            Button::new(SharedString::from(format!("particles-{suffix}")))
                .label(label)
                .small()
                .outline()
                .disabled(wanted == count)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_particles(wanted, cx));
                    cx.notify();
                }))
        };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("fewer", "\u{2212}", -32))
            .child(div().child(t!("settings-particles-value", count = count as i64)))
            .child(step("more", "+", 32));

        self.row(
            t!("settings-particles"),
            t!("settings-particles-detail"),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    /// How the spectrum is drawn behind the fullscreen artwork, off included.
    fn visualizer_style_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).visualizer_style();

        let picker = Picker::new(VISUALIZER_STYLE, &self.popovers, chosen.label())
            .width(Picker::NARROW)
            .items(VisualizerStyle::ALL.map(|style| {
                MenuItem::new(style.id(), style.label())
                    .selected(style == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_visualizer_style(style, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-visualizer"),
            t!("settings-visualizer-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn fullscreen_controls_autohide_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).fullscreen_controls_autohide();

        let picker = Picker::new(
            FULLSCREEN_CONTROLS_AUTOHIDE,
            &self.popovers,
            i18n::lookup(chosen.key(), None),
        )
        .width(Picker::NARROW)
        .items(FullscreenControlsAutohide::ALL.map(|fca| {
            MenuItem::new(fca.id(), i18n::lookup(fca.key(), None))
                .selected(fca == chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_fullscreen_controls_autohide(fca, cx)
                    });
                    cx.notify();
                }))
        }));

        self.row(
            t!("settings-fullscreen-controls-autohide"),
            t!("settings-fullscreen-controls-autohide-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn motion_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).stillness();

        let picker = Picker::new(MOTION, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Stillness::ALL.into_iter().map(|stillness| {
                MenuItem::new(stillness.id(), stillness.label())
                    .selected(current == stillness)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_stillness(stillness, cx);
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-motion"),
            t!("settings-motion-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn pace_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).pace();

        let picker = Picker::new(PACE, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Pace::ALL.into_iter().map(|pace| {
                MenuItem::new(pace.id(), pace.label())
                    .selected(current == pace)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_pace(pace, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-pace"),
            t!("settings-pace-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn saver_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let current = self.settings.read(cx).saver();

        let picker = Picker::new(SAVER, &self.popovers, current.label())
            .width(Picker::NARROW)
            .items(Saver::ALL.into_iter().map(|saver| {
                MenuItem::new(saver.id(), saver.label())
                    .selected(current == saver)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_saver(saver, cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-saver"),
            t!("settings-saver-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn playback_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.playback.read(cx).normalisation();

        self.row(
            t!("settings-normalisation"),
            t!("settings-normalisation-detail"),
            muted,
            small,
            Switch::new("normalisation", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.set_normalisation(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn adaptive_menu_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).adaptive_menu();

        self.row(
            t!("settings-adaptive-menu"),
            t!("settings-adaptive-menu-detail"),
            muted,
            small,
            Switch::new("adaptive-menu", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_adaptive_menu(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn tray_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).close_to_tray();

        self.row(
            t!("settings-close-to-tray"),
            t!("settings-close-to-tray-detail"),
            muted,
            small,
            Switch::new("close-to-tray", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_close_to_tray(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn gapless_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.playback.read(cx).gapless();

        self.row(
            t!("settings-gapless"),
            t!("settings-gapless-detail"),
            muted,
            small,
            Switch::new("gapless", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.set_gapless(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn equalizer_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.playback.read(cx).equalizer();

        self.row(
            t!("settings-equalizer"),
            t!("settings-equalizer-detail"),
            muted,
            small,
            Switch::new("equalizer", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.set_equalizer(!on, cx));
                }))
                .into_any_element(),
        )
    }

    /// The preset picker. It reads the current curve back, so a band moved by hand shows as
    /// Custom and a curve that happens to match a preset shows that preset's name.
    fn equalizer_preset_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let gains = self.playback.read(cx).equalizer_gains();
        let chosen = Preset::matching(&gains);
        let label = match chosen {
            Some(preset) => i18n::lookup(preset_key(preset), None),
            None => t!("settings-equalizer-custom"),
        };

        let picker = Picker::new(EQUALIZER_PRESETS, &self.popovers, label)
            .width(Picker::NARROW)
            .items(Preset::ALL.map(|preset| {
                MenuItem::new(preset.id(), i18n::lookup(preset_key(preset), None))
                    .selected(chosen == Some(preset))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.playback.update(cx, |playback, cx| {
                            playback.set_equalizer_gains(&preset.gains(), cx)
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-equalizer-preset"),
            t!("settings-equalizer-preset-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    /// One vertical slider per band with its gain above and its frequency below. Shown only
    /// while the equalizer is on. Dragging writes straight through to the engines, so the
    /// change is heard as it is made.
    fn equalizer_bands_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let gains = self.playback.read(cx).equalizer_gains();
        let span = equalizer::MAX_GAIN - equalizer::MIN_GAIN;

        let columns = self.bands.iter().enumerate().map(|(band, state)| {
            let gain = gains[band];
            let fraction = (gain - equalizer::MIN_GAIN) / span;
            let slider = Scrubber::new(state, fraction)
                .vertical()
                .colors(theme.progress_bar, theme.muted, theme.foreground)
                .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                    let raw = equalizer::MIN_GAIN + fraction * span;
                    let gain = (raw / EQUALIZER_STEP).round() * EQUALIZER_STEP;
                    this.playback.update(cx, |playback, cx| {
                        playback.set_equalizer_gain(band, gain, cx)
                    });
                }));

            div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(theme.text(Text::Tiny))
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .line_height(relative(LEADING))
                        .child(t!("settings-equalizer-decibels", db = decibels(gain))),
                )
                .child(
                    div()
                        .h(theme.metrics.cover)
                        .flex()
                        .justify_center()
                        .child(slider),
                )
                .child(
                    div()
                        .text_size(theme.text(Text::Tiny))
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .line_height(relative(LEADING))
                        .child(hertz(equalizer::FREQUENCIES[band])),
                )
        });

        Setting {
            title: t!("settings-group-equalizer"),
            detail: SharedString::default(),
            element: div()
                .flex()
                .w_full()
                .py_3()
                .children(columns)
                .into_any_element(),
        }
    }

    fn sleep_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).sleep_timer();

        let picker = Picker::plain(SLEEP, &self.popovers, t!("settings-sleep-configure"))
            .width(Picker::REGULAR)
            .selected(self.playback.read(cx).sleep().is_some())
            .items(match self.popovers.shows(SLEEP) {
                true => vec![self.sleep_dial(cx)],
                false => Vec::new(),
            });
        let action = div()
            .flex()
            .items_center()
            .gap_2()
            .when(on, |this| this.child(picker))
            .child(
                Switch::new("sleep-timer", on).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_sleep_timer(!on, cx));
                    if on {
                        this.popovers.close();
                        this.playback
                            .update(cx, |playback, cx| playback.set_sleep(None, cx));
                    }
                })),
            );

        self.row(
            t!("settings-sleep"),
            t!("settings-sleep-detail"),
            muted,
            small,
            action.into_any_element(),
        )
    }

    /// The slider that arms the timer: off at the left, end of track at the right, minutes in
    /// between, with the quarter hours widened.
    fn sleep_dial(&self, cx: &mut Context<Self>) -> MenuItem {
        let theme = *cx.theme();
        let current = self
            .pending_sleep
            .unwrap_or_else(|| self.playback.read(cx).sleep());

        let dial = div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .py_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_size(theme.text(Text::Small))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(t!("settings-sleep")),
                    )
                    .child(sleep_label(current)),
            )
            .child(
                Scrubber::new(&self.sleep, sleep_slot(current) as f32 / SLEEP_LAST as f32)
                    .colors(
                        theme.progress_bar,
                        theme.muted_foreground.opacity(0.3),
                        theme.foreground,
                    )
                    .on_move(cx.listener(|this, fraction: &f32, _, cx| {
                        this.pending_sleep = Some(sleep_at_fraction(*fraction));
                        cx.notify();
                    }))
                    .on_release(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                        let Some(sleep) = this.pending_sleep.take() else {
                            return;
                        };
                        this.playback
                            .update(cx, |playback, cx| playback.set_sleep(sleep, cx));
                    })),
            );

        MenuItem::new("sleep-dial", "").content(dial)
    }

    /// The Widevine module row, which only appears while the current provider is one whose
    /// tracks need the module and this build has a host for one. Sonora uses a browser's copy
    /// when one is here and otherwise offers Google's download, so the row says where that
    /// stands and offers the download by hand when the user said no or nothing asked yet.
    fn widevine_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let state = self.drm.read(cx).state().clone();
        let (detail, note) = widevine_copy(&state);
        let offerable = matches!(state, CdmState::Declined | CdmState::Missing);
        let removable = matches!(state, CdmState::Ready(Origin::Fetched));

        self.row(
            t!("settings-widevine"),
            i18n::lookup(detail, None),
            muted,
            small,
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_color(muted)
                .text_size(small)
                .child(i18n::lookup(note, None))
                .when(offerable, |row| {
                    row.child(
                        Button::new("fetch-widevine")
                            .label(t!("settings-widevine-fetch"))
                            .small()
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.drm.update(cx, |drm, cx| drm.download(cx));
                            })),
                    )
                })
                .when(removable, |row| {
                    row.child(
                        Button::new("uninstall-widevine")
                            .label(t!("settings-widevine-uninstall"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let drm = this.drm.clone();
                                Confirm::ask(
                                    Kind::Widevine,
                                    move |cx| drm.update(cx, |drm, cx| drm.uninstall(cx)),
                                    cx,
                                );
                            })),
                    )
                })
                .into_any_element(),
        )
    }

    fn updates_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).check_updates();

        self.row(
            t!("settings-check-updates"),
            t!("settings-check-updates-detail"),
            muted,
            small,
            Switch::new("check-updates", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_check_updates(!on, cx));
                }))
                .into_any_element(),
        )
    }

    /// The About row that opens the current log file in whatever the system reads text with.
    /// The button is disabled when the platform names no state or cache folder to log into.
    fn log_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let path = state::log_file();

        self.row(
            t!("settings-log"),
            t!("settings-log-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            Button::new("open-log")
                .label(t!("settings-log-open"))
                .small()
                .outline()
                .disabled(path.is_none())
                .on_click(move |_, _, _| {
                    let Some(path) = &path else { return };
                    if let Err(error) = open_path(path) {
                        log::warn!("settings: cannot open {}: {error}", path.display());
                    }
                })
                .into_any_element(),
        )
    }

    fn panel_lyrics_size_row(&self, cx: &mut Context<Self>) -> Setting {
        let scale = self.settings.read(cx).panel_lyrics_scale();

        self.lyrics_size_row(
            "panel-lyrics-size",
            "settings-panel-lyrics-size",
            "settings-panel-lyrics-size-detail",
            scale,
            |settings, scale, cx| settings.set_panel_lyrics_scale(scale, cx),
            cx,
        )
    }

    fn fullscreen_lyrics_size_row(&self, cx: &mut Context<Self>) -> Setting {
        let scale = self.settings.read(cx).fullscreen_lyrics_scale();

        self.lyrics_size_row(
            "fullscreen-lyrics-size",
            "settings-fullscreen-lyrics-size",
            "settings-fullscreen-lyrics-size-detail",
            scale,
            |settings, scale, cx| settings.set_fullscreen_lyrics_scale(scale, cx),
            cx,
        )
    }

    fn lyrics_size_row(
        &self,
        id: &'static str,
        title: &'static str,
        detail: &'static str,
        scale: f32,
        apply: fn(&mut AppSettings, f32, &mut Context<AppSettings>),
        cx: &mut Context<Self>,
    ) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let step = move |suffix: &'static str, label: &'static str, delta: f32| {
            let wanted = (scale + delta).clamp(MIN_LYRICS_SCALE, MAX_LYRICS_SCALE);

            Button::new(SharedString::from(format!("{id}-{suffix}")))
                .label(label)
                .small()
                .outline()
                .disabled(wanted == scale)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| apply(settings, wanted, cx));
                    cx.notify();
                }))
        };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("smaller", "\u{2212}", -0.1))
            .child(div().child(t!(
                "settings-lyrics-size-value",
                size = (scale * 100.).round() as i64
            )))
            .child(step("larger", "+", 0.1));

        self.row(
            i18n::lookup(title, None),
            i18n::lookup(detail, None),
            muted,
            small,
            actions.into_any_element(),
        )
    }

    fn discord_slots(&self, cx: &App) -> Vec<Slot> {
        let mut slots = vec![Slot::Title("settings-group-discord"), Slot::Discord];
        if self.settings.read(cx).discord_presence() {
            slots.push(Slot::DiscordName);
            slots.push(Slot::DiscordShowPaused);
            slots.push(Slot::DiscordBadge);
            slots.push(Slot::DiscordAnonymous);
            slots.push(Slot::DiscordButtons);
        }
        slots
    }

    fn discord_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).discord_presence();

        self.row(
            t!("settings-discord"),
            t!("settings-discord-detail"),
            muted,
            small,
            Switch::new("discord", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_presence(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn discord_name_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let chosen = self.settings.read(cx).discord_name();

        let picker = Picker::new(
            DISCORD_NAME,
            &self.popovers,
            i18n::lookup(chosen.key(), None),
        )
        .width(Picker::NARROW)
        .items(DiscordName::ALL.map(|name| {
            MenuItem::new(name.id(), i18n::lookup(name.key(), None))
                .selected(name == chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_name(name, cx));
                    cx.notify();
                }))
        }));

        self.row(
            t!("settings-discord-name"),
            t!("settings-discord-name-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn discord_show_paused_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).discord_show_paused();

        self.row(
            t!("settings-discord-show-paused"),
            t!("settings-discord-show-paused-detail"),
            muted,
            small,
            Switch::new("discord-show-paused", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_show_paused(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn discord_badge_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).discord_badge();

        self.row(
            t!("settings-discord-badge"),
            t!("settings-discord-badge-detail"),
            muted,
            small,
            Switch::new("discord-badge", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_badge(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn discord_anonymous_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).discord_without_details();

        self.row(
            t!("settings-discord-anonymous"),
            t!("settings-discord-anonymous-detail"),
            muted,
            small,
            Switch::new("discord-anonymous", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_discord_without_details(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    fn discord_buttons_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let settings = self.settings.read(cx);
        let sonora = settings.discord_sonora_button();
        let provider = settings.discord_provider_button();

        let picker = Picker::new(
            DISCORD_BUTTONS,
            &self.popovers,
            t!("settings-discord-buttons-pick"),
        )
        .width(Picker::NARROW)
        .sticky()
        .item(
            MenuItem::new(
                "discord-button-provider",
                t!("settings-discord-name-provider"),
            )
            .selected(provider)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_discord_provider_button(!provider, cx)
                });
                cx.notify();
            })),
        )
        .item(
            MenuItem::new("discord-button-sonora", t!("settings-discord-name-sonora"))
                .selected(sonora)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_discord_sonora_button(!sonora, cx)
                    });
                    cx.notify();
                })),
        );

        self.row(
            t!("settings-discord-buttons"),
            t!("settings-discord-buttons-detail"),
            muted,
            small,
            picker.into_any_element(),
        )
    }

    fn lyrics_for_local_files_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).lyrics_for_local_files();

        self.row(
            t!("settings-lyrics-for-local-files"),
            t!("settings-lyrics-for-local-files-detail"),
            muted,
            small,
            Switch::new("lyrics-for-local-files", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_lyrics_for_local_files(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    fn lyrics_providers_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let settings = self.settings.read(cx);
        let providers = [
            (music::lyrics::LOCAL, "settings-lyrics-provider-local"),
            ("Spotify", "settings-lyrics-provider-spotify"),
            ("YouTube Music", "settings-lyrics-provider-youtube"),
            ("Apple Music", "settings-lyrics-provider-apple-music"),
            ("Musixmatch", "settings-lyrics-provider-musixmatch"),
            ("LrcLib", "settings-lyrics-provider-lrclib"),
            ("Kugou", "settings-lyrics-provider-kugou"),
            ("NetEase", "settings-lyrics-provider-netease"),
        ];
        let count = providers
            .iter()
            .filter(|(provider, _)| settings.lyrics_provider_enabled(provider))
            .count();
        let picker = Picker::new(
            "lyrics-providers",
            &self.popovers,
            t!("settings-lyrics-providers-selected", count = count),
        )
        .sticky()
        .items(providers.map(|(provider, label)| {
            MenuItem::new(label, i18n::lookup(label, None))
                .selected(settings.lyrics_provider_enabled(provider))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        let enabled = settings.lyrics_provider_enabled(provider);
                        settings.set_lyrics_provider(provider, !enabled, cx);
                    });
                }))
        }));
        self.row(
            t!("settings-lyrics-providers"),
            t!("settings-lyrics-providers-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            picker.into_any_element(),
        )
    }

    fn prefer_local_lyrics_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).prefer_local_lyrics();

        self.row(
            t!("settings-prefer-local-lyrics"),
            t!("settings-prefer-local-lyrics-detail"),
            muted,
            small,
            Switch::new("prefer-local-lyrics", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_prefer_local_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn karaoke_lyrics_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).karaoke_lyrics();

        self.row(
            t!("settings-karaoke-lyrics"),
            t!("settings-karaoke-lyrics-detail"),
            muted,
            small,
            Switch::new("karaoke-lyrics", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_karaoke_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn blur_lyrics_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let on = self.settings.read(cx).blur_lyrics();

        self.row(
            t!("settings-blur-lyrics"),
            t!("settings-blur-lyrics-detail"),
            muted,
            small,
            Switch::new("blur-lyrics", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur_lyrics(!on, cx));
                }))
                .into_any_element(),
        )
    }

    fn romanized_lyrics_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let settings = self.settings.read(cx);
        let on = settings.romanized_lyrics();
        let scripts = settings.romanization_scripts();
        let picker = Picker::new(
            "romanization-scripts",
            &self.popovers,
            t!("settings-romanization-writing-systems"),
        )
        .width(Picker::REGULAR)
        .sticky()
        .items(WritingSystem::ALL.map(|writing_system| {
            let (id, label) = romanization_script_copy(writing_system);
            let selected = scripts.contains(writing_system);
            MenuItem::new(id, i18n::lookup(label, None))
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_romanization_script(writing_system, !selected, cx);
                    });
                }))
        }));
        let action = div()
            .flex()
            .items_center()
            .gap_2()
            .when(on, |this| this.child(picker))
            .child(Switch::new("romanized-lyrics", on).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_romanized_lyrics(!on, cx);
                    });
                },
            )));

        self.row(
            t!("settings-romanized-lyrics"),
            t!("settings-romanized-lyrics-detail"),
            muted,
            small,
            action.into_any_element(),
        )
    }

    fn local_folder_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);
        let paths = self.session.read(cx).local_paths();

        let add = local::choose_button("add-local-folder")
            .label(t!("settings-add-folder"))
            .icon("icons/plus.svg")
            .small()
            .outline();

        let scan = Scan::global(cx).read(cx);
        let scanning = scan.progress();
        // Until the walk is over there is no total to be a fraction of, and on a network share
        // that is the longest part, so it says so rather than showing nothing.
        let note = match (scanning, scan.done()) {
            (Some(progress), _) => Some(match progress.percent() {
                Some(percent) => t!("settings-scan-progress", percent = percent),
                None => t!("settings-scan-walking"),
            }),
            (None, Some(took)) => Some(t!("settings-scan-done", seconds = text::lapsed(took))),
            (None, None) => None,
        };
        let note = note.map(|text| {
            div()
                .text_color(muted)
                .text_size(small)
                .child(text)
                .into_any_element()
        });

        let rescan = (!paths.is_empty()).then(|| {
            Button::new("rescan-local-folder")
                .label(t!("settings-rescan"))
                .small()
                .ghost()
                .disabled(scanning.is_some())
                .on_click(cx.listener(|this, _, _, cx| this.rescan_local_folder(cx)))
        });

        let Setting {
            title,
            detail,
            element: header,
        } = self.row(
            t!("settings-local-folder"),
            match paths.is_empty() {
                true => t!("settings-local-folder-empty"),
                false => SharedString::default(),
            },
            muted,
            small,
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(note)
                .child(add)
                .children(rescan)
                .into_any_element(),
        );

        let element =
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(header)
                .children((!paths.is_empty()).then(|| {
                    div().flex().flex_col().gap_1().pb_2().children(
                        paths.into_iter().enumerate().map(|(index, path)| {
                            Self::local_folder_item(index, path, muted, small, &mut *cx)
                        }),
                    )
                }))
                .into_any_element();

        Setting {
            title,
            detail,
            element,
        }
    }

    fn local_folder_item(
        index: usize,
        path: String,
        muted: gpui::Hsla,
        small: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("local-folder-item", index))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(muted)
                    .text_size(small)
                    .child(SharedString::from(path.clone())),
            )
            .child(
                Button::new(("remove-local-folder", index))
                    .ghost()
                    .small()
                    .icon("icons/x.svg")
                    .tooltip("settings-remove-folder")
                    .tint(muted)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_local_folder(path.clone(), cx)
                    })),
            )
            .into_any_element()
    }

    fn rescan_local_folder(&mut self, cx: &mut Context<Self>) {
        Scan::global(cx).update(cx, |scan, _| scan.asked());
        Sonora::global(cx)
            .library
            .clone()
            .update(cx, |library, cx| library.rescan_local(true, cx));
    }

    fn remove_local_folder(&mut self, path: String, cx: &mut Context<Self>) {
        Sonora::global(cx)
            .library
            .clone()
            .update(cx, |library, cx| {
                library.remove_local_folder(PathBuf::from(path), cx)
            });
    }

    /// One row per scrobbling service, in the order `music::scrobble` lists them.
    fn scrobble_slots(&self, cx: &App) -> Vec<Slot> {
        let services = self.scrobbling.read(cx).rows().len();
        (0..services).map(Slot::Scrobble).collect()
    }

    fn scrobble_row(&self, index: usize, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let row = &self.scrobbling.read(cx).rows()[index];
        let service = row.id();
        let link = row.link();
        let linked = row.linked();
        let linking = row.linking();
        let enabled = row.enabled();
        let title = i18n::lookup(&format!("settings-{service}"), None);
        let detail = match row.state() {
            ScrobbleState::Off => t!("settings-scrobble-off"),
            ScrobbleState::Linking => t!("settings-scrobble-waiting"),
            ScrobbleState::On(name) => match name.is_empty() {
                true => t!("settings-scrobble-on"),
                false => t!("settings-scrobble-as", name = name.as_ref()),
            },
            ScrobbleState::Failed(key) => i18n::lookup(key, None),
        };

        let action = match linked {
            true => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Switch::new(
                        SharedString::from(format!("scrobble-on-{service}")),
                        enabled,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.scrobbling.update(cx, |scrobbling, cx| {
                            scrobbling.set_enabled(service, !enabled, cx)
                        });
                    })),
                )
                .child(
                    Button::new(SharedString::from(format!("scrobble-unlink-{service}")))
                        .label(t!("settings-scrobble-disconnect"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.scrobbling
                                .update(cx, |scrobbling, cx| scrobbling.disconnect(service, cx));
                        })),
                )
                .into_any_element(),
            false => match linking {
                true => Button::new(SharedString::from(format!("scrobble-cancel-{service}")))
                    .label(t!("common-cancel"))
                    .small()
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.scrobbling
                            .update(cx, |scrobbling, cx| scrobbling.cancel(cx));
                    }))
                    .into_any_element(),
                false => Button::new(SharedString::from(format!("scrobble-link-{service}")))
                    .label(t!("settings-scrobble-connect"))
                    .small()
                    .outline()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_scrobble(service, link, cx)),
                    )
                    .into_any_element(),
            },
        };

        self.row(title, detail, muted, small, action)
    }

    /// Starts a link. A service with nothing to type goes straight to the browser; the rest put
    /// up the dialog with whatever was stored last time already in the fields.
    fn open_scrobble(&mut self, service: &'static str, link: Link, cx: &mut Context<Self>) {
        let account = self.settings.read(cx).account(service);
        let (first, second) = match link {
            Link::Browser => {
                return self.scrobbling.update(cx, |scrobbling, cx| {
                    scrobbling.connect(service, Secret::None, cx)
                });
            }
            Link::Keys => (
                Field {
                    hint: "settings-scrobble-key",
                    value: account.key,
                    masked: false,
                },
                Some(Field {
                    hint: "settings-scrobble-secret",
                    value: account.secret,
                    masked: true,
                }),
            ),
            Link::Token => (
                Field {
                    hint: "settings-scrobble-token",
                    value: account.session,
                    masked: true,
                },
                None,
            ),
            Link::Server => (
                Field {
                    hint: "settings-scrobble-server",
                    value: account.server,
                    masked: false,
                },
                Some(Field {
                    hint: "settings-scrobble-key",
                    value: account.session,
                    masked: true,
                }),
            ),
        };

        self.scrobble_first.update(cx, |input, cx| {
            input.set_hint(first.hint, cx);
            input.set_masked(first.masked, cx);
            input.set_text(first.value, cx);
        });
        if let Some(second) = second {
            self.scrobble_second.update(cx, |input, cx| {
                input.set_hint(second.hint, cx);
                input.set_masked(second.masked, cx);
                input.set_text(second.value, cx);
            });
        }
        self.scrobble_prompt = Some(service);
        cx.notify();
    }

    fn link_scrobble(&mut self, service: &'static str, link: Link, cx: &mut Context<Self>) {
        let first = self.scrobble_first.read(cx).text().to_string();
        let second = self.scrobble_second.read(cx).text().to_string();
        let secret = match link {
            Link::Browser => Secret::None,
            Link::Keys => Secret::Keys {
                key: first,
                secret: second,
            },
            Link::Token => Secret::Token(first),
            Link::Server => Secret::Server {
                url: first,
                key: second,
            },
        };

        self.scrobble_prompt = None;
        self.scrobbling
            .update(cx, |scrobbling, cx| scrobbling.connect(service, secret, cx));
    }

    fn scrobble_modal(&self, service: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self
            .scrobbling
            .read(cx)
            .rows()
            .iter()
            .find(|row| row.id() == service);
        let link = row.map(|row| row.link()).unwrap_or(Link::Token);
        let signup = row.and_then(|row| row.signup());
        let title = i18n::lookup(&format!("settings-{service}"), None);
        let request = match link {
            Link::Token => "settings-scrobble-token-request",
            _ => "settings-scrobble-request",
        };

        Modal::new(
            "settings-scrobble-prompt",
            t!("settings-scrobble-title", service = title.as_ref()),
        )
        .w(px(560.))
        .detail(i18n::lookup(&format!("settings-{service}-detail"), None))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(self.scrobble_first.clone())
                .when(link != Link::Token, |this| {
                    this.child(self.scrobble_second.clone())
                }),
        )
        .when_some(signup, |this, url| {
            this.action(
                Button::new("settings-scrobble-request")
                    .ghost()
                    .label(i18n::lookup(request, None))
                    .on_click(move |_, _, cx| cx.open_url(url)),
            )
        })
        .action(
            Button::new("settings-scrobble-cancel")
                .ghost()
                .label(t!("common-cancel"))
                .on_click(cx.listener(|this, _, _, cx| this.close_scrobble(cx))),
        )
        .action(
            Button::new("settings-scrobble-submit")
                .primary()
                .label(t!("settings-scrobble-connect"))
                .on_click(cx.listener(move |this, _, _, cx| this.link_scrobble(service, link, cx))),
        )
        .on_dismiss(cx.listener(|this, _, _, cx| this.close_scrobble(cx)))
    }

    fn close_scrobble(&mut self, cx: &mut Context<Self>) {
        self.scrobble_prompt = None;
        cx.notify();
    }

    /// The provider accounts as plain data, shared by the row, its height and its search
    /// entry, so the three never disagree about what is shown.
    fn providers(&self, cx: &App) -> Vec<Account> {
        let session = self.session.read(cx);
        let signed_out = matches!(session.state(), SessionState::SignedOut);
        let guest = !session.authenticated();
        let waiting = match session.state() {
            SessionState::Authorizing(prompt) => !matches!(prompt, Some(SignInPrompt::Accounts(_))),
            _ => false,
        };
        let loading = session.is_pending();
        let chosen = self.chosen.filter(|_| loading);
        session
            .providers()
            .map(|info| Account {
                slug: info.slug,
                name: info.name,
                options: info.options,
                web_sign_in: info.web_sign_in,
                stored: info.stored && !info.guest,
                // a session on its way up reports no account yet, so while it loads the card
                // it belongs to keeps the radio rather than leaving the list blank
                active: match chosen {
                    Some(picked) => picked == info.slug,
                    None if loading => info.active && !info.guest,
                    None => info.active && !signed_out && !guest,
                },
                cancel: waiting && info.pending,
                error: info.error,
            })
            .collect()
    }

    /// The guest card, when a provider offers an anonymous session at all. A guest run leaves
    /// that provider's own card connected to nothing, so only this card reports it.
    fn guest(&self, cx: &App) -> Option<Guest> {
        let session = self.session.read(cx);
        let signed_out = matches!(session.state(), SessionState::SignedOut);
        let info = session.providers().find(|info| {
            info.options
                .iter()
                .any(|option| matches!(option, SignIn::Anonymous))
        })?;
        let loading = session.is_pending();
        let active = match self.chosen.filter(|_| loading) {
            Some(picked) => picked == GUEST,
            None if loading => info.active && info.guest,
            None => info.active && !signed_out && !session.authenticated(),
        };
        Some(Guest {
            slug: info.slug,
            stored: info.guest,
            active,
        })
    }

    /// The words the accounts row answers a search with: every provider name, and the guest
    /// entry when one is shown.
    fn account_words(&self, cx: &App) -> String {
        let mut names: Vec<String> = self
            .providers(cx)
            .iter()
            .map(|account| account.name.to_string())
            .collect();
        if self.guest(cx).is_some() {
            names.push(t!("login-guest-title").to_string());
        }
        names.join(" ")
    }

    fn accounts_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();
        let pending = self.session.read(cx).is_pending();
        let names = self.account_words(cx);
        let mut cards = Vec::new();
        for account in self.providers(cx) {
            cards.push(self.account_card(account, pending, cx).into_any_element());
        }
        if let Some(guest) = self.guest(cx) {
            cards.push(self.guest_card(guest, pending, cx).into_any_element());
        }
        let title = t!("settings-accounts");
        let detail = t!("settings-accounts-detail");

        let element = div()
            .flex()
            .flex_col()
            .gap_3()
            .py_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .line_height(relative(LEADING))
                            .child(title.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .line_height(relative(LEADING))
                            .text_color(theme.muted_foreground)
                            .text_size(theme.text(Text::Small))
                            .child(detail.clone()),
                    ),
            )
            .children(cards)
            .into_any_element();

        // the provider names are words too, so "spotify" finds the accounts
        Setting {
            title,
            detail: format!("{detail} {names}").into(),
            element,
        }
    }

    fn account_card(
        &self,
        account: Account,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Account {
            slug,
            name,
            options,
            web_sign_in,
            stored,
            active,
            cancel,
            error,
        } = account;
        let status = Some(match (active, stored) {
            (true, _) => t!("settings-provider-current"),
            (false, true) => t!("settings-provider-connected"),
            (false, false) => t!("settings-provider-none"),
        });
        let press: Option<Press> = match (cancel, stored, active) {
            (true, ..) | (_, true, true) => None,
            (false, true, false) => Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = Some(slug);
                this.session
                    .update(cx, |session, cx| session.switch(slug, cx));
            }))),
            (false, false, _) => Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = None;
                this.start_sign_in(slug, name, &options, web_sign_in, cx);
            }))),
        };

        card(
            AccountCard {
                id: SharedString::from(format!("account-{slug}")),
                logo: crate::shared::provider_logo(slug),
                name: name.into(),
                status,
                selected: active,
                trailing: self.trailing(slug, slug, stored, cancel, pending, cx),
                error,
                press: press.filter(|_| !pending),
            },
            cx,
        )
    }

    /// The guest card. Its buttons drive the provider the anonymous session belongs to, so
    /// they carry ids of their own rather than that provider's, which has a card too.
    fn guest_card(&self, guest: Guest, pending: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let Guest {
            slug,
            stored,
            active,
        } = guest;
        let status = active.then(|| t!("settings-provider-guest"));
        let press: Option<Press> = match (stored, active) {
            (_, true) => None,
            (true, false) => Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = Some(GUEST);
                this.session
                    .update(cx, |session, cx| session.switch(slug, cx));
            }))),
            (false, false) => Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.chosen = Some(GUEST);
                this.session.update(cx, |session, cx| {
                    session.sign_in(slug, SignIn::Anonymous, cx)
                });
            }))),
        };

        card(
            AccountCard {
                id: SharedString::from("account-guest"),
                logo: "icons/hat-glasses.svg",
                name: t!("login-guest-title"),
                status,
                selected: active,
                trailing: self.trailing(GUEST, slug, active, false, pending, cx),
                error: None,
                press: press.filter(|_| !pending),
            },
            cx,
        )
    }

    /// What sits at the right end of a card: the one button it has while an account is
    /// connected, the same shape carrying a cancel while a sign-in is in flight, and an arrow
    /// for a card whose whole surface starts a sign-in.
    fn trailing(
        &self,
        id: &'static str,
        slug: &'static str,
        stored: bool,
        cancel: bool,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        if cancel {
            return Button::new(SharedString::from(format!("cancel-{id}")))
                .icon("icons/x.svg")
                .tooltip("common-cancel")
                .w(theme.metrics.control)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| this.abandon(cx)))
                .into_any_element();
        }
        if !stored {
            return div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(theme.metrics.control)
                .h(theme.metrics.control)
                .child(
                    svg()
                        .path(icons::path("icons/chevron-right.svg"))
                        .size(ARROW)
                        .flex_none()
                        .text_color(theme.muted_foreground),
                )
                .into_any_element();
        }

        Button::new(SharedString::from(format!("sign-out-{id}")))
            .icon("icons/log-out.svg")
            .tooltip("settings-sign-out")
            .w(theme.metrics.control)
            .disabled(pending)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.chosen = None;
                this.session
                    .update(cx, |session, cx| session.forget(slug, cx));
            }))
            .into_any_element()
    }

    /// Starts a sign-in the way the provider asks for it. One method goes straight through,
    /// a server asks for its address and credentials, and anything with a choice to make puts
    /// the choice up in a dialog.
    fn start_sign_in(
        &mut self,
        slug: &'static str,
        provider: &'static str,
        options: &[SignIn],
        web_sign_in: bool,
        cx: &mut Context<Self>,
    ) {
        let methods: Vec<&SignIn> = options
            .iter()
            .filter(|option| offered(option, false))
            .collect();
        let buttons: usize = methods
            .iter()
            .map(|method| method_count(method, web_sign_in))
            .sum();
        match methods.as_slice() {
            [SignIn::Credentials { .. }] => self.open_credentials(slug, cx),
            [SignIn::Secret] if buttons == 1 => self.start_manual(slug, provider, cx),
            [method] if buttons == 1 => {
                let method = (*method).clone();
                self.session
                    .update(cx, |session, cx| session.sign_in(slug, method, cx));
            }
            [] => {}
            _ => {
                self.sign_in_for = Some((slug, provider));
                cx.notify();
            }
        }
    }

    /// Closes the dialog that is up, topmost first, the way clicking outside it does. A
    /// prompt a sign-in raised takes the sign-in down with it.
    fn escape(&mut self, cx: &mut Context<Self>) {
        if self.credentials_for.is_some() {
            return self.abandon_credentials(cx);
        }
        if self.scrobble_prompt.is_some() {
            return self.close_scrobble(cx);
        }
        let prompted = matches!(
            self.session.read(cx).state(),
            SessionState::Authorizing(Some(_))
        );
        if prompted {
            return self.abandon(cx);
        }
        if self.sign_in_for.is_some() {
            self.close_sign_in(cx);
        }
    }

    fn close_sign_in(&mut self, cx: &mut Context<Self>) {
        self.sign_in_for = None;
        self.sign_in_running = false;
        cx.notify();
    }

    /// The choice of sign-in methods, for a provider that has more than one. The buttons are
    /// the same ones the card used to carry.
    fn sign_in_prompt(
        &self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let pending = self.session.read(cx).is_pending();
        let options = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug)
            .map(|info| info.options)
            .unwrap_or_default();
        let web_sign_in = self
            .session
            .read(cx)
            .providers()
            .find(|info| info.slug == slug)
            .is_some_and(|info| info.web_sign_in);
        let buttons = options
            .into_iter()
            .filter(|option| offered(option, false))
            .flat_map(|method| {
                self.method_buttons(slug, provider, method, web_sign_in, pending, cx)
            });

        Modal::new(
            "settings-sign-in-prompt",
            t!("login-choose-title", provider = provider),
        )
        .w(px(420.))
        .close_button()
        .detail(t!("login-choose-detail", provider = provider))
        .child(
            div()
                .flex()
                .flex_col()
                .justify_center()
                .gap_3()
                .w_full()
                .min_h(theme.metrics.control * CHOICES)
                .children(buttons.map(|button| div().w_full().child(button))),
        )
        .on_dismiss(cx.listener(|this, _, _, cx| this.close_sign_in(cx)))
    }

    fn abandon(&mut self, cx: &mut Context<Self>) {
        self.clear_secret(cx);
        self.clear_credentials(cx);
        self.session
            .update(cx, |session, cx| session.cancel_sign_in(cx));
    }

    fn open_credentials(&mut self, slug: &'static str, cx: &mut Context<Self>) {
        self.credentials_for = Some(slug);
        cx.notify();
    }

    fn clear_credentials(&mut self, cx: &mut Context<Self>) {
        self.credentials_for = None;
        self.server.update(cx, |input, cx| input.set_text("", cx));
        self.username.update(cx, |input, cx| input.set_text("", cx));
        self.password.update(cx, |input, cx| input.set_text("", cx));
    }

    fn abandon_credentials(&mut self, cx: &mut Context<Self>) {
        self.clear_credentials(cx);
        cx.notify();
    }

    fn submit_credentials(&mut self, cx: &mut Context<Self>) {
        let Some(slug) = self.credentials_for else {
            return;
        };
        let server = self.server.read(cx).text().to_string();
        let username = self.username.read(cx).text().to_string();
        let password = self.password.read(cx).text().to_string();
        if server.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
            return;
        }
        self.clear_credentials(cx);
        self.session.update(cx, |session, cx| {
            session.sign_in(
                slug,
                SignIn::Credentials {
                    server,
                    username,
                    password,
                },
                cx,
            )
        });
    }

    fn credentials_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Modal::new("settings-server-prompt", t!("login-server-title"))
            .w(px(560.))
            .detail(t!("login-server-detail"))
            .child(self.server.clone())
            .child(self.username.clone())
            .child(self.password.clone())
            .action(
                Button::new("settings-cancel-server")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.abandon_credentials(cx))),
            )
            .action(
                Button::new("settings-submit-server")
                    .label(t!("login-server-submit"))
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.submit_credentials(cx))),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.abandon_credentials(cx)))
    }

    fn start_manual(&mut self, slug: &'static str, provider: &'static str, cx: &mut Context<Self>) {
        self.manual_secret = Some((slug, provider));
        let hint = CookiePrompt::hint(slug);
        self.secret.update(cx, |input, cx| input.set_hint(hint, cx));
        self.session
            .update(cx, |session, cx| session.sign_in_with_cookies(slug, cx));
    }

    fn clear_secret(&mut self, cx: &mut Context<Self>) {
        self.manual_secret = None;
        self.secret.update(cx, |input, cx| input.set_text("", cx));
    }

    fn submit_secret(&mut self, cx: &mut Context<Self>) {
        let text = self.secret.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.clear_secret(cx);
        self.session
            .update(cx, |session, cx| session.submit_input(text, cx));
    }

    fn secret_prompt(
        &self,
        slug: &'static str,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        CookiePrompt::new(slug, provider, self.secret.clone())
            .on_submit(cx.listener(|this, _, _, cx| this.submit_secret(cx)))
            .on_cancel(cx.listener(|this, _, _, cx| this.abandon(cx)))
    }

    /// The buttons that start one sign-in method. A cookie sign-in yields the browser window
    /// only where a backend can draw one, and always the manual paste beside it.
    fn method_buttons(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: SignIn,
        web_sign_in: bool,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut buttons = Vec::new();
        let manual = matches!(method, SignIn::Secret);
        if !manual || web_sign_in {
            buttons.push(
                self.method_button(slug, provider, method, pending, cx)
                    .into_any_element(),
            );
        }
        if manual {
            buttons.push(
                Button::new(SharedString::from(format!("connect-{slug}-cookies-manual")))
                    .label(t!("login-connect-cookies"))
                    .secondary()
                    .disabled(pending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sign_in_running = true;
                        this.start_manual(slug, provider, cx);
                    }))
                    .into_any_element(),
            );
        }
        buttons
    }

    fn method_button(
        &self,
        slug: &'static str,
        provider: &'static str,
        method: SignIn,
        pending: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (id, label) = match &method {
            SignIn::Default => (
                format!("connect-{slug}"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Anonymous => (format!("connect-{slug}-guest"), t!("login-guest-use")),
            SignIn::Secret => (
                format!("connect-{slug}-cookies"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Path(_) => (
                format!("connect-{slug}-path"),
                t!("login-sign-in", provider = provider),
            ),
            SignIn::Credentials { .. } => (
                format!("connect-{slug}-server"),
                t!("login-sign-in", provider = provider),
            ),
        };

        Button::new(SharedString::from(id))
            .label(label)
            .primary()
            .disabled(pending)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sign_in_running = true;
                match &method {
                    SignIn::Credentials { .. } => this.open_credentials(slug, cx),
                    method => {
                        let method = method.clone();
                        this.session
                            .update(cx, |session, cx| session.sign_in(slug, method, cx));
                    }
                }
            }))
    }

    fn version_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();

        self.row(
            t!("settings-version"),
            t!("settings-version-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            div().child(VERSION).into_any_element(),
        )
    }

    fn license_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();

        self.row(
            t!("settings-license"),
            t!("settings-license-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            Button::new("license")
                .label(t!("settings-license-view"))
                .small()
                .outline()
                .icon("icons/link.svg")
                .on_click(|_, _, cx| cx.open_url(LICENSE_URL))
                .into_any_element(),
        )
    }

    fn source_row(&self, cx: &mut Context<Self>) -> Setting {
        let theme = *cx.theme();

        self.row(
            t!("settings-source"),
            t!("settings-source-detail"),
            theme.muted_foreground,
            theme.text(Text::Small),
            Button::new("source")
                .label(t!("settings-source-view"))
                .small()
                .outline()
                .icon("icons/link.svg")
                .on_click(|_, _, cx| cx.open_url(SOURCE_URL))
                .into_any_element(),
        )
    }

    fn notice(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        div()
            .text_color(theme.muted_foreground)
            .text_size(theme.text(Text::Small))
            .child(t!("settings-notice"))
    }

    fn team(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        InfoCard::new(t!("settings-team")).flex_none().child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(MEMBERS.into_iter().enumerate().map(|(index, member)| {
                    div()
                        .id(("team-member", index))
                        .flex()
                        .items_center()
                        .gap_3()
                        .px(theme.metrics.pad)
                        .py(theme.metrics.pad / 2.)
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.secondary_hover))
                        .on_click(move |_, _, cx| cx.open_url(member.profile))
                        .child(Avatar::new(Some(member.avatar)).size(theme.metrics.thumb))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(div().font_weight(FontWeight::MEDIUM).child(member.login))
                                .child(
                                    div()
                                        .text_size(theme.text(Text::Small))
                                        .text_color(theme.muted_foreground)
                                        .child(t!("settings-team-github")),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(theme.text(Text::Small))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.muted_foreground)
                                .child(member.role.label()),
                        )
                })),
        )
    }

    fn row(
        &self,
        title: SharedString,
        detail: SharedString,
        muted: gpui::Hsla,
        small: Pixels,
        action: gpui::AnyElement,
    ) -> Setting {
        // The deck measures this row from `standard_height`, so both lines pin themselves to
        // that leading and truncate to one, and an empty detail still keeps its line.
        let detail_line = px((small / px(1.) * LEADING).round());
        let element = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .line_height(relative(LEADING))
                            .child(title.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .min_h(detail_line)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .line_height(relative(LEADING))
                            .text_color(muted)
                            .text_size(small)
                            .child(detail.clone()),
                    ),
            )
            .child(div().flex_none().child(action))
            .into_any_element();

        Setting {
            title,
            detail,
            element,
        }
    }

    fn account_modal(
        &self,
        accounts: Vec<AccountChoice>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        AccountPicker::new(accounts)
            .on_pick(cx.listener(|this, id: &SharedString, _, cx| {
                let id = id.to_string();
                this.session
                    .update(cx, |session, cx| session.submit_input(id, cx));
            }))
            .on_cancel(cx.listener(|this, _, _, cx| {
                this.session
                    .update(cx, |session, cx| session.cancel_sign_in(cx));
            }))
    }
}

fn romanization_script_copy(writing_system: WritingSystem) -> (&'static str, &'static str) {
    match writing_system {
        WritingSystem::Japanese => ("romanization-japanese", "settings-romanization-japanese"),
        WritingSystem::Chinese => ("romanization-chinese", "settings-romanization-chinese"),
        WritingSystem::Korean => ("romanization-korean", "settings-romanization-korean"),
        WritingSystem::Cyrillic => ("romanization-cyrillic", "settings-romanization-cyrillic"),
        WritingSystem::Greek => ("romanization-greek", "settings-romanization-greek"),
        WritingSystem::Arabic => ("romanization-arabic", "settings-romanization-arabic"),
        WritingSystem::Other => ("romanization-other", "settings-romanization-other"),
    }
}

fn samples(pack: &'static icons::Pack, tint: gpui::Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .children(icons::SAMPLES.iter().map(|name| {
            svg()
                .path(icons::shown(pack, name))
                .size(px(14.))
                .flex_none()
                .text_color(tint)
        }))
}

/// One card: the headline with every button beside it, and the sign-in error when one is
/// shown. Summed from the same fixed parts `card` is built of, so the deck never clips one.
fn card_height(theme: &Theme, error: bool) -> Pixels {
    let head = line(theme, Text::Body) + HALF_GAP + line(theme, Text::Small);
    let mut inner = head.max(theme.metrics.control);
    if error {
        inner += SECTION_GAP + line(theme, Text::Small).max(ERROR_ICON);
    }
    px(2.) + theme.metrics.pad * 2. + inner
}

/// The shell every account draws in: the radio saying whether it is the one playing, the logo,
/// the name over its status, and whatever the card ends with. The whole surface takes the click
/// when there is one, so the trailing button has to stop the press from reaching it.
struct AccountCard {
    id: SharedString,
    logo: &'static str,
    name: SharedString,
    status: Option<SharedString>,
    selected: bool,
    trailing: AnyElement,
    error: Option<Failure>,
    press: Option<Press>,
}

fn card(card: AccountCard, cx: &App) -> impl IntoElement {
    let AccountCard {
        id,
        logo,
        name,
        status,
        selected,
        trailing,
        error,
        press,
    } = card;
    let theme = *cx.theme();
    // every card stands as tall as the tallest thing it can hold, so the one without a status
    // line under its name is no shorter than the rest
    let head = line(&theme, Text::Body) + HALF_GAP + line(&theme, Text::Small);
    div()
        .id(id.clone())
        .flex()
        .flex_col()
        .gap_3()
        .p(theme.metrics.pad)
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .when_some(press, |this, press| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme.secondary))
                .on_click(move |event, window, cx| press(event, window, cx))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(head.max(theme.metrics.control))
                .child(Radio::new(
                    SharedString::from(format!("{id}-radio")),
                    selected,
                ))
                .child(
                    svg()
                        .path(icons::path(logo))
                        .size(theme.metrics.control_small)
                        .flex_none()
                        .text_color(theme.foreground),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .line_height(relative(LEADING))
                                .font_weight(FontWeight::MEDIUM)
                                .child(name),
                        )
                        .children(status.map(|status| {
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .line_height(relative(LEADING))
                                .text_color(theme.muted_foreground)
                                .text_size(theme.text(Text::Small))
                                .child(status)
                        })),
                )
                .child(div().flex().flex_none().items_center().child(trailing)),
        )
        .when_some(error, |this, error| this.child(account_error(&error, cx)))
}

/// How many buttons one sign-in method draws: the method itself, and the manual paste beside
/// a cookie one. A provider that draws exactly one never opens the choice dialog.
fn method_count(method: &SignIn, web_sign_in: bool) -> usize {
    let manual = matches!(method, SignIn::Secret);
    usize::from(!manual || web_sign_in) + usize::from(manual)
}

/// A provider's sign-in error in one line, for the fixed-height card. The full notice does
/// not fit a measured row.
fn account_error(error: &Failure, cx: &App) -> impl IntoElement {
    let theme = *cx.theme();
    div()
        .flex()
        .items_center()
        .gap_2()
        .h(line(&theme, Text::Small).max(ERROR_ICON))
        .child(
            svg()
                .path(icons::path("icons/circle-alert.svg"))
                .size(ERROR_ICON)
                .flex_none()
                .text_color(theme.danger),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .line_height(relative(LEADING))
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(crate::shared::trouble::short(error)),
        )
}

/// What the Widevine row says: the detail key for its state, and the note beside it.
/// Shared by the row and its search entry.
fn widevine_copy(state: &CdmState) -> (&'static str, &'static str) {
    match state {
        CdmState::Looking => ("settings-widevine-detail", "settings-widevine-looking"),
        CdmState::Ready(Origin::Configured) => {
            ("settings-widevine-detail", "settings-widevine-configured")
        }
        CdmState::Ready(Origin::Installed) => {
            ("settings-widevine-detail", "settings-widevine-installed")
        }
        CdmState::Ready(Origin::Fetched) => {
            ("settings-widevine-detail", "settings-widevine-fetched")
        }
        CdmState::Wanted | CdmState::Offered(_) => {
            ("settings-widevine-none", "settings-widevine-asking")
        }
        CdmState::Offering => ("settings-widevine-none", "settings-widevine-fetching"),
        CdmState::Installing => ("settings-widevine-none", "settings-widevine-installing"),
        CdmState::Declined | CdmState::Missing => {
            ("settings-widevine-none", "settings-widevine-missing")
        }
    }
}

/// Hands a file to the system's default application for it, without waiting on that program.
fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()?;

    #[cfg(target_os = "macos")]
    Command::new("open").arg(path).spawn()?;

    #[cfg(target_os = "linux")]
    Command::new("xdg-open").arg(path).spawn()?;

    Ok(())
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let searching = self.searching();
        // a search can list the typeface row from any category
        let in_appearance = self.tab == SettingsTab::Appearance || searching;
        let picking_typefaces = self.popovers.shows(TYPEFACES);
        if (in_appearance || picking_typefaces) && self.installed.is_none() && !self.loading_fonts {
            self.loading_fonts = true;
            let text_system = cx.text_system().clone();
            let io = Io::global(cx);
            self.font_task = Some(cx.spawn(async move |this, cx| {
                let names = io
                    .spawn_blocking(move || usable_fonts(text_system))
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

        if picking_typefaces {
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
        let taken = accounts.is_some() || manual_secret.is_some() || self.credentials_for.is_some();
        let sign_in_for = self.sign_in_for.filter(|_| !taken);

        // a dialog takes the key focus, since escape only reaches the page from inside it
        let dialog = taken || sign_in_for.is_some() || self.scrobble_prompt.is_some();
        match (dialog, self.grabbed) {
            (true, false) => {
                window.focus(&self.focus, cx);
                self.grabbed = true;
            }
            (false, true) => self.grabbed = false,
            _ => {}
        }

        let general = self.tab == SettingsTab::General && !searching;
        let about = self.tab == SettingsTab::About && !searching;

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                cx.stop_propagation();
                this.escape(cx);
            }))
            // the padding, the fade and the scrollbar all hang off the header's measured
            // height, so the page sits out the first frame rather than snapping into place
            .when(!self.header_measured, |this| this.invisible())
            .child(
                Scroller::new("settings", &self.scrollbar)
                    .flex()
                    .flex_col()
                    .items_center()
                    // the rows dissolve over the header's height as they scroll up beneath
                    // it, the way the verse sheet fades its edges, so no sharp content ever
                    // sits over the blur. The tail reaches past the header so the handoff
                    // has no hard edge.
                    .when(effects(), |this| {
                        this.fade_edges(self.header_height + HEADER_FADE_TAIL, px(0.))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .w_full()
                            .max_w(WIDTH)
                            .px_6()
                            .pb_6()
                            .pt(self.header_height)
                            .when(general, |this| {
                                this.child(self.profile(cx))
                                    .child(Separator::horizontal().w_full())
                            })
                            .child(self.panel(window, cx))
                            .when(about, |this| {
                                this.child(self.team(cx)).child(self.notice(cx))
                            }),
                    ),
            )
            .when_some(accounts, |this, accounts| {
                this.child(self.account_modal(accounts, cx).into_any_element())
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
    }
}

/// The search field and the category bar over the settings page. `Workspace` floats it over
/// the page, outside the transition, so a category switch fades the rows and nothing else, and
/// the rows scroll beneath it. It reads the page's state, writes back through
/// `SettingsView::select`, and reports its height so the page can start below it.
pub struct SettingsHeader {
    view: Entity<SettingsView>,
    /// The category clicked last, kept free of the hover shade until the pointer leaves it,
    /// so the change of selection never flashes.
    calm: Option<SettingsTab>,
}

impl SettingsHeader {
    pub fn new(view: Entity<SettingsView>, cx: &mut Context<Self>) -> Self {
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        Self { view, calm: None }
    }
}

impl Render for SettingsHeader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.view.read(cx);
        let search = page.search.clone();
        let chosen = page.tab;
        let searching = page.searching();
        let height = page.header_height;
        let calm = self.calm;
        let view = self.view.clone();
        let theme = *cx.theme();

        // a search lights no category, since its rows come from all of them, and picking
        // one ends the search
        let categories = TabBar::new("settings-categories")
            .max_w_full()
            .blurred()
            .items(SettingsTab::ALL.map(|tab| {
                Button::new(tab.id())
                    .label(i18n::lookup(tab.key(), None))
                    .icon(tab.icon())
                    .small()
                    .ghost()
                    .selected(!searching && tab == chosen)
                    .when(calm == Some(tab), Button::hoverless)
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if !hovered && this.calm == Some(tab) {
                            this.calm = None;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.calm = Some(tab);
                        this.view.update(cx, |view, cx| view.select(tab, cx));
                        navigate(Destination::Settings(tab), cx);
                    }))
            }));

        div()
            .relative()
            .flex()
            .justify_center()
            .px_6()
            // the veil is absolute and takes the header's own size, so the last child, the
            // column, is the one to measure
            .on_children_prepainted(move |bounds, _, cx| {
                let Some(height) = bounds.last().map(|bounds| bounds.size.height) else {
                    return;
                };
                view.update(cx, |view, cx| view.set_header_height(height, cx));
            })
            // The haze follows the window: a see-through page has content of its own passing
            // under the header and reads worse without it. Only the flat fallback is dropped
            // there, since a solid band over a see-through page is a slab.
            .when(effects() || !theme.transparent, |this| {
                this.child(veil(
                    Edge::Top,
                    height,
                    HEADER_BLUR,
                    theme.background,
                    window,
                ))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(WIDTH)
                    .gap_2()
                    .pt_6()
                    .pb_6()
                    .child(search)
                    .child(div().flex().justify_center().child(categories)),
            )
    }
}

fn usable_fonts(text_system: std::sync::Arc<gpui::TextSystem>) -> Vec<SharedString> {
    let mut names = text_system.all_font_names();
    names.sort_unstable();
    names.dedup();

    names
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(SharedString::from)
        .collect()
}

fn sleep_slot(sleep: Option<Sleep>) -> usize {
    match sleep {
        None => 0,
        Some(Sleep::EndOfTrack) => SLEEP_LAST,
        Some(Sleep::After(after)) => minute_slot(after.as_secs() / 60),
    }
}

fn sleep_at_fraction(fraction: f32) -> Option<Sleep> {
    let slot = (fraction.clamp(0., 1.) * SLEEP_LAST as f32).round() as usize;
    match slot {
        0 => None,
        SLEEP_LAST => Some(Sleep::EndOfTrack),
        slot => Some(Sleep::After(Duration::from_secs(slot_minute(slot) * 60))),
    }
}

fn minute_slot(minutes: u64) -> usize {
    let minutes = minutes.clamp(1, SLEEP_MAX_MINUTES);
    let earlier_magnets = SLEEP_MAGNETS
        .iter()
        .filter(|magnet| **magnet < minutes)
        .count();
    let width = match SLEEP_MAGNETS.contains(&minutes) {
        true => SLEEP_MAGNET_WEIGHT,
        false => 1,
    };
    minutes as usize + earlier_magnets * (SLEEP_MAGNET_WEIGHT - 1) + (width - 1) / 2
}

fn slot_minute(slot: usize) -> u64 {
    let mut first = 1;
    for minute in 1..=SLEEP_MAX_MINUTES {
        let width = match SLEEP_MAGNETS.contains(&minute) {
            true => SLEEP_MAGNET_WEIGHT,
            false => 1,
        };
        if slot < first + width {
            return minute;
        }
        first += width;
    }
    SLEEP_MAX_MINUTES
}

fn sleep_label(sleep: Option<Sleep>) -> SharedString {
    match sleep {
        Some(Sleep::EndOfTrack) => t!("settings-sleep-end-of-track"),
        Some(Sleep::After(after)) => t!("settings-sleep-minutes", count = after.as_secs() / 60),
        None => t!("settings-sleep-off"),
    }
}

fn preset_key(preset: Preset) -> &'static str {
    match preset {
        Preset::Flat => "settings-equalizer-flat",
        Preset::BassBoost => "settings-equalizer-bass-boost",
        Preset::BassReducer => "settings-equalizer-bass-reducer",
        Preset::TrebleBoost => "settings-equalizer-treble-boost",
        Preset::Vocal => "settings-equalizer-vocal",
        Preset::Rock => "settings-equalizer-rock",
        Preset::Pop => "settings-equalizer-pop",
        Preset::Jazz => "settings-equalizer-jazz",
        Preset::Classical => "settings-equalizer-classical",
        Preset::Electronic => "settings-equalizer-electronic",
        Preset::Acoustic => "settings-equalizer-acoustic",
        Preset::Loudness => "settings-equalizer-loudness",
    }
}

/// A band gain as the readout shows it: signed, with the decimal only when it is not zero.
fn decibels(gain: f32) -> String {
    match gain == 0. {
        true => "0".to_owned(),
        false => format!("{gain:+}"),
    }
}

/// A band's centre frequency in hertz below a kilohertz and in kilohertz from there on.
fn hertz(frequency: f32) -> SharedString {
    match frequency >= 1_000. {
        true => t!(
            "settings-equalizer-kilohertz",
            khz = (frequency / 1_000.).round() as i64
        ),
        false => t!("settings-equalizer-hertz", hz = frequency.round() as i64),
    }
}
