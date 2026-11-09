//! The General page: where Sonora starts and in which language, the window's frame, motion and
//! context menus.

use i18n::{Language, t};
use router::{NavEntry, Screen};
use rune::{Cx, El, Tx};
use ui::{Menu, MenuItem, OptionCard, Pace, Picker, Rounding, Saver, Stillness};

use super::appearance::corner;
use super::{Layout, Row, columns, grid, group, matches_query, segmented, stacked, switch};
use crate::{Ui, act};

const STARTUP: &str = "startup";
const ENTRIES: &str = "entries";
const LANGUAGES: &str = "languages";
/// How far a control that cannot be used fades.
const FADED: f32 = 0.4;

pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut window = vec![
        switch(
            theme,
            "close-to-tray",
            t!("settings-close-to-tray"),
            settings.close_to_tray(),
            |ui, on| ui.store.settings.set_close_to_tray(on, &mut ui.store.cx),
        ),
        switch(
            theme,
            "tray-icon",
            t!("settings-tray-icon"),
            settings.tray_icon(),
            |ui, on| ui.store.settings.set_tray_icon(on, &mut ui.store.cx),
        ),
    ];
    window.extend(decorations(ui, layer, layout));
    vec![
        group(
            theme,
            Some(("settings-group-startup", "icons/house.svg")),
            None,
            [startup(ui, layer), entries(ui, layer), language(ui, layer)],
        ),
        group(
            theme,
            Some(("settings-group-window", "icons/panel-left-open.svg")),
            None,
            window,
        ),
        group(
            theme,
            Some(("settings-group-motion", "icons/loader-circle.svg")),
            None,
            [motion(ui), pace(ui), saver(ui)],
        ),
        group(
            theme,
            Some(("settings-group-context-menus", "icons/list.svg")),
            None,
            [switch(
                theme,
                "adaptive-menu",
                t!("settings-adaptive-menu"),
                settings.adaptive_menu(),
                |ui, on| ui.store.settings.set_adaptive_menu(on, &mut ui.store.cx),
            )],
        ),
    ]
}

/// The rows about the window's own frame, which differ by platform.
#[allow(unused_variables, reason = "some platforms offer fewer of these rows")]
fn decorations(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut rows = Vec::new();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        rows.push(switch(
            theme,
            "server-side-decorations",
            t!("settings-server-side-decorations"),
            settings.server_side_decorations(),
            |ui, on| {
                ui.store
                    .settings
                    .set_server_side_decorations(on, &mut ui.store.cx)
            },
        ));
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        rows.push(switch(
            theme,
            "window-controls",
            t!("settings-window-controls"),
            settings.window_controls(),
            |ui, on| ui.store.settings.set_window_controls(on, &mut ui.store.cx),
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        rows.push(side(ui));
        rows.push(switch(
            theme,
            "traffic-light-controls",
            t!("settings-traffic-light-controls"),
            settings.traffic_light_controls(),
            |ui, on| {
                ui.store
                    .settings
                    .set_traffic_light_controls(on, &mut ui.store.cx)
            },
        ));
    }
    // Server-side decorations put the compositor in charge of the frame, so window rounding is
    // only ever this app's call with client-side ones.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let rounds = !settings.server_side_decorations();
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let rounds = true;
    if rounds {
        rows.push(window_rounding(ui, layer, layout));
    }
    rows
}

fn startup(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let chosen = Screen::from_id(ui.store.settings.startup()).unwrap_or(Screen::Home);
    let guest = !ui.store.session.authenticated();
    let picker = Picker::new(
        STARTUP,
        &ui.screens.settings.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        i18n::lookup(chosen.key(), None),
    )
    .name(t!("settings-startup"))
    .width(Picker::<Ui>::NARROW)
    .items(Screen::ALL.map(|screen| {
        let item =
            MenuItem::new(screen.id(), i18n::lookup(screen.key(), None)).selected(screen == chosen);
        match guest && screen.needs_account() {
            true => item.disabled().tooltip(t!("settings-startup-no-guest")),
            false => item.on_press(act(move |ui: &mut Ui| {
                ui.store.settings.set_startup(screen.id(), &mut ui.store.cx)
            })),
        }
    }))
    .build(theme, layer);
    Row::new("startup", t!("settings-startup"), picker).build(theme)
}

fn entries(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let picker = Picker::new(
        ENTRIES,
        &ui.screens.settings.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        t!("settings-entries-pick"),
    )
    .name(t!("settings-entries"))
    .width(Picker::<Ui>::REGULAR)
    .sticky()
    .items(NavEntry::ALL.map(|entry| {
        let shown = settings.nav_shown(entry.id());
        MenuItem::new(entry.id(), i18n::lookup(entry.key(), None))
            .selected(shown)
            .stays_open()
            .on_press(act(move |ui: &mut Ui| {
                ui.store
                    .settings
                    .set_nav_shown(entry.id(), !shown, &mut ui.store.cx)
            }))
    }))
    .build(theme, layer);
    Row::new("entries", t!("settings-entries"), picker).build(theme)
}

/// The language picker, with a search over the languages that takes the keys as it opens.
fn language(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.settings;
    let chosen = ui.store.settings.language().to_owned();
    let current = match Language::from_id(&chosen) {
        Some(language) => language.label().to_owned(),
        None => t!("settings-language-system").into_owned(),
    };
    let asked = state.languages.query();
    let entries: Vec<(&'static str, String)> =
        std::iter::once((i18n::AUTO, t!("settings-language-system").into_owned()))
            .chain(
                Language::ALL
                    .into_iter()
                    .map(|language| (language.id(), language.label().to_owned())),
            )
            .filter(|(id, label)| matches_query(id, label, &asked))
            .collect();
    let barren = entries.is_empty();
    let mut items: Vec<MenuItem<Ui>> = entries
        .into_iter()
        .map(|(id, label)| {
            MenuItem::new(format!("language-{id}"), label)
                .checked(chosen == id)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.settings.set_language(id, &mut ui.store.cx)
                }))
        })
        .collect();
    if barren {
        items.push(MenuItem::new("language-empty", t!("settings-language-none")).disabled());
    }
    let menu = Menu::new("languages-menu")
        .width(Picker::<Ui>::WIDE)
        .max_height(theme.metrics.list_row * 7.)
        .search(
            &state.languages,
            t!("settings-language-search"),
            |ui: &mut Ui| &mut ui.screens.settings.languages,
        );
    let picker = Picker::new(
        LANGUAGES,
        &state.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        current,
    )
    .name(t!("settings-language"))
    .width(Picker::<Ui>::WIDE)
    .menu(menu)
    .items(items)
    .on_open(|ui: &mut Ui, _: &mut Tx| ui.screens.settings.languages.reset())
    .build(theme, layer);
    Row::new("language", t!("settings-language"), picker).build(theme)
}

