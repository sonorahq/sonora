use gpui::prelude::*;
use gpui::{App, ClickEvent, ClipboardItem, Context, Entity, SharedString, Window};
use i18n::t;
use music::{Album, GenreItem, MediaKind, Playlist, SavedArtist, Track};
use router::{Destination, navigate};
use state::{Addition, Detail, FolderRow, History, Library, Origin, Playback, Shelf, Sonora};
use ui::{Menu, MenuItem, MenuSearch, Pin, PinKind, Scrollbar, SubmenuState};

use crate::shared::confirm::Confirm;
use crate::shared::pins::Pinned as _;
use crate::shared::playlist_editor::{Edit, PlaylistEditor};
use crate::shared::tag_editor::TagEditor;

/// Something a context menu can open on, wherever it was clicked.
#[derive(Clone)]
pub(crate) enum Item {
    Album(Album),
    Playlist(Playlist),
    Folder(FolderRow),
    Artist(SavedArtist),
    Track(Track),
}

impl Item {
    /// The item behind a shelf card; a genre has no menu.
    pub(crate) fn of(item: &GenreItem) -> Option<Self> {
        match item {
            GenreItem::Album(album) => Some(Self::Album(album.clone())),
            GenreItem::Playlist(playlist) => Some(Self::Playlist(playlist.clone())),
            GenreItem::Artist(artist) => Some(Self::Artist(artist.clone())),
            GenreItem::Track(track) => Some(Self::Track(track.clone())),
            GenreItem::Genre(_) => None,
        }
    }

