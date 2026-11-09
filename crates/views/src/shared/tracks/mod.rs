//! The track table every screen lists songs in: a [`ui::Table`] over a slice of tracks the
//! store holds, with Sonora's columns, its search and filters, the playing row, hearts and
//! download marks, and playback through the store.
//!
//! A screen keeps a [`TrackList`] in its state and describes where its tracks live with a
//! [`TrackRows`]. [`TrackTable`] draws them. The order on screen (sorted, searched, filtered)
//! is worked out once per change of the tracks, the sort, the search or the filters, and kept
//! in the `TrackList`, so a view that runs again for anything else costs nothing per row: a
//! list of 10,000 songs builds the thirty rows on screen.

mod columns;
mod sieve;
mod sort;

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use i18n::t;
use music::Track;
use router::Destination;
use rune::{Drag, El, PointerPress, Tx, row};
use state::{AppSettings, Origin, Progress, Shelf};
use ui::{
    ArrowRing, Button, ColumnSpec, Filter, FilterChange, FlagAxis, Pending, RangeAxis, Sorting,
    Table, TableCell, TableState, Unit,
};

use crate::Ui;
use crate::shared::cells::{self, Act, Title};
use crate::shared::page;

pub(crate) use columns::{
    ALBUM_COLUMNS, HISTORY_COLUMNS, LIBRARY_COLUMNS, TrackField, album_columns, artist_columns,
    playlist_columns,
};
pub(crate) use sieve::TrackSieve;
pub(crate) use sort::initial;

/// How strongly an unsaved track's heart shows on its row.
const SETTLED: f32 = 0.5;
/// How strongly the text of a row that cannot play now shows.
const FADED: f32 = 0.5;

/// Finds a list's tracks in the window state, in the order the store keeps them.
pub(crate) type Source = Rc<dyn for<'a> Fn(&'a Ui) -> &'a [Track]>;
type ListFn = Rc<dyn for<'a> Fn(&'a Ui) -> &'a TrackList>;
type ListMutFn = Rc<dyn for<'a> Fn(&'a mut Ui) -> &'a mut TrackList>;
/// A right-click on rows: the tracks it picked and where, in window points, to open a menu.
pub(crate) type OnContext = Rc<dyn Fn(&mut Ui, Picked, PointerPress, &mut Tx)>;
/// Delete pressed on picked rows.
pub(crate) type OnRemove = Rc<dyn Fn(&mut Ui, Picked, &mut Tx)>;
/// Every phase of a drag that starts on a row, with the tracks it carries.
pub(crate) type OnDrag = Rc<dyn Fn(&mut Ui, &[Track], &mut Drag, &mut Tx)>;
/// A right-click on the header, where a column menu opens.
pub(crate) type OnHead = Rc<dyn Fn(&mut Ui, PointerPress, &mut Tx)>;

/// Rows a right-click or Delete picked: the tracks in the order they stand, their row keys
/// (as the table's selection names them), and which of the album and artist columns show, so
/// an adaptive menu can leave out what the row already links to.
pub(crate) struct Picked {
    pub(crate) tracks: Vec<Track>,
    pub(crate) keys: Vec<String>,
    pub(crate) album: bool,
    pub(crate) artists: bool,
}

/// What the number at the head of a row counts.
#[derive(Clone, Copy, PartialEq)]
enum Numbering {
    /// The row's place in the list, for anything that is not an album in order.
    Listing,
    /// The track's own number on the album.
    Track,
    /// The track's number behind its disc, for an album that spans more than one.
    Disc,
}

/// The shortest and longest track of a source, by where the source was and how long it was.
type Spread = (usize, usize, Option<(f32, f32)>);

/// What the rows on screen were worked out from. A change in any of it works them out again.
#[derive(Clone, PartialEq)]
struct Stamp {
    /// Where the source's tracks live and how many there are: a store that replaces its rows
    /// moves them.
    at: usize,
    len: usize,
    sorting: Option<(TrackField, ui::Sort)>,
    sieve: TrackSieve,
    query: String,
    /// The hearts and downloads of the source's tracks, while a filter reads them.
    facts: u64,
}

/// The rows on screen: which source track each shows, in order, and the key each is known by.
pub(crate) struct Order {
    stamp: Stamp,
    /// The source index of each row on screen.
    rows: Vec<u32>,
    /// Each row's key: the track's id, made unique where a list holds a song twice.
    keys: Vec<String>,
    /// Where each key stands.
    at: HashMap<String, u32>,
    /// Bumped each time the rows are worked out, for the table's cheap commits.
    version: u64,
}

impl Order {
    /// How many rows show.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The source index of row `display`.
    pub(crate) fn source(&self, display: usize) -> Option<usize> {
        self.rows.get(display).map(|&at| at as usize)
    }

    /// The key of row `display`.
    pub(crate) fn key(&self, display: usize) -> Option<&str> {
        self.keys.get(display).map(String::as_str)
    }

