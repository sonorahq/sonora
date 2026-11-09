//! The settings page: one tab of sections at a time under the floating header, or the settings
//! that answer a search, each opening its page and scrolling to its row.

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

use std::cmp::Reverse;

use router::{Destination, SettingsPage};
use rune::{Cursor, Cx, El, Span, TextEdit, Tx, col, icon, rich, row, text};
use ui::{MenuSearch, PRESS_SCALE, Popovers, ScrubberState, Text, Theme, Weight, font};

pub(crate) use layout::{Layout, frame};
#[allow(
    unused_imports,
    reason = "the pages of the other settings files build from these"
)]
pub(crate) use rows::{
    GROUPS, Row, columns, drill, grid, group, heading, matches_query, outward, segmented, stacked,
    switch, toggle, whole,
};

use crate::Ui;

/// How much a query found in a setting's title counts over one found in its detail.
const TITLE_WEIGHT: u32 = 2;
/// The side of the icon tile before a search result.
const RESULT_ART: f32 = 36.;
const RESULT_ICON: f32 = 18.;
/// The space between a result's tile and its words, and between its two lines.
const RESULT_GAP: f32 = 12.;
const RESULT_LINES: f32 = 2.;

/// This screen's own view state.
#[derive(Default)]
pub struct State {
    /// The pickers and menus of every page; one is open at a time.
    pub popovers: Popovers,
    /// The search field. While it holds text the page shows the settings that answer it.
    pub search: TextEdit,
    /// The key of what the tab scrolls into view once it is laid out: the row a search result
    /// opened, or the title of a section further down a tab that a route named.
    pub reveal: Option<String>,
    /// The search over the language picker's rows.
    pub languages: MenuSearch,
    /// The installed typefaces and the search over the typeface picker's rows.
    pub fonts: appearance::Fonts,
    pub typefaces: MenuSearch,
    pub opacity: ScrubberState,
    /// The theme family whose variant menu opened last, which owns that menu's popover.
    pub theme_variants: Option<String>,
    /// The user's theme family waiting for the user to confirm its deletion.
    pub deleting_theme: Option<String>,
    /// The cache of played tracks: the slider, the limit a drag points at until it lets go, and
    /// the measuring of what the cache and the downloads take.
    pub storage: storage::State,
    /// The field for the custom name Sonora goes by in Spotify's device list.
    pub connect_name: connect::State,
    /// The address of the listener's own Sonora Sessions server.
    pub sessions_server: sessions::State,
    /// The still spectra the visualizer cards draw.
    pub visualizers: now_playing::State,
    /// What the pages the other settings files draw keep between frames.
    pub accounts: accounts::State,
    pub playback: playback::State,
    pub scrobbling: scrobbling::State,
    pub lyrics: lyrics::State,
    /// What the previews keep running or picked while settings are open.
    pub previews: preview::Previews,
}

impl State {
    pub(crate) fn searching(&self) -> bool {
        !self.search.text.trim().is_empty()
    }

    /// Lets the page follow the window to `destination`: a route always lands on a plain tab,
    /// with nothing open, and a route to a later section of a tab reveals that section's title.
    pub(crate) fn arrived(&mut self, destination: &Destination) {
        let Destination::Settings(page) = destination else {
            return;
        };
        self.popovers.close();
        if self.searching() {
            self.search.clear();
        }
        self.reveal = (page.parent().is_none() && *page != page.tab().lead())
            .then(|| format!("settings-section-{}", page.id()));
    }
}

/// One setting a search can find: the page it lives on, the slug its row is keyed by, and the
/// Fluent keys of its title and detail.
struct Entry {
    page: SettingsPage,
    slug: &'static str,
    title: &'static str,
    detail: &'static str,
}

const fn entry(
    page: SettingsPage,
    slug: &'static str,
    title: &'static str,
    detail: &'static str,
) -> Entry {
    Entry {
        page,
        slug,
        title,
        detail,
    }
}

use SettingsPage as P;

