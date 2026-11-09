//! The Members panel of the right sidebar: who is in the Sonora Session or Spotify Jam the
//! listener is in, with the session's code and the way out of it. `state::Listening` and
//! `state::Devices` answer everything, and this only draws it.

use i18n::t;
use music::connect::JamMember;
use rune::{Cx, El, ScrollSpec, Tx, col, row, text};
use ui::{Button, Text, Weight};

use crate::chrome::player_bar::rems;
use crate::chrome::{jam, session};
use crate::{Ui, act, act_tx};

/// How many faces the summary shows before it counts the rest.
const FACES: usize = 6;

/// What the panel lists: a Sonora Session first, since the two never play together, or else a
/// Jam on a provider with a device network the listener keeps on.
enum Gathering {
    Session {
        code: String,
        hosting: bool,
        reconnecting: bool,
    },
    Jam {
        owner: bool,
        link: Option<String>,
    },
}

/// The panel, filling the sidebar it sits in.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let _ = (cx, layer);
    let theme = &ui.theme;
    let header = row()
        .key("members-header")
        .center_y()
        .h(theme.metrics.header.round())
        .px(rems(theme, 0.5))
        .border_sides([0., 0., 1., 0.], theme.border.color())
        .child(ui::spoken_eyebrow(
            "members-heading",
            t!("members-title"),
            theme,
        ));
    let root = col()
        .key("members")
        .role("complementary")
        .label(t!("members-title"))
        .w_full()
        .h_full()
        .min_h(0.)
        .min_w(0.)
        .child(header);

    let Some((gathering, people)) = gathering(ui) else {
        return root.child(
            ui::spoken_vacant("members-empty", t!("members-empty"), theme).grow(1.),
        );
    };

    let count = people.len();
    let status = match &gathering {
        Gathering::Session {
            reconnecting: true,
            ..
        } => t!("devices-session-reconnecting"),
        Gathering::Session { .. } => t!("devices-session-people", count = count),
        Gathering::Jam { .. } => t!("devices-jam-people", count = count),
    }
    .into_owned();
    // whoever started the session or the Jam may send anyone else out of it
    let removable = matches!(
        gathering,
        Gathering::Session { hosting: true, .. } | Gathering::Jam { owner: true, .. }
    );
    let in_session = matches!(gathering, Gathering::Session { .. });
    let actions = actions(ui, gathering);

    let rows = people.iter().enumerate().map(|(index, member)| {
        // the listener reads as "You", the way the list in a Jam names them
        let shown = match member.you {
            true => t!("devices-jam-you").into_owned(),
            false => jam::name(member).to_owned(),
        };
        let mut line = row()
            .key(format!("member-{}", member.id))
            .role("listitem")
            .label(shown.clone())
            .position_in_set(index + 1, count)
            .center_y()
            .gap(rems(theme, 0.5))
            .h(theme.metrics.row)
            .px(rems(theme, 0.5))
            .radius(theme.radius)
            .hover(theme.secondary_hover.color())
            .child(jam::face(ui, member, theme.metrics.control_small))
            .child(
                text(shown)
                    .font(ui::font(theme, Text::Body, Weight::NORMAL))
                    .color(theme.foreground.color())
                    .truncate()
                    .grow(1.)
                    .shrink()
                    .min_w(0.)
                    .a11y_hidden(),
            );
        if member.host {
            line = line.child(jam::badge(
                format!("member-host-{index}"),
                t!("devices-jam-host"),
                theme,
            ));
        }
        if removable && !member.host && !member.you {
            let id = member.id.clone();
            line = line.child(
                Button::new(format!("member-remove-{}", member.id))
                    .small()
                    .name(t!("devices-jam-remove-member", name = jam::name(member)))
                    .label(t!("devices-jam-remove"))
                    .on_press(act(move |ui: &mut Ui| match in_session {
                        true => ui.store.listening.kick(&id),
                        false => ui.store.devices.kick(&id),
                    }))
                    .build(theme),
            );
        }
        line
    });

    root.child(
        col()
            .key("members-summary")
            .items_center()
            .gap(rems(theme, 0.5))
            .padding(rems(theme, 0.75))
            .border_sides([0., 0., 1., 0.], theme.border.color())
            .child(
                col()
                    .key("members-faces")
                    .role("text")
                    .label(status.clone())
                    .items_center()
                    .gap(rems(theme, 0.25))
                    .max_w_full()
                    .child(jam::faces(ui, &people, FACES, theme.metrics.control, 0.))
                    .child(
                        text(status)
                            .font(ui::font(theme, Text::Small, Weight::NORMAL))
                            .color(theme.muted_foreground.color())
                            .truncate()
                            .a11y_hidden(),
                    ),
            )
            .child(
                row()
                    .flex_wrap()
                    .center()
                    .gap(rems(theme, 0.25))
                    .children(actions),
            ),
    )
    .child(
        col()
            .key("members-list")
            .role("list")
            .label(t!("members-title"))
            .grow(1.)
            .min_h(0.)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(col().w_full().padding(rems(theme, 0.5)).children(rows)),
    )
}

