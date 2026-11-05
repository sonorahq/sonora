use gpui::prelude::*;
use gpui::{Context, Entity, SharedString, Window, div};
use i18n::t;
use state::Outputs;
use ui::ActiveTheme as _;
use ui::{MenuItem, Picker, Popovers, Spinner, Text};

use crate::shared::device_menu::{DeviceMenu, device_rows};

/// The popover key of the device menu, among the other popovers of the page it sits on.
const OUTPUT_DEVICES: &str = "output-devices";

/// The output device picker: "System default" first, then every device the system offers in the
/// shared `DeviceMenu`. Picking goes through `Outputs`, and the picker is held behind a spinner
/// while a pick is being opened.
pub(crate) struct OutputPicker {
    outputs: Entity<Outputs>,
    popovers: Popovers,
    menu: DeviceMenu,
}

impl OutputPicker {
    /// `popovers` is the group of the page the picker sits on, so opening it closes the rest.
    pub(crate) fn new(
        outputs: Entity<Outputs>,
        popovers: Popovers,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&outputs, |_, _, cx| cx.notify()).detach();
        Self {
            outputs,
            popovers,
            menu: DeviceMenu::new(cx),
        }
    }
}

impl Render for OutputPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let small = theme.text(Text::Small);

        let rows = device_rows(self.outputs.read(cx).devices(), |_| true);
        let open = self.popovers.shows(OUTPUT_DEVICES);
        self.menu.prepare(&rows, open, cx);

        let outputs = self.outputs.read(cx);
        let chosen = outputs.chosen();
        let switching = outputs.switching();
        let devices = outputs.devices();
        let label = match chosen {
            None => t!("settings-output-device-system"),
            Some(id) => match devices.iter().find(|device| device.id == id) {
                Some(device) => device.name.clone().into(),
                // until the list is read, the open stream is the one place the name is known
                None if !outputs.listed() => match outputs.active().filter(|open| open.id == id) {
                    Some(open) => open.name.clone().into(),
                    None => t!("settings-output-device-loading"),
                },
                None => t!("settings-output-device-missing"),
            },
        };

        // the default names the device it plays on, which only a sound server says
        let current = devices
            .iter()
            .find(|device| device.system)
            .map(|device| device.name.clone());
        let default = MenuItem::new("output-system", t!("settings-output-device-system"))
            .selected(chosen.is_none())
            .when_some(current, |item, current| {
                item.detail(div().text_color(muted).text_size(small).child(current))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.outputs
                    .update(cx, |outputs, cx| outputs.choose(None, cx));
            }));
        let item = |index: usize, label: String| {
            let device = &devices[index];
            let id = device.id.clone();
            let item = MenuItem::new(SharedString::from(format!("output-device-{id}")), label)
                .selected(chosen == Some(id.as_str()))
                .when_some(device.detail.clone(), |item, detail| {
                    item.detail(div().text_color(muted).text_size(small).child(detail))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.outputs
                        .update(cx, |outputs, cx| outputs.choose(Some(id), cx));
                }));
            vec![item]
        };
        let items = self.menu.items(
            rows,
            devices,
            |device| chosen == Some(device.id.as_str()),
            item,
        );
        // a device plugged in since the list was read is only found by asking again
        let picker = Picker::new(OUTPUT_DEVICES, &self.popovers, label)
            .name(t!("settings-output-device"))
            // a long device name is cut short rather than crowding the setting's own label
            .max_w(Picker::WIDE)
            .on_open(cx.listener(|this, _, _, cx| {
                this.outputs.update(cx, |outputs, cx| outputs.refresh(cx));
            }))
            .menu(self.menu.menu("output-devices-menu"))
            .disabled(switching)
            .item(default)
            .items(items);

        div()
            .flex()
            .items_center()
            .gap_2()
            .when(switching, |row| row.child(Spinner::new("output-switching")))
            .child(picker)
    }
}