/// Every setting a search reads, page by page in the order the tabs show them. Kept beside the
/// pages: a new row needs its entry here to be found, under the slug its [`Row`] is keyed by.
const ENTRIES: &[Entry] = &[
    entry(
        P::General,
        "startup",
        "settings-startup",
        "settings-startup-detail",
    ),
    entry(
        P::General,
        "entries",
        "settings-entries",
        "settings-entries-detail",
    ),
    entry(
        P::General,
        "language",
        "settings-language",
        "settings-language-detail",
    ),
    entry(
        P::General,
        "close-to-tray",
        "settings-close-to-tray",
        "settings-close-to-tray-detail",
    ),
    entry(
        P::General,
        "tray-icon",
        "settings-tray-icon",
        "settings-tray-icon-detail",
    ),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    entry(
        P::General,
        "server-side-decorations",
        "settings-server-side-decorations",
        "settings-server-side-decorations-detail",
    ),
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    entry(
        P::General,
        "window-controls",
        "settings-window-controls",
        "settings-window-controls-detail",
    ),
    #[cfg(not(target_os = "macos"))]
    entry(
        P::General,
        "controls-side",
        "settings-controls-side",
        "settings-controls-side-detail",
    ),
    #[cfg(not(target_os = "macos"))]
    entry(
        P::General,
        "traffic-light-controls",
        "settings-traffic-light-controls",
        "settings-traffic-light-controls-detail",
    ),
    entry(
        P::General,
        "window-rounding",
        "settings-window-rounding",
        "settings-window-rounding-detail",
    ),
    entry(
        P::General,
        "motion",
        "settings-motion",
        "settings-motion-detail",
    ),
    entry(P::General, "pace", "settings-pace", "settings-pace-detail"),
    entry(
        P::General,
        "saver",
        "settings-saver",
        "settings-saver-detail",
    ),
    entry(
        P::General,
        "adaptive-menu",
        "settings-adaptive-menu",
        "settings-adaptive-menu-detail",
    ),
    entry(
        P::About,
        "version",
        "settings-version",
        "settings-version-detail",
    ),
    entry(
        P::About,
        "check-updates",
        "settings-check-updates",
        "settings-check-updates-detail",
    ),
    entry(
        P::About,
        "update-channel",
        "settings-update-channel",
        "settings-update-channel-detail",
    ),
    entry(P::About, "log", "settings-log", "settings-log-detail"),
    entry(
        P::About,
        "license",
        "settings-license",
        "settings-license-detail",
    ),
    entry(
        P::About,
        "source",
        "settings-source",
        "settings-source-detail",
    ),
    entry(
        P::Providers,
        "accounts",
        "settings-accounts",
        "settings-accounts-detail",
    ),
    entry(P::Providers, "local-folder", "settings-local-folder", ""),
    entry(
        P::Appearance,
        "theme",
        "settings-theme",
        "settings-theme-detail",
    ),
    entry(
        P::Appearance,
        "adaptive-theme",
        "settings-adaptive",
        "settings-adaptive-detail",
    ),
    entry(
        P::Appearance,
        "community-themes",
        "settings-theme-explore",
        "settings-theme-explore-detail",
    ),
    entry(
        P::Appearance,
        "icons",
        "settings-icons",
        "settings-icons-detail",
    ),
    entry(
        P::Appearance,
        "corners",
        "settings-corners",
        "settings-corners-detail",
    ),
    entry(
        P::Appearance,
        "typeface",
        "settings-typeface",
        "settings-typeface-detail",
    ),
    entry(
        P::Appearance,
        "font",
        "settings-font",
        "settings-font-detail",
    ),
    entry(
        P::Appearance,
        "opacity",
        "settings-opacity",
        "settings-opacity-detail",
    ),
    entry(
        P::Appearance,
        "blur-window",
        "settings-blur-window",
        "settings-blur-window-detail",
    ),
    entry(
        P::Appearance,
        "blur",
        "settings-blur",
        "settings-blur-detail",
    ),
    entry(
        P::PlayerBar,
        "player-bar-layout",
        "settings-player-bar-layout",
        "settings-player-bar-layout-detail",
    ),
    entry(
        P::PlayerBar,
        "player-bar-transparent",
        "settings-player-bar-transparent",
        "settings-player-bar-transparent-detail",
    ),
    entry(
        P::NowPlaying,
        "ambient",
        "settings-ambient",
        "settings-ambient-detail",
    ),
    entry(
        P::NowPlaying,
        "ambient-motion",
        "settings-ambient-motion",
        "settings-ambient-motion-detail",
    ),
    entry(
        P::NowPlaying,
        "visualizer",
        "settings-visualizer",
        "settings-visualizer-detail",
    ),
    entry(
        P::NowPlaying,
        "visualizer-absolute",
        "settings-visualizer-absolute",
        "settings-visualizer-absolute-detail",
    ),
    entry(
        P::NowPlaying,
        "fullscreen-controls-autohide",
        "settings-fullscreen-controls-autohide",
        "settings-fullscreen-controls-autohide-detail",
    ),
    entry(
        P::NowPlaying,
        "os-fullscreen",
        "settings-os-fullscreen",
        "settings-os-fullscreen-detail",
    ),
    entry(
        P::Lyrics,
        "lyrics",
        "settings-lyrics",
        "settings-lyrics-detail",
    ),
    entry(
        P::Lyrics,
        "karaoke-lyrics",
        "settings-karaoke-lyrics",
        "settings-karaoke-lyrics-detail",
    ),
    entry(
        P::Lyrics,
        "blur-lyrics",
        "settings-blur-lyrics",
        "settings-blur-lyrics-detail",
    ),
    entry(
        P::Lyrics,
        "romanized-lyrics",
        "settings-romanized-lyrics",
        "settings-romanized-lyrics-detail",
    ),
    entry(
        P::Lyrics,
        "panel-lyrics-size",
        "settings-panel-lyrics-size",
        "settings-panel-lyrics-size-detail",
    ),
    entry(
        P::Lyrics,
        "fullscreen-lyrics-size",
        "settings-fullscreen-lyrics-size",
        "settings-fullscreen-lyrics-size-detail",
    ),
    entry(
        P::Lyrics,
        "lyrics-sources",
        "settings-lyrics-sources",
        "settings-lyrics-providers-detail",
    ),
    entry(
        P::Lyrics,
        "lyrics-for-local-files",
        "settings-lyrics-for-local-files",
        "settings-lyrics-for-local-files-detail",
    ),
    entry(
        P::LyricsSources,
        "lyrics-priority",
        "settings-lyrics-priority",
        "settings-lyrics-priority-detail",
    ),
    entry(
        P::Playback,
        "normalisation",
        "settings-normalisation",
        "settings-normalisation-detail",
    ),
    entry(
        P::Playback,
        "gapless",
        "settings-gapless",
        "settings-gapless-detail",
    ),
    entry(
        P::Playback,
        "sleep",
        "settings-sleep",
        "settings-sleep-detail",
    ),
    entry(
        P::Playback,
        "stay-awake",
        "settings-stay-awake",
        "settings-stay-awake-detail",
    ),
    entry(
        P::Playback,
        "stream-quality",
        "settings-stream-quality",
        "settings-stream-quality-detail",
    ),
    entry(
        P::Playback,
        "stream-format",
        "settings-stream-format",
        "settings-stream-format-detail",
    ),
    entry(
        P::Playback,
        "stream-bitrate",
        "settings-stream-bitrate",
        "settings-stream-bitrate-detail",
    ),
    entry(P::Playback, "widevine", "settings-widevine", ""),
    entry(
        P::Playback,
        "sessions-button",
        "settings-sessions-button",
        "",
    ),
    entry(
        P::Playback,
        "sessions-official",
        "settings-sessions-official",
        "",
    ),
    entry(
        P::Playback,
        "sessions-server",
        "settings-sessions-server",
        "",
    ),
    entry(
        P::Playback,
        "spotify-connect",
        "settings-spotify-connect",
        "settings-spotify-connect-detail",
    ),
    entry(
        P::Playback,
        "spotify-connect-name",
        "settings-spotify-connect-name",
        "settings-spotify-connect-name-detail",
    ),
    entry(
        P::Playback,
        "spotify-connect-custom",
        "settings-spotify-connect-custom",
        "settings-spotify-connect-custom-detail",
    ),
    entry(
        P::Sound,
        "output-device",
        "settings-output-device",
        "settings-output-device-detail",
    ),
    entry(
        P::Sound,
        "sample-rate",
        "settings-sample-rate",
        "settings-sample-rate-detail",
    ),
    entry(
        P::Sound,
        "buffer",
        "settings-buffer",
        "settings-buffer-detail",
    ),
    entry(
        P::Sound,
        "match-rate",
        "settings-match-rate",
        "settings-match-rate-detail",
    ),
    entry(
        P::Sound,
        "equalizer",
        "settings-equalizer",
        "settings-equalizer-detail",
    ),
    entry(
        P::Storage,
        "keep-tracks",
        "settings-keep-tracks",
        "settings-keep-tracks-detail",
    ),
    entry(
        P::Storage,
        "cache-size",
        "settings-cache-size",
        "settings-cache-size-detail",
    ),
    entry(
        P::Storage,
        "cache-used",
        "settings-cache-used",
        "settings-cache-used-detail",
    ),
    entry(
        P::Storage,
        "offline-mode",
        "settings-offline-mode",
        "settings-offline-mode-detail",
    ),
    entry(
        P::Storage,
        "downloads-used",
        "settings-downloads-used",
        "settings-downloads-used-detail",
    ),
    entry(
        P::Discord,
        "discord",
        "settings-discord",
        "settings-discord-detail",
    ),
    entry(
        P::Discord,
        "discord-name",
        "settings-discord-name",
        "settings-discord-name-detail",
    ),
    entry(
        P::Discord,
        "discord-show-paused",
        "settings-discord-show-paused",
        "settings-discord-show-paused-detail",
    ),
    entry(
        P::Discord,
        "discord-badge",
        "settings-discord-badge",
        "settings-discord-badge-detail",
    ),
    entry(
        P::Discord,
        "discord-anonymous",
        "settings-discord-anonymous",
        "settings-discord-anonymous-detail",
    ),
    entry(
        P::Discord,
        "discord-hide-on-connect",
        "settings-discord-hide-on-connect",
        "settings-discord-hide-on-connect-detail",
    ),
    entry(
        P::Discord,
        "discord-buttons",
        "settings-discord-buttons",
        "settings-discord-buttons-detail",
    ),
    entry(
        P::Discord,
        "artwork-for-local-files",
        "settings-artwork-for-local-files",
        "settings-artwork-for-local-files-detail",
    ),
];

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let state = &ui.screens.settings;
    let destination = ui.nav.current();
    let page = match &destination {
        Destination::Settings(page) => *page,
        _ => SettingsPage::General,
    };
    let (body, named, scroll) = match state.searching() {
        true => (
            results(ui),
            Some(i18n::t!("settings-search-results").into_owned()),
            "settings-scroll-search".to_owned(),
        ),
        false => (
            tab(ui, cx, layer, page),
            None,
            format!("settings-scroll-{}", page.id()),
        ),
    };
    // what the tab waits to reveal lands just below the header, clear of the fade
    let clear = Layout::of(ui, cx).clear();
    let body = body.on_layout_tx(move |ui: &mut Ui, _: [f32; 4], tx: &mut Tx| {
        if let Some(key) = ui.screens.settings.reveal.take() {
            tx.scroll_key_to(key, clear);
        }
    });
    themes::delete_modal(ui, layer);
    // a new opening of settings picks a new track for the previews, and a closing stops what
    // they keep running
    frame(ui, cx, layer, &destination, named, scroll, body).on_visible(
        |ui: &mut Ui, shown: bool, _: &mut Tx| {
            let previews = &mut ui.screens.settings.previews;
            match shown {
                true => previews.opened(&ui.store),
                false => previews.closed(&mut ui.store),
            }
        },
    )
}

