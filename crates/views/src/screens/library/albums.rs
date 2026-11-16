use std::cell::RefCell;
use std::cmp::Ordering;
use std::io::Write as _;
use ui::{ActiveTheme as _, Filter, FilterChange, FlagAxis, RangeAxis, Unit};

use gpui::{AnyElement, App, Entity, SharedString, TextAlign};
use i18n::t;
use music::Album;
use state::{
    Collection, Library, LibraryPart, LibraryState, Network, Origin, Playback, Shelf, Sonora,
};
use ui::rank::{HANDY, NICE, SPARE, USEFUL};
use ui::{Cell, ColumnSpec, Menu, Pin, Sort, TableSource, Width};

use crate::shared::blend::{self, Joined, Reach};
use crate::shared::cells::{self, DATE, NUMBER, TRAILING, YEAR};
use crate::shared::menus::{ItemMenu, album_menu};
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};
use crate::shared::tracks::initial;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AlbumField {
    Index,
    Cover,
    Name,
    Artists,
    Year,
    TrackCount,
    AddedAt,
    LikedAt,
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

/// The day a favorite was made, for a shelf that lists its whole catalog and so dates each row by
/// when it joined. It stays off until the listener turns it on, and sorts from the sort menu.
const LIKED_AT: ColumnSpec<AlbumField> = ColumnSpec {
    field: AlbumField::LikedAt,
    key: "liked-at",
    header: "column-date-liked",
    optional: true,
    ..ADDED_AT
};

pub(super) const COLUMNS: &[ColumnSpec<AlbumField>] = &[
    INDEX,
    COVER,
    NAME,
    ARTISTS,
    RELEASE_YEAR,
    ADDED_AT,
    LIKED_AT,
    TRACK_COUNT,
];

pub(super) struct AlbumSource {
    library: Entity<Library>,
    playback: Entity<Playback>,
    /// The state of the row menu's Add to playlist submenu.
    menu: ItemMenu,
    /// The shelf the Local only or Streaming only filter keeps the albums to.
    reach: Reach,
    year_span: Option<(f32, f32)>,
    starred: bool,
    /// Keep only albums that were downloaded whole.
    downloaded: bool,
    spread: RefCell<Option<Spread>>,
}

struct Spread {
    stamp: usize,
    years: Vec<f32>,
}

impl AlbumSource {
    pub(super) fn shelved(
        library: Entity<Library>,
        playback: Entity<Playback>,
        menu: ItemMenu,
    ) -> Self {
        Self {
            library,
            playback,
            menu,
            reach: Reach::default(),
            year_span: None,
            starred: false,
            downloaded: false,
            spread: RefCell::new(None),
        }
    }

    /// Whether the page badges its local albums, see `Reach::marks_local`.
    pub(super) fn marks_local(&self, cx: &App) -> bool {
        self.reach.marks_local(self.albums(cx).mixed())
    }

    fn index_cell(&self, cell: &Cell<AlbumField>, album: &Album, cx: &App) -> AnyElement {
        let origin = Origin::album(album.id.clone()).named(album.name.clone());
        let playing = self.playback.read(cx).playing_from(&origin, cx);
        let press = cells::toggle(&self.playback, playing, move |playback, cx| {
            playback.play_origin(origin.clone(), cx)
        });

        cells::index(cell, playing, true, None, None, press, cx)
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<Album> {
        self.albums(cx).get(row).cloned()
    }

    /// Every year the library holds, in order and without repeats. The stops deliberately ignore
    /// the search and the other filters, so narrowing the grid to nothing still leaves the
    /// slider standing at its full span.
    pub(super) fn years(&self, cx: &App) -> Vec<f32> {
        let stamp = self.albums(cx).len();
        if let Some(spread) = self.spread.borrow().as_ref()
            && spread.stamp == stamp
        {
            return spread.years.clone();
        }

        let mut years: Vec<f32> = self
            .albums(cx)
            .iter()
            .filter(|album| album.year > 0)
            .map(|album| album.year as f32)
            .collect();
        years.sort_by(f32::total_cmp);
        years.dedup();
        *self.spread.borrow_mut() = Some(Spread {
            stamp,
            years: years.clone(),
        });

        years
    }

    /// The albums of both shelves, the streaming ones first.
    fn albums<'a>(&self, cx: &'a App) -> Joined<'a, Album> {
        blend::joined(self.library.read(cx), LibraryState::albums)
    }

    /// Whether a shelf with albums listed lists more than the favorites, so a favorites filter
    /// has something to do.
    fn catalog(&self, cx: &App) -> bool {
        let library = self.library.read(cx);
        [Shelf::Streaming, Shelf::Local].into_iter().any(|shelf| {
            library.shape(shelf).is_catalog() && !library.state(shelf).albums().is_empty()
        })
    }

    /// The day the listener favorited an album, when a catalog shelf knows it.
    fn liked_at(&self, album: &Album, cx: &App) -> Option<i64> {
        self.library
            .read(cx)
            .liked_at(LibraryPart::Albums, &album.id)
    }

    /// Whether offline mode keeps every album to its downloads, which holds only while there
    /// are no local ones.
    fn downloads_only(&self, cx: &App) -> bool {
        Network::downloads_only(Shelf::Streaming, cx)
            && self
                .library
                .read(cx)
                .state(Shelf::Local)
                .albums()
                .is_empty()
    }
}

