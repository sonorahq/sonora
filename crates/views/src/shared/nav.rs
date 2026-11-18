use gpui::prelude::*;
use gpui::{Context, ElementId, Hsla, Pixels, Point, Window};
use router::NavEntry;
use state::AppSettings;
use ui::{Button, Ghost};

/// The rows the sidebar lists above its pinned section, in the order they take until the
/// listener drags one elsewhere. The pinned section always closes the list, so it is not here.
pub(crate) const MOVABLE: [NavEntry; 8] = [
    NavEntry::Home,
    NavEntry::Search,
    NavEntry::History,
    NavEntry::Settings,
    NavEntry::Songs,
    NavEntry::Albums,
    NavEntry::Artists,
    NavEntry::Playlists,
];

/// A sidebar row being dragged to a new place, from the sidebar or from Settings. It draws as
/// a chip under the cursor and changes the saved order only when dropped.
#[derive(Clone)]
pub(crate) struct DraggedNav {
    pub entry: NavEntry,
    /// Where the cursor grabbed the row, so the chip starts under it.
    position: Point<Pixels>,
}

impl DraggedNav {
    pub fn new(entry: NavEntry) -> Self {
        Self {
            entry,
            position: Point::default(),
        }
    }

    /// The drag payload `on_drag` builds its chip from, placed where the cursor grabbed it.
    pub fn at(&self, position: Point<Pixels>) -> Self {
        Self {
            position,
            ..self.clone()
        }
    }
}

impl Render for DraggedNav {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Ghost::new(self.position, i18n::lookup(self.entry.key(), None))
    }
}

/// A navigation row as the sidebars draw it: a ghost button with its label at the start, in
/// `tint`, that takes `accent` while hovered or pressed and lets it fade once the pointer
/// leaves. The caller adds the icon and lights the row it considers current with
/// `.lit(true, accent)`.
pub(crate) fn nav_row(
    id: impl Into<ElementId>,
    key: &'static str,
    tint: Hsla,
    accent: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .label(i18n::lookup(key, None))
        .tint(tint)
        .gap_2p5()
        .justify_start()
        .lit(false, accent)
        .active(move |style| style.bg(accent))
}

/// A navigation row reduced to its icon, named by a tooltip beside it.
pub(crate) fn nav_icon(
    id: impl Into<ElementId>,
    key: &'static str,
    icon: &'static str,
    tint: Hsla,
    accent: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .icon(icon)
        .tooltip_trailing(key)
        .tint(tint)
        .w_full()
        .lit(false, accent)
        .active(move |style| style.bg(accent))
}

/// Whether the sidebar shows `entry`. An optional row shows once the listener adds it, any other
/// until they hide it.
pub(crate) fn entry_shown(entry: NavEntry, settings: &AppSettings) -> bool {
    match entry.optional() {
        true => settings.nav_added(entry.id()),
        false => settings.nav_shown(entry.id()),
    }
}

/// Shows or hides `entry` in the sidebar, the counterpart of `entry_shown`.
pub(crate) fn set_entry_shown(
    entry: NavEntry,
    shown: bool,
    settings: &mut AppSettings,
    cx: &mut Context<AppSettings>,
) {
    match entry.optional() {
        true => settings.set_nav_added(entry.id(), shown, cx),
        false => settings.set_nav_shown(entry.id(), shown, cx),
    }
}

/// Every movable row, hidden ones included, in the order the listener left them.
pub(crate) fn nav_order(settings: &AppSettings) -> Vec<NavEntry> {
    reconcile(settings.nav_order())
}

/// Moves `entry` in front of `before`, or to the end when `before` is `None`. A gap is counted
/// among the rows a list shows, so the caller names the row after it rather than its index.
pub(crate) fn place_entry(
    entry: NavEntry,
    before: Option<NavEntry>,
    settings: &mut AppSettings,
    cx: &mut Context<AppSettings>,
) {
    let order = moved(nav_order(settings), entry, before);
    settings.set_nav_order(
        order.iter().map(|entry| entry.id().to_owned()).collect(),
        cx,
    );
}

/// The gap a row dragged over the row at `index` would leave, unless that gap is where the
/// dragged row already sits in `shown`, where a drop would change nothing.
pub(crate) fn useful_gap(gap: usize, dragged: NavEntry, shown: &[NavEntry]) -> Option<usize> {
    match shown.iter().position(|entry| *entry == dragged) {
        Some(from) if gap == from || gap == from + 1 => None,
        _ => Some(gap),
    }
}

/// The saved ids as rows, skipping any that are gone or repeated, with rows the save does not
/// name yet appended in their default order.
fn reconcile(saved: &[String]) -> Vec<NavEntry> {
    let mut order: Vec<NavEntry> = Vec::with_capacity(MOVABLE.len());
    for entry in saved.iter().filter_map(|id| NavEntry::from_id(id)) {
        if MOVABLE.contains(&entry) && !order.contains(&entry) {
            order.push(entry);
        }
    }
    for entry in MOVABLE {
        if !order.contains(&entry) {
            order.push(entry);
        }
    }
    order
}

/// `order` with `entry` taken out and put back in front of `before`, or last.
fn moved(mut order: Vec<NavEntry>, entry: NavEntry, before: Option<NavEntry>) -> Vec<NavEntry> {
    if before == Some(entry) {
        return order;
    }
    order.retain(|known| *known != entry);
    let at = before
        .and_then(|before| order.iter().position(|known| *known == before))
        .unwrap_or(order.len());
    order.insert(at, entry);
    order
}
