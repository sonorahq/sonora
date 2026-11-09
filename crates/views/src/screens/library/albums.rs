//! A shelf's albums: their columns, search, filters (year, favorites, downloaded), sort and
//! groups, and how a row and a card draw one.

use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use i18n::t;
use music::Album;
use router::Destination;
use rune::El;
use state::{Collection, Origin, Shelf};
use ui::rank::{HANDY, NICE, SPARE, USEFUL};
use ui::{
    Card, ColumnSpec, Filter, FilterChange, FlagAxis, Pin, RangeAxis, Sort, TableCell, TextAlign,
    Unit, Width,
};

use super::Section;
use super::items::{Items, Kind, index_cell};
use crate::Ui;
use crate::shared::cards::{self, Fit};
use crate::shared::cells::{self, DATE, NUMBER, TRAILING, YEAR};
use crate::shared::menus::Target;
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};
use crate::shared::tracks::initial;

/// What one column of the albums table shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlbumField {
    Index,
    Cover,
    Name,
    Artists,
    Year,
    TrackCount,
    AddedAt,
}

const COLUMN: ColumnSpec<AlbumField> = ColumnSpec::filling(AlbumField::Index);

const INDEX: ColumnSpec<AlbumField> = ColumnSpec::numbering(AlbumField::Index, NUMBER);

const COVER: ColumnSpec<AlbumField> = ColumnSpec::artwork(AlbumField::Cover);

const NAME: ColumnSpec<AlbumField> = ColumnSpec {
    field: AlbumField::Name,
    key: "name",
    header: "column-album",
    width: Width::Fill(0.55),
    ..COLUMN
};

const ARTISTS: ColumnSpec<AlbumField> = ColumnSpec {
    field: AlbumField::Artists,
    key: "artists",
    header: "column-artist",
    width: Width::Fill(0.45),
    rank: USEFUL,
    ..COLUMN
};

const RELEASE_YEAR: ColumnSpec<AlbumField> = ColumnSpec {
    field: AlbumField::Year,
    key: "year",
    header: "column-year",
    align: TextAlign::Right,
    width: Width::Fixed(YEAR),
    rank: HANDY,
    ..COLUMN
};

const TRACK_COUNT: ColumnSpec<AlbumField> = ColumnSpec {
    field: AlbumField::TrackCount,
    key: "tracks",
    header: "column-tracks",
    align: TextAlign::Right,
    width: Width::Fixed(TRAILING),
    rank: SPARE,
    ..COLUMN
};

const ADDED_AT: ColumnSpec<AlbumField> = ColumnSpec {
    field: AlbumField::AddedAt,
    key: "added-at",
    header: "column-date-added",
    width: Width::Fixed(DATE),
    rank: NICE,
    first: Sort::Descending,
    ..COLUMN
};

const COLUMNS: &[ColumnSpec<AlbumField>] = &[
    INDEX,
    COVER,
    NAME,
    ARTISTS,
    RELEASE_YEAR,
    ADDED_AT,
    TRACK_COUNT,
];

/// What the albums' filter popover narrows them by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AlbumSieve {
    years: Option<(f32, f32)>,
    starred: bool,
    /// Keep only albums that were downloaded whole.
    downloaded: bool,
}

/// Every year the shelf holds, in order and without repeats, by where the albums were and how
/// many. The stops ignore the search and the other filters, so narrowing the grid to nothing
/// still leaves the slider standing at its full span.
#[derive(Default)]
pub(crate) struct Years {
    stamp: (usize, usize),
    years: Rc<Vec<f32>>,
}

/// A shelf's albums.
pub(crate) struct Albums;

impl Kind for Albums {
    type Item = Album;
    type Field = AlbumField;
    type Sieve = AlbumSieve;
    type Memo = Option<Years>;

