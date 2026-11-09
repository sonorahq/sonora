//! The album and playlist pages: a hero with the cover, the facts and the Play, Shuffle, Like,
//! Download and More buttons, over the virtualised track table, with an album's copyright notices
//! and its recommendations under the rows. Both kinds share this screen; each keeps its own
//! table state.

use std::rc::Rc;

use i18n::t;
use music::{AlbumDetail, GenreItem, Track};
use router::{Destination, LibraryTab};
use rune::{Cx, Drag, El, PointerPress, Tx, col, row};
use state::{AppSettings, Collection, Detail, Origin, Progress, SessionState, Shelf};
use ui::{
    ArrowRing, Button, InlineLink, InlineLinks, MenuItem, Pending, Picker, Pin, PinKind, Popovers,
    Text, Weight,
};

use crate::chrome::toolbar::Tools;
use crate::chrome::{player_bar, sidebar_left, tools};
use crate::shared::album_grid::CardGrid;
use crate::shared::cards::{self, Fit, OnItem};
use crate::shared::hero::{
    Hero, Listing, MetaStrip, PlayButton, action_gap, copyright_notices, like_button,
    release_date_label,
};
use crate::shared::menus::{self, Target, TrackColumns};
use crate::shared::page::{self, rems};
use crate::shared::pins::{Pinned as _, origin_tag};
use crate::shared::shelves::{Rail, Rails};
use crate::shared::tracks::{
    Picked, TrackList, TrackRows, TrackTable, album_columns, playlist_columns,
};
use crate::shared::{cells, confirm, trouble};
use crate::{Ui, act};

/// The room above the recommendations, in rems.
const RAIL_DROP: f32 = 2.5;
/// The room above the notices and between their lines, in rems.
const NOTICE_DROP: f32 = 0.5;
const NOTICE_GAP: f32 = 0.25;
/// The space between the recommendation tabs, in rems.
const TAB_GAP: f32 = 0.25;

/// Which list the recommendation rail shows: releases, or artists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RailTab {
    #[default]
    Albums,
    Artists,
}

impl RailTab {
    fn id(self) -> &'static str {
        match self {
            Self::Albums => "rail-tab-albums",
            Self::Artists => "rail-tab-artists",
        }
    }

    fn label(self) -> String {
        match self {
            Self::Albums => t!("album-tab-albums"),
            Self::Artists => t!("album-tab-artists"),
        }
        .into_owned()
    }
}

/// The album and playlist pages' own state: each kind's table, the recommendation rail's scroll
/// and tab, and the hero's More menu.
#[derive(Default)]
pub struct State {
    album: TrackList,
    playlist: TrackList,
    rails: Rails,
    rail_tab: RailTab,
    popovers: Popovers,
}

