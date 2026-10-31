use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, Hsla, Pixels, SharedString, Window, div, px, relative, svg,
};
use i18n::t;
use state::SYSTEM_FONT;
use ui::{
    ActiveTheme as _, Button, Look, MAX_FONT, MAX_TRANSPARENCY, MIN_FONT, MenuItem, OptionCard,
    Picker, Rounding, Scrubber, Switch, Theme, ThemeKind,
};

use super::rows::{Announced as _, grid, outward};
use super::{Group, SettingsView, Slot, TYPEFACES};
use crate::shared::popups::matches_query;

/// The website's gallery of themes people made, whose install buttons hand a theme to Sonora.
const COMMUNITY_THEMES: &str = "https://sonorahq.org/themes";
/// How large a pack's sample glyphs draw on its card.
const SAMPLE: Pixels = px(14.);
const TYPEFACE_LIMIT: usize = 200;
const TYPEFACE_LEAD: usize = 2;
/// How many faces are previewed before a row has been measured.
const TYPEFACE_GUESS: usize = 24;
/// How many faces are loaded a frame.
const TYPEFACE_BATCH: usize = 3;

impl SettingsView {
    pub(super) fn appearance_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-theme", "icons/palette.svg")
                .cards(Slot::Theme)
                .rows([Slot::Adaptive, Slot::CommunityThemes]),
            Group::new("settings-icons", "icons/layout-grid.svg").cards(Slot::Icons),
            Group::new("settings-corners", "icons/maximize.svg").cards(Slot::Corners),
            Group::new("settings-group-text", "icons/text-select.svg")
                .rows([Slot::Typeface, Slot::Font]),
            Group::new("settings-group-window", "icons/panel-left-open.svg").rows(
                [Slot::Opacity]
                    .into_iter()
                    .chain(ui::WINDOW_BLUR.then_some(Slot::WindowBlur))
                    .chain([Slot::Blur]),
            ),
        ]
    }

    /// The look the theme is drawn from now, with the tint the adaptive theme put on it.
    pub(super) fn look(&self, cx: &Context<Self>) -> Look {
        Look {
            tint: cx.theme().tint,
            tint_secondary: cx.theme().tint_secondary,
            ..self.settings.read(cx).look()
        }
    }

    pub(super) fn typeface_entries(&self) -> Vec<SharedString> {
        let asked = self.typefaces.query();
        let installed = self.installed.as_deref().unwrap_or_default();

        std::iter::once(SharedString::from(SYSTEM_FONT))
            .chain(installed.iter().cloned())
            .filter(|name| {
                let label = match name.as_ref() == SYSTEM_FONT {
                    true => t!("settings-typeface-system"),
                    false => name.clone(),
                };
                matches_query(name, &label, &asked)
            })
            .take(TYPEFACE_LIMIT)
            .collect()
    }

    pub(super) fn typeface_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).font().to_owned();
        let bundled = chosen == SYSTEM_FONT;
        let current = match bundled {
            true => t!("settings-typeface-system"),
            false => SharedString::from(chosen.clone()),
        };

        let picking = self.popovers.shows(TYPEFACES);
        let asked = self.typefaces.query();
        let installed = self.installed.as_deref().unwrap_or_default();

        let mut entries = Vec::new();
        if picking {
            entries.push((
                SharedString::from(SYSTEM_FONT),
                t!("settings-typeface-system"),
            ));
            entries.extend(installed.iter().map(|name| (name.clone(), name.clone())));
            entries = entries
                .into_iter()
                .filter(|(id, label)| matches_query(id, label, &asked))
                .take(TYPEFACE_LIMIT)
                .collect();
        }

        let barren = picking && entries.is_empty();
        let count = entries.len();
        let cursor = self.typefaces.cursor(count);
        let submitted = entries
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();

        let mut items = Vec::new();
        let mut waiting = false;

        if picking {
            let scroll = self.typefaces.scroll(cx);
            let row = scroll
                .bounds_for_item(0)
                .map(|item| item.size.height)
                .filter(|height| *height > px(0.));
            let first = row.map_or(cursor, |row| {
                ((-scroll.offset().y) / row).floor().max(0.) as usize
            });
            let shown = row.map_or(TYPEFACE_GUESS, |row| {
                (self.typefaces.height() / row).ceil() as usize
            });
            let previewed = first.saturating_sub(TYPEFACE_LEAD)..first + shown + TYPEFACE_LEAD;

            let mut budget = TYPEFACE_BATCH;
            let mut faced = self.typeface_faced.borrow_mut();

            // forget the faces that scrolled out of view, so scrolling back spends
            // the per-frame budget on them again rather than facing them all at once
            faced.retain(|name| {
                entries
                    .iter()
                    .enumerate()
                    .any(|(place, (id, _))| id == name && previewed.contains(&place))
            });

            items = entries
                .into_iter()
                .enumerate()
                .map(|(place, (id, label))| {
                    let name = id.clone();
                    let preview = name.clone();
                    let wanted = name.as_ref() != SYSTEM_FONT && previewed.contains(&place);
                    let shows = match (wanted, faced.contains(&name)) {
                        (false, _) => false,
                        (true, true) => true,
                        (true, false) => match budget {
                            0 => {
                                waiting = true;
                                false
                            }
                            _ => {
                                budget -= 1;
                                faced.insert(name.clone());
                                true
                            }
                        },
                    };
                    MenuItem::new(id, label)
                        .selected(place == cursor)
                        .checked(chosen == name.as_ref())
                        .when(shows, |item| item.face(preview))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let name = name.to_string();
                            this.settings
                                .update(cx, |settings, cx| settings.set_font(name, cx));
                            this.popovers.close();
                            cx.notify();
                        }))
                })
                .collect::<Vec<_>>();
            drop(faced);
        }

        if waiting {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }

        let picker = Picker::new(TYPEFACES, &self.popovers, current)
            .name(t!("settings-typeface"))
            .width(Picker::WIDE)
            .menu(self.typefaces.menu("typefaces-menu", Picker::WIDE))
            .items(items)
            .when(self.loading_fonts, |picker| {
                picker.item(
                    MenuItem::new("typeface-loading", t!("settings-typeface-loading")).disabled(),
                )
            })
            .when(barren && !self.loading_fonts, |picker| {
                picker
                    .item(MenuItem::new("typeface-empty", t!("settings-typeface-none")).disabled())
            });

        let keys = self.typefaces.controls(
            picker,
            count,
            move |this, place, _, cx| {
                let Some(name) = submitted.get(place) else {
                    return;
                };
                this.settings
                    .update(cx, |settings, cx| settings.set_font(name.to_string(), cx));
                this.popovers.close();
                cx.notify();
            },
            cx,
        );

        self.row(t!("settings-typeface"), keys.into_any_element())
    }

    pub(super) fn blur_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides();

        self.row(
            t!("settings-blur"),
            Switch::new("blur", look.blur)
                .name(t!("settings-blur"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let blur = !look.blur;
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur(blur, cx));
                    Theme::set(Look { blur, ..look }, &overrides, cx);
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// Asks the platform to blur the desktop behind a see-through window. It needs something to
    /// show through, so the switch is off and disabled while the window is opaque.
    pub(super) fn blur_window_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides();
        let opaque = !look.transparent;

        self.row(
            t!("settings-blur-window"),
            Switch::new("blur-window", look.blur_window && !opaque)
                .name(t!("settings-blur-window"))
                .disabled(opaque)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let blur_window = !look.blur_window;
                    this.settings
                        .update(cx, |settings, cx| settings.set_blur_window(blur_window, cx));
                    Theme::set(
                        Look {
                            blur_window,
                            ..look
                        },
                        &overrides,
                        cx,
                    );
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    pub(super) fn font_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides();

        let step = move |id: &'static str, label: &'static str, name: &'static str, delta: f32| {
            let overrides = overrides.clone();
            let wanted = (look.font + delta).clamp(MIN_FONT, MAX_FONT);

            Button::new(id)
                .label(label)
                .name(i18n::lookup(name, None))
                .small()
                .outline()
                .disabled(wanted == look.font)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_font_size(wanted, cx));
                    Theme::set(
                        Look {
                            font: wanted,
                            ..look
                        },
                        &overrides,
                        cx,
                    );
                    cx.notify();
                }))
        };

        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(step("font-smaller", "−", "settings-font-smaller", -1.))
            .child(ui::spoken(
                "settings-font-value",
                t!("settings-font-value", size = look.font.round() as i64),
            ))
            .child(step("font-larger", "+", "settings-font-larger", 1.));

        self.row(t!("settings-font"), actions.into_any_element())
    }

    /// One card per icon pack, its thumbnail drawn with five of the pack's own glyphs.
    pub(super) fn icon_cards(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let chosen = self.settings.read(cx).icons().to_owned();
        let current = icons::pack(&chosen).unwrap_or_else(icons::active);
        let packs = icons::packs().collect::<Vec<_>>();
        let count = packs.len();

        grid(count as u16, window, cx)
            .children(packs.into_iter().enumerate().map(|(place, pack)| {
                OptionCard::new(pack.id)
                    .choice(pack.title(), pack.id == current.id, place, count)
                    .thumb_height(theme.metrics.list_row * 1.1)
                    .thumb(centred(samples(pack, theme.foreground)))
                    .label(pack.title())
                    .selected(pack.id == current.id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_icons(pack.id, cx));
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    /// One card per corner rounding, its thumbnail the corner of a surface drawn at that radius.
    pub(super) fn corner_cards(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides();

        grid(Rounding::ALL.len() as u16, window, cx)
            .children(
                Rounding::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(place, rounding)| {
                        let overrides = overrides.clone();
                        OptionCard::new(rounding.id())
                            .choice(
                                rounding.label(),
                                look.rounding == rounding,
                                place,
                                Rounding::ALL.len(),
                            )
                            .thumb(corner(rounding, cx))
                            .label(rounding.label())
                            .selected(look.rounding == rounding)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.update(cx, |settings, cx| {
                                    settings.set_rounding(rounding.id(), cx);
                                });
                                Theme::set(Look { rounding, ..look }, &overrides, cx);
                                cx.notify();
                            }))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn opacity_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let look = self.look(cx);
        let overrides = self.settings.read(cx).theme_overrides();
        let transparency = match look.transparent {
            true => look.transparency,
            false => 0.,
        };
        let value = 1. - transparency / MAX_TRANSPARENCY;
        let percent = ((1. - transparency) * 100.).round() as i64;

        let control = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("settings-opacity-slider")
                    .role(Role::Group)
                    .aria_label(t!("settings-opacity"))
                    .w(theme.metrics.cover)
                    .child(
                        Scrubber::new(&self.opacity, value)
                            .name(t!("settings-opacity"))
                            .value(t!("settings-opacity-value", percent = percent))
                            .colors(theme.progress_bar, theme.muted, theme.foreground)
                            .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                                let transparency = (1. - *fraction) * MAX_TRANSPARENCY;
                                let transparent = transparency > 0.;
                                this.settings.update(cx, |settings, cx| {
                                    settings.set_transparent(transparent, cx);
                                    settings.set_transparency(transparency, cx);
                                });
                                Theme::set(
                                    Look {
                                        transparent,
                                        transparency,
                                        ..look
                                    },
                                    &overrides,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                ui::spoken(
                    "settings-opacity-value",
                    t!("settings-opacity-value", percent = percent),
                )
                .flex_none()
                .w(theme.metrics.control * 1.5)
                .whitespace_nowrap()
                .text_right(),
            );

        self.row(t!("settings-opacity"), control.into_any_element())
    }

    pub(super) fn community_themes_row(&self, cx: &mut Context<Self>) -> AnyElement {
        outward("community-themes", t!("settings-theme-explore"), None, cx)
            .on_click(|_, _, cx| cx.open_url(COMMUNITY_THEMES))
            .into_any_element()
    }

    pub(super) fn adaptive_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let (on, neutral) = {
            let settings = self.settings.read(cx);
            let theme = settings.theme();
            (
                settings.adaptive_theme(),
                [ThemeKind::System, ThemeKind::Dark, ThemeKind::Light]
                    .into_iter()
                    .any(|kind| kind.id() == theme),
            )
        };
        let look = self.look(cx);

        self.row(
            t!("settings-adaptive"),
            Switch::new("adaptive-theme", on)
                .name(t!("settings-adaptive"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let adaptive = !on;
                    let reset = adaptive && !neutral;
                    let kind = match reset {
                        true => ThemeKind::Dark,
                        false => look.kind,
                    };
                    let overrides = this.settings.update(cx, |settings, cx| {
                        settings.set_adaptive_theme(adaptive, cx);
                        if reset {
                            settings.set_theme(kind.id(), cx);
                        }
                        settings.theme_overrides()
                    });
                    if reset {
                        Theme::fade(Look { kind, ..look }, &overrides, cx);
                    }
                }))
                .into_any_element(),
        )
    }
}

/// Five glyphs drawn from `pack`, the way its card shows it.
fn samples(pack: &'static icons::Pack, tint: Hsla) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .children(icons::SAMPLES.iter().map(|name| {
            svg()
                .path(icons::shown(pack, name))
                .size(SAMPLE)
                .flex_none()
                .text_color(tint)
        }))
}

pub(super) fn usable_fonts(text_system: std::sync::Arc<gpui::TextSystem>) -> Vec<SharedString> {
    let mut names = text_system.all_font_names();
    names.sort_unstable();
    names.dedup();

    names
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(SharedString::from)
        .collect()
}

/// The top left corner of a surface drawn at `rounding`'s radius, the rest of it cut off by
/// the thumbnail.
pub(super) fn corner(rounding: Rounding, cx: &App) -> Div {
    let theme = cx.theme();

    div().size_full().relative().child(
        div()
            .absolute()
            .top(relative(0.3))
            .left(relative(0.25))
            .w_full()
            .h_full()
            .rounded(rounding.radius())
            .border_2()
            .border_color(theme.muted_foreground)
            .bg(theme.secondary),
    )
}

/// `content` in the middle of a thumbnail.
fn centred(content: impl IntoElement) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(content)
}
