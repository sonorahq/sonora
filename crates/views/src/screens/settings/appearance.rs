//! The Appearance page: the theme, the icon pack, the corners, the typeface and its size, and
//! how see-through and blurred the window is.

use std::sync::{Arc, Mutex};

use i18n::t;
use rune::{Cx, El, Tx, el, icon, row, text};
use state::{Job, SYSTEM_FONT};
use ui::{
    Button, MAX_FONT, MAX_TRANSPARENCY, MIN_FONT, Menu, MenuItem, OptionCard, Picker, Rounding,
    Scrubber, Text, Theme, ThemeKind, Weight, font,
};

use super::{Layout, Row, columns, grid, group, matches_query, outward, switch, themes, whole};
use crate::{Ui, act};

/// The website's gallery of themes people made, whose install buttons hand a theme to Sonora.
const COMMUNITY_THEMES: &str = "https://sonorahq.org/themes";
/// How large a pack's sample glyphs draw on its card.
const SAMPLE: f32 = 14.;
const SAMPLE_GAP: f32 = 8.;
const TYPEFACES: &str = "typefaces";
/// How many faces the picker lists at most.
const TYPEFACE_LIMIT: usize = 200;
/// The space between the font size's steps and its value.
const STEP_GAP: f32 = 8.;
/// Where the corner a rounding card draws starts, as parts of the thumbnail.
const CORNER_LEFT: f32 = 0.25;
const CORNER_TOP: f32 = 0.3;
const CORNER_EDGE: f32 = 2.;
/// The space option cards keep between them.
const CARDS_GAP: f32 = 12.;

/// The typefaces installed on the system, read once off the UI thread the first time the
/// typeface row shows.
#[derive(Default)]
pub struct Fonts {
    names: Arc<Mutex<Option<Vec<String>>>>,
    job: Option<Job>,
}

impl Fonts {
    /// The installed faces, or `None` while they are being read.
    fn names(&self) -> Option<Vec<String>> {
        self.names
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn loading(&self) -> bool {
        self.job.is_some() && self.names().is_none()
    }
}

pub(super) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut window = vec![opacity(ui)];
    if ui::WINDOW_BLUR {
        let opaque = !settings.transparent();
        window.push(
            Row::new(
                "blur-window",
                t!("settings-blur-window"),
                super::toggle(
                    theme,
                    "blur-window",
                    t!("settings-blur-window"),
                    settings.blur_window() && !opaque,
                    opaque,
                    |ui, on| ui.store.settings.set_blur_window(on, &mut ui.store.cx),
                ),
            )
            .disabled(opaque)
            .build(theme),
        );
    }
    window.push(switch(
        theme,
        "blur",
        t!("settings-blur"),
        settings.blur(),
        |ui, on| ui.store.settings.set_blur(on, &mut ui.store.cx),
    ));
    vec![
        group(
            theme,
            Some(("settings-theme", "icons/palette.svg")),
            Some(themes::cards(ui, cx, layer)),
            [adaptive(ui), community(theme)],
        ),
        group(
            theme,
            Some(("settings-icons", "icons/layout-grid.svg")),
            Some(icon_cards(ui, layer, layout)),
            [],
        ),
        group(
            theme,
            Some(("settings-corners", "icons/maximize.svg")),
            Some(corner_cards(ui, layer, layout)),
            [],
        ),
        group(
            theme,
            Some(("settings-group-text", "icons/text-select.svg")),
            None,
            [typeface(ui, layer), font_size(ui)],
        ),
        group(
            theme,
            Some(("settings-group-window", "icons/panel-left-open.svg")),
            None,
            window,
        ),
    ]
}

/// The adaptive theme lays the cover's tint over a neutral theme, so turning it on with a theme
/// of its own chosen goes back to Dark.
fn adaptive(ui: &Ui) -> El {
    let settings = &ui.store.settings;
    let neutral = ThemeKind::ALL
        .into_iter()
        .any(|kind| kind.id() == settings.theme());
    switch(
        &ui.theme,
        "adaptive-theme",
        t!("settings-adaptive"),
        settings.adaptive_theme(),
        move |ui, adaptive| {
            let settings = &mut ui.store.settings;
            settings.set_adaptive_theme(adaptive, &mut ui.store.cx);
            if adaptive && !neutral {
                settings.set_theme(ThemeKind::Dark.id(), &mut ui.store.cx);
            }
        },
    )
}

fn community(theme: &Theme) -> El {
    whole(
        outward("community-themes", t!("settings-theme-explore"), None).on_press_tx(
            |_: &mut Ui, tx: &mut Tx| {
                if let Err(error) = tx.open_url(COMMUNITY_THEMES) {
                    log::warn!("settings: cannot open the theme gallery: {error:?}");
                }
            },
        ),
        theme,
    )
}

