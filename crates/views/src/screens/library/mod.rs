//! A library shelf: the streaming provider's (`Destination::Library`) or the local files'
//! (`Destination::Local`), one page per tab. Songs lists the shelf's tracks under a hero in the
//! virtualised track table; Albums, Playlists and Artists show as a virtualised grid of cards or
//! as a table, as the title bar's view switch picks. Each page says why it is empty, loading,
//! offline, or (local files) waiting for a folder.
//!
//! Each shelf keeps its own lists in [`State`], so a sort, a filter or a selection made on one
//! shelf stays there. Their layouts, sorts, filters and view modes are saved in settings under
//! the keys the GPUI version used, and put back the first time a shelf is shown.

mod albums;
mod artists;
mod items;
mod playlists;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i18n::t;
use music::{GenreItem, Shape};
use router::{Destination, LibraryTab};
use rune::{Cx, El, PointerPress, ScrollSpec, Transition, Tx, col, el, row};
use state::{Addition, AppSettings, LibraryPart, LibraryState, Origin, Shelf};
use ui::{Button, FilterChange, Mode, Pending, ScrollWatch, Sort, Sorting, Vacancy};

use crate::Ui;
use crate::chrome::toolbar::Tools;
use crate::chrome::{player_bar, sidebar_left, tools as bar};
use crate::shared::cards::OnItem;
use crate::shared::hero::{Hero, Listing, MetaStrip, PlayButton, action_gap};
use crate::shared::menus::{self, Target, TrackColumns};
use crate::shared::pins::Pinned as _;
use crate::shared::playlist_editor::{self, Edit};
use crate::shared::tracks::{self, LIBRARY_COLUMNS, Order, TrackList, TrackRows, TrackTable};
use crate::shared::{confirm, local, page, trouble};
use albums::Albums;
use artists::Artists;
use items::{Items, Kind};
use playlists::Playlists;

/// How many skeleton rows the songs table holds while a known number of songs is on its way:
/// enough to fill any window, few enough to build at once.
const SKELETONS: usize = 40;
/// The column the songs table sorts by until the listener picks another.
const SONGS_FIRST: &str = "added-at";

/// One page of a shelf. Songs lists the favorites on a `Shape::Saved` shelf and every song on a
/// `Shape::Catalog` one; the other three follow the same rule for their kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Songs,
    Albums,
    Playlists,
    Artists,
}

impl From<LibraryTab> for Section {
    fn from(tab: LibraryTab) -> Self {
        match tab {
            LibraryTab::Songs => Section::Songs,
            LibraryTab::Albums => Section::Albums,
            LibraryTab::Playlists => Section::Playlists,
            LibraryTab::Artists => Section::Artists,
        }
    }
}

impl Section {
    /// The settings key a section's layout is stored under. The names predate the shelves and
    /// stay so stored layouts survive.
    fn key(self, shelf: Shelf) -> &'static str {
        match (shelf, self) {
            (Shelf::Streaming, Section::Songs) => "songs",
            (Shelf::Streaming, Section::Albums) => "albums",
            (Shelf::Streaming, Section::Playlists) => "playlists",
            (Shelf::Streaming, Section::Artists) => "artists",
            (Shelf::Local, Section::Songs) => "local-songs",
            (Shelf::Local, Section::Albums) => "local-albums",
            (Shelf::Local, Section::Playlists) => "local-playlists",
            (Shelf::Local, Section::Artists) => "local-artists",
        }
    }

    /// The Fluent key of the page's name, which its table is announced by.
    fn title(self) -> &'static str {
        match self {
            Section::Songs => "nav-songs",
            Section::Albums => "nav-albums",
            Section::Playlists => "nav-playlists",
            Section::Artists => "nav-artists",
        }
    }

    fn vacancy(self, shelf: Shelf, shape: Shape) -> &'static str {
        match (shelf, shape, self) {
            (Shelf::Local, _, Section::Songs) => "library-no-local-songs",
            (Shelf::Local, _, Section::Albums) => "library-no-local-albums",
            (Shelf::Local, _, Section::Playlists) => "library-no-local-playlists",
            (Shelf::Local, _, Section::Artists) => "library-no-local-artists",
            (Shelf::Streaming, Shape::Saved, Section::Songs) => "library-no-songs",
            (Shelf::Streaming, Shape::Saved, Section::Albums) => "library-no-albums",
            (Shelf::Streaming, Shape::Saved, Section::Artists) => "library-no-artists",
            (Shelf::Streaming, Shape::Catalog(_), Section::Songs) => "library-no-catalog-songs",
            (Shelf::Streaming, Shape::Catalog(_), Section::Albums) => "library-no-catalog-albums",
            (Shelf::Streaming, Shape::Catalog(_), Section::Artists) => "library-no-catalog-artists",
            (Shelf::Streaming, _, Section::Playlists) => "library-no-playlists",
        }
    }

    fn glyph(self, shape: Shape) -> &'static str {
        match (self, shape) {
            (Section::Songs, Shape::Saved) => "icons/heart.svg",
            (Section::Songs, Shape::Catalog(_)) => "icons/music.svg",
            (Section::Albums, _) => "icons/disc-3.svg",
            (Section::Playlists, _) => "icons/list-music.svg",
            (Section::Artists, _) => "icons/user-round.svg",
        }
    }

    fn part(self) -> LibraryPart {
        match self {
            Section::Songs => LibraryPart::Tracks,
            Section::Albums => LibraryPart::Albums,
            Section::Playlists => LibraryPart::Playlists,
            Section::Artists => LibraryPart::Artists,
        }
    }
}