impl State {
    /// Puts back the layout, sort and filters saved for the page arrived at, and forgets what the
    /// last item of the same kind had narrowed or selected.
    pub(crate) fn arrived(&mut self, destination: &Destination, settings: &AppSettings) {
        let (list, key) = match destination {
            Destination::Album(_) => (&mut self.album, remember(Collection::Album, None)),
            Destination::Playlist(id) => (
                &mut self.playlist,
                remember(Collection::Playlist, Some(id.as_ref())),
            ),
            _ => return,
        };
        *list = TrackList::new();
        list.restore(settings, &key);
        self.rails.reset();
        self.rail_tab = RailTab::Albums;
        self.popovers.close();
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let Some(kind) = kind(ui) else {
        return col().key("screen-detail");
    };
    let detail = page(ui, kind);
    if let Some(failure) = failure(ui, kind, detail) {
        return col()
            .key("detail-failure")
            .role("alert")
            .w_full()
            .h_full()
            .center()
            .child(failure);
    }
    let theme = &ui.theme;
    let width = page::content_width(cx, 0.);
    let rows = rows(ui, kind);
    let title = detail
        .and_then(Detail::header)
        .map(|header| header.title.clone())
        .unwrap_or_default();
    let id = detail.and_then(Detail::id).unwrap_or_default().to_owned();

    let mut table = TrackTable::new("detail-tracks", &rows)
        .width(width)
        .lead(hero(ui, kind, detail, &rows, width, layer))
        .foot(foot(ui, detail, width))
        .pending(pending(detail))
        .label(title)
        .bottom(theme.metrics.inset + player_bar::bottom(ui, cx))
        .remember(remember(kind, Some(id.as_str())));

    let listed = id.clone();
    table = table.on_context(
        move |ui: &mut Ui, picked: Picked, press: PointerPress, _: &mut Tx| {
            let list = match kind {
                Collection::Album => menus::TrackList::Album(listed.clone()),
                Collection::Playlist => menus::TrackList::Playlist,
            };
            let columns = TrackColumns {
                album: picked.album,
                artists: picked.artists,
            };
            menus::open_tracks(ui, picked.tracks, list, columns, press);
        },
    );
    table = table.on_remove(move |ui: &mut Ui, picked: Picked, _: &mut Tx| {
        match kind {
            Collection::Album => confirm::library_songs(ui, picked.tracks),
            Collection::Playlist => {
                let count = picked.tracks.len();
                let ids = picked
                    .tracks
                    .into_iter()
                    .filter_map(|track| track.id)
                    .collect();
                confirm::playlist_songs(ui, ids, count);
            }
        }
        ui.settle();
    });
    let carried = rows.clone();
    table = table.on_row_drag(
        move |ui: &mut Ui, tracks: &[Track], drag: &mut Drag, tx: &mut Tx| {
            let Some(track) = tracks.first() else {
                return;
            };
            let Some(pin) = track.pin() else {
                return;
            };
            let tag = moving(ui, &carried, track);
            sidebar_left::carry(ui, &pin.origin(tag), drag, tx);
        },
    );
    table.build(ui)
}

/// The tools the title bar carries for the album or playlist on show: the filters, the sort and
/// the search field.
pub(crate) fn tools(ui: &Ui, layer: &mut Vec<El>) -> Tools {
    let Some(kind) = kind(ui) else {
        return Tools::default();
    };
    let rows = rows(ui, kind);
    let id = page(ui, kind).and_then(Detail::id).map(str::to_owned);
    let key = remember(kind, id.as_deref());
    let (sifted, sorted, saved) = (rows.clone(), rows.clone(), key.clone());
    Tools {
        items: vec![
            tools::filters(
                ui,
                rows.filters(ui),
                move |ui: &mut Ui, change| sifted.filter(ui, change, Some(&key)),
                layer,
            ),
            tools::sorts(
                ui,
                rows.sorts(ui),
                move |ui: &mut Ui, column| sorted.cycle_sort(ui, column, Some(&saved)),
                layer,
            ),
        ],
        search: Some("filter-album"),
    }
}

/// Leaves a playlist page whose playlist was deleted or taken out of the library while it showed:
/// back where the listener came from, or to the library's playlists.
pub(crate) fn follow(ui: &mut Ui) {
    let Destination::Playlist(_) = ui.nav.current() else {
        return;
    };
    let gone = ui
        .store
        .pages
        .playlist
        .as_ref()
        .is_some_and(|detail| detail.id().is_none());
    let signed_in = matches!(
        ui.store.session.state(),
        SessionState::SignedIn(_) | SessionState::Offline(_)
    );
    if !gone || !signed_in {
        return;
    }
    match ui.nav.can_go_back() {
        true => ui.back(),
        false => ui.go(Destination::Library(LibraryTab::Playlists)),
    }
}

/// Whether the page on show is an album or a playlist.
fn kind(ui: &Ui) -> Option<Collection> {
    match ui.nav.current() {
        Destination::Album(_) => Some(Collection::Album),
        Destination::Playlist(_) => Some(Collection::Playlist),
        _ => None,
    }
}

/// The page model of `kind`.
fn page(ui: &Ui, kind: Collection) -> Option<&Detail> {
    match kind {
        Collection::Album => ui.store.pages.album.as_ref(),
        Collection::Playlist => ui.store.pages.playlist.as_ref(),
    }
}

/// The settings key the table's layout, sort and filters are saved under: one for every album,
/// one per playlist.
fn remember(kind: Collection, id: Option<&str>) -> String {
    match (kind, id) {
        (Collection::Playlist, Some(id)) if !id.is_empty() => format!("playlist:{id}"),
        (Collection::Playlist, _) => "playlist".to_owned(),
        (Collection::Album, _) => "album".to_owned(),
    }
}

fn album_tracks(ui: &Ui) -> &[Track] {
    ui.store
        .pages
        .album
        .as_ref()
        .map(Detail::tracks)
        .unwrap_or_default()
}

fn playlist_tracks(ui: &Ui) -> &[Track] {
    ui.store
        .pages
        .playlist
        .as_ref()
        .map(Detail::tracks)
        .unwrap_or_default()
}

fn album_list(ui: &Ui) -> &TrackList {
    &ui.screens.detail.album
}

fn album_list_mut(ui: &mut Ui) -> &mut TrackList {
    &mut ui.screens.detail.album
}

fn playlist_list(ui: &Ui) -> &TrackList {
    &ui.screens.detail.playlist
}

fn playlist_list_mut(ui: &mut Ui) -> &mut TrackList {
    &mut ui.screens.detail.playlist
}

/// The table's rows for the page of `kind`: an album with its own track numbers and play counts
/// where the provider has them, a playlist with who added each song where others add to it.
fn rows(ui: &Ui, kind: Collection) -> TrackRows {
    let detail = page(ui, kind);
    let query = ui.chrome.toolbar.query(&ui.nav.current());
    match kind {
        Collection::Album => {
            let id = detail.and_then(Detail::id).unwrap_or_default().to_owned();
            let playcounts = ui.store.session.capabilities_of(Shelf::of(&id)).playcounts;
            let origin = detail
                .and_then(Detail::album)
                .map(|album| Origin::album(album.id.clone()).named(album.name.clone()));
            TrackRows::new(
                album_tracks,
                album_list,
                album_list_mut,
                album_columns(playcounts),
            )
            .origin(origin)
            .liked()
            .album(id)
            .query(query)
        }
        Collection::Playlist => {
            let blend = detail
                .and_then(Detail::playlist)
                .is_some_and(|list| list.blend);
            let shared = playlist_tracks(ui)
                .iter()
                .any(|track| track.added_by.is_some());
            let origin = detail
                .and_then(Detail::playlist)
                .map(|list| Origin::playlist(list.id.clone()).named(list.name.clone()));
            TrackRows::new(
                playlist_tracks,
                playlist_list,
                playlist_list_mut,
                playlist_columns(blend, shared),
            )
            .origin(origin)
            .liked()
            .query(query)
        }
    }
}

/// The tag a song dragged off the page carries, so a drop on another playlist moves it. A song
/// its playlist holds more than once stays a copy, since the provider would take out every copy
/// and the Undo would bring back one.
fn moving(ui: &Ui, rows: &TrackRows, track: &Track) -> Option<String> {
    let origin = rows.whence()?;
    let once = origin.whence != state::Whence::Playlist
        || rows
            .tracks(ui)
            .iter()
            .filter(|other| other.id == track.id)
            .count()
            == 1;
    once.then(|| origin_tag(origin)).flatten()
}

/// As many skeleton rows as the header says the page holds, or a screenful when it opened
/// without a header or with a provider that reports no count.
fn pending(detail: Option<&Detail>) -> Option<Pending> {
    let detail = detail?;
    if !detail.is_loading() {
        return None;
    }
    Some(
        match detail.header().map_or(0, |header| header.track_count) {
            0 => Pending::Screen,
            count => Pending::Rows(count as usize),
        },
    )
}

/// The hero: cover, kind, title, facts, and the actions. Buttons whose action the More menu
/// also offers fold into it from the right when the row runs out of room, Download first and
/// Shuffle last. Play and More always show.
fn hero(
    ui: &Ui,
    kind: Collection,
    detail: Option<&Detail>,
    rows: &TrackRows,
    width: f32,
    layer: &mut Vec<El>,
) -> El {
    let theme = &ui.theme;
    let muted = theme.muted_foreground;
    let small = theme.text(Text::Small);
    let header = detail.and_then(Detail::header);
    let kind = header.map(|header| header.kind).unwrap_or(kind);
    let title = header
        .map(|header| header.title.clone())
        .unwrap_or_default();
    let eyebrow = match kind {
        Collection::Playlist => t!("detail-playlist").into_owned(),
        Collection::Album => i18n::lookup(
            cards::release_key(
                header
                    .and_then(|header| header.release)
                    .unwrap_or(music::ReleaseType::Album),
            ),
            None,
        )
        .into_owned(),
    };

    let mut strip = MetaStrip::new();
    if let Some(header) = header {
        if let Some(artist) = header.artist.clone() {
            strip = strip.item(
                cells::artist_links("detail-artist", &header.artist_refs, artist, muted)
                    .text_size(small)
                    .truncate()
                    .build(theme),
            );
        }
        if let Some(owner) = header.owner.clone() {
            strip = strip.item(
                InlineLinks::new(
                    "detail-owner",
                    [InlineLink::new(owner.name.clone(), Some(owner.id))],
                    owner.name,
                    muted,
                )
                .text_size(small)
                .truncate()
                .on_link(|ui: &mut Ui, id: &str, _: &mut Tx| ui.go(Destination::User(id.into())))
                .build(theme),
            );
        }
        if let Some(date) = header.release_date.as_deref() {
            strip = strip.text(release_date_label(date));
        }
        if let Some(owner) = header.owner_name.clone() {
            strip = strip.text(owner);
        }
        if header.track_count > 0 {
            strip = strip.text(t!("count-songs", count = header.track_count).into_owned());
        }
    }
    let duration: std::time::Duration = detail
        .map(Detail::tracks)
        .unwrap_or_default()
        .iter()
        .map(|track| track.duration)
        .sum();
    if !duration.is_zero() {
        strip = strip.text(ui::runtime(duration).into_owned());
    }

    let target = detail.and_then(|detail| target(ui, detail));
    let play = PlayButton::new("play-detail", Listing::Listed(rows.clone()));
    let shuffle = PlayButton::shuffle("shuffle-detail", Listing::Listed(rows.clone()));
    let mut spare = match target.is_some() {
        true => {
            let step = Button::width(std::iter::empty::<&str>(), theme) + action_gap(theme);
            let room = Hero::room(width - theme.metrics.inset * 2., theme);
            ((room - play.width(theme) - step) / step).floor().max(0.) as usize
        }
        false => usize::MAX,
    };
    let mut fits = || match spare {
        0 => false,
        _ => {
            spare -= 1;
            true
        }
    };
    let (shuffle, folded): (Option<El>, Option<MenuItem<Ui>>) = match fits() {
        true => (Some(shuffle.build(ui)), None),
        false => (None, Some(shuffle.shuffle_item(ui))),
    };
    let library = detail
        .and_then(|detail| library_button(ui, detail))
        .filter(|_| fits());
    let download = detail
        .and_then(|detail| download_button(ui, detail))
        .filter(|_| fits());
    let overflow = target.as_ref().map(|target| {
        Picker::icon(
            "detail-overflow",
            &ui.screens.detail.popovers,
            |ui: &mut Ui| &mut ui.screens.detail.popovers,
            "icons/ellipsis.svg",
        )
        .tooltip(t!("common-more"))
        .large()
        .left()
        .menu(menus::menu(ui, target).lead(folded))
        .build(theme, layer)
    });
    let actions = row()
        .key("detail-actions")
        .center_y()
        .gap(action_gap(theme))
        .child(play.build(ui))
        .children(shuffle)
        .children(library)
        .children(download)
        .children(overflow);

    let cover = header.and_then(|header| header.cover.clone());
    let pin = detail.and_then(Detail::id).map(|id| {
        let kind = match kind {
            Collection::Album => PinKind::Album,
            Collection::Playlist => PinKind::Playlist,
        };
        Pin::new(kind, id, title.clone()).cover(cover.clone())
    });
    let loading = detail.is_some_and(Detail::is_loading);
    let mut hero = Hero::new("detail-hero", title)
        .pending(header.is_none() && loading)
        .cover(cover)
        .eyebrow(eyebrow)
        .meta(strip.build(theme))
        .actions(actions)
        .padded();
    if let Some(target) = target {
        hero = hero.on_context(move |ui: &mut Ui, press: PointerPress, _: &mut Tx| {
            menus::open(ui, target.clone(), press)
        });
    }
    let hero = hero.build(ui);
    match pin {
        // The whole hero carries the page's pin to the sidebar.
        Some(pin) => hero.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
            sidebar_left::carry(ui, &pin, drag, tx)
        }),
        None => hero,
    }
}

