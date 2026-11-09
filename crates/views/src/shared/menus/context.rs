//! The context menus of every song, album, artist, playlist and pin, wherever they were opened
//! from: a row of toggles on top (the favorite, the library, the download, the pin), then the
//! playlists, the queue, the pages to go to and the link, in groups split by separators.
//!
//! One menu is open at a time and its state lives in [`Menus`], which the workspace draws over
//! everything: a screen only calls [`open`] (or hands a card [`super::opener`]) with what the menu
//! is for and where it was asked for.

use std::rc::Rc;

use i18n::t;
use music::{Album, GenreItem, MediaKind, Playlist, SavedArtist, Track};
use router::Destination;
use rune::{El, PointerPress, Tx, row};
use state::{Addition, Collection, Library, Origin, Outcome, Progress, QueuePlacement, Shelf};
use ui::{ArrowRing, Button, Cover, Menu, MenuItem, MenuSearch, Pin, PinKind, Placement, Theme};

use crate::shared::confirm;
use crate::shared::dialogs::rems;
use crate::shared::pins::{Pinned as _, destination};
use crate::shared::playlist_editor::{self, Edit};
use crate::shared::tag_editor;
use crate::{Ui, act, act_tx};

/// The narrowest the menu of one song, album, artist or playlist gets, which is wide enough for
/// English. A locale with longer labels widens the menu to fit them.
const ITEM_MENU_WIDTH: f32 = 196.;
/// How narrow a menu over several selected tracks can get, since its labels carry the count.
const SELECTION_MENU_WIDTH: f32 = 248.;
/// How wide the playlist and artist submenus are, and how tall before they scroll.
const SUBMENU_WIDTH: f32 = 220.;
const SUBMENU_HEIGHT: f32 = 360.;
/// The playlist cover drawn before each playlist's name.
const ART: f32 = 20.;
/// The key the open menu is mounted under.
const KEY: &str = "item-menu";

/// What a context menu is open on.
#[derive(Clone)]
pub(crate) enum Target {
    Album(Album),
    Playlist(Playlist),
    /// An artist, and whether its own page is the one showing, which leaves out Go to artist.
    Artist {
        artist: SavedArtist,
        here: bool,
    },
    /// One song or a selection, the list it was picked in and which columns that list shows.
    Tracks {
        tracks: Vec<Track>,
        list: TrackList,
        columns: TrackColumns,
    },
    /// A pin in the sidebar, whose item the library may or may not hold.
    Pin(Pin),
    /// The background of a library page's playlists: New playlist, and Import on a local shelf.
    NewPlaylist(Shelf),
}

impl Target {
    /// The item behind a shelf card; a genre has no menu.
    pub(crate) fn of(item: &GenreItem) -> Option<Self> {
        match item {
            GenreItem::Album(album) => Some(Self::Album(album.clone())),
            GenreItem::Playlist(playlist) => Some(Self::Playlist(playlist.clone())),
            GenreItem::Artist(artist) => Some(Self::artist(artist.clone())),
            GenreItem::Track(track) => Some(Self::track(track.clone())),
            GenreItem::Genre(_) => None,
        }
    }

    /// One song, from a list that shows neither its album nor its artists.
    pub(crate) fn track(track: Track) -> Self {
        Self::Tracks {
            tracks: vec![track],
            list: TrackList::Plain,
            columns: TrackColumns::default(),
        }
    }

    /// An artist opened from anywhere but its own page.
    pub(crate) fn artist(artist: SavedArtist) -> Self {
        Self::Artist {
            artist,
            here: false,
        }
    }
}

/// The list a track menu was opened in, which adds the entries only that list has.
#[derive(Clone, Default)]
pub(crate) enum TrackList {
    #[default]
    Plain,
    /// An album's own track list, by the album's id: Go to album is left out.
    Album(String),
    /// The playlist page's list: Remove from playlist is offered.
    Playlist,
    /// The history: Remove from history is offered.
    History,
}

/// Which columns the list beside the menu already shows, so the menu leaves out the pages they
/// link to.
#[derive(Clone, Copy, Default)]
pub(crate) struct TrackColumns {
    pub album: bool,
    pub artists: bool,
}

/// Where a menu opens.
#[derive(Clone)]
pub(crate) enum Spot {
    /// At a window point, as a context menu opens where the secondary press landed.
    At(f32, f32),
    /// Under the end of the element keyed so, as a "more" button's menu hangs.
    Below(String),
}

impl From<PointerPress> for Spot {
    fn from(press: PointerPress) -> Self {
        Self::At(press.x, press.y)
    }
}

impl From<(f32, f32)> for Spot {
    fn from((x, y): (f32, f32)) -> Self {
        Self::At(x, y)
    }
}

/// The open context menu and its submenus' state.
#[derive(Default)]
pub(crate) struct Menus {
    open: Option<Opened>,
    playlists: bool,
    artists: bool,
    search: MenuSearch,
}

struct Opened {
    target: Target,
    spot: Spot,
}

impl Menus {
    /// Whether a context menu is open.
    pub(crate) fn shows(&self) -> bool {
        self.open.is_some()
    }

    /// Opens the menu of `target` at `spot`, with its submenus shut and its search empty.
    pub(crate) fn open(&mut self, target: Target, spot: impl Into<Spot>) {
        self.playlists = false;
        self.artists = false;
        self.search.reset();
        self.open = Some(Opened {
            target,
            spot: spot.into(),
        });
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
        self.playlists = false;
        self.artists = false;
    }
}

/// Opens the menu of `target` at `spot`.
pub(crate) fn open(ui: &mut Ui, target: Target, spot: impl Into<Spot>) {
    ui.dialogs.menu.open(target, spot);
}

