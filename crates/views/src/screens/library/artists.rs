use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;
use ui::{ActiveTheme as _, Filter, FilterChange, FlagAxis};

use gpui::{AnyElement, App, Entity, SharedString};
use i18n::t;
use music::SavedArtist;
use state::{Library, LibraryPart, Network, Origin, Playback, Shelf, Sonora};
use ui::rank::{ESSENTIAL, HANDY};
use ui::{Cell, ColumnSpec, Menu, Pin, Sort, TableSource, Width};

use crate::shared::cells::{self, DATE, NUMBER};
use crate::shared::menus::artist_menu;
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};
use crate::shared::tracks::initial;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ArtistField {
    Index,
    Cover,
    Name,
    AddedAt,
}

const COLUMN: ColumnSpec<ArtistField> = ColumnSpec::filling(ArtistField::Index);

const INDEX: ColumnSpec<ArtistField> = ColumnSpec::numbering(ArtistField::Index, NUMBER);

const COVER: ColumnSpec<ArtistField> = ColumnSpec::artwork(ArtistField::Cover);

const NAME: ColumnSpec<ArtistField> = ColumnSpec {
    field: ArtistField::Name,
    key: "name",
    header: "column-name",
    rank: ESSENTIAL,
    ..COLUMN
};

const ADDED_AT: ColumnSpec<ArtistField> = ColumnSpec {
    field: ArtistField::AddedAt,
    key: "added-at",
    header: "column-date-added",
    width: Width::Fixed(DATE),
    rank: HANDY,
    first: Sort::Descending,
    ..COLUMN
};

pub(super) const COLUMNS: &[ColumnSpec<ArtistField>] = &[INDEX, COVER, NAME, ADDED_AT];

pub(super) struct ArtistSource {
    library: Entity<Library>,
    playback: Entity<Playback>,
    shelf: Shelf,
    starred: bool,
    heard: RefCell<Option<Heard>>,
}

/// The artists there is downloaded music of, worked out again only when the shelf's songs or the
/// downloads change in number.
struct Heard {
    stamp: (usize, usize),
    artists: HashSet<String>,
}

impl ArtistSource {
    pub(super) fn shelved(
        library: Entity<Library>,
        playback: Entity<Playback>,
        shelf: Shelf,
    ) -> Self {
        Self {
            library,
            playback,
            shelf,
            starred: false,
            heard: RefCell::new(None),
        }
    }

    fn index_cell(&self, cell: &Cell<ArtistField>, artist: &SavedArtist, cx: &App) -> AnyElement {
        let origin = Origin::artist(artist.id.clone()).named(artist.name.clone());
        let playing = self.playback.read(cx).playing_from(&origin, cx);
        let played = origin.clone();
        let press = cells::toggle(&self.playback, playing, move |playback, cx| {
            playback.play_origin(played.clone(), cx)
        });

        cells::index(cell, playing, true, None, None, press, cx)
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<SavedArtist> {
        self.artists(cx).get(row).cloned()
    }

    fn artists<'a>(&self, cx: &'a App) -> &'a [SavedArtist] {
        self.library.read(cx).state(self.shelf).artists()
    }

    /// Whether there is downloaded music of an artist, on a downloaded album or playlist or in
    /// a downloaded song of the shelf. This is what offline mode keeps the list to.
    fn downloaded(&self, id: &str, cx: &App) -> bool {
        let tracks = self.library.read(cx).state(self.shelf).tracks();
        let downloads = Sonora::global(cx).downloads.read(cx);
        let stamp = (tracks.len(), downloads.len());
        let mut heard = self.heard.borrow_mut();
        if heard.as_ref().is_none_or(|heard| heard.stamp != stamp) {
            let kept = tracks
                .iter()
                .filter(|track| track.id.as_deref().is_some_and(|id| downloads.has(id)))
                .flat_map(|track| &track.artist_refs)
                .filter_map(|artist| artist.id.as_deref());
            let artists = downloads
                .credited()
                .chain(kept)
                .map(str::to_owned)
                .collect();
            *heard = Some(Heard { stamp, artists });
        }
        heard
            .as_ref()
            .is_some_and(|heard| heard.artists.contains(id))
    }

    /// Whether the shelf lists more than the favorites, so a favorites filter has something to do.
    fn catalog(&self, cx: &App) -> bool {
        self.library.read(cx).shape(self.shelf).is_catalog()
    }
}

