//! The title bar: the sidebar toggle, back, forward and a refresh that turns while the page
//! reloads, the downloads button and the page's own tools on the right, the right sidebar's
//! toggle, and the window controls when the app draws the window's frame itself. Everything not
//! a control moves the window.

use router::Destination;
use rune::{Color, Cx, El, row};
use state::{HistoryState, LibraryState, SessionState, Shelf};
use ui::{Button, Room, Theme, TrafficLightControls, WindowControls};

use crate::Ui;
use crate::chrome::{sidebar_left, toolbar};

/// The bar's bottom edge.
const BORDER: f32 = 1.;

/// The title bar's own state.
#[derive(Default)]
pub struct State {
    /// The page a refresh reloaded, so the refresh button turns until that page has its data
    /// and stops as soon as the window moves elsewhere.
    refreshing: Option<Destination>,
}

/// What the shell around the bar asks of it.
#[derive(Clone, Copy)]
pub(crate) struct Options {
    /// The sidebar toggle, the history buttons, the downloads and the page tools.
    pub navigation: bool,
    /// The hairline along the bottom. Its colour goes, its pixel stays, so a bar without it
    /// centres its controls where one with it does.
    pub border: bool,
    /// Lets what is behind the window (the fullscreen ambient) show through.
    pub transparent: bool,
}

impl Options {
    /// The workspace's bar: everything, over the theme's background.
    pub(crate) const WORKSPACE: Self = Self {
        navigation: true,
        border: true,
        transparent: false,
    };
    /// The bar over the sign-in page: only the window's own controls.
    pub(crate) const BARE: Self = Self {
        navigation: false,
        border: true,
        transparent: false,
    };
}

/// The bar the root view puts over the workspace or the sign-in page.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let options = match signed_out(ui) {
        true => Options::BARE,
        false => Options::WORKSPACE,
    };
    bar(ui, cx, layer, options)
}

/// The bar as `options` asks for it, for a shell that draws its own (the fullscreen player).
pub(crate) fn bar(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, options: Options) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let height = theme.metrics.title_bar;
    let navigation = options.navigation;
    let right =
        (navigation && crate::window_room(cx).fits(Room::Wide)).then_some(ui.chrome.right_open);
    let decorated = client_decorated(ui);
    let leading = decorated && settings.controls_on_left();
    // macOS draws its traffic lights over the bar's start; the bar's content starts past them.
    let inset = cx.titlebar_inset();
    let pad = theme.metrics.pad;
    let start = match (leading, inset > 0.) {
        (true, _) => pad,
        (false, true) => inset,
        (false, false) => pad * 1.5,
    };

    let mut first = row()
        .key("title-bar-start")
        .center_y()
        .gap(pad / 2.)
        .pl(start)
        .pr(match navigation {
            true => pad / 2.,
            false => pad * 1.5,
        });
    if leading {
        first = first.child(window_controls(ui, true));
    }
    if navigation {
        first = first.child(sidebar_toggle(ui, cx));
    }

    let mut middle = row()
        .key("title-bar-middle")
        .grow(1.)
        .min_w(0.)
        .center_y()
        .gap(pad / 2.)
        .pr(match right.is_some() {
            true => pad / 2.,
            false => pad * 1.5,
        });
    if navigation {
        middle = middle.child(history(ui, theme));
    }
    middle = middle.child(rune::spacer());
    if navigation {
        middle = middle
            .children(downloads(ui, cx, layer))
            .child(toolbar::view(ui, cx, layer));
    }

    let mut bar = row()
        .key("title-bar")
        .role("banner")
        .center_y()
        .w_full()
        .h(height)
        .border_sides(
            [0., 0., BORDER, 0.],
            match options.border {
                true => theme.title_bar_border.color(),
                false => Color::TRANSPARENT,
            },
        )
        .window_drag()
        .child(first)
        .child(middle);
    if !options.transparent && !theme.transparent {
        bar = bar.fill(theme.background.color());
    }
    if let Some(radius) = window_radius(ui) {
        bar = bar.corners([radius, radius, 0., 0.]);
    }
    if let Some(open) = right {
        bar = bar.child(
            row()
                .key("title-bar-end")
                .pr(pad * 1.5)
                .child(aside_toggle(ui, open)),
        );
    }
    if decorated && !leading {
        bar = bar.child(
            row()
                .key("title-bar-controls")
                .pr(pad)
                .child(window_controls(ui, false)),
        );
    }
    bar
}

