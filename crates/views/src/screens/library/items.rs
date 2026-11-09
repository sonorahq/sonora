//! The machinery the albums, playlists and artists pages of a shelf share: the rows a search,
//! the filters and the sort leave, worked out once per change and kept in the page's state; the
//! table they list in; and the grid of cards they show as, virtualised by rows of cards with a
//! heading wherever the sort starts a new group.
//!
//! Each kind of item answers for itself through [`Kind`]: where its items live, how they match,
//! sort and group, what they filter by, and how a cell and a card draw one.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use router::Destination;
use rune::{Drag, El, GrowthSpec, PointerPress, Tx, col};
use state::{AppSettings, FilterValue, Origin, Shelf};
use ui::{
    Card, ColumnSpec, Filter, FilterChange, Mode, Pending, Pin, Sort, SortAxis, Sorting, Table,
    TableCell, TableState,
};

use super::Section;
use crate::Ui;
use crate::chrome::sidebar_left;
use crate::shared::album_grid::{CardLayout, card_row};
use crate::shared::cells::{self, Act};
use crate::shared::menus::{self, Target};
use crate::shared::page::{self, rems};

/// The room between two rows of cards, and between a group's heading and its cards, in rems.
const ROW_GAP: f32 = 1.5;
/// The room over a group's heading, in rems.
const HEAD_TOP: f32 = 0.5;

/// One kind of item a shelf lists on a page of its own.
pub(crate) trait Kind: Sized + 'static {
    type Item: Clone + 'static;
    type Field: Copy + PartialEq + 'static;
    /// What the filter popover narrows the items by.
    type Sieve: Copy + Default + PartialEq + 'static;
    /// What a kind caches between views, such as the years its filter steps through.
    type Memo: Default + 'static;

    /// The page the items list on.
    const SECTION: Section;
    /// What the keys of the page's elements start with.
    const NAME: &'static str;
    const COLUMNS: &'static [ColumnSpec<Self::Field>];
    /// The column the list sorts by until the listener picks another, newest first.
    const FIRST: &'static str;
    /// Every filter axis the kind offers, by key, so stored filters find theirs.
    const AXES: &'static [&'static str];

    /// The shelf's items, in the order the store keeps them.
    fn items(ui: &Ui, shelf: Shelf) -> &[Self::Item];
    fn list(ui: &Ui, shelf: Shelf) -> &Items<Self>;
    fn list_mut(ui: &mut Ui, shelf: Shelf) -> &mut Items<Self>;

    fn id(item: &Self::Item) -> &str;
    /// What a screen reader calls the item's row.
    fn name(item: &Self::Item) -> &str;
    /// Whether the item holds `query`, already lowercase and trimmed.
    fn hits(item: &Self::Item, query: &str) -> bool;
    /// Whether the item passes the filters.
    fn keeps(ui: &Ui, shelf: Shelf, sieve: &Self::Sieve, item: &Self::Item) -> bool;
    /// A hash of what [`Kind::keeps`] reads besides the item itself (its heart, its download),
    /// so the rows follow a change there. Zero while nothing narrows by them.
    fn facts(ui: &Ui, shelf: Shelf, sieve: &Self::Sieve, items: &[Self::Item]) -> u64;
    fn compare(a: &Self::Item, b: &Self::Item, field: Self::Field) -> Ordering;
    /// The heading a sort by `field` files the item under, for a field that groups.
    fn group(item: &Self::Item, field: Self::Field) -> Option<String>;
    /// The axes the filter popover offers.
    fn axes(ui: &Ui, shelf: Shelf, sieve: &Self::Sieve) -> Vec<Filter>;
    /// Applies a change from the filter popover. True when it named one of the kind's axes.
    fn filter(sieve: &mut Self::Sieve, change: FilterChange) -> bool;
    /// Whether any axis narrows the items.
    fn narrowed(sieve: &Self::Sieve) -> bool;

    /// What the cell of `field` draws for the item, inside `frame`.
    fn cell(
        ui: &Ui,
        shelf: Shelf,
        item: &Self::Item,
        cell: &TableCell<Self::Field>,
        frame: El,
    ) -> El;
    /// The item's card, a tile `width` wide.
    fn card(ui: &Ui, shelf: Shelf, item: &Self::Item, key: String, width: f32) -> Card;
    /// Whether the card's art is round.
    fn round() -> bool {
        false
    }
    fn origin(item: &Self::Item) -> Origin;
    fn open(item: &Self::Item) -> Destination;
    fn menu(item: &Self::Item) -> Target;
    fn pin(item: &Self::Item) -> Option<Pin>;
    /// Takes the picked items off the shelf, asking first where that is the listener's call.
    fn remove(ui: &mut Ui, shelf: Shelf, items: Vec<Self::Item>);
}