/// Opens the menu of one song or a selection at `spot`.
pub(crate) fn open_tracks(
    ui: &mut Ui,
    tracks: Vec<Track>,
    list: TrackList,
    columns: TrackColumns,
    spot: impl Into<Spot>,
) {
    if tracks.is_empty() {
        return;
    }
    open(
        ui,
        Target::Tracks {
            tracks,
            list,
            columns,
        },
        spot,
    );
}

/// Closes the open context menu.
pub(crate) fn close(ui: &mut Ui) {
    ui.dialogs.menu.close();
}

/// A secondary-press handler that opens the menu of `target` where the press landed.
pub(crate) fn at(target: Target) -> impl Fn(&mut Ui, PointerPress, &mut Tx) + 'static {
    move |ui: &mut Ui, press: PointerPress, _: &mut Tx| open(ui, target.clone(), press)
}

/// A press handler that opens the menu of `target` under the element keyed `trigger`.
pub(crate) fn below(target: Target, trigger: impl Into<String>) -> impl Fn(&mut Ui) + 'static {
    let trigger = trigger.into();
    move |ui: &mut Ui| open(ui, target.clone(), Spot::Below(trigger.clone()))
}

/// Mounts the open context menu, if any, into `layer`.
pub(crate) fn view(ui: &Ui, layer: &mut Vec<El>) {
    let Some(opened) = &ui.dialogs.menu.open else {
        return;
    };
    let theme = &ui.theme;
    let menu = menu(ui, &opened.target);
    match &opened.spot {
        Spot::At(x, y) => ui::Popup::new(KEY, (*x, *y), menu)
            .on_close(close)
            .build(theme, layer),
        Spot::Below(trigger) => menu
            .anchor(trigger.clone(), Placement::BottomEnd, ui::TRIGGER_GAP)
            .on_dismiss(close)
            .on_pick(|ui: &mut Ui, _: &mut Tx| close(ui))
            .build(theme, layer),
    }
}

/// The menu of `target`, for a caller that mounts it itself (in a `ui::Popover`, say). Its
/// submenus keep their state in [`Menus`], so only one such menu should be open at a time.
pub(crate) fn menu(ui: &Ui, target: &Target) -> Menu<Ui> {
    let menu = match target {
        Target::Album(album) => album_menu(ui, album),
        Target::Playlist(playlist) => playlist_menu(ui, playlist),
        Target::Artist { artist, here } => artist_menu(ui, artist, *here),
        Target::Tracks {
            tracks,
            list,
            columns,
        } => track_menu(ui, tracks, list, *columns),
        Target::Pin(pin) => pin_menu(ui, pin),
        Target::NewPlaylist(shelf) => new_playlist_menu(*shelf),
    };
    // A screen reader hears the menu named after what it acts on.
    let name = match target {
        Target::Album(album) => Some(album.name.clone()),
        Target::Playlist(playlist) => Some(playlist.name.clone()),
        Target::Artist { artist, .. } => Some(artist.name.clone()),
        Target::Tracks { tracks, .. } => match tracks.as_slice() {
            [track] => Some(track.name.clone()),
            _ => None,
        },
        Target::Pin(pin) => Some(pin.title.clone()),
        Target::NewPlaylist(_) => None,
    };
    match name {
        Some(name) => menu.name(name),
        None => menu,
    }
}

