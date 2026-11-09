//! A shelf's playlists: their columns, search, filters (by you, downloaded) and sort, and how a
//! row and a card draw one.

use std::cmp::Ordering;
use std::hash::{DefaultHasher, Hash, Hasher};

use i18n::t;
use music::Playlist;
use router::Destination;
use rune::El;
use state::{Collection, Origin, Shelf};
use ui::rank::{ESSENTIAL, HANDY, NICE, SPARE};
use ui::{Card, ColumnSpec, Filter, FilterChange, FlagAxis, Pin, TableCell, TextAlign, Width};

use super::Section;
use super::items::{Items, Kind, index_cell};
use crate::Ui;
use crate::shared::cards::{self, Fit};
use crate::shared::cells::{self, DATE, NUMBER, TRAILING};
use crate::shared::menus::Target;
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};

/// What one column of the playlists table shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaylistField {
    Index,
    Cover,
    Name,
    Owner,
    TrackCount,
    Modified,
}

const COLUMN: ColumnSpec<PlaylistField> = ColumnSpec::filling(PlaylistField::Index);

const INDEX: ColumnSpec<PlaylistField> = ColumnSpec::numbering(PlaylistField::Index, NUMBER);

const COVER: ColumnSpec<PlaylistField> = ColumnSpec::artwork(PlaylistField::Cover);

const NAME: ColumnSpec<PlaylistField> = ColumnSpec {
    field: PlaylistField::Name,
    key: "name",
    header: "column-name",
    width: Width::Fill(0.55),
    rank: ESSENTIAL,
    ..COLUMN
};

const OWNER: ColumnSpec<PlaylistField> = ColumnSpec {
    field: PlaylistField::Owner,
    key: "owner",
    header: "column-owner",
    width: Width::Fill(0.45),
    rank: NICE,
    ..COLUMN
};

const TRACK_COUNT: ColumnSpec<PlaylistField> = ColumnSpec {
    field: PlaylistField::TrackCount,
    key: "tracks",
    header: "column-tracks",
    align: TextAlign::Right,
    width: Width::Fixed(TRAILING),
    rank: SPARE,
    ..COLUMN
};

const MODIFIED: ColumnSpec<PlaylistField> = ColumnSpec {
    field: PlaylistField::Modified,
    key: "modified",
    header: "column-modified",
    width: Width::Fixed(DATE),
    rank: HANDY,
    ..COLUMN
};

const COLUMNS: &[ColumnSpec<PlaylistField>] = &[INDEX, COVER, NAME, OWNER, MODIFIED, TRACK_COUNT];

/// What the playlists' filter popover narrows them by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PlaylistSieve {
    owned: bool,
    /// Keep only playlists that were downloaded whole.
    downloaded: bool,
}

/// A shelf's playlists.
pub(crate) struct Playlists;

impl Kind for Playlists {
    type Item = Playlist;
    type Field = PlaylistField;
    type Sieve = PlaylistSieve;
    type Memo = ();

