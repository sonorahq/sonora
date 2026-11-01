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
    ActiveTheme as _, Button, Menu, MenuItem, Modal, OptionCard, Picker, Rail, Theme, ThemeKind,
    ThemeOverrides,
};

use super::SettingsView;
use super::rows::Announced as _;
use crate::shared::theme_miniature::{CARD, THUMB, filling, sliced};

/// How many variants of a family its card shows side by side at most.
const SLICES: usize = 4;
/// The side of a variant's swatch in the menu.
const SWATCH: f32 = 14.;
/// How large the open folder glyph draws on its card.
const GLYPH: Pixels = px(18.);

/// One card of the rail: a built-in theme or a family of theme files.
enum Entry {
    Default,
    Ambient,
    Family(ThemeFamily),
}

impl SettingsView {
    /// The light/dark policy is independent of the selected colour family.
    pub(super) fn appearance_cards(
        &self,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chosen = self.settings.read(cx).appearance_mode();
        let radius = cx.theme().radius;
        super::rows::grid(ThemeKind::ALL.len() as u16, window, cx)
            .children(ThemeKind::ALL.into_iter().enumerate().map(|(place, kind)| {
                let palettes = match kind {
                    ThemeKind::System => vec![Theme::dark(), Theme::light()],
                    _ => vec![Theme::for_kind(kind)],
                };
                OptionCard::new(kind.id())
                    .choice(kind.label(), chosen == kind, place, ThemeKind::ALL.len())
                    .w_full()
                    .thumb_height(THUMB)
                    .thumb(filling(&palettes, radius))
                    .label(kind.label())
                    .selected(chosen == kind)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_appearance_mode(kind, cx));
                    }))
            }))
            .into_any_element()
    }

    pub(super) fn theme_cards(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (selected, installed, families) = {
            let settings = self.settings.read(cx);
            (
                settings.theme().to_owned(),
                settings.theme_family().is_some(),
                settings.theme_families().to_vec(),
            )
        };

        let mode = self.settings.read(cx).appearance_mode();
        let missing = selected != "default" && selected != "ambient" && !installed;
        let count = 2
            + families
                .iter()
                .filter(|family| family.variants_for(mode).next().is_some())
                .count()
            + usize::from(missing);
        let mut cards = [Entry::Default, Entry::Ambient]
            .into_iter()
            .chain(
                families
                    .into_iter()
                    .filter(|family| family.variants_for(mode).next().is_some())
                    .map(Entry::Family),
            )
            .enumerate()
            .map(|(place, entry)| {
                let chosen = match &entry {
                    Entry::Default => selected == "default",
                    Entry::Ambient => selected == "ambient",
                    Entry::Family(family) => selected == family.id,
                };
                self.theme_card(entry, chosen, (place, count), cx)
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

    /// One card: its miniature and its name. A family of several variants names the variant a
    /// click picks after its own name, and that line opens the menu of all of them.
    fn theme_card(
        &self,
        entry: Entry,
        chosen: bool,
        (place, count): (usize, usize),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let radius = cx.theme().radius;

        match entry {
            Entry::Default | Entry::Ambient => {
                let ambient = matches!(entry, Entry::Ambient);
                let settings = self.settings.read(cx);
                let kind = settings.resolved_appearance();
                let mut palette = Theme::for_kind(kind);
                let (id, label) = match ambient {
                    true => ("ambient", t!("settings-ambient")),
                    false => ("default", t!("settings-theme-default")),
                };
                if ambient {
                    let sampled = self
                        .previews
                        .playing_or_picked(cx)
                        .cover
                        .as_deref()
                        .and_then(|cover| ui::cover_palette(cover, cx))
                        .unwrap_or_default();
                    let appearance = ui::Appearance::resolve(
                        settings.look(),
                        &ThemeOverrides::default(),
                        ui::CoverPalette {
                            primary: sampled.primary.or(Some(palette.selection)),
                            ..sampled
                        },
                        1.,
                    );
                    palette = appearance.theme;
                    palette.background = appearance.ambient.palette.background;
                }
                OptionCard::new(id)
                    .choice(label.clone(), chosen, place, count)
                    .w(CARD)
                    .thumb_height(THUMB)
                    .thumb(sliced(&[palette], radius, 1.))
                    .label(label)
                    .selected(chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_theme(id, cx));
                    }))
                    .into_any_element()
            }
            Entry::Family(family) => {
                let mode = self.settings.read(cx).appearance_mode();
                let variants = family.variants_for(mode);
                let palettes = variants
                    .clone()
                    .take(SLICES)
                    .map(|variant| variant.palette())
                    .collect::<Vec<_>>();
                let several = variants.count() > 1;
                let label = self
                    .settings
                    .read(cx)
                    .variant_for(&family.id, mode)
                    .map_or_else(|| family.name.clone(), |variant| variant.title.clone());
                let id = family.id.clone();
                let doomed = family.id.clone();

                let card =
                    OptionCard::new(SharedString::from(format!("theme-family:{}", family.id)))
                        .choice(SharedString::from(label.clone()), chosen, place, count)
                        .w(CARD)
                        .thumb_height(THUMB)
                        .thumb(sliced(&palettes, radius, 1.))
                        .when(mode != ThemeKind::System, |card| card.label(label))
                        .aria_label(family.name.clone())
                        .selected(chosen)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.update(cx, |settings, cx| {
                                settings.set_theme(&id, cx);
                            });
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
                if mode == ThemeKind::System {
                    let controls = div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .children(
                            [ThemeKind::Light, ThemeKind::Dark]
                                .into_iter()
                                .filter_map(|kind| self.card_variant_picker(&family, kind, cx)),
                        );
                    return card.controls(controls).into_any_element();
                }
                card.when(several, |card| {
                    let key = SharedString::from(format!("theme-variants:{}", family.id));
                    let menu = Menu::new("theme-variants-menu")
                        .w(Picker::NARROW)
                        .when(self.popovers.shows(&key), |menu| {
                            menu.items(self.variant_items(&family, mode, cx))
                        });
                    card.variant_menu(key, &self.popovers, menu)
                })
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

    /// Deletes the family the dialog asked about and lets the theme observer apply the fallback.
    fn delete_theme(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.deleting_theme.take() else {
            return;
        };
        self.settings.update(cx, |settings, cx| {
            settings.delete_theme_family(&id, cx);
        });
        self.popovers.close();
        cx.notify();
    }

    /// Every System theme card exposes independent light and dark choices before selection.
    fn card_variant_picker(
        &self,
        family: &ThemeFamily,
        kind: ThemeKind,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let chosen = self.settings.read(cx).variant_for(&family.id, kind)?;
        let key = SharedString::from(format!("theme-variant:{}:{}", family.id, kind.id()));
        let label = chosen.title.clone();
        let open = self.popovers.shows(&key);
        Some(
            Picker::new(key, &self.popovers, label)
                .name(kind.label())
                .w_full()
                .appearance(kind)
                .when(open, |picker| {
                    picker.items(self.variant_items(family, kind, cx))
                })
                .into_any_element(),
        )
    }

    /// Shares variant selection and remembered-mode handling between both card dropdown layouts.
    fn variant_items(
        &self,
        family: &ThemeFamily,
        kind: ThemeKind,
        cx: &mut Context<Self>,
    ) -> Vec<MenuItem> {
        let chosen = self
            .settings
            .read(cx)
            .variant_for(&family.id, kind)
            .map(|variant| variant.id.clone());
        family
            .variants_for(kind)
            .map(|variant| {
                let id = variant.id.clone();
                let family_id = family.id.clone();
                MenuItem::new(
                    SharedString::from(format!("theme-variant:{}", variant.id)),
                    variant.name.clone(),
                )
                .leading(swatch(&variant.palette()))
                .checked(chosen.as_deref() == Some(variant.id.as_str()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_theme_variant(&family_id, &id, cx)
                    });
                    this.popovers.close();
                }))
            })
            .collect()
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