    /// Where the row keyed `key` stands.
    pub(crate) fn display(&self, key: &str) -> Option<usize> {
        self.at.get(key).map(|&at| at as usize)
    }
}

/// A track table's own state, kept in its screen's state: the table's selection, column
/// layout and sort, the search and the filters, and the rows they leave.
pub(crate) struct TrackList {
    pub(crate) table: TableState,
    sieve: TrackSieve,
    query: String,
    order: RefCell<Option<Rc<Order>>>,
    versions: Cell<u64>,
    /// The shortest and longest track, by where the source was and how long.
    spread: Cell<Option<Spread>>,
}

impl Default for TrackList {
    fn default() -> Self {
        Self {
            table: TableState::new().multi_select(true),
            sieve: TrackSieve::default(),
            query: String::new(),
            order: RefCell::new(None),
            versions: Cell::new(0),
            spread: Cell::new(None),
        }
    }
}

impl TrackList {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The search the rows are narrowed by.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// Narrows the rows to those whose title, artists, album or contributor hold `query`.
    pub(crate) fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_lowercase();
    }

    pub(crate) fn sieve(&self) -> &TrackSieve {
        &self.sieve
    }

    /// Whether a filter narrows the rows.
    pub(crate) fn filtered(&self) -> bool {
        self.sieve.active()
    }

    /// Applies a change from the filter popover. True when it named one of this list's axes.
    pub(crate) fn filter(&mut self, change: FilterChange) -> bool {
        let sieve = &mut self.sieve;
        match change {
            FilterChange::Range("filter-duration", value) => sieve.duration = Some(value),
            FilterChange::Flag("filter-hide-explicit", value) => sieve.hide_explicit = value,
            FilterChange::Flag("filter-playable", value) => sieve.playable = value,
            FilterChange::Flag("filter-favorites", value) => sieve.favorites = value,
            FilterChange::Flag("filter-downloaded", value) => sieve.downloaded = value,
            FilterChange::Reset => *sieve = TrackSieve::default(),
            _ => return false,
        }
        true
    }

    /// Puts back the column layout, the sort and the filters saved under `key`, as the page
    /// opens. Call it from a handler, such as the one that arrives at the page.
    pub(crate) fn restore(&mut self, settings: &AppSettings, key: &str) {
        self.table.set_layout(settings.table(key));
        if let Some(sorting) = settings.sorting(key) {
            self.table.set_sorting(sorting);
        }
        let Some(stored) = settings.filters(key) else {
            return;
        };
        for (axis, value) in stored {
            let change = match value {
                state::FilterValue::Flag(on) => match axis.as_str() {
                    "filter-hide-explicit" => FilterChange::Flag("filter-hide-explicit", on),
                    "filter-playable" => FilterChange::Flag("filter-playable", on),
                    "filter-favorites" => FilterChange::Flag("filter-favorites", on),
                    "filter-downloaded" => FilterChange::Flag("filter-downloaded", on),
                    _ => continue,
                },
                state::FilterValue::Range(low, high) => match axis.as_str() {
                    "filter-duration" => FilterChange::Range("filter-duration", (low, high)),
                    _ => continue,
                },
            };
            self.filter(change);
        }
    }

    /// The rows on screen, worked out again only when what they come from changed.
    fn order(
        &self,
        ui: &Ui,
        tracks: &[Track],
        columns: &'static [ColumnSpec<TrackField>],
        downloads_only: bool,
    ) -> Rc<Order> {
        let sorting = self.table.sorting().and_then(|sorting| {
            columns
                .iter()
                .find(|spec| spec.key == sorting.column)
                .map(|spec| (spec.field, sorting.order))
        });
        let reads = self.sieve.favorites || self.sieve.downloaded || downloads_only;
        let stamp = Stamp {
            at: tracks.as_ptr() as usize,
            len: tracks.len(),
            sorting,
            sieve: self.sieve,
            query: self.query.clone(),
            facts: match reads {
                true => facts(ui, tracks),
                false => 0,
            },
        };
        if let Some(order) = self.order.borrow().as_ref()
            && order.stamp == stamp
        {
            return order.clone();
        }

        let sieve = self.sieve;
        let keeps = |track: &Track| {
            if !sieve.keeps(track) {
                return false;
            }
            if sieve.favorites && !track.id.as_deref().is_some_and(|id| saved(ui, id)) {
                return false;
            }
            if (sieve.downloaded || downloads_only)
                && !track.id.as_deref().is_some_and(|id| downloaded(ui, id))
            {
                return false;
            }
            sort::hits(track, &self.query)
        };
        let mut rows: Vec<u32> = (0..tracks.len())
            .filter(|&at| keeps(&tracks[at]))
            .map(|at| at as u32)
            .collect();
        if let Some((field, order)) = stamp.sorting {
            rows.sort_by(|&a, &b| {
                let ordering = sort::compare(tracks, field, a as usize, b as usize);
                match order {
                    ui::Sort::Ascending => ordering,
                    ui::Sort::Descending => ordering.reverse(),
                }
            });
        }
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let keys: Vec<String> = rows
            .iter()
            .map(|&at| match tracks[at as usize].id.as_deref() {
                Some(id) => {
                    let times = seen.entry(id).or_default();
                    *times += 1;
                    match *times {
                        1 => id.to_owned(),
                        n => format!("{id}~{n}"),
                    }
                }
                None => format!("row-{at}"),
            })
            .collect();
        let at = keys
            .iter()
            .enumerate()
            .map(|(display, key)| (key.clone(), display as u32))
            .collect();
        let version = self.versions.get() + 1;
        self.versions.set(version);
        let order = Rc::new(Order {
            stamp,
            rows,
            keys,
            at,
            version,
        });
        *self.order.borrow_mut() = Some(order.clone());
        order
    }

    /// The shortest and longest track in the whole list, `None` only when it is empty. The span
    /// ignores the filters and the search, so narrowing the list never shrinks the slider that
    /// did the narrowing.
    fn extent(&self, tracks: &[Track]) -> Option<(f32, f32)> {
        let stamp = (tracks.as_ptr() as usize, tracks.len());
        if let Some((at, len, extent)) = self.spread.get()
            && (at, len) == stamp
        {
            return extent;
        }
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        for track in tracks {
            let seconds = track.duration.as_secs_f32();
            low = low.min(seconds);
            high = high.max(seconds);
        }
        let extent = (low <= high).then_some((low, high));
        self.spread.set(Some((stamp.0, stamp.1, extent)));
        extent
    }
}

