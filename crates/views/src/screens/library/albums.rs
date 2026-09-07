use std::cell::RefCell;
use std::cmp::Ordering;
use ui::{ActiveTheme as _, Filter, FilterChange, RangeAxis, Unit};

use gpui::{AnyElement, App, Entity, SharedString, TextAlign};
use i18n::t;
use music::Album;
use state::{Library, LibraryPart, LibraryState, Origin, Playback};
use ui::rank::{HANDY, NICE, SPARE, USEFUL};
use ui::{Cell, ColumnSpec, Menu, Pin, TableSource, Width};

use crate::shared::cells::{self, DATE, NUMBER, TRAILING, YEAR};
use crate::shared::menus::album_menu;
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
    ..COLUMN
};

pub(super) const COLUMNS: &[ColumnSpec<AlbumField>] = &[
    INDEX,
    COVER,
    NAME,
    ARTISTS,
    RELEASE_YEAR,
    ADDED_AT,
    TRACK_COUNT,
];

pub(super) struct AlbumSource {
    library: Entity<Library>,
    playback: Entity<Playback>,
    local: bool,
    year_span: Option<(f32, f32)>,
    spread: RefCell<Option<Spread>>,
}

struct Spread {
    stamp: (usize, String),
    years: Vec<f32>,
}

impl AlbumSource {
    pub(super) fn shelved(
        library: Entity<Library>,
        playback: Entity<Playback>,
        local: bool,
    ) -> Self {
        Self {
            library,
            playback,
            local,
            year_span: None,
            spread: RefCell::new(None),
        }
    }

    fn index_cell(&self, cell: &Cell<AlbumField>, album: &Album, cx: &App) -> AnyElement {
        let origin = Origin::album(album.id.clone()).named(album.name.clone());
        let state = self.playback.read(cx).playing_from(&origin);
        let played = origin.clone();
        let press = cells::toggle(&self.playback, state.clone(), move |playback, cx| {
            playback.play_origin(played.clone(), cx)
        });

        cells::index(cell, state, true, None, press, cx)
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<Album> {
        self.albums(cx).get(row).cloned()
    }

    pub(super) fn years(&self, query: &str, cx: &App) -> Vec<f32> {
        let stamp = (self.albums(cx).len(), query.to_owned());
        if let Some(spread) = self.spread.borrow().as_ref()
            && spread.stamp == stamp
        {
            return spread.years.clone();
        }

        let mut years: Vec<f32> = self
            .albums(cx)
            .iter()
            .filter(|album| album.year > 0 && hits(album, query))
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

    fn albums<'a>(&self, cx: &'a App) -> &'a [Album] {
        let library = self.library.read(cx);
        let state = match self.local {
            true => library.local_state(),
            false => library.state(),
        };
        match state {
            LibraryState::Ready { albums, .. } => albums.as_slice(),
            _ => &[],
        }
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
        self.at(row, cx).is_some_and(|album| {
            if let Some((low, high)) = self.year_span {
                let year = album.year as f32;
                if album.year == 0 || year < low - 0.5 || year > high + 0.5 {
                    return false;
                }
            }
            hits(&album, query)
        })
    }

    fn filter_axes(&self, query: &str, cx: &App) -> Vec<Filter> {
        let years = self.years(query, cx);
        let (Some(first), Some(last)) = (years.first(), years.last()) else {
            return Vec::new();
        };
        let bounds = (*first, *last);
        let value = self.year_span.unwrap_or(bounds);

        vec![Filter::Range(
            RangeAxis {
                key: "filter-year",
                label: t!("filter-year"),
                bounds,
                value,
                unit: Unit::Plain,
                values: Some(years),
            }
            .clamped(),
        )]
    }

    fn filter(&mut self, change: FilterChange, _cx: &App) -> bool {
        match change {
            FilterChange::Range("filter-year", value) => {
                self.year_span = Some(value);
                true
            }
            FilterChange::Reset => {
                self.year_span = None;
                true
            }
            _ => false,
        }
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.year_span.is_some()
    }

    fn playing(&self, row: usize, cx: &App) -> bool {
        self.albums(cx).get(row).is_some_and(|album| {
            let origin = Origin::album(album.id.clone());
            self.playback.read(cx).playing_from(&origin).is_some()
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        match self.local {
            true => self.library.read(cx).local_loading(LibraryPart::Albums),
            false => self.library.read(cx).loading(LibraryPart::Albums),
        }
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
            false,
            cx,
        ))
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
            AlbumField::Name => cells::dim(&cell, album.name.clone(), theme.foreground),
            AlbumField::Artists => cells::artists(
                &cell,
                album.artist_refs.clone(),
                album.artists.clone(),
                muted,
            ),
            AlbumField::Year => cells::dim(&cell, year(album), muted),
            AlbumField::TrackCount => cells::dim(&cell, format!("{}", album.track_count), muted),
            AlbumField::AddedAt => cells::dim(&cell, cells::stamp(album.added_at), muted),
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
    holds(&album.name, query) || holds(&album.artists, query) || holds(&year(album), query)
}

fn year(album: &Album) -> String {
    if album.year > 0 {
        format!("{}", album.year)
    } else {
        String::new()
    }
}