/// What the More menu and a right-click on the hero are about: the album, or the playlist as
/// the library knows it.
fn target(ui: &Ui, detail: &Detail) -> Option<Target> {
    let id = detail.id()?;
    Some(match detail.header()?.kind {
        Collection::Album => Target::Album(detail.album()?.clone()),
        Collection::Playlist => Target::Playlist(
            ui.store
                .library
                .playlist(id)
                .cloned()
                .or_else(|| detail.playlist().cloned())?,
        ),
    })
}

/// The heart that saves or unsaves the album or someone else's playlist. A playlist of the
/// listener's own has none, since it is in the library for as long as it exists.
fn library_button(ui: &Ui, detail: &Detail) -> Option<El> {
    let library = &ui.store.library;
    let id = detail.id()?.to_owned();
    let (saved, busy, press): (bool, bool, Box<dyn Fn(&mut Ui)>) = match detail.header()?.kind {
        Collection::Album => {
            let album = detail.album()?.clone();
            let saved = library.saved_album(&id);
            (
                saved,
                library.pending_album(&id),
                Box::new(move |ui: &mut Ui| {
                    if saved && !confirm::unstarring(&ui.store.library, &album.id) {
                        confirm::albums(ui, vec![album.clone()]);
                        return;
                    }
                    let store = &mut ui.store;
                    store
                        .library
                        .heart_album(album.clone(), &store.session, &mut store.cx);
                }),
            )
        }
        Collection::Playlist => {
            let known = library.playlist(&id).cloned();
            let playlist = known.clone().or_else(|| detail.playlist().cloned())?;
            if playlist.owned {
                return None;
            }
            let saved = known.is_some();
            (
                saved,
                false,
                Box::new(move |ui: &mut Ui| match saved {
                    true => confirm::playlists(ui, vec![playlist.id.clone()]),
                    false => {
                        let store = &mut ui.store;
                        store.library.add_playlist_to_library(
                            playlist.clone(),
                            &store.session,
                            &mut store.cx,
                        );
                    }
                }),
            )
        }
    };
    Some(like_button(
        "detail-toggle-library",
        saved,
        busy,
        act(move |ui: &mut Ui| press(ui)),
        &ui.theme,
    ))
}