    /// The item's menu. A track's carries the playlist and artist submenus, whose state
    /// `menus` holds; the rest need only the playback.
    pub(crate) fn menu(
        &self,
        menus: &ItemMenu,
        playback: Entity<Playback>,
        opened_here: bool,
        cx: &App,
    ) -> Menu {
        match self {
            Self::Album(album) => album_menu(album.clone(), playback, menus, cx),
            Self::Playlist(playlist) => playlist_menu(playlist.clone(), playback, cx),
            Self::Folder(folder) => folder_menu(&folder.id, &folder.name, cx),
            Self::Artist(artist) => artist_menu(artist.clone(), playback, opened_here, cx),
            Self::Track(track) => menus.for_track(track, cx),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TrackColumns {
    pub album: bool,
    pub artists: bool,
}

#[derive(Clone)]
pub(crate) struct ItemMenu {
    playlist_submenu: SubmenuState,
    artist_submenu: SubmenuState,
    playlist_scrollbar: Entity<Scrollbar>,
    playlist_search: MenuSearch,
}

impl ItemMenu {
    pub fn new(playlist_scrollbar: Entity<Scrollbar>, cx: &mut App) -> Self {
        Self {
            playlist_submenu: SubmenuState::default(),
            artist_submenu: SubmenuState::default(),
            playlist_scrollbar,
            playlist_search: MenuSearch::new("menu-search-playlists", cx),
        }
    }

    pub fn reset(&self, cx: &App) {
        self.playlist_submenu.reset();
        self.playlist_search.reset();
        self.artist_submenu.reset();
        self.playlist_scrollbar
            .read(cx)
            .scroll()
            .set_offset(gpui::Point::default());
    }

    pub fn for_track(&self, track: &Track, cx: &App) -> Menu {
        self.build(
            std::slice::from_ref(track),
            None,
            None,
            None,
            TrackColumns::default(),
            cx,
        )
    }

    pub fn for_table_tracks(&self, tracks: &[Track], columns: TrackColumns, cx: &App) -> Menu {
        self.build(tracks, None, None, None, columns, cx)
    }

    pub fn for_album_tracks(
        &self,
        tracks: &[Track],
        album_id: &str,
        columns: TrackColumns,
        cx: &App,
    ) -> Menu {
        self.build(tracks, None, None, Some(album_id), columns, cx)
    }

    pub fn for_playlist_tracks(
        &self,
        tracks: &[Track],
        detail: Entity<Detail>,
        columns: TrackColumns,
        cx: &App,
    ) -> Menu {
        let ids: Vec<String> = tracks.iter().filter_map(|track| track.id.clone()).collect();
        let count = tracks.len();
        let remove = match ids.is_empty() {
            true => MenuItem::new(
                "remove-from-playlist",
                counted(
                    "menu-remove-from-playlist",
                    "menu-remove-tracks-from-playlist",
                    count,
                ),
            )
            .icon("icons/x.svg")
            .disabled(),
            false => MenuItem::new(
                "remove-from-playlist",
                counted(
                    "menu-remove-from-playlist",
                    "menu-remove-tracks-from-playlist",
                    count,
                ),
            )
            .icon("icons/x.svg")
            .on_click(move |_, _, cx| {
                Confirm::playlist_songs(ids.clone(), detail.clone(), count, cx)
            }),
        };
        self.build(tracks, Some(remove), None, None, columns, cx)
    }

    pub fn for_history_tracks(
        &self,
        tracks: &[Track],
        history: Entity<History>,
        columns: TrackColumns,
        cx: &App,
    ) -> Menu {
        let held = tracks.to_vec();
        let forget = MenuItem::new(
            "remove-from-history",
            counted(
                "menu-remove-from-history",
                "menu-remove-tracks-from-history",
                tracks.len(),
            ),
        )
        .icon("icons/trash-2.svg")
        .on_click(move |_, _, cx| Confirm::history_songs(held.clone(), history.clone(), cx));
        self.build(tracks, None, Some(forget), None, columns, cx)
    }

    /// The Add to playlist submenu for `addition`: a New playlist entry, then the playlists on
    /// `shelf` the listener can add to. A playlist that already holds every named track is
    /// checked, and `many` says whether one track or a run of them is being added. An album is
    /// never checked, since its tracks are only read once it is added.
    fn playlists(&self, addition: Addition, shelf: Shelf, many: bool, cx: &App) -> Menu {
        let library = Sonora::global(cx).library.clone();
        let playlists: Vec<Playlist> = library
            .read(cx)
            .state(shelf)
            .playlists()
            .iter()
            .filter(|playlist| playlist.owned || playlist.collaborative)
            .cloned()
            .collect();
        let ids = match &addition {
            Addition::Tracks(ids) => ids.clone(),
            Addition::Album(_) => Vec::new(),
        };
        let album = matches!(addition, Addition::Album(_));
        let created = addition.clone();
        let new_playlist = MenuItem::new("new-playlist", t!("menu-new-playlist"))
            .icon("icons/plus.svg")
            .on_click(move |_, window, cx| {
                PlaylistEditor::open(
                    Edit::Create {
                        addition: created.clone(),
                        shelf,
                    },
                    window,
                    cx,
                );
            });
        if playlists.is_empty() {
            Menu::new("playlist-submenu")
                .w(gpui::px(220.))
                .item(new_playlist)
                .item(MenuItem::separator("playlist-separator"))
                .item(MenuItem::new("no-playlists", t!("menu-no-playlists")).disabled())
        } else {
            let query = self.playlist_search.query(cx);
            let searching = !query.is_empty();
            let found: Vec<Playlist> = playlists
                .into_iter()
                .filter(|playlist| playlist.name.to_lowercase().contains(&query))
                .collect();
            let unmatched = found.is_empty();
            Menu::new("playlist-submenu")
                .w(gpui::px(220.))
                .max_h(gpui::px(360.))
                .scrollbar(self.playlist_scrollbar.clone())
                .search(self.playlist_search.clone())
                .when(!searching, |menu| {
                    menu.item(new_playlist)
                        .item(MenuItem::separator("playlist-separator"))
                })
                .when(unmatched, |menu| {
                    menu.item(
                        MenuItem::new("no-matching-playlists", t!("search-no-matches")).disabled(),
                    )
                })
                .items(found.into_iter().map(|playlist| {
                    let held = !ids.is_empty()
                        && ids
                            .iter()
                            .all(|id| library.read(cx).holds(&playlist.id, id).unwrap_or(false));
                    let item =
                        MenuItem::new(format!("playlist-{}", playlist.id), playlist.name.clone())
                            .artwork(playlist.cover.clone())
                            .checked(held);
                    if album {
                        let library = library.clone();
                        let playlist_id = playlist.id.clone();
                        let addition = addition.clone();
                        return item.on_click(move |_, _, cx| {
                            library.update(cx, |library, cx| {
                                library.add_tracks_to_playlist(
                                    playlist_id.clone(),
                                    addition.clone(),
                                    cx,
                                )
                            });
                        });
                    }
                    match ids.is_empty() {
                        true => item.disabled(),
                        false => {
                            let library = library.clone();
                            let playlist_id = playlist.id.clone();
                            let track_ids = ids.clone();
                            item.on_click(move |_, window, cx| {
                                if held && !many {
                                    PlaylistEditor::open(
                                        Edit::Again {
                                            playlist: playlist.clone(),
                                            track: track_ids[0].clone(),
                                        },
                                        window,
                                        cx,
                                    );
                                    return;
                                }
                                if held {
                                    return;
                                }
                                let missing: Vec<String> = track_ids
                                    .iter()
                                    .filter(|id| {
                                        !library.read(cx).holds(&playlist_id, id).unwrap_or(false)
                                    })
                                    .cloned()
                                    .collect();
                                if missing.is_empty() {
                                    return;
                                }
                                library.update(cx, |library, cx| {
                                    library.add_tracks_to_playlist(
                                        playlist_id.clone(),
                                        Addition::Tracks(missing),
                                        cx,
                                    )
                                });
                            })
                        }
                    }
                }))
        }
    }

    fn build(
        &self,
        tracks: &[Track],
        library_action: Option<MenuItem>,
        trailing: Option<MenuItem>,
        current_album: Option<&str>,
        columns: TrackColumns,
        cx: &App,
    ) -> Menu {
        let Some(track) = tracks.first() else {
            return Menu::new("track-context-menu");
        };
        let count = tracks.len();
        let many = count > 1;
        let library = Sonora::global(cx).library.clone();
        let held: Vec<String> = tracks.iter().filter_map(|track| track.id.clone()).collect();
        let imported = !held.is_empty() && held.iter().all(|id| music::is_local_id(id));
        let ids: Vec<String> = match imported {
            true => held,
            false => held
                .into_iter()
                .filter(|id| !music::is_local_id(id))
                .collect(),
        };
        let barren = ids.is_empty();
        let shelf = match imported {
            true => Shelf::Local,
            false => Shelf::Streaming,
        };
        let playlist_menu = self.playlists(Addition::Tracks(ids.clone()), shelf, many, cx);
        let copy = match (many, track.id.clone()) {
            (true, _) => None,
            (false, Some(id)) => Some(
                MenuItem::new("copy-track-link", t!("menu-copy-link"))
                    .icon("icons/link.svg")
                    .on_click(move |_, _, cx| copy_link(MediaKind::Track, &id, cx)),
            ),
            (false, None) => Some(
                MenuItem::new("copy-track-link", t!("menu-copy-link"))
                    .icon("icons/link.svg")
                    .disabled(),
            ),
        };
        let queued: Vec<Track> = tracks
            .iter()
            .filter(|track| track.playable)
            .cloned()
            .collect();
        let next = queue_item(
            "play-next",
            counted("menu-play-next", "menu-play-tracks-next", count),
            "icons/list-start.svg",
            &queued,
            Playback::play_next,
            Playback::play_next_all,
        );
        let queue = queue_item(
            "add-to-queue",
            counted("menu-add-to-queue", "menu-add-tracks-to-queue", count),
            "icons/list-plus.svg",
            &queued,
            Playback::enqueue,
            Playback::enqueue_all,
        );
        let last = queue_item(
            "play-last",
            counted("menu-play-last", "menu-play-tracks-last", count),
            "icons/list-end.svg",
            &queued,
            Playback::play_last,
            Playback::play_last_all,
        );
        // A provider that lists no station tracks gets no station item at all, rather than one
        // that plays the seed and stops.
        let stations = Sonora::global(cx).session.read(cx).capabilities().radio;
        let radio = match (many || !stations, track.id.is_some() && track.playable) {
            (true, _) => None,
            (false, true) => {
                let track = track.clone();
                Some(
                    MenuItem::new("song-radio", t!("menu-song-radio"))
                        .icon("icons/radio.svg")
                        .on_click(move |_, _, cx| {
                            let playback = Sonora::global(cx).playback.clone();
                            playback.update(cx, |playback, cx| playback.play_radio(&track, cx));
                        }),
                )
            }
            (false, false) => Some(
                MenuItem::new("song-radio", t!("menu-song-radio"))
                    .icon("icons/radio.svg")
                    .disabled(),
            ),
        };
        let toggle_library = library_toggle(tracks, &library, cx);
        let membership =
            (!barren && !imported && Sonora::global(cx).session.read(cx).capabilities().library)
                .then(|| library_membership(tracks, &library, cx));

        let album = match (many, columns.album, track.album_id.clone()) {
            (true, _, _) | (false, true, _) => None,
            (false, false, Some(id)) if Some(id.as_str()) == current_album => None,
            (false, false, Some(id)) => Some(
                MenuItem::new("go-to-album", t!("menu-go-to-album"))
                    .icon("icons/disc-3.svg")
                    .on_click(move |_, _, cx| navigate(Destination::Album(id.clone().into()), cx)),
            ),
            (false, false, None) => Some(
                MenuItem::new("go-to-album", t!("menu-go-to-album"))
                    .icon("icons/disc-3.svg")
                    .disabled(),
            ),
        };

        let artists = track
            .artist_refs
            .iter()
            .filter_map(|artist| {
                let id = artist.id.clone()?;
                Some((artist.name.clone(), id))
            })
            .collect::<Vec<_>>();
        let artist = match (many, columns.artists, artists.len()) {
            (true, _, _) | (false, true, _) => None,
            (false, false, 0) => Some(
                MenuItem::new("go-to-artist", t!("menu-go-to-artist"))
                    .icon("icons/user.svg")
                    .disabled(),
            ),
            (false, false, 1) => {
                let id = artists[0].1.clone();
                Some(
                    MenuItem::new("go-to-artist", t!("menu-go-to-artist"))
                        .icon("icons/user.svg")
                        .on_click(move |_, _, cx| {
                            navigate(Destination::Artist(id.clone().into()), cx)
                        }),
                )
            }
            (false, false, _) => {
                let artist_menu = Menu::new("artist-submenu")
                    .w(gpui::px(220.))
                    .max_h(gpui::px(360.))
                    .items(artists.into_iter().map(|(name, id)| {
                        MenuItem::new(format!("artist-{id}"), name).on_click(move |_, _, cx| {
                            navigate(Destination::Artist(id.clone().into()), cx)
                        })
                    }));
                Some(
                    MenuItem::new("go-to-artist", t!("menu-go-to-artist"))
                        .icon("icons/user.svg")
                        .submenu(artist_menu, self.artist_submenu.clone()),
                )
            }
        };

        let details = match (many, track.id.clone()) {
            (true, _) => None,
            (false, Some(id)) => Some(
                MenuItem::new("view-details", t!("menu-view-details"))
                    .icon("icons/info.svg")
                    .on_click(move |_, _, cx| navigate(Destination::Song(id.clone().into()), cx)),
            ),
            (false, None) => Some(
                MenuItem::new("view-details", t!("menu-view-details"))
                    .icon("icons/info.svg")
                    .disabled(),
            ),
        };

        let pinnable = (!many).then(|| track.pin()).flatten();
        let edit = (!many && imported).then(|| {
            let track = track.clone();
            MenuItem::new("edit-tags", t!("menu-edit-tags"))
                .icon("icons/pencil.svg")
                .on_click(move |_, window, cx| TagEditor::open(track.clone(), window, cx))
        });

        let delete_files = imported.then(|| {
            let ids = ids.clone();
            MenuItem::new(
                "delete-track-files",
                counted("menu-delete-track-file", "menu-delete-track-files", count),
            )
            .icon("icons/trash-2.svg")
            .on_click(move |_, _, cx| Confirm::delete_track_files(ids.clone(), cx))
        });

        let add_to_playlist = (!barren).then(|| {
            MenuItem::new(
                "add-to-playlist",
                counted("menu-add-to-playlist", "menu-add-tracks-to-playlist", count),
            )
            .icon("icons/square-plus.svg")
            .submenu(playlist_menu, self.playlist_submenu.clone())
        });

        sections(
            Menu::new("track-context-menu")
                .relative()
                .w(gpui::px(match many {
                    true => 248.,
                    false => 210.,
                })),
            vec![
                add_to_playlist
                    .into_iter()
                    .chain([library_action.unwrap_or(toggle_library)])
                    .chain(membership)
                    .collect(),
                [next, queue, last].into_iter().chain(radio).collect(),
                album.into_iter().chain(artist).collect(),
                details
                    .into_iter()
                    .chain(edit)
                    .chain(copy)
                    .chain(delete_files)
                    .collect(),
                pinnable
                    .map(|pin| pin_action(&pin, cx))
                    .into_iter()
                    .collect(),
                trailing.into_iter().collect(),
            ],
        )
    }
}

/// A queue item for the playable tracks of a selection. It is disabled when there are none, and
/// picking it hands a single track to `one` and a run of them to `many`.
fn queue_item(
    id: &'static str,
    label: SharedString,
    icon: &'static str,
    queued: &[Track],
    one: fn(&mut Playback, Track, &mut Context<Playback>),
    many: fn(&mut Playback, Vec<Track>, &mut Context<Playback>),
) -> MenuItem {
    let item = MenuItem::new(id, label).icon(icon);
    if queued.is_empty() {
        return item.disabled();
    }
    let queued = queued.to_vec();
    item.on_click(move |_, _, cx| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| match queued.len() {
            1 => one(playback, queued[0].clone(), cx),
            _ => many(playback, queued.clone(), cx),
        });
    })
}

fn counted(one: &'static str, many: &'static str, count: usize) -> SharedString {
    if count <= 1 {
        return i18n::lookup(one, None);
    }
    let mut args = i18n::FluentArgs::new();
    args.set("count", count as i64);
    i18n::lookup(many, Some(&args))
}

fn library_toggle(tracks: &[Track], library: &Entity<Library>, cx: &App) -> MenuItem {
    let count = tracks.len();
    let actionable: Vec<Track> = tracks
        .iter()
        .filter(|track| {
            track
                .id
                .as_deref()
                .is_some_and(|id| !library.read(cx).pending(id))
        })
        .cloned()
        .collect();
    let saved = !actionable.is_empty()
        && actionable.iter().all(|track| {
            track
                .id
                .as_deref()
                .is_some_and(|id| library.read(cx).saved(id))
        });
    let item = MenuItem::new(
        "toggle-library",
        match saved {
            true => counted(
                "menu-remove-from-library",
                "menu-remove-tracks-from-library",
                count,
            ),
            false => counted("menu-add-to-library", "menu-add-tracks-to-library", count),
        },
    )
    .icon(match saved {
        true => "icons/heart-off.svg",
        false => "icons/heart.svg",
    });

    match actionable.is_empty() {
        true => item.disabled(),
        false => {
            let library = library.clone();
            item.on_click(move |_, _, cx| {
                if saved {
                    Confirm::library_songs(actionable.clone(), cx);
                    return;
                }
                library.update(cx, |library, cx| {
                    let tracks = actionable
                        .iter()
                        .filter(|track| !track.id.as_deref().is_some_and(|id| library.saved(id)))
                        .cloned()
                        .collect();
                    library.save_tracks(tracks, true, cx);
                });
            })
        }
    }
}

/// Add to Library or Remove from Library for tracks, on a provider whose library is apart
/// from its favorites. Several tracks are added unless every one of them is there already,
/// in which case they are removed.
fn library_membership(tracks: &[Track], library: &Entity<Library>, cx: &App) -> MenuItem {
    let count = tracks.len();
    let actionable: Vec<Track> = tracks
        .iter()
        .filter(|track| {
            track
                .id
                .as_deref()
                .is_some_and(|id| !library.read(cx).pending_library(id))
        })
        .cloned()
        .collect();
    let present = !actionable.is_empty()
        && actionable.iter().all(|track| {
            track
                .id
                .as_deref()
                .is_some_and(|id| library.read(cx).in_library(id))
        });
    let item = MenuItem::new(
        "toggle-library-membership",
        match present {
            true => counted("menu-library-remove", "menu-library-remove-tracks", count),
            false => counted("menu-library-add", "menu-library-add-tracks", count),
        },
    )
    .icon(match present {
        true => "icons/library-big-off.svg",
        false => "icons/library-big.svg",
    });

    match actionable.is_empty() {
        true => item.disabled(),
        false => {
            let library = library.clone();
            item.on_click(move |_, _, cx| {
                library.update(cx, |library, cx| {
                    for track in actionable.clone() {
                        let there = track.id.as_deref().is_some_and(|id| library.in_library(id));
                        if there == present {
                            library.set_track_in_library(track, !present, cx);
                        }
                    }
                });
            })
        }
    }
}

/// The same for one album, and nothing on a provider whose library is its favorites.
fn album_membership_item(album: Album, cx: &App) -> Option<MenuItem> {
    let session = Sonora::global(cx).session.read(cx);
    if !session.capabilities().library || music::is_local_id(&album.id) {
        return None;
    }
    let library = Sonora::global(cx).library.clone();
    let present = library.read(cx).in_library(&album.id);
    let item = MenuItem::new(
        "toggle-album-membership",
        match present {
            true => t!("menu-library-remove"),
            false => t!("menu-library-add"),
        },
    )
    .icon(match present {
        true => "icons/library-big-off.svg",
        false => "icons/library-big.svg",
    });
    Some(match library.read(cx).pending_library(&album.id) {
        true => item.disabled(),
        false => item.on_click(move |_, _, cx| {
            let library = Sonora::global(cx).library.clone();
            library.update(cx, |library, cx| {
                library.set_album_in_library(album.clone(), !present, cx)
            });
        }),
    })
}

fn sections(menu: Menu, groups: Vec<Vec<MenuItem>>) -> Menu {
    groups
        .into_iter()
        .filter(|group| !group.is_empty())
        .enumerate()
        .fold(menu, |menu, (index, group)| {
            match index {
                0 => menu,
                _ => menu.item(MenuItem::separator(format!("section-{index}"))),
            }
            .items(group)
        })
}

/// An album's menu, laid out like a track's: Add to playlist and the library first, then the
/// queue, then the link and the pin. `menus` holds the playlist submenu's state.
pub(crate) fn album_menu(
    album: Album,
    playback: Entity<Playback>,
    menus: &ItemMenu,
    cx: &App,
) -> Menu {
    let album_id = album.id.clone();
    let next = album_id.clone();
    let queued = album_id.clone();
    let last = album_id.clone();
    let copied = album_id.clone();
    let nexting = playback.clone();
    let queueing = playback.clone();
    let lasting = playback;
    let add_to_playlist = MenuItem::new("add-album-to-playlist", t!("menu-add-to-playlist"))
        .icon("icons/square-plus.svg")
        .submenu(
            menus.playlists(
                Addition::Album(album_id.clone()),
                Shelf::of(&album_id),
                true,
                cx,
            ),
            menus.playlist_submenu.clone(),
        );

    sections(
        Menu::new("album-context-menu"),
        vec![
            [add_to_playlist, album_library_item(album.clone(), cx)]
                .into_iter()
                .chain(album_membership_item(album.clone(), cx))
                .collect(),
            vec![
                MenuItem::new("play-album-next", t!("menu-play-next"))
                    .icon("icons/list-start.svg")
                    .on_click(move |_, _, cx| {
                        nexting.update(cx, |playback, cx| playback.play_album_next(&next, cx));
                    }),
                MenuItem::new("enqueue-album", t!("menu-add-to-queue"))
                    .icon("icons/list-plus.svg")
                    .on_click(move |_, _, cx| {
                        queueing.update(cx, |playback, cx| playback.enqueue_album(&queued, cx));
                    }),
                MenuItem::new("play-album-last", t!("menu-play-last"))
                    .icon("icons/list-end.svg")
                    .on_click(move |_, _, cx| {
                        lasting.update(cx, |playback, cx| playback.play_album_last(&last, cx));
                    }),
            ],
            vec![
                MenuItem::new("copy-album-link", t!("menu-copy-link"))
                    .icon("icons/link.svg")
                    .on_click(move |_, _, cx| copy_link(MediaKind::Album, &copied, cx)),
            ],
            album
                .pin()
                .map(|pin| pin_action(&pin, cx))
                .into_iter()
                .collect(),
        ],
    )
}

fn album_library_item(album: Album, cx: &App) -> MenuItem {
    let library = Sonora::global(cx).library.clone();
    let saved = library.read(cx).saved_album(&album.id);
    let item = MenuItem::new(
        "toggle-album-library",
        match saved {
            true => t!("menu-remove-from-library"),
            false => t!("menu-add-to-library"),
        },
    )
    .icon(match saved {
        true => "icons/heart-off.svg",
        false => "icons/heart.svg",
    });

    match library.read(cx).pending_album(&album.id) {
        true => item.disabled(),
        false => item.on_click(move |_, _, cx| match saved {
            true => Confirm::albums(vec![album.clone()], cx),
            false => {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| library.toggle_album(album.clone(), cx));
            }
        }),
    }
}

