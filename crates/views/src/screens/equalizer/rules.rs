//! The device rules of advanced mode: which sound plays once the sound moves to a device, a row
//! per rule and a row that adds one.

use i18n::t;
use music::output::{self, DeviceOption, OutputId, Sink};
use rune::{El, Tx, col, el, row, text};
use state::{DeviceRule, RuleDevice, Store};
use ui::{Button, MenuItem, Picker, Text, Weight, font};

use super::heading;
use crate::shared::device_menu::{self, device_rows};
use crate::shared::equalizer::{SOUND_MENU_HEIGHT, sketch, sound_curve, sound_items, sound_label};
use crate::shared::page::rems;
use crate::{Ui, act};

const DEVICE: &str = "equalizer-rule-device";
const SOUND: &str = "equalizer-rule-sound";
/// The icon before the group's heading.
const ICON: &str = "icons/headphones.svg";
/// How wide a rule's sound column is, in cover sizes, so the sounds line up under each other.
const SOUND_COLUMN: f32 = 1.4;

/// The device rules' group: a row for each rule with a remove button, then a row that adds one.
/// A rule says which sound plays once the sound moves to its device.
pub(super) fn view(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let playing = playing_output(ui);
    let listed = listed_outputs(ui);
    let settings = &ui.store.settings;
    let governing = playing
        .as_ref()
        .and_then(|output| settings.equalizer_rule_for_output(&output.id))
        .map(|rule| rule.device.clone());
    let rules = settings.equalizer_rules();
    let count = rules.len();
    let rows: Vec<El> = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let connected = match &rule.device {
                RuleDevice::Other => true,
                RuleDevice::Output(output) => {
                    playing.as_ref().is_some_and(|now| now.id == output.id)
                        || listed
                            .iter()
                            .any(|(listed, connected)| *connected && listed.id == output.id)
                }
            };
            let playing = governing
                .as_ref()
                .is_some_and(|device| device.id() == rule.device.id());
            rule_row(ui, index, count, rule, playing, connected)
        })
        .collect();

    let mut group = col().key("equalizer-rules-group").w_full().child(heading(
        theme,
        "equalizer-rules",
        ICON,
        None,
    ));
    if !rows.is_empty() {
        group = group.child(
            col()
                .key("equalizer-rule-list")
                .role("list")
                .label(t!("equalizer-rules"))
                .w_full()
                .children(rows),
        );
    }
    group.child(add_rule(ui, layer, playing, listed))
}

/// One rule: its device, marked when it is playing now or not connected, the sound it plays,
/// and a button that removes it. Screen readers hear it as item `index` of `count`, named by its
/// device with the sound and the mark after it.
fn rule_row(
    ui: &Ui,
    index: usize,
    count: usize,
    rule: &DeviceRule,
    playing: bool,
    connected: bool,
) -> El {
    let theme = &ui.theme;
    let marker = match (playing, connected) {
        (true, _) => Some(t!("equalizer-rule-playing").into_owned()),
        (false, false) => Some(t!("equalizer-rule-disconnected").into_owned()),
        (false, true) => None,
    };
    let shape = sound_curve(&ui.store.settings, &rule.sound).map(|curve| sketch(&curve));
    let label = device_label(&rule.device);
    let sound = sound_label(&rule.sound);
    let about = match &marker {
        Some(marker) => format!("{sound}{}{marker}", t!("equalizer-status-separator")),
        None => sound.clone(),
    };
    let device = rule.device.clone();
    let remove = Button::new(format!("equalizer-rule-remove-{index}"))
        .small()
        .ghost()
        .icon("icons/x.svg")
        .tooltip(t!("equalizer-rule-remove"))
        .on_press(act(move |ui: &mut Ui| {
            let Store {
                equalizer,
                settings,
                cx,
                ..
            } = &mut ui.store;
            equalizer.remove_rule(&device, settings, cx);
        }))
        .build(theme);
    let ink = match connected {
        true => theme.foreground,
        false => theme.muted_foreground,
    };
    let body = font(theme, Text::Body, Weight::NORMAL);
    let mut name = row()
        .key("device")
        .grow(1.)
        .shrink()
        .min_w(0.)
        .center_y()
        .gap(rems(theme, 0.5))
        .child(
            text(label.clone())
                .font(body)
                .color(ink.color())
                .shrink()
                .min_w(0.)
                .truncate()
                .a11y_hidden(),
        );
    if let Some(marker) = marker {
        name = name.child(
            text(marker)
                .font(font(theme, Text::Small, Weight::NORMAL))
                .color(match playing {
                    true => theme.primary.color(),
                    false => theme.muted_foreground.color(),
                })
                .a11y_hidden(),
        );
    }
    let mut sound_column = row()
        .key("sound")
        .w(theme.metrics.cover * SOUND_COLUMN)
        .center_y()
        .gap(rems(theme, 0.5));
    if let Some(shape) = shape {
        sound_column = sound_column.child(shape.build(theme));
    }
    sound_column = sound_column.child(
        text(sound)
            .font(body)
            .color(ink.color())
            .shrink()
            .min_w(0.)
            .truncate()
            .a11y_hidden(),
    );

    row()
        .key(format!(
            "equalizer-rule-{}",
            rule.device.id().unwrap_or("other")
        ))
        .role("listitem")
        .label(label)
        .description(about)
        .position_in_set(index + 1, count)
        .w_full()
        .center_y()
        .gap(rems(theme, 0.75))
        .h(theme.metrics.list_row)
        .transition(rune::Transition::opacity())
        .child(name)
        .child(sound_column)
        .child(remove)
}

