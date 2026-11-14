use std::rc::Rc;

use ui::{
    ActiveTheme as _, Button, Card, Deck, DraggedPin, Edge, EdgeSwell, Menu, MenuItem, Panel,
    Picker, Pin, PinKind, Pinnable as _, Popover, Popup, SNUG, Scroller, Shield, Side, Spot,
    Swells, Tabs, Text, Vacancy, drop_gap, drop_into, drop_marker,
};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, DragMoveEvent, ElementId, Entity, MouseButton, MouseDownEvent,
    Pixels, Point, Render, ScrollHandle, SharedString, svg,
};
use gpui::{Window, div, px};
use music::Shape;
use router::{
    Destination, LibraryTab, NavEntry, Navigation, NavigationEvent, SettingsPage, navigate,
    open_settings,
};
use state::{
    AppSettings, Leaving, Library, Origin, PinSort, Pins, Playback, Session, Shelf, Sonora,
};

use crate::shared::menus::{ItemMenu, item_menu};
use crate::shared::nav::{nav_icon, nav_row};
use crate::shared::pins::leaving;
use crate::shared::playlist_editor::{Edit, PlaylistEditor};

/// The one drag list the pinned section keeps, so a pin dropped anywhere in it lands in order.
const PINS: &str = "sidebar-pins";

/// The sidebar's navigation rows in order: the entry that can hide each, its icon and where it goes.
/// Settings has no entry, so it is always shown, and it opens on the settings page last visited
/// rather than on the one named here.
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

const MIN_WIDTH: Pixels = px(160.);
const MAX_WIDTH: Pixels = px(400.);
const HINT_HEIGHT: Pixels = px(42.);
const VACANCY_HEIGHT: Pixels = px(88.);
/// How far the pin mark on a library row falls back from the accent.
const PIN_MARK: f32 = 0.7;
/// The space between two pinned entries, the same as the `gap_1` between the rows above them.
const ROW_GAP: Pixels = px(4.);
/// The rail's side padding in rem, matching its `px_2`.
const RAIL_INSET: f32 = 0.5;
/// How far past the rail's edge a group's menu opens.
const FLYOUT_GAP: Pixels = px(4.);
const BORDER: Pixels = px(1.);

/// The two navigation entries that expand into tabs rather than navigate.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Library,
    Local,
}

impl Group {
    const ALL: [Self; 2] = [Self::Library, Self::Local];

    fn of(destination: &Destination) -> Option<Self> {
        match destination {
            Destination::Library(_) => Some(Self::Library),
            Destination::Local(_) => Some(Self::Local),
            _ => None,
        }
    }

    /// The Fluent key of the group's name, which also keys the menu it opens on the rail.
    fn key(self) -> &'static str {
        match self {
            Self::Library => "nav-library",
            Self::Local => "nav-local",
        }
    }

    /// Where `tab` of the group leads.
    fn at(self, tab: LibraryTab) -> Destination {
        match self {
            Self::Library => Destination::Library(tab),
            Self::Local => Destination::Local(tab),
        }
    }
}

pub(crate) struct SidebarLeft {
    settings: Entity<AppSettings>,
    session: Entity<Session>,
    trail: Entity<Navigation>,
    at: Destination,
    width: Pixels,
    open: bool,
    /// Whether the listener folded the sidebar down to the rail.
    rail: bool,
    rail_width: Pixels,
    /// Whether the window leaves too little room for the full width, so only the rail fits.
    narrow: bool,
    cramped: bool,
    forced: Option<bool>,
    /// The width and fold an edge drag started from, which a drag that closes the sidebar
    /// puts back so it reopens the way it was.
    grabbed: Option<(Pixels, bool)>,
    /// Whether an edge drag is held below half the rail, which closes the sidebar on release.
    closing: bool,
    library_open: bool,
    local_open: bool,
    pinned_open: bool,
    dropping: bool,
    drop_gap: Option<usize>,
    /// The playlist a dragged song is held over, which takes the song instead of a new pin.
    drop_into: Option<Pin>,
    playback: Entity<Playback>,
    pins: Entity<Pins>,
    library: Entity<Library>,
    track_menu: ItemMenu,
    context_menu: Option<(Pin, Point<Pixels>)>,
    scrollbar: Entity<ui::Scrollbar>,
    swell: EdgeSwell,
    popovers: ui::Popovers,
}