pub(crate) fn artist_menu(
    artist: SavedArtist,
    playback: Entity<Playback>,
    opened_here: bool,
    cx: &App,
) -> Menu {
    let artist_id = artist.id.clone();
    let opened = artist_id.clone();
    let played = Origin::artist(artist_id.clone()).named(artist.name.clone());
    let next = artist_id.clone();
    let queued = artist_id.clone();
    let last = artist_id.clone();
    let copied = artist_id.clone();
    let playing = playback.clone();
    let nexting = playback.clone();
    let queueing = playback.clone();
    let lasting = playback;

    let open = match opened_here {
        true => Vec::new(),
        false => vec![
            MenuItem::new("open-artist", t!("menu-go-to-artist"))
                .icon("icons/info.svg")
                .on_click(move |_, _, cx| navigate(Destination::Artist(opened.clone().into()), cx)),
        ],
    };

    sections(
        Menu::new("artist-context-menu"),
        vec![
            open,
            vec![
                MenuItem::new("play-artist", t!("menu-play-artist"))
                    .icon("icons/play.svg")
                    .on_click(move |_, _, cx| {
                        playing.update(cx, |playback, cx| playback.play_origin(played.clone(), cx));
                    }),
                MenuItem::new("play-artist-next", t!("menu-play-next"))
                    .icon("icons/list-start.svg")
                    .on_click(move |_, _, cx| {
                        nexting.update(cx, |playback, cx| playback.play_artist_next(&next, cx));
                    }),
                MenuItem::new("enqueue-artist", t!("menu-add-to-queue"))
                    .icon("icons/list-plus.svg")
                    .on_click(move |_, _, cx| {
                        queueing.update(cx, |playback, cx| playback.enqueue_artist(&queued, cx));
                    }),
                MenuItem::new("play-artist-last", t!("menu-play-last"))
                    .icon("icons/list-end.svg")
                    .on_click(move |_, _, cx| {
                        lasting.update(cx, |playback, cx| playback.play_artist_last(&last, cx));
                    }),
            ],
            artist_library_item(artist.clone(), cx)
                .into_iter()
                .collect(),
            vec![
                MenuItem::new("copy-artist-link", t!("menu-copy-link"))
                    .icon("icons/link.svg")
                    .on_click(move |_, _, cx| copy_link(MediaKind::Artist, &copied, cx)),
            ],
            artist
                .pin()
                .map(|pin| pin_action(&pin, cx))
                .into_iter()
                .collect(),
        ],
    )
}