/// What the rows on screen were worked out from. A change in any of it works them out again.
#[derive(Clone, PartialEq)]
struct Stamp<S> {
    at: usize,
    len: usize,
    sorting: Option<Sorting>,
    sieve: S,
    query: String,
    facts: u64,
}

/// The rows on screen: which item each shows, in order, and the key each is known by.
pub(crate) struct Order {
    rows: Vec<u32>,
    keys: Vec<String>,
    at: HashMap<String, u32>,
    /// Bumped each time the rows are worked out, for the table's cheap commits.
    version: u64,
}

impl Order {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// The item index of row `display`.
    pub(crate) fn source(&self, display: usize) -> Option<usize> {
        self.rows.get(display).map(|&at| at as usize)
    }

    pub(crate) fn key(&self, display: usize) -> Option<&str> {
        self.keys.get(display).map(String::as_str)
    }

    pub(crate) fn display(&self, key: &str) -> Option<usize> {
        self.at.get(key).map(|&at| at as usize)
    }
}

/// One row of the grid: a group's heading, or up to a row's worth of cards by their place in
/// the order.
enum DeckRow {
    Heading(String),
    Cards(std::ops::Range<usize>),
}

/// The rows of the grid, worked out again only when the order or the number of columns moves.
struct Deck {
    version: u64,
    columns: usize,
    rows: Rc<Vec<DeckRow>>,
}

/// A page of items' own state, kept in the screen's state: the table's selection, layout and
/// sort, the filters, and the rows they leave.
pub(crate) struct Items<K: Kind> {
    pub(crate) table: TableState,
    pub(crate) sieve: K::Sieve,
    pub(crate) memo: RefCell<K::Memo>,
    order: RefCell<Option<(Stamp<K::Sieve>, Rc<Order>)>>,
    deck: RefCell<Option<Deck>>,
    versions: Cell<u64>,
}

impl<K: Kind> Default for Items<K> {
    fn default() -> Self {
        Self {
            table: TableState::new().multi_select(true),
            sieve: K::Sieve::default(),
            memo: RefCell::default(),
            order: RefCell::new(None),
            deck: RefCell::new(None),
            versions: Cell::new(0),
        }
    }
}

impl<K: Kind> Items<K> {
    /// Puts back the layout, the sort and the filters saved under `key`, newest first when no
    /// sort was ever saved.
    pub(crate) fn restore(&mut self, settings: &AppSettings, key: &str) {
        self.table.set_layout(settings.table(key));
        self.table.set_sorting(Some(Sorting {
            column: K::FIRST.to_owned(),
            order: Sort::Descending,
        }));
        if let Some(sorting) = settings.sorting(key) {
            self.table.set_sorting(sorting);
        }
        for (axis, value) in settings.filters(key).unwrap_or_default() {
            let Some(&axis) = K::AXES.iter().find(|known| **known == axis) else {
                continue;
            };
            let change = match value {
                FilterValue::Flag(on) => FilterChange::Flag(axis, on),
                FilterValue::Range(low, high) => FilterChange::Range(axis, (low, high)),
            };
            K::filter(&mut self.sieve, change);
        }
    }
}

/// The shelf's settings key for `K`'s page.
fn key<K: Kind>(shelf: Shelf) -> &'static str {
    K::SECTION.key(shelf)
}

/// What the title bar's search field narrows the shelf by, lowercase.
pub(crate) fn query(ui: &Ui, shelf: Shelf) -> String {
    ui.chrome
        .toolbar
        .query(&super::destination(shelf))
        .trim()
        .to_lowercase()
}

