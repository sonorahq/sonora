//! The right sidebar: the queue and the lyrics, and the members of the Session or Jam the listener
//! is in. It slides in and out on a width spring and resizes from a grip along its inner edge;
//! which tab shows, how wide it is and whether it is open are remembered in settings.

use rune::{Cx, El, Transition, Tx, el};
use state::SideTab;
use ui::{MIN_CONTENT, Panel, Room, Side, Springs};

use crate::Ui;
use crate::chrome::aside;
use crate::chrome::{RIGHT_MAX, RIGHT_MIN};

/// How finely the window's width is read for the grip's reach, so a resize of the window only
/// runs the view again every few points.
const REACH_STEP: f32 = 16.;

/// The sidebar's own view state: each panel's.
#[derive(Default)]
pub struct State {
    pub queue: aside::queue::State,
    pub lyrics: aside::lyrics::State,
    /// The tab the last poll found on screen, so a panel coming into view, whoever opened it,
    /// starts where it belongs.
    shown: Option<SideTab>,
}

/// The sidebar, or nothing while it is closed or the window is too narrow for it. The panel
/// stays mounted while it slides out, so the page beside it widens on the same spring.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let chrome = &ui.chrome;
    let window = crate::window_room(cx);
    let shown = chrome.right_open && available(window);
    let width = chrome.right_width.clamp(RIGHT_MIN, RIGHT_MAX);
    // The panel stands at the slot's left edge and keeps its own width, so a narrowing slot
    // carries it out past the window's right edge rather than squeezing it.
    let slot = el()
        .key("sidebar-right-slot")
        .h_full()
        .min_w(0.)
        .w(match shown {
            true => width,
            false => 0.,
        })
        .motion(Springs::SLOW);
    if !shown {
        return slot;
    }
    let window_width = cx
        .layout_with("window", |rect| (rect[2] / REACH_STEP).floor() * REACH_STEP)
        .unwrap_or(f32::MAX);
    let reach = RIGHT_MAX.min((window_width - MIN_CONTENT).max(RIGHT_MIN));
    let tab = shown_tab(ui);
    let mut panel = Panel::new("sidebar-right", Side::Right, width)
        .limits(RIGHT_MIN, RIGHT_MAX)
        .reach(reach)
        .on_resize(|ui: &mut Ui, width: f32, _: &mut Tx| {
            ui.chrome.right_width = width;
            ui.store
                .settings
                .set_sidebar_right_width(width, &mut ui.store.cx);
        })
        .child(aside::view(ui, cx, layer, tab))
        .build(theme)
        .w(width)
        .min_w(width)
        .transition(Transition::opacity());
    if !theme.transparent {
        panel = panel.fill(theme.background.color());
    }
    slot.child(panel)
}

/// Whether a window of `room` has space for the sidebar beside the page.
pub(crate) fn available(room: Room) -> bool {
    room.fits(Room::Wide)
}

/// The tab the sidebar shows: the remembered one, or the queue while lyrics are turned off.
pub(crate) fn shown_tab(ui: &Ui) -> SideTab {
    match ui.chrome.right_tab {
        SideTab::Lyrics if !ui.store.settings.lyrics_enabled() => SideTab::Queue,
        tab => tab,
    }
}

/// Opens or closes the sidebar on the tab it last showed.
pub(crate) fn toggle(ui: &mut Ui) {
    ui.chrome.right_open = !ui.chrome.right_open;
    if ui.chrome.right_open {
        arrive(ui);
    }
    remember(ui);
}

/// Shows `tab`, or closes the sidebar when it already shows it, as the player bar's queue,
/// lyrics and members buttons do. Lyrics stay shut while they are turned off.
pub(crate) fn show(ui: &mut Ui, tab: SideTab) {
    if tab == SideTab::Lyrics && !ui.store.settings.lyrics_enabled() {
        return;
    }
    if ui.chrome.right_open && shown_tab(ui) == tab {
        close(ui);
        return;
    }
    if ui.chrome.right_tab != tab {
        ui.store
            .settings
            .set_sidebar_right_tab(tab, &mut ui.store.cx);
    }
    ui.chrome.right_tab = tab;
    ui.chrome.right_open = true;
    arrive(ui);
    remember(ui);
}

/// Shows `tab`, leaving the sidebar open when it already shows it, where [`show`] would close it.
pub(crate) fn reveal(ui: &mut Ui, tab: SideTab) {
    if ui.chrome.right_open && shown_tab(ui) == tab {
        return;
    }
    show(ui, tab);
}

pub(crate) fn close(ui: &mut Ui) {
    ui.chrome.aside.queue.dismiss();
    ui.chrome.right_open = false;
    remember(ui);
}

/// Feeds the panels' live values before a frame: the lyrics follow the song and the queue keeps
/// the playing song in view. True when the view has to run again.
pub(crate) fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    let open = ui.chrome.right_open;
    let tab = shown_tab(ui);
    let shown = open.then_some(tab);
    if ui.chrome.aside.shown != shown {
        ui.chrome.aside.shown = shown;
        if shown.is_some() {
            arrive(ui);
        }
    }
    let queue = aside::queue::poll(ui, open && tab == SideTab::Queue, tx);
    let lyrics = aside::lyrics::poll(ui, open && tab == SideTab::Lyrics, tx);
    queue | lyrics
}

/// A panel coming into view starts where it belongs: the queue on the playing song, the lyrics
/// on the sung line.
fn arrive(ui: &mut Ui) {
    match shown_tab(ui) {
        SideTab::Queue => ui.chrome.aside.queue.anchor(),
        SideTab::Lyrics => ui.chrome.aside.lyrics.anchor(),
        SideTab::Members => {}
    }
}

fn remember(ui: &mut Ui) {
    let open = ui.chrome.right_open;
    ui.store
        .settings
        .set_sidebar_right_open(open, &mut ui.store.cx);
}