/// Where a list's tracks live and how they play: the source in the store, the [`TrackList`] in
/// the screen's state, the columns, and the collection playback names as their origin. Cheap
/// to clone; a Play button and the table share one.
#[derive(Clone)]
pub(crate) struct TrackRows {
    source: Source,
    list: ListFn,
    list_mut: ListMutFn,
    columns: &'static [ColumnSpec<TrackField>],
    origin: Option<Origin>,
    /// The library shelf the rows are, which offline mode keeps to its downloads.
    shelved: Option<Shelf>,
    /// Offers a favorites filter and hearts only when this shelf lists more than favorites.
    starrable: Option<Shelf>,
    /// Hearts on the rows.
    liked: bool,
    /// The album the rows are, for its own track numbers.
    album: Option<String>,
}

impl TrackRows {
    /// `source` finds the tracks in the store, `list` and `list_mut` the [`TrackList`] in the
    /// screen's state.
    pub(crate) fn new(
        source: impl for<'a> Fn(&'a Ui) -> &'a [Track] + 'static,
        list: impl for<'a> Fn(&'a Ui) -> &'a TrackList + 'static,
        list_mut: impl for<'a> Fn(&'a mut Ui) -> &'a mut TrackList + 'static,
        columns: &'static [ColumnSpec<TrackField>],
    ) -> Self {
        Self {
            source: Rc::new(source),
            list: Rc::new(list),
            list_mut: Rc::new(list_mut),
            columns,
            origin: None,
            shelved: None,
            starrable: None,
            liked: false,
            album: None,
        }
    }

    /// The collection the rows are, which playback names as where its queue came from.
    pub(crate) fn origin(mut self, origin: Option<Origin>) -> Self {
        self.origin = origin;
        self
    }

    /// Hearts on the rows, which save or unsave each track.
    pub(crate) fn liked(mut self) -> Self {
        self.liked = true;
        self
    }

    /// Offers a favorites filter, and hearts, only when `shelf` lists more than the favorites.
    pub(crate) fn starrable(mut self, shelf: Shelf) -> Self {
        self.starrable = Some(shelf);
        self.liked = true;
        self
    }

    /// Marks the rows as a library shelf, which offline mode keeps to its downloads.
    pub(crate) fn shelved(mut self, shelf: Shelf) -> Self {
        self.shelved = Some(shelf);
        self
    }

    /// The rows are album `id`'s, numbered by its own track numbers where the shelf is a
    /// catalog of files.
    pub(crate) fn album(mut self, id: impl Into<String>) -> Self {
        self.album = Some(id.into());
        self
    }

    /// The columns the rows show.
    pub(crate) fn columns(&self) -> &'static [ColumnSpec<TrackField>] {
        self.columns
    }

    /// The tracks as the store keeps them, before any sort or search.
    pub(crate) fn tracks<'a>(&self, ui: &'a Ui) -> &'a [Track] {
        (self.source)(ui)
    }

    pub(crate) fn list<'a>(&self, ui: &'a Ui) -> &'a TrackList {
        (self.list)(ui)
    }

    pub(crate) fn list_mut<'a>(&self, ui: &'a mut Ui) -> &'a mut TrackList {
        (self.list_mut)(ui)
    }

    /// The collection playback names as the rows' origin.
    pub(crate) fn whence(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }

    /// The rows on screen.
    pub(crate) fn order(&self, ui: &Ui) -> Rc<Order> {
        let tracks = (self.source)(ui);
        (self.list)(ui).order(ui, tracks, self.columns, self.downloads_only(ui))
    }

    /// The tracks in the order they show, for a queue.
    pub(crate) fn ordered(&self, ui: &Ui) -> Vec<Track> {
        let tracks = (self.source)(ui);
        let order = self.order(ui);
        order
            .rows
            .iter()
            .filter_map(|&at| tracks.get(at as usize).cloned())
            .collect()
    }

    /// The first row that can play, if any.
    pub(crate) fn first_playable(&self, ui: &Ui) -> Option<usize> {
        let tracks = (self.source)(ui);
        let order = self.order(ui);
        order
            .rows
            .iter()
            .position(|&at| tracks.get(at as usize).is_some_and(|track| track.playable))
    }

    /// Whether a row shows the track `id`.
    pub(crate) fn holds(&self, ui: &Ui, id: &str) -> bool {
        self.order(ui).display(id).is_some()
    }

    /// The tracks of the rows keyed `keys`, in the order given.
    pub(crate) fn picked(&self, ui: &Ui, keys: &[String]) -> Vec<Track> {
        let tracks = (self.source)(ui);
        let order = self.order(ui);
        keys.iter()
            .filter_map(|key| order.display(key))
            .filter_map(|display| order.source(display))
            .filter_map(|at| tracks.get(at).cloned())
            .collect()
    }

    /// The tracks the selection holds, in the order they stand, or the cursor's alone.
    pub(crate) fn selected(&self, ui: &Ui) -> Vec<Track> {
        let order = self.order(ui);
        let keys = (self.list)(ui)
            .table
            .picked(order.len(), |at| Cow::Borrowed(order.key(at).unwrap_or("")));
        self.picked(ui, &keys)
    }

    /// Plays the rows from row `display`, unless that row cannot play while the network is gone.
    pub(crate) fn play(&self, ui: &mut Ui, display: usize) {
        let queued = self.ordered(ui);
        let Some(track) = queued.get(display) else {
            return;
        };
        if ui.store.unreachable_reason(track).is_some() {
            return;
        }
        ui.store.play_tracks(queued, display, self.origin.clone());
        ui.settle();
    }

    /// Pauses or resumes row `display` when it holds the current track, else plays from it.
    pub(crate) fn play_or_toggle(&self, ui: &mut Ui, display: usize) {
        let tracks = (self.source)(ui);
        let order = self.order(ui);
        let Some(track) = order.source(display).and_then(|at| tracks.get(at)) else {
            return;
        };
        let current = ui
            .store
            .playback
            .track()
            .and_then(|track| track.id.as_deref());
        match current.is_some() && current == track.id.as_deref() {
            true => {
                ui.store.toggle_play();
                ui.settle();
            }
            false => self.play(ui, display),
        }
    }

    /// Starts the rows from the first playable one, or a random one with `shuffle`.
    pub(crate) fn start(&self, ui: &mut Ui, shuffle: bool) {
        let tracks = self.ordered(ui);
        match shuffle {
            true => ui.store.shuffle_tracks(tracks, self.origin.clone()),
            false => ui.store.play_any(tracks, self.origin.clone()),
        }
        ui.settle();
    }

    /// The axes the filter popover offers for these rows.
    pub(crate) fn filters(&self, ui: &Ui) -> Vec<Filter> {
        let list = (self.list)(ui);
        let sieve = list.sieve;
        let duration = list.extent((self.source)(ui)).map(|bounds| {
            Filter::Range(
                RangeAxis {
                    key: "filter-duration",
                    label: t!("filter-duration"),
                    bounds,
                    value: sieve.duration.unwrap_or(bounds),
                    unit: Unit::Clock,
                    values: None,
                }
                .clamped(),
            )
        });
        let flag = |key: &'static str, on: bool| {
            Filter::Flag(FlagAxis {
                key,
                label: i18n::lookup(key, None),
                on,
            })
        };
        let mut axes: Vec<Filter> = duration.into_iter().collect();
        axes.push(flag("filter-hide-explicit", sieve.hide_explicit));
        axes.push(flag("filter-playable", sieve.playable));
        if self.catalog(ui) {
            axes.push(flag("filter-favorites", sieve.favorites));
        }
        // Offline mode already keeps a shelf to its downloads, so the filter would do nothing.
        let offered = sieve.downloaded || !ui.store.downloads.is_empty();
        if offered && !self.downloads_only(ui) {
            axes.push(flag("filter-downloaded", sieve.downloaded));
        }
        axes
    }

    /// Applies a change from the filter popover and saves the filters under `key`.
    pub(crate) fn filter(&self, ui: &mut Ui, change: FilterChange, key: Option<&str>) {
        (self.list_mut)(ui).filter(change);
        if let Some(key) = key {
            let axes = self.filters(ui);
            let stored = ui.store.settings.filters(key).unwrap_or_default();
            let filters = page::snapshot(axes, stored);
            let store = &mut ui.store;
            store.settings.set_filters(key, filters, &mut store.cx);
        }
    }

    /// Whether offline mode keeps these rows to their downloads.
    fn downloads_only(&self, ui: &Ui) -> bool {
        self.shelved
            .is_some_and(|shelf| ui.store.network.downloads_only(shelf))
    }

    /// Whether the starrable shelf lists more than the favorites.
    fn catalog(&self, ui: &Ui) -> bool {
        self.starrable
            .is_some_and(|shelf| ui.store.library.shape(shelf).is_catalog())
    }

    /// Whether the rows carry hearts: on a saved shelf every listed track is a favorite, so the
    /// heart would say nothing.
    fn hearts(&self, ui: &Ui) -> bool {
        self.liked && (self.starrable.is_none() || self.catalog(ui))
    }

    /// How the rows are numbered. A catalog shelf holds only the files it was given, so an album
    /// there is numbered by each track's own place in it and a missing file leaves a gap.
    fn numbering(&self, ui: &Ui, tracks: &[Track]) -> Numbering {
        let Some(id) = self.album.as_deref() else {
            return Numbering::Listing;
        };
        if !ui.store.library.shape(Shelf::of(id)).is_catalog() {
            return Numbering::Listing;
        }
        let first = tracks.first().map(disc);
        match tracks.iter().any(|track| Some(disc(track)) != first) {
            true => Numbering::Disc,
            false => Numbering::Track,
        }
    }
}