/// Reloads the shown page, as the refresh button and its shortcut do, and turns the button
/// until the page has its data. A page still loading ignores it, so a held key cannot keep
/// restarting the load, and a page with nothing to reload ignores it too.
pub(crate) fn refresh(ui: &mut Ui) {
    let at = ui.nav.current();
    let Some(busy) = loading(ui, &at) else {
        return;
    };
    if busy {
        return;
    }
    crate::shells::workspace::refresh(ui);
    ui.chrome.title_bar.refreshing = Some(at);
}

/// Whether the sign-in page shows instead of the workspace, as the root view decides it.
fn signed_out(ui: &Ui) -> bool {
    match ui.store.session.state() {
        SessionState::SignedOut | SessionState::Failed(_) => true,
        SessionState::Restoring | SessionState::SignedIn(_) | SessionState::Offline(_) => false,
        SessionState::Authorizing(_) => ui.chrome.signing_in,
    }
}

/// The button that shows and hides the left sidebar.
fn sidebar_toggle(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let theme = &ui.theme;
    let open = sidebar_left::shown(ui, cx);
    let cramped = sidebar_left::cramped(ui, cx);
    square(
        Button::new("sidebar-toggle")
            .icon(match open {
                true => "icons/panel-left-close.svg",
                false => "icons/panel-left-open.svg",
            })
            .tooltip(i18n::t!("nav-sidebar"))
            .small()
            .expanded(open)
            .on_press(move |ui: &mut Ui| sidebar_left::toggle(ui, cramped))
            .build(theme),
        theme,
    )
    .no_window_drag()
}

/// The button that shows and hides the queue and lyrics.
fn aside_toggle(ui: &Ui, open: bool) -> El {
    let theme = &ui.theme;
    square(
        Button::new("sidebar-right-toggle")
            .icon(match open {
                true => "icons/panel-right-close.svg",
                false => "icons/panel-right-open.svg",
            })
            .tooltip(i18n::t!("nav-sidebar-right"))
            .small()
            .expanded(open)
            .on_press(crate::act(|ui: &mut Ui| {
                let open = !ui.chrome.right_open;
                ui.chrome.right_open = open;
                ui.store
                    .settings
                    .set_sidebar_right_open(open, &mut ui.store.cx);
            }))
            .build(theme),
        theme,
    )
    .no_window_drag()
}

/// Back, forward and, on a page that can reload, refresh. They light with the sidebar's accent,
/// as the rows under them do.
fn history(ui: &Ui, theme: &Theme) -> El {
    let at = ui.nav.current();
    let (back, forward) = (ui.nav.can_go_back(), ui.nav.can_go_forward());
    let spinning =
        ui.chrome.title_bar.refreshing.as_ref() == Some(&at) && loading(ui, &at).unwrap_or(false);
    let step = |key: &str, icon: &str, tip: String, enabled: bool, go: fn(&mut Ui)| {
        let button = Button::new(key)
            .icon(icon)
            .tooltip(tip)
            .disabled(!enabled)
            .on_press(crate::act(go))
            .build(theme);
        lit(button, enabled, theme)
    };

    let mut group = row()
        .key("titlebar-history")
        .role("toolbar")
        .label(i18n::t!("nav-history"))
        .center_y()
        .no_window_drag()
        .child(step(
            "history-back",
            "icons/chevron-left.svg",
            i18n::t!("nav-back").into_owned(),
            back,
            Ui::back,
        ))
        .child(step(
            "history-forward",
            "icons/chevron-right.svg",
            i18n::t!("nav-forward").into_owned(),
            forward,
            Ui::forward,
        ));
    if at.refreshes() {
        let button = Button::new("history-refresh")
            .icon("icons/refresh-cw.svg")
            .tooltip(i18n::t!("app-refresh"))
            .spinning(spinning)
            .on_press(crate::act(refresh))
            .build(theme);
        group = group.child(lit(button, true, theme));
    }
    group
}

