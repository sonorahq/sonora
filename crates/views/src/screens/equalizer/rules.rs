use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, div};
use i18n::t;
use music::output::{self, DeviceOption, OutputId, Sink};
use state::{DeviceRule, RuleDevice};
use ui::{ActiveTheme as _, Button, MenuItem, Picker, Text};

use super::{EqualizerView, heading};
use crate::shared::equalizer::{SOUND_MENU_HEIGHT, sketch, sound_curve, sound_items, sound_label};

const DEVICE: &str = "equalizer-rule-device";
const SOUND: &str = "equalizer-rule-sound";
/// The icon before the group's heading.
const ICON: &str = "icons/headphones.svg";
/// How wide a rule's sound column is, in cover sizes, so the sounds line up under each other.
const SOUND_COLUMN: f32 = 1.4;

/// A device the add row's device menu offers, with what is known about it.
struct Offered {
    device: RuleDevice,
    playing: bool,
    connected: bool,
}

impl EqualizerView {
    /// The device rules' group: a row for each rule with a remove button, then a row that adds
    /// one. A rule says which sound plays once the sound moves to its device.
    pub(super) fn rules(&self, cx: &mut Context<Self>) -> AnyElement {
        let offered = self.offered(cx);
        let settings = self.settings.read(cx);
        let governing = self
            .outputs
            .read(cx)
            .binding_key()
            .and_then(|output| settings.equalizer_rule_for_output(&output.id))
            .map(|rule| rule.device.clone());
        let rules = settings.equalizer_rules().to_vec();
        let count = rules.len();
        let rows: Vec<AnyElement> = rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                let connected = match &rule.device {
                    RuleDevice::Other => true,
                    RuleDevice::Output(output) => offered.iter().any(|offer| {
                        offer.connected && offer.device.id() == Some(output.id.as_str())
                    }),
                };
                let playing = governing
                    .as_ref()
                    .is_some_and(|device| device.id() == rule.device.id());
                self.rule_row(index, count, rule, playing, connected, cx)
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .child(heading("equalizer-rules", ICON, None, cx))
            .when(!rows.is_empty(), |group| {
                group.child(
                    div()
                        .id("equalizer-rule-list")
                        .role(Role::List)
                        .aria_label(t!("equalizer-rules"))
                        .flex()
                        .flex_col()
                        .children(rows),
                )
            })
            .child(self.add_rule(offered, cx))
            .into_any_element()
    }

