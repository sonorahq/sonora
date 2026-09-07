use std::collections::{HashMap, HashSet};

use music::{Playlist, PlaylistEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistRow {
    Folder {
        id: String,
        name: String,
        depth: usize,
        count: usize,
    },
    Playlist {
        id: String,
        index: usize,
        depth: usize,
    },
}

impl PlaylistRow {
    pub fn depth(&self) -> usize {
        match self {
            Self::Folder { depth, .. } | Self::Playlist { depth, .. } => *depth,
        }
    }
}

pub fn split(entries: Vec<PlaylistEntry>) -> (Vec<Playlist>, Vec<PlaylistRow>) {
    let mut playlists = Vec::new();
    let mut rows = Vec::new();
    flatten(entries, 0, &mut playlists, &mut rows);
    recount(&mut rows);
    (playlists, rows)
}

fn flatten(
    entries: Vec<PlaylistEntry>,
    depth: usize,
    playlists: &mut Vec<Playlist>,
    rows: &mut Vec<PlaylistRow>,
) {
    for entry in entries {
        match entry {
            PlaylistEntry::Playlist(playlist) => {
                rows.push(PlaylistRow::Playlist {
                    id: playlist.id.clone(),
                    index: playlists.len(),
                    depth,
                });
                playlists.push(playlist);
            }
            PlaylistEntry::Folder(folder) => {
                rows.push(PlaylistRow::Folder {
                    id: folder.id,
                    name: folder.name,
                    depth,
                    count: 0,
                });
                flatten(folder.entries, depth + 1, playlists, rows);
            }
        }
    }
}

pub fn relist(playlists: &[Playlist], rows: &mut Vec<PlaylistRow>) {
    let indices: HashMap<&str, usize> = playlists
        .iter()
        .enumerate()
        .map(|(index, playlist)| (playlist.id.as_str(), index))
        .collect();

    rows.retain_mut(|row| match row {
        PlaylistRow::Folder { .. } => true,
        PlaylistRow::Playlist { id, index, .. } => match indices.get(id.as_str()) {
            Some(found) => {
                *index = *found;
                true
            }
            None => false,
        },
    });

    let listed: HashSet<&str> = rows
        .iter()
        .filter_map(|row| match row {
            PlaylistRow::Playlist { id, .. } => Some(id.as_str()),
            PlaylistRow::Folder { .. } => None,
        })
        .collect();
    let unlisted: Vec<PlaylistRow> = playlists
        .iter()
        .enumerate()
        .filter(|(_, playlist)| !listed.contains(playlist.id.as_str()))
        .map(|(index, playlist)| PlaylistRow::Playlist {
            id: playlist.id.clone(),
            index,
            depth: 0,
        })
        .collect();
    rows.extend(unlisted);

    recount(rows);
}

fn recount(rows: &mut [PlaylistRow]) {
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut counts = vec![0; rows.len()];
    for (at, row) in rows.iter().enumerate() {
        let depth = row.depth();
        while open.last().is_some_and(|(_, opened)| *opened >= depth) {
            open.pop();
        }
        match row {
            PlaylistRow::Folder { .. } => open.push((at, depth)),
            PlaylistRow::Playlist { .. } => {
                for (folder, _) in &open {
                    counts[*folder] += 1;
                }
            }
        }
    }
    for (row, counted) in rows.iter_mut().zip(counts) {
        if let PlaylistRow::Folder { count, .. } = row {
            *count = counted;
        }
    }
}