/// Whether the search or a filter narrows `K`'s rows.
pub(crate) fn filtering<K: Kind>(ui: &Ui, shelf: Shelf) -> bool {
    K::narrowed(&K::list(ui, shelf).sieve) || !query(ui, shelf).is_empty()
}

/// The rows on screen, worked out again only when what they come from changed.
pub(crate) fn order<K: Kind>(ui: &Ui, shelf: Shelf) -> Rc<Order> {
    let list = K::list(ui, shelf);
    let items = K::items(ui, shelf);
    let sieve = list.sieve;
    let stamp = Stamp {
        at: items.as_ptr() as usize,
        len: items.len(),
        sorting: list.table.sorting().cloned(),
        sieve,
        query: query(ui, shelf),
        facts: K::facts(ui, shelf, &sieve, items),
    };
    if let Some((kept, order)) = list.order.borrow().as_ref()
        && *kept == stamp
    {
        return order.clone();
    }

    let mut rows: Vec<u32> = (0..items.len())
        .filter(|&at| {
            let item = &items[at];
            K::hits(item, &stamp.query) && K::keeps(ui, shelf, &sieve, item)
        })
        .map(|at| at as u32)
        .collect();
    if let Some((field, order)) = sorted::<K>(&list.table) {
        rows.sort_by(|&a, &b| {
            let ordering = K::compare(&items[a as usize], &items[b as usize], field);
            match order {
                Sort::Ascending => ordering,
                Sort::Descending => ordering.reverse(),
            }
        });
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let keys: Vec<String> = rows
        .iter()
        .map(|&at| {
            let id = K::id(&items[at as usize]);
            let times = seen.entry(id).or_default();
            *times += 1;
            match *times {
                1 => id.to_owned(),
                n => format!("{id}~{n}"),
            }
        })
        .collect();
    let at = keys
        .iter()
        .enumerate()
        .map(|(display, key)| (key.clone(), display as u32))
        .collect();
    let version = list.versions.get() + 1;
    list.versions.set(version);
    let order = Rc::new(Order {
        rows,
        keys,
        at,
        version,
    });
    *list.order.borrow_mut() = Some((stamp, order.clone()));
    order
}

/// The field and order the table sorts by, if any.
fn sorted<K: Kind>(table: &TableState) -> Option<(K::Field, Sort)> {
    let sorting = table.sorting()?;
    K::COLUMNS
        .iter()
        .find(|spec| spec.key == sorting.column)
        .map(|spec| (spec.field, sorting.order))
}

/// The item row `display` shows.
fn item_at<'a, K: Kind>(
    ui: &'a Ui,
    shelf: Shelf,
    order: &Order,
    display: usize,
) -> Option<&'a K::Item> {
    order
        .source(display)
        .and_then(|at| K::items(ui, shelf).get(at))
}

/// The items of the rows keyed `keys`, in the order given.
fn picked<'a, K: Kind>(ui: &'a Ui, shelf: Shelf, keys: &[String]) -> Vec<&'a K::Item> {
    let order = order::<K>(ui, shelf);
    keys.iter()
        .filter_map(|key| order.display(key))
        .filter_map(|display| item_at::<K>(ui, shelf, &order, display))
        .collect()
}

/// The columns the list can be sorted by, the sorted one with its order.
pub(crate) fn sort_axes<K: Kind>(ui: &Ui, shelf: Shelf) -> Vec<SortAxis> {
    let active = K::list(ui, shelf).table.sorting();
    K::COLUMNS
        .iter()
        .filter(|spec| spec.sortable)
        .map(|spec| SortAxis {
            key: spec.key,
            label: spec.label(),
            order: active
                .filter(|sorting| sorting.column == spec.key)
                .map(|sorting| sorting.order),
        })
        .collect()
}

/// Steps the column keyed `column` through its orders, as the sort picker does, and saves it.
pub(crate) fn sort<K: Kind>(ui: &mut Ui, shelf: Shelf, column: &'static str) {
    let next = K::list_mut(ui, shelf).table.cycle(K::COLUMNS, column);
    let store = &mut ui.store;
    store
        .settings
        .set_sorting(key::<K>(shelf), next, &mut store.cx);
}

