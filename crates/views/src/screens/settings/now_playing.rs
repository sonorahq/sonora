use std::time::Duration;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, div, svg};
use i18n::t;
use state::{FullscreenControlsAutohide, SideTab};
use ui::{ActiveTheme as _, MenuItem, Perch, Picker, Switch, Text, Tipped as _, VisualizerStyle};

use super::rows::{chip, chips};
use super::{Group, SettingsView, Slot};

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
const AUTOHIDE: &str = "fullscreen-controls-autohide";
const VISUALIZER: &str = "visualizer";

impl SettingsView {
    /// The Now playing page, which stands beside its preview where the column has room: what
    /// fills the view and how the spectrum draws, then how the controls and the window behave.
    /// The preview shows every choice, so none needs a picture of its own.
    pub(super) fn now_playing_groups(&self, cx: &App) -> Vec<Group> {
        let shown = self.settings.read(cx).visualizer_style().shown();
        vec![
            Group::untitled()
                .rows([Slot::Ambient, Slot::Visualizer])
                .rows(
                    shown
                        .then_some([Slot::VisualizerScreens, Slot::VisualizerAbsolute])
                        .into_iter()
                        .flatten(),
                ),
            Group::untitled().rows([Slot::FullscreenControlsAutohide, Slot::OsFullscreen]),
        ]
    }

    /// The fullscreen background sampled from the cover and its drift, as two chips. It carries
    /// the cover's hues itself, so fullscreen tints from the artwork with the adaptive theme off
    /// as well. The drift needs the field, so its chip is held while the field is off.
    pub(super) fn ambient_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let ambient = settings.ambient();
        let motion = settings.ambient_motion();

        let field = chip(
            "ambient",
            "icons/palette.svg",
            t!("settings-ambient-short"),
            t!("settings-ambient"),
            ambient,
            cx,
        )
        .tooltip("settings-ambient-detail")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.settings
                .update(cx, |settings, cx| settings.set_ambient(!ambient, cx));
        }));
        let drift = chip(
            "ambient-motion",
            "icons/loader-circle.svg",
            t!("settings-ambient-motion-short"),
            t!("settings-ambient-motion"),
            motion && ambient,
            cx,
        )
        .disabled(!ambient)
        .tooltip("settings-ambient-motion-detail")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.settings
                .update(cx, |settings, cx| settings.set_ambient_motion(!motion, cx));
        }));

        self.row(
            t!("settings-group-background"),
            chips([field, drift]).into_any_element(),
        )
    }

    /// How the spectrum is drawn behind the fullscreen artwork, off included, as a picker,
    /// which keeps the row narrow enough to stand beside the preview in every language. The
    /// preview beside it shows the style chosen. While Soloist plays Spotify a red warning
    /// stands before the picker, saying the moment the pointer reaches it that the visualizer
    /// follows local files only, since Soloist's sound never passes through Sonora.
    pub(super) fn visualizer_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let chosen = self.settings.read(cx).visualizer_style();
        let warning = self.local_only(cx).map(|note| {
            div()
                .id("settings-visualizer-soloist")
                .role(Role::Image)
                .aria_label(note.clone())
                .flex_none()
                .flex()
                .items_center()
                .child(
                    svg()
                        .path(icons::path("icons/triangle-alert.svg"))
                        .size(theme.text(Text::Body))
                        .text_color(theme.status_error),
                )
                .tip_label(note, Perch::Below)
                // the mark is small and says nothing by itself, so its words come at once
                .tooltip_show_delay(Duration::ZERO)
        });

        let picker = Picker::new(VISUALIZER, &self.popovers, chosen.label())
            .width(Picker::NARROW)
            .items(VisualizerStyle::ALL.map(|style| {
                MenuItem::new(style.id(), style.label())
                    .selected(style == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_visualizer_style(style, cx));
                        this.popovers.close();
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-visualizer"),
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(warning)
                .child(picker.name(t!("settings-visualizer")))
                .into_any_element(),
        )
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

    /// When the fullscreen controls step aside, as a picker, which keeps the row narrow enough to
    /// stand beside the preview.
    pub(super) fn fullscreen_controls_autohide_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).fullscreen_controls_autohide();

        let picker = Picker::new(AUTOHIDE, &self.popovers, i18n::lookup(chosen.key(), None))
            .width(Picker::NARROW)
            .items(FullscreenControlsAutohide::ALL.map(|choice| {
                MenuItem::new(choice.id(), i18n::lookup(choice.key(), None))
                    .selected(choice == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_fullscreen_controls_autohide(choice, cx)
                        });
                        this.popovers.close();
                        cx.notify();
                    }))
            }));

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
