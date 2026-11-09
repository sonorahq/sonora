//! The left sidebar: Home, Search, the two libraries with their tabs folding out under them,
//! History and Settings, then the pinned section, whose entries can be reordered by dragging
//! and take songs dragged onto a playlist. It resizes by its inner edge, and a window too narrow
//! for it and the page lays it over the page instead, shut until asked for.

use std::borrow::Cow;
use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use router::{Destination, LibraryTab, NavEntry, SettingsPage};
use rune::{
    Color, Cursor, Cx, Drag, El, GrowthSpec, Phase, Picked, ScrollSpec, Transition, Tx, col, el,
    icon, row, text,
};
use state::{Origin, PinSort, Shelf};
use ui::{
    Avatar, Button, Card, Carry, Cover, Edge, Ghost, Menu, MenuItem, PRESS_SCALE, Panel, Picker,
    Pin, PinKind, Placement, Popovers, Popup, SNUG, ScrollWatch, Side, Springs, Tabs, Text, Theme,
    Vacancy, Weight, drop_marker,
};

use crate::chrome::{LEFT_MAX, LEFT_MIN};
use crate::{Ui, act};

/// The sidebar's navigation rows in order: the entry that can hide each, its icon and where it
/// goes. Settings has no entry, so it is always shown, and it opens on the settings page last
/// visited rather than on the one named here.
pub(crate) const NAV: [(Option<NavEntry>, &str, Destination); 6] = [
    (Some(NavEntry::Home), "icons/house.svg", Destination::Home),
    (
        Some(NavEntry::Search),
        "icons/search.svg",
        Destination::Search,
    ),
    (
        Some(NavEntry::Library),
        "icons/library-big.svg",
        Destination::Library(LibraryTab::Songs),
    ),
    (
        Some(NavEntry::Local),
        "icons/file-music.svg",
        Destination::Local(LibraryTab::Songs),
    ),
    (
        Some(NavEntry::History),
        "icons/rotate-ccw-clock.svg",
        Destination::History,
    ),
    (
        None,
        "icons/settings.svg",
        Destination::Settings(SettingsPage::General),
    ),
];

const LIBRARY_TABS: [(&str, LibraryTab); 4] = [
    ("nav-songs", LibraryTab::Songs),
    ("nav-albums", LibraryTab::Albums),
    ("nav-artists", LibraryTab::Artists),
    ("nav-playlists", LibraryTab::Playlists),
];

/// The sort picker's key in the sidebar's popover group, and its trigger's.
const SORT: &str = "sidebar-pins-sort";
const SORT_TRIGGER: &str = "sidebar-pins-sort-trigger";
/// The id the selected row's fill carries, so it flies to whichever row is selected next.
const SELECTED: &str = "sidebar-selected";
/// The scroll viewport's key, which the return button glides back to its top.
const ROWS: &str = "sidebar-left-rows";
/// The prefix of the invisible drop zones over the pinned rows, and the zone over the header.
const ZONE: &str = "sidebar-pz-";
const HEADER_ZONE: &str = "sidebar-pz-head";
/// The four bands a pinned row is cut into for a drop: the outer two land above or below it,
/// the middle two on it.
const BANDS: usize = 4;
const HINT_HEIGHT: f32 = 42.;
const VACANCY_HEIGHT: f32 = 88.;
/// How far the pin mark on a library row falls back from the accent.
const PIN_MARK: f32 = 0.7;
const ICON: f32 = 16.;
/// The side padding of a regular button, which a navigation row keeps; a fixed size, unlike the
/// spacing around it.
const ROW_INSET: f32 = 12.;
/// The space under the pinned section's header.
const HEADER_FOOT: f32 = 2.;
/// How far the pointer travels before a press on a pinned row becomes a drag.
const DRAG_START: f32 = 4.;
/// The dashes of the drop hint's border.
const DASH: (f32, f32) = (4., 3.);
/// How far a laid-over sidebar slides in from.
const SLIDE: f32 = 24.;

/// The two navigation entries that expand into tabs rather than navigate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Library,
    Local,
}

impl Group {
    fn of(destination: &Destination) -> Option<Self> {
        match destination {
            Destination::Library(_) => Some(Self::Library),
            Destination::Local(_) => Some(Self::Local),
            _ => None,
        }
    }

    fn tab(self, tab: LibraryTab) -> Destination {
        match self {
            Self::Library => Destination::Library(tab),
            Self::Local => Destination::Local(tab),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Local => "local",
        }
    }
}

/// A pin on its way somewhere, from the pinned list or from a page.
struct Dragging {
    pin: Pin,
}

/// The sidebar's own view state. Its width and whether it is open live in the chrome's state,
/// since the workspace lays out by them.
pub struct State {
    library_open: bool,
    local_open: bool,
    /// The page a group was folded shut on, which keeps it shut there even though the page is
    /// one of its own.
    folded: Option<Destination>,
    pinned_open: bool,
    /// Shown or hidden by hand while the window is cramped, and whether it was cramped then: a
    /// window that grows or shrinks past the line forgets it.
    forced: Option<(bool, bool)>,
    drag: Option<Dragging>,
    /// Whether the dragged pin is over the sidebar, where letting go drops it.
    over: bool,
    drop_gap: Option<usize>,
    /// The playlist a dragged song is held over, which takes the song instead of a new pin.
    drop_into: Option<Pin>,
    carry: Carry,
    /// The pin a context menu is open on, and where.
    menu: Option<(Pin, (f32, f32))>,
    popovers: Popovers,
    scroll: ScrollWatch,
}