impl SidebarLeft {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let session = Sonora::global(cx).session.clone();
        let playback = Sonora::global(cx).playback.clone();
        let pins = Sonora::global(cx).pins.clone();
        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        cx.observe(&pins, |_, _, cx| cx.notify()).detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let chrome = super::Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();
        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| ui::Scrollbar::inset().watching(me));
        let scrollbar = cx.new(|_| {
            ui::Scrollbar::new(ScrollHandle::new())
                .passive()
                .watching(me)
        });
        let width = sidebar_width(settings.read(cx));
        let open = settings.read(cx).sidebar_open();
        let rail = settings.read(cx).sidebar_rail();
        let pinned_open = settings.read(cx).sidebar_pinned_open();
        let trail = router::trail(cx);

        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&trail, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&trail, |this, _, _: &NavigationEvent, cx| {
            this.dismiss(cx);
            cx.notify();
        })
        .detach();

        let at = trail.read(cx).current();
        let library_open = matches!(at, Destination::Library(_));
        let local_open = matches!(at, Destination::Local(_));

        Self {
            settings,
            session,
            trail,
            at,
            width,
            open,
            rail,
            rail_width: Pixels::ZERO,
            narrow: false,
            forced: None,
            cramped: false,
            grabbed: None,
            closing: false,
            library_open,
            local_open,
            pinned_open,
            dropping: false,
            drop_gap: None,
            drop_into: None,
            playback,
            pins,
            library,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            context_menu: None,
            scrollbar,
            swell: EdgeSwell::new(),
            popovers: ui::Popovers::default(),
        }
    }

    fn follow(&mut self, current: &Destination) {
        if self.at == *current {
            return;
        }
        self.at = current.clone();
        let (library, local) = expanded(current);
        self.library_open |= library;
        self.local_open |= local;
    }

    fn dismiss_menu(&mut self, cx: &mut Context<Self>) {
        self.track_menu.reset(cx);
        self.context_menu = None;
        cx.notify();
    }

    fn menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (pin, position) = self.context_menu.clone()?;
        let menu = item_menu(&pin, &self.track_menu, self.playback.clone(), cx);

        Some(
            Popup::new(position, menu)
                .on_close(cx.listener(|this, _, _, cx| this.dismiss_menu(cx))),
        )
    }

    pub fn is_open(&self) -> bool {
        self.forced.unwrap_or(self.open && !self.cramped)
    }

    pub fn overlays(&self) -> bool {
        self.cramped && self.is_open()
    }

    /// Whether the window has folded a full sidebar onto the rail by itself. An overlaid one
    /// floats over the content, so it keeps the listener's own width.
    fn squeezed(&self) -> bool {
        self.narrow && !self.cramped
    }

    /// Whether the sidebar is drawn as the rail, by choice or because the window is narrow.
    fn folded(&self) -> bool {
        self.rail || self.squeezed()
    }

    fn breadth(&self) -> Pixels {
        match (self.closing, self.folded()) {
            (true, _) => Pixels::ZERO,
            (false, true) => self.rail_width,
            (false, false) => self.width,
        }
    }

    pub fn overlay_width(&self) -> Pixels {
        match self.overlays() {
            true => self.breadth(),
            false => Pixels::ZERO,
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.overlays() {
            return;
        }
        self.forced = Some(false);
        cx.notify();
    }

    pub fn occupied_width(&self) -> Pixels {
        match self.is_open() && !self.overlays() {
            true => self.breadth(),
            false => Pixels::ZERO,
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        match self.cramped {
            true => self.forced = Some(!self.is_open()),
            false => {
                self.open = !self.open;
                self.persist(cx);
            }
        }
        cx.notify();
    }

    fn ceiling(&self, window: &Window, cx: &Context<Self>) -> Pixels {
        let reserved = match self.overlays() {
            true => Pixels::ZERO,
            // Held to the room that keeps it unfolded, so a drag never widens it onto the rail.
            false => SNUG + super::Chrome::reserved_right(cx),
        };

        super::cap(MIN_WIDTH, MAX_WIDTH, reserved, window)
    }

    /// Steps the sidebar down as the window narrows, from the room left once `right` pixels
    /// are held back for the right sidebar: a full sidebar folds onto the rail once it would
    /// leave the content less than `SNUG`, and either one goes cramped when even the rail
    /// leaves less than that. This runs inside a render, where a notify schedules nothing,
    /// so a flip asks for a full window refresh too. That effect lands once the draw is over.
    pub fn adapt(&mut self, right: Pixels, window: &Window, cx: &mut Context<Self>) {
        self.width = ui::snapped(self.width, window);
        self.rail_width = rail_width(window, cx);

        let room = window.viewport_size().width - right;
        let narrow = room - self.width < SNUG;
        let cramped = room - self.rail_width < SNUG;
        if narrow == self.narrow && cramped == self.cramped {
            return;
        }
        if cramped != self.cramped {
            self.forced = None;
        }
        self.narrow = narrow;
        self.cramped = cramped;
        cx.notify();
        cx.refresh_windows();
    }

    /// Follows a drag of the edge. Below half the rail the sidebar shrinks to nothing but keeps
    /// its edge, so the drag can still bring it back, and it closes once the drag lets go.
    /// Otherwise it folds to the rail below halfway between the rail and `MIN_WIDTH` and
    /// unfolds above it, except while the window holds the sidebar on the rail.
    fn resize(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.grabbed.get_or_insert((self.width, self.rail));
        self.closing = width < self.rail_width / 2.;
        if !self.closing && !self.squeezed() {
            let rail = width < (self.rail_width + MIN_WIDTH) / 2.;
            if !rail {
                self.width = width.max(MIN_WIDTH);
            }
            self.rail = rail;
            self.persist(cx);
        }
        cx.notify();
    }

    /// Ends an edge drag. One let go of below half the rail closes the sidebar with the width
    /// and fold the drag started from, so the toggle reopens it as it was. Runs inside a render,
    /// so it asks for a window refresh for the title bar's toggle to follow.
    fn settle(&mut self, cx: &mut Context<Self>) {
        let grabbed = self.grabbed.take();
        if !std::mem::take(&mut self.closing) {
            return;
        }
        if let Some((width, rail)) = grabbed {
            self.width = width;
            self.rail = rail;
        }
        match self.cramped {
            true => self.forced = Some(false),
            false => self.open = false,
        }
        self.persist(cx);
        cx.refresh_windows();
    }

    /// The navigation entries, each followed by its tabs while its group is open. The rail lists
    /// the entries alone, since a group there opens its tabs in a menu beside it.
    fn navigation(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // A shelf with rows on it belongs in the sidebar even before the session has restored,
        // since a snapshot fills it while the provider is still answering.
        let stocked = self.session.read(cx).authenticated()
            || self.library.read(cx).stocked(Shelf::Streaming);
        let mut rows = Vec::new();
        for (index, (entry, _, destination)) in NAV.iter().enumerate() {
            if entry.is_some_and(|entry| !self.settings.read(cx).nav_shown(entry.id())) {
                continue;
            }
            let group = Group::of(destination);
            if group == Some(Group::Library) && !stocked {
                continue;
            }
            rows.push(self.nav(index, window, cx));
            if let Some(group) = group.filter(|group| !self.folded() && self.opened(*group)) {
                rows.push(self.tabs(group, cx));
            }
        }
        rows
    }

    /// The pinned section: its header, and its entries once it is expanded.
    fn pins(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.settings.read(cx).nav_shown(NavEntry::Pins.id()) {
            return Vec::new();
        }

        let header = self.pins_header(cx);
        if !self.pinned_open {
            return vec![header];
        }

        std::iter::once(header)
            .chain(self.pin_entries(window, cx))
            .collect()
    }

    /// The pinned entries, as a `Deck` so only the ones on screen are ever built, or what stands
    /// in for them while there are none.
    fn pin_entries(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pins = self.pins.read(cx);
        let pinned = pins.entries(cx);
        let rest = match self.settings.read(cx).sidebar_full_library() {
            true => pins.library(cx),
            false => Rc::new(Vec::new()),
        };
        if pinned.is_empty() && rest.is_empty() {
            return (!self.folded()).then(|| match self.dropping {
                true => hint(cx),
                false => vacancy(),
            });
        }

        let held = pinned.len();
        let count = held + rest.len();
        let row = ui::snapped(cx.theme().metrics.list_row, window);
        Some(
            Deck::new("sidebar-pins-deck")
                .rows((0..count).map(|_| row))
                .gap(ROW_GAP)
                .draw(cx.processor(move |this, index: usize, _, cx| {
                    let pin = match index < held {
                        true => pinned.get(index),
                        false => rest.get(index - held),
                    };
                    match pin {
                        Some(pin) => this.pin_row(index, pin.clone(), held, count, cx),
                        None => div().into_any_element(),
                    }
                }))
                .into_any_element(),
        )
    }

    fn opened(&self, group: Group) -> bool {
        match group {
            Group::Library => self.library_open,
            Group::Local => self.local_open,
        }
    }

    fn flip(&mut self, group: Group) {
        let open = match group {
            Group::Library => &mut self.library_open,
            Group::Local => &mut self.local_open,
        };
        *open = !*open;
    }

    fn flip_pins(&mut self, cx: &mut Context<Self>) {
        self.pinned_open = !self.pinned_open;
        let open = self.pinned_open;
        self.settings.update(cx, |settings, cx| {
            settings.set_sidebar_pinned_open(open, cx)
        });
        cx.notify();
    }

    fn pins_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let open = self.pinned_open;
        let folded = self.folded();
        let toggle = match folded {
            true => nav_icon(
                "sidebar-pins-toggle",
                "nav-pinned",
                "icons/pin.svg",
                match open {
                    true => theme.foreground,
                    false => theme.muted_foreground,
                },
                theme.sidebar_accent,
            )
            .aria_expanded(open)
            .on_click(cx.listener(|this, _, _, cx| this.flip_pins(cx)))
            .into_any_element(),
            false => self.pins_toggle(cx),
        };

        div()
            .id("sidebar-pins-header")
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .w_full()
            .min_w_0()
            .when(!folded, |header| {
                header
                    .h(theme.metrics.control_small)
                    .pl_2()
                    // Narrower on the right than on the left, so the sort glyph falls on the
                    // same line as the chevrons of the rows above.
                    .pr_1()
            })
            .mt(theme.metrics.pad)
            .mb(px(2.))
            .child(toggle)
            .when(open && !folded, |header| header.child(self.sort_picker(cx)))
            // Dragging over the header aims at the top of the list, which is otherwise
            // out of reach above the first entry.
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<DraggedPin>, _, cx| {
                    if !event.bounds.contains(&event.event.position) || this.drop_gap == Some(0) {
                        return;
                    }
                    this.drop_gap = Some(0);
                    this.drop_into = None;
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    fn pins_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let open = self.pinned_open;

        div()
            .id("sidebar-pins-toggle")
            .role(Role::Button)
            .aria_label(i18n::lookup("nav-pinned", None))
            .aria_expanded(open)
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            .cursor_pointer()
            .child(
                svg()
                    .path(icons::path(chevron(open)))
                    .flex_none()
                    .size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground),
            )
            .child(
                ui::eyebrow(i18n::lookup("nav-pinned", None), cx)
                    .min_w_0()
                    .truncate(),
            )
            .on_click(cx.listener(|this, _, _, cx| this.flip_pins(cx)))
            .into_any_element()
    }

    fn sort_picker(&self, cx: &App) -> AnyElement {
        let chosen = self.pins.read(cx).sort(cx);
        let reversed = self.pins.read(cx).reversed(cx);
        let full = self.settings.read(cx).sidebar_full_library();
        let arrow = match reversed {
            true => "icons/chevron-down.svg",
            false => "icons/chevron-up.svg",
        };
        let pins = self.pins.clone();

        Picker::icon(
            "sidebar-pins-sort",
            &self.popovers,
            "icons/arrow-up-down.svg",
        )
        .tooltip("tool-sort")
        .sticky()
        .width(Picker::NARROW)
        .tint(match self.pins.read(cx).sorted(cx) {
            true => cx.theme().primary,
            false => cx.theme().muted_foreground,
        })
        .items(PinSort::ALL.into_iter().map(move |sort| {
            let pins = pins.clone();

            MenuItem::new(sort.id(), i18n::lookup(sort.key(), None))
                .selected(Some(sort) == chosen)
                .when(Some(sort) == chosen, |item| item.icon(arrow))
                .on_click(move |_, _, cx| {
                    pins.update(cx, |pins, cx| pins.choose(sort, cx));
                })
        }))
        .item(MenuItem::separator("sidebar-pins-scope"))
        .item(
            MenuItem::new(
                "sidebar-full-library",
                i18n::lookup("nav-show-full-library", None),
            )
            .checked(full)
            .on_click({
                let settings = self.settings.clone();
                move |_, _, cx| {
                    settings.update(cx, |settings, cx| {
                        settings.set_sidebar_full_library(!full, cx)
                    });
                }
            }),
        )
        .into_any_element()
    }

    /// One entry. The first `count` of them are the pins, which carry the drop slots. The rest
    /// is the library underneath, which can be dragged up into the pins but holds no slot.
    fn pin_row(
        &self,
        index: usize,
        pin: Pin,
        count: usize,
        total: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let accent = theme.sidebar_accent;
        let destination = Destination::from(&pin);
        let active = destination == self.trail.read(cx).current();
        let opened = pin.clone();
        let target = pin.clone();
        let aimed = self.drop_into.as_ref() == Some(&pin);
        let held = index < count;
        // Only worth marking when the library sits alongside, since otherwise every row is a pin.
        let marked = held && self.settings.read(cx).sidebar_full_library();
        let edge = match self.drop_gap {
            Some(gap) if held && gap == index => Some(Edge::Above),
            Some(gap) if gap == count && index + 1 == count => Some(Edge::Below),
            _ => None,
        };

        let origin = Origin::from(&pin);
        let playing = self.playback.read(cx).engaged(&origin, cx);
        let warmed = origin.clone();

        let key = format!("{}-{}", pin.kind.key(), pin.id);
        let card = Card::new(SharedString::from(format!("pinned-{key}")), pin.label())
            .aria_description(i18n::lookup(pin.kind.key(), None))
            .cover(pin.cover.clone())
            .fallback(pin.kind.icon())
            .when(pin.kind.round(), Card::circle)
            .when(self.folded(), Card::glance)
            .play(
                playing,
                cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.toggle_origin(&origin, cx));
                }),
            )
            .warm({
                let playback = self.playback.clone();
                move |_, cx| playback.update(cx, |playback, cx| playback.warm_origin(&warmed, cx))
            })
            .tint(match active {
                true => theme.foreground,
                false => theme.muted_foreground,
            })
            .meta(caption(pin.kind.key(), marked, cx))
            .lit(active && !aimed, accent)
            .chosen(aimed)
            .press(move |_, _, cx| navigate(destination.clone(), cx))
            .menu(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.track_menu.reset(cx);
                this.context_menu = Some((opened.clone(), event.position));
                cx.notify();
            }))
            .when_else(
                held,
                |card| card.pin_from(pin.clone(), Spot::new(PINS, index)),
                |card| card.pin(pin.clone()),
            )
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DraggedPin>, _, cx| {
                    let Some(gap) = drop_gap(event.bounds, event.event.position, index) else {
                        // A fast move can skip the edges, so the row lets go of its aim itself.
                        if this.drop_into.as_ref() == Some(&target) {
                            this.drop_into = None;
                            cx.notify();
                        }
                        return;
                    };
                    let into = (drop_into(event.bounds, event.event.position)
                        && this.accepts(&event.drag(cx).pin, &target, cx))
                    .then(|| target.clone());
                    let gap = match (&into, held) {
                        (Some(_), _) => None,
                        (None, true) => Some(gap),
                        (None, false) => Some(count),
                    };
                    if this.drop_gap != gap || this.drop_into != into {
                        this.drop_gap = gap;
                        this.drop_into = into;
                        cx.notify();
                    }
                }),
            );

        div()
            .id(SharedString::from(format!("pinned-slot-{key}")))
            .role(Role::ListItem)
            .aria_position_in_set(index + 1)
            .aria_size_of_set(total)
            .relative()
            .flex_none()
            .w_full()
            .min_w_0()
            .child(card)
            .when_some(edge, |this, edge| this.child(drop_marker(edge, cx)))
            .into_any_element()
    }

    /// One navigation entry. An expandable one flips its group open and shut, or on the rail
    /// opens the group's menu. Any other one navigates and is lit while its section is current.
    fn nav(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let accent = theme.sidebar_accent;
        let (entry, icon, destination) = NAV[index].clone();
        let key = entry.map_or("nav-settings", NavEntry::key);
        let current = self.trail.read(cx).current();
        let group = Group::of(&destination);
        let active = match group {
            Some(group) => Group::of(&current) == Some(group),
            None => destination.same_section(&current),
        };
        let tint = match active {
            true => theme.foreground,
            false => theme.muted_foreground,
        };
        let rail = self.folded();
        let row = match rail {
            true => nav_icon(index, key, icon, tint, accent),
            false => nav_row(index, key, tint, accent).icon(icon),
        };

        match group {
            Some(group) if rail => self.flyout(group, row.lit(active, accent), window, cx),
            Some(group) => row
                .trailing(chevron(self.opened(group)))
                .aria_expanded(self.opened(group))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.flip(group);
                    cx.notify();
                }))
                .into_any_element(),
            None => row
                .lit(active, accent)
                .when(active, |button| button.aria_selected(true))
                .on_click(move |_, _, cx| match destination.in_settings() {
                    true => open_settings(cx),
                    false => navigate(destination.clone(), cx),
                })
                .into_any_element(),
        }
    }

    /// A group's entry on the rail. `button` opens a menu just past the rail's edge that names
    /// the group and lists its tabs, each with its glyph and full name, and picking one
    /// navigates there.
    fn flyout(&self, group: Group, button: Button, window: &Window, cx: &App) -> AnyElement {
        let current = self.trail.read(cx).current();
        let name = i18n::lookup(group.key(), None);
        let gap = window.rem_size() * RAIL_INSET + BORDER + FLYOUT_GAP;
        let items = LIBRARY_TABS.into_iter().map(|(key, tab)| {
            let destination = group.at(tab);

            MenuItem::new(key, i18n::lookup(key, None))
                .icon(tab.icon())
                .selected(destination == current)
                .on_click(move |_, _, cx| navigate(destination.clone(), cx))
        });
        let menu = Menu::new(group.key())
            .name(name.clone())
            .w(Picker::NARROW)
            .beside(gap)
            .header(ui::eyebrow(name, cx).px_3().pt_1())
            .items(items);

        Popover::new(group.key(), self.popovers.clone())
            .button(button)
            .menu(menu)
            .commands()
            .w_full()
            .into_any_element()
    }

    /// The tabs under an expanded group, which only the full sidebar draws. They carry no
    /// glyphs, since the names beside them already say everything.
    fn tabs(&self, group: Group, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let accent = theme.sidebar_accent;
        let current = self.trail.read(cx).current();
        let tab = |id: ElementId, key: &'static str, destination: Destination| {
            let chosen = destination == current;
            let tint = match chosen {
                true => theme.foreground,
                false => theme.muted_foreground,
            };

            nav_row(id, key, tint, accent)
                .flex_1()
                .lit(chosen, accent)
                .when(chosen, |button| button.aria_selected(true))
                .on_click(move |_, _, cx| navigate(destination.clone(), cx))
        };

        Tabs::new()
            .items(
                LIBRARY_TABS
                    .into_iter()
                    .enumerate()
                    .map(|(slot, (name, tab_id))| {
                        let id = match group {
                            Group::Library => name.into(),
                            Group::Local => ("local-tab", slot).into(),
                        };
                        tab(id, name, group.at(tab_id))
                    }),
            )
            .into_any_element()
    }

    /// Whether `target` takes the dragged `song`: a playlist the listener may edit, on the same
    /// shelf as the song, since a provider adds only its own tracks.
    fn accepts(&self, song: &Pin, target: &Pin, cx: &App) -> bool {
        song.kind == PinKind::Song
            && target.kind == PinKind::Playlist
            && Shelf::of(&song.id) == Shelf::of(&target.id)
            && self.library.read(cx).is_editable(&target.id)
    }

    /// Adds a dropped song to a playlist, and asks first when the playlist holds the song already.
    /// Shift held on the drop moves the song instead.
    fn add_into(&self, target: &Pin, song: &Pin, window: &mut Window, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        let Some(playlist) = library.playlist(&target.id).cloned() else {
            return;
        };
        let track = song.id.clone();
        let leaving = window
            .modifiers()
            .shift
            .then(|| self.leaves(song, &playlist.id, cx))
            .flatten();
        match library.holds(&playlist.id, &track) == Some(true) {
            true => PlaylistEditor::open(
                Edit::Again {
                    playlist,
                    track,
                    leaving,
                },
                window,
                cx,
            ),
            false => self.library.update(cx, |library, cx| match leaving {
                Some(leaving) => library.move_to_playlist(playlist.id, track, leaving, cx),
                None => library.add_to_playlist(playlist.id, track, cx),
            }),
        }
    }

    /// What a moved song leaves. The favorites count only on a `Shape::Saved` shelf, since a
    /// catalog page lists every song.
    fn leaves(&self, song: &Pin, target: &str, cx: &App) -> Option<Leaving> {
        let library = self.library.read(cx);
        match leaving(song.origin.as_deref()?) {
            Leaving::Liked => {
                (library.shape(Shelf::Streaming) == Shape::Saved).then_some(Leaving::Liked)
            }
            Leaving::Playlist(id) => {
                (id != target && library.is_editable(&id)).then_some(Leaving::Playlist(id))
            }
        }
    }

    fn persist(&self, cx: &mut Context<Self>) {
        let width = self.width / px(1.);
        let open = self.open;
        let rail = self.rail;
        self.settings.update(cx, |settings, cx| {
            settings.set_sidebar(width, open, rail, cx)
        });
    }
}