fn artist_library_item(artist: SavedArtist, cx: &App) -> Option<MenuItem> {
    if !Sonora::global(cx)
        .session
        .read(cx)
        .capabilities()
        .follow_artists
    {
        return None;
    }
    let library = Sonora::global(cx).library.clone();
    let saved = library.read(cx).saved_artist(&artist.id);
    let item = MenuItem::new(
        "toggle-artist-library",
        match saved {
            true => t!("menu-remove-from-library"),
            false => t!("menu-add-to-library"),
        },
    )
    .icon(match saved {
        true => "icons/heart-off.svg",
        false => "icons/heart.svg",
    });

    Some(match library.read(cx).pending_artist(&artist.id) {
        true => item.disabled(),
        false => item.on_click(move |_, _, cx| match saved {
            true => Confirm::artists(vec![artist.clone()], cx),
            false => {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| library.toggle_artist(artist.clone(), cx));
            }
        }),
    })
}

pub(crate) fn playlist_menu(playlist: Playlist, playback: Entity<Playback>, cx: &App) -> Menu {
    let next = playlist.id.clone();
    let queued = playlist.id.clone();
    let last = playlist.id.clone();
    let copied = playlist.id.clone();
    let nexting = playback.clone();
    let queueing = playback.clone();
    let lasting = playback;
    let id = playlist.id.clone();
    let public = playlist.public;
    let pinnable = playlist.pin();
    let imported = music::is_local_id(&playlist.id);
    let visibility = (!imported).then(|| {
        MenuItem::new(
            "playlist-visibility",
            match public {
                true => t!("menu-make-playlist-private"),
                false => t!("menu-make-playlist-public"),
            },
        )
        .icon("icons/user.svg")
        .on_click({
            let id = id.clone();
            move |_, _, cx| {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| {
                    library.set_playlist_public(id.clone(), !public, cx)
                });
            }
        })
    });
    let actions = match playlist.owned {
        true => visibility
            .into_iter()
            .chain([
                MenuItem::new("rename-playlist", t!("menu-rename-playlist"))
                    .icon("icons/pencil.svg")
                    .on_click({
                        let playlist = playlist.clone();
                        move |_, window, cx| {
                            PlaylistEditor::open(Edit::Rename(playlist.clone()), window, cx);
                        }
                    }),
                MenuItem::new("delete-playlist", t!("menu-delete-playlist"))
                    .icon("icons/trash-2.svg")
                    .on_click(move |_, window, cx| {
                        PlaylistEditor::open(Edit::Delete(playlist.clone()), window, cx);
                    }),
            ])
            .collect(),
        false => vec![playlist_library_item(playlist.clone(), cx)],
    };

    sections(
        Menu::new("playlist-context-menu"),
        vec![
            vec![
                MenuItem::new("play-playlist-next", t!("menu-play-next"))
                    .icon("icons/list-start.svg")
                    .on_click(move |_, _, cx| {
                        nexting.update(cx, |playback, cx| playback.play_playlist_next(&next, cx));
                    }),
                MenuItem::new("enqueue-playlist", t!("menu-add-to-queue"))
                    .icon("icons/list-plus.svg")
                    .on_click(move |_, _, cx| {
                        queueing.update(cx, |playback, cx| playback.enqueue_playlist(&queued, cx));
                    }),
                MenuItem::new("play-playlist-last", t!("menu-play-last"))
                    .icon("icons/list-end.svg")
                    .on_click(move |_, _, cx| {
                        lasting.update(cx, |playback, cx| playback.play_playlist_last(&last, cx));
                    }),
            ],
            actions,
            match imported {
                true => Vec::new(),
                false => vec![
                    MenuItem::new("copy-playlist-link", t!("menu-copy-link"))
                        .icon("icons/link.svg")
                        .on_click(move |_, _, cx| copy_link(MediaKind::Playlist, &copied, cx)),
                ],
            },
            pinnable
                .map(|pin| pin_action(&pin, cx))
                .into_iter()
                .collect(),
        ],
    )
}

