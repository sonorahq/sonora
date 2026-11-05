use std::cmp::Ordering;
use ui::{ActiveTheme as _, Filter, FilterChange, FlagAxis};

use gpui::{AnyElement, App, Entity, TextAlign};
use i18n::t;
use music::Playlist;
use router::Destination;
use state::{Collection, Library, LibraryPart, Network, Origin, Playback, Shelf, Sonora};
use ui::rank::{ESSENTIAL, HANDY, NICE, SPARE};
use ui::{Cell, ColumnSpec, Menu, Pin, TableSource, Width};

use crate::shared::cells::{self, DATE, NUMBER, TRAILING};
use crate::shared::menus::playlist_menu;
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PlaylistField {
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

pub(super) const COLUMNS: &[ColumnSpec<PlaylistField>] =
    &[INDEX, COVER, NAME, OWNER, MODIFIED, TRACK_COUNT];

pub(super) struct PlaylistSource {
    library: Entity<Library>,
    playback: Entity<Playback>,
    shelf: Shelf,
    owned: bool,
    /// Keep only playlists that were downloaded whole.
    downloaded: bool,
}

impl PlaylistSource {
    pub(super) fn shelved(
        library: Entity<Library>,
        playback: Entity<Playback>,
        shelf: Shelf,
    ) -> Self {
        Self {
            library,
            playback,
            shelf,
            owned: false,
            downloaded: false,
        }
    }

    fn index_cell(&self, cell: &Cell<PlaylistField>, playlist: &Playlist, cx: &App) -> AnyElement {
        let origin = Origin::playlist(playlist.id.clone()).named(playlist.name.clone());
        let playing = self.playback.read(cx).playing_from(&origin, cx);
        let played = origin.clone();
        let press = cells::toggle(&self.playback, playing, move |playback, cx| {
            playback.play_origin(played.clone(), cx)
        });

        cells::index(cell, playing, true, None, None, press, cx)
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<Playlist> {
        self.playlists(cx).get(row).cloned()
    }

    fn playlists<'a>(&self, cx: &'a App) -> &'a [Playlist] {
        self.library.read(cx).state(self.shelf).playlists()
    }
}

impl TableSource for PlaylistSource {
    type Field = PlaylistField;

    fn columns(&self) -> &'static [ColumnSpec<PlaylistField>] {
        COLUMNS
    }

    fn rows(&self, cx: &App) -> usize {
        self.playlists(cx).len()
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        self.playlists(cx).get(row).is_some_and(|playlist| {
            (!self.owned || playlist.owned)
                && (!(self.downloaded || Network::downloads_only(self.shelf, cx))
                    || downloaded(&playlist.id, cx))
                && (holds(&playlist.name, query) || holds(&playlist.owner, query))
        })
    }

    fn filter_axes(&self, cx: &App) -> Vec<Filter> {
        let mut axes = vec![Filter::Flag(FlagAxis {
            key: "filter-owned",
            label: t!("filter-owned"),
            on: self.owned,
        })];
        // Offline mode already keeps the shelf to its downloads, so the filter would do nothing.
        let offered = self.downloaded || !Sonora::global(cx).downloads.read(cx).is_empty();
        if offered && !Network::downloads_only(self.shelf, cx) {
            axes.push(Filter::Flag(FlagAxis {
                key: "filter-downloaded",
                label: t!("filter-downloaded"),
                on: self.downloaded,
            }));
        }
        axes
    }

    fn filter(&mut self, change: FilterChange, _cx: &App) -> bool {
        match change {
            FilterChange::Flag("filter-owned", value) => {
                self.owned = value;
                true
            }
            FilterChange::Flag("filter-downloaded", value) => {
                self.downloaded = value;
                true
            }
            FilterChange::Reset => {
                self.owned = false;
                self.downloaded = false;
                true
            }
            _ => false,
        }
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.owned || self.downloaded
    }

    fn playing(&self, row: usize, cx: &App) -> bool {
        self.playlists(cx).get(row).is_some_and(|playlist| {
            let origin = Origin::playlist(playlist.id.clone());
            self.playback.read(cx).playing_from(&origin, cx).is_some()
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.library
            .read(cx)
            .waiting(self.shelf, LibraryPart::Playlists, cx)
    }

    fn pin(&self, row: usize, cx: &App) -> Option<Pin> {
        self.playlists(cx).get(row)?.pin()
    }

    fn picking(&self) -> bool {
        true
    }

    fn context_menu(&self, rows: &[usize], _visible: &[PlaylistField], cx: &App) -> Option<Menu> {
        Some(playlist_menu(
            self.at(*rows.first()?, cx)?,
            self.playback.clone(),
            cx,
        ))
    }

    fn cell(&self, cell: Cell<PlaylistField>, cx: &mut App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        let Some(playlist) = self.playlists(cx).get(cell.row) else {
            return cells::blank(&cell);
        };

        if cell.field == PlaylistField::Index {
            return self.index_cell(&cell, playlist, cx);
        }

        match cell.field {
            PlaylistField::Cover => cells::artwork(&cell, playlist.cover.clone()),
            PlaylistField::Name => cells::dim(&cell, playlist.name.clone(), theme.foreground),
            PlaylistField::Owner => match playlist.owner_id.is_empty() {
                true => cells::dim(&cell, playlist.owner.clone(), muted),
                false => cells::link(
                    &cell,
                    "playlist-owner",
                    playlist.owner.clone(),
                    muted,
                    Destination::User(playlist.owner_id.clone().into()),
                ),
            },
            PlaylistField::TrackCount => match playlist.track_count == 0 && !playlist.owned {
                true => cells::blank(&cell),
                false => cells::dim(&cell, format!("{}", playlist.track_count), muted),
            },
            PlaylistField::Modified => cells::dim(&cell, cells::stamp(playlist.modified_at), muted),
            PlaylistField::Index => cells::blank(&cell),
        }
    }

    fn compare(&self, field: PlaylistField, a: usize, b: usize, cx: &App) -> Ordering {
        let playlists = self.playlists(cx);
        let text = |index: usize, pick: fn(&Playlist) -> &str| {
            playlists.get(index).map(pick).unwrap_or_default()
        };

        match field {
            PlaylistField::Name => folded(
                text(a, |playlist| &playlist.name),
                text(b, |playlist| &playlist.name),
            ),
            PlaylistField::Owner => folded(
                text(a, |playlist| &playlist.owner),
                text(b, |playlist| &playlist.owner),
            ),
            PlaylistField::TrackCount => playlists
                .get(a)
                .map(|playlist| playlist.track_count)
                .cmp(&playlists.get(b).map(|playlist| playlist.track_count)),
            PlaylistField::Modified => playlists
                .get(a)
                .map(|playlist| playlist.modified_at)
                .cmp(&playlists.get(b).map(|playlist| playlist.modified_at)),
            PlaylistField::Index | PlaylistField::Cover => a.cmp(&b),
        }
    }
}

/// Whether a playlist was downloaded whole, or is on its way.
fn downloaded(id: &str, cx: &App) -> bool {
    Sonora::global(cx)
        .downloads
        .read(cx)
        .set(Collection::Playlist, id)
        .is_some()
}
