//! The Theme section of Appearance: one card per theme on a rail that scrolls sideways, each
//! card a soft, flat miniature of the Sonora window painted in that theme.

use std::f32::consts::FRAC_PI_4;

use i18n::t;
use rune::{Cx, El, Tx, col, el};
use state::ThemeFamily;
use ui::{Button, Menu, MenuItem, Modal, OptionCard, Picker, Rail, Theme, ThemeKind};

use super::glyph_thumb;
use crate::shared::theme_miniature::{CARD, SLICES, THUMB, sliced};
use crate::{Ui, act};

/// The popover key of the variant menu, which one card at a time owns.
const VARIANTS: &str = "theme-variants";
/// The side of a variant's swatch in the menu.
const SWATCH: f32 = 14.;
/// How large the open folder glyph draws on its card.
const GLYPH: f32 = 18.;

/// One card of the rail: a built-in theme or a family of theme files.
enum Entry<'a> {
    Kind(ThemeKind),
    Family(&'a ThemeFamily),
}

/// The theme cards on one rail: System, Dark and Light, then every family of theme files, then
/// the card that opens the themes folder. While the adaptive theme is on only the neutral
/// themes can be chosen, since the tint is laid over those. A saved theme whose file is gone
/// keeps a card of its own, chosen and disabled, so the page says what became of it.
pub(super) fn cards(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let selected = settings.theme().to_owned();
    let chosen_family = settings
        .theme_variant()
        .map(|(family, _)| family.id.clone());
    let adaptive = settings.adaptive_theme();
    let families = settings.theme_families();
    let built_in = ThemeKind::ALL.iter().any(|kind| kind.id() == selected);
    let missing = !built_in && chosen_family.is_none();
    let count = ThemeKind::ALL.len() + families.len() + usize::from(missing);

    let mut cards: Vec<El> = ThemeKind::ALL
        .into_iter()
        .map(Entry::Kind)
        .chain(families.iter().map(Entry::Family))
        .enumerate()
        .map(|(place, entry)| {
            let chosen = match &entry {
                Entry::Kind(kind) => kind.id() == selected,
                Entry::Family(family) => chosen_family.as_ref() == Some(&family.id),
            };
            card(ui, layer, entry, chosen, adaptive).position_in_set(place + 1, count)
        })
        .collect();

    if missing {
        let name = t!("theme-unavailable", name = selected.clone()).into_owned();
        cards.push(
            OptionCard::<Ui>::new("unavailable-theme")
                .thumb_height(THUMB)
                .label(name)
                .selected(true)
                .disabled(true)
                .build(theme, layer)
                .w(CARD)
                .position_in_set(count, count),
        );
    }

    cards.push(
        OptionCard::new("open-theme-folder")
            .thumb_height(THUMB)
            .dashed()
            .thumb(glyph_thumb(theme, "icons/folder-plus.svg", GLYPH))
            .label(t!("settings-theme-folder"))
            .on_press(|ui: &mut Ui| {
                let path = ui.store.settings.ensure_themes_directory();
                if let Err(error) = open::that_detached(&path) {
                    log::warn!("settings: cannot open {}: {error}", path.display());
                }
            })
            .build(theme, layer)
            .w(CARD),
    );

    // the ring around the chosen card stands outside it, so the rail leaves it room
    let room = theme.metrics.pad;
    col().key("theme-rail").role("radiogroup").w_full().child(
        Rail::new("theme-cards")
            .arrows_at(room + THUMB / 2.)
            .children(cards.into_iter().map(|card| col().py(room).child(card)))
            .build::<Ui>(theme)
            .w_full()
            .min_w(0.),
    )
}