impl State {
    pub fn new(settings: &state::AppSettings) -> Self {
        Self {
            library_open: false,
            local_open: false,
            folded: None,
            pinned_open: settings.sidebar_pinned_open(),
            forced: None,
            drag: None,
            over: false,
            drop_gap: None,
            drop_into: None,
            carry: Carry::new(),
            menu: None,
            popovers: Popovers::default(),
            scroll: ScrollWatch::new(),
        }
    }

    fn flag(&mut self, group: Group) -> &mut bool {
        match group {
            Group::Library => &mut self.library_open,
            Group::Local => &mut self.local_open,
        }
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let cramped = cramped(ui, cx);
    let open = is_open(ui, cramped);
    let overlaid = cramped && open;
    let width = ui.chrome.left_width.clamp(LEFT_MIN, LEFT_MAX);
    let keep = match overlaid {
        true => 0.,
        false => SNUG + reserved_right(ui),
    };
    let reach = cx
        .layout_with("window", move |rect| {
            (rect[2] - keep).max(LEFT_MIN).min(LEFT_MAX).round()
        })
        .unwrap_or(LEFT_MAX);
    mount(ui, layer);

    let rows = rows(ui, width, theme);
    let mut panel = Panel::new("sidebar-left", Side::Left, width)
        .limits(LEFT_MIN, LEFT_MAX)
        .reach(reach)
        .clears_scrollbar()
        .child(
            col()
                .key("sidebar-left-clip")
                .grow(1.)
                .min_h(0.)
                .w_full()
                .clip_rounded(0.)
                .child(rows),
        );
    if open {
        panel = panel.on_resize(|ui: &mut Ui, width: f32, _: &mut Tx| {
            ui.chrome.left_width = width;
            let open = ui.chrome.left_open;
            ui.store.settings.set_sidebar(width, open, &mut ui.store.cx);
            ui.settle();
        });
    }
    let mut panel = panel.build(theme).motion(Springs::BASE);
    if !theme.transparent {
        panel = panel.fill(theme.sidebar.color());
    }

    match (overlaid, open) {
        (true, _) => el()
            .key("sidebar-left-overlay")
            .absolute(0., 0.)
            .w_full()
            .h_full()
            .raise()
            .child(
                ui::shield::<Ui>("sidebar-shield")
                    .absolute(0., 0.)
                    .w_full()
                    .h_full()
                    .on_press(act(dismiss)),
            )
            .child(
                panel
                    .absolute(0., 0.)
                    .fill(theme.sidebar.color())
                    .transition(Transition::offset(-SLIDE, 0.).combined(Transition::opacity())),
            ),
        // Shut, the panel springs to nothing and its edge fades, the rows held at their width
        // inside a clip so nothing reflows as it goes.
        (false, false) => panel
            .w(0.)
            .border_sides([0., 0., 0., 0.], Color::TRANSPARENT),
        (false, true) => panel,
    }
}

/// Whether the sidebar shows, laid over the page or beside it.
pub(crate) fn shown(ui: &Ui, cx: &Cx<Ui>) -> bool {
    is_open(ui, cramped(ui, cx))
}

/// Whether the window leaves too little room for the page beside the sidebar, which then lays
/// itself over the page and stays shut until asked for. An open right sidebar counts even while
/// the window is too narrow to show it, so this sidebar gives way first.
pub(crate) fn cramped(ui: &Ui, cx: &Cx<Ui>) -> bool {
    let taken = ui.chrome.left_width.clamp(LEFT_MIN, LEFT_MAX) + reserved_right(ui);
    cx.layout_with("window", move |rect| rect[2] - taken < SNUG)
        .unwrap_or(false)
}

/// Shows or hides the sidebar, as its title bar toggle does. Beside the page the choice is
/// kept; laid over a cramped window it lasts until the window crosses the line again.
pub(crate) fn toggle(ui: &mut Ui, cramped: bool) {
    match cramped {
        true => {
            let open = is_open(ui, true);
            ui.chrome.sidebar.forced = Some((true, !open));
        }
        false => {
            ui.chrome.sidebar.forced = None;
            ui.chrome.left_open = !ui.chrome.left_open;
            let (width, open) = (ui.chrome.left_width, ui.chrome.left_open);
            ui.store.settings.set_sidebar(width, open, &mut ui.store.cx);
        }
    }
    ui.settle();
}

/// Carries `pin` along a drag that started anywhere in the window: call it from the dragged
/// element's `on_drag` with every phase. Over the sidebar the pinned section opens and shows
/// where the pin would land; let go there and it is pinned at that place, or, for a song held
/// over a playlist it can join, added to that playlist. The ghost under the pointer is drawn
/// by the sidebar.
pub(crate) fn carry(ui: &mut Ui, pin: &Pin, drag: &Drag, tx: &mut Tx) {
    carry_from(ui, pin, drag, tx);
}

fn is_open(ui: &Ui, cramped: bool) -> bool {
    match ui.chrome.sidebar.forced {
        Some((when, open)) if when == cramped => open,
        _ => ui.chrome.left_open && !cramped,
    }
}

/// The width an open right sidebar holds back from this one.
fn reserved_right(ui: &Ui) -> f32 {
    match ui.chrome.right_open {
        true => ui.chrome.right_width,
        false => 0.,
    }
}

/// Shuts a sidebar laid over the page, as a press beside it or a trip from it does.
fn dismiss(ui: &mut Ui) {
    if let Some((true, _)) = ui.chrome.sidebar.forced {
        ui.chrome.sidebar.forced = Some((true, false));
    }
}

/// Goes to `destination` from a row of the sidebar.
fn travel(ui: &mut Ui, destination: Destination) {
    dismiss(ui);
    ui.chrome.sidebar.menu = None;
    if let Some(group) = Group::of(&destination) {
        *ui.chrome.sidebar.flag(group) = true;
        ui.chrome.sidebar.folded = None;
    }
    match destination.in_settings() {
        true => ui.open_settings(),
        false => ui.go(destination),
    }
}

/// Whether `group` shows its tabs: opened by hand or by going to one of them, unless it was
/// folded shut on the page showing now.
fn opened(ui: &Ui, group: Group, at: &Destination) -> bool {
    let state = &ui.chrome.sidebar;
    let flag = match group {
        Group::Library => state.library_open,
        Group::Local => state.local_open,
    };
    let here = Group::of(at) == Some(group);
    match here && state.folded.as_ref() == Some(at) {
        true => flag,
        false => flag || here,
    }
}

/// Mounts what the sidebar floats over the window: the pin sort menu, a pin's context menu and
/// the ghost of a pin being dragged.
fn mount(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let state = &ui.chrome.sidebar;
    if state.popovers.shows(SORT) {
        sort_menu(ui)
            .anchor(SORT_TRIGGER, Placement::BottomEnd, ui::TRIGGER_GAP)
            .on_dismiss(|ui: &mut Ui| ui.chrome.sidebar.popovers.close())
            .build(theme, layer);
    }
    if let Some((pin, at)) = &state.menu {
        Popup::new("sidebar-pin-menu", *at, pin_menu(ui, pin))
            .on_close(|ui: &mut Ui| ui.chrome.sidebar.menu = None)
            .build(theme, layer);
    }
    if let Some(dragging) = &state.drag
        && let Some(artwork) = &ui.artwork
    {
        let pin = &dragging.pin;
        let mut ghost = Ghost::new(pin.title.clone())
            .art(pin.cover.as_deref())
            .fallback(pin.kind.icon());
        if pin.kind.round() {
            ghost = ghost.circle();
        }
        layer.push(ghost.build(artwork, &state.carry, theme));
    }
}

/// The scrolling rows: the navigation, then the pinned section, as one virtual list so a long
/// library alongside the pins costs only the rows on screen.
fn rows(ui: &Ui, width: f32, theme: &Theme) -> El {
    let shown = ui.store.settings.nav_shown(NavEntry::Pins.id());
    let pinned = match shown && ui.chrome.sidebar.pinned_open {
        true => pinned(ui),
        false => (Arc::new(Vec::new()), Arc::new(Vec::new())),
    };
    let count = pinned.0.len() + pinned.1.len();
    let list_row = theme.metrics.list_row.round();
    let nav = (NAV.len() as f32) * (theme.metrics.control + gap(theme));
    let heights = std::iter::once(nav)
        .chain(shown.then_some(theme.metrics.control_small + theme.metrics.pad))
        .chain((0..count).map(|_| list_row + gap(theme)));
    let deck = ui::Deck::new("sidebar-deck").rows(heights).build(
        move |ui: &Ui, index| row_key(ui, shown, index),
        move |ui: &Ui, index| draw(ui, shown, index),
    );
    let scroll = &ui.chrome.sidebar.scroll;
    let viewport = scroll.vertical(
        col()
            .key(ROWS)
            .role("navigation")
            .label(i18n::t!("nav-sidebar"))
            .grow(1.)
            .min_h(0.)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(deck),
        ui::return_reach(theme, 0.),
    );
    col()
        .key("sidebar-left-region")
        .w(width - 1.)
        .h_full()
        .child(viewport)
        .child(ui::return_top::<Ui>(ROWS, scroll, theme))
}

/// The pins in their order, and the library under them when the full library is shown.
fn pinned(ui: &Ui) -> (Arc<Vec<Pin>>, Arc<Vec<Pin>>) {
    let store = &ui.store;
    let pins = store.pins.entries(&store.settings, &store.session);
    let rest = match store.settings.sidebar_full_library() {
        true => store
            .pins
            .library(&store.settings, &store.session, &store.library),
        false => Arc::new(Vec::new()),
    };
    (pins, rest)
}

fn row_key(ui: &Ui, shown: bool, index: usize) -> Cow<'static, str> {
    match (index, shown) {
        (0, _) => Cow::Borrowed("sidebar-nav"),
        (1, true) => Cow::Borrowed("sidebar-pins-head"),
        _ => {
            let (pins, rest) = pinned(ui);
            let at = index - 2;
            match pins.get(at).or_else(|| rest.get(at - pins.len().min(at))) {
                Some(pin) => Cow::Owned(format!("pinned-slot-{}", pin_key(pin))),
                None => Cow::Owned(format!("pinned-slot-{at}")),
            }
        }
    }
}