/// The axes the filter popover offers, and whether anything is narrowed. A page keeps its
/// funnel while anything is, even once the axes have gone with the rows, or an empty result
/// would lock the filter that emptied it in place.
pub(crate) fn filters<K: Kind>(ui: &Ui, shelf: Shelf) -> (Vec<Filter>, bool) {
    let sieve = &K::list(ui, shelf).sieve;
    (K::axes(ui, shelf, sieve), K::narrowed(sieve))
}

/// Applies a change from the filter popover and saves the filters.
pub(crate) fn filter<K: Kind>(ui: &mut Ui, shelf: Shelf, change: FilterChange) {
    K::filter(&mut K::list_mut(ui, shelf).sieve, change);
    let axes = K::axes(ui, shelf, &K::list(ui, shelf).sieve);
    let key = key::<K>(shelf);
    let stored = ui.store.settings.filters(key).unwrap_or_default();
    let filters = page::snapshot(axes, stored);
    let store = &mut ui.store;
    store.settings.set_filters(key, filters, &mut store.cx);
}

/// The items as a table: a row per item under a sticky header, `width` wide, with room for
/// `bottom` under the last row. Double-click opens an item, a right-click its menu, Delete
/// takes the picked ones off the shelf, and a row dragged to the sidebar pins it.
pub(crate) fn table<K: Kind>(ui: &Ui, shelf: Shelf, width: f32, bottom: f32, waiting: bool) -> El {
    let theme = &ui.theme;
    let order = order::<K>(ui, shelf);
    let name = viewport::<K>(shelf, Mode::List);
    let count = order.len();
    let ids = order.clone();
    let mut table = Table::<Ui, K::Field>::new(
        name.clone(),
        K::COLUMNS,
        move |ui: &Ui| &K::list(ui, shelf).table,
        move |ui: &mut Ui| &mut K::list_mut(ui, shelf).table,
        move |_: &Ui| count,
        move |_: &Ui, at: usize| Cow::Owned(ids.key(at).unwrap_or_default().to_owned()),
    )
    .width(width)
    .version(order.version)
    .bottom(bottom)
    .pending(waiting.then_some(Pending::Screen))
    .label(i18n::lookup(K::SECTION.title(), None).into_owned());

    let drawn = (order.clone(), name.clone());
    table = table.cell(move |ui: &Ui, cell: &TableCell<K::Field>| {
        let (order, table) = &drawn;
        let key = order.key(cell.row).unwrap_or_default();
        let frame = cells::frame(cell, &format!("{table}/{key}"));
        match item_at::<K>(ui, shelf, order, cell.row) {
            Some(item) => K::cell(ui, shelf, item, cell, frame),
            None => frame,
        }
    });
    let named = order.clone();
    table = table.name(move |ui: &Ui, at: usize| {
        item_at::<K>(ui, shelf, &named, at)
            .map(|item| K::name(item).to_owned())
            .unwrap_or_default()
    });
    let lit = order.clone();
    table = table.playing(move |ui: &Ui, at: usize| {
        item_at::<K>(ui, shelf, &lit, at).is_some_and(|item| {
            ui.store
                .playback
                .playing_from(&K::origin(item), &ui.store.queue)
                .is_some()
        })
    });
    table = table
        .on_open(move |ui: &mut Ui, id: &str, _: &mut Tx| {
            let Some(to) = picked::<K>(ui, shelf, &[id.to_owned()])
                .first()
                .map(|item| K::open(item))
            else {
                return;
            };
            ui.go(to);
        })
        .on_context(
            move |ui: &mut Ui, keys: Vec<String>, at: PointerPress, _: &mut Tx| {
                let Some(target) = picked::<K>(ui, shelf, &keys)
                    .first()
                    .map(|item| K::menu(item))
                else {
                    return;
                };
                menus::open(ui, target, at);
            },
        )
        .on_remove(move |ui: &mut Ui, keys: Vec<String>, _: &mut Tx| {
            let items: Vec<K::Item> = picked::<K>(ui, shelf, &keys).into_iter().cloned().collect();
            K::remove(ui, shelf, items);
            ui.settle();
        })
        .on_row_drag(
            move |ui: &mut Ui, keys: &[String], drag: &mut Drag, tx: &mut Tx| {
                let Some(pin) = picked::<K>(ui, shelf, keys)
                    .first()
                    .and_then(|item| K::pin(item))
                else {
                    return;
                };
                sidebar_left::carry(ui, &pin, drag, tx);
            },
        )
        .on_sort(move |ui: &mut Ui, sorting: Option<Sorting>, _: &mut Tx| {
            let store = &mut ui.store;
            store
                .settings
                .set_sorting(key::<K>(shelf), sorting, &mut store.cx);
        })
        .on_layout(move |ui: &mut Ui, _: &mut Tx| {
            let layout = K::list(ui, shelf).table.layout().clone();
            let store = &mut ui.store;
            store
                .settings
                .set_table(key::<K>(shelf), layout, &mut store.cx);
        });
    table.build(ui, theme)
}

