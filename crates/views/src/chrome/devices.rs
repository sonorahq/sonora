use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Entity, SharedString, Window, div};
use i18n::t;
use music::connect::{Command, DeviceKind};
use state::{Devices, Sonora};
use ui::{ActiveTheme as _, MenuItem, Picker, Popovers};

use crate::chrome::jam;

/// The player bar's button for the account's other devices: where playback is, and the list to
/// move it between this app and the rest. It draws nothing unless the signed-in provider has a
/// device network and the listener has it on.
pub(crate) struct DevicePicker {
    devices: Entity<Devices>,
    popovers: Popovers,
}

impl DevicePicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();

        Self {
            devices,
            popovers: Popovers::default(),
        }
    }
}

impl Render for DevicePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.devices.read(cx);
        if !state.available() {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let elsewhere = state.elsewhere().cloned();
        let devices = self.devices.clone();

        let mut items = vec![
            MenuItem::new("device-here", t!("devices-this-computer"))
                .icon(icon(DeviceKind::Computer))
                .selected(elsewhere.is_none())
                .on_click({
                    let devices = devices.clone();
                    move |_, _, cx| devices.read(cx).transfer_here()
                }),
        ];
        for device in state.devices() {
            let id = device.id.clone();
            let playing = elsewhere
                .as_ref()
                .is_some_and(|elsewhere| elsewhere.device.id == device.id);
            items.push(
                MenuItem::new(
                    SharedString::from(format!("device-{}", device.id)),
                    device.name.clone(),
                )
                .icon(icon(device.kind))
                .selected(playing)
                .on_click({
                    let devices = devices.clone();
                    move |_, _, cx| devices.read(cx).transfer(&id)
                }),
            );
        }

        if let Some(elsewhere) = &elsewhere {
            let id = elsewhere.device.id.clone();
            let control = |command: Command| {
                let devices = devices.clone();
                let id = id.clone();
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    devices.read(cx).control(&id, command.clone())
                }
            };
            let (toggle, label, glyph) = match elsewhere.state.playing {
                true => (Command::Pause, t!("devices-pause"), "icons/pause.svg"),
                false => (Command::Play, t!("devices-resume"), "icons/play.svg"),
            };

            items.push(MenuItem::separator("device-separator"));
            items.push(
                MenuItem::new("device-previous", t!("player-previous"))
                    .icon("icons/skip-back.svg")
                    .on_click(control(Command::Previous)),
            );
            items.push(
                MenuItem::new("device-toggle", label)
                    .icon(glyph)
                    .on_click(control(toggle)),
            );
            items.push(
                MenuItem::new("device-next", t!("player-next"))
                    .icon("icons/skip-forward.svg")
                    .on_click(control(Command::Next)),
            );
        }

        items.push(MenuItem::separator("jam-separator"));
        match state.jam() {
            Some(jam) => {
                if let Some(link) = jam.link.clone() {
                    items.push(
                        MenuItem::new("jam-code", t!("devices-jam-code-menu"))
                            .icon("icons/qr-code.svg")
                            .on_click({
                                let devices = devices.clone();
                                move |_, _, cx| {
                                    devices.update(cx, |devices, cx| devices.show_code(true, cx))
                                }
                            }),
                    );
                    items.push(
                        MenuItem::new("jam-copy", t!("devices-jam-copy"))
                            .icon("icons/copy.svg")
                            .on_click(move |_, _, cx| jam::copy_link(&link, cx)),
                    );
                }
                items.push(
                    MenuItem::new(
                        "jam-leave",
                        match jam.owner {
                            true => t!("devices-jam-end"),
                            false => t!("devices-jam-leave"),
                        },
                    )
                    .icon("icons/log-out.svg")
                    .on_click({
                        let devices = devices.clone();
                        move |_, _, cx| devices.update(cx, |devices, cx| devices.leave(cx))
                    }),
                );
            }
            None => {
                items.push(
                    MenuItem::new("jam-start", t!("devices-jam-start"))
                        .icon("icons/users-round-collaboration.svg")
                        .on_click({
                            let devices = devices.clone();
                            move |_, _, cx| devices.update(cx, |devices, _| devices.start_jam())
                        }),
                );
                items.push(
                    MenuItem::new("jam-join", t!("devices-jam-join"))
                        .icon("icons/link.svg")
                        .on_click({
                            let devices = devices.clone();
                            move |_, _, cx| {
                                devices.update(cx, |devices, cx| devices.show_join(true, cx))
                            }
                        }),
                );
            }
        }

        Picker::icon("devices", &self.popovers, "icons/monitor-speaker.svg")
            .tooltip_above("player-devices")
            .selected(elsewhere.is_some())
            .tint(match elsewhere.is_some() {
                true => theme.foreground,
                false => theme.muted_foreground,
            })
            .width(Picker::WIDE)
            .items(items)
            .into_any_element()
    }
}

fn icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Computer => "icons/monitor.svg",
        DeviceKind::Phone => "icons/smartphone.svg",
        DeviceKind::Tablet => "icons/tablet.svg",
        DeviceKind::Tv => "icons/tv.svg",
        DeviceKind::Speaker | DeviceKind::Console | DeviceKind::Car | DeviceKind::Other => {
            "icons/speaker.svg"
        }
    }
}