fn draw(ui: &Ui, shown: bool, index: usize) -> El {
    let theme = &ui.theme;
    let pad = rems(theme, 0.75);
    match (index, shown) {
        (0, _) => navigation(ui).px(pad).pt(pad).pb(gap(theme)).w_full(),
        (1, true) => pins_head(ui).px(pad).w_full(),
        _ => {
            let (pins, rest) = pinned(ui);
            let at = index - 2;
            let held = pins.len();
            let total = held + rest.len();
            let pin = match at < held {
                true => pins.get(at),
                false => rest.get(at - held),
            };
            let row = match pin {
                Some(pin) => pin_row(ui, at, pin, held, total),
                None => el(),
            };
            let last = at + 1 == total;
            col()
                .w_full()
                .px(pad)
                .pb(match last {
                    true => ui::perch_room(theme),
                    false => gap(theme),
                })
                .child(row)
        }
    }
}

/// The navigation entries, each group followed by its tabs while open. Each entry is a row of a
/// growth stack, so a group folding open pushes the rows under it down on a spring rather than
/// in one step.
fn navigation(ui: &Ui) -> El {
    let theme = &ui.theme;
    let store = &ui.store;
    let at = ui.nav.current();
    // A shelf with rows on it belongs in the sidebar even before the session has restored,
    // since a snapshot fills it while the provider is still answering.
    let stocked = store.session.authenticated() || store.library.stocked(Shelf::Streaming);
    let entries: Vec<usize> = (0..NAV.len())
        .filter(|&index| {
            let (entry, _, destination) = &NAV[index];
            if entry.is_some_and(|entry| !store.settings.nav_shown(entry.id())) {
                return false;
            }
            !(Group::of(destination) == Some(Group::Library) && !stocked)
        })
        .collect();
    let last = entries.len().saturating_sub(1);
    col()
        .key("sidebar-navigation")
        .growth(GrowthSpec::default())
        .children(entries.into_iter().enumerate().map(|(place, index)| {
            let entry = nav_entry(ui, index, &at);
            let entry = match place < last {
                true => entry.pb(gap(theme)),
                false => entry,
            };
            entry.fold_glide(Springs::BASE)
        }))
}