pub(crate) fn item_menu(
    pin: &Pin,
    tracks: &ItemMenu,
    playback: Entity<Playback>,
    cx: &App,
) -> Menu {
    let library = Sonora::global(cx).library.clone();
    let built = match pin.kind {
        PinKind::Album => library
            .read(cx)
            .album(&pin.id)
            .cloned()
            .map(|album| album_menu(album, playback.clone(), tracks, cx)),
        PinKind::Playlist => library
            .read(cx)
            .playlist(&pin.id)
            .cloned()
            .map(|playlist| playlist_menu(playlist, playback.clone(), cx)),
        PinKind::Artist => Some(artist_menu(
            library
                .read(cx)
                .artist(&pin.id)
                .cloned()
                .unwrap_or_else(|| pinned_artist(pin)),
            playback.clone(),
            false,
            cx,
        )),
        PinKind::Song => saved_track(&pin.id, cx).map(|track| tracks.for_track(&track, cx)),
        // Nothing but the sparse menu fits a folder: it opens, and it pins.
        PinKind::Folder => None,
    };

    built.unwrap_or_else(|| sparse_menu(pin, playback, cx))
}

/// The menu of a folder of playlists, wherever one is listed.
pub(crate) fn folder_menu(id: &str, name: &str, cx: &App) -> Menu {
    let pin = Pin::new(PinKind::Folder, id, name);
    sparse_menu(&pin, Sonora::global(cx).playback.clone(), cx)
}

