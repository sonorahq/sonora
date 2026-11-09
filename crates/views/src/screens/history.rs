//! The listening history: a hero with the play count and Clear history over the virtualised
//! table of what played, newest first, with a line under the header while it holds nothing.

use i18n::t;
use music::Track;
use rune::{Cx, Drag, El, PointerPress, Transition, Tx, row};
use state::HistoryState;
use ui::{Button, Modal, Pending};

use crate::chrome::{player_bar, sidebar_left};
use crate::shared::confirm;
use crate::shared::hero::{Hero, MetaStrip};
use crate::shared::menus::{self, TrackColumns};
use crate::shared::page;
use crate::shared::pins::Pinned as _;
use crate::shared::tracks::{HISTORY_COLUMNS, Picked, TrackList, TrackRows, TrackTable};
use crate::{Ui, act};

/// The history page's own state: its table, and whether Clear history is asking first.
#[derive(Default)]
pub struct State {
    list: TrackList,
    clearing: bool,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let width = page::content_width(cx, 0.);
    let rows = rows(ui);
    let history = &ui.store.history;
    let tracks = history.tracks();
    let duration: std::time::Duration = tracks.iter().map(|track| track.duration).sum();
    let count = tracks.len();

    let mut strip = MetaStrip::new().text(t!("count-songs", count = count).into_owned());
    if !duration.is_zero() {
        strip = strip.text(ui::runtime(duration).into_owned());
    }
    let clear = Button::new("clear-history")
        .outline()
        .icon("icons/trash-2.svg")
        .label(t!("history-clear"))
        .disabled(count == 0)
        .on_press(|ui: &mut Ui| ui.screens.history.clearing = true)
        .build(theme);
    let hero = Hero::new("history-hero", t!("nav-history").into_owned())
        .fallback("icons/rotate-ccw-clock.svg")
        .accent()
        .eyebrow(t!("detail-playlist").into_owned())
        .meta(strip.build(theme))
        .actions(row().center_y().child(clear))
        .padded()
        .build(ui);

    let loading = matches!(history.state(), HistoryState::Loading);
    let mut table = TrackTable::new("history-tracks", &rows)
        .width(width)
        .lead(hero)
        .pending(loading.then_some(Pending::Screen))
        .label(t!("nav-history").into_owned())
        .bottom(theme.metrics.inset + player_bar::bottom(ui, cx))
        .on_context(
            |ui: &mut Ui, picked: Picked, press: PointerPress, _: &mut Tx| {
                let columns = TrackColumns {
                    album: picked.album,
                    artists: picked.artists,
                };
                menus::open_tracks(ui, picked.tracks, menus::TrackList::History, columns, press);
            },
        )
        .on_remove(|ui: &mut Ui, picked: Picked, _: &mut Tx| {
            confirm::history_songs(ui, picked.tracks)
        })
        .on_row_drag(
            |ui: &mut Ui, tracks: &[Track], drag: &mut Drag, tx: &mut Tx| {
                if let Some(pin) = tracks.first().and_then(|track| track.pin()) {
                    sidebar_left::carry(ui, &pin, drag, tx);
                }
            },
        );
    if let Some(note) = note(ui, &rows) {
        // The note fades in as the last row goes, after a clear or a search that matches nothing.
        table = table.foot(
            ui::spoken_vacant("history-empty", note, theme)
                .role("status")
                .transition(Transition::opacity()),
        );
    }
    if ui.screens.history.clearing {
        confirmation(ui, layer);
    }
    table.build(ui)
}

fn tracks(ui: &Ui) -> &[Track] {
    ui.store.history.tracks()
}

fn list(ui: &Ui) -> &TrackList {
    &ui.screens.history.list
}

fn list_mut(ui: &mut Ui) -> &mut TrackList {
    &mut ui.screens.history.list
}

/// The history's rows, narrowed by what the title bar's search field holds.
fn rows(ui: &Ui) -> TrackRows {
    TrackRows::new(tracks, list, list_mut, HISTORY_COLUMNS)
        .liked()
        .query(ui.chrome.toolbar.query(&ui.nav.current()))
}

/// What the page says under the header when no row shows: that the history did not load, that
/// the search matches nothing, or that nothing has played yet.
fn note(ui: &Ui, rows: &TrackRows) -> Option<String> {
    match ui.store.history.state() {
        HistoryState::Loading => None,
        HistoryState::Failed => Some(t!("history-not-loaded").into_owned()),
        _ if !rows.order(ui).is_empty() => None,
        _ if !ui.chrome.toolbar.query(&ui.nav.current()).is_empty() || rows.list(ui).filtered() => {
            Some(t!("library-no-matches").into_owned())
        }
        _ => Some(t!("history-empty").into_owned()),
    }
}

/// Asks before Clear history forgets every play.
fn confirmation(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    Modal::new("clear-history", t!("history-clear-title"))
        .detail(t!("history-clear-confirm"))
        .action(
            Button::new("cancel-clear-history")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(dismiss)
                .build(theme),
        )
        .action(
            Button::new("apply-clear-history")
                .destructive()
                .label(t!("common-delete"))
                .on_press(act(|ui: &mut Ui| {
                    ui.store.clear_history();
                    dismiss(ui);
                }))
                .build(theme)
                .default_button(),
        )
        .on_dismiss(dismiss)
        .build(theme, layer);
}

fn dismiss(ui: &mut Ui) {
    ui.screens.history.clearing = false;
}