/// The row that adds a rule: a device picker, a sound picker, and the button that adds the rule
/// once both are chosen. A device that has a rule already gets the new one instead. The device
/// picker opens the output picker's own menu, with the output playing now first when the system
/// does not list it, the outputs that have a rule but are not connected after the listed ones,
/// and every other device last.
fn add_rule(
    ui: &Ui,
    layer: &mut Vec<El>,
    playing: Option<OutputId>,
    listed: Vec<(OutputId, bool)>,
) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.equalizer;
    let settings = &ui.store.settings;
    let devices = ui.store.outputs.devices();
    let rows = device_rows(devices, |device| !output::is_direct_copy(&device.id));
    let small = font(theme, Text::Small, Weight::NORMAL);
    let ruled = |device: &RuleDevice| {
        settings
            .equalizer_rules()
            .iter()
            .any(|rule| rule.device.id() == device.id())
    };
    let chosen = state.rule_device.as_ref();
    let playing_id = playing.as_ref().map(|output| output.id.clone());
    let item = |device: RuleDevice, label: String, connected: bool, about: Option<&String>| {
        let playing = device.id().is_some() && device.id() == playing_id.as_deref();
        let detail = match (ruled(&device), playing, connected || playing) {
            (true, _, _) => Some(t!("equalizer-rule-replaces").into_owned()),
            (false, true, _) => Some(t!("equalizer-rule-playing").into_owned()),
            (false, false, false) => Some(t!("equalizer-rule-disconnected").into_owned()),
            (false, false, true) => about.cloned(),
        };
        let key = format!("equalizer-rule-device-{}", device.id().unwrap_or("other"));
        let mut item = MenuItem::<Ui>::new(key, label)
            .selected(chosen.is_some_and(|chosen| chosen.id() == device.id()));
        if let Some(detail) = detail {
            item = item.detail(
                text(detail)
                    .font(small)
                    .color(theme.muted_foreground.color()),
            );
        }
        item.on_press(move |ui: &mut Ui| {
            let state = &mut ui.screens.equalizer;
            state.rule_device = Some(device.clone());
            state.popovers.close();
        })
    };
    let known = |id: &str| listed.iter().any(|(output, _)| output.id == id);

    let leading = playing
        .clone()
        .filter(|output| !known(&output.id))
        .map(|output| {
            let label = output.name.clone();
            item(RuleDevice::Output(output), label, true, None)
        });
    let chosen_id = chosen.and_then(RuleDevice::id);
    let listed_items = state.device_menu.items(
        rows,
        devices,
        |device| chosen_id.is_some_and(|id| belongs(id, &device.id)),
        |index, label| {
            let device = &devices[index];
            device_outputs(device, &label)
                .into_iter()
                .map(|(output, label, connected)| {
                    item(
                        RuleDevice::Output(output),
                        label,
                        connected,
                        device.detail.as_ref(),
                    )
                })
                .collect()
        },
        |ui: &mut Ui| &mut ui.screens.equalizer.device_menu,
    );
    let absent = settings
        .equalizer_rules()
        .iter()
        .filter_map(|rule| match &rule.device {
            RuleDevice::Output(output) => Some(output.clone()),
            RuleDevice::Other => None,
        })
        .filter(|output| !known(&output.id) && playing_id.as_deref() != Some(&output.id))
        .map(|output| {
            let label = output.name.clone();
            item(RuleDevice::Output(output), label, false, None)
        });
    let other = [
        MenuItem::separator("equalizer-rule-devices-other"),
        item(
            RuleDevice::Other,
            t!("equalizer-rule-other").into_owned(),
            true,
            None,
        ),
    ];
    let items: Vec<MenuItem<Ui>> = leading
        .into_iter()
        .chain(listed_items)
        .chain(absent)
        .chain(other)
        .collect();

    // a device plugged in since the list was read is only found by asking again
    let device_picker = Picker::new(
        DEVICE,
        &state.popovers,
        |ui: &mut Ui| &mut ui.screens.equalizer.popovers,
        chosen.map_or_else(
            || t!("equalizer-rule-choose-device").into_owned(),
            device_label,
        ),
    )
    .name(t!("equalizer-rule-device"))
    .left()
    .on_open(|ui: &mut Ui, _: &mut Tx| {
        ui.screens.equalizer.device_menu.reset();
        ui.store.outputs.refresh(&mut ui.store.cx);
        ui.settle();
    })
    .menu(device_menu::menu("equalizer-rule-devices-menu"))
    .items(items)
    .build(theme, layer)
    .w(Picker::<Ui>::REGULAR);

    let height = theme.metrics.cover * SOUND_MENU_HEIGHT;
    let picked = state.rule_sound.clone();
    let shape = picked
        .as_ref()
        .and_then(|sound| sound_curve(settings, sound))
        .map(|curve| sketch(&curve));
    let sounds = sound_items(
        settings,
        theme,
        |sound| picked.as_ref() == Some(sound),
        |ui: &mut Ui, sound| {
            let state = &mut ui.screens.equalizer;
            state.rule_sound = Some(sound.clone());
            state.popovers.close();
        },
    );
    let mut sound_picker = Picker::new(
        SOUND,
        &state.popovers,
        |ui: &mut Ui| &mut ui.screens.equalizer.popovers,
        picked.as_ref().map_or_else(
            || t!("equalizer-rule-choose-sound").into_owned(),
            sound_label,
        ),
    )
    .name(t!("equalizer-sound"))
    .left()
    .width(Picker::<Ui>::REGULAR)
    .scroll_after(height)
    .items(sounds);
    if let Some(shape) = shape {
        sound_picker = sound_picker.leading(shape.build(theme));
    }
    let sound_picker = sound_picker.build(theme, layer).w(Picker::<Ui>::NARROW);

    let ready = state.rule_device.is_some() && state.rule_sound.is_some();
    let add = Button::new("equalizer-rule-add")
        .small()
        .outline()
        .icon("icons/plus.svg")
        .label(t!("equalizer-rule-add"))
        .disabled(!ready)
        .on_press(act(submit_rule))
        .build(theme);

    row()
        .key("equalizer-rule-new")
        .w_full()
        .flex_wrap()
        .center_y()
        .gaps(rems(theme, 0.5), rems(theme, 0.5))
        .min_h(theme.metrics.list_row)
        .child(device_picker)
        .child(sound_picker)
        .child(el().key("gap").grow(1.))
        .child(add)
}