/// One navigation entry. A group flips its tabs open and shut and is lit while one of them
/// shows; any other entry navigates and is selected while its section shows.
fn nav_entry(ui: &Ui, index: usize, at: &Destination) -> El {
    let theme = &ui.theme;
    let (entry, icon, destination) = NAV[index].clone();
    let key = entry.map_or("nav-settings", NavEntry::key);
    let label = i18n::lookup(key, None).into_owned();
    let Some(group) = Group::of(&destination) else {
        let selected = destination.same_section(at);
        let row = nav_row(key, label, Some(icon), selected, selected, theme)
            .on_press(act(move |ui: &mut Ui| travel(ui, destination.clone())));
        return col().key(format!("{key}-entry")).w_full().child(row);
    };
    let open = opened(ui, group, at);
    let lit = Group::of(at) == Some(group);
    let row = nav_row(key, label, Some(icon), lit, false, theme)
        .expanded(open)
        .child(chevron(open, theme.metrics.pad * 2., theme, lit))
        .on_press(act(move |ui: &mut Ui| {
            let at = ui.nav.current();
            let open = opened(ui, group, &at);
            *ui.chrome.sidebar.flag(group) = !open;
            ui.chrome.sidebar.folded = (open && Group::of(&at) == Some(group)).then_some(at);
        }));
    let mut entry = col()
        .key(format!("{}-group", group.name()))
        .w_full()
        .child(row);
    if open {
        entry = entry.child(tabs(group, at, theme));
    }
    entry
}

/// The tabs under an open group, which grow in as it opens.
fn tabs(group: Group, at: &Destination, theme: &Theme) -> El {
    let count = LIBRARY_TABS.len();
    let items = LIBRARY_TABS
        .into_iter()
        .enumerate()
        .map(|(slot, (key, tab))| {
            let destination = group.tab(tab);
            let chosen = destination == *at;
            nav_row(
                &format!("{}-tab-{key}", group.name()),
                i18n::lookup(key, None).into_owned(),
                None,
                chosen,
                chosen,
                theme,
            )
            .position_in_set(slot + 1, count)
            .on_press(act(move |ui: &mut Ui| travel(ui, destination.clone())))
        });
    Tabs::new()
        .items(items)
        .build(theme)
        .key(format!("{}-tabs", group.name()))
        .mt(gap(theme))
        .transition(Transition::opacity())
}