/// The downloads button's place in the bar. It shows only while something downloads, and is
/// drawn by `chrome::downloads` once that is ported; until then the slot stays empty.
fn downloads(_ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> Option<El> {
    None
}

/// The window's own minimize, maximize and close, as themed buttons or the three dots.
fn window_controls(ui: &Ui, leading: bool) -> El {
    let theme = &ui.theme;
    match traffic_lights(ui) {
        true => TrafficLightControls::new(leading).build::<Ui>(theme),
        false => WindowControls::new(leading).build::<Ui>(theme),
    }
}

/// Whether the app draws the window's frame, and with it the window controls: under client-side
/// decorations on Linux, when asked for on Windows, never on macOS, where AppKit draws them.
fn client_decorated(ui: &Ui) -> bool {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    return !ui.store.settings.server_side_decorations();
    #[cfg(target_os = "windows")]
    return ui.store.settings.window_controls();
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "windows")))]
    {
        let _ = ui;
        false
    }
}

/// Whether the controls are the three coloured dots rather than themed buttons.
fn traffic_lights(ui: &Ui) -> bool {
    #[cfg(not(target_os = "macos"))]
    return ui.store.settings.traffic_light_controls();
    #[cfg(target_os = "macos")]
    {
        let _ = ui;
        false
    }
}

/// The radius the bar rounds the window's top corners with while the app draws the frame, or
/// `None` for square corners. It never exceeds half the bar's height.
fn window_radius(ui: &Ui) -> Option<f32> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        if !client_decorated(ui) {
            return None;
        }
        match ui.store.settings.window_rounding() {
            ui::Rounding::Square => None,
            rounding => Some(rounding.radius().min(ui.theme.metrics.title_bar / 2.)),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = ui;
        None
    }
}

/// Whether `at` is still waiting for its data, or `None` for a page with nothing to reload.
fn loading(ui: &Ui, at: &Destination) -> Option<bool> {
    let store = &ui.store;
    let pages = &store.pages;
    let shelf = |shelf: Shelf| matches!(store.library.state(shelf), LibraryState::Loading);
    Some(match at {
        Destination::Home => store.home.is_loading(&store.session, &store.library),
        Destination::History => matches!(store.history.state(), HistoryState::Loading),
        Destination::Library(_) => shelf(Shelf::Streaming),
        Destination::Local(_) => store.session.scanning() || shelf(Shelf::Local),
        Destination::Album(_) => pages.album.as_ref().is_some_and(|page| page.is_loading()),
        Destination::Playlist(_) => pages
            .playlist
            .as_ref()
            .is_some_and(|page| page.is_loading()),
        Destination::Artist(_) => pages.artist.as_ref().is_some_and(|page| page.is_loading()),
        Destination::Genre(_) => pages.genre.as_ref().is_some_and(|page| page.is_loading()),
        Destination::Song(_) => pages.song.as_ref().is_some_and(|page| page.is_loading()),
        Destination::User(_) => pages.user.as_ref().is_some_and(|page| page.is_loading()),
        Destination::Search
        | Destination::Settings(_)
        | Destination::Equalizer
        | Destination::Fullscreen => return None,
    })
}

/// An icon-only small button as a square, its icon centred.
fn square(button: El, theme: &Theme) -> El {
    button.w(theme.metrics.control_small).px(0.)
}

/// A history button: a regular-height square that hovers and presses in the sidebar's accent
/// while it can be pressed.
fn lit(button: El, enabled: bool, theme: &Theme) -> El {
    let button = button.w(theme.metrics.control).px(0.);
    match enabled {
        true => button
            .hover(theme.sidebar_accent.color())
            .press(Color::TRANSPARENT),
        false => button,
    }
}