/// The tab on show, or the drill-in on its own: each of its pages in turn. The frame draws the
/// first page's title, and every later page opens on its own title, set well apart from the
/// page above it.
fn tab(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, page: SettingsPage) -> El {
    let theme = &ui.theme;
    let pages: Vec<SettingsPage> = match page.parent() {
        Some(_) => vec![page],
        None => page.tab().sections().to_vec(),
    };
    let layout = Layout::of(ui, cx);
    let sections = pages.into_iter().enumerate().map(|(index, page)| {
        let mut section = col().key(format!("settings-page-{}", page.id())).w_full();
        if index > 0 {
            section = section.child(
                layout::section_title(theme, page)
                    .mt(GROUPS)
                    .mb(theme.metrics.inset),
            );
        }
        section.child(
            col()
                .key("groups")
                .w_full()
                .gap(GROUPS)
                .children(content(ui, cx, layer, page, &layout)),
        )
    });
    col()
        .key(format!("settings-sections-{}", page.tab().id()))
        .w_full()
        .gap(GROUPS)
        .children(sections)
}

/// What a page shows: its preview, its groups and anything under them, each one element of the
/// page's column. Pages another file draws return their own.
fn content(
    ui: &Ui,
    cx: &mut Cx<Ui>,
    layer: &mut Vec<El>,
    page: SettingsPage,
    layout: &Layout,
) -> Vec<El> {
    match page {
        SettingsPage::General => general::view(ui, cx, layer, layout),
        SettingsPage::Appearance => appearance::view(ui, cx, layer, layout),
        SettingsPage::PlayerBar => player_bar::view(ui, cx, layer, layout),
        SettingsPage::NowPlaying => {
            let mut blocks = vec![preview::now_playing::view(ui, cx, layout)];
            blocks.extend(now_playing::view(ui, cx, layer, layout));
            blocks
        }
        SettingsPage::Storage => storage::view(ui, cx, layer),
        SettingsPage::Discord => {
            let mut blocks = vec![preview::discord::view(ui, cx, layout)];
            blocks.extend(discord::view(ui, cx, layer));
            blocks
        }
        SettingsPage::Providers => accounts::view(ui, cx, layer, layout),
        SettingsPage::MusicFolders => accounts::folders(ui, cx, layer, layout),
        SettingsPage::Playback => {
            let mut groups = playback::view(ui, cx, layer, layout);
            groups.push(sessions::view(ui, cx, layer));
            groups.extend(connect::view(ui, cx, layer));
            groups
        }
        SettingsPage::Scrobbling => scrobbling::view(ui, cx, layer, layout),
        SettingsPage::Lyrics => lyrics::view(ui, cx, layer, layout),
        SettingsPage::LyricsSources => lyrics::sources(ui, cx, layer, layout),
        SettingsPage::Sound => sound::view(ui, cx, layer, layout),
        SettingsPage::About => about::view(ui, cx, layer, layout),
    }
}

