use gpui::prelude::*;
use gpui::{AnyElement, App, Context, MouseButton, Pixels, SharedString, div, px, svg};
use i18n::t;
use router::{Destination, navigate};
use state::Outputs;
use ui::{ActiveTheme as _, Button, Picker, Switch, Text};

use super::{Group, SettingsView, Slot};
use crate::shared::equalizer::sound_picker;

/// The chevron at the end of the equalizer row, as large as a button's icon.
const CHEVRON: Pixels = px(16.);
/// The key of the equalizer row's sound picker.
const SOUND: &str = "settings-equalizer-sound";

impl SettingsView {
    pub(super) fn sound_groups(&self, cx: &App) -> Vec<Group> {
        let failed = self.outputs.read(cx).failure().is_some();
        // with nothing open there is no rate or buffer to show
        let output = match failed {
            true => vec![Slot::OutputDevice, Slot::OutputFailure],
            false => vec![Slot::OutputDevice, Slot::SampleRate, Slot::Buffer],
        };
        vec![
            Group::new("settings-group-output", "icons/volume-2.svg").rows(output),
            Group::new("settings-equalizer", "icons/sliders-horizontal.svg")
                .rows([Slot::Equalizer]),
        ]
    }

    /// The device picker, whose "System default" follows whatever the system plays on.
    pub(super) fn output_device_row(&self) -> AnyElement {
        self.row(
            t!("settings-output-device"),
            self.output_picker.clone().into_any_element(),
        )
    }

    /// Why nothing plays: the device to play on would not open. It stands in for the rate and
    /// buffer rows.
    pub(super) fn output_failure_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (title, detail) = self.output_failure(cx);