/// The people in the Sonora Session or Jam the listener is in, the host first, or none.
pub(crate) fn gathered(ui: &Ui) -> Vec<JamMember> {
    if let Some(room) = ui.store.listening.room() {
        return session::people(room);
    }
    let devices = &ui.store.devices;
    devices
        .jam()
        .filter(|_| devices.available())
        .map(|jam| jam.members.clone())
        .unwrap_or_default()
}

/// The gathering the listener is in and the people in it, the host first.
fn gathering(ui: &Ui) -> Option<(Gathering, Vec<JamMember>)> {
    let listening = &ui.store.listening;
    if let Some(room) = listening.room() {
        return Some((
            Gathering::Session {
                code: room.code.clone(),
                hosting: room.hosting(),
                reconnecting: listening.reconnecting(),
            },
            session::people(room),
        ));
    }
    let devices = &ui.store.devices;
    let jam = devices.jam().filter(|_| devices.available())?;
    Some((
        Gathering::Jam {
            owner: jam.owner,
            link: jam.link.clone(),
        },
        jam.members.clone(),
    ))
}

/// The buttons under the faces: the session's code to copy and the way out, or the Jam's.
fn actions(ui: &Ui, gathering: Gathering) -> Vec<El> {
    let theme = &ui.theme;
    let button = |key: &str, path: &str, label: String| {
        Button::new(key).ghost().small().icon(path).label(label)
    };
    match gathering {
        Gathering::Session { code, hosting, .. } => vec![
            // the code itself stays off screen, where anyone looking could read it
            button("members-code", "icons/copy.svg", t!("devices-session-copy").into())
                .on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
                    session::copy_code(ui, tx, &code)
                }))
                .build(theme),
            button(
                "members-leave",
                "icons/log-out.svg",
                match hosting {
                    true => t!("devices-session-end"),
                    false => t!("devices-session-leave"),
                }
                .into(),
            )
            .on_press(act(|ui: &mut Ui| ui.store.listening.leave(&mut ui.store.cx)))
            .build(theme),
        ],
        Gathering::Jam { owner, link } => {
            let mut actions = Vec::new();
            if let Some(link) = link {
                actions.push(
                    button("members-jam-copy", "icons/copy.svg", t!("devices-jam-copy").into())
                        .on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
                            jam::copy_link(ui, tx, &link)
                        }))
                        .build(theme),
                );
            }
            actions.push(
                button(
                    "members-jam-leave",
                    "icons/log-out.svg",
                    match owner {
                        true => t!("devices-jam-end"),
                        false => t!("devices-jam-leave"),
                    }
                    .into(),
                )
                .on_press(act(|ui: &mut Ui| ui.store.devices.leave(&mut ui.store.cx)))
                .build(theme),
            );
            actions
        }
    }
}
