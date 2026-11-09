//! The search field and the pill bar of tabs that float over every settings page. The page
//! under them is keyed by its tab, so a tab change fades the rows and nothing else, while the
//! bar's lit fill slides from the old tab to the new one.

use router::{Destination, SettingsPage, SettingsTab};
use rune::{Cx, El, Key, KeyEvent, Tx, col, row};
use ui::{Button, Input, Room, TabBar};

use super::layout::{HEADER, Layout};
use crate::Ui;

/// The space between the search field and the tabs.
pub(crate) const GAP: f32 = 8.;

/// The header, as wide as the content column. A search lights no tab, since its results come
/// from all of them, and a narrow column keeps the icons and leaves the names to the tooltips.
pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.settings;
    let chosen = match state.searching() {
        true => None,
        false => ui.nav.current().settings_page().map(SettingsPage::tab),
    };
    let named = layout.room().fits(Room::Roomy);

    let search = Input::new(
        "settings-search",
        &state.search,
        i18n::t!("settings-search"),
        |ui: &mut Ui| &mut ui.screens.settings.search,
    )
    .icon("icons/search.svg")
    .blurred()
    .shortcut("secondary-f")
    .clearable()
    .on_change(|ui: &mut Ui, _: &mut Tx| {
        // a search swaps the page for hits from every page, so whatever the tab was asked to
        // reveal answers for nothing once one starts
        ui.screens.settings.reveal = None;
        ui.screens.settings.popovers.close();
    })
    .on_key(|ui: &mut Ui, event: &KeyEvent, tx: &mut Tx| {
        if event.key != Key::Escape || ui.screens.settings.search.text.is_empty() {
            return false;
        }
        ui.screens.settings.search.clear();
        tx.blur();
        true
    })
    .build(theme);

    let tabs = TabBar::new("settings-tabs")
        .name(i18n::lookup("nav-settings", None))
        .max_width(layout.width)
        .blurred()
        .items(SettingsTab::ALL.map(|tab| {
            let button = Button::new(format!("settings-tab-{}", tab.id()))
                .icon(tab.icon())
                .small()
                .ghost()
                .selected(chosen == Some(tab))
                .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
                    let settings = &mut ui.screens.settings;
                    settings.search.clear();
                    settings.reveal = None;
                    ui.go(Destination::Settings(tab.lead()));
                    tx.scroll_into_view(format!("settings-tab-{}", tab.id()));
                    ui.settle();
                });
            let label = i18n::lookup(tab.key(), None);
            match named {
                true => button.label(label),
                false => button.tooltip(label),
            }
        }))
        .build(theme);

    col()
        .key(HEADER)
        .items_center()
        .w(layout.width)
        .gap(GAP)
        .py(theme.metrics.inset)
        .child(row().key("settings-search-row").w_full().child(search))
        .child(
            row()
                .key("settings-tabs-row")
                .w_full()
                .justify_center()
                .child(tabs),
        )
}
