//! The output device picker: "System default" first, then every device the system offers.
//! Picking goes through `Outputs`, and the picker is held behind a spinner while a pick is
//! being opened.

use i18n::t;
use rune::{Cx, El, Tx, row, text};
use ui::{MenuItem, Picker, Spinner, Text, Weight};

use crate::chrome::player_bar::rems;
use crate::shared::device_menu::{self, DeviceMenu, device_rows};
use crate::{Ui, act};

/// The popover key of the device menu among the chrome's popovers.
const OUTPUT_DEVICES: &str = "output-devices";

/// The picker's own state: which of the device menu's submenus is open.
#[derive(Default)]
pub struct State {
    menu: DeviceMenu,
}

/// The picker, with the spinner before it while a device is being switched to.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let _ = cx;
    let theme = &ui.theme;
    let outputs = &ui.store.outputs;
    let chosen = outputs.chosen();
    let switching = outputs.switching();
    let devices = outputs.devices();
    let label = match chosen {
        None => t!("settings-output-device-system").into_owned(),
        Some(id) => match devices.iter().find(|device| device.id == id) {
            Some(device) => device.name.clone(),
            // until the list is read, the open stream is the one place the name is known
            None if !outputs.listed() => match outputs.active().filter(|open| open.id == id) {
                Some(open) => open.name.clone(),
                None => t!("settings-output-device-loading").into_owned(),
            },
            None => t!("settings-output-device-missing").into_owned(),
        },
    };
    let detail = |words: String| {
        text(words)
            .font(ui::font(theme, Text::Small, Weight::NORMAL))
            .color(theme.muted_foreground.color())
            .wrap()
    };

    // the default names the device it plays on, which only a sound server says
    let current = devices
        .iter()
        .find(|device| device.system)
        .map(|device| device.name.clone());
    let mut default = MenuItem::<Ui>::new("output-system", t!("settings-output-device-system"))
        .selected(chosen.is_none())
        .on_press(act(|ui: &mut Ui| {
            ui.store.outputs.choose(None, &mut ui.store.cx)
        }));
    if let Some(current) = current {
        default = default.detail(detail(current));
    }
    let item = |index: usize, label: String| {
        let device = &devices[index];
        let id = device.id.clone();
        let mut item = MenuItem::<Ui>::new(format!("output-device-{id}"), label)
            .selected(chosen == Some(id.as_str()))
            .on_press(act(move |ui: &mut Ui| {
                ui.store.outputs.choose(Some(id.clone()), &mut ui.store.cx)
            }));
        if let Some(about) = device.detail.clone() {
            item = item.detail(detail(about));
        }
        vec![item]
    };
    let items = ui.chrome.output_picker.menu.items(
        device_rows(devices, |_| true),
        devices,
        |device| chosen == Some(device.id.as_str()),
        item,
        |ui: &mut Ui| &mut ui.chrome.output_picker.menu,
    );
    let picker = Picker::new(
        OUTPUT_DEVICES,
        &ui.chrome.popovers,
        |ui: &mut Ui| &mut ui.chrome.popovers,
        label,
    )
    .name(t!("settings-output-device"))
    // a device plugged in since the list was read is only found by asking again
    .on_open(|ui: &mut Ui, _: &mut Tx| {
        ui.chrome.output_picker.menu.reset();
        ui.store.outputs.refresh(&mut ui.store.cx);
        ui.settle();
    })
    .menu(device_menu::menu("output-devices-menu"))
    .disabled(switching)
    .item(default)
    .items(items)
    .build(theme, layer)
    // a long device name is cut short rather than crowding the setting's own label
    .max_w(Picker::<Ui>::WIDE);

    let mut line = row().key("output-picker").center_y().gap(rems(theme, 0.5));
    if switching {
        line = line.child(
            Spinner::new()
                .name(t!("settings-output-device-loading"))
                .build(theme),
        );
    }
    line.child(picker)
}
