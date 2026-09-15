use std::collections::{HashMap, HashSet};

use music::{Playlist, PlaylistEntry};

/// One row of a shelf's playlist outline: a folder, or a playlist together with where it sits in
/// the shelf's flat playlist list. A row in the root carries no parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistRow {
    Folder {
        id: String,
        name: String,
        parent: Option<String>,
    },
    Playlist {
        id: String,
        index: usize,
        parent: Option<String>,
    },
}

impl PlaylistRow {
    pub fn id(&self) -> &str {
        match self {
            Self::Folder { id, .. } | Self::Playlist { id, .. } => id,
        }
    }

    pub fn parent(&self) -> Option<&str> {
        match self {
            Self::Folder { parent, .. } | Self::Playlist { parent, .. } => parent.as_deref(),
        }
    }
}

/// A folder as a page lists it: its name and how much sits directly inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRow {
    pub id: String,
    pub name: String,
    /// Playlists directly inside, not counting those in a folder further down.
    pub playlists: usize,
    /// Folders directly inside.
    pub folders: usize,
}

/// The shape of a shelf's playlists: which folder holds what, in the order the provider listed
/// them. Levels are worked out once, so a page that shows one folder never walks the whole tree.
#[derive(Default)]
pub struct Outline {
    rows: Vec<PlaylistRow>,
    root: Vec<usize>,
    inside: HashMap<String, Vec<usize>>,
    at: HashMap<String, usize>,
}

impl Outline {
    pub fn rows(&self) -> &[PlaylistRow] {
        &self.rows
    }

    pub fn row(&self, at: usize) -> Option<&PlaylistRow> {
        self.rows.get(at)
    }

    /// The rows directly inside `folder`, or the root ones for `None`. Unknown folders are empty,
    /// which is what a page opened on a folder that has since gone shows.
    pub fn level(&self, folder: Option<&str>) -> &[usize] {
        match folder {
            None => &self.root,
            Some(id) => self.inside.get(id).map_or(&[][..], Vec::as_slice),
        }
    }

    /// Whether any folder exists at all. A shelf without folders lists as it always did.
    pub fn folded(&self) -> bool {
        !self.inside.is_empty()
    }

    pub fn folder(&self, id: &str) -> Option<FolderRow> {
        self.folder_at(*self.at.get(id)?)
    }

    /// The folder at `at`, when the row there is one.
    pub fn folder_at(&self, at: usize) -> Option<FolderRow> {
        match self.rows.get(at)? {
            PlaylistRow::Folder { id, name, .. } => Some(self.counted(id, name)),
            PlaylistRow::Playlist { .. } => None,
        }
    }

    /// The playlist index at `at`, when the row there is one.
    pub fn playlist_at(&self, at: usize) -> Option<usize> {
        match self.rows.get(at)? {
            PlaylistRow::Playlist { index, .. } => Some(*index),
            PlaylistRow::Folder { .. } => None,
        }
    }

    /// The folders leading to `id`, outermost first, the folder itself last. Empty when no folder
    /// answers to `id`.
    pub fn path(&self, id: &str) -> Vec<FolderRow> {
        let mut path = Vec::new();
        let mut step = Some(id.to_owned());
        while let Some(id) = step {
            let Some(folder) = self.folder(&id) else {
                break;
            };
            step = self.parent_of(&id).map(str::to_owned);
            path.push(folder);
        }
        path.reverse();
        path
    }

    /// Every playlist inside `folder`, however deep, as indices into the shelf's playlists.
    pub fn playlists_in(&self, folder: &str) -> Vec<usize> {
        let mut found = Vec::new();
        self.gather(folder, &mut found);
        found
    }

    fn gather(&self, folder: &str, found: &mut Vec<usize>) {
        for &at in self.level(Some(folder)) {
            match self.rows.get(at) {
                Some(PlaylistRow::Playlist { index, .. }) => found.push(*index),
                Some(PlaylistRow::Folder { id, .. }) => self.gather(id, found),
                None => {}
            }
        }
    }

    fn parent_of(&self, id: &str) -> Option<&str> {
        self.rows
            .get(*self.at.get(id)?)
            .and_then(PlaylistRow::parent)
    }

    fn counted(&self, id: &str, name: &str) -> FolderRow {
        let level = self.level(Some(id));
        let folders = level
            .iter()
            .filter(|&&at| matches!(self.rows.get(at), Some(PlaylistRow::Folder { .. })))
            .count();

        FolderRow {
            id: id.to_owned(),
            name: name.to_owned(),
            playlists: level.len() - folders,
            folders,
        }
    }

    fn index(&mut self) {
        self.root.clear();
        self.inside.clear();
        self.at.clear();
        for at in 0..self.rows.len() {
            self.at.insert(self.rows[at].id().to_owned(), at);
            if let PlaylistRow::Folder { id, .. } = &self.rows[at] {
                self.inside.entry(id.clone()).or_default();
            }
            match self.rows[at].parent() {
                None => self.root.push(at),
                Some(parent) => self.inside.entry(parent.to_owned()).or_default().push(at),
            }
        }
    }
}

/// Splits what a provider listed into the flat playlists every page reads and the outline that
/// says how they are grouped.
pub fn split(entries: Vec<PlaylistEntry>) -> (Vec<Playlist>, Outline) {
    let mut playlists = Vec::new();
    let mut outline = Outline::default();
    flatten(entries, None, &mut playlists, &mut outline.rows);
    outline.index();
    (playlists, outline)
}

fn flatten(
    entries: Vec<PlaylistEntry>,
    parent: Option<&str>,
    playlists: &mut Vec<Playlist>,
    rows: &mut Vec<PlaylistRow>,
) {
    for entry in entries {
        match entry {
            PlaylistEntry::Playlist(playlist) => {
                rows.push(PlaylistRow::Playlist {
                    id: playlist.id.clone(),
                    index: playlists.len(),
                    parent: parent.map(str::to_owned),
                });
                playlists.push(playlist);
            }
            PlaylistEntry::Folder(folder) => {
                rows.push(PlaylistRow::Folder {
                    id: folder.id.clone(),
                    name: folder.name,
                    parent: parent.map(str::to_owned),
                });
                flatten(folder.entries, Some(&folder.id), playlists, rows);
            }
        }
    }
}

/// Puts the outline back in step with a playlist list that changed under it: a playlist that left
/// drops out, one that arrived joins the root, and the rest keep the folder they were in.
pub fn relist(playlists: &[Playlist], outline: &mut Outline) {
    let indices: HashMap<&str, usize> = playlists
        .iter()
        .enumerate()
        .map(|(index, playlist)| (playlist.id.as_str(), index))
        .collect();

    let mut listed = HashSet::new();
    outline.rows.retain_mut(|row| match row {
        PlaylistRow::Folder { .. } => true,
        PlaylistRow::Playlist { id, index, .. } => match indices.get(id.as_str()) {
            Some(found) => {
                *index = *found;
                listed.insert(*found);
                true
            }
            None => false,
        },
    });

    outline.rows.extend(
        playlists
            .iter()
            .enumerate()
            .filter(|(index, _)| !listed.contains(index))
            .map(|(index, playlist)| PlaylistRow::Playlist {
                id: playlist.id.clone(),
                index,
                parent: None,
            }),
    );
    outline.index();
}