impl TableSource for AlbumSource {
    type Field = AlbumField;

    fn columns(&self) -> &'static [ColumnSpec<AlbumField>] {
        COLUMNS
    }

    fn rows(&self, cx: &App) -> usize {
        self.albums(cx).len()
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        self.albums(cx).get(row).is_some_and(|album| {
            if self.starred && !self.library.read(cx).saved_album(&album.id) {
                return false;
            }
            if !self.reach.keeps(&album.id) {
                return false;
            }
            if (self.downloaded || Network::downloads_only(Shelf::of(&album.id), cx))
                && !downloaded(&album.id, cx)
            {
                return false;
            }
            if let Some((low, high)) = self.year_span {
                let year = album.year as f32;
                if album.year == 0 || year < low - 0.5 || year > high + 0.5 {
                    return false;
                }
            }
            hits(album, query)
        })
    }

    fn filter_axes(&self, cx: &App) -> Vec<Filter> {
        let mut axes = Vec::new();
        let years = self.years(cx);
        if let (Some(first), Some(last)) = (years.first(), years.last()) {
            let bounds = (*first, *last);
            let value = self.year_span.unwrap_or(bounds);
            axes.push(Filter::Range(
                RangeAxis {
                    key: "filter-year",
                    label: t!("filter-year"),
                    bounds,
                    value,
                    unit: Unit::Plain,
                    values: Some(years),
                }
                .clamped(),
            ));
        }
        if self.catalog(cx) {
            axes.push(Filter::Flag(FlagAxis {
                key: "filter-favorites",
                label: t!("filter-favorites"),
                on: self.starred,
            }));
        }
        // Offline mode already keeps the shelf to its downloads, so the filter would do nothing.
        let offered = self.downloaded || !Sonora::global(cx).downloads.read(cx).is_empty();
        if offered && !self.downloads_only(cx) {
            axes.push(Filter::Flag(FlagAxis {
                key: "filter-downloaded",
                label: t!("filter-downloaded"),
                on: self.downloaded,
            }));
        }
        axes.extend(self.reach.axes(self.albums(cx).mixed()));
        axes
    }

    fn filter(&mut self, change: FilterChange, _cx: &App) -> bool {
        if self.reach.filter(&change) {
            return true;
        }
        match change {
            FilterChange::Range("filter-year", value) => {
                self.year_span = Some(value);
                true
            }
            FilterChange::Flag("filter-favorites", value) => {
                self.starred = value;
                true
            }
            FilterChange::Flag("filter-downloaded", value) => {
                self.downloaded = value;
                true
            }
            FilterChange::Reset => {
                self.year_span = None;
                self.starred = false;
                self.downloaded = false;
                self.reach = Reach::default();
                true
            }
            _ => false,
        }
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.year_span.is_some() || self.starred || self.downloaded || self.reach.active()
    }

    fn playing(&self, row: usize, cx: &App) -> bool {
        self.albums(cx).get(row).is_some_and(|album| {
            let origin = Origin::album(album.id.clone());
            self.playback.read(cx).playing_from(&origin, cx).is_some()
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        blend::waiting(self.library.read(cx), LibraryPart::Albums, cx)
    }

    fn pin(&self, row: usize, cx: &App) -> Option<Pin> {
        self.albums(cx).get(row)?.pin()
    }

    fn picking(&self) -> bool {
        true
    }

    fn context_menu(&self, rows: &[usize], _visible: &[AlbumField], cx: &App) -> Option<Menu> {
        Some(album_menu(
            self.at(*rows.first()?, cx)?,
            self.playback.clone(),
            &self.menu,
            cx,
        ))
    }

    fn context_menu_will_open(&self, _rows: &[usize], cx: &App) {
        self.menu.reset(cx);
    }

    fn populated(&self, field: AlbumField, cx: &App) -> bool {
        match field {
            AlbumField::LikedAt => self.library.read(cx).dates_likes(LibraryPart::Albums),
            _ => true,
        }
    }

    fn cell(&self, cell: Cell<AlbumField>, cx: &mut App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        let Some(album) = self.albums(cx).get(cell.row) else {
            return cells::blank(&cell);
        };

        if cell.field == AlbumField::Index {
            return self.index_cell(&cell, album, cx);
        }

        match cell.field {
            AlbumField::Cover => cells::artwork(&cell, album.cover.clone()),
            AlbumField::Name => cells::name(
                &cell,
                album.name.clone(),
                theme.foreground,
                self.marks_local(cx) && Shelf::of(&album.id).local(),
            ),
            AlbumField::Artists => cells::artists(
                &cell,
                album.artist_refs.clone(),
                album.artists.clone(),
                muted,
            ),
            AlbumField::Year => cells::dim(&cell, year(album), muted),
            AlbumField::TrackCount => cells::dim(&cell, format!("{}", album.track_count), muted),
            AlbumField::AddedAt => cells::dim(&cell, cells::stamp(album.added_at), muted),
            AlbumField::LikedAt => cells::dim(&cell, cells::stamp(self.liked_at(album, cx)), muted),
            AlbumField::Index => cells::blank(&cell),
        }
    }

    fn compare(&self, field: AlbumField, a: usize, b: usize, cx: &App) -> Ordering {
        let albums = self.albums(cx);
        let text = |index: usize, pick: fn(&Album) -> &str| {
            albums.get(index).map(pick).unwrap_or_default()
        };

        match field {
            AlbumField::Name => folded(text(a, |album| &album.name), text(b, |album| &album.name)),
            AlbumField::Artists => folded(
                text(a, |album| &album.artists),
                text(b, |album| &album.artists),
            ),
            AlbumField::Year => albums
                .get(a)
                .map(|album| album.year)
                .cmp(&albums.get(b).map(|album| album.year)),
            AlbumField::TrackCount => albums
                .get(a)
                .map(|album| album.track_count)
                .cmp(&albums.get(b).map(|album| album.track_count)),
            AlbumField::AddedAt => albums
                .get(a)
                .map(|album| album.added_at)
                .cmp(&albums.get(b).map(|album| album.added_at)),
            AlbumField::LikedAt => {
                let at = |index| albums.get(index).and_then(|album| self.liked_at(album, cx));
                at(a).cmp(&at(b))
            }
            AlbumField::Index | AlbumField::Cover => a.cmp(&b),
        }
    }

    fn group(&self, field: AlbumField, row: usize, cx: &App) -> Option<SharedString> {
        let album = self.albums(cx).get(row)?;

        match field {
            AlbumField::Name => Some(initial(&album.name)),
            AlbumField::Artists => Some(initial(&album.artists)),
            AlbumField::Year => Some(match album.year {
                0 => t!("common-unknown"),
                year => SharedString::from(year.to_string()),
            }),
            _ => None,
        }
    }
}

fn hits(album: &Album, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    holds(&album.name, query) || holds(&album.artists, query) || year_holds(album, query)
}

/// Whether the album's year, as `year` prints it, holds the query. The digits are written on the
/// stack, since a filter asks this of every album on every keystroke.
fn year_holds(album: &Album, query: &str) -> bool {
    if album.year <= 0 {
        return query.is_empty();
    }
    let mut digits = [0u8; 12];
    let mut cursor = std::io::Cursor::new(&mut digits[..]);
    if write!(cursor, "{}", album.year).is_err() {
        return false;
    }
    let written = cursor.position() as usize;
    std::str::from_utf8(&digits[..written]).is_ok_and(|text| holds(text, query))
}

fn year(album: &Album) -> String {
    if album.year > 0 {
        format!("{}", album.year)
    } else {
        String::new()
    }
}

/// Whether an album was downloaded whole, or is on its way.
fn downloaded(id: &str, cx: &App) -> bool {
    Sonora::global(cx)
        .downloads
        .read(cx)
        .set(Collection::Album, id)
        .is_some()
}