/// The menu of one song or a selection: the tiles, Add to playlist and the list's own action,
/// the three queue placements, the album, artist and radio, the details, tags, link and file,
/// and the list's own removal last.
fn track_menu(ui: &Ui, tracks: &[Track], list: &TrackList, columns: TrackColumns) -> Menu<Ui> {
    let Some(track) = tracks.first() else {
        return Menu::new("track-context-menu");
    };
    let store = &ui.store;
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
        .then(|| copy_link_item(ui, "copy-track-link", MediaKind::Track, track.id.as_deref()));
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
    let stations = store.session.capabilities().radio;
    let radio_item = || MenuItem::new("song-radio", t!("menu-song-radio")).icon("icons/radio.svg");
    let radio = match (many || !stations, track.id.is_some() && track.playable) {
        (true, _) => None,
        (false, true) => {
            let seed = track.clone();
            Some(radio_item().on_press(act(move |ui: &mut Ui| ui.store.play_radio(&seed))))
        }
        (false, false) => Some(radio_item().disabled()),
    };
    let current_album = match list {
        TrackList::Album(id) => Some(id.as_str()),
        _ => None,
    };
    let album_item =
        || MenuItem::new("go-to-album", t!("menu-go-to-album")).icon("icons/disc-3.svg");
    let album = match (many, columns.album, track.album_id.clone()) {
        (true, _, _) | (false, true, _) => None,
        (false, false, Some(id)) if Some(id.as_str()) == current_album => None,
        (false, false, Some(id)) => Some(album_item().on_press(go(Destination::Album(id.into())))),
        (false, false, None) => Some(album_item().disabled()),
    };

    let artists: Vec<(String, String)> = track
        .artist_refs
        .iter()
        .filter_map(|artist| Some((artist.name.clone(), artist.id.clone()?)))
        .collect();
    let artist_item =
        || MenuItem::new("go-to-artist", t!("menu-go-to-artist")).icon("icons/user.svg");
    let artist = match (many, columns.artists, artists.len()) {
        (true, _, _) | (false, true, _) => None,
        (false, false, 0) => Some(artist_item().disabled()),
        (false, false, 1) => {
            let id = artists[0].1.clone();
            Some(artist_item().on_press(go(Destination::Artist(id.into()))))
        }
        (false, false, _) => {
            let submenu = Menu::new("artist-submenu")
                .width(SUBMENU_WIDTH)
                .max_height(SUBMENU_HEIGHT)
                .items(artists.into_iter().map(|(name, id)| {
                    MenuItem::new(format!("artist-{id}"), name)
                        .on_press(go(Destination::Artist(id.into())))
                }));
            Some(artist_item().submenu(
                submenu,
                ui.dialogs.menu.artists,
                |ui: &mut Ui, open: bool| ui.dialogs.menu.artists = open,
            ))
        }
    };

    let details_item =
        || MenuItem::new("view-details", t!("menu-view-details")).icon("icons/info.svg");
    let details = match (many, track.id.clone()) {
        (true, _) => None,
        (false, Some(id)) => Some(details_item().on_press(go(Destination::Song(id.into())))),
        (false, None) => Some(details_item().disabled()),
    };

    let pinnable = (!many).then(|| track.pin()).flatten();
    let edit = (!many && imported).then(|| {
        let track = track.clone();
        MenuItem::new("edit-tags", t!("menu-edit-tags"))
            .icon("icons/pencil.svg")
            .on_press(act(move |ui: &mut Ui| tag_editor::open(ui, track.clone())))
    });

    let delete_files = imported.then(|| {
        let ids = ids.clone();
        MenuItem::new(
            "delete-track-files",
            counted("menu-delete-track-file", "menu-delete-track-files", count),
        )
        .icon("icons/trash-2.svg")
        .on_press(act(move |ui: &mut Ui| {
            confirm::delete_track_files(ui, ids.clone())
        }))
    });

    let add_to_playlist = (!ids.is_empty()).then(|| {
        MenuItem::new(
            "add-to-playlist",
            counted("menu-add-to-playlist", "menu-add-tracks-to-playlist", count),
        )
        .icon("icons/square-plus.svg")
        .submenu(
            playlist_rows(ui, Addition::Tracks(ids.clone()), shelf),
            ui.dialogs.menu.playlists,
            |ui: &mut Ui, open: bool| ui.dialogs.menu.playlists = open,
        )
    });

    let (library_action, trailing) = match list {
        TrackList::Playlist => {
            let label = counted(
                "menu-remove-from-playlist",
                "menu-remove-tracks-from-playlist",
                count,
            );
            let item = MenuItem::new("remove-from-playlist", label).icon("icons/x.svg");
            let named: Vec<String> = tracks.iter().filter_map(|track| track.id.clone()).collect();
            let item = match named.is_empty() {
                true => item.disabled(),
                false => item.on_press(act(move |ui: &mut Ui| {
                    confirm::playlist_songs(ui, named.clone(), count)
                })),
            };
            (Some(item), None)
        }
        TrackList::History => {
            let held = tracks.to_vec();
            let item = MenuItem::new(
                "remove-from-history",
                counted(
                    "menu-remove-from-history",
                    "menu-remove-tracks-from-history",
                    count,
                ),
            )
            .icon("icons/trash-2.svg")
            .on_press(act(move |ui: &mut Ui| {
                confirm::history_songs(ui, held.clone())
            }));
            (None, Some(item))
        }
        TrackList::Plain | TrackList::Album(_) => (None, None),
    };
    let tiles = track_tiles(ui, tracks, &ids, shelf, imported, pinnable);

    let mut shell = item_shell(ui, "track-context-menu", tiles);
    if many {
        shell = shell.min_width(SELECTION_MENU_WIDTH);
    }
    sections(
        shell,
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

/// The Save to submenu for `addition`: New playlist, then the playlists on `shelf` the listener
/// can add to, narrowed by the submenu's search. New playlist hides while a search narrows the
/// list. A playlist holding every named track is checked, and picking it takes the tracks out
/// again with the menu left open. An album is never checked, since its tracks are only read
/// once it is added, and picking a playlist for it closes the menu.
fn playlist_rows(ui: &Ui, addition: Addition, shelf: Shelf) -> Menu<Ui> {
    let theme = &ui.theme;
    let library = &ui.store.library;
    let playlists: Vec<&Playlist> = library
        .state(shelf)
        .playlists()
        .iter()
        .filter(|playlist| playlist.owned || playlist.collaborative)
        .collect();
    let ids = match &addition {
        Addition::Tracks(ids) => ids.clone(),
        Addition::Album(_) => Vec::new(),
    };
    let album = matches!(addition, Addition::Album(_));
    let created = addition.clone();
    let new_playlist = MenuItem::new("new-playlist", t!("menu-new-playlist"))
        .icon("icons/plus.svg")
        .on_press(act(move |ui: &mut Ui| {
            playlist_editor::open(
                ui,
                Edit::Create {
                    addition: created.clone(),
                    shelf,
                },
            )
        }));
    let base = Menu::new("playlist-submenu").width(SUBMENU_WIDTH);
    if playlists.is_empty() {
        return base
            .item(new_playlist)
            .item(MenuItem::separator("playlist-separator"))
            .item(MenuItem::new("no-playlists", t!("menu-no-playlists")).disabled());
    }
    let search = &ui.dialogs.menu.search;
    let query = search.query();
    let searching = !query.is_empty();
    let found: Vec<&Playlist> = playlists
        .into_iter()
        .filter(|playlist| playlist.name.to_lowercase().contains(&query))
        .collect();
    let mut menu = base.max_height(SUBMENU_HEIGHT).search(
        search,
        t!("menu-search-playlists"),
        |ui: &mut Ui| &mut ui.dialogs.menu.search,
    );
    if !searching {
        menu = menu
            .item(new_playlist)
            .item(MenuItem::separator("playlist-separator"));
    }
    if found.is_empty() {
        menu =
            menu.item(MenuItem::new("no-matching-playlists", t!("search-no-matches")).disabled());
    }
    menu.items(found.into_iter().map(|playlist| {
        let held = !ids.is_empty()
            && ids
                .iter()
                .all(|id| library.holds(&playlist.id, id).unwrap_or(false));
        let art = Cover::new(ui.artwork(), playlist.cover.as_deref())
            .size(ART)
            .radius(ui::tucked(theme.radius, theme.font_size))
            .fallback("icons/list-music.svg")
            .build(theme);
        let mut item = MenuItem::new(format!("playlist-{}", playlist.id), playlist.name.clone())
            .leading(art)
            .checked(held);
        let playlist_id = playlist.id.clone();
        if album {
            let addition = addition.clone();
            return item.on_press(act(move |ui: &mut Ui| {
                let store = &mut ui.store;
                store.library.add_tracks_to_playlist(
                    playlist_id.clone(),
                    addition.clone(),
                    &store.session,
                    &mut store.cx,
                );
            }));
        }
        item = item.stays_open();
        if ids.is_empty() {
            return item.disabled();
        }
        let track_ids = ids.clone();
        item.on_press(act(move |ui: &mut Ui| {
            let store = &mut ui.store;
            if held {
                store.library.remove_tracks_from_playlist(
                    playlist_id.clone(),
                    track_ids.clone(),
                    &store.session,
                    &mut store.cx,
                );
                return;
            }
            let missing: Vec<String> = track_ids
                .iter()
                .filter(|id| !store.library.holds(&playlist_id, id).unwrap_or(false))
                .cloned()
                .collect();
            if missing.is_empty() {
                return;
            }
            store.library.add_tracks_to_playlist(
                playlist_id.clone(),
                Addition::Tracks(missing),
                &store.session,
                &mut store.cx,
            );
        }))
    }))
}

/// An album's menu, laid out like a track's: the tiles on top, then Add to playlist, the queue
/// and the link.
fn album_menu(ui: &Ui, album: &Album) -> Menu<Ui> {
    let album_id = album.id.clone();
    let from = Origin::album(album_id.clone()).named(album.name.clone());
    let add_to_playlist = MenuItem::new("add-album-to-playlist", t!("menu-add-to-playlist"))
        .icon("icons/square-plus.svg")
        .submenu(
            playlist_rows(ui, Addition::Album(album_id.clone()), Shelf::of(&album_id)),
            ui.dialogs.menu.playlists,
            |ui: &mut Ui, open: bool| ui.dialogs.menu.playlists = open,
        );

    sections(
        item_shell(ui, "album-context-menu", album_tiles(ui, album)),
        vec![
            vec![add_to_playlist],
            collection_queue_items(from),
            vec![copy_link_item(
                ui,
                "copy-album-link",
                MediaKind::Album,
                Some(&album_id),
            )],
        ],
    )
}

/// An artist's menu: the tiles on top, then the artist page unless it is open already, playing
/// and queueing the artist, and the link.
fn artist_menu(ui: &Ui, artist: &SavedArtist, here: bool) -> Menu<Ui> {
    let artist_id = artist.id.clone();
    let played = Origin::artist(artist_id.clone()).named(artist.name.clone());
    let queued_from = played.clone();

    let open = match here {
        true => Vec::new(),
        false => vec![
            MenuItem::new("open-artist", t!("menu-go-to-artist"))
                .icon("icons/info.svg")
                .on_press(go(Destination::Artist(artist_id.clone().into()))),
        ],
    };

    sections(
        item_shell(ui, "artist-context-menu", artist_tiles(ui, artist)),
        vec![
            open,
            vec![
                MenuItem::new("play-artist", t!("menu-play-artist"))
                    .icon("icons/play.svg")
                    .on_press(act(move |ui: &mut Ui| {
                        ui.store.play_collection(played.clone())
                    })),
            ]
            .into_iter()
            .chain(collection_queue_items(queued_from))
            .collect(),
            vec![copy_link_item(
                ui,
                "copy-artist-link",
                MediaKind::Artist,
                Some(&artist_id),
            )],
        ],
    )
}

/// A playlist's menu: the tiles on top, then the queue, the owner's edits and the link.
fn playlist_menu(ui: &Ui, playlist: &Playlist) -> Menu<Ui> {
    let from = Origin::playlist(playlist.id.clone()).named(playlist.name.clone());
    let id = playlist.id.clone();
    let public = playlist.public;
    let imported = music::is_local_id(&playlist.id);
    let tiles = playlist_tiles(ui, playlist);
    let visibility = (!imported).then(|| {
        let id = id.clone();
        MenuItem::new(
            "playlist-visibility",
            match public {
                true => t!("menu-make-playlist-private"),
                false => t!("menu-make-playlist-public"),
            },
        )
        .icon("icons/user.svg")
        .on_press(act(move |ui: &mut Ui| {
            let store = &mut ui.store;
            store
                .library
                .set_playlist_public(id.clone(), !public, &store.session, &mut store.cx);
        }))
    });
    let actions = match playlist.owned {
        true => {
            let edited = playlist.clone();
            let deleted = playlist.clone();
            let exported = playlist.clone();
            visibility
                .into_iter()
                .chain([
                    MenuItem::new("edit-playlist", t!("menu-edit-playlist"))
                        .icon("icons/pencil.svg")
                        .on_press(act(move |ui: &mut Ui| {
                            playlist_editor::open(ui, Edit::Details(edited.clone()))
                        })),
                    MenuItem::new("delete-playlist", t!("menu-delete-playlist"))
                        .icon("icons/trash-2.svg")
                        .on_press(act(move |ui: &mut Ui| {
                            playlist_editor::open(ui, Edit::Delete(deleted.clone()))
                        })),
                ])
                .chain(imported.then(|| {
                    MenuItem::new("export-playlist", t!("menu-export-playlist"))
                        .icon("icons/square-arrow-out-up-right.svg")
                        .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
                            export_playlist(ui, exported.clone(), tx)
                        })
                }))
                .collect()
        }
        false => Vec::new(),
    };

    sections(
        item_shell(ui, "playlist-context-menu", tiles),
        vec![
            collection_queue_items(from),
            actions,
            match imported {
                true => Vec::new(),
                false => vec![copy_link_item(
                    ui,
                    "copy-playlist-link",
                    MediaKind::Playlist,
                    Some(&id),
                )],
            },
        ],
    )
}