impl Swells for SidebarLeft {
    fn swell(&mut self) -> &mut EdgeSwell {
        &mut self.swell
    }

    fn swell_bar(&self) -> &Entity<ui::Scrollbar> {
        &self.scrollbar
    }

    fn swelling(&self, _cx: &App) -> bool {
        self.is_open()
    }
}

impl Render for SidebarLeft {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let sidebar_bg = theme.sidebar;
        let sidebar_border = theme.sidebar_border;

        let current = self.trail.read(cx).current();
        self.follow(&current);
        self.adapt(super::Chrome::reserved_right(cx), window, cx);

        if !cx.has_active_drag() {
            self.dropping = false;
            self.drop_gap = None;
            self.drop_into = None;
            self.settle(cx);
        }
        let closing = self.closing;

        let mut rows = self.navigation(window, cx);
        rows.extend(self.pins(window, cx));

        let overlaid = self.overlays();
        let rail = self.folded();
        // A group's menu belongs to the rail, so one left open must not come back with it.
        if !rail
            && Group::ALL
                .iter()
                .any(|group| self.popovers.shows(group.key()))
        {
            self.popovers.close();
        }
        // The edge drags past the rail to zero, so a drag can close the sidebar.
        let panel = Panel::new("sidebar-left", Side::Left, self.breadth())
            .limits(Pixels::ZERO, MAX_WIDTH)
            .reach(self.ceiling(window, cx))
            .grip_above(super::Chrome::bottom(cx))
            .on_resize(cx.listener(|this, width: &Pixels, _, cx| this.resize(*width, cx)))
            .on_drag_move(cx.listener(|this, _: &DragMoveEvent<DraggedPin>, _, cx| {
                if this.dropping {
                    return;
                }
                this.dropping = true;
                // A drag has nowhere to land while the section is closed, so open it.
                if !this.pinned_open {
                    this.pinned_open = true;
                    this.settings.update(cx, |settings, cx| {
                        settings.set_sidebar_pinned_open(true, cx)
                    });
                }
                cx.notify();
            }))
            .on_drop(cx.listener(|this, dragged: &DraggedPin, window, cx| {
                let gap = this.drop_gap.take();
                this.dropping = false;
                match this.drop_into.take() {
                    Some(playlist) => this.add_into(&playlist, &dragged.pin, window, cx),
                    None => {
                        let pin = dragged.pin.clone().origin(None);
                        this.pins.update(cx, |pins, cx| pins.place(pin, gap, cx));
                    }
                }
                cx.notify();
            }))
            .when(!self.is_open(), |this| this.hidden())
            .when(!theme.transparent, |this| this.bg(sidebar_bg))
            .border_color(sidebar_border)
            // A sidebar dragged shut keeps only its edge until the drag lets go.
            .when(closing, |this| this.border_r_0())
            .when(overlaid, |this| {
                this.occlude().absolute().left_0().top_0().bottom_0()
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(closing, |this| this.hidden())
                    .on_drag_move(
                        cx.listener(|this, event: &DragMoveEvent<DraggedPin>, _, cx| {
                            let bar = this.scrollbar.clone();
                            let bottom = super::Chrome::bottom(cx);
                            this.swell
                                .hold(&bar, event.bounds, event.event.position, bottom, cx);
                        }),
                    )
                    .child(
                        Scroller::new("sidebar-left-rows", &self.scrollbar)
                            .bottom_inset(super::Chrome::bottom(cx))
                            .size_full()
                            .child(
                                div()
                                    .id("sidebar-navigation")
                                    .role(Role::Navigation)
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .w_full()
                                    .py_3()
                                    .map(|column| match rail {
                                        true => column.px_2(),
                                        false => column.px_3(),
                                    })
                                    // Leave the last row somewhere to go, clear of the
                                    // button that floats over the bottom.
                                    .pb(ui::perch_room(cx))
                                    .children(rows),
                            ),
                    )
                    .children(
                        ui::return_top("sidebar-return-top", &self.scrollbar, cx).map(|perch| {
                            match super::Chrome::bottom(cx) > Pixels::ZERO {
                                true => perch
                                    .bottom(super::Chrome::bottom(cx) + theme.metrics.pad * 1.5),
                                false => perch,
                            }
                        }),
                    ),
            )
            .children(self.menu(cx));

        match overlaid {
            false => panel.into_any_element(),
            true => div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .child(
                    Shield::new("sidebar-shield")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, cx| this.dismiss(cx)),
                        ),
                )
                .child(panel)
                .into_any_element(),
        }
    }
}