/// The track table: a builder over [`TrackRows`], drawn with [`ui::Table`]. Give it a bounded
/// height; it is its own scroll viewport and grows to fill its parent, with an optional lead
/// (the page's hero) scrolling above its sticky header.
pub(crate) struct TrackTable {
    key: String,
    rows: TrackRows,
    width: f32,
    lead: Option<El>,
    foot: Option<El>,
    pending: Option<Pending>,
    label: Option<String>,
    bottom: f32,
    remember: Option<String>,
    on_context: Option<OnContext>,
    on_remove: Option<OnRemove>,
    on_drag: Option<OnDrag>,
    on_head: Option<OnHead>,
}

impl TrackTable {
    /// `key` names the table's scroll viewport, unique on the page.
    pub(crate) fn new(key: impl Into<String>, rows: &TrackRows) -> Self {
        Self {
            key: key.into(),
            rows: rows.clone(),
            width: 0.,
            lead: None,
            foot: None,
            pending: None,
            label: None,
            bottom: 0.,
            remember: None,
            on_context: None,
            on_remove: None,
            on_drag: None,
            on_head: None,
        }
    }

    /// The width the columns share: the content area's, less nothing; the table spans it.
    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// An element that scrolls above the header, such as the page's hero.
    pub(crate) fn lead(mut self, lead: El) -> Self {
        self.lead = Some(lead);
        self
    }

