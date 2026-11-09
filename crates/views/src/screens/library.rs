//! TEMPORARY gallery wiring by shared-A to look at the shared pieces; reverted after.

use i18n::t;
use router::{Destination, LibraryTab};
use rune::{Cx, El, ScrollSpec, col, row};
use state::{Origin, Shelf};

use crate::Ui;
use crate::shared::album_grid::album_grid;
use crate::shared::hero::{Hero, Listing, MetaStrip, PlayButton, action_gap};
use crate::shared::tracks::{LIBRARY_COLUMNS, TrackList, TrackRows, TrackTable};

#[derive(Default)]
pub struct State {
    pub songs: TrackList,
}

fn rows() -> TrackRows {
    TrackRows::new(
        |ui: &Ui| ui.store.library.state(Shelf::Local).tracks(),
        |ui: &Ui| &ui.screens.library.songs,
        |ui: &mut Ui| &mut ui.screens.library.songs,
        LIBRARY_COLUMNS,
    )
    .origin(Some(Origin::local()))
    .starrable(Shelf::Local)
    .shelved(Shelf::Local)
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let width = cx.layout_with("content", |rect| rect[2].round()).unwrap_or(1000.);
    let tab = match ui.nav.current() {
        Destination::Local(tab) | Destination::Library(tab) => tab,
        _ => LibraryTab::Songs,
    };
    let rows = rows();
    let tracks = rows.tracks(ui);
    let total: std::time::Duration = tracks.iter().map(|track| track.duration).sum();
    let meta = MetaStrip::new()
        .text(t!("count-songs", count = tracks.len()).into_owned())
        .text(ui::runtime(total).into_owned())
        .build(theme);
    let actions = row()
        .center_y()
        .gap(action_gap(theme))
        .child(PlayButton::new("play-library", Listing::Listed(rows.clone())).build(ui))
        .child(PlayButton::shuffle("shuffle-library", Listing::Listed(rows.clone())).build(ui));
    let hero = Hero::new("library-hero", t!("nav-songs").into_owned())
        .accent()
        .eyebrow(t!("nav-local").into_owned())
        .fallback("icons/disc-3.svg")
        .meta(meta)
        .actions(actions)
        .padded()
        .build(ui);
    match tab {
        LibraryTab::Albums => col()
            .key("screen-library")
            .w_full()
            .h_full()
            .min_h(0.)
            .scroll(ScrollSpec { stick_bottom: false })
            .child(hero)
            .child(
                col().px(theme.metrics.inset).child(album_grid(
                    ui,
                    "library-albums",
                    width - theme.metrics.inset * 2.,
                    |ui: &Ui| ui.store.library.state(Shelf::Local).albums(),
                    None,
                )),
            ),
        _ => col()
            .key("screen-library")
            .w_full()
            .h_full()
            .min_h(0.)
            .child(
                TrackTable::new("library-songs", &rows)
                    .width(width)
                    .lead(hero)
                    .remember("library-local-songs")
                    .label(t!("nav-songs").into_owned())
                    .build(ui),
            ),
    }
}