/// A row as the sidebar draws it: its icon and label in the foreground while `lit` and muted
/// otherwise, the sidebar's accent under the pointer, and, while `selected`, a fill of that
/// accent that flies from the row selected before.
fn nav_row(
    key: &str,
    label: String,
    glyph: Option<&str>,
    lit: bool,
    selected: bool,
    theme: &Theme,
) -> El {
    let tint = match lit {
        true => theme.foreground,
        false => theme.muted_foreground,
    };
    let mut row = row()
        .key(key.to_owned())
        .role("button")
        .label(label.clone())
        .w_full()
        .min_w(0.)
        .h(theme.metrics.control)
        .px(ROW_INSET)
        .gap(rems(theme, 0.625))
        .center_y()
        .radius(theme.radius)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .press_scale(PRESS_SCALE)
        .motion(Springs::CONTROL);
    row = match selected {
        true => row.focus_selected(true).child(selected_fill(theme)),
        false => row
            .hover(theme.sidebar_accent.color())
            .press(Color::TRANSPARENT),
    };
    if let Some(glyph) = glyph {
        row = row.child(icon_el(glyph, theme, lit));
    }
    row.child(
        text(label)
            .key("label")
            .font(ui::font(theme, Text::Body, Weight::NORMAL))
            .color(tint.color())
            .truncate()
            .grow(1.)
            .shrink()
            .min_w(0.)
            .a11y_hidden(),
    )
}

/// The selected row's fill. One element carries the id, so a new selection starts it where the
/// old one was and springs it over.
fn selected_fill(theme: &Theme) -> El {
    el().key(SELECTED)
        .matched(SELECTED)
        .absolute(0., 0.)
        .w_full()
        .h_full()
        .radius(theme.radius)
        .fill(theme.sidebar_accent.color())
        .a11y_hidden()
}

fn icon_el(path: &str, theme: &Theme, lit: bool) -> El {
    let tint = match lit {
        true => theme.foreground,
        false => theme.muted_foreground,
    };
    icon(icons::get(path), ICON)
        .color(tint.color())
        .a11y_hidden()
}

/// The pinned section's header, which folds the section and, while open, sorts it; under it the
/// empty section's note, or while a pin is dragged over an empty section the place to drop it.
fn pins_head(ui: &Ui) -> El {
    let theme = &ui.theme;
    let state = &ui.chrome.sidebar;
    let open = state.pinned_open;
    let name = i18n::t!("nav-pinned").into_owned();
    let toggle = row()
        .key("sidebar-pins-toggle")
        .role("button")
        .label(name.clone())
        .expanded(open)
        .grow(1.)
        .min_w(0.)
        .h_full()
        .center_y()
        .gap(theme.font_size * 0.25)
        .focusable(true)
        .cursor(Cursor::Pointer)
        .press_scale(PRESS_SCALE)
        .child(chevron(open, theme.text(Text::Small), theme, false))
        .child(ui::eyebrow(&name, theme).truncate().min_w(0.).a11y_hidden())
        .on_press(act(|ui: &mut Ui| {
            let open = !ui.chrome.sidebar.pinned_open;
            ui.chrome.sidebar.pinned_open = open;
            ui.store
                .settings
                .set_sidebar_pinned_open(open, &mut ui.store.cx);
        }));
    let mut header = row()
        .key("sidebar-pins-header")
        .w_full()
        .min_w(0.)
        .h(theme.metrics.control_small)
        .center_y()
        .justify_between()
        .pl(rems(theme, 0.5))
        // Narrower on the right than on the left, so the sort glyph falls on the same line as
        // the chevrons of the rows above.
        .pr(rems(theme, 0.25))
        .child(toggle);
    if open {
        header = header.child(sort_trigger(ui));
    }
    if state.drag.is_some() {
        header = header.child(zone(
            HEADER_ZONE.to_owned(),
            0.,
            theme.metrics.control_small,
        ));
    }

    let mut head = col()
        .key("sidebar-pins")
        .w_full()
        .pt(theme.metrics.pad)
        .pb(HEADER_FOOT)
        .child(header);
    if open {
        let (pins, rest) = pinned(ui);
        if pins.is_empty() && rest.is_empty() {
            head = head.child(match state.drag.is_some() {
                true => hint(theme),
                false => vacancy(theme),
            });
        }
    }
    head
}