    const SECTION: Section = Section::Playlists;
    const NAME: &'static str = "playlist";
    const COLUMNS: &'static [ColumnSpec<PlaylistField>] = COLUMNS;
    const FIRST: &'static str = "modified";
    const AXES: &'static [&'static str] = &["filter-owned", "filter-downloaded"];

    fn items(ui: &Ui, shelf: Shelf) -> &[Playlist] {
        ui.store.library.state(shelf).playlists()
    }

    fn list(ui: &Ui, shelf: Shelf) -> &Items<Self> {
        &ui.screens.library.shelf(shelf).playlists
    }

    fn list_mut(ui: &mut Ui, shelf: Shelf) -> &mut Items<Self> {
        &mut ui.screens.library.shelf_mut(shelf).playlists
    }

    fn id(playlist: &Playlist) -> &str {
        &playlist.id
    }

    fn name(playlist: &Playlist) -> &str {
        &playlist.name
    }

    fn hits(playlist: &Playlist, query: &str) -> bool {
        holds(&playlist.name, query) || holds(&playlist.owner, query)
    }

    fn keeps(ui: &Ui, shelf: Shelf, sieve: &PlaylistSieve, playlist: &Playlist) -> bool {
        (!sieve.owned || playlist.owned)
            && (!(sieve.downloaded || ui.store.network.downloads_only(shelf))
                || downloaded(ui, &playlist.id))
    }

    fn facts(ui: &Ui, shelf: Shelf, sieve: &PlaylistSieve, playlists: &[Playlist]) -> u64 {
        if !sieve.downloaded && !ui.store.network.downloads_only(shelf) {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        for playlist in playlists {
            downloaded(ui, &playlist.id).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn compare(a: &Playlist, b: &Playlist, field: PlaylistField) -> Ordering {
        match field {
            PlaylistField::Name => folded(&a.name, &b.name),
            PlaylistField::Owner => folded(&a.owner, &b.owner),
            PlaylistField::TrackCount => a.track_count.cmp(&b.track_count),
            PlaylistField::Modified => a.modified_at.cmp(&b.modified_at),
            PlaylistField::Index | PlaylistField::Cover => Ordering::Equal,
        }
    }

    fn group(_: &Playlist, _: PlaylistField) -> Option<String> {
        None
    }

    fn axes(ui: &Ui, shelf: Shelf, sieve: &PlaylistSieve) -> Vec<Filter> {
        let mut axes = vec![Filter::Flag(FlagAxis {
            key: "filter-owned",
            label: t!("filter-owned"),
            on: sieve.owned,
        })];
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

    fn filter(sieve: &mut PlaylistSieve, change: FilterChange) -> bool {
        match change {
            FilterChange::Flag("filter-owned", on) => sieve.owned = on,
            FilterChange::Flag("filter-downloaded", on) => sieve.downloaded = on,
            FilterChange::Reset => *sieve = PlaylistSieve::default(),
            _ => return false,
        }
        true
    }

    fn narrowed(sieve: &PlaylistSieve) -> bool {
        sieve.owned || sieve.downloaded
    }

    fn cell(
        ui: &Ui,
        _: Shelf,
        playlist: &Playlist,
        cell: &TableCell<PlaylistField>,
        frame: El,
    ) -> El {
        let theme = &ui.theme;
        let muted = theme.muted_foreground;
        match cell.field {
            PlaylistField::Index => index_cell(ui, origin(playlist), cell.row, frame),
            PlaylistField::Cover => {
                cells::artwork(frame, ui.artwork(), playlist.cover.as_deref(), theme)
            }
            PlaylistField::Name => cells::dim(
                frame,
                "name",
                playlist.name.clone(),
                theme.foreground,
                theme,
            ),
            PlaylistField::Owner => match playlist.owner_id.is_empty() {
                true => cells::dim(frame, "owner", playlist.owner.clone(), muted, theme),
                false => cells::link(
                    frame,
                    "playlist-owner",
                    playlist.owner.clone(),
                    muted,
                    Destination::User(playlist.owner_id.as_str().into()),
                    theme,
                ),
            },
            PlaylistField::TrackCount => match playlist.track_count == 0 && !playlist.owned {
                true => frame,
                false => cells::dim(
                    frame,
                    "tracks",
                    playlist.track_count.to_string(),
                    muted,
                    theme,
                ),
            },
            PlaylistField::Modified => cells::dim(
                frame,
                "modified",
                cells::stamp(playlist.modified_at),
                muted,
                theme,
            ),
        }
    }

    fn card(ui: &Ui, shelf: Shelf, playlist: &Playlist, key: String, width: f32) -> Card {
        let opener = super::opener();
        let build = match shelf.local() {
            true => cards::imported_playlist_card,
            false => cards::playlist_card,
        };
        build(ui, key, playlist, Fit::Tile(width), Some(&opener))
    }

    fn origin(playlist: &Playlist) -> Origin {
        origin(playlist)
    }

    fn open(playlist: &Playlist) -> Destination {
        Destination::Playlist(playlist.id.as_str().into())
    }

    fn menu(playlist: &Playlist) -> Target {
        Target::Playlist(playlist.clone())
    }

    fn pin(playlist: &Playlist) -> Option<Pin> {
        playlist.pin()
    }

    /// Takes the playlists the listener follows out of the library; their own stay, since
    /// deleting one is its own question.
    fn remove(ui: &mut Ui, _: Shelf, playlists: Vec<Playlist>) {
        let ids: Vec<String> = playlists
            .into_iter()
            .filter(|playlist| !playlist.owned)
            .map(|playlist| playlist.id)
            .collect();
        crate::shared::confirm::playlists(ui, ids);
    }
}

/// Where playback names a playlist's queue as coming from.
fn origin(playlist: &Playlist) -> Origin {
    Origin::playlist(playlist.id.clone()).named(playlist.name.clone())
}

/// Whether a playlist was downloaded whole, or is on its way.
fn downloaded(ui: &Ui, id: &str) -> bool {
    ui.store.downloads.set(Collection::Playlist, id).is_some()
}