/// The library screen's state: one set of lists per shelf.
#[derive(Default)]
pub struct State {
    streaming: Shelved,
    local: Shelved,
}

/// One shelf's lists, and what the page keeps between visits.
#[derive(Default)]
pub(crate) struct Shelved {
    songs: TrackList,
    albums: Items<Albums>,
    playlists: Items<Playlists>,
    artists: Items<Artists>,
    /// Whether the saved layouts, sorts and filters were put back.
    restored: bool,
    /// Where the shown page's list is scrolled to.
    watch: ScrollWatch,
    /// The page shown last, so coming back to it lands where it was left.
    shown: Option<Section>,
    /// Where to put the list back to once it is laid out.
    rewind: Option<f32>,
    /// The listed songs' total length, by the order it was summed for.
    runtime: RefCell<Option<(Rc<Order>, Duration)>>,
}

impl State {
    pub(crate) fn shelf(&self, shelf: Shelf) -> &Shelved {
        match shelf {
            Shelf::Streaming => &self.streaming,
            Shelf::Local => &self.local,
        }
    }

    pub(crate) fn shelf_mut(&mut self, shelf: Shelf) -> &mut Shelved {
        match shelf {
            Shelf::Streaming => &mut self.streaming,
            Shelf::Local => &mut self.local,
        }
    }

    /// Puts back a shelf's saved layouts, sorts and filters the first time it is shown, and
    /// lands a page where it was left when the window comes back to it. Moving to another tab
    /// starts that tab at its top.
    pub(crate) fn arrived(&mut self, destination: &Destination, settings: &AppSettings) {
        let Some((shelf, section)) = place(destination) else {
            return;
        };
        let shelved = self.shelf_mut(shelf);
        if !shelved.restored {
            shelved.songs.table.set_sorting(Some(Sorting {
                column: SONGS_FIRST.to_owned(),
                order: Sort::Descending,
            }));
            shelved.songs.restore(settings, Section::Songs.key(shelf));
            shelved.albums.restore(settings, Section::Albums.key(shelf));
            shelved
                .playlists
                .restore(settings, Section::Playlists.key(shelf));
            shelved
                .artists
                .restore(settings, Section::Artists.key(shelf));
            shelved.restored = true;
        }
        shelved.rewind = match shelved.shown == Some(section) {
            true => Some(shelved.watch.offset()),
            false => None,
        };
        shelved.shown = Some(section);
    }
}

/// The shelf and page a destination shows, for the library's destinations.
fn place(at: &Destination) -> Option<(Shelf, Section)> {
    match at {
        Destination::Library(tab) => Some((Shelf::Streaming, Section::from(*tab))),
        Destination::Local(tab) => Some((Shelf::Local, Section::from(*tab))),
        _ => None,
    }
}

