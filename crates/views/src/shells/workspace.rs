//! The workspace: the left sidebar, the page, the right sidebar and the player bar under them.
//! The page is keyed by where the window is, so moving to another page plays its entrance
//! while the one it replaces fades out over it.

use router::Destination;
use rune::{Cx, El, Transition, Tx, col, row};

use crate::chrome::{player_bar, sidebar_left, sidebar_right, toasts};
use crate::screens;
use crate::Ui;

/// How far a page grows from as it enters, and how blurred it starts: the GPUI entrance's veil.
const ENTRANCE_SCALE: f32 = 0.99;
const ENTRANCE_BLUR: f32 = 1.5;

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let destination = ui.nav.current();
    let page = page(ui, &destination, cx, layer)
        .key(page_key(&destination))
        .absolute(0., 0.)
        .w_full()
        .h_full()
        .transition(Transition::asymmetric(
            Transition::opacity()
                .combined(Transition::scale(ENTRANCE_SCALE))
                .combined(Transition::blur(ENTRANCE_BLUR)),
            Transition::opacity(),
        ));
    let content = col()
        .key("content")
        .grow(1.)
        .min_w(0.)
        .min_h(0.)
        .clip_rounded(0.)
        .child(page);

    let body = row()
        .key("workspace")
        .role("main")
        .grow(1.)
        .min_h(0.)
        .w_full()
        .child(sidebar_left::view(ui, cx, layer))
        .child(content)
        .child(sidebar_right::view(ui, cx, layer));

    let toasts = toasts::view(ui, cx, layer);
    layer.push(toasts);
    col()
        .key("workspace-shell")
        .grow(1.)
        .min_h(0.)
        .w_full()
        .fill(theme.background.color())
        .child(body)
        .child(player_bar::view(ui, cx, layer))
}

/// The screen that draws `destination`.
fn page(ui: &Ui, destination: &Destination, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    match destination {
        Destination::Home => screens::home::view(ui, cx, layer),
        Destination::Search => screens::search::view(ui, cx, layer),
        Destination::History => screens::history::view(ui, cx, layer),
        Destination::Library(_) | Destination::Local(_) => screens::library::view(ui, cx, layer),
        Destination::Album(_) | Destination::Playlist(_) => screens::detail::view(ui, cx, layer),
        Destination::Artist(_) => screens::artist::view(ui, cx, layer),
        Destination::Song(_) => screens::song::view(ui, cx, layer),
        Destination::User(_) => screens::user::view(ui, cx, layer),
        Destination::Genre(_) => screens::genre::view(ui, cx, layer),
        Destination::Settings(_) => screens::settings::view(ui, cx, layer),
        Destination::Equalizer => screens::equalizer::view(ui, cx, layer),
        Destination::Fullscreen => screens::home::view(ui, cx, layer),
    }
}

/// A key per page, so a page replaced by another of the same screen (one album by the next)
/// enters afresh, while tabs of one section stay one page.
fn page_key(destination: &Destination) -> String {
    match destination {
        Destination::Home => "page-home".into(),
        Destination::Search => "page-search".into(),
        Destination::History => "page-history".into(),
        Destination::Library(_) => "page-library".into(),
        Destination::Local(_) => "page-local".into(),
        Destination::Album(id) => format!("page-album-{id}"),
        Destination::Playlist(id) => format!("page-playlist-{id}"),
        Destination::Artist(id) => format!("page-artist-{id}"),
        Destination::Song(id) => format!("page-song-{id}"),
        Destination::User(id) => format!("page-user-{id}"),
        Destination::Genre(id) => format!("page-genre-{id}"),
        Destination::Settings(_) | Destination::Equalizer => "page-settings".into(),
        Destination::Fullscreen => "page-fullscreen".into(),
    }
}

/// Loads the shown page again, as the title bar's refresh button and its shortcut do.
pub(crate) fn refresh(ui: &mut Ui) {
    let store = &mut ui.store;
    match ui.nav.current() {
        Destination::Home => store.refresh_home(),
        Destination::History => store.refresh_history(),
        Destination::Library(_) => store.reload_library(state::Shelf::Streaming),
        Destination::Local(_) => store.reload_library(state::Shelf::Local),
        Destination::Album(_) => store.refresh_detail(state::Collection::Album),
        Destination::Playlist(_) => store.refresh_detail(state::Collection::Playlist),
        Destination::Artist(_) => store.refresh_artist(),
        Destination::Song(_) => store.refresh_song(),
        Destination::User(_) => store.refresh_user(),
        Destination::Genre(_) => store.refresh_genre(),
        Destination::Search
        | Destination::Settings(_)
        | Destination::Equalizer
        | Destination::Fullscreen => {}
    }
}

/// Opens the shown page's filter field, for pages that have one.
pub(crate) fn open_filter(ui: &mut Ui, tx: &mut Tx) {
    let _ = (ui, tx);
}

/// Closes whatever is open on top of the page: a menu, a popover, a dialog.
pub(crate) fn dismiss(ui: &mut Ui) {
    ui.chrome.popovers.close();
}
