use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Pixels, Window, div, px, svg};
use i18n::t;
use state::{FullscreenControlsAutohide, SideTab};
use ui::{
    ActiveTheme as _, Levels, MenuItem, OptionCard, Picker, Switch, Text, Visualizer,
    VisualizerStyle,
};

use super::rows::{Announced as _, grid, segmented};
use super::{Group, Row, SettingsView, Slot};

/// How large the cross on the Off card draws.
const GLYPH: Pixels = px(18.);
/// How much of a visualizer card's height its spectrum takes.
const SPECTRUM: f32 = 0.8;
/// How thick a card's border is, which the spectrum sits inside.
const EDGE: Pixels = px(1.);
/// The views of the fullscreen player the visualizer can be drawn in, with the id each one's
/// menu item goes by and the key of its name. `None` is the cover alone.
const SCREENS: [(Option<SideTab>, &str, &str); 3] = [
    (None, "visualizer-screen-cover", "fullscreen-artwork"),
    (
        Some(SideTab::Lyrics),
        "visualizer-screen-lyrics",
        "lyrics-title",
    ),
    (
        Some(SideTab::Queue),
        "visualizer-screen-queue",
        "queue-title",
    ),
];
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
                        .then_some([Slot::VisualizerScreens, Slot::VisualizerAbsolute])
                        .into_iter()
                        .flatten(),
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
    /// The visualizer styles as cards. While Soloist plays Spotify a line under them says the
    /// visualizer follows local files only, since Soloist's sound never passes through Sonora.
    pub(super) fn visualizer_cards(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let note = self.local_only(cx);
        let chosen = self.settings.read(cx).visualizer_style();
        let height = theme.metrics.list_row * 1.5;
        // the card clips only to a rectangle, so the spectrum cuts itself to the inside of the
        // card's rounded border
        let corner = (theme.radius - EDGE).max(Pixels::ZERO);

        let cards = grid(VisualizerStyle::ALL.len() as u16, window, cx).children(
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
        );
        let Some(note) = note else {
            return cards.into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(cards)
            .child(
                div()
                    .id("settings-visualizer-note")
                    .role(Role::Paragraph)
                    .aria_label(note.clone())
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground)
                    .child(note),
            )
            .into_any_element()
    }

    /// Which views of the fullscreen player draw the visualizer. The menu stays open so several
    /// can be flipped in a row.
    pub(super) fn visualizer_screens_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let screens = self.settings.read(cx).visualizer_screens();
        let picker = Picker::new(
            "visualizer-screens",
            &self.popovers,
            t!("settings-visualizer-screens-pick"),
        )
        .width(Picker::NARROW)
        .sticky()
        .items(SCREENS.map(|(panel, id, name)| {
            let shown = screens.contains(panel);
            MenuItem::new(id, i18n::lookup(name, None))
                .selected(shown)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_visualizer_screen(panel, !shown, cx);
                    });
                    cx.notify();
                }))
        }));

        self.row(
            t!("settings-visualizer-screens"),
            picker
                .name(t!("settings-visualizer-screens"))
                .into_any_element(),
        )
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