/// How wide one of `count` option cards stands across the column at its room.
pub(super) fn card_width(layout: &Layout, count: usize) -> f32 {
    let columns = columns(count, layout.room()) as f32;
    ((layout.width - CARDS_GAP * (columns - 1.)) / columns).max(0.)
}

/// One card per icon pack, its thumbnail drawn with five of the pack's own glyphs.
fn icon_cards(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let chosen = ui.store.settings.icons().to_owned();
    let current = icons::pack(&chosen).unwrap_or_else(icons::active);
    let packs: Vec<&'static icons::Pack> = icons::packs().collect();
    let count = packs.len();
    let cards =
        packs.into_iter().map(|pack| {
            let samples = row().w_full().h_full().center().gap(SAMPLE_GAP).children(
                icons::SAMPLES.iter().map(|name| {
                    icon(icons::get_in(pack, name), SAMPLE).color(theme.foreground.color())
                }),
            );
            let id = pack.id;
            OptionCard::new(format!("icons-{}", pack.id))
                .thumb_height(theme.metrics.list_row * 1.1)
                .thumb(samples)
                .label(pack.title())
                .selected(pack.id == current.id)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.settings.set_icons(id, &mut ui.store.cx)
                }))
                .build(theme, layer)
        });
    grid(
        "icons",
        columns(count, layout.room()),
        cards.collect::<Vec<_>>(),
    )
}

/// One card per corner rounding, its thumbnail the corner of a surface drawn at that radius.
fn corner_cards(ui: &Ui, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let current = Rounding::from_id(ui.store.settings.rounding());
    let width = card_width(layout, Rounding::ALL.len());
    let cards = Rounding::ALL.into_iter().map(|rounding| {
        OptionCard::new(format!("corners-{}", rounding.id()))
            .thumb(corner_in(theme, rounding, width))
            .label(rounding.label())
            .selected(current == rounding)
            .on_press(act(move |ui: &mut Ui| {
                ui.store
                    .settings
                    .set_rounding(rounding.id(), &mut ui.store.cx)
            }))
            .build(theme, layer)
    });
    grid(
        "corners",
        columns(Rounding::ALL.len(), layout.room()),
        cards.collect::<Vec<_>>(),
    )
}

/// The top left corner of a surface drawn at `rounding`'s radius, the rest of it cut off by
/// the thumbnail, for a card about the default width.
pub(super) fn corner(theme: &Theme, rounding: Rounding) -> El {
    corner_in(theme, rounding, theme.metrics.cover)
}

/// [`corner`] in a thumbnail `width` wide.
pub(super) fn corner_in(theme: &Theme, rounding: Rounding, width: f32) -> El {
    let height = theme.metrics.list_row * 1.5;
    el().w_full().h_full().child(
        el().absolute(width * CORNER_LEFT, height * CORNER_TOP)
            .size(width, height)
            .radius(rounding.radius())
            .border(CORNER_EDGE, theme.muted_foreground.color())
            .fill(theme.secondary.color()),
    )
}

/// The typeface picker: the bundled face, then every face the system has, with a search over
/// them. The faces are read the first time the row lays out.
fn typeface(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.settings;
    let chosen = ui.store.settings.font().to_owned();
    let current = match chosen == SYSTEM_FONT {
        true => t!("settings-typeface-system").into_owned(),
        false => chosen.clone(),
    };
    let picking = state.popovers.shows(TYPEFACES);
    let mut items: Vec<MenuItem<Ui>> = Vec::new();
    if picking {
        let asked = state.typefaces.query();
        let installed = state.fonts.names().unwrap_or_default();
        let entries: Vec<(String, String)> = std::iter::once((
            SYSTEM_FONT.to_owned(),
            t!("settings-typeface-system").into_owned(),
        ))
        .chain(installed.into_iter().map(|name| (name.clone(), name)))
        .filter(|(id, label)| matches_query(id, label, &asked))
        .take(TYPEFACE_LIMIT)
        .collect();
        let barren = entries.is_empty();
        items = entries
            .into_iter()
            .enumerate()
            .map(|(place, (name, label))| {
                let checked = chosen == name;
                MenuItem::new(format!("typeface-{place}"), label)
                    .checked(checked)
                    .on_press(act(move |ui: &mut Ui| {
                        ui.store.settings.set_font(name.clone(), &mut ui.store.cx)
                    }))
            })
            .collect();
        if state.fonts.loading() {
            items.push(
                MenuItem::new("typeface-loading", t!("settings-typeface-loading")).disabled(),
            );
        } else if barren {
            items.push(MenuItem::new("typeface-empty", t!("settings-typeface-none")).disabled());
        }
    }
    let menu = Menu::new("typefaces-menu")
        .width(Picker::<Ui>::WIDE)
        .max_height(theme.metrics.list_row * 7.)
        .search(
            &state.typefaces,
            t!("settings-typeface-search"),
            |ui: &mut Ui| &mut ui.screens.settings.typefaces,
        );
    let picker = Picker::new(
        TYPEFACES,
        &state.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        current,
    )
    .name(t!("settings-typeface"))
    .width(Picker::<Ui>::WIDE)
    .menu(menu)
    .items(items)
    .on_open(|ui: &mut Ui, _: &mut Tx| {
        ui.screens.settings.typefaces.reset();
        load_fonts(ui);
    })
    .build(theme, layer);
    Row::new("typeface", t!("settings-typeface"), picker)
        .build(theme)
        .on_layout(|ui: &mut Ui, _: [f32; 4]| load_fonts(ui))
}