/// The button over the open section that orders the pins.
fn sort_trigger(ui: &Ui) -> El {
    let theme = &ui.theme;
    let store = &ui.store;
    let sorted = store.pins.sorted(&store.settings);
    let open = ui.chrome.sidebar.popovers.shows(SORT);
    Button::new(SORT_TRIGGER)
        .icon("icons/arrow-up-down.svg")
        .tooltip(i18n::t!("tool-sort"))
        .small()
        .selected(open)
        .expanded(open)
        .tint(match sorted {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .on_press(|ui: &mut Ui| ui.chrome.sidebar.popovers.toggle(SORT))
        .build(theme)
        .w(theme.metrics.control_small)
        .px(0.)
}

/// The sort orders, the one in effect checked with its direction, and whether the library
/// shows under the pins.
fn sort_menu(ui: &Ui) -> Menu<Ui> {
    let store = &ui.store;
    let chosen = store.pins.sort(&store.settings);
    let arrow = match store.pins.reversed(&store.settings) {
        true => "icons/chevron-down.svg",
        false => "icons/chevron-up.svg",
    };
    let full = store.settings.sidebar_full_library();
    Menu::new("sidebar-pins-sort-menu")
        .width(Picker::<Ui>::NARROW)
        .items(PinSort::ALL.into_iter().map(|sort| {
            let item = MenuItem::new(sort.id(), i18n::lookup(sort.key(), None))
                .selected(Some(sort) == chosen)
                .on_press(act(move |ui: &mut Ui| ui.store.choose_pin_sort(sort)));
            match Some(sort) == chosen {
                true => item.icon(arrow),
                false => item,
            }
        }))
        .item(MenuItem::separator("sidebar-pins-scope"))
        .item(
            MenuItem::new("sidebar-full-library", i18n::t!("nav-show-full-library"))
                .checked(full)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store
                        .settings
                        .set_sidebar_full_library(!full, &mut ui.store.cx)
                })),
        )
}

/// One entry. The first `held` of them are the pins, which carry the drop slots. The rest is
/// the library underneath, which can be dragged up into the pins but holds no slot.
fn pin_row(ui: &Ui, index: usize, pin: &Pin, held: usize, total: usize) -> El {
    let theme = &ui.theme;
    let store = &ui.store;
    let state = &ui.chrome.sidebar;
    let destination = destination(pin);
    let active = destination == ui.nav.current();
    let aimed = state.drop_into.as_ref() == Some(pin);
    let pinned = index < held;
    // Only worth marking when the library sits alongside, since otherwise every row is a pin.
    let marked = pinned && store.settings.sidebar_full_library();
    let edge = match state.drop_gap {
        Some(gap) if pinned && gap == index => Some(Edge::Above),
        Some(gap) if gap == held && index + 1 == held => Some(Edge::Below),
        _ => None,
    };
    let origin = Origin::from(pin);
    let playing = store.playback.playing_from(&origin, &store.queue) == Some(true);
    let key = pin_key(pin);
    let side = (theme.metrics.list_row.round() - theme.metrics.pad * 2.).round();
    let art = match ui.artwork.as_ref() {
        Some(artwork) if pin.kind.round() => Avatar::new(artwork, pin.cover.as_deref())
            .size(side)
            .fallback(pin.kind.icon())
            .build(theme),
        Some(artwork) => Cover::new(artwork, pin.cover.as_deref())
            .size(side)
            .radius(theme.radius.min(4.))
            .fallback(pin.kind.icon())
            .build(theme),
        None => el().size(side, side),
    };
    let kind = i18n::lookup(pin.kind.key(), None).into_owned();

    let opened = pin.clone();
    let dragged = pin.clone();
    let mut card = Card::new(format!("pinned-{key}"), pin.title.clone())
        .description(kind.clone())
        .art(art)
        .play(playing, act_tx_origin(origin))
        .tint(match active {
            true => theme.foreground,
            false => theme.muted_foreground,
        })
        .bare_meta(caption(&kind, marked, theme))
        .chosen(aimed)
        .on_press(act(move |ui: &mut Ui| travel(ui, destination.clone())))
        .menu(move |ui: &mut Ui, press: rune::PointerPress, _: &mut Tx| {
            ui.chrome.sidebar.menu = Some((opened.clone(), (press.x, press.y)));
        });
    if pin.kind.round() {
        card = card.circle();
    }
    card = match active && !aimed {
        true => card.flat(),
        false => card.hover(theme.sidebar_accent),
    };
    let card = card
        .build(theme)
        .on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            carry_from(ui, &dragged, drag, tx);
        });

    let mut slot = el()
        .key(format!("pinned-{key}-place"))
        .role("listitem")
        .position_in_set(index + 1, total)
        .w_full()
        .min_w(0.);
    if active && !aimed {
        slot = slot.child(selected_fill(theme));
    }
    slot = slot.child(card);
    if let Some(edge) = edge {
        slot = slot.child(drop_marker(edge, theme));
    }
    if state.drag.is_some() {
        let height = theme.metrics.list_row.round() / BANDS as f32;
        for band in 0..BANDS {
            slot = slot.child(zone(
                format!("{ZONE}{index}-{band}"),
                band as f32 * height,
                height,
            ));
        }
    }
    slot
}

/// The play handler of a pin's card: pauses or resumes when the queue came from it, starts it
/// otherwise.
fn act_tx_origin(origin: Origin) -> impl Fn(&mut Ui, &mut Tx) + 'static {
    crate::act_tx(move |ui: &mut Ui, _: &mut Tx| ui.store.toggle_collection(&origin))
}