/// The items as a grid of cards, virtualised by rows of cards: only the rows near the view are
/// built. A sort by a field that groups (a name's initial, a year) heads each group. Make it a
/// direct child of the page's scroll viewport; `available` is the width inside `inset` at each
/// side.
pub(crate) fn grid<K: Kind>(ui: &Ui, shelf: Shelf, available: f32, inset: f32, bottom: f32) -> El {
    let theme = &ui.theme;
    let order = order::<K>(ui, shelf);
    let layout = CardLayout::new(available);
    let rows = deck::<K>(ui, shelf, &order, layout.columns);
    let row_gap = rems(theme, ROW_GAP);
    let height = Card::tile_height(layout.card, theme) + row_gap;
    let count = rows.len();
    let total = order.len();
    let named = (order.clone(), rows.clone());
    let built = (order, rows);
    col()
        .key(format!("library-{}-deck", K::NAME))
        .w_full()
        .px(inset)
        .pb(bottom)
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            height,
            move |_: &Ui, at: usize| {
                let (order, rows) = &named;
                Cow::Owned(match rows.get(at) {
                    Some(DeckRow::Heading(label)) => format!("head-{label}"),
                    Some(DeckRow::Cards(range)) => {
                        format!("row-{}", order.key(range.start).unwrap_or_default())
                    }
                    None => format!("gone-{at}"),
                })
            },
            move |ui: &Ui, at: usize| {
                let (order, rows) = &built;
                let theme = &ui.theme;
                let last = at + 1 == rows.len();
                match rows.get(at) {
                    Some(DeckRow::Heading(label)) => {
                        col().w_full().pt(rems(theme, HEAD_TOP)).pb(row_gap).child(
                            ui::spoken_heading(format!("library-group-{label}"), label, 2, theme)
                                .truncate(),
                        )
                    }
                    Some(DeckRow::Cards(range)) => {
                        let cards = range.clone().filter_map(|display| {
                            let item = item_at::<K>(ui, shelf, order, display)?;
                            Some(card::<K>(ui, shelf, item, layout.card, display, total))
                        });
                        let line = card_row(layout, cards);
                        match last {
                            true => line,
                            false => col().w_full().pb(row_gap).child(line),
                        }
                    }
                    None => col(),
                }
            },
        )
}

/// Skeleton cards laid out like the grid that will replace them, enough rows to fill a viewport
/// `tall`. Round ones for a kind whose cards are round.
pub(crate) fn pending_grid<K: Kind>(ui: &Ui, available: f32, inset: f32, tall: f32) -> El {
    let theme = &ui.theme;
    let layout = CardLayout::new(available);
    let row_gap = rems(theme, ROW_GAP);
    let tile = Card::tile_height(layout.card, theme);
    let rows = (tall / (tile + row_gap)).ceil().max(1.) as usize;
    col()
        .key(format!("library-{}-pending", K::NAME))
        .w_full()
        .px(inset)
        .gap(row_gap)
        .a11y_hidden()
        .children((0..rows).map(|row| {
            card_row(
                layout,
                (0..layout.columns).map(|column| {
                    let card = Card::skeleton(format!(
                        "library-pending-{}",
                        row * layout.columns + column
                    ))
                    .tile(layout.card);
                    match K::round() {
                        true => card.circle(),
                        false => card,
                    }
                    .build(theme)
                }),
            )
        }))
}