/// Whether a setting's row stands on its page now, so a search finds only rows it can open.
fn shown(ui: &Ui, slug: &str) -> bool {
    let settings = &ui.store.settings;
    let playback = &ui.store.playback;
    let connect = ui.store.devices.supported();
    match slug {
        "keep-tracks" => ui.store.session.capabilities().keeps,
        "cache-size" => ui.store.session.capabilities().keeps && playback.keep_tracks(),
        "spotify-connect" => connect,
        "spotify-connect-name" => connect && settings.spotify_connect(),
        "spotify-connect-custom" => {
            connect
                && settings.spotify_connect()
                && settings.spotify_connect_name() == state::ConnectName::Custom
        }
        "sessions-server" => !settings.sessions_official(),
        "visualizer-absolute" => settings.visualizer_style().shown(),
        "blur-window" => ui::WINDOW_BLUR,
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        "window-rounding" => !settings.server_side_decorations(),
        "discord-name"
        | "discord-show-paused"
        | "discord-badge"
        | "discord-anonymous"
        | "discord-buttons" => settings.discord_presence(),
        "discord-hide-on-connect" => {
            settings.discord_presence() && settings.spotify_connect() && connect
        }
        _ => true,
    }
}

/// Every setting that answers the query, best match first, with its title. Ties keep the order
/// of the pages. The query may span the title and the detail, and a hit in the title alone
/// counts on top.
fn found(ui: &Ui, query: &str) -> Vec<Hit> {
    let mut hits: Vec<(u32, usize, Hit)> = settings(ui)
        .into_iter()
        .enumerate()
        .filter_map(|(order, hit)| {
            let whole = fuzzy(&format!("{} {}", hit.title, hit.detail), query)?;
            let titled = fuzzy(&hit.title, query).unwrap_or(0) * TITLE_WEIGHT;
            Some((whole + titled, order, hit))
        })
        .collect();
    hits.sort_by_key(|(score, order, _)| (Reverse(*score), *order));
    hits.into_iter().map(|(_, _, hit)| hit).collect()
}

