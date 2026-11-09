//! The player bar's Sessions button, for listening elsewhere and together: the account's other
//! devices and the Jam when the provider has a device network the listener keeps on, and Sonora
//! Sessions on every streaming provider. The player bar leaves it out while no streaming
//! provider is signed in.

use i18n::t;
use music::connect::{DeviceKind, Elsewhere};
use rune::{Cx, El, Tx};
use state::SideTab;
use ui::{Menu, MenuItem, Picker};

use crate::chrome::{jam, members, session, sidebar_right};
use crate::{Ui, act, act_tx};

/// The popover key of the Sessions menu among the chrome's popovers.
const SESSIONS: &str = "sessions";
/// How many faces the Members item shows, and how much of a small control each one fills.
const MEMBER_FACES: usize = 3;
const MEMBER_FACE: f32 = 0.7;

/// The Sessions button's own state: whether the Spotify Jams submenu is open.
#[derive(Default)]
pub struct State {
    jams: bool,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let _ = cx;
    let theme = &ui.theme;
    let devices = &ui.store.devices;
    let connect = devices.available();
    let elsewhere = devices.elsewhere().cloned().filter(|_| connect);
    let together = ui.store.listening.room().is_some();
    let mut items = match connect {
        true => connect_items(ui, elsewhere.as_ref()),
        false => Vec::new(),
    };
    if !items.is_empty() {
        items.push(MenuItem::separator("session-separator"));
    }
    if connect {
        items.push(jam_item(ui));
    }
    items.extend(members_item(ui));
    items.extend(session_items(ui));
    // an accent dot over the icon says a session or a Jam is under way, or that playback is on
    // another device
    let busy = together || (connect && devices.jam().is_some()) || elsewhere.is_some();
    // while another device plays, the button wears that device's icon
    let glyph = elsewhere
        .as_ref()
        .map_or("icons/monitor-speaker.svg", |elsewhere| {
            icon(elsewhere.device.kind)
        });
    let mut picker = Picker::icon(
        SESSIONS,
        &ui.chrome.popovers,
        |ui: &mut Ui| &mut ui.chrome.popovers,
        glyph,
    )
    .tooltip_above(t!("player-sessions"))
    .name(t!("player-sessions"))
    .tint(match busy {
        true => theme.foreground,
        false => theme.muted_foreground,
    })
    .width(Picker::<Ui>::WIDE)
    .items(items);
    if busy {
        picker = picker.status(theme.primary);
    }
    picker.build(theme, layer)
}

/// The account's other devices, with the one that plays marked.
fn connect_items(ui: &Ui, elsewhere: Option<&Elsewhere>) -> Vec<MenuItem<Ui>> {
    let mut items = vec![
        MenuItem::new("device-here", t!("devices-this-computer"))
            .icon(icon(DeviceKind::Computer))
            .selected(elsewhere.is_none())
            .on_press(act(|ui: &mut Ui| ui.store.devices.transfer_here())),
    ];
    for device in ui.store.devices.devices() {
        let id = device.id.clone();
        let playing = elsewhere.is_some_and(|elsewhere| elsewhere.device.id == device.id);
        items.push(
            MenuItem::new(format!("device-{}", device.id), device.name.clone())
                .icon(icon(device.kind))
                .selected(playing)
                .on_press(act(move |ui: &mut Ui| ui.store.devices.transfer(&id))),
        );
    }
    items
}

/// The Spotify Jam's items, folded into one submenu so they do not crowd the session's.
fn jam_item(ui: &Ui) -> MenuItem<Ui> {
    let mut entries = Vec::new();
    match ui.store.devices.jam() {
        Some(jam) => {
            if let Some(link) = jam.link.clone() {
                entries.push(
                    MenuItem::new("jam-code", t!("devices-jam-code-menu"))
                        .icon("icons/qr-code.svg")
                        .on_press(act(|ui: &mut Ui| {
                            ui.store.devices.show_code(true, &mut ui.store.cx)
                        })),
                );
                entries.push(
                    MenuItem::new("jam-copy", t!("devices-jam-copy"))
                        .icon("icons/copy.svg")
                        .on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
                            jam::copy_link(ui, tx, &link)
                        })),
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
                .on_press(act(|ui: &mut Ui| ui.store.devices.leave(&mut ui.store.cx))),
            );
        }
        None => {
            entries.push(
                MenuItem::new("jam-start", t!("devices-jam-start"))
                    .icon("icons/users-round-collaboration.svg")
                    .on_press(act(|ui: &mut Ui| ui.store.devices.start_jam())),
            );
            entries.push(
                MenuItem::new("jam-join", t!("devices-jam-join"))
                    .icon("icons/link.svg")
                    .on_press(act(|ui: &mut Ui| {
                        ui.store.devices.show_join(true, &mut ui.store.cx)
                    })),
            );
        }
    }
    MenuItem::new("spotify-jams", t!("devices-spotify-jams"))
        .icon("icons/spotify.svg")
        .submenu(
            Menu::new("spotify-jams-menu")
                .width(Picker::<Ui>::WIDE)
                .on_pick(|ui: &mut Ui, _: &mut Tx| {
                    ui.chrome.sessions.jams = false;
                    ui.chrome.popovers.close();
                })
                .items(entries),
            ui.chrome.sessions.jams,
            |ui: &mut Ui, open: bool| ui.chrome.sessions.jams = open,
        )
}

/// Opens the Members panel, with the first faces of the people in the session or Jam at the end
/// of the row. `None` while the listener is in neither.
fn members_item(ui: &Ui) -> Option<MenuItem<Ui>> {
    let people = members::gathered(ui);
    if people.is_empty() {
        return None;
    }
    let size = ui.theme.metrics.control_small * MEMBER_FACE;
    Some(
        MenuItem::new("members", t!("members-title"))
            .icon("icons/user.svg")
            .trailing(jam::faces(ui, &people, MEMBER_FACES, size, 0.))
            .on_press(act(|ui: &mut Ui| {
                sidebar_right::reveal(ui, SideTab::Members)
            })),
    )
}

/// Starting or joining a Sonora Session, or what to do with the one the listener is in.
fn session_items(ui: &Ui) -> Vec<MenuItem<Ui>> {
    let listening = &ui.store.listening;
    let Some(room) = listening.room() else {
        let asking = listening.asking();
        let start = MenuItem::new("session-start", t!("devices-session-start"))
            .icon("icons/users-round-collaboration.svg")
            .on_press(act(|ui: &mut Ui| ui.store.start_listening()));
        let join = MenuItem::new("session-join", t!("devices-session-join"))
            .icon("icons/rectangle-ellipsis.svg")
            .on_press(act(session::open_join));
        return match asking {
            true => vec![start.disabled(), join.disabled()],
            false => vec![start, join],
        };
    };
    vec![
        MenuItem::new("session-code", t!("devices-session-code-menu"))
            .icon("icons/rectangle-ellipsis.svg")
            .on_press(act(|ui: &mut Ui| ui.store.listening.show_code(true))),
        MenuItem::new(
            "session-leave",
            match room.hosting() {
                true => t!("devices-session-end"),
                false => t!("devices-session-leave"),
            },
        )
        .icon("icons/log-out.svg")
        .on_press(act(|ui: &mut Ui| {
            ui.store.listening.leave(&mut ui.store.cx)
        })),
    ]
}

/// The icon a device of `kind` wears.
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
