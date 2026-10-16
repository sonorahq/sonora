use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div};
use i18n::t;
use music::output::Device;
use state::equalizer_profile_label;
use ui::{ActiveTheme as _, MenuItem, Picker, Text};

use super::EqualizerView;

const THIS_OUTPUT: &str = "equalizer-output-this-picker";
const OTHER_OUTPUTS: &str = "equalizer-output-others-picker";

impl EqualizerView {
    /// The output played on now, the profile bound to it, and the profile every output with
    /// none of its own plays. Left out while nothing says which output plays.
    pub(super) fn outputs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let device = self.outputs.read(cx).device()?.clone();
        let theme = *cx.theme();
        let settings = self.settings.read(cx);
        let names: Vec<SharedString> = settings
            .equalizer_profiles()
            .map(equalizer_profile_label)
            .collect();
        let bound = settings.equalizer_profile_bound_to(&device.id);
        let others = settings.equalizer_other_outputs();
        let shown = |index: Option<usize>, none: SharedString| {
            index
                .and_then(|index| names.get(index).cloned())
                .unwrap_or(none)
        };

        let this = Picker::new(
            THIS_OUTPUT,
            &self.popovers,
            shown(bound, t!("equalizer-output-unbound")),
        )
        .width(Picker::NARROW)
        .items(self.choices(
            THIS_OUTPUT,
            &names,
            bound,
            t!("equalizer-output-unbound"),
            {
                let device = device.clone();
                move |this, index, cx| this.bind(&device, index, cx)
            },
            cx,
        ));
        let other = Picker::new(
            OTHER_OUTPUTS,
            &self.popovers,
            shown(others, t!("equalizer-output-keep")),
        )
        .width(Picker::NARROW)
        .items(self.choices(
            OTHER_OUTPUTS,
            &names,
            others,
            t!("equalizer-output-keep"),
            |this, index, cx| {
                this.playback.update(cx, |playback, cx| {
                    playback.set_equalizer_other_outputs(index, cx)
                })
            },
            cx,
        ));
        let labelled = |label: SharedString, picker: Picker| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(label)
                .child(picker)
        };

        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_4()
                .gap_y_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(div().flex_none().child(t!("equalizer-output")))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(device_name(&device)),
                        ),
                )
                .child(div().flex_1())
                .child(labelled(t!("equalizer-output-this"), this))
                .child(labelled(t!("equalizer-output-others"), other))
                .into_any_element(),
        )
    }

    /// No profile, then every profile, the chosen one marked.
    fn choices(
        &self,
        id: &'static str,
        names: &[SharedString],
        chosen: Option<usize>,
        none: SharedString,
        pick: impl Fn(&mut Self, Option<usize>, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> Vec<MenuItem> {
        let options = std::iter::once((None, none))
            .chain(names.iter().cloned().enumerate().map(|(i, n)| (Some(i), n)));
        options
            .map(|(index, label)| {
                let pick = pick.clone();
                MenuItem::new((id, index.map_or(0, |index| index + 1)), label)
                    .selected(index == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| pick(this, index, cx)))
            })
            .collect()
    }

    fn bind(&mut self, device: &Device, index: Option<usize>, cx: &mut Context<Self>) {
        self.playback.update(cx, |playback, cx| {
            playback.bind_equalizer_output(device, index, cx)
        });
    }
}

/// The device's name, with the port that plays when it has more than one.
pub(super) fn device_name(device: &Device) -> SharedString {
    match &device.port {
        Some(port) => t!(
            "equalizer-output-port",
            device = device.name.clone(),
            port = port.clone()
        ),
        None => device.name.clone().into(),
    }
}
