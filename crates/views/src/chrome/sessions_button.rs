use gpui::prelude::*;
use gpui::{App, Context, Entity, SharedString, Window, div};
use i18n::t;
use input::ShowMembers;
use music::connect::{DeviceKind, Elsewhere};
use state::{Devices, Listening, Sonora};
use ui::{ActiveTheme as _, Menu, MenuItem, Picker, Popovers, SubmenuState};

use crate::chrome::jam;
use crate::chrome::members::gathered;

/// How many faces the Members item shows, and how much of a small control each one fills.
const MEMBER_FACES: usize = 3;
const MEMBER_FACE: f32 = 0.7;

/// The player bar's Sessions button, for listening elsewhere and together: the account's other
/// devices and the Jam when the provider has a device network the listener keeps on, and Sonora
/// Sessions on every streaming provider. It draws nothing while no streaming provider is signed
/// in.
pub(crate) struct SessionsButton {
    devices: Entity<Devices>,
    listening: Entity<Listening>,
    popovers: Popovers,
    /// The Spotify Jams submenu, kept between frames so it stays open under the pointer.
    jams: SubmenuState,
}

impl SessionsButton {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let devices = Sonora::global(cx).devices.clone();
        let listening = Sonora::global(cx).listening.clone();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        cx.observe(&listening, |_, _, cx| cx.notify()).detach();

        Self {
            devices,
            listening,
            popovers: Popovers::default(),
            jams: SubmenuState::default(),
        }
    }

    /// The account's other devices, with the one that plays marked.
    fn connect_items(&self, elsewhere: Option<&Elsewhere>, cx: &App) -> Vec<MenuItem> {
        let state = self.devices.read(cx);
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
            let playing = elsewhere.is_some_and(|elsewhere| elsewhere.device.id == device.id);
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

        items
    }

    /// The Spotify Jam's items, folded into one submenu so they do not crowd the session's.
    fn jam_item(&self, cx: &App) -> MenuItem {
        let state = self.devices.read(cx);
        let devices = self.devices.clone();
        let mut entries = Vec::new();
        match state.jam() {
            Some(jam) => {
                if let Some(link) = jam.link.clone() {
                    entries.push(
                        MenuItem::new("jam-code", t!("devices-jam-code-menu"))
                            .icon("icons/qr-code.svg")
                            .on_click({
                                let devices = devices.clone();
                                move |_, _, cx| {
                                    devices.update(cx, |devices, cx| devices.show_code(true, cx))
                                }
                            }),
                    );
                    entries.push(
                        MenuItem::new("jam-copy", t!("devices-jam-copy"))
                            .icon("icons/copy.svg")
                            .on_click(move |_, _, cx| jam::copy_link(&link, cx)),
                    );
                }
                entries.push(
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
                entries.push(
                    MenuItem::new("jam-start", t!("devices-jam-start"))
                        .icon("icons/users-round-collaboration.svg")
                        .on_click({
                            let devices = devices.clone();
                            move |_, _, cx| devices.update(cx, |devices, _| devices.start_jam())
                        }),
                );
                entries.push(
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
        MenuItem::new("spotify-jams", t!("devices-spotify-jams"))
            .icon("icons/spotify.svg")
            .submenu(
                Menu::new("spotify-jams-menu")
                    .w(Picker::WIDE)
                    .items(entries),
                self.jams.clone(),
            )
    }

    /// Opens the Members panel, with the first faces of the people in the session or Jam at the
    /// end of the row. `None` while the listener is in neither.
    fn members_item(&self, window: &Window, cx: &App) -> Option<MenuItem> {
        let people = gathered(cx);
        if people.is_empty() {
            return None;
        }
        let theme = *cx.theme();
        let faces = jam::faces(
            &people,
            MEMBER_FACES,
            theme.metrics.control_small * MEMBER_FACE,
            0.,
            window,
            theme,
        );
        Some(
            MenuItem::new("members", t!("members-title"))
                .icon("icons/user.svg")
                .trailing(faces)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowMembers), cx)),
        )
    }

    /// Starting or joining a Sonora Session, or what to do with the one the listener is in.
    fn session_items(&self, cx: &App) -> Vec<MenuItem> {
        let listening = self.listening.clone();
        let state = listening.read(cx);
        let Some(room) = state.room() else {
            return vec![
                MenuItem::new("session-start", t!("devices-session-start"))
                    .icon("icons/users-round-collaboration.svg")
                    .when(state.asking(), MenuItem::disabled)
                    .on_click({
                        let listening = listening.clone();
                        move |_, _, cx| listening.update(cx, |listening, cx| listening.start(cx))
                    }),
                MenuItem::new("session-join", t!("devices-session-join"))
                    .icon("icons/rectangle-ellipsis.svg")
                    .when(state.asking(), MenuItem::disabled)
                    .on_click(move |_, _, cx| {
                        listening.update(cx, |listening, cx| listening.show_join(true, cx))
                    }),
            ];
        };
        vec![
            MenuItem::new("session-code", t!("devices-session-code-menu"))
                .icon("icons/rectangle-ellipsis.svg")
                .on_click({
                    let listening = listening.clone();
                    move |_, _, cx| {
                        listening.update(cx, |listening, cx| listening.show_code(true, cx))
                    }
                }),
            MenuItem::new(
                "session-leave",
                match room.hosting() {
                    true => t!("devices-session-end"),
                    false => t!("devices-session-leave"),
                },
            )
            .icon("icons/log-out.svg")
            .on_click(move |_, _, cx| listening.update(cx, |listening, cx| listening.leave(cx))),
        ]
    }
}

impl Render for SessionsButton {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.devices.read(cx);
        let connect = state.available();
        let listening = self.listening.read(cx);
        if !connect && !listening.available(cx) {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let elsewhere = state.elsewhere().cloned().filter(|_| connect);
        let together = listening.room().is_some();
        let mut items = match connect {
            true => self.connect_items(elsewhere.as_ref(), cx),
            false => Vec::new(),
        };
        if !items.is_empty() {
            items.push(MenuItem::separator("session-separator"));
        }
        if connect {
            items.push(self.jam_item(cx));
        }
        items.extend(self.members_item(window, cx));
        items.extend(self.session_items(cx));
        // an accent dot over the icon says a session or a Jam is under way, or that playback is on
        // another device
        let busy = together || state.jam().is_some_and(|_| connect) || elsewhere.is_some();

        // while another device plays, the button wears that device's icon
        let glyph = elsewhere
            .as_ref()
            .map_or("icons/monitor-speaker.svg", |elsewhere| {
                icon(elsewhere.device.kind)
            });
        Picker::icon("sessions", &self.popovers, glyph)
            .tooltip_above("player-sessions")
            .tint(match busy {
                true => theme.foreground,
                false => theme.muted_foreground,
            })
            .when(busy, |picker| picker.status(theme.primary))
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