pub(crate) fn pinned_artist(pin: &Pin) -> SavedArtist {
    SavedArtist {
        id: pin.id.clone(),
        name: pin.title.clone(),
        cover: pin.cover.clone(),
        added_at: None,
    }
}

fn sparse_menu(pin: &Pin, playback: Entity<Playback>, cx: &App) -> Menu {
    let destination = Destination::from(pin);
    let copied = pin.id.clone();
    let kind = media_kind(pin.kind);
    let open = open_key(pin.kind).map(|key| {
        MenuItem::new("open-pin", i18n::lookup(key, None))
            .icon("icons/info.svg")
            .on_click(move |_, _, cx| navigate(destination.clone(), cx))
    });

    let link = kind
        .map(|kind| {
            MenuItem::new("copy-pin-link", t!("menu-copy-link"))
                .icon("icons/link.svg")
                .on_click(move |_, _, cx| copy_link(kind, &copied, cx))
        })
        .into_iter()
        .collect();

    sections(
        Menu::new("pin-context-menu"),
        vec![
            open.into_iter().collect(),
            transport_items(pin, playback),
            link,
            vec![pin_action(pin, cx)],
        ],
    )
}

/// The entry that opens a pin's page, which only an artist's and a song's menus carry, the
/// way their full menus do.
fn open_key(kind: PinKind) -> Option<&'static str> {
    match kind {
        PinKind::Album | PinKind::Playlist | PinKind::Folder => None,
        PinKind::Artist => Some("menu-go-to-artist"),
        PinKind::Song => Some("menu-view-details"),
    }
}