    /// An element that scrolls under the last row, such as an album's notices. The room set
    /// with [`TrackTable::bottom`] goes under it.
    pub(crate) fn foot(mut self, foot: El) -> Self {
        self.foot = Some(foot);
        self
    }

    /// Skeleton rows while the source has none yet.
    pub(crate) fn pending(mut self, pending: Option<Pending>) -> Self {
        self.pending = pending;
        self
    }

    /// What a screen reader calls the table.
    pub(crate) fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Room under the last row, for the player bar or a perched control.
    pub(crate) fn bottom(mut self, room: f32) -> Self {
        self.bottom = room;
        self
    }

    /// Saves the column layout and the sort under `key` in settings as they change, to put back
    /// with [`TrackList::restore`].
    pub(crate) fn remember(mut self, key: impl Into<String>) -> Self {
        self.remember = Some(key.into());
        self
    }

    /// A right-click on rows. The screen opens the track menu there.
    pub(crate) fn on_context(
        mut self,
        f: impl Fn(&mut Ui, Picked, PointerPress, &mut Tx) + 'static,
    ) -> Self {
        self.on_context = Some(Rc::new(f));
        self
    }

    /// Delete or Backspace on picked rows. The screen asks before it removes them.
    pub(crate) fn on_remove(mut self, f: impl Fn(&mut Ui, Picked, &mut Tx) + 'static) -> Self {
        self.on_remove = Some(Rc::new(f));
        self
    }

    /// Every phase of a drag that starts on a row, with the tracks it carries. The screen draws
    /// the ghost and handles the drop.
    pub(crate) fn on_row_drag(
        mut self,
        f: impl Fn(&mut Ui, &[Track], &mut Drag, &mut Tx) + 'static,
    ) -> Self {
        self.on_drag = Some(Rc::new(f));
        self
    }

