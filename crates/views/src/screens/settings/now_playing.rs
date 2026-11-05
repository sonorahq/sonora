use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Pixels, Window, div, px, svg};
use i18n::t;
use state::FullscreenControlsAutohide;
use ui::{ActiveTheme as _, Levels, OptionCard, Switch, Visualizer, VisualizerStyle};

use super::rows::{Announced as _, grid, segmented};
use super::{Group, Row, SettingsView, Slot};

/// How large the cross on the Off card draws.
const GLYPH: Pixels = px(18.);
/// How much of a visualizer card's height its spectrum takes.
const SPECTRUM: f32 = 0.8;
/// How thick a card's border is, which the spectrum sits inside.
const EDGE: Pixels = px(1.);
/// The band levels a visualizer card draws, a spectrum caught at one moment.
const STILL: [f32; 16] = [
    0.42, 0.66, 0.84, 0.7, 0.5, 0.58, 0.76, 0.6, 0.44, 0.52, 0.64, 0.46, 0.34, 0.4, 0.28, 0.22,
];

impl SettingsView {
    pub(super) fn now_playing_groups(&self, cx: &App) -> Vec<Group> {
        let settings = self.settings.read(cx);
        vec![
            Group::new("settings-group-background", "icons/blend.svg")
                .rows([Slot::Ambient, Slot::AmbientMotion]),
            Group::new("settings-visualizer", "icons/music-2.svg")
                .cards(Slot::Visualizer)
                .rows(
                    settings
                        .visualizer_style()
                        .shown()
                        .then_some(Slot::VisualizerAbsolute),
                ),
            Group::new("settings-group-controls", "icons/sliders-horizontal.svg")
                .rows([Slot::FullscreenControlsAutohide, Slot::OsFullscreen]),
        ]
    }

    /// The fullscreen background sampled from the cover. It carries the cover's hues itself,
    /// so fullscreen tints from the artwork with the adaptive theme off as well.
    pub(super) fn ambient_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).ambient();

        self.row(
            t!("settings-ambient"),
            Switch::new("ambient", on)
                .name(t!("settings-ambient"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_ambient(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn ambient_motion_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).ambient_motion();
        let disabled = !self.settings.read(cx).ambient();

        Row::new(
            t!("settings-ambient-motion"),
            Switch::new("ambient-motion", on)
                .name(t!("settings-ambient-motion"))
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_ambient_motion(!on, cx));
                }))
                .into_any_element(),
        )
        .disabled(disabled)
        .into_any_element()
    }

    /// How the spectrum is drawn behind the fullscreen artwork, off included.
    /// One card per visualizer style, each drawing a still spectrum the way that style does.
    pub(super) fn visualizer_cards(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let chosen = self.settings.read(cx).visualizer_style();
        let height = theme.metrics.list_row * 1.5;
        // the card clips only to a rectangle, so the spectrum cuts itself to the inside of the
        // card's rounded border
        let corner = (theme.radius - EDGE).max(Pixels::ZERO);

        grid(VisualizerStyle::ALL.len() as u16, window, cx)
            .children(
                VisualizerStyle::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(place, style)| {
                        let thumb = match style.shown() {
                            false => div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    svg()
                                        .path(icons::path("icons/x.svg"))
                                        .size(GLYPH)
                                        .text_color(theme.muted_foreground),
                                ),
                            true => div().size_full().flex().flex_col().justify_end().child(
                                Visualizer::new(still(), height * SPECTRUM)
                                    .style_kind(style)
                                    .tint(theme.foreground)
                                    .behind(theme.secondary)
                                    .corner_radius(corner)
                                    .w_full(),
                            ),
                        };
                        OptionCard::new(style.id())
                            .choice(
                                style.label(),
                                style == chosen,
                                place,
                                VisualizerStyle::ALL.len(),
                            )
                            .thumb(thumb)
                            .label(style.label())
                            .selected(style == chosen)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.update(cx, |settings, cx| {
                                    settings.set_visualizer_style(style, cx)
                                });
                                cx.notify();
                            }))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn visualizer_absolute_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).visualizer_absolute();

        self.row(
            t!("settings-visualizer-absolute"),
            Switch::new("visualizer-absolute", on)
                .name(t!("settings-visualizer-absolute"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_visualizer_absolute(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn fullscreen_controls_autohide_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).fullscreen_controls_autohide();

        let picker = segmented(
            "fullscreen-controls-autohide",
            FullscreenControlsAutohide::ALL.map(|fca| (fca, i18n::lookup(fca.key(), None))),
            chosen,
            cx.listener(|this, fca: &FullscreenControlsAutohide, _, cx| {
                let fca = *fca;
                this.settings.update(cx, |settings, cx| {
                    settings.set_fullscreen_controls_autohide(fca, cx)
                });
                cx.notify();
            }),
        );

        self.row(
            t!("settings-fullscreen-controls-autohide"),
            picker
                .name(t!("settings-fullscreen-controls-autohide"))
                .into_any_element(),
        )
    }

    pub(super) fn os_fullscreen_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).os_fullscreen();

        self.row(
            t!("settings-os-fullscreen"),
            Switch::new("os-fullscreen", on)
                .name(t!("settings-os-fullscreen"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_os_fullscreen(!on, cx));
                }))
                .into_any_element(),
        )
    }
}

/// A still frame of a spectrum, the shape the visualizer cards draw.
fn still() -> Levels {
    Levels {
        left: STILL.to_vec(),
        right: STILL.iter().rev().copied().collect(),
    }
}