/// A destination on `shelf`, which the title bar's search field reads the same for every tab.
pub(crate) fn destination(shelf: Shelf) -> Destination {
    match shelf {
        Shelf::Streaming => Destination::Library(LibraryTab::Songs),
        Shelf::Local => Destination::Local(LibraryTab::Songs),
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let Some((shelf, section)) = place(&ui.nav.current()) else {
        return col().key("screen-library");
    };
    let theme = &ui.theme;
    let mode = mode(ui, shelf, section);
    let content = match unconfigured(ui, shelf) {
        true => centered(local::unconfigured("configure-local-folder", theme)),
        false => match section {
            Section::Songs => songs(ui, cx, shelf),
            Section::Albums => listing::<Albums>(ui, cx, shelf, mode),
            Section::Playlists => listing::<Playlists>(ui, cx, shelf, mode),
            Section::Artists => listing::<Artists>(ui, cx, shelf, mode),
        },
    };
    // each tab and view mode is its own page, cross-fading into the next
    let content = content
        .key(format!("library-{}-{mode:?}", section.key(shelf)))
        .w_full()
        .h_full()
        .min_h(0.)
        .transition(Transition::opacity());
    let page = col()
        .key("screen-library")
        .w_full()
        .h_full()
        .min_h(0.)
        .child(content);
    match section {
        // a right-click on the playlists page offers a new one
        Section::Playlists => page.on_secondary_press(menus::at(Target::NewPlaylist(shelf))),
        _ => page,
    }
}

/// The page's tools in the title bar: a new playlist and (local files) an import on the
/// playlists page, the list or grid switch, the filters while there is anything to narrow by,
/// and the sort; and the search field.
pub(crate) fn tools(ui: &Ui, layer: &mut Vec<El>) -> Tools {
    let Some((shelf, section)) = place(&ui.nav.current()) else {
        return Tools::default();
    };
    let theme = &ui.theme;
    let mut items = Vec::new();
    if section == Section::Playlists {
        items.push(
            Button::new("new-playlist")
                .icon("icons/plus.svg")
                .tooltip(t!("menu-new-playlist"))
                .small()
                .ghost()
                .on_press(move |ui: &mut Ui| create_playlist(ui, shelf))
                .build(theme),
        );
        if shelf.local() {
            items.push(
                Button::new("import-playlist")
                    .icon("icons/file-music.svg")
                    .tooltip(t!("menu-import-playlist"))
                    .small()
                    .ghost()
                    .on_press_tx(|_: &mut Ui, tx: &mut Tx| local::import_playlist(tx))
                    .build(theme),
            );
        }
    }
    if section != Section::Songs {
        let key = section.key(shelf);
        items.push(bar::views(
            ui,
            mode(ui, shelf, section),
            move |ui: &mut Ui, mode: Mode| {
                let store = &mut ui.store;
                store.settings.set_view(key, mode, &mut store.cx);
            },
        ));
    }
    let (axes, narrowed) = match section {
        Section::Songs => {
            let rows = songs_rows(shelf);
            (rows.filters(ui), rows.list(ui).filtered())
        }
        Section::Albums => items::filters::<Albums>(ui, shelf),
        Section::Playlists => items::filters::<Playlists>(ui, shelf),
        Section::Artists => items::filters::<Artists>(ui, shelf),
    };
    if !axes.is_empty() || narrowed {
        items.push(bar::filters(
            ui,
            axes,
            move |ui: &mut Ui, change: FilterChange| filter(ui, shelf, section, change),
            layer,
        ));
    }
    let sorts = match section {
        Section::Songs => songs_rows(shelf).sorts(ui),
        Section::Albums => items::sort_axes::<Albums>(ui, shelf),
        Section::Playlists => items::sort_axes::<Playlists>(ui, shelf),
        Section::Artists => items::sort_axes::<Artists>(ui, shelf),
    };
    items.push(bar::sorts(
        ui,
        sorts,
        move |ui: &mut Ui, column: &'static str| sort(ui, shelf, section, column),
        layer,
    ));
    Tools {
        items,
        search: Some("filter-library"),
    }
}

/// The context menu handler a card takes: its item's menu where the pointer is.
pub(crate) fn opener() -> OnItem {
    Rc::new(
        |ui: &mut Ui, item: GenreItem, press: PointerPress, _: &mut Tx| {
            if let Some(target) = Target::of(&item) {
                menus::open(ui, target, press);
            }
        },
    )
}

/// Whether a page shows as a list or a grid. Songs is always a list.
fn mode(ui: &Ui, shelf: Shelf, section: Section) -> Mode {
    match section {
        Section::Songs => Mode::List,
        _ => ui.store.settings.view_or(section.key(shelf), Mode::Grid),
    }
}

/// Opens the dialog that names a new, empty playlist on `shelf`.
fn create_playlist(ui: &mut Ui, shelf: Shelf) {
    menus::close(ui);
    playlist_editor::open(
        ui,
        Edit::Create {
            addition: Addition::Tracks(Vec::new()),
            shelf,
        },
    );
}

/// Applies a change from the filter popover to the page's list.
fn filter(ui: &mut Ui, shelf: Shelf, section: Section, change: FilterChange) {
    match section {
        Section::Songs => songs_rows(shelf).filter(ui, change, Some(Section::Songs.key(shelf))),
        Section::Albums => items::filter::<Albums>(ui, shelf, change),
        Section::Playlists => items::filter::<Playlists>(ui, shelf, change),
        Section::Artists => items::filter::<Artists>(ui, shelf, change),
    }
}

/// Steps the page's sort along the column keyed `column`, as the sort picker does.
fn sort(ui: &mut Ui, shelf: Shelf, section: Section, column: &'static str) {
    match section {
        Section::Songs => songs_rows(shelf).cycle_sort(ui, column, Some(Section::Songs.key(shelf))),
        Section::Albums => items::sort::<Albums>(ui, shelf, column),
        Section::Playlists => items::sort::<Playlists>(ui, shelf, column),
        Section::Artists => items::sort::<Artists>(ui, shelf, column),
    }
}

/// Where a shelf's songs live and how they play.
fn songs_rows(shelf: Shelf) -> TrackRows {
    TrackRows::new(
        move |ui: &Ui| ui.store.library.state(shelf).tracks(),
        move |ui: &Ui| &ui.screens.library.shelf(shelf).songs,
        move |ui: &mut Ui| &mut ui.screens.library.shelf_mut(shelf).songs,
        LIBRARY_COLUMNS,
    )
    .origin(Some(origin(shelf)))
    .starrable(shelf)
    .shelved(shelf)
}

fn origin(shelf: Shelf) -> Origin {
    match shelf {
        Shelf::Streaming => Origin::saved(),
        Shelf::Local => Origin::local(),
    }
}

/// The songs page: the hero over the track table, which scrolls it away under its sticky
/// header.
fn songs(ui: &Ui, cx: &mut Cx<Ui>, shelf: Shelf) -> El {
    let theme = &ui.theme;
    let inset = theme.metrics.inset;
    let query = items::query(ui, shelf);
    let rows = songs_rows(shelf).query(&query);
    let order = rows.order(ui);
    let filtering = rows.list(ui).filtered() || !query.is_empty();
    let hero = hero(ui, shelf, &rows, &order, filtering);
    let bottom = player_bar::bottom(ui, cx) + inset;
    let key = Section::Songs.key(shelf);
    if let Some(note) = note(ui, shelf, Section::Songs, order.len(), filtering) {
        return col()
            .key(format!("library-songs-{}", items::shelf_name(shelf)))
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(hero)
            .child(col().w_full().pb(bottom).child(note));
    }
    let library = &ui.store.library;
    let waiting =
        order.is_empty() && library.waiting(shelf, LibraryPart::Tracks, &ui.store.network);
    let pending = waiting.then(|| {
        library
            .expected(shelf, LibraryPart::Tracks)
            .map_or(Pending::Screen, |count| Pending::Rows(count.min(SKELETONS)))
    });
    let viewport = format!("library-songs-{}", items::shelf_name(shelf));
    let table = TrackTable::new(viewport.clone(), &rows)
        .width(page::content_width(cx, 0.))
        .lead(hero)
        .pending(pending)
        .label(t!("nav-songs").into_owned())
        .bottom(bottom)
        .remember(key)
        .on_context(
            |ui: &mut Ui, picked: tracks::Picked, press: PointerPress, _: &mut Tx| {
                let columns = TrackColumns {
                    album: picked.album,
                    artists: picked.artists,
                };
                menus::open_tracks(ui, picked.tracks, menus::TrackList::Plain, columns, press);
            },
        )
        .on_remove(move |ui: &mut Ui, picked: tracks::Picked, _: &mut Tx| {
            // on a saved shelf the list is the favorites, so removing a row unstars it
            if ui.store.library.shape(shelf) == Shape::Saved {
                confirm::library_songs(ui, picked.tracks);
                ui.settle();
            }
        })
        .on_row_drag(
            |ui: &mut Ui, tracks: &[music::Track], drag: &mut rune::Drag, tx: &mut Tx| {
                if let Some(pin) = tracks.first().and_then(|track| track.pin()) {
                    sidebar_left::carry(ui, &pin, drag, tx);
                }
            },
        )
        .build(ui);
    remembered(ui, shelf, viewport, table)
}

/// The songs page's hero: what the shelf's songs are, how many and how long, and Play and
/// Shuffle over them in the order they show.
fn hero(ui: &Ui, shelf: Shelf, rows: &TrackRows, order: &Rc<Order>, filtering: bool) -> El {
    let theme = &ui.theme;
    let library = &ui.store.library;
    let listed = order.len();
    // While the songs are still arriving, the provider's own total is the count to show, so it
    // does not climb a page at a time. A filter counts what it kept.
    let count = match filtering {
        true => listed,
        false => library
            .expected(shelf, LibraryPart::Tracks)
            .map_or(listed, |expected| expected.max(listed)),
    };
    let duration = runtime(ui, shelf, rows, order);
    let mut meta = MetaStrip::new().text(t!("count-songs", count = count).into_owned());
    if !duration.is_zero() {
        meta = meta.text(ui::runtime(duration).into_owned());
    }
    let (title, icon, eyebrow) = match (library.shape(shelf), shelf) {
        (Shape::Catalog(_), Shelf::Local) => (t!("nav-songs"), "icons/disc-3.svg", t!("nav-local")),
        (Shape::Catalog(_), Shelf::Streaming) => {
            (t!("nav-songs"), "icons/disc-3.svg", t!("nav-library"))
        }
        (Shape::Saved, _) => (
            t!("library-liked-songs"),
            "icons/heart-filled.svg",
            t!("detail-playlist"),
        ),
    };
    let actions = row()
        .center_y()
        .gap(action_gap(theme))
        .child(PlayButton::new("play-library", Listing::Listed(rows.clone())).build(ui))
        .child(PlayButton::shuffle("shuffle-library", Listing::Listed(rows.clone())).build(ui));
    Hero::new("library-hero", title.into_owned())
        .fallback(icon)
        .accent()
        .eyebrow(eyebrow.into_owned())
        .meta(meta.build(theme))
        .actions(actions)
        .padded()
        .build(ui)
}

/// The listed songs' total length, summed again only when the rows change.
fn runtime(ui: &Ui, shelf: Shelf, rows: &TrackRows, order: &Rc<Order>) -> Duration {
    let cache = &ui.screens.library.shelf(shelf).runtime;
    if let Some((kept, duration)) = cache.borrow().as_ref()
        && Rc::ptr_eq(kept, order)
    {
        return *duration;
    }
    let tracks = rows.tracks(ui);
    let duration = (0..order.len())
        .filter_map(|display| order.source(display))
        .filter_map(|at| tracks.get(at))
        .map(|track| track.duration)
        .sum();
    *cache.borrow_mut() = Some((order.clone(), duration));
    duration
}

/// The albums, playlists or artists page: a grid of cards or a table, or what the page says
/// instead when there is nothing to list.
fn listing<K: Kind>(ui: &Ui, cx: &mut Cx<Ui>, shelf: Shelf, mode: Mode) -> El {
    let theme = &ui.theme;
    let inset = theme.metrics.inset;
    let count = items::count::<K>(ui, shelf);
    let waiting = count == 0
        && ui
            .store
            .library
            .waiting(shelf, K::SECTION.part(), &ui.store.network);
    let note = note(
        ui,
        shelf,
        K::SECTION,
        count,
        items::filtering::<K>(ui, shelf),
    );
    let bottom = player_bar::bottom(ui, cx) + inset;
    let viewport = items::viewport::<K>(shelf, mode);
    if let Some(note) = note {
        return centered(note);
    }
    match mode {
        Mode::List => {
            let table = items::table::<K>(ui, shelf, page::content_width(cx, 0.), bottom, waiting);
            remembered(ui, shelf, viewport, table)
        }
        Mode::Grid => {
            let available = page::content_width(cx, page::reserved(inset));
            let grid = match waiting {
                true => {
                    let tall = cx
                        .layout_with("content", |rect| rect[3].round())
                        .unwrap_or(0.);
                    items::pending_grid::<K>(ui, available, inset, tall)
                }
                false => items::grid::<K>(ui, shelf, available, inset, bottom),
            };
            let deck = col()
                .key(viewport.clone())
                .w_full()
                .grow(1.)
                .min_h(0.)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(el().key("library-deck-top").h(inset))
                .child(grid);
            remembered(ui, shelf, viewport, deck)
        }
    }
}

/// The scroll viewport keyed `viewport`, followed so the page can be put back where it was left
/// when the window comes back to it.
fn remembered(ui: &Ui, shelf: Shelf, viewport: String, list: El) -> El {
    let watch = &ui.screens.library.shelf(shelf).watch;
    col()
        .w_full()
        .h_full()
        .min_h(0.)
        .on_layout_tx(move |ui: &mut Ui, _: [f32; 4], tx: &mut Tx| {
            match ui.screens.library.shelf_mut(shelf).rewind.take() {
                Some(offset) if offset > 0. => tx.scroll_to_instant(viewport.clone(), 0., offset),
                _ => tx.live_only(),
            }
        })
        .child(watch.vertical(list, 0.))
}

/// A block centred in the page.
fn centered(block: El) -> El {
    col()
        .w_full()
        .h_full()
        .min_h(0.)
        .center()
        .padding(0.)
        .child(block)
}

/// Whether the local shelf has no folder to list. A scan in flight is not that: a folder is
/// only recorded once its scan lands, so the setup screen would otherwise cover the whole of the
/// first import, which is the longest one there is.
fn unconfigured(ui: &Ui, shelf: Shelf) -> bool {
    shelf.local() && ui.store.session.local_paths().is_empty() && ui.store.scan.progress().is_none()
}

/// What a page with `count` rows says instead of listing them, if anything: the scan filling an
/// empty local page, offline mode with nothing downloaded, a shelf that did not load, a search
/// that matched nothing, or the shelf's own empty line.
fn note(ui: &Ui, shelf: Shelf, section: Section, count: usize, filtering: bool) -> Option<El> {
    let theme = &ui.theme;
    let library = &ui.store.library;
    let shape = library.shape(shelf);
    if let Some(scanning) = scanning(ui, shelf, section, count) {
        return Some(scanning);
    }
    if library.waiting(shelf, section.part(), &ui.store.network) {
        return None;
    }
    let matchless = || {
        Vacancy::new(t!("library-no-matches"))
            .spoken("library-no-matches")
            .build(theme)
    };
    // Offline mode lists only what was downloaded, and with nothing there it says why.
    if ui.store.network.downloads_only(shelf) {
        return match (count, filtering) {
            (0, false) => Some(trouble::offline(theme)),
            (0, true) => Some(matchless()),
            _ => None,
        };
    }
    let retry = move |ui: &mut Ui| {
        let store = &mut ui.store;
        store.library.refresh(shelf, &store.session, &mut store.cx);
    };
    match library.state(shelf) {
        LibraryState::Loading => return None,
        LibraryState::Failed(reason) => {
            return Some(trouble::lost(
                ui,
                "library-lost",
                t!("library-not-loaded"),
                Some(reason),
                retry,
            ));
        }
        _ if count > 0 => return None,
        _ => {}
    }
    Some(
        match (filtering, library.part_problem(shelf, section.part())) {
            (true, _) => matchless(),
            (false, Some(reason)) => trouble::lost(
                ui,
                "library-part-lost",
                t!("library-part-not-loaded"),
                Some(reason),
                retry,
            ),
            (false, None) => Vacancy::new(i18n::lookup(section.vacancy(shelf, shape), None))
                .spoken("library-vacancy")
                .icon(section.glyph(shape))
                .build(theme),
        },
    )
}

/// What an empty local page says while a scan is filling it. The page is not empty, it is
/// early, so it counts the files read instead of offering the vacancy's caption.
fn scanning(ui: &Ui, shelf: Shelf, section: Section, count: usize) -> Option<El> {
    if !shelf.local() || count > 0 {
        return None;
    }
    let progress = ui.store.scan.progress()?;
    let caption = match progress.found {
        0 => t!("library-scanning"),
        found => t!(
            "library-scanning-progress",
            read = progress.read,
            found = found
        ),
    };
    Some(
        Vacancy::new(caption)
            .spoken("library-scanning")
            .icon(section.glyph(ui.store.library.shape(shelf)))
            .build(&ui.theme),
    )
}