    /// A right-click on the header.
    pub(crate) fn on_head_context(
        mut self,
        f: impl Fn(&mut Ui, PointerPress, &mut Tx) + 'static,
    ) -> Self {
        self.on_head = Some(Rc::new(f));
        self
    }

    pub(crate) fn build(self, ui: &Ui) -> El {
        let Self {
            key,
            rows,
            width,
            lead,
            foot,
            pending,
            label,
            bottom,
            remember,
            on_context,
            on_remove,
            on_drag,
            on_head,
        } = self;
        let theme = &ui.theme;
        let tracks = rows.tracks(ui);
        let order = rows.order(ui);
        let current = ui
            .store
            .playback
            .track()
            .and_then(|track| track.id.as_deref());
        // A track listed twice counts as playing only at its first row.
        let playing_at = current.and_then(|id| {
            tracks
                .iter()
                .position(|track| track.id.as_deref() == Some(id))
        });
        let shown = Shown {
            playing_at,
            control: ui.store.playback.control(),
            numbering: rows.numbering(ui, tracks),
            hearts: rows.hearts(ui),
        };

        let state_rows = rows.clone();
        let state_mut_rows = rows.clone();
        let count = order.len();
        let ids = order.clone();
        let mut table = Table::<Ui, TrackField>::new(
            key.clone(),
            rows.columns,
            move |ui: &Ui| &state_rows.list(ui).table,
            move |ui: &mut Ui| &mut state_mut_rows.list_mut(ui).table,
            move |_: &Ui| count,
            move |_: &Ui, at: usize| Cow::Owned(ids.key(at).unwrap_or_default().to_owned()),
        )
        .width(width)
        .version(order.version)
        .pending(pending)
        .bottom(bottom);

        let drawn = (rows.clone(), order.clone(), key.clone(), shown);
        table = table.cell(move |ui: &Ui, cell: &TableCell<TrackField>| {
            let (rows, order, table, shown) = &drawn;
            cell_of(ui, rows, order, table, *shown, cell)
        });
        let named = (rows.clone(), order.clone());
        table = table.name(move |ui: &Ui, at: usize| {
            let (rows, order) = &named;
            let Some(track) = order.source(at).and_then(|at| rows.tracks(ui).get(at)) else {
                return String::new();
            };
            match track.artists.is_empty() {
                true => track.name.clone(),
                false => t!(
                    "track-row-name",
                    title = track.name.as_str(),
                    artists = track.artists.as_str()
                )
                .into_owned(),
            }
        });
        let lit = order.clone();
        table = table
            .playing(move |_: &Ui, at: usize| playing_at.is_some() && lit.source(at) == playing_at);
        let tipped = (rows.clone(), order.clone());
        table = table.tip(move |ui: &Ui, at: usize| {
            let (rows, order) = &tipped;
            let track = order.source(at).and_then(|at| rows.tracks(ui).get(at))?;
            let reason = ui.store.unreachable_reason(track)?;
            let mut args = i18n::FluentArgs::new();
            if let Some(provider) = reason.provider() {
                args.set("name", i18n::Value::value(provider));
            }
            Some(i18n::lookup(reason.key(), Some(&args)).into_owned())
        });
        let filled = rows.clone();
        table =
            table.populated(move |ui: &Ui, field: TrackField| populated(filled.tracks(ui), field));
        if let Some(lead) = lead {
            table = table.lead(lead);
        }
        if let Some(foot) = foot {
            table = table.foot(foot);
        }
        if let Some(label) = label {
            table = table.label(label);
        }

        let opened = (rows.clone(), order.clone());
        table = table.on_open(move |ui: &mut Ui, id: &str, _: &mut Tx| {
            let (rows, order) = &opened;
            if let Some(display) = rows.order(ui).display(id).or_else(|| order.display(id)) {
                rows.play(ui, display);
            }
        });
        if let Some(remember) = remember {
            let sorted = (rows.clone(), remember.clone());
            table = table.on_sort(move |ui: &mut Ui, sorting: Option<Sorting>, _: &mut Tx| {
                let (_, key) = &sorted;
                let store = &mut ui.store;
                store.settings.set_sorting(key, sorting, &mut store.cx);
            });
            let laid = (rows.clone(), remember);
            table = table.on_layout(move |ui: &mut Ui, _: &mut Tx| {
                let (rows, key) = &laid;
                let layout = rows.list(ui).table.layout().clone();
                let store = &mut ui.store;
                store.settings.set_table(key, layout, &mut store.cx);
            });
        }
        if let Some(on_context) = on_context {
            let picked = rows.clone();
            table = table.on_context(
                move |ui: &mut Ui, keys: Vec<String>, at: PointerPress, tx: &mut Tx| {
                    let picked = pick(ui, &picked, keys);
                    on_context(ui, picked, at, tx);
                },
            );
        }
        if let Some(on_remove) = on_remove {
            let picked = rows.clone();
            table = table.on_remove(move |ui: &mut Ui, keys: Vec<String>, tx: &mut Tx| {
                let picked = pick(ui, &picked, keys);
                on_remove(ui, picked, tx);
            });
        }
        if let Some(on_drag) = on_drag {
            let carried = rows.clone();
            table = table.on_row_drag(
                move |ui: &mut Ui, keys: &[String], drag: &mut Drag, tx: &mut Tx| {
                    let tracks = carried.picked(ui, keys);
                    on_drag(ui, &tracks, drag, tx);
                },
            );
        }
        if let Some(on_head) = on_head {
            table = table.on_head_context(move |ui: &mut Ui, at: PointerPress, tx: &mut Tx| {
                on_head(ui, at, tx)
            });
        }
        table.build(ui, theme)
    }
}

