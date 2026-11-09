//! TEMPORARY gallery wiring by shared-A to look at the shared pieces; reverted after.

use rune::{Cx, El, ScrollSpec, col};

use crate::Ui;
use crate::shared::picks::Picks;
use crate::shared::shelves::{Rails, Shelves};

#[derive(Default)]
pub struct State {
    pub page: usize,
    pub rails: Rails,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let width = cx.layout_with("content", |rect| rect[2].round()).unwrap_or(1000.)
        - theme.metrics.inset * 2.;
    let items = ui.store.home.quick_picks();
    let sections = ui.store.home.sections();
    let loading = ui.store.home.is_loading(&ui.store.session, &ui.store.library);
    let picks = Picks::mixed("quick-picks", &items, width, ui.screens.home.page)
        .title("home-quick-picks")
        .vacancy("home-quick-picks-empty")
        .loading(loading)
        .on_page(|ui: &mut Ui, page| ui.screens.home.page = page)
        .build(ui);
    let shelves = match sections.is_empty() && ui.store.home.is_feeding() {
        true => Shelves::pending("home-shelf", width, theme),
        false => Shelves::new("home-shelf", &sections, &ui.screens.home.rails, width).build(ui),
    };
    col()
        .key("screen-home")
        .w_full()
        .h_full()
        .min_h(0.)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(
            col()
                .w_full()
                .padding(theme.metrics.inset)
                .gap(32.)
                .child(picks)
                .child(shelves),
        )
}