    const SECTION: Section = Section::Albums;
    const NAME: &'static str = "album";
    const COLUMNS: &'static [ColumnSpec<AlbumField>] = COLUMNS;
    const FIRST: &'static str = "added-at";
    const AXES: &'static [&'static str] = &["filter-year", "filter-favorites", "filter-downloaded"];

    fn items(ui: &Ui, shelf: Shelf) -> &[Album] {
        ui.store.library.state(shelf).albums()
    }

    fn list(ui: &Ui, shelf: Shelf) -> &Items<Self> {
        &ui.screens.library.shelf(shelf).albums
    }

    fn list_mut(ui: &mut Ui, shelf: Shelf) -> &mut Items<Self> {
        &mut ui.screens.library.shelf_mut(shelf).albums
    }

    fn id(album: &Album) -> &str {
        &album.id
    }

    fn name(album: &Album) -> &str {
        &album.name
    }

    fn hits(album: &Album, query: &str) -> bool {
        query.is_empty()
            || holds(&album.name, query)
            || holds(&album.artists, query)
            || (album.year > 0 && holds(&album.year.to_string(), query))
    }

    fn keeps(ui: &Ui, shelf: Shelf, sieve: &AlbumSieve, album: &Album) -> bool {
        if sieve.starred && !ui.store.library.saved_album(&album.id) {
            return false;
        }
        if (sieve.downloaded || ui.store.network.downloads_only(shelf))
            && !downloaded(ui, &album.id)
        {
            return false;
        }
        match sieve.years {
            Some((low, high)) => {
                let year = album.year as f32;
                album.year != 0 && year >= low - 0.5 && year <= high + 0.5
            }
            None => true,
        }
    }

    fn facts(ui: &Ui, shelf: Shelf, sieve: &AlbumSieve, albums: &[Album]) -> u64 {
        let downloads = sieve.downloaded || ui.store.network.downloads_only(shelf);
        if !sieve.starred && !downloads {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        for album in albums {
            (
                sieve.starred && ui.store.library.saved_album(&album.id),
                downloads && downloaded(ui, &album.id),
            )
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    fn compare(a: &Album, b: &Album, field: AlbumField) -> Ordering {
        match field {
            AlbumField::Name => folded(&a.name, &b.name),
            AlbumField::Artists => folded(&a.artists, &b.artists),
            AlbumField::Year => a.year.cmp(&b.year),
            AlbumField::TrackCount => a.track_count.cmp(&b.track_count),
            AlbumField::AddedAt => a.added_at.cmp(&b.added_at),
            AlbumField::Index | AlbumField::Cover => Ordering::Equal,
        }
    }

    fn group(album: &Album, field: AlbumField) -> Option<String> {
        match field {
            AlbumField::Name => Some(initial(&album.name)),
            AlbumField::Artists => Some(initial(&album.artists)),
            AlbumField::Year => Some(match album.year {
                0 => t!("common-unknown").into_owned(),
                year => year.to_string(),
            }),
            _ => None,
        }
    }

    fn axes(ui: &Ui, shelf: Shelf, sieve: &AlbumSieve) -> Vec<Filter> {
        let mut axes = Vec::new();
        let years = years(ui, shelf);
        if let (Some(first), Some(last)) = (years.first(), years.last()) {
            let bounds = (*first, *last);
            axes.push(Filter::Range(
                RangeAxis {
                    key: "filter-year",
                    label: t!("filter-year"),
                    bounds,
                    value: sieve.years.unwrap_or(bounds),
                    unit: Unit::Plain,
                    values: Some(years.to_vec()),
                }
                .clamped(),
            ));
        }
        if ui.store.library.shape(shelf).is_catalog() {
            axes.push(Filter::Flag(FlagAxis {
                key: "filter-favorites",
                label: t!("filter-favorites"),
                on: sieve.starred,
            }));
        }
        // Offline mode already keeps the shelf to its downloads, so the filter would do nothing.
        let offered = sieve.downloaded || !ui.store.downloads.is_empty();
        if offered && !ui.store.network.downloads_only(shelf) {
            axes.push(Filter::Flag(FlagAxis {
                key: "filter-downloaded",
                label: t!("filter-downloaded"),
                on: sieve.downloaded,
            }));
        }
        axes
    }

    fn filter(sieve: &mut AlbumSieve, change: FilterChange) -> bool {
        match change {
            FilterChange::Range("filter-year", value) => sieve.years = Some(value),
            FilterChange::Flag("filter-favorites", on) => sieve.starred = on,
            FilterChange::Flag("filter-downloaded", on) => sieve.downloaded = on,
            FilterChange::Reset => *sieve = AlbumSieve::default(),
            _ => return false,
        }
        true
    }

    fn narrowed(sieve: &AlbumSieve) -> bool {
        sieve.years.is_some() || sieve.starred || sieve.downloaded
    }

    fn cell(ui: &Ui, _: Shelf, album: &Album, cell: &TableCell<AlbumField>, frame: El) -> El {
        let theme = &ui.theme;
        let muted = theme.muted_foreground;
        match cell.field {
            AlbumField::Index => index_cell(ui, origin(album), cell.row, frame),
            AlbumField::Cover => cells::artwork(frame, ui.artwork(), album.cover.as_deref(), theme),
            AlbumField::Name => {
                cells::dim(frame, "name", album.name.clone(), theme.foreground, theme)
            }
            AlbumField::Artists => cells::artists(
                frame,
                &album.artist_refs,
                album.artists.clone(),
                muted,
                theme,
            ),
            AlbumField::Year => cells::dim(frame, "year", year(album), muted, theme),
            AlbumField::TrackCount => {
                cells::dim(frame, "tracks", album.track_count.to_string(), muted, theme)
            }
            AlbumField::AddedAt => cells::dim(
                frame,
                "added-at",
                cells::stamp(album.added_at),
                muted,
                theme,
            ),
        }
    }

    fn card(ui: &Ui, _: Shelf, album: &Album, key: String, width: f32) -> Card {
        let opener = super::opener();
        cards::album_card(ui, key, album, Fit::Tile(width), Some(&opener))
    }

    fn origin(album: &Album) -> Origin {
        origin(album)
    }

    fn open(album: &Album) -> Destination {
        Destination::Album(album.id.as_str().into())
    }

    fn menu(album: &Album) -> Target {
        Target::Album(album.clone())
    }

    fn pin(album: &Album) -> Option<Pin> {
        album.pin()
    }

    fn remove(ui: &mut Ui, _: Shelf, albums: Vec<Album>) {
        crate::shared::confirm::albums(ui, albums);
    }
}

/// Where playback names an album's queue as coming from.
fn origin(album: &Album) -> Origin {
    Origin::album(album.id.clone()).named(album.name.clone())
}

/// Every year the shelf's albums hold, worked out again only when the albums change.
fn years(ui: &Ui, shelf: Shelf) -> Rc<Vec<f32>> {
    let albums = Albums::items(ui, shelf);
    let stamp = (albums.as_ptr() as usize, albums.len());
    let mut memo = Albums::list(ui, shelf).memo.borrow_mut();
    if let Some(kept) = memo.as_ref()
        && kept.stamp == stamp
    {
        return kept.years.clone();
    }
    let mut years: Vec<f32> = albums
        .iter()
        .filter(|album| album.year > 0)
        .map(|album| album.year as f32)
        .collect();
    years.sort_by(f32::total_cmp);
    years.dedup();
    let years = Rc::new(years);
    *memo = Some(Years {
        stamp,
        years: years.clone(),
    });
    years
}

/// The year a row shows, blank when unknown.
fn year(album: &Album) -> String {
    match album.year > 0 {
        true => album.year.to_string(),
        false => String::new(),
    }
}

/// Whether an album was downloaded whole, or is on its way.
fn downloaded(ui: &Ui, id: &str) -> bool {
    ui.store.downloads.set(Collection::Album, id).is_some()
}