/// One setting as a search reads it: where it lives, the slug its row is keyed by, and its
/// title and detail resolved.
struct Hit {
    page: SettingsPage,
    slug: String,
    title: String,
    detail: String,
}

/// Every setting that stands on its page now, in the order of the pages, with the details that
/// change with what the app holds (the accounts, the Widevine module, a failed output, each
/// scrobbling service) read afresh.
fn settings(ui: &Ui) -> Vec<Hit> {
    let lookup = |key: &str| match key.is_empty() {
        true => String::new(),
        false => i18n::lookup(key, None).into_owned(),
    };
    let mut hits = Vec::new();
    for entry in ENTRIES.iter().filter(|entry| shown(ui, entry.slug)) {
        let mut detail = lookup(entry.detail);
        match entry.slug {
            "accounts" => detail = format!("{detail} {}", accounts::account_words(ui)),
            "widevine" => detail = playback::widevine_detail(ui),
            _ => {}
        }
        hits.push(Hit {
            page: entry.page,
            slug: entry.slug.to_owned(),
            title: lookup(entry.title),
            detail,
        });
        if entry.slug == "output-device" && ui.store.outputs.failure().is_some() {
            let (title, detail) = sound::output_failure(ui);
            hits.push(Hit {
                page: entry.page,
                slug: "output-failure".to_owned(),
                title,
                detail,
            });
        }
    }
    for (index, row) in ui.store.scrobbling.rows().iter().enumerate() {
        if let Some((title, detail)) = scrobbling::scrobble_meta(ui, index) {
            hits.push(Hit {
                page: SettingsPage::Scrobbling,
                slug: format!("scrobble-{}", row.id()),
                title,
                detail,
            });
        }
    }
    hits
}