        self.row(
            title,
            ui::spoken("settings-output-failure", detail)
                .max_w(theme.metrics.cover * 2.)
                .truncate()
                .text_size(theme.text(Text::Label))
                .text_color(theme.muted_foreground)
                .into_any_element(),
        )
    }

    /// The failure row's title and detail.
    pub(super) fn output_failure(&self, cx: &App) -> (SharedString, SharedString) {
        let outputs = self.outputs.read(cx);
        let error = outputs
            .failure()
            .map(|failure| failure.error.clone())
            .unwrap_or_default();
        (
            t!("settings-output-failed"),
            t!("settings-output-failed-detail", error = error),
        )
    }

    /// The rate the device runs at. Where Sonora can open the device at other rates it steps
    /// through them, otherwise it offers a way into the system's own settings where there is one.
    /// The row keeps its shape while the stream reopens, showing the rate it reopens at.
    pub(super) fn sample_rate_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let outputs = self.outputs.read(cx);
        let device = outputs.current();
        let current = outputs.rate(cx);
        let rates = device
            .map(|device| device.rates.clone())
            .unwrap_or_default();
        // while another device is being tried, the open stream is not the one that will play
        let locked = outputs.switching();
        let settings_uri = match rates.len() > 1 {
            true => None,
            false => {
                music::output::system_audio_settings_uri(device.map(|device| device.id.as_str()))
            }
        };
        let rate = current
            .map(|rate| t!("settings-sample-rate-value", khz = f64::from(rate) / 1_000.))
            .unwrap_or_else(|| t!("settings-output-idle"));
        let rate = ui::spoken("sample-rate-value", rate);
        let stepper = current.filter(|_| rates.len() > 1).map(|current| {
            output_stepper(
                "sample-rate",
                ("settings-sample-rate-lower", "settings-sample-rate-higher"),
                current,
                &rates,
                locked,
                Outputs::set_rate,
                cx,
            )
        });

        self.row(
            t!("settings-sample-rate"),
            div()
                .flex()
                .items_center()
                .gap_2()
                .map(|row| match stepper {
                    Some((lower, higher)) => row.child(lower).child(rate).child(higher),
                    None => row.child(rate),
                })
                .when_some(settings_uri, |row, uri| {
                    row.child(
                        Button::new("sample-rate-change")
                            .label(t!("settings-sample-rate-change"))
                            .small()
                            .outline()
                            .icon("icons/link.svg")
                            .on_click(move |_, _, cx| cx.open_url(&uri)),
                    )
                })
                .into_any_element(),
        )
    }

    /// The buffer Sonora asks the device for, in frames, with the buffers the device allows to
    /// step through. The row keeps its shape while the stream reopens, showing the buffer it
    /// reopens with.
    pub(super) fn buffer_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let outputs = self.outputs.read(cx);
        let current = outputs.buffer(cx);
        let buffers = outputs
            .current()
            .map(|device| device.buffers.clone())
            .unwrap_or_default();
        let locked = outputs.switching();
        let buffer = current
            .map(|frames| SharedString::from(frames.to_string()))
            .unwrap_or_else(|| t!("settings-output-idle"));
        let buffer = ui::spoken("buffer-value", buffer);
        let stepper = current.filter(|_| buffers.len() > 1).map(|current| {
            output_stepper(
                "buffer",
                ("settings-buffer-smaller", "settings-buffer-larger"),
                current,
                &buffers,
                locked,
                Outputs::set_buffer,
                cx,
            )
        });

        self.row(
            t!("settings-buffer"),
            div()
                .flex()
                .items_center()
                .gap_2()
                .map(|row| match stepper {
                    Some((lower, higher)) => row.child(lower).child(buffer).child(higher),
                    None => row.child(buffer),
                })
                .into_any_element(),
        )
    }

    /// The way into the equalizer: its name, a picker of every sound it can play, its switch and
    /// a chevron. The row opens the equalizer page, while the picker and the switch work in place.
    pub(super) fn equalizer_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let on = self.equalizer.read(cx).enabled();
        let sound = sound_picker(SOUND, &self.popovers, &self.equalizer, &self.settings, cx)
            .width(Picker::REGULAR)
            .name(t!("equalizer-sound"));

        div()
            .id("open-equalizer")
            .role(gpui::accesskit::Role::Button)
            .aria_label(t!("settings-equalizer"))
            .flex()
            .items_center()
            .gap_3()
            .h(theme.metrics.list_row)
            .mx_neg_3()
            .px_3()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|style| style.bg(theme.secondary_hover))
            .on_click(|_, _, cx| navigate(Destination::Equalizer, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(t!("settings-equalizer")),
            )
            .child(
                div()
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(sound),
            )
            .child(
                div()
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Switch::new("equalizer", on)
                            .name(t!("settings-equalizer"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.equalizer
                                    .update(cx, |equalizer, cx| equalizer.set_enabled(!on, cx));
                            })),
                    ),
            )
            .child(
                svg()
                    .path(icons::path("icons/chevron-right.svg"))
                    .size(CHEVRON)
                    .flex_none()
                    .text_color(theme.muted_foreground),
            )
            .into_any_element()
    }
}

/// The minus and plus buttons beside an output setting's value, which `apply` the next lower or
/// higher of `options` than `current`. A button with nothing further that way is disabled, and
/// both are while `locked`. `names` holds the Fluent keys screen readers name the two buttons by.
pub(super) fn output_stepper(
    id: &'static str,
    names: (&'static str, &'static str),
    current: u32,
    options: &[u32],
    locked: bool,
    apply: fn(&mut Outputs, u32, &mut Context<Outputs>),
    cx: &mut Context<SettingsView>,
) -> (Button, Button) {
    let lower = options
        .iter()
        .rev()
        .find(|&&option| option < current)
        .copied();
    let higher = options.iter().find(|&&option| option > current).copied();
    let step =
        |suffix: &'static str, label: &'static str, name: &'static str, next: Option<u32>| {
            Button::new(SharedString::from(format!("{id}-{suffix}")))
                .label(label)
                .name(i18n::lookup(name, None))
                .small()
                .outline()
                .disabled(locked || next.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(next) = next else { return };
                    this.outputs
                        .update(cx, |outputs, cx| apply(outputs, next, cx));
                }))
        };
    (
        step("lower", "\u{2212}", names.0, lower),
        step("higher", "+", names.1, higher),
    )
}
