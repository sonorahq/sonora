use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, ClickEvent, ClipboardItem, Entity, Pixels, SharedString, Window, div, px};
use i18n::t;
use music::{Album, GenreItem, MediaKind, Playlist, SavedArtist, Track};
use router::{Destination, navigate};
use state::{
    Addition, Collection, Detail, History, Library, Origin, Outcome, Playback, Progress,
    QueuePlacement, Shelf, Sonora, Toasts,
};
use ui::{
    ActiveTheme as _, ArrowRing, Button, Menu, MenuItem, MenuSearch, Pin, PinKind, Scrollbar,
    SubmenuState,
};

use crate::shared::confirm::Confirm;
use crate::shared::local;
use crate::shared::pins::Pinned as _;
use crate::shared::playlist_editor::{Edit, PlaylistEditor};
use crate::shared::tag_editor::TagEditor;

/// The narrowest the menu of one song, album, artist or playlist gets, which is wide enough for
/// English. A locale with longer labels widens the menu to fit them.
const ITEM_MENU_WIDTH: Pixels = px(196.);

/// How narrow a menu over several selected tracks can get, since its labels carry the count.
const SELECTION_MENU_WIDTH: Pixels = px(248.);

/// Something a context menu can open on, wherever it was clicked.
#[derive(Clone)]
pub(crate) enum Item {
    Album(Album),
    Playlist(Playlist),
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