/// What a search shows instead of the page: every setting that answers it, the matched letters
/// of its name lit, each naming the page it lives on and opening it there.
fn results(ui: &Ui) -> El {
    let theme = &ui.theme;
    let query = ui.screens.settings.search.text.trim().to_lowercase();
    let hits = found(ui, &query);
    if hits.is_empty() {
        return col().key("settings-results").w_full().child(
            ui::Vacancy::new(i18n::t!("search-no-matches"))
                .icon("icons/search.svg")
                .spoken("settings-results-empty")
                .build(theme),
        );
    }
    let count = hits.len();
    // the rows pad their tile in from their own edge, so the list reaches out by as much to put
    // the tile in line with the column
    col()
        .key("settings-results")
        .role("list")
        .w_full()
        .mx(-theme.metrics.pad)
        .children(
            hits.into_iter()
                .enumerate()
                .map(|(index, hit)| result(theme, hit, &query, index, count)),
        )
}

/// One search result: the icon of the tab it lives in, the setting's name with the letters the
/// query found lit, and the page it lives on under it.
fn result(
    theme: &Theme,
    Hit {
        page, slug, title, ..
    }: Hit,
    query: &str,
    index: usize,
    count: usize,
) -> El {
    let tab = page.tab();
    let place = match page.parent() {
        Some(_) => i18n::t!(
            "settings-result-place",
            parent = i18n::lookup(tab.key(), None).to_string(),
            page = i18n::lookup(page.key(), None).to_string()
        )
        .into_owned(),
        None => i18n::lookup(page.key(), None).into_owned(),
    };
    let lit = lit(&title, query)
        .into_iter()
        .map(|(start, end)| {
            Span::new(start, end)
                .weight(Weight::SEMIBOLD)
                .color(theme.foreground.color())
        })
        .collect::<Vec<_>>();
    let art = row()
        .key("art")
        .center()
        .size(RESULT_ART, RESULT_ART)
        .radius(theme.radius)
        .fill(theme.secondary.color())
        .child(icon(icons::get(tab.icon()), RESULT_ICON).color(theme.muted_foreground.color()))
        .a11y_hidden();
    let words = col()
        .key("words")
        .grow(1.)
        .shrink()
        .min_w(0.)
        .gap(RESULT_LINES)
        .child(
            rich(title.clone(), lit)
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.foreground.opacity(0.85).color())
                .truncate()
                .a11y_hidden(),
        )
        .child(
            text(place.clone())
                .font(font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .truncate()
                .a11y_hidden(),
        );
    let key = format!("settings-result-{}-{}", page.id(), slug);
    row()
        .key(key)
        .role("button")
        .label(title)
        .description(place)
        .position_in_set(index + 1, count)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .center_y()
        .gap(RESULT_GAP)
        .h(theme.metrics.list_row)
        .px(theme.metrics.pad)
        .radius(theme.radius)
        .hover(theme.secondary_hover.color())
        .press(theme.secondary_active.color())
        .press_scale(PRESS_SCALE)
        .on_press_tx(crate::act_tx(move |ui: &mut Ui, _: &mut Tx| {
            open_result(ui, page, &slug)
        }))
        .child(art)
        .child(words)
}

