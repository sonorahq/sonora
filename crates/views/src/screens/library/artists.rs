//! A shelf's artists: their columns, search, the favorites filter, sort and groups, and how a
//! row and a round card draw one.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use i18n::t;
use music::SavedArtist;
use router::Destination;
use rune::El;
use state::{Origin, Shelf};
use ui::rank::{ESSENTIAL, HANDY};
use ui::{Card, ColumnSpec, Filter, FilterChange, FlagAxis, Pin, Sort, TableCell, Width};

use super::Section;
use super::items::{Items, Kind, index_cell};
use crate::Ui;
use crate::shared::cards::{self, Fit};
use crate::shared::cells::{self, DATE, NUMBER};
use crate::shared::menus::Target;
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};
use crate::shared::tracks::initial;

/// What one column of the artists table shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtistField {
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

const COLUMNS: &[ColumnSpec<ArtistField>] = &[INDEX, COVER, NAME, ADDED_AT];

/// What the artists' filter popover narrows them by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ArtistSieve {
    starred: bool,
}

/// The artists there is downloaded music of, worked out again only when the shelf's songs or
/// the downloads change in number.
#[derive(Default)]
pub(crate) struct Heard {
    stamp: (usize, usize),
    artists: Rc<HashSet<String>>,
}

/// A shelf's artists.
pub(crate) struct Artists;

impl Kind for Artists {
    type Item = SavedArtist;
    type Field = ArtistField;
    type Sieve = ArtistSieve;
    type Memo = Option<Heard>;

