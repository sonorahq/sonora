use std::cmp::Ordering;
use ui::{ActiveTheme as _, Filter, FilterChange, FlagAxis};

use gpui::{AnyElement, App, Entity, TextAlign};
use i18n::t;
use music::Playlist;
use router::Destination;
use state::{Library, LibraryPart, LibraryState, Origin, Playback};
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
    local: bool,
    owned: bool,
}

impl PlaylistSource {
    pub(super) fn shelved(
        library: Entity<Library>,
        playback: Entity<Playback>,
        local: bool,
    ) -> Self {
        Self {
            library,
            playback,
            local,
            owned: false,
        }
    }

    fn index_cell(&self, cell: &Cell<PlaylistField>, playlist: &Playlist, cx: &App) -> AnyElement {
        let origin = Origin::playlist(playlist.id.clone()).named(playlist.name.clone());
        let state = self.playback.read(cx).playing_from(&origin);
        let played = origin.clone();
        let press = cells::toggle(&self.playback, state.clone(), move |playback, cx| {
            playback.play_origin(played.clone(), cx)
        });

        cells::index(cell, state, true, None, press, cx)
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<Playlist> {
        self.playlists(cx).get(row).cloned()
    }

    fn playlists<'a>(&self, cx: &'a App) -> &'a [Playlist] {
        let library = self.library.read(cx);
        let state = match self.local {
            true => library.local_state(),
            false => library.state(),
        };
        match state {
            LibraryState::Ready { playlists, .. } => playlists.as_slice(),
            _ => &[],
        }
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
                && (holds(&playlist.name, query) || holds(&playlist.owner, query))
        })
    }

    fn filter_axes(&self, _query: &str, _cx: &App) -> Vec<Filter> {
        vec![Filter::Flag(FlagAxis {
            key: "filter-owned",
            label: t!("filter-owned"),
            on: self.owned,
        })]
    }

    fn filter(&mut self, change: FilterChange, _cx: &App) -> bool {
        match change {
            FilterChange::Flag("filter-owned", value) => {
                self.owned = value;
                true
            }
            FilterChange::Reset => {
                self.owned = false;
                true
            }
            _ => false,
        }
    }

    fn filtered(&self, _cx: &App) -> bool {
        self.owned
    }

    fn playing(&self, row: usize, cx: &App) -> bool {
        self.playlists(cx).get(row).is_some_and(|playlist| {
            let origin = Origin::playlist(playlist.id.clone());
            self.playback.read(cx).playing_from(&origin).is_some()
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        match self.local {
            true => self.library.read(cx).local_loading(LibraryPart::Playlists),
            false => self.library.read(cx).loading(LibraryPart::Playlists),
        }
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
            false,
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
            PlaylistField::TrackCount => {
                cells::dim(&cell, format!("{}", playlist.track_count), muted)
            }
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
