//! The Theme section of Appearance: one card per theme on a rail that scrolls sideways, each
//! card a soft, flat miniature of the Sonora window painted in that theme.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, Pixels, SharedString, div, linear_color_stop, linear_gradient, px,
    svg,
};
use i18n::t;
use state::ThemeFamily;
use ui::{
    ActiveTheme as _, Button, Look, Menu, MenuItem, Modal, OptionCard, Picker, Rail, Theme,
    ThemeKind,
};

use super::SettingsView;
use super::rows::Announced as _;
use crate::shared::theme_miniature::{CARD, SLICES, THUMB, sliced};

/// The popover key of the variant menu, which one card at a time owns.
const VARIANTS: &str = "theme-variants";
/// The side of a variant's swatch in the menu.
const SWATCH: f32 = 14.;
/// How large the open folder glyph draws on its card.
const GLYPH: Pixels = px(18.);

/// One card of the rail: a built-in theme or a family of theme files.
enum Entry {
    Kind(ThemeKind),
    Family(ThemeFamily),
}

impl SettingsView {
    /// The theme cards on one rail: System, Dark and Light, then every family of theme files,
    /// then the card that opens the themes folder. While the adaptive theme is on only the
    /// neutral themes can be chosen, since the tint is laid over those. A saved theme whose file
    /// is gone keeps a card of its own, chosen and disabled, so the page says what became of it.
    pub(super) fn theme_cards(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (selected, chosen_family, adaptive, families) = {
            let settings = self.settings.read(cx);
            (
                settings.theme().to_owned(),
                settings
                    .theme_variant()
                    .map(|(family, _)| family.id.clone()),
                settings.adaptive_theme(),
                settings.theme_families().to_vec(),
            )
        };
        let built_in = ThemeKind::ALL.iter().any(|kind| kind.id() == selected);
        let missing = !built_in && chosen_family.is_none();
        let count = ThemeKind::ALL.len() + families.len() + usize::from(missing);

        let mut cards = ThemeKind::ALL
            .into_iter()
            .map(Entry::Kind)
            .chain(families.into_iter().map(Entry::Family))
            .enumerate()
            .map(|(place, entry)| {
                let chosen = match &entry {
                    Entry::Kind(kind) => kind.id() == selected,
                    Entry::Family(family) => chosen_family.as_ref() == Some(&family.id),
                };
                self.theme_card(entry, chosen, adaptive, (place, count), cx)
            })
            .collect::<Vec<_>>();

        if missing {
            let name = t!("theme-unavailable", name = selected.clone());
            cards.push(
                OptionCard::new("unavailable-theme")
                    .choice(name.clone(), true, count - 1, count)
                    .w(CARD)
                    .thumb_height(THUMB)
                    .label(name)
                    .selected(true)
                    .disabled(true)
                    .into_any_element(),
            );
        }

        let settings = self.settings.clone();
        cards.push(
            OptionCard::new("open-theme-folder")
                .role(Role::Button)
                .aria_label(t!("settings-theme-folder"))
                .w(CARD)
                .thumb_height(THUMB)
                .dashed()
                .thumb(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path(icons::path("icons/folder-plus.svg"))
                                .size(GLYPH)
                                .text_color(theme.muted_foreground),
                        ),
                )
                .label(t!("settings-theme-folder"))
                .on_click(move |_, _, cx| {
                    let path =
                        settings.update(cx, |settings, _| settings.ensure_themes_directory());
                    if let Err(error) = open::that_detached(&path) {
                        log::warn!("settings: cannot open {}: {error}", path.display());
                    }
                })
                .into_any_element(),
        );