/// Adds the rule the add row holds, in place of the one its device had, and empties the row.
fn submit_rule(ui: &mut Ui) {
    let state = &mut ui.screens.equalizer;
    let (Some(device), Some(sound)) = (state.rule_device.take(), state.rule_sound.take()) else {
        return;
    };
    let Store {
        equalizer,
        settings,
        cx,
        ..
    } = &mut ui.store;
    equalizer.set_rule(DeviceRule { device, sound }, settings, cx);
}

/// The output playing now, named with the port that plays when its sink has several.
fn playing_output(ui: &Ui) -> Option<OutputId> {
    let outputs = &ui.store.outputs;
    outputs.binding_key().map(|output| OutputId {
        name: output_name(
            &output.name,
            outputs
                .playing_sink()
                .and_then(Sink::shown_port)
                .map(|port| port.name.as_str()),
        ),
        id: output.id,
    })
}

/// Every output the system lists that a rule can be added for, each with whether it is
/// connected: a sound server's sink once for each of its ports.
fn listed_outputs(ui: &Ui) -> Vec<(OutputId, bool)> {
    ui.store
        .outputs
        .devices()
        .iter()
        .filter(|device| !output::is_direct_copy(&device.id))
        .flat_map(|device| device_outputs(device, &device.name))
        .map(|(output, _, connected)| (output, connected))
        .collect()
}

/// What a rule's device is called: the output's name as it was seen, or the words for every
/// other device.
fn device_label(device: &RuleDevice) -> String {
    match device {
        RuleDevice::Output(output) => output.name.clone(),
        RuleDevice::Other => t!("equalizer-rule-other").into_owned(),
    }
}

/// An output's name, with the port that plays when it has more than one.
fn output_name(name: &str, port: Option<&str>) -> String {
    match port {
        Some(port) => t!(
            "equalizer-output-port",
            device = name.to_owned(),
            port = port.to_owned()
        )
        .into_owned(),
        None => name.to_owned(),
    }
}

/// The outputs a listed device offers rules for, each with the label its menu row shows and
/// whether it is connected: one for each port of a sound server's sink, named with the port when
/// there are several, else the device. `label` is what the device goes by where the menu lists
/// it, while the output keeps the device's full name.
fn device_outputs(device: &DeviceOption, label: &str) -> Vec<(OutputId, String, bool)> {
    let Some(sink) = device.sink.as_ref().filter(|sink| !sink.ports.is_empty()) else {
        let output = OutputId {
            id: device.id.clone(),
            name: device.name.clone(),
        };
        return vec![(output, label.to_owned(), true)];
    };
    let several = sink.ports.len() > 1;
    sink.ports
        .iter()
        .map(|port| {
            let shown = several.then_some(port.name.as_str());
            let output = OutputId {
                id: output::output_key(&device.id, Some(port)),
                name: output_name(&device.name, shown),
            };
            (output, output_name(label, shown), port.available)
        })
        .collect()
}

/// Whether the output keyed `key` is the device `device` or one of its ports.
fn belongs(key: &str, device: &str) -> bool {
    key.strip_prefix(device)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('#'))
}