/// A pin's menu: the full menu of the item when the library holds it, else a sparse one built
/// from the pin alone.
fn pin_menu(ui: &Ui, pin: &Pin) -> Menu<Ui> {
    let library = &ui.store.library;
    let built = match pin.kind {
        PinKind::Album => library.album(&pin.id).map(|album| album_menu(ui, album)),
        PinKind::Playlist => library
            .playlist(&pin.id)
            .map(|playlist| playlist_menu(ui, playlist)),
        PinKind::Artist => Some(artist_menu(
            ui,
            &library
                .artist(&pin.id)
                .cloned()
                .unwrap_or_else(|| pinned_artist(pin)),
            false,
        )),
        PinKind::Song => saved_track(library, &pin.id).map(|track| {
            track_menu(
                ui,
                std::slice::from_ref(track),
                &TrackList::Plain,
                TrackColumns::default(),
            )
        }),
    };
    built.unwrap_or_else(|| sparse_menu(ui, pin))
}

/// The artist a pin names, for a pin the library no longer lists.
pub(crate) fn pinned_artist(pin: &Pin) -> SavedArtist {
    SavedArtist {
        id: pin.id.clone(),
        name: pin.title.clone(),
        cover: pin.cover.clone(),
        ..Default::default()
    }
}

fn sparse_menu(ui: &Ui, pin: &Pin) -> Menu<Ui> {
    let open = open_key(pin.kind).map(|key| {
        MenuItem::new("open-pin", i18n::lookup(key, None))
            .icon("icons/info.svg")
            .on_press(go(destination(pin)))
    });

    sections(
        Menu::new("pin-context-menu").min_width(ITEM_MENU_WIDTH),
        vec![
            open.into_iter().collect(),
            transport_items(pin),
            vec![copy_link_item(
                ui,
                "copy-pin-link",
                media_kind(pin.kind),
                Some(&pin.id),
            )],
            vec![pin_action(ui, pin)],
        ],
    )
}

