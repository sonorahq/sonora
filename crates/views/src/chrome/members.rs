//! The Members panel of the right sidebar: who is in the Sonora Session or Spotify Jam the
//! listener is in, with the session's code and the way out of it. `state::Listening` and
//! `state::Devices` answer everything, and this only draws it.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{Context, Entity, Render, SharedString, Window, div};
use i18n::t;
use music::connect::JamMember;
use state::{Devices, Listening, Sonora};
use ui::{ActiveTheme as _, Button, Text, eyebrow, snapped, vacant};

use crate::chrome::{jam, session};

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

pub(crate) struct Members {
    listening: Entity<Listening>,
    devices: Entity<Devices>,
}

impl Members {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let listening = Sonora::global(cx).listening.clone();
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&listening, |_, _, cx| cx.notify()).detach();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        Self { listening, devices }
    }

    /// The gathering the listener is in and the people in it, the host first.
    fn gathering(&self, cx: &gpui::App) -> Option<(Gathering, Vec<JamMember>)> {
        let listening = self.listening.read(cx);
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
        let devices = self.devices.read(cx);
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
    fn actions(&self, gathering: Gathering) -> Vec<Button> {
        match gathering {
            Gathering::Session { code, hosting, .. } => {
                let listening = self.listening.clone();
                vec![
                    Button::new("members-code")
                        .ghost()
                        .small()
                        .icon("icons/copy.svg")
                        // the code itself stays off screen, where anyone looking could read it
                        .label(t!("devices-session-copy"))
                        .on_click(move |_, _, cx| session::copy_code(&code, cx)),
                    Button::new("members-leave")
                        .ghost()
                        .small()
                        .icon("icons/log-out.svg")
                        .label(match hosting {
                            true => t!("devices-session-end"),
                            false => t!("devices-session-leave"),
                        })
                        .on_click(move |_, _, cx| {
                            listening.update(cx, |listening, cx| listening.leave(cx))
                        }),
                ]
            }
            Gathering::Jam { owner, link } => {
                let devices = self.devices.clone();
                let mut actions = Vec::new();
                if let Some(link) = link {
                    actions.push(
                        Button::new("members-jam-copy")
                            .ghost()
                            .small()
                            .icon("icons/copy.svg")
                            .label(t!("devices-jam-copy"))
                            .on_click(move |_, _, cx| jam::copy_link(&link, cx)),
                    );
                }
                actions.push(
                    Button::new("members-jam-leave")
                        .ghost()
                        .small()
                        .icon("icons/log-out.svg")
                        .label(match owner {
                            true => t!("devices-jam-end"),
                            false => t!("devices-jam-leave"),
                        })
                        .on_click(move |_, _, cx| {
                            devices.update(cx, |devices, cx| devices.leave(cx))
                        }),
                );
                actions
            }
        }
    }
}

impl Render for Members {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .h(snapped(theme.metrics.header, window))
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .child(eyebrow(t!("members-title"), cx));
        let root = div()
            .id("members")
            .role(Role::Complementary)
            .aria_label(t!("members-title"))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(header);

        let Some((gathering, people)) = self.gathering(cx) else {
            return root.child(
                vacant(t!("members-empty"), cx)
                    .id("members-empty")
                    .role(Role::Label)
                    .aria_value(t!("members-empty"))
                    .flex_1(),
            );
        };

        let count = people.len();
        let status = match &gathering {
            Gathering::Session {
                reconnecting: true, ..
            } => t!("devices-session-reconnecting"),
            Gathering::Session { .. } => t!("devices-session-people", count = count),
            Gathering::Jam { .. } => t!("devices-jam-people", count = count),
        };
        // whoever started the session or the Jam may send anyone else out of it
        let removable = matches!(
            gathering,
            Gathering::Session { hosting: true, .. } | Gathering::Jam { owner: true, .. }
        );
        let session = matches!(gathering, Gathering::Session { .. });
        let actions = self.actions(gathering);

        let devices = self.devices.clone();
        let listening = self.listening.clone();
        let rows = people.iter().enumerate().map(|(index, member)| {
            let id = member.id.clone();
            let devices = devices.clone();
            let listening = listening.clone();
            div()
                .id(SharedString::from(format!("member-{}", member.id)))
                .role(Role::ListItem)
                .aria_label(SharedString::from(jam::name(member).to_owned()))
                .aria_position_in_set(index + 1)
                .aria_size_of_set(count)
                .flex()
                .items_center()
                .gap_2()
                .h(theme.metrics.row)
                .px_2()
                .rounded(theme.radius)
                .hover(|this| this.bg(theme.secondary_hover))
                .child(jam::face(member, theme.metrics.control_small))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(jam::name(member).to_owned()),
                )
                .when(member.host, |this| {
                    this.child(jam::badge(
                        ("member-host", index),
                        t!("devices-jam-host"),
                        theme,
                    ))
                })
                .when(member.you, |this| {
                    this.child(jam::badge(
                        ("member-you", index),
                        t!("devices-jam-you"),
                        theme,
                    ))
                })
                .when(removable && !member.host && !member.you, |this| {
                    this.child(
                        Button::new(("member-remove", index))
                            .small()
                            .name(t!("devices-jam-remove-member", name = jam::name(member)))
                            .label(t!("devices-jam-remove"))
                            .on_click(move |_, _, cx| match session {
                                true => listening.read(cx).kick(&id),
                                false => devices.read(cx).kick(&id),
                            }),
                    )
                })
        });

        root.child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .id("members-faces")
                        .role(Role::Label)
                        .aria_value(status.clone())
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(jam::faces(
                            &people,
                            FACES,
                            theme.metrics.control,
                            0.,
                            window,
                            theme,
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground)
                                .child(status),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .children(actions),
                ),
        )
        .child(
            div()
                .id("members-list")
                .role(Role::List)
                .aria_label(t!("members-title"))
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_2()
                .children(rows),
        )
    }
}

/// The people in the Sonora Session or Jam the listener is in, the host first, or none.
pub(crate) fn gathered(cx: &gpui::App) -> Vec<JamMember> {
    let sonora = Sonora::global(cx);
    if let Some(room) = sonora.listening.read(cx).room() {
        return session::people(room);
    }
    let devices = sonora.devices.read(cx);
    devices
        .jam()
        .filter(|_| devices.available())
        .map(|jam| jam.members.clone())
        .unwrap_or_default()
}