    /// One rule: its device, marked when it is playing now or not connected, the sound it
    /// plays, and a button that removes it. Screen readers hear it as item `index` of `count`,
    /// named by its device with the sound and the mark after it.
    fn rule_row(
        &self,
        index: usize,
        count: usize,
        rule: &DeviceRule,
        playing: bool,
        connected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let marker = match (playing, connected) {
            (true, _) => Some(t!("equalizer-rule-playing")),
            (false, false) => Some(t!("equalizer-rule-disconnected")),
            (false, true) => None,
        };
        let shape = sound_curve(self.settings.read(cx), &rule.sound).map(|curve| sketch(&curve));
        let label = device_label(&rule.device);
        let sound = sound_label(&rule.sound);
        let about: SharedString = match &marker {
            Some(marker) => {
                let separator = t!("equalizer-status-separator");
                format!("{sound}{separator}{marker}").into()
            }
            None => sound.clone(),
        };
        let id = SharedString::from(format!(
            "equalizer-rule-{}",
            rule.device.id().unwrap_or("other")
        ));
        let device = rule.device.clone();
        let remove = Button::new(("equalizer-rule-remove", index))
            .small()
            .ghost()
            .icon("icons/x.svg")
            .tooltip("equalizer-rule-remove")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.equalizer
                    .update(cx, |equalizer, cx| equalizer.remove_rule(&device, cx))
            }));

        div()
            .id(id)
            .role(Role::ListItem)
            .aria_label(label.clone())
            .aria_description(about)
            .aria_position_in_set(index + 1)
            .aria_size_of_set(count)
            .flex()
            .items_center()
            .gap_3()
            .h(theme.metrics.list_row)
            .when(!connected, |row| row.text_color(theme.muted_foreground))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(div().min_w_0().truncate().child(label))
                    .when_some(marker, |line, marker| {
                        line.child(
                            div()
                                .flex_none()
                                .text_size(theme.text(Text::Small))
                                .text_color(match playing {
                                    true => theme.primary,
                                    false => theme.muted_foreground,
                                })
                                .child(marker),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .w(theme.metrics.cover * SOUND_COLUMN)
                    .children(shape)
                    .child(div().min_w_0().truncate().child(sound)),
            )
            .child(remove)
            .into_any_element()
    }

    /// The row that adds a rule: a device picker, a sound picker, and the button that adds the
    /// rule once both are chosen. A device that has a rule already gets the new one instead.
    fn add_rule(&self, offered: Vec<Offered>, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let small = theme.text(Text::Small);
        let settings = self.settings.read(cx);
        let ruled = |device: &RuleDevice| {
            settings
                .equalizer_rules()
                .iter()
                .any(|rule| rule.device.id() == device.id())
        };
        let chosen = self.rule_device.as_ref();
        let item = |index: usize, offer: Offered| {
            let detail = match (ruled(&offer.device), offer.playing, offer.connected) {
                (true, _, _) => Some(t!("equalizer-rule-replaces")),
                (false, true, _) => Some(t!("equalizer-rule-playing")),
                (false, false, false) => Some(t!("equalizer-rule-disconnected")),
                (false, false, true) => None,
            };
            let device = offer.device;
            MenuItem::new(("equalizer-rule-device", index), device_label(&device))
                .selected(chosen.is_some_and(|chosen| chosen.id() == device.id()))
                .when_some(detail, |item, detail| {
                    item.detail(
                        div()
                            .text_size(small)
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.rule_device = Some(device.clone());
                    cx.notify();
                }))
        };
        let items = offered.into_iter().enumerate().flat_map(|(index, offer)| {
            let divider = (index > 0 && offer.device == RuleDevice::Other)
                .then(|| MenuItem::separator("equalizer-rule-devices-other"));
            divider.into_iter().chain([item(index, offer)])
        });
        let items: Vec<MenuItem> = items.collect();
        let height = theme.metrics.cover * SOUND_MENU_HEIGHT;
        let device_picker = Picker::new(
            DEVICE,
            &self.popovers,
            chosen.map_or_else(|| t!("equalizer-rule-choose-device"), device_label),
        )
        .name(t!("equalizer-rule-device"))
        .left()
        .width(Picker::WIDE)
        .w(Picker::REGULAR)
        .on_open(cx.listener(|this, _, _, cx| {
            this.outputs.update(cx, |outputs, cx| outputs.refresh(cx));
        }))
        .scroll_after(height)
        .items(items);

        let picked = self.rule_sound.clone();
        let shape = picked
            .as_ref()
            .and_then(|sound| sound_curve(settings, sound))
            .map(|curve| sketch(&curve));
        let view = cx.entity().downgrade();
        let sounds = sound_items(
            settings,
            |sound| picked.as_ref() == Some(sound),
            move |sound, cx| {
                let sound = sound.clone();
                view.update(cx, |this, cx| {
                    this.rule_sound = Some(sound);
                    cx.notify();
                })
                .ok();
            },
        );
        let sound_picker = Picker::new(
            SOUND,
            &self.popovers,
            picked
                .as_ref()
                .map_or_else(|| t!("equalizer-rule-choose-sound"), sound_label),
        )
        .name(t!("equalizer-sound"))
        .when_some(shape, Picker::leading)
        .left()
        .width(Picker::REGULAR)
        .w(Picker::NARROW)
        .scroll_after(height)
        .items(sounds);

        let ready = self.rule_device.is_some() && self.rule_sound.is_some();
        let add = Button::new("equalizer-rule-add")
            .small()
            .outline()
            .icon("icons/plus.svg")
            .label(t!("equalizer-rule-add"))
            .disabled(!ready)
            .on_click(cx.listener(|this, _, _, cx| this.submit_rule(cx)));

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .min_h(theme.metrics.list_row)
            .child(device_picker)
            .child(sound_picker)
            .child(div().flex_1())
            .child(add)
            .into_any_element()
    }

    /// Adds the rule the add row holds, in place of the one its device had, and empties the row.
    fn submit_rule(&mut self, cx: &mut Context<Self>) {
        let (Some(device), Some(sound)) = (self.rule_device.take(), self.rule_sound.take()) else {
            return;
        };
        self.equalizer.update(cx, |equalizer, cx| {
            equalizer.set_rule(DeviceRule { device, sound }, cx)
        });
        cx.notify();
    }

    /// Every device a rule can be added for, each once: the output playing now, then the devices
    /// the output picker lists, a sound server's sink once for each of its ports, then the devices
    /// that have a rule, and last every other device.
    fn offered(&self, cx: &App) -> Vec<Offered> {
        let outputs = self.outputs.read(cx);
        let current = outputs.binding_key().map(|output| OutputId {
            name: output_name(
                &output.name,
                outputs
                    .playing_sink()
                    .and_then(Sink::shown_port)
                    .map(|port| port.name.as_str()),
            )
            .into(),
            id: output.id,
        });
        let devices = outputs.devices().iter().flat_map(device_outputs);
        let ruled = self
            .settings
            .read(cx)
            .equalizer_rules()
            .iter()
            .filter_map(|rule| match &rule.device {
                RuleDevice::Output(output) => Some((output.clone(), false)),
                RuleDevice::Other => None,
            });
        let playing = current.as_ref().map(|output| output.id.clone());

        let mut offered: Vec<Offered> = Vec::new();
        for (output, connected) in current
            .map(|output| (output, true))
            .into_iter()
            .chain(devices)
            .chain(ruled)
        {
            if offered
                .iter()
                .any(|offer| offer.device.id() == Some(output.id.as_str()))
            {
                continue;
            }
            offered.push(Offered {
                playing: playing.as_deref() == Some(output.id.as_str()),
                connected: connected || playing.as_deref() == Some(output.id.as_str()),
                device: RuleDevice::Output(output),
            });
        }
        offered.push(Offered {
            device: RuleDevice::Other,
            playing: false,
            connected: true,
        });
        offered
    }
}