/// The menu behind the playlists of a library page. A local shelf also offers to import an
/// M3U file.
fn new_playlist_menu(shelf: Shelf) -> Menu<Ui> {
    Menu::new("playlist-background-menu")
        .item(
            MenuItem::new("create-playlist", t!("menu-new-playlist"))
                .icon("icons/plus.svg")
                .on_press(act(move |ui: &mut Ui| {
                    playlist_editor::open(
                        ui,
                        Edit::Create {
                            addition: Addition::Tracks(Vec::new()),
                            shelf,
                        },
                    )
                })),
        )
        .items(shelf.local().then(|| {
            MenuItem::new("import-playlist", t!("menu-import-playlist"))
                .icon("icons/file-music.svg")
                .on_press_tx(import_playlist)
        }))
}

/// Pins or unpins anything the app can open. The provider that keeps pins of its own is told
/// alongside the local list.
pub(crate) fn pin_action(ui: &Ui, pin: &Pin) -> MenuItem<Ui> {
    let store = &ui.store;
    let known = store.session.slug_for(&pin.id).is_some();
    let pinned = store.pins.holds(pin, &store.settings, &store.session);
    let item = MenuItem::new(
        "pin",
        match pinned {
            true => t!("nav-unpin"),
            false => t!("nav-pin"),
        },
    )
    .icon("icons/pin.svg");
    match known {
        false => item.disabled(),
        true => {
            let held = pin.clone();
            item.on_press(act(move |ui: &mut Ui| ui.store.toggle_pin(held.clone())))
        }
    }
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

/// The three queue placements of a whole collection, which share their labels with a track's.
fn collection_queue_items(origin: Origin) -> Vec<MenuItem<Ui>> {
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
    .map(|(key, label, icon, placement)| {
        let origin = origin.clone();
        MenuItem::new(key, i18n::lookup(label, None))
            .icon(icon)
            .on_press(act(move |ui: &mut Ui| {
                ui.store.enqueue_collection(origin.clone(), placement)
            }))
    })
    .collect()
}

/// What a sparse pin menu plays: a song's radio or an artist, then the queue placements of
/// anything but a song.
fn transport_items(pin: &Pin) -> Vec<MenuItem<Ui>> {
    let played = Origin::from(pin);
    let mut items = match pin.kind {
        PinKind::Song | PinKind::Artist => {
            let origin = played.clone();
            let (label, icon) = match pin.kind {
                PinKind::Song => ("menu-song-radio", "icons/radio.svg"),
                _ => ("menu-play-artist", "icons/play.svg"),
            };
            vec![
                MenuItem::new("play-pin", i18n::lookup(label, None))
                    .icon(icon)
                    .on_press(act(move |ui: &mut Ui| {
                        ui.store.play_collection(origin.clone())
                    })),
            ]
        }
        _ => Vec::new(),
    };
    if pin.kind != PinKind::Song {
        items.extend(collection_queue_items(played));
    }
    items
}

/// A queue item for the playable tracks of a selection. It is disabled when there are none, and
/// picking it uses the same placement path for one song and several.
fn queue_item(
    key: &'static str,
    label: String,
    icon: &'static str,
    queued: &[Track],
    placement: QueuePlacement,
) -> MenuItem<Ui> {
    let item = MenuItem::new(key, label).icon(icon);
    if queued.is_empty() {
        return item.disabled();
    }
    let queued = queued.to_vec();
    item.on_press(act(move |ui: &mut Ui| {
        ui.store.enqueue_tracks(queued.clone(), None, placement)
    }))
}

/// The Copy link item. The link is made while the menu is built, so an item with no link shows
/// up disabled and says why on hover.
fn copy_link_item(
    ui: &Ui,
    key: &'static str,
    kind: MediaKind,
    target: Option<&str>,
) -> MenuItem<Ui> {
    let item = MenuItem::new(key, t!("menu-copy-link")).icon("icons/link.svg");
    match share_link(ui, kind, target) {
        Ok(url) => item.on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
            tx.copy(url.clone());
            state::Toasts::show(Outcome::Done, "toast-link-copied", &mut ui.store.cx);
        })),
        Err(reason) => item.disabled().tooltip(i18n::lookup(reason, None)),
    }
}