impl TableSource for ArtistSource {
    type Field = ArtistField;

    fn columns(&self) -> &'static [ColumnSpec<ArtistField>] {
        COLUMNS
    }

    fn rows(&self, cx: &App) -> usize {
        self.artists(cx).len()
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        self.artists(cx).get(row).is_some_and(|artist| {
            if self.starred && !self.library.read(cx).saved_artist(&artist.id) {
                return false;
            }
            if Network::downloads_only(self.shelf, cx) && !self.downloaded(&artist.id, cx) {
                return false;
            }
            holds(&artist.name, query)
        })
    }

    fn filter_axes(&self, cx: &App) -> Vec<Filter> {
        match self.catalog(cx) {
            true => vec![Filter::Flag(FlagAxis {
                key: "filter-favorites",
                label: t!("filter-favorites"),
                on: self.starred,
            })],
            false => Vec::new(),
        }
    }

    fn filter(&mut self, change: FilterChange, _cx: &App) -> bool {
        match change {
            FilterChange::Flag("filter-favorites", value) => {
                self.starred = value;
                true
            }
            FilterChange::Reset => {
                self.starred = false;
                true
            }
            _ => false,
        }
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.starred
    }

    fn playing(&self, row: usize, cx: &App) -> bool {
        self.artists(cx).get(row).is_some_and(|artist| {
            let origin = Origin::artist(artist.id.clone());
            self.playback.read(cx).playing_from(&origin, cx).is_some()
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.library
            .read(cx)
            .waiting(self.shelf, LibraryPart::Artists, cx)
    }

    fn pin(&self, row: usize, cx: &App) -> Option<Pin> {
        self.artists(cx).get(row)?.pin()
    }

    fn picking(&self) -> bool {
        true
    }

    fn context_menu(&self, rows: &[usize], _visible: &[ArtistField], cx: &App) -> Option<Menu> {
        Some(artist_menu(
            self.at(*rows.first()?, cx)?,
            self.playback.clone(),
            false,
            cx,
        ))
    }

    fn cell(&self, cell: Cell<ArtistField>, cx: &mut App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        let Some(artist) = self.artists(cx).get(cell.row) else {
            return cells::blank(&cell);
        };

        if cell.field == ArtistField::Index {
            return self.index_cell(&cell, artist, cx);
        }

        match cell.field {
            ArtistField::Cover => cells::avatar(&cell, artist.cover.clone()),
            ArtistField::Name => cells::dim(&cell, artist.name.clone(), theme.foreground),
            ArtistField::AddedAt => cells::dim(&cell, cells::stamp(artist.added_at), muted),
            ArtistField::Index => cells::blank(&cell),
        }
    }

    fn compare(&self, field: ArtistField, a: usize, b: usize, cx: &App) -> Ordering {
        let artists = self.artists(cx);
        let at = |index: usize| artists.get(index);

        match field {
            ArtistField::Name => folded(
                at(a).map(|artist| artist.name.as_str()).unwrap_or_default(),
                at(b).map(|artist| artist.name.as_str()).unwrap_or_default(),
            ),
            ArtistField::AddedAt => at(a)
                .map(|artist| artist.added_at)
                .cmp(&at(b).map(|artist| artist.added_at)),
            ArtistField::Index | ArtistField::Cover => a.cmp(&b),
        }
    }

    fn group(&self, field: ArtistField, row: usize, cx: &App) -> Option<SharedString> {
        let artist = self.artists(cx).get(row)?;

        match field {
            ArtistField::Name => Some(initial(&artist.name)),
            _ => None,
        }
    }
}