/// One card: its miniature and its name. A family of several variants names the variant a
/// press picks after its own name, and that line opens the menu of all of them.
fn card(ui: &Ui, layer: &mut Vec<El>, entry: Entry, chosen: bool, adaptive: bool) -> El {
    let theme = &ui.theme;
    let radius = theme.radius;
    match entry {
        Entry::Kind(kind) => {
            let palettes = match kind {
                ThemeKind::System => vec![Theme::dark(), Theme::light()],
                kind => vec![Theme::for_kind(kind)],
            };
            OptionCard::new(format!("theme-{}", kind.id()))
                .thumb_height(THUMB)
                .thumb(sliced(&palettes, radius, 1.))
                .label(kind.label())
                .selected(chosen)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.settings.set_theme(kind.id(), &mut ui.store.cx)
                }))
                .build(theme, layer)
                .w(CARD)
        }
        Entry::Family(family) => {
            let settings = &ui.store.settings;
            let palettes: Vec<Theme> = family
                .variants
                .iter()
                .take(SLICES)
                .map(|variant| variant.palette())
                .collect();
            let several = family.variants.len() > 1;
            // the variant on show while the family is chosen, else the one a press would pick
            let picked = match chosen {
                true => settings.theme_variant().map(|(_, variant)| variant),
                false => settings.family_variant(&family.id),
            }
            .filter(|_| several)
            .map(|variant| (variant.id.clone(), variant.name.clone()));
            let label = match &picked {
                Some((_, name)) => format!("{} {name}", family.name),
                None => family.name.clone(),
            };
            let id = family.id.clone();
            let mut card = OptionCard::new(format!("theme-family-{}", family.id))
                .thumb_height(THUMB)
                .thumb(sliced(&palettes, radius, 1.))
                .label(label)
                .selected(chosen)
                .disabled(adaptive)
                .on_press(act(move |ui: &mut Ui| {
                    ui.store.settings.set_theme_family(&id, &mut ui.store.cx);
                }));
            if adaptive {
                card = card.tooltip(t!("settings-theme-unavailable"));
            }
            if !family.bundled {
                let doomed = family.id.clone();
                card = card.on_delete(t!("settings-theme-delete"), move |ui: &mut Ui| {
                    ui.screens.settings.deleting_theme = Some(doomed.clone());
                });
            }
            if let Some((picked, _)) = picked {
                card = variants(ui, card, family, &picked);
            }
            card.build(theme, layer).w(CARD)
        }
    }
}

/// Gives a family's card the menu of its variants, `picked` checked, opened from its name.
/// Only the family whose menu opened last owns the popover, so the name on any other card hands
/// it to its own.
fn variants(ui: &Ui, card: OptionCard<Ui>, family: &ThemeFamily, picked: &str) -> OptionCard<Ui> {
    let state = &ui.screens.settings;
    if state.theme_variants.as_deref() != Some(family.id.as_str()) {
        let id = family.id.clone();
        return card.on_variants(move |ui: &mut Ui, _: &mut Tx| {
            let settings = &mut ui.screens.settings;
            settings.theme_variants = Some(id.clone());
            settings.popovers.open(VARIANTS);
        });
    }
    let items = family.variants.iter().map(|variant| {
        let id = variant.id.clone();
        MenuItem::new(
            format!("theme-variant-{}", variant.id),
            variant.name.clone(),
        )
        .leading(swatch(&variant.palette()))
        .checked(picked == variant.id)
        .on_press(act(move |ui: &mut Ui| {
            ui.store.settings.set_theme(id.clone(), &mut ui.store.cx)
        }))
    });
    let menu = Menu::new("theme-variants-menu")
        .width(Picker::<Ui>::NARROW)
        .items(items);
    card.variant_menu(
        VARIANTS,
        &state.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        menu,
    )
}

/// The question before the user's theme family is deleted, pushed into `layer` while one waits
/// and the family is still there.
pub(super) fn delete_modal(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let Some(id) = ui.screens.settings.deleting_theme.as_deref() else {
        return;
    };
    let Some(family) = ui
        .store
        .settings
        .theme_families()
        .iter()
        .find(|family| family.id == id)
    else {
        return;
    };
    Modal::new(
        "theme-delete",
        t!("settings-theme-delete-title", name = family.name.clone()),
    )
    .detail(t!("settings-theme-delete-detail"))
    .action(
        Button::new("cancel-theme-delete")
            .ghost()
            .label(t!("common-cancel"))
            .on_press(|ui: &mut Ui| ui.screens.settings.deleting_theme = None)
            .build(theme),
    )
    .action(
        Button::new("apply-theme-delete")
            .destructive()
            .label(t!("common-delete"))
            .on_press(act(delete))
            .build(theme),
    )
    .on_dismiss(|ui: &mut Ui| ui.screens.settings.deleting_theme = None)
    .build(theme, layer);
}

/// Deletes the family the dialog asked about. When it was the theme in use the window goes
/// back to Dark, the theme settings fall back to.
fn delete(ui: &mut Ui) {
    let Some(id) = ui.screens.settings.deleting_theme.take() else {
        return;
    };
    ui.store.settings.delete_theme_family(&id, &mut ui.store.cx);
    if ui.screens.settings.theme_variants.as_deref() == Some(id.as_str()) {
        ui.screens.settings.theme_variants = None;
    }
}

/// A variant's swatch in the menu: a circle cut on the diagonal into its page colour at the top
/// left and its accent at the bottom right. The cut blends over one point so its edge stays
/// smooth.
fn swatch(palette: &Theme) -> El {
    let blend = 0.5 / SWATCH;
    el().size(SWATCH, SWATCH)
        .radius(SWATCH / 2.)
        .linear_gradient(
            FRAC_PI_4 * 3.,
            [
                (0.5 - blend, palette.background.color()),
                (0.5 + blend, palette.primary.color()),
            ],
        )
        .a11y_hidden()
}