/// Opens the page a result lives on and scrolls its setting into view once the tab lays out.
fn open_result(ui: &mut Ui, page: SettingsPage, slug: &str) {
    ui.screens.settings.search.clear();
    ui.go(Destination::Settings(page));
    ui.screens.settings.reveal = Some(format!("settings-row-{slug}"));
}

/// How well `text` answers `query`, or `None` when it does not: every word of the query has to
/// be found in it, and words found at the start of a word, or the whole query found as typed,
/// count for more.
fn fuzzy(text: &str, query: &str) -> Option<u32> {
    let text = text.to_lowercase();
    let mut score = 0;
    for word in query.split_whitespace() {
        let at = text.find(word)?;
        let starts = at == 0 || !text[..at].ends_with(char::is_alphanumeric);
        score += 1 + word.len() as u32 + u32::from(starts) * 4;
    }
    if text.contains(query) {
        score += 8;
    }
    Some(score)
}

/// The byte ranges of `title` each word of `query` was found at, ignoring case. A title whose
/// lower case moves its letters about lights nothing.
fn lit(title: &str, query: &str) -> Vec<(usize, usize)> {
    let lower = title.to_lowercase();
    if lower.len() != title.len() {
        return Vec::new();
    }
    let mut ranges: Vec<(usize, usize)> = query
        .split_whitespace()
        .filter_map(|word| lower.find(word).map(|at| (at, at + word.len())))
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// A small surface holding `glyph` in the middle, the thumbnail an option card shows for a
/// choice that is not a picture of itself.
pub(crate) fn glyph_thumb(theme: &Theme, glyph: &str, size: f32) -> El {
    row()
        .w_full()
        .h_full()
        .center()
        .child(icon(icons::get(glyph), size).color(theme.muted_foreground.color()))
}