/// What a rule's device is called: the output's name as it was seen, or the words for every
/// other device.
fn device_label(device: &RuleDevice) -> SharedString {
    match device {
        RuleDevice::Output(output) => output.name.clone().into(),
        RuleDevice::Other => t!("equalizer-rule-other"),
    }
}

/// An output's name, with the port that plays when it has more than one.
fn output_name(name: &str, port: Option<&str>) -> SharedString {
    match port {
        Some(port) => t!(
            "equalizer-output-port",
            device = name.to_owned(),
            port = port.to_owned()
        ),
        None => name.to_owned().into(),
    }
}

/// The outputs a listed device offers rules for, each with whether it is connected: one for each
/// port of a sound server's sink, named with the port when there are several, else the device.
fn device_outputs(device: &DeviceOption) -> Vec<(OutputId, bool)> {
    let Some(sink) = device.sink.as_ref().filter(|sink| !sink.ports.is_empty()) else {
        let output = OutputId {
            id: device.id.clone(),
            name: device.name.clone(),
        };
        return vec![(output, true)];
    };
    let several = sink.ports.len() > 1;
    sink.ports
        .iter()
        .map(|port| {
            let shown = several.then_some(port.name.as_str());
            let output = OutputId {
                id: output::output_key(&device.id, Some(port)),
                name: output_name(&device.name, shown).into(),
            };
            (output, port.available)
        })
        .collect()
}