/// What every row of one build shares: where playback is and how rows are numbered and marked.
#[derive(Clone, Copy)]
struct Shown {
    /// The source index of the playing track's first row.
    playing_at: Option<usize>,
    control: Option<bool>,
    numbering: Numbering,
    hearts: bool,
}

/// What one cell of the track table draws.
fn cell_of(
    ui: &Ui,
    rows: &TrackRows,
    order: &Rc<Order>,
    table: &str,
    shown: Shown,
    cell: &TableCell<TrackField>,
) -> El {
    let theme = &ui.theme;
    let key = order.key(cell.row).unwrap_or_default();
    let frame = cells::frame(cell, &format!("{table}/{key}"));
    let Some(at) = order.source(cell.row) else {
        return frame;
    };
    let Some(track) = rows.tracks(ui).get(at) else {
        return frame;
    };
    let muted = theme.muted_foreground;
    let faded = muted.opacity(FADED);
    let plays_now = track.playable && ui.store.unreachable_reason(track).is_none();
    let (title, detail) = match plays_now {
        true => (None, muted),
        false => (Some(faded), faded),
    };
    match cell.field {
        TrackField::Index => {
            let playing = (shown.playing_at == Some(at))
                .then_some(shown.control)
                .flatten();
            let number =
                number(track, shown.numbering).unwrap_or_else(|| (cell.row + 1).to_string());
            let press = plays_now.then(|| -> Act {
                let rows = rows.clone();
                let key = key.to_owned();
                Rc::new(move |ui: &mut Ui, _: &mut Tx| match playing {
                    Some(_) => {
                        ui.store.toggle_play();
                        ui.settle();
                    }
                    None => {
                        if let Some(display) = rows.order(ui).display(&key) {
                            rows.play(ui, display);
                        }
                    }
                })
            });
            cells::index(frame, playing, plays_now, number, press, theme)
        }
        TrackField::Cover => cells::artwork(frame, ui.artwork(), track.cover.as_deref(), theme),
        TrackField::Title => cells::title(
            frame,
            Title {
                name: track.name.clone(),
                color: title,
                explicit: track.explicit,
                status: download_mark(ui, track),
                liked: shown.hearts.then(|| heart(ui, rows, track)).flatten(),
                press: None,
            },
            theme,
        ),
        TrackField::Artists => cells::artists(
            frame,
            &track.artist_refs,
            track.artists.clone(),
            detail,
            theme,
        ),
        TrackField::Album => match track.album_id.as_deref() {
            Some(album) => cells::link(
                frame,
                "album",
                track.album.clone(),
                detail,
                Destination::Album(album.into()),
                theme,
            ),
            None => cells::dim(frame, "album", track.album.clone(), detail, theme),
        },
        TrackField::AddedAt => cells::credited(
            frame,
            track.added_by.as_deref(),
            cells::stamp(track.added_at),
            detail,
            ui.artwork(),
            theme,
        ),
        TrackField::AddedBy => match track.added_by.as_deref() {
            Some(added) => cells::added_by(frame, added, detail, ui.artwork(), theme),
            None => frame,
        },
        TrackField::PlayedAt => cells::dim(
            frame,
            "played-at",
            cells::relative_stamp(track.added_at),
            detail,
            theme,
        ),
        TrackField::Plays => cells::dim(
            frame,
            "plays",
            track.playcount.map(cells::count).unwrap_or_default(),
            detail,
            theme,
        ),
        TrackField::Duration => cells::length(frame, track.duration, detail, theme),
    }
}

/// The label a row rests at, or `None` to count the rows.
fn number(track: &Track, numbering: Numbering) -> Option<String> {
    if track.track_number == 0 {
        return None;
    }
    match numbering {
        Numbering::Listing => None,
        Numbering::Track => Some(track.track_number.to_string()),
        Numbering::Disc => Some(format!("{}.{}", disc(track), track.track_number)),
    }
}

/// The disc a track sits on. Providers disagree on what an absent disc tag means, so a
/// single-disc album reads as disc one either way.
fn disc(track: &Track) -> u32 {
    track.disc_number.max(1)
}

