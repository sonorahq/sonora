use std::cmp::Ordering;

use gpui::{AnyElement, App, Entity, TextAlign};
use i18n::t;
use music::Playlist;
use router::Destination;
use state::{FolderRow, Library, LibraryPart, Origin, Outline, Playback, Shelf};
use ui::rank::{ESSENTIAL, HANDY, NICE, SPARE};
use ui::{
    ActiveTheme as _, Cell, ColumnSpec, Filter, FilterChange, FlagAxis, Menu, Pin, TableSource,
    Width,
};

use crate::shared::cards::{folder_pin, holding};
use crate::shared::cells::{self, DATE, NUMBER, TRAILING};
use crate::shared::menus::{folder_menu, playlist_menu};
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};

const FOLDER: &str = "icons/folder.svg";

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
    /// The folder whose contents are listed, or the root of the shelf.
    folder: Option<String>,
    owned: bool,
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
            folder: None,
            owned: false,
        }
    }

    fn outline<'a>(&self, cx: &'a App) -> &'a Outline {
        self.library.read(cx).state(self.shelf).outline()
    }

    fn playlists<'a>(&self, cx: &'a App) -> &'a [Playlist] {
        self.library.read(cx).state(self.shelf).playlists()
    }

    /// Where a listed row sits in the outline.
    fn spot(&self, row: usize, cx: &App) -> Option<usize> {
        self.outline(cx)
            .level(self.folder.as_deref())
            .get(row)
            .copied()
    }

    fn playlist<'a>(&self, row: usize, cx: &'a App) -> Option<&'a Playlist> {
        let index = self.outline(cx).playlist_at(self.spot(row, cx)?)?;
        self.playlists(cx).get(index)
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<Playlist> {
        self.playlist(row, cx).cloned()
    }

    pub(super) fn folder_at(&self, row: usize, cx: &App) -> Option<FolderRow> {
        self.outline(cx).folder_at(self.spot(row, cx)?)
    }

    /// Every playlist inside a folder, however deep. Sorting and filtering a folder row both
    /// answer for what it holds rather than for the folder itself.
    fn held<'a>(&self, folder: &str, cx: &'a App) -> Vec<&'a Playlist> {
        let playlists = self.playlists(cx);
        self.outline(cx)
            .playlists_in(folder)
            .into_iter()
            .filter_map(|index| playlists.get(index))
            .collect()
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

    /// A folder reads as a row of its own: a glyph where the cover goes, what it holds where the
    /// owner goes, and nothing in the columns that only a playlist can answer.
    fn folder_cell(&self, cell: Cell<PlaylistField>, folder: FolderRow, cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        match cell.field {
            PlaylistField::Cover => cells::symbol(&cell, FOLDER, muted, cx),
            PlaylistField::Name => cells::dim(&cell, folder.name, theme.foreground),
            PlaylistField::Owner => cells::dim(&cell, holding(&folder), muted),
            PlaylistField::Index | PlaylistField::TrackCount | PlaylistField::Modified => {
                cells::blank(&cell)
            }
        }
    }
}

impl TableSource for PlaylistSource {
    type Field = PlaylistField;

    fn columns(&self) -> &'static [ColumnSpec<PlaylistField>] {
        COLUMNS
    }

    fn rows(&self, cx: &App) -> usize {
        self.outline(cx).level(self.folder.as_deref()).len()
    }

    /// A folder answers for itself and for everything it holds, so a search never hides a match
    /// behind a folder the query does not name.
    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        if let Some(folder) = self.folder_at(row, cx) {
            return !self.owned
                && (holds(&folder.name, query)
                    || self
                        .held(&folder.id, cx)
                        .iter()
                        .any(|playlist| holds(&playlist.name, query)));
        }
        self.playlist(row, cx).is_some_and(|playlist| {
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
        self.playlist(row, cx).is_some_and(|playlist| {
            let origin = Origin::playlist(playlist.id.clone());
            self.playback.read(cx).playing_from(&origin).is_some()
        })
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.library
            .read(cx)
            .loading(self.shelf, LibraryPart::Playlists)
    }

    fn pin(&self, row: usize, cx: &App) -> Option<Pin> {
        match self.folder_at(row, cx) {
            Some(folder) => {
                let cover = self.library.read(cx).folder_cover(&folder.id);
                Some(folder_pin(&folder, cover))
            }
            None => self.playlist(row, cx)?.pin(),
        }
    }

    fn picking(&self) -> bool {
        true
    }

    fn context_menu(&self, rows: &[usize], _visible: &[PlaylistField], cx: &App) -> Option<Menu> {
        let row = *rows.first()?;
        if let Some(folder) = self.folder_at(row, cx) {
            return Some(folder_menu(&folder.id, &folder.name, cx));
        }
        Some(playlist_menu(
            self.at(row, cx)?,
            self.playback.clone(),
            false,
            cx,
        ))
    }

    fn cell(&self, cell: Cell<PlaylistField>, cx: &mut App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        if let Some(folder) = self.folder_at(cell.row, cx) {
            return self.folder_cell(cell, folder, cx);
        }
        let Some(playlist) = self.playlist(cell.row, cx) else {
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

    /// Folders lead the page, whatever the column is sorted on.
    fn leads(&self, row: usize, cx: &App) -> bool {
        self.folder_at(row, cx).is_some()
    }

    fn compare(&self, field: PlaylistField, a: usize, b: usize, cx: &App) -> Ordering {
        match (self.folder_at(a, cx), self.folder_at(b, cx)) {
            (Some(p), Some(q)) => self
                .compare_folders(field, &p, &q, cx)
                .then(folded(&p.name, &q.name)),
            (None, None) => {
                let (Some(i), Some(j)) = (self.index_of(a, cx), self.index_of(b, cx)) else {
                    return a.cmp(&b);
                };
                compare_playlists(self.playlists(cx), field, i, j)
            }
            // `leads` has already put the folders ahead; this only keeps the order settled.
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
        }
    }
}

impl PlaylistSource {
    fn index_of(&self, row: usize, cx: &App) -> Option<usize> {
        self.outline(cx).playlist_at(self.spot(row, cx)?)
    }

    /// A folder has none of a playlist's fields, so it borrows them from what it holds: the newest
    /// change inside it, the tracks it adds up to, the playlists it counts.
    fn compare_folders(
        &self,
        field: PlaylistField,
        a: &FolderRow,
        b: &FolderRow,
        cx: &App,
    ) -> Ordering {
        match field {
            PlaylistField::Name | PlaylistField::Index | PlaylistField::Cover => Ordering::Equal,
            PlaylistField::Owner => a.playlists.cmp(&b.playlists),
            PlaylistField::TrackCount => {
                let tracks = |folder: &FolderRow| {
                    self.held(&folder.id, cx)
                        .iter()
                        .map(|playlist| playlist.track_count)
                        .sum::<u32>()
                };
                tracks(a).cmp(&tracks(b))
            }
            PlaylistField::Modified => {
                let touched = |folder: &FolderRow| {
                    self.held(&folder.id, cx)
                        .iter()
                        .filter_map(|playlist| playlist.modified_at)
                        .max()
                };
                touched(a).cmp(&touched(b))
            }
        }
    }
}

fn compare_playlists(playlists: &[Playlist], field: PlaylistField, a: usize, b: usize) -> Ordering {
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
