use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{AnyElement, App, Entity, TextAlign};
use i18n::t;
use music::Playlist;
use router::Destination;
use state::{Library, LibraryPart, Origin, Playback, PlaylistRow, Shelf};
use ui::rank::{ESSENTIAL, HANDY, NICE, SPARE};
use ui::{
    ActiveTheme as _, Branch, Cell, ColumnSpec, Filter, FilterChange, FlagAxis, Menu, Pin,
    TableSource, Width,
};

use crate::shared::cells::{self, DATE, NUMBER, TRAILING, Tap};
use crate::shared::menus::playlist_menu;
use crate::shared::pins::Pinned as _;
use crate::shared::text::{folded, holds};

const FOLDER: &str = "icons/folder.svg";
const FOLDER_OPEN: &str = "icons/folder-open.svg";

type Fold = Rc<dyn Fn(&str, &mut App)>;

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

pub(super) struct FolderRow {
    pub id: String,
    pub name: String,
    pub depth: usize,
    pub count: usize,
    pub open: bool,
}

pub(super) struct PlaylistSource {
    library: Entity<Library>,
    playback: Entity<Playback>,
    shelf: Shelf,
    owned: bool,
    open: HashSet<String>,
    fold: Option<Fold>,
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
            open: HashSet::new(),
            fold: None,
        }
    }

    pub(super) fn folded(mut self, fold: impl Fn(&str, &mut App) + 'static) -> Self {
        self.fold = Some(Rc::new(fold));
        self
    }

    pub(super) fn toggle(&mut self, id: &str) {
        if !self.open.remove(id) {
            self.open.insert(id.to_owned());
        }
    }

    fn outline<'a>(&self, cx: &'a App) -> &'a [PlaylistRow] {
        self.library.read(cx).state(self.shelf).outline()
    }

    fn playlists<'a>(&self, cx: &'a App) -> &'a [Playlist] {
        self.library.read(cx).state(self.shelf).playlists()
    }

    fn playlist<'a>(&self, row: usize, cx: &'a App) -> Option<&'a Playlist> {
        match self.outline(cx).get(row)? {
            PlaylistRow::Playlist { index, .. } => self.playlists(cx).get(*index),
            PlaylistRow::Folder { .. } => None,
        }
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<Playlist> {
        self.playlist(row, cx).cloned()
    }

    pub(super) fn folder_at(&self, row: usize, cx: &App) -> Option<FolderRow> {
        match self.outline(cx).get(row)? {
            PlaylistRow::Folder {
                id,
                name,
                depth,
                count,
            } => Some(FolderRow {
                id: id.clone(),
                name: name.clone(),
                depth: *depth,
                count: *count,
                open: self.open.contains(id),
            }),
            PlaylistRow::Playlist { .. } => None,
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

    fn folder_cell(&self, cell: Cell<PlaylistField>, folder: FolderRow, cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        match cell.field {
            PlaylistField::Index => {
                let fold = self.fold.clone();
                let id = folder.id;
                let press: Tap = Box::new(move |cx| {
                    if let Some(fold) = &fold {
                        fold(&id, cx);
                    }
                });
                cells::disclosure(&cell, folder.open, press, cx)
            }
            PlaylistField::Cover => {
                let icon = match folder.open {
                    true => FOLDER_OPEN,
                    false => FOLDER,
                };
                cells::symbol(&cell, icon, muted, cx)
            }
            PlaylistField::Name => {
                cells::nested(&cell, folder.depth, folder.name, theme.foreground, cx)
            }
            PlaylistField::Owner => {
                cells::dim(&cell, t!("count-playlists", count = folder.count), muted)
            }
            PlaylistField::TrackCount | PlaylistField::Modified => cells::blank(&cell),
        }
    }
}

impl TableSource for PlaylistSource {
    type Field = PlaylistField;

    fn columns(&self) -> &'static [ColumnSpec<PlaylistField>] {
        COLUMNS
    }

    fn rows(&self, cx: &App) -> usize {
        self.outline(cx).len()
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        match self.outline(cx).get(row) {
            Some(PlaylistRow::Folder { name, .. }) => !self.owned && holds(name, query),
            Some(PlaylistRow::Playlist { index, .. }) => {
                self.playlists(cx).get(*index).is_some_and(|playlist| {
                    (!self.owned || playlist.owned)
                        && (holds(&playlist.name, query) || holds(&playlist.owner, query))
                })
            }
            None => false,
        }
    }

    fn parent(&self, row: usize, cx: &App) -> Option<usize> {
        let outline = self.outline(cx);
        let depth = outline.get(row)?.depth();
        if depth == 0 {
            return None;
        }
        (0..row).rev().find(|&above| outline[above].depth() < depth)
    }

    fn branch(&self, row: usize, cx: &App) -> Option<Branch> {
        let folder = self.folder_at(row, cx)?;
        Some(Branch {
            label: folder.name.into(),
            open: folder.open,
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
        self.playlist(row, cx)?.pin()
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

        if let Some(folder) = self.folder_at(cell.row, cx) {
            return self.folder_cell(cell, folder, cx);
        }
        let depth = self.outline(cx).get(cell.row).map(PlaylistRow::depth);
        let Some((playlist, depth)) = self.playlist(cell.row, cx).zip(depth) else {
            return cells::blank(&cell);
        };

        if cell.field == PlaylistField::Index {
            return self.index_cell(&cell, playlist, cx);
        }

        match cell.field {
            PlaylistField::Cover => cells::artwork(&cell, playlist.cover.clone()),
            PlaylistField::Name => {
                cells::nested(&cell, depth, playlist.name.clone(), theme.foreground, cx)
            }
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
        let outline = self.outline(cx);
        let (x, y) = (outline.get(a), outline.get(b));
        let leaf = |row: Option<&PlaylistRow>| matches!(row, Some(PlaylistRow::Playlist { .. }));

        leaf(x).cmp(&leaf(y)).then_with(|| match (x, y) {
            (
                Some(PlaylistRow::Folder { name: p, .. }),
                Some(PlaylistRow::Folder { name: q, .. }),
            ) => match field {
                PlaylistField::Index | PlaylistField::Cover => a.cmp(&b),
                _ => folded(p, q),
            },
            (
                Some(PlaylistRow::Playlist { index: i, .. }),
                Some(PlaylistRow::Playlist { index: j, .. }),
            ) => compare_playlists(self.playlists(cx), field, *i, *j),
            _ => a.cmp(&b),
        })
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