/// Which side the window's controls stand on. It only means something while Sonora draws its
/// own controls, so it fades and holds still otherwise.
#[cfg(not(target_os = "macos"))]
fn side(ui: &Ui) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let left = settings.controls_on_left();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let shown = !settings.server_side_decorations();
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let shown = settings.window_controls();
    let mut sides = segmented(
        theme,
        "controls-side",
        t!("settings-controls-side"),
        [
            (true, t!("common-left").into_owned()),
            (false, t!("common-right").into_owned()),
        ],
        left,
        move |ui, left| {
            if shown {
                ui.store
                    .settings
                    .set_controls_on_left(left, &mut ui.store.cx);
            }
        },
    );
    if !shown {
        sides = sides.opacity(FADED);
    }
    Row::new("controls-side", t!("settings-controls-side"), sides).build(theme)
}

/// The window's corners as one card per rounding, each showing a corner at that radius. AppKit
/// has no square window, so macOS offers only the corners it can draw.
fn window_rounding(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let current = ui.store.settings.window_rounding();
    let offered: Vec<Rounding> = Rounding::ALL
        .into_iter()
        .filter(|rounding| cfg!(not(target_os = "macos")) || *rounding != Rounding::Square)
        .collect();
    let cards = offered.iter().copied().map(|rounding| {
        OptionCard::new(format!("window-{}", rounding.id()))
            .thumb(corner(theme, rounding))
            .label(rounding.label())
            .selected(current == rounding)
            .on_press(act(move |ui: &mut Ui| {
                ui.store
                    .settings
                    .set_window_rounding(rounding, &mut ui.store.cx)
            }))
            .build(theme, layer)
    });
    let cards = grid(
        "window-rounding",
        columns(offered.len(), layout.room()),
        cards.collect::<Vec<_>>(),
    );
    stacked(
        theme,
        "window-rounding",
        t!("settings-window-rounding"),
        cards,
    )
}

fn motion(ui: &Ui) -> El {
    let theme = &ui.theme;
    let choices = segmented(
        theme,
        "motion",
        t!("settings-motion"),
        Stillness::ALL.map(|choice| (choice, choice.label().into_owned())),
        ui.store.settings.stillness(),
        |ui, choice| ui.store.settings.set_stillness(choice, &mut ui.store.cx),
    );
    Row::new("motion", t!("settings-motion"), choices).build(theme)
}

fn pace(ui: &Ui) -> El {
    let theme = &ui.theme;
    let choices = segmented(
        theme,
        "pace",
        t!("settings-pace"),
        Pace::ALL.map(|choice| (choice, choice.label().into_owned())),
        ui.store.settings.pace(),
        |ui, choice| ui.store.settings.set_pace(choice, &mut ui.store.cx),
    );
    Row::new("pace", t!("settings-pace"), choices).build(theme)
}

fn saver(ui: &Ui) -> El {
    let theme = &ui.theme;
    let choices = segmented(
        theme,
        "saver",
        t!("settings-saver"),
        Saver::ALL.map(|choice| (choice, choice.label().into_owned())),
        ui.store.settings.saver(),
        |ui, choice| ui.store.settings.set_saver(choice, &mut ui.store.cx),
    );
    Row::new("saver", t!("settings-saver"), choices)
        .detail(t!("settings-saver-detail"))
        .build(theme)
}