fn transport_items(pin: &Pin, playback: Entity<Playback>) -> Vec<MenuItem> {
    // A folder holds playlists rather than tracks, so it has no transport at all.
    let Some(played) = Origin::from_pin(pin) else {
        return Vec::new();
    };
    let next = pin.id.clone();
    let queued = pin.id.clone();
    let last = pin.id.clone();
    let nexting = playback.clone();
    let queueing = playback.clone();
    let lasting = playback.clone();

    match pin.kind {
        PinKind::Album => vec![
            MenuItem::new("play-pin-next", t!("menu-play-next"))
                .icon("icons/list-start.svg")
                .on_click(move |_, _, cx| {
                    nexting.update(cx, |playback, cx| playback.play_album_next(&next, cx));
                }),
            MenuItem::new("enqueue-pin", t!("menu-add-to-queue"))
                .icon("icons/list-plus.svg")
                .on_click(move |_, _, cx| {
                    queueing.update(cx, |playback, cx| playback.enqueue_album(&queued, cx));
                }),
            MenuItem::new("play-pin-last", t!("menu-play-last"))
                .icon("icons/list-end.svg")
                .on_click(move |_, _, cx| {
                    lasting.update(cx, |playback, cx| playback.play_album_last(&last, cx));
                }),
        ],
        PinKind::Playlist => vec![
            MenuItem::new("play-pin-next", t!("menu-play-next"))
                .icon("icons/list-start.svg")
                .on_click(move |_, _, cx| {
                    nexting.update(cx, |playback, cx| playback.play_playlist_next(&next, cx));
                }),
            MenuItem::new("enqueue-pin", t!("menu-add-to-queue"))
                .icon("icons/list-plus.svg")
                .on_click(move |_, _, cx| {
                    queueing.update(cx, |playback, cx| playback.enqueue_playlist(&queued, cx));
                }),
            MenuItem::new("play-pin-last", t!("menu-play-last"))
                .icon("icons/list-end.svg")
                .on_click(move |_, _, cx| {
                    lasting.update(cx, |playback, cx| playback.play_playlist_last(&last, cx));
                }),
        ],
        PinKind::Artist => vec![
            MenuItem::new("play-pin", t!("menu-play-artist"))
                .icon("icons/play.svg")
                .on_click(move |_, _, cx| {
                    playback.update(cx, |playback, cx| playback.play_origin(played.clone(), cx));
                }),
            MenuItem::new("play-pin-next", t!("menu-play-next"))
                .icon("icons/list-start.svg")
                .on_click(move |_, _, cx| {
                    nexting.update(cx, |playback, cx| playback.play_artist_next(&next, cx));
                }),
            MenuItem::new("enqueue-pin", t!("menu-add-to-queue"))
                .icon("icons/list-plus.svg")
                .on_click(move |_, _, cx| {
                    queueing.update(cx, |playback, cx| playback.enqueue_artist(&queued, cx));
                }),
            MenuItem::new("play-pin-last", t!("menu-play-last"))
                .icon("icons/list-end.svg")
                .on_click(move |_, _, cx| {
                    lasting.update(cx, |playback, cx| playback.play_artist_last(&last, cx));
                }),
        ],
        PinKind::Song => vec![
            MenuItem::new("play-pin", t!("menu-song-radio"))
                .icon("icons/radio.svg")
                .on_click(move |_, _, cx| {
                    playback.update(cx, |playback, cx| playback.play_origin(played.clone(), cx));
                }),
        ],
        PinKind::Folder => Vec::new(),
    }
}