/// How wide the left sidebar stands from its saved width, which the settings rail matches.
pub(crate) fn sidebar_width(settings: &AppSettings) -> Pixels {
    px(settings.sidebar_width()).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// One pinned row's width plus the rail's padding and border.
fn rail_width(window: &Window, cx: &App) -> Pixels {
    let row = cx.theme().metrics.list_row;
    ui::snapped(row + window.rem_size() * (RAIL_INSET * 2.) + BORDER, window)
}

/// The kind of an entry, behind a dimmed pin when it is one of the pinned ones.
fn caption(kind: &'static str, marked: bool, cx: &App) -> impl IntoElement {
    let theme = *cx.theme();

    div()
        .flex()
        .items_center()
        .gap(theme.metrics.pad / 4.)
        .min_w_0()
        .when(marked, |row| {
            row.child(
                svg()
                    .path(icons::path("icons/pin.svg"))
                    .flex_none()
                    .size(theme.text(Text::Tiny))
                    .text_color(theme.primary.opacity(PIN_MARK)),
            )
        })
        .child(div().min_w_0().truncate().child(i18n::lookup(kind, None)))
}

fn hint(cx: &App) -> AnyElement {
    let theme = *cx.theme();

    div()
        .flex()
        .flex_none()
        .min_w_0()
        .items_center()
        .justify_center()
        // A narrow sidebar wraps the line, so the box grows rather than the text spilling out.
        .min_h(HINT_HEIGHT)
        .mx_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .border_1()
        .border_dashed()
        .border_color(theme.sidebar_border)
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .text_center()
        .child(ui::spoken("sidebar-pin-hint", i18n::lookup("nav-pin-hint", None)).min_w_0())
        .into_any_element()
}

fn vacancy() -> AnyElement {
    Vacancy::new(i18n::lookup("nav-nothing-pinned", None))
        .spoken("sidebar-nothing-pinned")
        .icon("icons/pin.svg")
        .compact()
        .flex_none()
        .h(VACANCY_HEIGHT)
        .px_2()
        .into_any_element()
}

/// Which groups the route opens: Your Library and Local Music, in that order.
fn expanded(current: &Destination) -> (bool, bool) {
    (
        matches!(current, Destination::Library(_)),
        matches!(current, Destination::Local(_)),
    )
}

fn chevron(open: bool) -> &'static str {
    match open {
        true => "icons/chevron-down.svg",
        false => "icons/chevron-right.svg",
    }
}

#[cfg(test)]
mod tests {
    use router::{Destination, LibraryTab, SettingsPage};

    use super::expanded;

    #[test]
    fn a_section_expands_only_where_it_leads() {
        assert_eq!(
            expanded(&Destination::Library(LibraryTab::Albums)),
            (true, false)
        );
        assert_eq!(
            expanded(&Destination::Local(LibraryTab::Albums)),
            (false, true)
        );
        assert_eq!(
            expanded(&Destination::Settings(SettingsPage::Providers)),
            (false, false)
        );
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
            assert_eq!(expanded(&destination), (false, false), "{destination:?}");
        }
    }
}
