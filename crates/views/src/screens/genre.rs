//! A genre's page: its name over its shelves, each a rail of cards or, in list mode, lanes of
//! rows. The title bar's view switch picks between the two.

use std::rc::Rc;

use i18n::t;
use music::GenreItem;
use router::Destination;
use rune::{Cx, El, PointerPress, ScrollSpec, Tx, col, text};
use state::GenreDetails;
use ui::{Mode, Skeleton, Text, Weight, font};

use crate::Ui;
use crate::chrome::toolbar::Tools;
use crate::chrome::{player_bar, tools};
use crate::shared::cards::OnItem;
use crate::shared::menus::{self, Target};
use crate::shared::page::{self, rems};
use crate::shared::shelves::{Rails, Shelves};
use crate::shared::trouble;

/// How wide the title skeleton reaches while the genre's name is on its way.
const TITLE_SKELETON: f32 = 240.;
/// The settings key the view mode is kept under.
const SECTION: &str = "genre";
/// The room between the title and the shelves, in rems.
const GAP: f32 = 2.;

/// The genre page's own state: the scroll of each of its rails.
#[derive(Default)]
pub struct State {
    rails: Rails,
}

impl State {
    /// Starts every rail of a genre arrived at from its first card.
    pub(crate) fn arrived(&mut self, destination: &Destination) {
        if let Destination::Genre(_) = destination {
            self.rails.reset();
        }
    }
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let detail = ui.store.pages.genre.as_ref();
    if let Some(failure) = detail.and_then(|detail| failure(ui, detail)) {
        return col()
            .key("genre-failure")
            .role("alert")
            .w_full()
            .h_full()
            .center()
            .child(failure);
    }
    let inset = theme.metrics.inset;
    let width = page::content_width(cx, inset * 2.);
    let loading = detail.is_some_and(GenreDetails::is_loading);
    let sections = detail.map(GenreDetails::sections).unwrap_or_default();
    let empty = !loading && sections.is_empty();
    let title = detail.and_then(GenreDetails::name).map(|name| {
        text(name.to_owned())
            .key("genre-title")
            .role("heading")
            .level(1)
            .label(name.to_owned())
            .font(font(theme, Text::Display, Weight::BOLD))
            .color(theme.foreground.color())
            .truncate()
            .min_w(0.)
    });
    let title = title.or_else(|| {
        loading.then(|| {
            Skeleton::new()
                .build(theme)
                .size(TITLE_SKELETON, theme.text(Text::Display))
        })
    });
    let shelves = match loading && sections.is_empty() {
        true => Shelves::pending("genre-shelf", width, theme),
        false => {
            let opener: OnItem = Rc::new(
                |ui: &mut Ui, item: GenreItem, press: PointerPress, _: &mut Tx| {
                    if let Some(target) = Target::of(&item) {
                        menus::open(ui, target, press);
                    }
                },
            );
            Shelves::new("genre-shelf", &sections, &ui.screens.genre.rails, width)
                .mode(mode(ui))
                .on_item(opener)
                .build(ui)
        }
    };
    col()
        .key("genre-page")
        .w_full()
        .h_full()
        .min_h(0.)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .px(inset)
        .pt(inset)
        .pb(inset + player_bar::bottom(ui, cx))
        .gap(rems(theme, GAP))
        .children(title)
        .children(
            empty
                .then(|| ui::spoken_vacant("genre-empty", t!("genre-empty"), theme).role("status")),
        )
        .child(shelves)
}

/// The title bar's tools for a genre: the switch between rails and lanes.
pub(crate) fn tools(ui: &Ui) -> Tools {
    Tools {
        items: vec![tools::views(ui, mode(ui), |ui: &mut Ui, mode: Mode| {
            let store = &mut ui.store;
            store.settings.set_view(SECTION, mode, &mut store.cx);
        })],
        search: None,
    }
}

/// Whether the shelves show as rails of tiles or as lanes of rows.
fn mode(ui: &Ui) -> Mode {
    ui.store.settings.view_or(SECTION, Mode::Grid)
}

/// The page a genre that did not load shows instead of its shelves. Opening the same genre again
/// is the retry, since a failed load leaves the page empty.
fn failure(ui: &Ui, detail: &GenreDetails) -> Option<El> {
    let id = detail.id()?.to_owned();
    let reason = match trouble::unreachable(ui, &id) {
        true => None,
        false => Some(detail.error()?.to_owned()),
    };
    Some(trouble::lost(
        ui,
        "genre-lost",
        t!("trouble-not-loaded"),
        reason.as_deref(),
        move |ui: &mut Ui| ui.store.open_genre(&id),
    ))
}