/// How many rows the order shows, for a page deciding what to say about an empty one.
pub(crate) fn count<K: Kind>(ui: &Ui, shelf: Shelf) -> usize {
    order::<K>(ui, shelf).len()
}

/// One card of the grid, numbered in the whole list, which a drag carries to the sidebar's pins.
fn card<K: Kind>(
    ui: &Ui,
    shelf: Shelf,
    item: &K::Item,
    width: f32,
    display: usize,
    total: usize,
) -> El {
    let key = format!("library-{}-{}", K::NAME, K::id(item));
    let card = K::card(ui, shelf, item, key, width)
        .flat()
        .build(&ui.theme)
        .position_in_set(display + 1, total);
    match K::pin(item) {
        Some(pin) => card.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            sidebar_left::carry(ui, &pin, drag, tx)
        }),
        None => card,
    }
}

/// The rows of the grid for `columns` cards a row: cards packed in order, a heading starting a
/// new row wherever the group changes.
fn deck<K: Kind>(ui: &Ui, shelf: Shelf, order: &Rc<Order>, columns: usize) -> Rc<Vec<DeckRow>> {
    let list = K::list(ui, shelf);
    if let Some(deck) = list.deck.borrow().as_ref()
        && deck.version == order.version
        && deck.columns == columns
    {
        return deck.rows.clone();
    }
    let field = sorted::<K>(&list.table).map(|(field, _)| field);
    let items = K::items(ui, shelf);
    let mut rows = Vec::new();
    let mut start = 0;
    let mut group: Option<String> = None;
    for display in 0..order.len() {
        let label = field.and_then(|field| {
            order
                .source(display)
                .and_then(|at| items.get(at))
                .and_then(|item| K::group(item, field))
        });
        if let Some(text) = &label
            && group.as_ref() != Some(text)
        {
            if start < display {
                rows.push(DeckRow::Cards(start..display));
            }
            rows.push(DeckRow::Heading(text.clone()));
            start = display;
        }
        group = label;
        if display + 1 - start == columns {
            rows.push(DeckRow::Cards(start..display + 1));
            start = display + 1;
        }
    }
    if start < order.len() {
        rows.push(DeckRow::Cards(start..order.len()));
    }
    let rows = Rc::new(rows);
    *list.deck.borrow_mut() = Some(Deck {
        version: order.version,
        columns: columns.max(1),
        rows: rows.clone(),
    });
    rows
}

/// The press at the head of an item's row: pauses or resumes while the player plays from it,
/// starts it otherwise.
pub(crate) fn index_cell(ui: &Ui, origin: Origin, cell_row: usize, frame: El) -> El {
    let theme = &ui.theme;
    let playing = ui.store.playback.playing_from(&origin, &ui.store.queue);
    let press: Act = Rc::new(move |ui: &mut Ui, _: &mut Tx| {
        ui.store.toggle_collection(&origin);
        ui.settle();
    });
    cells::index(
        frame,
        playing,
        true,
        (cell_row + 1).to_string(),
        Some(press),
        theme,
    )
}

/// The key of the scroll viewport `K`'s page lists in on `shelf` in `mode`: the table itself,
/// or the column the grid scrolls in.
pub(crate) fn viewport<K: Kind>(shelf: Shelf, mode: Mode) -> String {
    match mode {
        Mode::List => format!("library-{}-{}", K::NAME, shelf_name(shelf)),
        Mode::Grid => format!("library-{}-{}-grid", K::NAME, shelf_name(shelf)),
    }
}

/// How a shelf is named in keys.
pub(crate) fn shelf_name(shelf: Shelf) -> &'static str {
    match shelf {
        Shelf::Streaming => "streaming",
        Shelf::Local => "local",
    }
}