/// Downloads the album or playlist for offline play, or removes its download. It shows for a
/// provider whose tracks can be downloaded, and for one that is offline with this page
/// downloaded. The ring fills as the tracks arrive.
fn download_button(ui: &Ui, detail: &Detail) -> Option<El> {
    let theme = &ui.theme;
    let downloads = &ui.store.downloads;
    let id = detail.id()?.to_owned();
    if music::is_local_id(&id) {
        return None;
    }
    let kind = detail.header()?.kind;
    let progress = downloads.set(kind, &id);
    if progress.is_none() && !ui.store.session.capabilities().downloads {
        return None;
    }
    let album = detail.album().cloned();
    let playlist = detail.playlist().cloned();
    let tracks = detail.tracks().to_vec();
    let share = downloads.share(kind, &id);
    let ring = |done: f32, color: ui::Hsla| {
        row()
            .key("detail-download-ring")
            .child(ArrowRing::new(Some(done)).color(color).build(theme))
    };
    let button = Button::new("detail-download").outline();
    let button = match progress {
        None => button
            .icon("icons/download.svg")
            .tooltip(t!("menu-download")),
        Some(Progress::Failed) => button
            .leading(ring(share, theme.danger))
            .tooltip(t!("menu-download")),
        Some(Progress::Waiting | Progress::Running) => button
            .leading(ring(share, theme.primary))
            .tooltip(t!("menu-downloading")),
        Some(Progress::Done) => button
            .leading(ring(1., theme.primary))
            .tooltip(t!("menu-remove-download")),
    };
    Some(
        button
            .on_press(act(move |ui: &mut Ui| {
                match (progress, &album, &playlist) {
                    (None | Some(Progress::Failed), Some(album), _) => {
                        ui.store.download_album(AlbumDetail {
                            album: album.clone(),
                            tracks: tracks.clone(),
                        })
                    }
                    (None | Some(Progress::Failed), None, Some(playlist)) => {
                        ui.store.download_playlist_by_id(playlist)
                    }
                    (None | Some(Progress::Failed), None, None) => {}
                    _ => ui.store.remove_downloaded_set(kind, &id),
                }
            }))
            .build(theme),
    )
}

