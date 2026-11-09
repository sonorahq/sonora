//! The Members panel: who is in the Sonora Session or Spotify Jam the listener is in, with the
//! session's code or the Jam's link to copy and the way out of it. `state::Listening` and
//! `state::Devices` answer everything, and this only draws it.

use i18n::t;
use music::connect::JamMember;
use rune::{Cx, El, ScrollSpec, TextStyle, Transition, Tx, col, row, text};
use state::SideTab;
use ui::{Button, Springs, Text, Theme, Weight};

use crate::chrome::{jam, session};
use crate::{Ui, act};

/// How many faces the summary shows before it counts the rest.
const FACES: usize = 6;
/// The badge beside the host's name: its side padding and its height's padding.
const BADGE_PAD: [f32; 2] = [8., 2.];

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

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let header = super::header(SideTab::Members, None, theme);
    let panel = col().w_full().h_full().min_h(0.).min_w(0.).child(header);
    let Some((gathering, people)) = gathering(ui) else {
        return panel.child(col().grow(1.).w_full().center().child(ui::spoken_vacant(
            "members-empty",
            t!("members-empty").to_string(),
            theme,
        )));
    };

    let count = people.len();
    let status = match &gathering {
        Gathering::Session {
            reconnecting: true, ..
        } => t!("devices-session-reconnecting"),
        Gathering::Session { .. } => t!("devices-session-people", count = count),
        Gathering::Jam { .. } => t!("devices-jam-people", count = count),
    }
    .to_string();
    // whoever started the session or the Jam may send anyone else out of it
    let removable = matches!(
        gathering,
        Gathering::Session { hosting: true, .. } | Gathering::Jam { owner: true, .. }
    );
    let session = matches!(gathering, Gathering::Session { .. });

    let summary = col()
        .key("members-summary")
        .w_full()
        .center()
        .gap(theme.metrics.pad)
        .padding(theme.metrics.pad * 1.5)
        .border_sides([0., 0., 1., 0.], theme.border.color())
        .child(
            col()
                .key("members-faces")
                .role("text")
                .label(status.clone())
                .items_center()
                .gap(theme.metrics.pad / 2.)
                .child(jam::faces(ui, &people, FACES, theme.metrics.control, 0.))
                .child(
                    text(status)
                        .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                        .color(theme.muted_foreground.color())
                        .truncate()
                        .a11y_hidden(),
                ),
        )
        .child(
            row()
                .flex_wrap()
                .center()
                .gap(theme.metrics.pad / 2.)
                .children(actions(gathering, theme)),
        );

    let rows = people.iter().enumerate().map(|(index, member)| {
        // the listener reads as "You", the way the list in a Jam names them
        let shown = match member.you {
            true => t!("devices-jam-you").to_string(),
            false => jam::name(member).to_owned(),
        };
        let mut line = row()
            .key(format!("member-{}", member.id))
            .role("listitem")
            .label(shown.clone())
            .position_in_set(index + 1, count)
            .w_full()
            .center_y()
            .gap(theme.metrics.pad)
            .h(theme.metrics.row)
            .px(theme.metrics.pad)
            .radius(theme.radius)
            .hover(theme.secondary_hover.color())
            .transition(Transition::opacity().combined(Transition::scale(0.96)))
            .child(jam::face(ui, member, theme.metrics.control_small))
            .child(
                text(shown)
                    .font(TextStyle::new(theme.text(Text::Body), Weight::NORMAL))
                    .color(theme.foreground.color())
                    .truncate()
                    .grow(1.)
                    .min_w(0.)
                    .a11y_hidden(),
            );
        if member.host {
            line = line.child(badge(
                format!("member-host-{index}"),
                t!("devices-jam-host").to_string(),
                theme,
            ));
        }
        if removable && !member.host && !member.you {
            let id = member.id.clone();
            line = line.child(
                Button::new(format!("member-remove-{}", member.id))
                    .small()
                    .name(t!("devices-jam-remove-member", name = jam::name(member)).to_string())
                    .label(t!("devices-jam-remove").to_string())
                    .on_press(act(move |ui: &mut Ui| match session {
                        true => ui.store.listening.kick(&id),
                        false => ui.store.devices.kick(&id),
                    }))
                    .build(theme),
            );
        }
        line
    });

    panel.child(summary).child(
        col()
            .key("members-list")
            .role("list")
            .label(t!("members-title").to_string())
            .grow(1.)
            .min_h(0.)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .w_full()
                    .padding(theme.metrics.pad)
                    // the last row scrolls clear of a translucent player bar
                    .pb(theme.metrics.pad + crate::chrome::player_bar::bottom(ui, cx))
                    .motion(Springs::BASE)
                    .children(rows),
            ),
    )
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
fn actions(gathering: Gathering, theme: &Theme) -> Vec<El> {
    let button = |key: &str, glyph: &str, label: String| {
        Button::new(key.to_owned())
            .ghost()
            .small()
            .icon(glyph.to_owned())
            .label(label)
    };
    match gathering {
        Gathering::Session { code, hosting, .. } => vec![
            // the code itself stays off screen, where anyone looking could read it
            button(
                "members-code",
                "icons/copy.svg",
                t!("devices-session-copy").to_string(),
            )
            .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
                session::copy_code(ui, tx, &code);
                ui.settle();
            })
            .build(theme),
            button(
                "members-leave",
                "icons/log-out.svg",
                match hosting {
                    true => t!("devices-session-end"),
                    false => t!("devices-session-leave"),
                }
                .to_string(),
            )
            .on_press(act(|ui: &mut Ui| {
                let state::Store { listening, cx, .. } = &mut ui.store;
                listening.leave(cx);
            }))
            .build(theme),
        ],
        Gathering::Jam { owner, link } => {
            let mut actions = Vec::new();
            if let Some(link) = link {
                actions.push(
                    button(
                        "members-jam-copy",
                        "icons/copy.svg",
                        t!("devices-jam-copy").to_string(),
                    )
                    .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
                        jam::copy_link(ui, tx, &link);
                        ui.settle();
                    })
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
                    .to_string(),
                )
                .on_press(act(|ui: &mut Ui| {
                    let state::Store { devices, cx, .. } = &mut ui.store;
                    devices.leave(cx);
                }))
                .build(theme),
            );
            actions
        }
    }
}

/// A small tag beside a member's name, which screen readers reach under `key`.
fn badge(key: String, label: String, theme: &Theme) -> El {
    text(label.clone())
        .key(key)
        .role("text")
        .label(label)
        .font(TextStyle::new(theme.text(Text::Tiny), Weight::NORMAL))
        .color(theme.muted_foreground.color())
        .px(BADGE_PAD[0])
        .py(BADGE_PAD[1])
        .radius(theme.text(Text::Tiny))
        .fill(theme.muted.color())
}