/// The heart after a row's title: filled in the accent while the track is saved, else a quiet
/// outline the pointer brings out with the row.
fn heart(ui: &Ui, rows: &TrackRows, track: &Track) -> Option<El> {
    let theme = &ui.theme;
    let id = track.id.clone()?;
    let library = &ui.store.library;
    let saved = library.saved(&id);
    let pending = library.pending(&id);
    let source = rows.source.clone();
    let button = Button::new("toggle-liked-track")
        .ghost()
        .backgroundless()
        .small()
        .icon(match saved {
            true => "icons/heart-filled.svg",
            false => "icons/heart.svg",
        })
        .tooltip(match saved {
            true => t!("menu-remove-from-library"),
            false => t!("menu-add-to-library"),
        })
        .tint(match saved {
            true => theme.primary,
            false => theme.muted_foreground.opacity(SETTLED),
        })
        .disabled(pending)
        .on_press(move |ui: &mut Ui| {
            let Some(track) = source(ui)
                .iter()
                .find(|track| track.id.as_deref() == Some(id.as_str()))
                .cloned()
            else {
                return;
            };
            let store = &mut ui.store;
            store.library.toggle(track, &store.session, &mut store.cx);
            ui.settle();
        })
        .build(theme);
    Some(match saved {
        true => button,
        false => button.group_opacity(0., 1.),
    })
}

/// How far a download the listener asked for has got, shown before the heart. The arrow sits
/// on a filled disc like the explicit badge, the accent ring fills as it arrives, and a track
/// still waiting its turn or only cached because it played shows nothing.
fn download_mark(ui: &Ui, track: &Track) -> Option<El> {
    let theme = &ui.theme;
    let id = track.id.as_deref()?;
    let downloads = &ui.store.downloads;
    let ring = |done| {
        ArrowRing::new(done)
            .color(theme.primary)
            .arrow(theme.muted_foreground)
            .backdrop(theme.muted)
    };
    let (ring, landed) = match downloads.track(id)? {
        Progress::Done => (ring(Some(1.)), true),
        Progress::Running => (ring(downloads.fraction(id)), false),
        Progress::Waiting => return None,
        Progress::Failed => (ring(Some(0.)).arrow(theme.danger), false),
    };
    // The key stays the same in every state, so the ring keeps its animation as it lands. Only
    // a landed download keeps a name, since no catalogued string says what a running or failed
    // one is doing here.
    let mark = row().key("downloaded-mark").child(ring.build(theme));
    Some(match landed {
        true => mark
            .role("image")
            .label(t!("filter-downloaded").into_owned()),
        false => mark.a11y_hidden(),
    })
}

/// Whether the source has anything for `field`'s column. An empty list shows every column, so
/// its skeletons keep the shape the rows will take.
fn populated(tracks: &[Track], field: TrackField) -> bool {
    if tracks.is_empty() {
        return true;
    }
    match field {
        TrackField::Artists => tracks.iter().any(|track| !track.artists.is_empty()),
        TrackField::Album => tracks.iter().any(|track| !track.album.is_empty()),
        TrackField::AddedAt | TrackField::PlayedAt => {
            tracks.iter().any(|track| track.added_at.is_some())
        }
        TrackField::AddedBy => tracks.iter().any(|track| track.added_by.is_some()),
        TrackField::Plays => tracks.iter().any(|track| track.playcount.is_some()),
        _ => true,
    }
}

/// The rows a handler named, with the columns an adaptive menu reads.
fn pick(ui: &Ui, rows: &TrackRows, keys: Vec<String>) -> Picked {
    let layout = rows.list(ui).table.layout();
    let shows = |field: TrackField| {
        rows.columns
            .iter()
            .any(|spec| spec.field == field && !layout.hides(spec.key))
    };
    let adaptive = ui.store.settings.adaptive_menu();
    Picked {
        tracks: rows.picked(ui, &keys),
        keys,
        album: adaptive && shows(TrackField::Album),
        artists: adaptive && shows(TrackField::Artists),
    }
}

/// Whether the library holds track `id`.
fn saved(ui: &Ui, id: &str) -> bool {
    ui.store.library.saved(id)
}

/// Whether track `id` was downloaded.
fn downloaded(ui: &Ui, id: &str) -> bool {
    ui.store.downloads.track(id) == Some(Progress::Done)
}

/// A hash of the hearts and downloads of `tracks`, so a filter that reads them follows them.
fn facts(ui: &Ui, tracks: &[Track]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for track in tracks {
        let Some(id) = track.id.as_deref() else {
            continue;
        };
        (saved(ui, id), downloaded(ui, id)).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod fixture {
    use std::time::Duration;

    use music::Track;

    pub(super) fn track(seconds: u64, explicit: bool, playable: bool) -> Track {
        Track {
            id: Some("id".to_owned()),
            playable,
            duration: Duration::from_secs(seconds),
            explicit,
            ..Default::default()
        }
    }
}