    const SECTION: Section = Section::Artists;
    const NAME: &'static str = "artist";
    const COLUMNS: &'static [ColumnSpec<ArtistField>] = COLUMNS;
    const FIRST: &'static str = "added-at";
    const AXES: &'static [&'static str] = &["filter-favorites"];

    fn items(ui: &Ui, shelf: Shelf) -> &[SavedArtist] {
        ui.store.library.state(shelf).artists()
    }

    fn list(ui: &Ui, shelf: Shelf) -> &Items<Self> {
        &ui.screens.library.shelf(shelf).artists
    }

    fn list_mut(ui: &mut Ui, shelf: Shelf) -> &mut Items<Self> {
        &mut ui.screens.library.shelf_mut(shelf).artists
    }

    fn id(artist: &SavedArtist) -> &str {
        &artist.id
    }

    fn name(artist: &SavedArtist) -> &str {
        &artist.name
    }

    fn hits(artist: &SavedArtist, query: &str) -> bool {
        holds(&artist.name, query)
    }

    fn keeps(ui: &Ui, shelf: Shelf, sieve: &ArtistSieve, artist: &SavedArtist) -> bool {
        if sieve.starred && !ui.store.library.saved_artist(&artist.id) {
            return false;
        }
        !ui.store.network.downloads_only(shelf) || heard(ui, shelf).contains(&artist.id)
    }

    fn facts(ui: &Ui, shelf: Shelf, sieve: &ArtistSieve, artists: &[SavedArtist]) -> u64 {
        let downloads = ui.store.network.downloads_only(shelf);
        if !sieve.starred && !downloads {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        if downloads {
            let tracks = ui.store.library.state(shelf).tracks();
            (tracks.len(), ui.store.downloads.len()).hash(&mut hasher);
        }
        if sieve.starred {
            for artist in artists {
                ui.store.library.saved_artist(&artist.id).hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn compare(a: &SavedArtist, b: &SavedArtist, field: ArtistField) -> Ordering {
        match field {
            ArtistField::Name => folded(&a.name, &b.name),
            ArtistField::AddedAt => a.added_at.cmp(&b.added_at),
            ArtistField::Index | ArtistField::Cover => Ordering::Equal,
        }
    }

    fn group(artist: &SavedArtist, field: ArtistField) -> Option<String> {
        match field {
            ArtistField::Name => Some(initial(&artist.name)),
            _ => None,
        }
    }

    fn axes(ui: &Ui, shelf: Shelf, sieve: &ArtistSieve) -> Vec<Filter> {
        match ui.store.library.shape(shelf).is_catalog() {
            true => vec![Filter::Flag(FlagAxis {
                key: "filter-favorites",
                label: t!("filter-favorites"),
                on: sieve.starred,
            })],
            false => Vec::new(),
        }
    }

    fn filter(sieve: &mut ArtistSieve, change: FilterChange) -> bool {
        match change {
            FilterChange::Flag("filter-favorites", on) => sieve.starred = on,
            FilterChange::Reset => *sieve = ArtistSieve::default(),
            _ => return false,
        }
        true
    }

    fn narrowed(sieve: &ArtistSieve) -> bool {
        sieve.starred
    }

    fn cell(
        ui: &Ui,
        _: Shelf,
        artist: &SavedArtist,
        cell: &TableCell<ArtistField>,
        frame: El,
    ) -> El {
        let theme = &ui.theme;
        match cell.field {
            ArtistField::Index => index_cell(ui, origin(artist), cell.row, frame),
            ArtistField::Cover => {
                cells::avatar(frame, ui.artwork(), artist.cover.as_deref(), theme)
            }
            ArtistField::Name => {
                cells::dim(frame, "name", artist.name.clone(), theme.foreground, theme)
            }
            ArtistField::AddedAt => cells::dim(
                frame,
                "added-at",
                cells::stamp(artist.added_at),
                theme.muted_foreground,
                theme,
            ),
        }
    }

    fn card(ui: &Ui, _: Shelf, artist: &SavedArtist, key: String, width: f32) -> Card {
        let opener = super::opener();
        cards::artist_card(ui, key, artist, Fit::Tile(width), Some(&opener))
    }

    fn round() -> bool {
        true
    }

    fn origin(artist: &SavedArtist) -> Origin {
        origin(artist)
    }

    fn open(artist: &SavedArtist) -> Destination {
        Destination::Artist(artist.id.as_str().into())
    }

    fn menu(artist: &SavedArtist) -> Target {
        Target::artist(artist.clone())
    }

    fn pin(artist: &SavedArtist) -> Option<Pin> {
        artist.pin()
    }

    /// Nothing to unfollow where following is not a thing the provider has.
    fn remove(ui: &mut Ui, shelf: Shelf, artists: Vec<SavedArtist>) {
        if !ui.store.session.capabilities_of(shelf).follow_artists {
            return;
        }
        crate::shared::confirm::artists(ui, artists);
    }
}

/// Where playback names an artist's queue as coming from.
fn origin(artist: &SavedArtist) -> Origin {
    Origin::artist(artist.id.clone()).named(artist.name.clone())
}

/// The artists there is downloaded music of, on a downloaded album or playlist or in a
/// downloaded song of the shelf. This is what offline mode keeps the list to.
fn heard(ui: &Ui, shelf: Shelf) -> Rc<HashSet<String>> {
    let tracks = ui.store.library.state(shelf).tracks();
    let downloads = &ui.store.downloads;
    let stamp = (tracks.len(), downloads.len());
    let mut memo = Artists::list(ui, shelf).memo.borrow_mut();
    if let Some(kept) = memo.as_ref()
        && kept.stamp == stamp
    {
        return kept.artists.clone();
    }
    let kept = tracks
        .iter()
        .filter(|track| track.id.as_deref().is_some_and(|id| downloads.has(id)))
        .flat_map(|track| &track.artist_refs)
        .filter_map(|artist| artist.id.as_deref());
    let artists: Rc<HashSet<String>> = Rc::new(
        downloads
            .credited()
            .chain(kept)
            .map(str::to_owned)
            .collect(),
    );
    *memo = Some(Heard {
        stamp,
        artists: artists.clone(),
    });
    artists
}