/// The link that shares `id`, or the Fluent key of the reason there is none.
fn share_link(ui: &Ui, kind: MediaKind, id: Option<&str>) -> Result<String, &'static str> {
    let Some(id) = id else {
        return Err("menu-copy-link-none");
    };
    let local = music::is_local_id(id);
    let session = &ui.store.session;
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

/// Asks for an M3U or M3U8 file and reads it into a new local playlist.
fn import_playlist(_: &mut Ui, tx: &mut Tx) {
    let options = rune::FileDialogOptions {
        title: t!("menu-import-playlist").into_owned(),
        ..Default::default()
    }
    .filter("M3U", &["m3u", "m3u8"]);
    tx.open_file(options, |ui: &mut Ui, picked, _: &mut Tx| {
        let store = &mut ui.store;
        match picked {
            Ok(Some(path)) => store
                .library
                .import_playlist(path, &store.session, &mut store.cx),
            Ok(None) => {}
            Err(error) => {
                log::warn!("library: cannot open the file dialog: {error}");
                state::Toasts::show(Outcome::Failed, "toast-file-dialog-failed", &mut store.cx);
            }
        }
        ui.settle();
    });
}

/// Asks where to save a local playlist and writes it there as M3U8.
fn export_playlist(_: &mut Ui, playlist: Playlist, tx: &mut Tx) {
    let name = playlist
        .name
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-");
    let directory = dirs::audio_dir()
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir);
    let options = rune::FileDialogOptions {
        title: t!("menu-export-playlist").into_owned(),
        directory,
        file_name: Some(format!("{name}.{PLAYLIST_EXTENSION}")),
        ..Default::default()
    };
    tx.save_file(options, move |ui: &mut Ui, picked, _: &mut Tx| {
        let store = &mut ui.store;
        match picked {
            Ok(Some(path)) => store.library.export_playlist(
                playlist.id.clone(),
                path,
                &store.session,
                &mut store.cx,
            ),
            Ok(None) => {}
            Err(error) => {
                log::warn!("library: cannot open the file dialog: {error}");
                state::Toasts::show(Outcome::Failed, "toast-file-dialog-failed", &mut store.cx);
            }
        }
        ui.settle();
    });
}

/// The extension an exported playlist is written with.
const PLAYLIST_EXTENSION: &str = "m3u8";

/// A handler that goes to `destination`.
fn go(destination: Destination) -> impl Fn(&mut Ui) + 'static {
    move |ui: &mut Ui| ui.go(destination.clone())
}

/// A label for one item or for `count` of them.
fn counted(one: &'static str, many: &'static str, count: usize) -> String {
    if count <= 1 {
        return i18n::lookup(one, None).into_owned();
    }
    let mut args = i18n::FluentArgs::new();
    args.set("count", count as i64);
    i18n::lookup(many, Some(&args)).into_owned()
}

