use gpui::prelude::*;
use gpui::{Bounds, Context, ElementId, Hsla, Pixels, Point, Window, px};
use router::NavEntry;
use state::AppSettings;
use ui::{Button, Ghost};

/// How far past its own edge a row answers a drag, which covers the space between two rows
/// and stays inside the half of the next row that names the same gap.
const REACH: Pixels = px(8.);

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
    save_order(&order, settings, cx);
}

/// Moves `entry` one place up or down among every movable row, hidden ones included, which is
/// what a key press does where a drag would. Does nothing at either end of the list.
pub(crate) fn shift_entry(
    entry: NavEntry,
    delta: isize,
    settings: &mut AppSettings,
    cx: &mut Context<AppSettings>,
) {
    let mut order = nav_order(settings);
    let Some(from) = order.iter().position(|known| *known == entry) else {
        return;
    };
    let Some(to) = from
        .checked_add_signed(delta)
        .filter(|to| *to < order.len())
    else {
        return;
    };
    order.swap(from, to);
    save_order(&order, settings, cx);
}

/// Whether a drag at `position` is for the row at `place` among `count`, drawn in `bounds`:
/// the cursor is over the row or in the space between it and a neighbour. Two rows reached at
/// once name the same gap, so the target never goes stale between rows. Nothing is reached past
/// either end of the list, where the list itself clears the target.
pub(crate) fn reaches(
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
    place: usize,
    count: usize,
) -> bool {
    let above = match place > 0 {
        true => REACH,
        false => Pixels::ZERO,
    };
    let below = match place + 1 < count {
        true => REACH,
        false => Pixels::ZERO,
    };

    position.x >= bounds.left()
        && position.x <= bounds.right()
        && position.y >= bounds.top() - above
        && position.y <= bounds.bottom() + below
}

/// The gap a row dragged over the row at `index` would leave, unless that gap is where the
/// dragged row already sits in `shown`, where a drop would change nothing.
pub(crate) fn useful_gap(gap: usize, dragged: NavEntry, shown: &[NavEntry]) -> Option<usize> {
    match shown.iter().position(|entry| *entry == dragged) {
        Some(from) if gap == from || gap == from + 1 => None,
        _ => Some(gap),
    }
}

fn save_order(order: &[NavEntry], settings: &mut AppSettings, cx: &mut Context<AppSettings>) {
    settings.set_nav_order(
        order.iter().map(|entry| entry.id().to_owned()).collect(),
        cx,
    );
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