/// What scrolls under the rows: an album's copyright notices, then its recommendations.
fn foot(ui: &Ui, detail: Option<&Detail>, width: f32) -> El {
    let theme = &ui.theme;
    col()
        .key("detail-foot")
        .w_full()
        .children(detail.and_then(|detail| notices(ui, detail)))
        .child(
            col()
                .key("detail-recommended")
                .w_full()
                .px(theme.metrics.inset)
                .pt(rems(theme, RAIL_DROP))
                .children(detail.and_then(|detail| recommended(ui, detail, width))),
        )
}

/// The album's copyright and label lines, or nothing on a playlist and on an album whose
/// provider names neither.
fn notices(ui: &Ui, detail: &Detail) -> Option<El> {
    let theme = &ui.theme;
    let notices = copyright_notices(detail.album()?);
    if notices.is_empty() {
        return None;
    }
    Some(
        col()
            .key("detail-notices")
            .px(theme.metrics.pad * 2.)
            .pt(rems(theme, NOTICE_DROP))
            .gap(rems(theme, NOTICE_GAP))
            .min_w(0.)
            .children(notices.into_iter().enumerate().map(|(index, notice)| {
                ui::spoken(format!("detail-notice-{index}"), notice, theme)
                    .font(ui::font(theme, Text::Tiny, Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .wrap()
            })),
    )
}

/// The recommendation rail, on album pages only: related releases under the albums tab, similar
/// artists under the artists tab. The tabs show while both lists hold something. A rail still on
/// its way reads as skeletons only while nothing of it is up.
fn recommended(ui: &Ui, detail: &Detail, width: f32) -> Option<El> {
    let theme = &ui.theme;
    detail.album()?;
    let layout = CardGrid::layout(width - theme.metrics.inset * 2.);
    let title = t!("album-also-like").into_owned();
    let albums = detail.also_like();
    let artists = detail.similar();
    if albums.is_empty() && artists.is_empty() {
        return detail
            .is_filling()
            .then(|| Rail::pending("detail-also-like", title, layout, theme));
    }
    let chosen = ui.screens.detail.rail_tab;
    // A tab without a list behind it is not one the page can be on.
    let tab = match chosen {
        RailTab::Albums if albums.is_empty() => RailTab::Artists,
        RailTab::Artists if artists.is_empty() => RailTab::Albums,
        tab => tab,
    };
    let opener: OnItem = Rc::new(
        |ui: &mut Ui, item: GenreItem, press: PointerPress, _: &mut Tx| {
            if let Some(target) = Target::of(&item) {
                menus::open(ui, target, press);
            }
        },
    );
    let cards: Vec<El> = match tab {
        RailTab::Albums => albums
            .iter()
            .enumerate()
            .map(|(place, album)| {
                cards::album_card(
                    ui,
                    format!("detail-also-like-{}", album.id),
                    album,
                    Fit::Tile(layout.card),
                    Some(&opener),
                )
                .flat()
                .build(theme)
                .position_in_set(place + 1, albums.len())
            })
            .collect(),
        RailTab::Artists => artists
            .iter()
            .enumerate()
            .map(|(place, artist)| {
                cards::artist_card(
                    ui,
                    format!("detail-similar-{}", artist.id),
                    artist,
                    Fit::Tile(layout.card),
                    Some(&opener),
                )
                .flat()
                .build(theme)
                .position_in_set(place + 1, artists.len())
            })
            .collect(),
    };
    // Each tab keeps a rail of its own, so switching starts the other list from its first card.
    let key = match tab {
        RailTab::Albums => "detail-also-like",
        RailTab::Artists => "detail-similar",
    };
    let mut rail = Rail::new(key, title, &ui.screens.detail.rails, layout).cards(cards);
    if !albums.is_empty() && !artists.is_empty() {
        let tabs = [RailTab::Albums, RailTab::Artists].map(|tab| {
            Button::new(tab.id())
                .label(tab.label())
                .small()
                .outline()
                .selected(tab == chosen)
                .toggled(tab == chosen)
                .on_press(move |ui: &mut Ui| {
                    let state = &mut ui.screens.detail;
                    if state.rail_tab != tab {
                        state.rail_tab = tab;
                        state.rails.reset();
                    }
                })
                .build(theme)
        });
        rail = rail.tabs(
            row()
                .key("detail-rail-tabs")
                .gap(rems(theme, TAB_GAP))
                .children(tabs),
        );
    }
    Some(rail.build(theme))
}

/// The page an album or a playlist shows instead of its hero and its table when it cannot be
/// read: the No connection state as soon as the network is gone, whatever rows the page happens
/// to hold, and the failure of its own load otherwise. A downloaded album or playlist opens from
/// its kept copy instead, network or not.
fn failure(ui: &Ui, kind: Collection, detail: Option<&Detail>) -> Option<El> {
    let detail = detail?;
    let id = detail.id()?;
    let kept = detail
        .kind()
        .is_some_and(|kind| ui.store.downloads.set(kind, id).is_some());
    if kept && detail.error().is_none() {
        return None;
    }
    let reason = match trouble::unreachable(ui, id) {
        true => None,
        false => Some(detail.error()?.to_owned()),
    };
    Some(trouble::lost(
        ui,
        "detail-lost",
        t!("trouble-not-loaded"),
        reason.as_deref(),
        move |ui: &mut Ui| ui.store.reload_detail(kind),
    ))
}