/// Joins the non-empty groups of a menu with a separator between each.
fn sections(menu: Menu<Ui>, groups: Vec<Vec<MenuItem<Ui>>>) -> Menu<Ui> {
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
fn item_shell(ui: &Ui, key: &'static str, tiles: Vec<Tile>) -> Menu<Ui> {
    let menu = Menu::new(key).min_width(ITEM_MENU_WIDTH);
    match tiles.is_empty() {
        true => menu,
        false => menu.header(tile_row(&ui.theme, tiles)),
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

fn saved_track<'a>(library: &'a Library, id: &str) -> Option<&'a Track> {
    library
        .state(Shelf::of(id))
        .tracks()
        .iter()
        .find(|track| track.id.as_deref() == Some(id))
}

/// What a tile does when pressed.
type TilePress = Rc<dyn Fn(&mut Ui)>;

/// One toggle of the tile row on top of an item menu. A tile with no press is drawn disabled.
struct Tile {
    key: &'static str,
    icon: &'static str,
    /// The Fluent key of the tooltip, which is also the name a screen reader announces.
    tip: &'static str,
    active: bool,
    /// Draws a download ring instead of the icon.
    ring: Option<Ring>,
    /// Closes the menu before the press runs, for a press that opens a dialog.
    closes: bool,
    press: Option<TilePress>,
}

/// A download ring drawn in place of a tile's icon, `done` of the way round or turning when the
/// amount is unknown. A failed download draws it in the danger color.
#[derive(Clone, Copy)]
struct Ring {
    done: Option<f32>,
    failed: bool,
}

impl Tile {
    fn new(key: &'static str, icon: &'static str, tip: &'static str) -> Self {
        Self {
            key,
            icon,
            tip,
            active: false,
            ring: None,
            closes: false,
            press: None,
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

    fn on_press(mut self, press: impl Fn(&mut Ui) + 'static) -> Self {
        self.press = Some(Rc::new(press));
        self
    }

    /// Sets the press only when `when` holds, leaving the tile disabled otherwise.
    fn on_press_if(self, when: bool, press: impl Fn(&mut Ui) + 'static) -> Self {
        match when {
            true => self.on_press(press),
            false => self,
        }
    }
}

/// The row of toggles on top of an item menu: the favorite, the library where it is apart from
/// the favorites, the download and the pin. The tiles share the row evenly, only a tile that is
/// on has a fill, and they leave the menu open when pressed unless they hand off to a dialog.
fn tile_row(theme: &Theme, tiles: Vec<Tile>) -> El {
    let tucked = ui::tucked(theme.radius, theme.font_size);
    row()
        .key("menu-tiles")
        .w_full()
        .gap(rems(theme, 0.25))
        .children(tiles.into_iter().map(|tile| {
            let Tile {
                key,
                icon,
                tip,
                active,
                ring,
                closes,
                press,
            } = tile;
            let mut button = Button::new(key)
                .ghost()
                .tooltip_above(i18n::lookup(tip, None))
                .toggled(active)
                .tint(match active {
                    true => theme.primary,
                    false => theme.popover_foreground,
                })
                .disabled(press.is_none());
            if active {
                button = button.fill(theme.secondary_active, theme.secondary_hover);
            }
            button = match ring {
                Some(Ring { done, failed }) => button.leading(
                    ArrowRing::new(done)
                        .color(match failed {
                            true => theme.danger,
                            false => theme.primary,
                        })
                        .build(theme),
                ),
                None => button.icon(icon),
            };
            if let Some(press) = press {
                button = button.on_press(move |ui: &mut Ui| {
                    if closes {
                        close(ui);
                    }
                    press(ui);
                    ui.settle();
                });
            }
            button.build(theme).grow(1.).min_w(0.).radius(tucked)
        }))
}

/// The pin tile, lit while the item is pinned. It is disabled for an item whose provider the
/// app cannot name again after a restart.
fn pin_tile(ui: &Ui, pin: Pin) -> Tile {
    let store = &ui.store;
    let known = store.session.slug_for(&pin.id).is_some();
    let pinned = store.pins.holds(&pin, &store.settings, &store.session);
    Tile::new(
        "tile-pin",
        "icons/pin.svg",
        match pinned {
            true => "nav-unpin",
            false => "nav-pin",
        },
    )
    .active(pinned)
    .on_press_if(known, move |ui: &mut Ui| ui.store.toggle_pin(pin.clone()))
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
    ui: &Ui,
    tracks: &[Track],
    ids: &[String],
    shelf: Shelf,
    imported: bool,
    pin: Option<Pin>,
) -> Vec<Tile> {
    let store = &ui.store;
    let library = &store.library;
    let editable = store.session.shape_of(shelf).editable();
    let actionable = |pending: fn(&Library, &str) -> bool| -> Vec<Track> {
        tracks
            .iter()
            .filter(|track| track.id.as_deref().is_some_and(|id| !pending(library, id)))
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
        !named.is_empty() && named.iter().all(|id| test(library, id))
    };

    let favorable = actionable(Library::pending);
    let saved = every(Library::saved);
    // Unfavoriting asks first where the heart is the library itself, and the menu gives way to
    // that question rather than staying open over it.
    let asks = saved
        && !favorable
            .first()
            .and_then(|track| track.id.as_deref())
            .is_some_and(|id| confirm::unstarring(library, id));
    let heart =
        heart_tile(saved)
            .closes(asks)
            .on_press_if(!favorable.is_empty(), move |ui: &mut Ui| {
                if saved {
                    confirm::library_songs(ui, favorable.clone());
                    return;
                }
                let store = &mut ui.store;
                let tracks = favorable
                    .iter()
                    .filter(|track| {
                        !track
                            .id
                            .as_deref()
                            .is_some_and(|id| store.library.saved(id))
                    })
                    .cloned()
                    .collect();
                store
                    .library
                    .save_tracks(tracks, true, &store.session, &mut store.cx);
            });

    let shelved = editable.then(|| {
        let held = actionable(Library::pending_library);
        let present = every(Library::in_library);
        Tile::new("tile-library", "icons/library-big.svg", "menu-save-library")
            .active(present)
            .on_press_if(!held.is_empty(), move |ui: &mut Ui| {
                let store = &mut ui.store;
                for track in held.clone() {
                    let there = track
                        .id
                        .as_deref()
                        .is_some_and(|id| store.library.in_library(id));
                    if there == present {
                        store.library.set_track_in_library(
                            track,
                            !present,
                            &store.session,
                            &mut store.cx,
                        );
                    }
                }
            })
    });

    let download = (!ids.is_empty() && !imported)
        .then(|| track_download_tile(ui, tracks, ids))
        .flatten();

    [heart]
        .into_iter()
        .chain(shelved)
        .chain(download)
        .chain(pin.map(|pin| pin_tile(ui, pin)))
        .collect()
}

/// The download tile, drawn the way a detail page's download button is. It shows the plain
/// arrow until the download is asked for, then a ring that fills as it arrives, and a red ring
/// once it failed. Pressing it starts the download, or retries a failed one, and once the
/// download is under way or in, pressing it stops it and removes what arrived.
fn download_tile(
    progress: Option<Progress>,
    done: Option<f32>,
    start: impl Fn(&mut Ui) + 'static,
    remove: impl Fn(&mut Ui) + 'static,
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
        None => tile.on_press(start),
        Some(Progress::Failed) => tile.ring(Some(done.unwrap_or(0.)), true).on_press(start),
        Some(Progress::Waiting | Progress::Running) => {
            tile.active(true).ring(done, false).on_press(remove)
        }
        Some(Progress::Done) => tile.active(true).ring(Some(1.), false).on_press(remove),
    }
}

/// The download tile of a track selection. The selection reads as downloading while any track
/// is on its way, as in once every track is, and as failed when a track failed and none is
/// left running. Nothing shows for a provider that cannot download, unless something is there
/// to remove.
fn track_download_tile(ui: &Ui, tracks: &[Track], ids: &[String]) -> Option<Tile> {
    let store = &ui.store;
    let downloads = &store.downloads;
    let states: Vec<Option<Progress>> = ids.iter().map(|id| downloads.track(id)).collect();
    let progress = match () {
        () if states.iter().all(|state| *state == Some(Progress::Done)) => Some(Progress::Done),
        () if states.contains(&Some(Progress::Running)) => Some(Progress::Running),
        () if states.contains(&Some(Progress::Waiting)) => Some(Progress::Waiting),
        () if states.contains(&Some(Progress::Failed)) => Some(Progress::Failed),
        () => None,
    };
    if progress.is_none() && !store.session.capabilities().downloads {
        return None;
    }
    let done = match ids {
        [id] => downloads.fraction(id),
        _ => {
            let arrived: f32 = ids
                .iter()
                .map(|id| match downloads.has(id) {
                    true => 1.,
                    false => downloads.fraction(id).unwrap_or(0.),
                })
                .sum();
            Some(arrived / ids.len() as f32)
        }
    };
    let tracks = tracks.to_vec();
    let ids = ids.to_vec();
    Some(download_tile(
        progress,
        done,
        move |ui: &mut Ui| ui.store.download_tracks(tracks.clone()),
        move |ui: &mut Ui| ui.store.remove_downloaded_tracks(&ids),
    ))
}

/// The whole album or playlist a download tile acts on.
#[derive(Clone)]
enum Saved {
    Album(Album),
    Playlist(Playlist),
}

/// The download tile of a whole album or playlist.
fn set_download_tile(ui: &Ui, saved: Saved) -> Option<Tile> {
    let (kind, id) = match &saved {
        Saved::Album(album) => (Collection::Album, album.id.clone()),
        Saved::Playlist(playlist) => (Collection::Playlist, playlist.id.clone()),
    };
    if music::is_local_id(&id) {
        return None;
    }
    let store = &ui.store;
    let progress = store.downloads.set(kind, &id);
    if progress.is_none() && !store.session.capabilities().downloads {
        return None;
    }
    let done = Some(store.downloads.share(kind, &id));
    Some(download_tile(
        progress,
        done,
        move |ui: &mut Ui| match &saved {
            Saved::Album(album) => ui.store.download_album_by_id(album),
            Saved::Playlist(playlist) => ui.store.download_playlist_by_id(playlist),
        },
        move |ui: &mut Ui| ui.store.remove_downloaded_set(kind, &id),
    ))
}

/// The tiles of an album menu: the favorite, the library where it is apart from the favorites,
/// the download and the pin.
fn album_tiles(ui: &Ui, album: &Album) -> Vec<Tile> {
    let store = &ui.store;
    let library = &store.library;
    let saved = library.saved_album(&album.id);
    let hearted = album.clone();
    let heart = heart_tile(saved)
        .closes(saved && !confirm::unstarring(library, &album.id))
        .on_press_if(
            !library.pending_album(&album.id),
            move |ui: &mut Ui| match saved {
                true => confirm::albums(ui, vec![hearted.clone()]),
                false => {
                    let store = &mut ui.store;
                    store
                        .library
                        .toggle_album(hearted.clone(), &store.session, &mut store.cx);
                }
            },
        );

    let editable = store.session.shape_of(Shelf::of(&album.id)).editable();
    let shelved = editable.then(|| {
        let present = library.in_library(&album.id);
        let shelved = album.clone();
        Tile::new("tile-library", "icons/library-big.svg", "menu-save-library")
            .active(present)
            .on_press_if(!library.pending_library(&album.id), move |ui: &mut Ui| {
                let store = &mut ui.store;
                store.library.set_album_in_library(
                    shelved.clone(),
                    !present,
                    &store.session,
                    &mut store.cx,
                );
            })
    });

    [heart]
        .into_iter()
        .chain(shelved)
        .chain(set_download_tile(ui, Saved::Album(album.clone())))
        .chain(album.pin().map(|pin| pin_tile(ui, pin)))
        .collect()
}

/// The tiles of an artist menu: the follow heart where the provider can follow artists, and the
/// pin.
fn artist_tiles(ui: &Ui, artist: &SavedArtist) -> Vec<Tile> {
    let store = &ui.store;
    let follow = store.session.capabilities().follow_artists.then(|| {
        let library = &store.library;
        let saved = library.saved_artist(&artist.id);
        let followed = artist.clone();
        heart_tile(saved)
            .closes(saved && !confirm::unstarring(library, &artist.id))
            .on_press_if(
                !library.pending_artist(&artist.id),
                move |ui: &mut Ui| match saved {
                    true => confirm::artists(ui, vec![followed.clone()]),
                    false => {
                        let store = &mut ui.store;
                        store.library.toggle_artist(
                            followed.clone(),
                            &store.session,
                            &mut store.cx,
                        );
                    }
                },
            )
    });
    follow
        .into_iter()
        .chain(artist.pin().map(|pin| pin_tile(ui, pin)))
        .collect()
}

/// The tiles of a playlist menu: the heart that keeps someone else's playlist in the library,
/// the download and the pin.
fn playlist_tiles(ui: &Ui, playlist: &Playlist) -> Vec<Tile> {
    let kept = (!playlist.owned).then(|| {
        let saved = ui.store.library.playlist(&playlist.id).is_some();
        let kept = playlist.clone();
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
        .on_press(move |ui: &mut Ui| match saved {
            true => confirm::playlists(ui, vec![kept.id.clone()]),
            false => {
                let store = &mut ui.store;
                store
                    .library
                    .add_playlist_to_library(kept.clone(), &store.session, &mut store.cx);
            }
        })
    });
    kept.into_iter()
        .chain(set_download_tile(ui, Saved::Playlist(playlist.clone())))
        .chain(playlist.pin().map(|pin| pin_tile(ui, pin)))
        .collect()
}