/// Pins or unpins anything the app can open. Every context menu carries it, and the provider
/// that keeps pins of its own is told alongside the local list.
pub(crate) fn pin_action(pin: &Pin, cx: &App) -> MenuItem {
    let pins = Sonora::global(cx).pins.clone();
    let session = Sonora::global(cx).session.clone();
    let known = session.read(cx).slug_for(&pin.id).is_some();
    let pinned = pins.read(cx).holds(pin, cx);
    let held = pin.clone();

    let item = MenuItem::new(
        "pin",
        i18n::lookup(if pinned { "nav-unpin" } else { "nav-pin" }, None),
    )
    .icon("icons/pin.svg");
    match known {
        false => item.disabled(),
        true => item.on_click(move |_, _, cx| {
            pins.update(cx, |pins, cx| pins.toggle(held.clone(), cx));
        }),
    }
}

/// What a pin shares as a link, or `None` for one with no page of its own on the provider: a
/// folder only exists inside the library that lists it.
fn media_kind(kind: PinKind) -> Option<MediaKind> {
    match kind {
        PinKind::Album => Some(MediaKind::Album),
        PinKind::Artist => Some(MediaKind::Artist),
        PinKind::Playlist => Some(MediaKind::Playlist),
        PinKind::Song => Some(MediaKind::Track),
        PinKind::Folder => None,
    }
}

fn saved_track(id: &str, cx: &App) -> Option<Track> {
    Sonora::global(cx)
        .library
        .read(cx)
        .state(Shelf::of(id))
        .tracks()
        .iter()
        .find(|track| track.id.as_deref() == Some(id))
        .cloned()
}

fn copy_link(kind: MediaKind, id: &str, cx: &mut App) {
    let session = Sonora::global(cx).session.read(cx);
    let client = match music::is_local_id(id) {
        true => session.local_client(),
        false => session.client(),
    };
    let Some(client) = client else {
        return;
    };
    let Some(url) = client.share_url(kind, id) else {
        return;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(url));
}

fn playlist_library_item(playlist: Playlist, cx: &App) -> MenuItem {
    let library = Sonora::global(cx).library.clone();
    let saved = library.read(cx).playlist(&playlist.id).is_some();

    match saved {
        true => {
            let id = playlist.id;
            MenuItem::new("leave-playlist", t!("menu-remove-playlist-from-library"))
                .icon("icons/heart-off.svg")
                .on_click(move |_, _, cx| Confirm::playlists(vec![id.clone()], cx))
        }
        false => MenuItem::new("join-playlist", t!("menu-add-playlist-to-library"))
            .icon("icons/heart.svg")
            .on_click(move |_, _, cx| {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| {
                    library.add_playlist_to_library(playlist.clone(), cx)
                });
            }),
    }
}

pub(crate) fn new_playlist_menu(
    on_create: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Menu {
    Menu::new("playlist-background-menu").item(
        MenuItem::new("create-playlist", t!("menu-new-playlist"))
            .icon("icons/plus.svg")
            .on_click(on_create),
    )
}