/// An invisible band across a row, `top` points down and `height` tall, that a drag finds under
/// the pointer. It takes no input, so the row under it still does.
fn zone(key: String, top: f32, height: f32) -> El {
    el().key(key)
        .absolute(0., top)
        .w_full()
        .h(height)
        .inert()
        .a11y_hidden()
}

/// The kind of an entry, behind a dimmed pin when it is one of the pinned ones.
fn caption(kind: &str, marked: bool, theme: &Theme) -> El {
    let mut line = row()
        .center_y()
        .gap(theme.metrics.pad / 4.)
        .min_w(0.)
        .a11y_hidden();
    if marked {
        line = line.child(
            icon(icons::get("icons/pin.svg"), theme.text(Text::Tiny))
                .color(theme.primary.opacity(PIN_MARK).color()),
        );
    }
    line.child(
        text(kind.to_owned())
            .font(ui::font(theme, Text::Small, Weight::NORMAL))
            .color(theme.muted_foreground.color())
            .truncate()
            .shrink()
            .min_w(0.),
    )
}

/// Where a dragged pin can land in an empty section.
fn hint(theme: &Theme) -> El {
    row()
        .key("sidebar-pin-hint")
        .min_h(HINT_HEIGHT)
        .mx(rems(theme, 0.5))
        .px(rems(theme, 0.5))
        .py(rems(theme, 0.25))
        .center()
        .radius(theme.radius)
        .border(1., theme.sidebar_border.color())
        .border_dash(DASH.0, DASH.1)
        .transition(Transition::opacity())
        .child(
            ui::spoken("sidebar-pin-hint-text", i18n::t!("nav-pin-hint"), theme)
                .font(ui::font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .align(rune::Align::Center)
                .wrap()
                .min_w(0.),
        )
}

fn vacancy(theme: &Theme) -> El {
    Vacancy::new(i18n::t!("nav-nothing-pinned"))
        .spoken("sidebar-nothing-pinned")
        .icon("icons/pin.svg")
        .compact()
        .build(theme)
        .key("sidebar-vacancy")
        .h(VACANCY_HEIGHT)
        .px(rems(theme, 0.5))
}

/// A pin's context menu: its page where the card does not open it, play, and pin or unpin.
fn pin_menu(ui: &Ui, pin: &Pin) -> Menu<Ui> {
    let store = &ui.store;
    let open = match pin.kind {
        PinKind::Album | PinKind::Playlist => None,
        PinKind::Artist => Some("menu-go-to-artist"),
        PinKind::Song => Some("menu-view-details"),
    };
    let held = store.pins.holds(pin, &store.settings, &store.session);
    let origin = Origin::from(pin);
    let mut menu = Menu::new("sidebar-pin-context").min_width(ITEM_MENU_WIDTH);
    if let Some(key) = open {
        let destination = destination(pin);
        menu = menu.item(
            MenuItem::new("open-pin", i18n::lookup(key, None))
                .icon("icons/info.svg")
                .on_press(act(move |ui: &mut Ui| travel(ui, destination.clone()))),
        );
    }
    let toggled = pin.clone();
    menu.item(
        MenuItem::new("play-pin", i18n::t!("common-play"))
            .icon("icons/play.svg")
            .on_press(act(move |ui: &mut Ui| ui.store.toggle_collection(&origin))),
    )
    .item(MenuItem::separator("pin-end"))
    .item(
        MenuItem::new(
            "toggle-pin",
            match held {
                true => i18n::t!("nav-unpin"),
                false => i18n::t!("nav-pin"),
            },
        )
        .icon(match held {
            true => "icons/pin-off.svg",
            false => "icons/pin.svg",
        })
        .on_press(act(move |ui: &mut Ui| match held {
            true => ui.store.unpin(toggled.clone()),
            false => ui.store.toggle_pin(toggled.clone()),
        })),
    )
}

/// The narrowest a pin's context menu gets.
const ITEM_MENU_WIDTH: f32 = 196.;

/// One phase of a drag of `pin`, from the pinned list or from anywhere else.
fn carry_from(ui: &mut Ui, pin: &Pin, drag: &Drag, tx: &mut Tx) {
    let state = &mut ui.chrome.sidebar;
    match drag.phase {
        Phase::Down => {
            state.drag = None;
            tx.live_only();
        }
        Phase::Move => {
            if state.drag.is_none() {
                if drag.dx.abs().max(drag.dy.abs()) < DRAG_START {
                    tx.live_only();
                    return;
                }
                state.drag = Some(Dragging { pin: pin.clone() });
                state.menu = None;
                // A drag has nowhere to land while the section is shut, so open it.
                if !state.pinned_open {
                    state.pinned_open = true;
                    ui.store
                        .settings
                        .set_sidebar_pinned_open(true, &mut ui.store.cx);
                }
            }
            let state = &mut ui.chrome.sidebar;
            state.carry.follow(drag);
            let [x, y] = drag.window_point();
            tx.element_at_in("sidebar-left", x, y, |ui: &mut Ui, picked, _: &mut Tx| {
                aim(ui, picked)
            });
        }
        Phase::Up => {
            let dragging = state.drag.take();
            let gap = state.drop_gap.take();
            let into = state.drop_into.take();
            let over = std::mem::take(&mut state.over);
            let Some(dragging) = dragging else {
                return;
            };
            if drag.cancelled() || !over {
                return;
            }
            match into {
                Some(playlist) => add_into(ui, &playlist, &dragging.pin),
                None => ui.store.place_pin(dragging.pin.origin(None), gap),
            }
            ui.settle();
        }
    }
}

/// Follows the pointer over the sidebar: the gap between two pins it would land in, or the
/// playlist it would join.
fn aim(ui: &mut Ui, picked: Option<Picked>) {
    let over = picked.is_some();
    let band = picked
        .as_ref()
        .and_then(Picked::key)
        .and_then(|key| key.strip_prefix(ZONE).map(str::to_owned));
    let (pins, rest) = pinned(ui);
    let held = pins.len();
    let state = &mut ui.chrome.sidebar;
    state.over = over;
    if !over {
        state.drop_gap = None;
        state.drop_into = None;
        return;
    }
    let Some(band) = band else {
        return;
    };
    if band == "head" {
        state.drop_gap = Some(0);
        state.drop_into = None;
        return;
    }
    let Some((index, band)) = band.split_once('-').and_then(|(index, band)| {
        Some((index.parse::<usize>().ok()?, band.parse::<usize>().ok()?))
    }) else {
        return;
    };
    let target = match index < held {
        true => pins.get(index),
        false => rest.get(index - held),
    }
    .cloned();
    let dragged = state.drag.as_ref().map(|dragging| dragging.pin.clone());
    let middle = band > 0 && band + 1 < BANDS;
    let into = match (target, dragged) {
        (Some(target), Some(song)) if middle && accepts(ui, &song, &target) => Some(target),
        _ => None,
    };
    let state = &mut ui.chrome.sidebar;
    state.drop_gap = match (&into, index < held) {
        (Some(_), _) => None,
        (None, true) => Some(match band < BANDS / 2 {
            true => index,
            false => index + 1,
        }),
        (None, false) => Some(held),
    };
    state.drop_into = into;
}

/// Whether `target` takes the dragged `song`: a playlist the listener may edit, on the same
/// shelf as the song, since a provider adds only its own tracks.
fn accepts(ui: &Ui, song: &Pin, target: &Pin) -> bool {
    song.kind == PinKind::Song
        && target.kind == PinKind::Playlist
        && Shelf::of(&song.id) == Shelf::of(&target.id)
        && ui.store.library.is_editable(&target.id)
}

/// Adds a dropped song to a playlist.
fn add_into(ui: &mut Ui, target: &Pin, song: &Pin) {
    let store = &mut ui.store;
    let Some(playlist) = store
        .library
        .playlist(&target.id)
        .map(|list| list.id.clone())
    else {
        return;
    };
    store
        .library
        .add_to_playlist(playlist, song.id.clone(), &store.session, &mut store.cx);
}

/// `n` of the old layout's rem, which follows the theme's text size: its `p_3` is `rems(.., 0.75)`.
fn rems(theme: &Theme, n: f32) -> f32 {
    theme.font_size * n
}

/// The space between two rows, the old `gap_1`.
fn gap(theme: &Theme) -> f32 {
    rems(theme, 0.25)
}

/// The page a pin opens.
fn destination(pin: &Pin) -> Destination {
    let id = pin.id.clone().into();
    match pin.kind {
        PinKind::Album => Destination::Album(id),
        PinKind::Artist => Destination::Artist(id),
        PinKind::Playlist => Destination::Playlist(id),
        PinKind::Song => Destination::Song(id),
    }
}

fn pin_key(pin: &Pin) -> String {
    format!("{}-{}", pin.kind.key(), pin.id)
}

/// The chevron of a section that folds: it points right while shut and turns to point down
/// as the section opens, springing round with a slight overshoot.
fn chevron(open: bool, size: f32, theme: &Theme, lit: bool) -> El {
    icon_el("icons/chevron-right.svg", theme, lit)
        .key("chevron")
        .size(size, size)
        .rotate(match open {
            true => FRAC_PI_2,
            false => 0.,
        })
        .motion(Springs::SETTLE)
}

#[cfg(test)]
mod tests {
    use router::{Destination, LibraryTab, SettingsPage};

    use super::Group;

    #[test]
    fn a_section_expands_only_where_it_leads() {
        assert_eq!(
            Group::of(&Destination::Library(LibraryTab::Albums)),
            Some(Group::Library)
        );
        assert_eq!(
            Group::of(&Destination::Local(LibraryTab::Albums)),
            Some(Group::Local)
        );
        assert!(Group::of(&Destination::Settings(SettingsPage::Providers)).is_none());
    }

    #[test]
    fn content_belongs_to_neither_section() {
        let away = [
            Destination::Home,
            Destination::Search,
            Destination::Album("id".into()),
            Destination::Playlist("id".into()),
            Destination::Artist("id".into()),
            Destination::Song("id".into()),
        ];

        for destination in away {
            assert!(Group::of(&destination).is_none(), "{destination:?}");
        }
    }
}