        // the ring around the chosen card stands outside it, so the rail leaves it room
        let room = theme.metrics.pad;
        Rail::new("theme-cards")
            .py(room)
            .arrows_at(room + THUMB / 2.)
            .children(cards)
            .into_any_element()
    }

    /// One card, placed by its index and the rail's length: its miniature and its name. A
    /// family of several variants names the variant a click picks after its own name, and that
    /// line opens the menu of all of them.
    fn theme_card(
        &self,
        entry: Entry,
        chosen: bool,
        adaptive: bool,
        (place, count): (usize, usize),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let look = self.look(cx);
        let radius = cx.theme().radius;

        match entry {
            Entry::Kind(kind) => {
                let palettes = match kind {
                    ThemeKind::System => vec![Theme::dark(), Theme::light()],
                    kind => vec![Theme::for_kind(kind)],
                };
                OptionCard::new(kind.id())
                    .choice(kind.label(), chosen, place, count)
                    .w(CARD)
                    .thumb_height(THUMB)
                    .thumb(sliced(&palettes, radius, 1.))
                    .label(kind.label())
                    .selected(chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let overrides = this.settings.update(cx, |settings, cx| {
                            settings.set_theme(kind.id(), cx);
                            settings.theme_overrides()
                        });
                        Theme::fade(Look { kind, ..look }, &overrides, cx);
                        cx.notify();
                    }))
                    .into_any_element()
            }
            Entry::Family(family) => {
                let palettes = family
                    .variants
                    .iter()
                    .take(SLICES)
                    .map(|variant| variant.palette())
                    .collect::<Vec<_>>();
                let several = family.variants.len() > 1;
                // the variant on show while the family is chosen, else the one a click would pick
                let settings = self.settings.read(cx);
                let picked = match chosen {
                    true => settings.theme_variant().map(|(_, variant)| variant),
                    false => settings.family_variant(&family.id, cx),
                }
                .filter(|_| several)
                .map(|variant| (variant.id.clone(), variant.name.clone()));
                let label = match &picked {
                    Some((_, name)) => format!("{} {name}", family.name),
                    None => family.name.clone(),
                };
                let id = family.id.clone();
                let doomed = family.id.clone();

                let card =
                    OptionCard::new(SharedString::from(format!("theme-family:{}", family.id)))
                        .choice(SharedString::from(label.clone()), chosen, place, count)
                        .w(CARD)
                        .thumb_height(THUMB)
                        .thumb(sliced(&palettes, radius, 1.))
                        .label(label)
                        .selected(chosen)
                        .disabled(adaptive)
                        .when(adaptive, |card| {
                            card.tooltip("settings-theme-unavailable")
                                .aria_description(t!("settings-theme-unavailable"))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let overrides = this.settings.update(cx, |settings, cx| {
                                settings.set_theme_family(&id, cx);
                                settings.theme_overrides()
                            });
                            Theme::fade(
                                Look {
                                    kind: ThemeKind::Dark,
                                    ..look
                                },
                                &overrides,
                                cx,
                            );
                            cx.notify();
                        }))
                        .when(!family.bundled, |card| {
                            card.on_delete(
                                "settings-theme-delete",
                                cx.listener(move |this, _, _, cx| {
                                    this.deleting_theme = Some(doomed.clone());
                                    cx.notify();
                                }),
                            )
                        });
                match picked {
                    Some((picked, _)) => self.with_variants(card, &family, &picked, cx),
                    None => card,
                }
                .into_any_element()
            }
        }
    }

    /// The question before the user's theme family is deleted, or `None` while none waits or
    /// the family is already gone.
    pub(super) fn theme_delete_modal(&self, cx: &mut Context<Self>) -> Option<Modal> {
        let id = self.deleting_theme.as_deref()?;
        let name = self
            .settings
            .read(cx)
            .theme_families()
            .iter()
            .find(|family| family.id == id)?
            .name
            .clone();
        let modal = Modal::new(
            "theme-delete",
            t!("settings-theme-delete-title", name = &name),
        )
        .detail(t!("settings-theme-delete-detail"))
        .action(
            Button::new("cancel-theme-delete")
                .ghost()
                .label(t!("common-cancel"))
                .on_click(cx.listener(|this, _, _, cx| this.close_theme_delete(cx))),
        )
        .action(
            Button::new("apply-theme-delete")
                .destructive()
                .label(t!("common-delete"))
                .on_click(cx.listener(|this, _, _, cx| this.delete_theme(cx))),
        )
        .on_dismiss(cx.listener(|this, _, _, cx| this.close_theme_delete(cx)));
        Some(modal)
    }

    pub(super) fn close_theme_delete(&mut self, cx: &mut Context<Self>) {
        self.deleting_theme = None;
        cx.notify();
    }

    /// Deletes the family the dialog asked about. When it was the theme in use the window fades
    /// to Dark, the theme settings fall back to.
    fn delete_theme(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.deleting_theme.take() else {
            return;
        };
        let look = self.look(cx);
        let fell_back = self.settings.update(cx, |settings, cx| {
            let chosen = settings
                .theme_variant()
                .is_some_and(|(family, _)| family.id == id);
            settings.delete_theme_family(&id, cx) && chosen
        });
        if fell_back {
            let overrides = self.settings.read(cx).theme_overrides();
            Theme::fade(
                Look {
                    kind: ThemeKind::Dark,
                    ..look
                },
                &overrides,
                cx,
            );
        }
        if self.theme_variants.as_deref() == Some(id.as_str()) {
            self.theme_variants = None;
        }
        cx.notify();
    }

    /// Gives a family's card the menu of its variants, `picked` checked, opened from its name.
    /// Only the family whose menu opened last owns the popover, so the name on any other card
    /// hands it to its own.
    fn with_variants(
        &self,
        card: OptionCard,
        family: &ThemeFamily,
        picked: &str,
        cx: &mut Context<Self>,
    ) -> OptionCard {
        if self.theme_variants.as_deref() != Some(family.id.as_str()) {
            let id = family.id.clone();
            return card.on_variants(cx.listener(move |this, _, _, cx| {
                this.theme_variants = Some(id.clone());
                this.popovers.open(VARIANTS);
                cx.notify();
            }));
        }

        let look = self.look(cx);
        let items = family.variants.iter().map(|variant| {
            let id = variant.id.clone();
            MenuItem::new(
                SharedString::from(format!("theme-variant:{}", variant.id)),
                variant.name.clone(),
            )
            .leading(swatch(&variant.palette()))
            .checked(picked == variant.id)
            .on_click(cx.listener(move |this, _, _, cx| {
                let overrides = this.settings.update(cx, |settings, cx| {
                    settings.set_theme(id.clone(), cx);
                    settings.theme_overrides()
                });
                Theme::fade(
                    Look {
                        kind: ThemeKind::Dark,
                        ..look
                    },
                    &overrides,
                    cx,
                );
                this.popovers.close();
                cx.notify();
            }))
        });
        let menu = Menu::new("theme-variants-menu")
            .w(Picker::NARROW)
            .items(items);

        card.variant_menu(VARIANTS, &self.popovers, menu)
    }
}

/// A variant's swatch in the menu: a circle cut on the diagonal into its page colour at the top
/// left and its accent at the bottom right. The cut blends over one pixel so its edge stays
/// smooth.
fn swatch(palette: &Theme) -> Div {
    let blend = 0.5 / SWATCH;
    div()
        .size(px(SWATCH))
        .flex_none()
        .rounded_full()
        .bg(linear_gradient(
            135.,
            linear_color_stop(palette.background, 0.5 - blend),
            linear_color_stop(palette.primary, 0.5 + blend),
        ))
}
