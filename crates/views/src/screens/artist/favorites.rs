//! The listener's favorites by the artist: the link under the hero that counts them, and the
//! dialog that lists them without adding them to the page.

use std::borrow::Cow;
use std::rc::Rc;

use i18n::t;
use music::Track;
use rune::{El, GrowthSpec, ScrollSpec, col, row};
use state::Origin;
use ui::{Button, Modal, Room};

use crate::Ui;
use crate::shared::page::rems;
use crate::shared::track_card::{RemoveHandler, StartHandler, TrackCard};

/// The dialog gives two song columns enough room for titles and controls, in covers.
const WIDTH: f32 = 5.;
/// How tall the list gets before it scrolls, in rows.
const ROWS: f32 = 8.;
/// The space under the link, and between the dialog's columns, in rems.
const LINK_GAP: f32 = 0.75;
const COLUMN_GAP: f32 = 0.5;

/// The link that opens the dialog, or nothing while the listener has no favorites by `id`.
pub(super) fn link(ui: &Ui, id: &str) -> Option<El> {
    let count = ui.store.library.favorites_by_artist(id).count();
    if count == 0 {
        return None;
    }
    let theme = &ui.theme;
    Some(
        col()
            .key("artist-favorites-link")
            .w_full()
            .pb(rems(theme, LINK_GAP))
            .child(
                Button::new("artist-favorites")
                    .icon("icons/heart.svg")
                    .label(t!("artist-favorite-songs-count", count = count))
                    .trailing("icons/chevron-right.svg")
                    .ghost()
                    .on_press(|ui: &mut Ui| ui.screens.artist.favorites = true)
                    .build(theme)
                    .w_full(),
            ),
    )
}

/// Mounts the dialog into `layer` while it is open: the favorites by `id` as track cards in one
/// column, or two on a roomy window, in a virtualised list that scrolls on its own.
pub(super) fn dialog(ui: &Ui, id: &str, origin: Option<Origin>, room: Room, layer: &mut Vec<El>) {
    if !ui.screens.artist.favorites {
        return;
    }
    let theme = &ui.theme;
    let tracks: Rc<Vec<Track>> =
        Rc::new(ui.store.library.favorites_by_artist(id).cloned().collect());
    let count = tracks.len();
    let columns = match room.fits(Room::Roomy) {
        true => 2,
        false => 1,
    };
    let queued = tracks.clone();
    let start: StartHandler = Rc::new(move |ui: &mut Ui, place| {
        ui.store
            .play_tracks(queued.as_ref().clone(), place, origin.clone());
    });
    let listed = tracks.clone();
    let remove: RemoveHandler = Rc::new(move |ui: &mut Ui, place| {
        let Some(track) = listed.get(place).cloned() else {
            return;
        };
        let store = &mut ui.store;
        store
            .library
            .save_tracks(vec![track], false, &store.session, &mut store.cx);
        ui.settle();
    });
    let height = theme.metrics.list_row.round();
    let rows = count.div_ceil(columns);
    let keyed = tracks.clone();
    let list = col()
        .key("artist-favorite-songs")
        .w_full()
        .max_h(height * ROWS)
        .min_h(0.)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("artist-favorite-rows")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    rows,
                    height,
                    move |_: &Ui, at: usize| {
                        let id = keyed
                            .get(at * columns)
                            .and_then(|track| track.id.as_deref())
                            .unwrap_or_default();
                        Cow::Owned(format!("row-{at}-{id}"))
                    },
                    move |ui: &Ui, at: usize| {
                        let cells = (0..columns).map(|column| {
                            let place = at * columns + column;
                            let card = TrackCard::new("artist-favorite-song", place, &tracks)
                                .detailed(true)
                                .start(Some(start.clone()))
                                .remove("menu-remove-from-library", remove.clone())
                                .build(ui);
                            col()
                                .key(format!("column-{column}"))
                                .grow(1.)
                                .shrink()
                                .min_w(0.)
                                .children(card)
                        });
                        row()
                            .w_full()
                            .gap(rems(&ui.theme, COLUMN_GAP))
                            .children(cells)
                    },
                ),
        );
    let mut modal = Modal::new("artist-favorites-dialog", t!("artist-favorite-songs"))
        .width(theme.metrics.cover * WIDTH)
        .close_button()
        .on_dismiss(|ui: &mut Ui| ui.screens.artist.favorites = false);
    modal = match count {
        0 => modal.child(ui::vacant(t!("library-no-songs"), theme)),
        _ => modal.child(list),
    };
    modal.build(theme, layer);
}