    /// The Save to submenu for `addition`: the `lead` rows, New playlist, then the playlists on
    /// `shelf` the listener can add to. The lead rows hide while a search narrows the playlists.
    /// A playlist holding every named track is checked, and picking it takes the tracks out
    /// again with the menu left open. An album is never checked, since its tracks are only read
    /// once it is added, and picking a playlist for it closes the menu.
    fn playlist_rows(
        &self,
        addition: Addition,
        shelf: Shelf,
        lead: Vec<MenuItem>,
        cx: &App,
    ) -> Menu {
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
                .items(lead)
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
                    menu.items(lead)
                        .item(new_playlist)
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
                            .checked(held)
                            .when(!album, MenuItem::stays_open);
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
                            item.on_click(move |_, _, cx| {
                                if held {
                                    library.update(cx, |library, cx| {
                                        library.remove_tracks_from_playlist(
                                            playlist_id.clone(),
                                            track_ids.clone(),
                                            cx,
                                        )
                                    });
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
        let held: Vec<String> = tracks.iter().filter_map(|track| track.id.clone()).collect();
        let imported = !held.is_empty() && held.iter().all(|id| music::is_local_id(id));
        let ids: Vec<String> = match imported {
            true => held,
            false => held
                .into_iter()
                .filter(|id| !music::is_local_id(id))
                .collect(),
        };
        let shelf = match imported {
            true => Shelf::Local,
            false => Shelf::Streaming,
        };
        let copy = (!many)
            .then(|| copy_link_item("copy-track-link", MediaKind::Track, track.id.as_deref(), cx));
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
            QueuePlacement::Next,
        );
        let queue = queue_item(
            "add-to-queue",
            counted("menu-add-to-queue", "menu-add-tracks-to-queue", count),
            "icons/list-plus.svg",
            &queued,
            QueuePlacement::End,
        );
        let last = queue_item(
            "play-last",
            counted("menu-play-last", "menu-play-tracks-last", count),
            "icons/list-end.svg",
            &queued,
            QueuePlacement::Last,
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

        let add_to_playlist = match ids.is_empty() {
            true => None,
            false => Some(
                MenuItem::new(
                    "add-to-playlist",
                    counted("menu-add-to-playlist", "menu-add-tracks-to-playlist", count),
                )
                .icon("icons/square-plus.svg")
                .submenu(
                    self.playlist_rows(Addition::Tracks(ids.clone()), shelf, Vec::new(), cx),
                    self.playlist_submenu.clone(),
                ),
            ),
        };
        let tiles = track_tiles(tracks, &ids, shelf, imported, pinnable, cx);

        sections(
            item_shell("track-context-menu", tiles)
                .relative()
                .when(many, |menu| menu.min_w(SELECTION_MENU_WIDTH)),
            vec![
                add_to_playlist.into_iter().chain(library_action).collect(),
                vec![next, queue, last],
                album.into_iter().chain(artist).chain(radio).collect(),
                details
                    .into_iter()
                    .chain(edit)
                    .chain(copy)
                    .chain(delete_files)
                    .collect(),
                trailing.into_iter().collect(),
            ],
        )
    }
}

/// A queue item for the playable tracks of a selection. It is disabled when there are none, and
/// picking it uses the same placement path for one song and several.
fn queue_item(
    id: &'static str,
    label: SharedString,
    icon: &'static str,
    queued: &[Track],
    placement: QueuePlacement,
) -> MenuItem {
    let item = MenuItem::new(id, label).icon(icon);
    if queued.is_empty() {
        return item.disabled();
    }
    let queued = queued.to_vec();
    item.on_click(move |_, _, cx| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| {
            playback.enqueue_tracks(queued.clone(), None, placement, cx);
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

/// Joins the non-empty groups of a menu with a separator between each.
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

/// Starts the menu of one song, album, artist or playlist at the shared width, with `tiles` on
/// top when there are any.
fn item_shell(id: &'static str, tiles: Vec<Tile>) -> Menu {
    let menu = Menu::new(id).min_w(ITEM_MENU_WIDTH);
    match tiles.is_empty() {
        true => menu,
        false => menu.header(Tiles(tiles)),
    }
}

/// One toggle of the tile row on top of an item menu. A tile with no click is drawn disabled.
struct Tile {
    id: &'static str,
    icon: &'static str,
    /// The Fluent key of the tooltip, which is also the name a screen reader announces.
    tip: &'static str,
    active: bool,
    /// Draws a download ring instead of the icon.
    ring: Option<Ring>,
    /// Closes the menu before the click runs, for a click that opens a dialog.
    closes: bool,
    on_click: Option<TileClick>,
}

type TileClick = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

/// A download ring drawn in place of a tile's icon, `done` of the way round or turning when the
/// amount is unknown. A failed download draws it in the danger color.
#[derive(Clone, Copy)]
struct Ring {
    done: Option<f32>,
    failed: bool,
}

impl Tile {
    fn new(id: &'static str, icon: &'static str, tip: &'static str) -> Self {
        Self {
            id,
            icon,
            tip,
            active: false,
            ring: None,
            closes: false,
            on_click: None,
        }
    }

    fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    fn ring(mut self, done: Option<f32>, failed: bool) -> Self {
        self.ring = Some(Ring { done, failed });
        self
    }

    fn closes(mut self, closes: bool) -> Self {
        self.closes = closes;
        self
    }

    fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl FluentBuilder for Tile {}

/// The row of toggles on top of an item menu: the favorite, the library where it is apart from
/// the favorites, the download and the pin. The tiles share the row evenly, only a tile that is on
/// has a fill, and they leave the menu open when picked unless they hand off to a dialog.
#[derive(IntoElement)]
struct Tiles(Vec<Tile>);

impl RenderOnce for Tiles {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let tucked = ui::tucked(theme.radius, window);
        div()
            .flex()
            .flex_row()
            .w_full()
            .gap_1()
            .children(self.0.into_iter().map(|tile| {
                let Tile {
                    id,
                    icon,
                    tip,
                    active,
                    ring,
                    closes,
                    on_click,
                } = tile;
                Button::new(id)
                    .ghost()
                    .flex_1()
                    .min_w_0()
                    .rounded(tucked)
                    .tooltip_above(tip)
                    .toggled(active)
                    .when(active, |button| {
                        button.fill(theme.secondary_active, theme.secondary_hover)
                    })
                    .tint(match active {
                        true => theme.primary,
                        false => theme.popover_foreground,
                    })
                    .disabled(on_click.is_none())
                    .map(|button| match ring {
                        Some(Ring { done, failed }) => button.leading(
                            ArrowRing::new("tile-download-ring", done).color(match failed {
                                true => theme.danger,
                                false => theme.primary,
                            }),
                        ),
                        None => button.icon(icon),
                    })
                    .when_some(on_click, |button, click| {
                        button.on_click(move |_, window, cx| {
                            if closes {
                                ui::close_menu(window, cx);
                            }
                            click(window, cx);
                        })
                    })
            }))
    }
}

/// The pin tile, lit while the item is pinned. It is disabled for an item whose provider the
/// app cannot name again after a restart.
fn pin_tile(pin: Pin, cx: &App) -> Tile {
    let pins = Sonora::global(cx).pins.clone();
    let known = Sonora::global(cx)
        .session
        .read(cx)
        .slug_for(&pin.id)
        .is_some();
    let pinned = pins.read(cx).holds(&pin, cx);
    Tile::new(
        "tile-pin",
        "icons/pin.svg",
        match pinned {
            true => "nav-unpin",
            false => "nav-pin",
        },
    )
    .active(pinned)
    .when(known, |tile| {
        tile.on_click(move |_, cx| pins.update(cx, |pins, cx| pins.toggle(pin.clone(), cx)))
    })
}

/// The heart tile, filled and lit while `saved`.
fn heart_tile(saved: bool) -> Tile {
    Tile::new(
        "tile-favorite",
        match saved {
            true => "icons/heart-filled.svg",
            false => "icons/heart.svg",
        },
        match saved {
            true => "menu-remove-from-library",
            false => "menu-add-to-library",
        },
    )
    .active(saved)
}

/// The tiles of a track menu. Several tracks are favorited unless every one of them is already,
/// in which case they are all taken out, and the library and download tiles work the same way.
fn track_tiles(
    tracks: &[Track],
    ids: &[String],
    shelf: Shelf,
    imported: bool,
    pin: Option<Pin>,
    cx: &App,
) -> Vec<Tile> {
    let sonora = Sonora::global(cx);
    let library = sonora.library.clone();
    let editable = sonora.session.read(cx).shape_of(shelf).editable();
    let state = library.read(cx);
    let actionable = |pending: fn(&Library, &str) -> bool| -> Vec<Track> {
        tracks
            .iter()
            .filter(|track| track.id.as_deref().is_some_and(|id| !pending(state, id)))
            .cloned()
            .collect()
    };
    // A pending change already shows in the library, so the tiles light up for where the
    // tracks are headed and are only disabled until it lands, the way the track rows do.
    let named: Vec<&str> = tracks
        .iter()
        .filter_map(|track| track.id.as_deref())
        .collect();
    let every = |test: fn(&Library, &str) -> bool| {
        !named.is_empty() && named.iter().all(|id| test(state, id))
    };

    let favorable = actionable(Library::pending);
    let saved = every(Library::saved);
    // Unfavoriting asks first where the heart is the library itself, and the menu gives way to
    // that question rather than staying open over it.
    let asks = saved
        && !favorable
            .first()
            .and_then(|track| track.id.as_deref())
            .is_some_and(|id| Confirm::unstarring(id, cx));
    let heart = heart_tile(saved)
        .closes(asks)
        .when(!favorable.is_empty(), |tile| {
            let library = library.clone();
            tile.on_click(move |_, cx| {
                if saved {
                    Confirm::library_songs(favorable.clone(), cx);
                    return;
                }
                library.update(cx, |library, cx| {
                    let tracks = favorable
                        .iter()
                        .filter(|track| !track.id.as_deref().is_some_and(|id| library.saved(id)))
                        .cloned()
                        .collect();
                    library.save_tracks(tracks, true, cx);
                });
            })
        });

    let shelved = editable.then(|| {
        let held = actionable(Library::pending_library);
        let present = every(Library::in_library);
        Tile::new("tile-library", "icons/library-big.svg", "menu-save-library")
            .active(present)
            .when(!held.is_empty(), |tile| {
                let library = library.clone();
                tile.on_click(move |_, cx| {
                    library.update(cx, |library, cx| {
                        for track in held.clone() {
                            let there =
                                track.id.as_deref().is_some_and(|id| library.in_library(id));
                            if there == present {
                                library.set_track_in_library(track, !present, cx);
                            }
                        }
                    });
                })
            })
    });

    let download = (!ids.is_empty() && !imported)
        .then(|| track_download_tile(tracks, ids, cx))
        .flatten();

    [heart]
        .into_iter()
        .chain(shelved)
        .chain(download)
        .chain(pin.map(|pin| pin_tile(pin, cx)))
        .collect()
}

/// The download tile, drawn the way a detail page's download button is. It shows the plain
/// arrow until the download is asked for, then a ring that fills as it arrives, and a red ring
/// once it failed. Picking it starts the download, or retries a failed one, and once the
/// download is under way or in, picking it stops it and removes what arrived.
fn download_tile(
    progress: Option<Progress>,
    done: Option<f32>,
    start: impl Fn(&mut App) + 'static,
    remove: impl Fn(&mut App) + 'static,
) -> Tile {
    let tile = Tile::new(
        "tile-download",
        "icons/download.svg",
        match progress {
            None | Some(Progress::Failed) => "menu-download",
            Some(Progress::Waiting | Progress::Running) => "menu-downloading",
            Some(Progress::Done) => "menu-remove-download",
        },
    );
    match progress {
        None => tile.on_click(move |_, cx| start(cx)),
        Some(Progress::Failed) => tile
            .ring(Some(done.unwrap_or(0.)), true)
            .on_click(move |_, cx| start(cx)),
        Some(Progress::Waiting | Progress::Running) => tile
            .active(true)
            .ring(done, false)
            .on_click(move |_, cx| remove(cx)),
        Some(Progress::Done) => tile
            .active(true)
            .ring(Some(1.), false)
            .on_click(move |_, cx| remove(cx)),
    }
}

/// The download tile of a track selection. The selection reads as downloading while any track
/// is on its way, as in once every track is, and as failed when a track failed and none is
/// left running. Nothing shows for a provider that cannot download, unless something is there
/// to remove.
fn track_download_tile(tracks: &[Track], ids: &[String], cx: &App) -> Option<Tile> {
    let sonora = Sonora::global(cx);
    let downloads = sonora.downloads.clone();
    let state = downloads.read(cx);
    let states: Vec<Option<Progress>> = ids.iter().map(|id| state.track(id)).collect();
    let progress = match () {
        () if states.iter().all(|state| *state == Some(Progress::Done)) => Some(Progress::Done),
        () if states.contains(&Some(Progress::Running)) => Some(Progress::Running),
        () if states.contains(&Some(Progress::Waiting)) => Some(Progress::Waiting),
        () if states.contains(&Some(Progress::Failed)) => Some(Progress::Failed),
        () => None,
    };
    if progress.is_none() && !sonora.session.read(cx).capabilities().downloads {
        return None;
    }
    let done = match ids {
        [id] => state.fraction(id),
        _ => {
            let arrived: f32 = ids
                .iter()
                .map(|id| match state.has(id) {
                    true => 1.,
                    false => state.fraction(id).unwrap_or(0.),
                })
                .sum();
            Some(arrived / ids.len() as f32)
        }
    };
    let tracks = tracks.to_vec();
    let ids = ids.to_vec();
    let removing = downloads.clone();
    Some(download_tile(
        progress,
        done,
        move |cx| {
            downloads.update(cx, |downloads, cx| {
                downloads.download_tracks(tracks.clone(), cx)
            })
        },
        move |cx| removing.update(cx, |downloads, cx| downloads.remove_tracks(&ids, cx)),
    ))
}

/// The download tile of a whole album or playlist.
fn set_download_tile(saved: Saved, cx: &App) -> Option<Tile> {
    let (kind, id) = match &saved {
        Saved::Album(album) => (Collection::Album, album.id.clone()),
        Saved::Playlist(playlist) => (Collection::Playlist, playlist.id.clone()),
    };
    if music::is_local_id(&id) {
        return None;
    }
    let sonora = Sonora::global(cx);
    let downloads = sonora.downloads.clone();
    let progress = downloads.read(cx).set(kind, &id);
    if progress.is_none() && !sonora.session.read(cx).capabilities().downloads {
        return None;
    }
    let done = Some(downloads.read(cx).share(kind, &id));
    let removing = downloads.clone();
    Some(download_tile(
        progress,
        done,
        move |cx| {
            downloads.update(cx, |downloads, cx| match &saved {
                Saved::Album(album) => downloads.download_album_by_id(album, cx),
                Saved::Playlist(playlist) => downloads.download_playlist_by_id(playlist, cx),
            })
        },
        move |cx| removing.update(cx, |downloads, cx| downloads.remove_set(kind, &id, cx)),
    ))
}

/// The tiles of an album menu: the favorite, the library where it is apart from the favorites,
/// the download and the pin.
fn album_tiles(album: &Album, cx: &App) -> Vec<Tile> {
    let sonora = Sonora::global(cx);
    let library = sonora.library.clone();
    let state = library.read(cx);
    let saved = state.saved_album(&album.id);
    let heart = heart_tile(saved)
        .closes(saved && !Confirm::unstarring(&album.id, cx))
        .when(!state.pending_album(&album.id), |tile| {
            let album = album.clone();
            tile.on_click(move |_, cx| match saved {
                true => Confirm::albums(vec![album.clone()], cx),
                false => {
                    let library = Sonora::global(cx).library.clone();
                    library.update(cx, |library, cx| library.toggle_album(album.clone(), cx));
                }
            })
        });

    let editable = sonora
        .session
        .read(cx)
        .shape_of(Shelf::of(&album.id))
        .editable();
    let shelved = editable.then(|| {
        let present = state.in_library(&album.id);
        Tile::new("tile-library", "icons/library-big.svg", "menu-save-library")
            .active(present)
            .when(!state.pending_library(&album.id), |tile| {
                let album = album.clone();
                let library = library.clone();
                tile.on_click(move |_, cx| {
                    library.update(cx, |library, cx| {
                        library.set_album_in_library(album.clone(), !present, cx)
                    });
                })
            })
    });

    [heart]
        .into_iter()
        .chain(shelved)
        .chain(set_download_tile(Saved::Album(album.clone()), cx))
        .chain(album.pin().map(|pin| pin_tile(pin, cx)))
        .collect()
}

/// An album's menu, laid out like a track's: the tiles on top, then Add to playlist, the queue
/// and the link. `menus` holds the playlist submenu's state.
pub(crate) fn album_menu(
    album: Album,
    playback: Entity<Playback>,
    menus: &ItemMenu,
    cx: &App,
) -> Menu {
    let album_id = album.id.clone();
    let from = Origin::album(album_id.clone()).named(album.name.clone());
    let add_to_playlist = MenuItem::new("add-album-to-playlist", t!("menu-add-to-playlist"))
        .icon("icons/square-plus.svg")
        .submenu(
            menus.playlist_rows(
                Addition::Album(album_id.clone()),
                Shelf::of(&album_id),
                Vec::new(),
                cx,
            ),
            menus.playlist_submenu.clone(),
        );

    sections(
        item_shell("album-context-menu", album_tiles(&album, cx)),
        vec![
            vec![add_to_playlist],
            collection_queue_items(from, playback),
            vec![copy_link_item(
                "copy-album-link",
                MediaKind::Album,
                Some(&album_id),
                cx,
            )],
        ],
    )
}

/// The tiles of an artist menu: the follow heart where the provider can follow artists, and the
/// pin.
fn artist_tiles(artist: &SavedArtist, cx: &App) -> Vec<Tile> {
    let sonora = Sonora::global(cx);
    let follow = sonora
        .session
        .read(cx)
        .capabilities()
        .follow_artists
        .then(|| {
            let state = sonora.library.read(cx);
            let saved = state.saved_artist(&artist.id);
            heart_tile(saved)
                .closes(saved && !Confirm::unstarring(&artist.id, cx))
                .when(!state.pending_artist(&artist.id), |tile| {
                    let artist = artist.clone();
                    tile.on_click(move |_, cx| match saved {
                        true => Confirm::artists(vec![artist.clone()], cx),
                        false => {
                            let library = Sonora::global(cx).library.clone();
                            library.update(cx, |library, cx| {
                                library.toggle_artist(artist.clone(), cx)
                            });
                        }
                    })
                })
        });
    follow
        .into_iter()
        .chain(artist.pin().map(|pin| pin_tile(pin, cx)))
        .collect()
}

/// An artist's menu: the tiles on top, then the artist page unless it is open already, playing
/// and queueing the artist, and the link.
pub(crate) fn artist_menu(
    artist: SavedArtist,
    playback: Entity<Playback>,
    opened_here: bool,
    cx: &App,
) -> Menu {
    let artist_id = artist.id.clone();
    let opened = artist_id.clone();
    let played = Origin::artist(artist_id.clone()).named(artist.name.clone());
    let queued_from = played.clone();
    let playing = playback.clone();

    let open = match opened_here {
        true => Vec::new(),
        false => vec![
            MenuItem::new("open-artist", t!("menu-go-to-artist"))
                .icon("icons/info.svg")
                .on_click(move |_, _, cx| navigate(Destination::Artist(opened.clone().into()), cx)),
        ],
    };

    sections(
        item_shell("artist-context-menu", artist_tiles(&artist, cx)),
        vec![
            open,
            vec![
                MenuItem::new("play-artist", t!("menu-play-artist"))
                    .icon("icons/play.svg")
                    .on_click(move |_, _, cx| {
                        playing.update(cx, |playback, cx| playback.play_origin(played.clone(), cx));
                    }),
            ]
            .into_iter()
            .chain(collection_queue_items(queued_from, playback))
            .collect(),
            vec![copy_link_item(
                "copy-artist-link",
                MediaKind::Artist,
                Some(&artist_id),
                cx,
            )],
        ],
    )
}

/// The tiles of a playlist menu: the heart that keeps someone else's playlist in the library,
/// the download and the pin.
fn playlist_tiles(playlist: &Playlist, cx: &App) -> Vec<Tile> {
    let kept = (!playlist.owned).then(|| {
        let saved = Sonora::global(cx)
            .library
            .read(cx)
            .playlist(&playlist.id)
            .is_some();
        let playlist = playlist.clone();
        Tile::new(
            "tile-favorite",
            match saved {
                true => "icons/heart-filled.svg",
                false => "icons/heart.svg",
            },
            match saved {
                true => "menu-remove-playlist-from-library",
                false => "menu-add-playlist-to-library",
            },
        )
        .active(saved)
        .closes(saved)
        .on_click(move |_, cx| match saved {
            true => Confirm::playlists(vec![playlist.id.clone()], cx),
            false => {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| {
                    library.add_playlist_to_library(playlist.clone(), cx)
                });
            }
        })
    });
    kept.into_iter()
        .chain(set_download_tile(Saved::Playlist(playlist.clone()), cx))
        .chain(playlist.pin().map(|pin| pin_tile(pin, cx)))
        .collect()
}

/// A playlist's menu: the tiles on top, then the queue, the owner's edits and the link.
pub(crate) fn playlist_menu(playlist: Playlist, playback: Entity<Playback>, cx: &App) -> Menu {
    let from = Origin::playlist(playlist.id.clone()).named(playlist.name.clone());
    let copied = playlist.id.clone();
    let id = playlist.id.clone();
    let public = playlist.public;
    let imported = music::is_local_id(&playlist.id);
    let tiles = playlist_tiles(&playlist, cx);
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
                MenuItem::new("edit-playlist", t!("menu-edit-playlist"))
                    .icon("icons/pencil.svg")
                    .on_click({
                        let playlist = playlist.clone();
                        move |_, window, cx| {
                            PlaylistEditor::open(Edit::Details(playlist.clone()), window, cx);
                        }
                    }),
                MenuItem::new("delete-playlist", t!("menu-delete-playlist"))
                    .icon("icons/trash-2.svg")
                    .on_click({
                        let playlist = playlist.clone();
                        move |_, window, cx| {
                            PlaylistEditor::open(Edit::Delete(playlist.clone()), window, cx);
                        }
                    }),
            ])
            .chain(imported.then(|| {
                MenuItem::new("export-playlist", t!("menu-export-playlist"))
                    .icon("icons/square-arrow-out-up-right.svg")
                    .on_click(move |_, _, cx| local::export_playlist(playlist.clone(), cx))
            }))
            .collect(),
        false => Vec::new(),
    };

    sections(
        item_shell("playlist-context-menu", tiles),
        vec![
            collection_queue_items(from, playback),
            actions,
            match imported {
                true => Vec::new(),
                false => vec![copy_link_item(
                    "copy-playlist-link",
                    MediaKind::Playlist,
                    Some(&copied),
                    cx,
                )],
            },
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
    };

    built.unwrap_or_else(|| sparse_menu(pin, playback, cx))
}

pub(crate) fn pinned_artist(pin: &Pin) -> SavedArtist {
    SavedArtist {
        id: pin.id.clone(),
        name: pin.title.clone(),
        cover: pin.cover.clone(),
        ..Default::default()
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

    sections(
        Menu::new("pin-context-menu").min_w(ITEM_MENU_WIDTH),
        vec![
            open.into_iter().collect(),
            transport_items(pin, playback),
            vec![copy_link_item("copy-pin-link", kind, Some(&copied), cx)],
            vec![pin_action(pin, cx)],
        ],
    )
}

/// The entry that opens a pin's page, which only an artist's and a song's menus carry, the
/// way their full menus do.
fn open_key(kind: PinKind) -> Option<&'static str> {
    match kind {
        PinKind::Album | PinKind::Playlist => None,
        PinKind::Artist => Some("menu-go-to-artist"),
        PinKind::Song => Some("menu-view-details"),
    }
}

/// The three queue placements share both their labels and their loading path.
fn collection_queue_items(origin: Origin, playback: Entity<Playback>) -> Vec<MenuItem> {
    [
        (
            "play-collection-next",
            "menu-play-next",
            "icons/list-start.svg",
            QueuePlacement::Next,
        ),
        (
            "enqueue-collection",
            "menu-add-to-queue",
            "icons/list-plus.svg",
            QueuePlacement::End,
        ),
        (
            "play-collection-last",
            "menu-play-last",
            "icons/list-end.svg",
            QueuePlacement::Last,
        ),
    ]
    .into_iter()
    .map(|(id, label, icon, placement)| {
        let origin = origin.clone();
        let playback = playback.clone();
        MenuItem::new(id, i18n::lookup(label, None))
            .icon(icon)
            .on_click(move |_, _, cx| {
                playback.update(cx, |playback, cx| {
                    playback.enqueue_collection(origin.clone(), placement, cx)
                });
            })
    })
    .collect()
}

fn transport_items(pin: &Pin, playback: Entity<Playback>) -> Vec<MenuItem> {
    let played = Origin::from(pin);
    let mut items = match pin.kind {
        PinKind::Song | PinKind::Artist => {
            let playing = playback.clone();
            let origin = played.clone();
            let (label, icon) = match pin.kind {
                PinKind::Song => ("menu-song-radio", "icons/radio.svg"),
                _ => ("menu-play-artist", "icons/play.svg"),
            };
            vec![
                MenuItem::new("play-pin", i18n::lookup(label, None))
                    .icon(icon)
                    .on_click(move |_, _, cx| {
                        playing.update(cx, |playback, cx| playback.play_origin(origin.clone(), cx));
                    }),
            ]
        }
        _ => Vec::new(),
    };
    if pin.kind != PinKind::Song {
        items.extend(collection_queue_items(played, playback));
    }
    items
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

fn media_kind(kind: PinKind) -> MediaKind {
    match kind {
        PinKind::Album => MediaKind::Album,
        PinKind::Artist => MediaKind::Artist,
        PinKind::Playlist => MediaKind::Playlist,
        PinKind::Song => MediaKind::Track,
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

/// The Copy link item. The link is made while the menu is built, so an item with no link shows
/// up disabled and says why on hover.
fn copy_link_item(id: &'static str, kind: MediaKind, target: Option<&str>, cx: &App) -> MenuItem {
    let item = MenuItem::new(id, t!("menu-copy-link")).icon("icons/link.svg");
    match share_link(kind, target, cx) {
        Ok(url) => item.on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
            Toasts::show(Outcome::Done, "toast-link-copied", cx);
        }),
        Err(reason) => item.disabled().tooltip(reason),
    }
}

/// The link that shares `id`, or the Fluent key of the reason there is none.
fn share_link(kind: MediaKind, id: Option<&str>, cx: &App) -> Result<String, &'static str> {
    let Some(id) = id else {
        return Err("menu-copy-link-none");
    };
    let local = music::is_local_id(id);
    let session = Sonora::global(cx).session.read(cx);
    let client = match local {
        true => session.local_client(),
        false => session.client(),
    };
    client
        .and_then(|client| client.share_url(kind, id))
        .ok_or(match local {
            true => "menu-copy-link-local",
            false => "menu-copy-link-none",
        })
}

/// The menu behind the playlists of a library page. A local shelf also offers to import an
/// M3U file.
pub(crate) fn new_playlist_menu(
    shelf: Shelf,
    on_create: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Menu {
    Menu::new("playlist-background-menu")
        .item(
            MenuItem::new("create-playlist", t!("menu-new-playlist"))
                .icon("icons/plus.svg")
                .on_click(on_create),
        )
        .items(shelf.local().then(|| {
            MenuItem::new("import-playlist", t!("menu-import-playlist"))
                .icon("icons/file-music.svg")
                .on_click(|_, _, cx| local::import_playlist(cx))
        }))
}

/// The whole album or playlist a download tile acts on.
enum Saved {
    Album(Album),
    Playlist(Playlist),
}