/// Starts reading the installed faces, unless they are read or being read.
fn load_fonts(ui: &mut Ui) {
    let fonts = &mut ui.screens.settings.fonts;
    if fonts.job.is_some() {
        return;
    }
    let names = fonts.names.clone();
    let io = ui.store.cx.io.clone();
    fonts.job = Some(ui.store.cx.spawn(move |link| async move {
        let found = io.spawn_blocking(installed_fonts).await.unwrap_or_default();
        *names
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(found);
        link.post(|store| store.cx.refresh());
    }));
}

/// The families fontconfig knows, sorted, without the hidden ones. Other systems offer the
/// bundled face alone.
fn installed_fonts() -> Vec<String> {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        let Ok(out) = std::process::Command::new("fc-list")
            .args(["--format", "%{family[0]}\n"])
            .output()
        else {
            return Vec::new();
        };
        let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.starts_with('.'))
            .map(str::to_owned)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }
    #[cfg(not(all(unix, not(target_vendor = "apple"))))]
    {
        Vec::new()
    }
}

/// The text size, a step down and a step up around the size it is now.
fn font_size(ui: &Ui) -> El {
    let theme = &ui.theme;
    let size = ui.store.settings.font_size();
    let step = |key: &str, label: &str, name: &str, delta: f32| {
        let wanted = (size + delta).clamp(MIN_FONT, MAX_FONT);
        Button::new(key)
            .label(label)
            .name(i18n::lookup(name, None))
            .small()
            .outline()
            .disabled(wanted == size)
            .on_press(act(move |ui: &mut Ui| {
                ui.store.settings.set_font_size(wanted, &mut ui.store.cx)
            }))
            .build(theme)
    };
    let value = t!("settings-font-value", size = size.round() as i64).into_owned();
    let actions = row()
        .key("font-steps")
        .center_y()
        .gap(STEP_GAP)
        .child(step("font-smaller", "−", "settings-font-smaller", -1.))
        .child(
            text(value.clone())
                .key("settings-font-value")
                .role("text")
                .label(value)
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.foreground.color()),
        )
        .child(step("font-larger", "+", "settings-font-larger", 1.));
    Row::new("font", t!("settings-font"), actions).build(theme)
}

/// How opaque the window is, as a slider with the percentage after it. The window turns
/// see-through as soon as the slider leaves the top.
fn opacity(ui: &Ui) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let transparency = match settings.transparent() {
        true => settings.transparency(),
        false => 0.,
    };
    let value = 1. - transparency / MAX_TRANSPARENCY;
    let percent = ((1. - transparency) * 100.).round() as i64;
    let said = t!("settings-opacity-value", percent = percent).into_owned();
    let slider = Scrubber::new(
        "settings-opacity-slider",
        &ui.screens.settings.opacity,
        value,
    )
    .name(t!("settings-opacity"))
    .value(said.clone())
    .colors(theme.progress_bar, theme.muted, theme.foreground)
    .on_move(|ui: &mut Ui, fraction: f32, _: &mut Tx| {
        let transparency = (1. - fraction.clamp(0., 1.)) * MAX_TRANSPARENCY;
        let settings = &mut ui.store.settings;
        settings.set_transparent(transparency > 0., &mut ui.store.cx);
        settings.set_transparency(transparency, &mut ui.store.cx);
        ui.settle();
    })
    .build(theme);
    let control = row()
        .key("opacity")
        .center_y()
        .gap(STEP_GAP)
        .child(
            row()
                .key("opacity-track")
                .w(theme.metrics.cover)
                .child(slider.w_full()),
        )
        .child(
            text(said.clone())
                .key("settings-opacity-value")
                .role("text")
                .label(said)
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.foreground.color())
                .w(theme.metrics.control * 1.5)
                .align(rune::Align::End),
        );
    Row::new("opacity", t!("settings-opacity"), control).build(theme)
}
