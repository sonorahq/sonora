//! The Jam as the app shows it: the question before a shared link is joined, the code a phone
//! scans to join the one the listener started, and the list of people in the one the listener is
//! in. All of them are `state::Devices`' to answer; this only draws them.

use gpui::prelude::*;
use std::hash::{DefaultHasher, Hash, Hasher};

use gpui::{
    AnyElement, App, ClipboardItem, Context, Div, ElementId, Entity, FocusHandle, Hsla, Pixels,
    Point, Render, SharedString, Window, div, px, rgb, svg,
};
use i18n::t;
use music::connect::{Jam, JamMember};
use router::{Destination, navigate};
use state::{Devices, Outcome, Sonora, Toasts};
use ui::{
    ActiveTheme as _, Avatar, Button, Dismiss, FORM_CONTEXT, Input, Menu, MenuItem, Modal, Picker,
    Popup, Qr, Submit, Text, Theme, snapped,
};

/// The side of the Jam's QR code. It does not scale with the theme, since a code has to stay a
/// size a camera can read.
const CODE: Pixels = px(220.);
/// How much of a face the next one covers while the faces rest, and the gap between them once
/// they spread apart, both as parts of a face's width.
const FACE_OVERLAP: f32 = 0.5;
const FACE_GAP: f32 = 0.15;
/// What a Jam link carries somewhere in it: a short link, a shared one, or the address Spotify
/// itself gives a session. Text without one of these is not looked up.
const JAM_LINKS: [&str; 3] = ["spotify.link/", "socialsession/", "sessions/join/"];
/// How wide the join dialog stands, in covers, so a whole link fits beside the paste button.
const JOIN_WIDTH: f32 = 3.2;
/// The ring round each face that parts it from the one it covers.
const RING: Pixels = px(1.);
/// The colours a person without a picture is drawn in, picked by their id the way Spotify picks
/// one. A person keeps their colour whatever the theme is, so these are literals and never theme
/// tokens.
const FACE_COLORS: [u32; 8] = [
    0x509bf5, 0xaf2896, 0xe8115b, 0xff6437, 0xf59b23, 0x27856a, 0x8d67ab, 0x1e90a0,
];
/// How much of a placeholder the person glyph fills, and how far darker than the colour it is.
const GLYPH: f32 = 0.6;
const GLYPH_SHADE: f32 = 0.45;

/// Asks whether to join the Jam a pasted link names. It shows for as long as `Devices` holds an
/// invite, wherever the listener is in the app, and either answer hands the invite back.
pub(crate) struct JamInvite {
    devices: Entity<Devices>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    shown: bool,
}

impl JamInvite {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        Self {
            devices,
            focus: cx.focus_handle(),
            restore: None,
            shown: false,
        }
    }

    fn decline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.devices.update(cx, |devices, cx| devices.dismiss(cx));
    }

    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.devices.update(cx, |devices, cx| devices.accept(cx));
    }

    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.shown = false;
    }
}

impl Render for JamInvite {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let asked = self.devices.read(cx).invite().map(|invite| {
            (
                host_name(&invite.jam).map(str::to_owned),
                invite.jam.members.len(),
            )
        });
        let Some((host, count)) = asked else {
            if self.shown {
                self.release(window, cx);
            }
            return div().into_any_element();
        };
        if !self.shown {
            self.restore = window.focused(cx);
            window.focus(&self.focus, cx);
            self.shown = true;
        }

        let title = match host.as_deref() {
            Some(host) => t!("devices-jam-invite-title", host = host),
            None => t!("devices-jam-invite-title-anonymous"),
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.decline(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.join(window, cx);
            }))
            .child(
                Modal::new("jam-invite", title)
                    .when(count > 0, |modal| {
                        modal.detail(t!("devices-jam-invite-detail", count = count))
                    })
                    .action(
                        Button::new("decline-jam")
                            .ghost()
                            .label(t!("common-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.decline(window, cx))),
                    )
                    .action(
                        Button::new("join-jam")
                            .primary()
                            .label(t!("devices-jam-invite-join"))
                            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.decline(window, cx))),
            )
            .into_any_element()
    }
}

/// Takes the link of a Jam to join, pasted or typed. It shows for as long as `Devices` says the
/// dialog is open. Joining hands the link to `Devices` to look up, and the invite it names then
/// asks to confirm, with who is in the Jam.
pub(crate) struct JamJoin {
    devices: Entity<Devices>,
    link: Entity<Input>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    shown: bool,
}

impl JamJoin {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        let link = cx.new(|cx| Input::new("devices-jam-link-hint", cx));
        cx.observe(&link, |_, _, cx| cx.notify()).detach();
        Self {
            devices,
            link,
            focus: cx.focus_handle(),
            restore: None,
            shown: false,
        }
    }

    /// The link in the field, when it reads as one.
    fn wanted(&self, cx: &App) -> Option<String> {
        let text = self.link.read(cx).text().trim();
        JAM_LINKS
            .iter()
            .any(|marker| text.contains(marker))
            .then(|| text.to_owned())
    }

    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self.wanted(cx) else {
            return;
        };
        self.devices
            .update(cx, |devices, cx| devices.peek(&link, cx));
        self.close(window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.devices
            .update(cx, |devices, cx| devices.show_join(false, cx));
    }

    /// Hands focus back and empties the field for the next time the dialog opens.
    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.link.update(cx, |input, cx| input.set_text("", cx));
        self.shown = false;
    }
}

impl Render for JamJoin {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.devices.read(cx).join_shown() {
            if self.shown {
                self.release(window, cx);
            }
            return div().into_any_element();
        }
        if !self.shown {
            self.restore = window.focused(cx);
            self.link.update(cx, |input, cx| input.focus(window, cx));
            self.shown = true;
        }

        let theme = *cx.theme();
        let ready = self.wanted(cx).is_some();
        let link = self.link.clone();

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.join(window, cx);
            }))
            .child(
                Modal::new("jam-join", t!("devices-jam-join-title"))
                    .w(theme.metrics.cover * JOIN_WIDTH)
                    .detail(t!("devices-jam-join-detail"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(self.link.clone()))
                            .child(
                                // as tall as the field beside it, so the two read as one row
                                Button::new("jam-paste")
                                    .secondary()
                                    .h(theme.metrics.field)
                                    .icon("icons/clipboard-paste.svg")
                                    .label(t!("devices-jam-paste"))
                                    .on_click(move |_, window, cx| {
                                        let Some(text) =
                                            cx.read_from_clipboard().and_then(|item| item.text())
                                        else {
                                            return;
                                        };
                                        link.update(cx, |input, cx| {
                                            input.set_text(text.trim().to_owned(), cx);
                                            input.focus(window, cx);
                                        });
                                    }),
                            ),
                    )
                    .action(
                        Button::new("cancel-jam-join")
                            .ghost()
                            .label(t!("common-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .action(
                        Button::new("confirm-jam-join")
                            .primary()
                            .disabled(!ready)
                            .label(t!("devices-jam-invite-join"))
                            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

/// The link to the Jam the listener started as a QR code, for a phone to scan. It shows for as
/// long as `Devices` says the code is on, wherever the listener is in the app, and closing it
/// hands that back.
pub(crate) struct JamCode {
    devices: Entity<Devices>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    shown: bool,
}

impl JamCode {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        Self {
            devices,
            focus: cx.focus_handle(),
            restore: None,
            shown: false,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.devices
            .update(cx, |devices, cx| devices.show_code(false, cx));
    }

    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.shown = false;
    }
}

impl Render for JamCode {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let devices = self.devices.read(cx);
        let shown = devices
            .jam()
            .filter(|_| devices.code_shown())
            .and_then(|jam| {
                let link = jam.link.clone()?;
                Some((host_name(jam).map(str::to_owned), link))
            });
        let Some((host, link)) = shown else {
            if self.shown {
                self.release(window, cx);
            }
            return div().into_any_element();
        };
        if !self.shown {
            self.restore = window.focused(cx);
            window.focus(&self.focus, cx);
            self.shown = true;
        }

        let theme = *cx.theme();
        let title = match host.as_deref() {
            Some(host) => t!("devices-jam-code-title", host = host),
            None => t!("devices-jam-code-title-anonymous"),
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .child(
                Modal::new("jam-code", title)
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .child(Qr::new(link.clone()).size(CODE).rounded(theme.radius * 2.)),
                    )
                    .action(
                        Button::new("copy-jam-code")
                            .ghost()
                            .label(t!("devices-jam-copy"))
                            .on_click(move |_, _, cx| copy_link(&link, cx)),
                    )
                    .action(
                        Button::new("close-jam-code")
                            .primary()
                            .label(t!("devices-jam-code-close"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

/// The list of people in the Jam the listener is in, opened as a popup over the player bar. The
/// owner gets a button to remove anyone else. It closes by itself once the Jam is over.
pub(crate) struct JamPeople {
    devices: Entity<Devices>,
    /// Where the list ends while it is open.
    at: Option<Point<Pixels>>,
}

impl JamPeople {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&devices, |this, devices, cx| {
            if devices.read(cx).jam().is_none() {
                this.at = None;
            }
            cx.notify();
        })
        .detach();
        Self { devices, at: None }
    }

    /// Opens the list upward from `at`, a point on the strip or menu that asked for it.
    pub fn show(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.at = Some(at);
        cx.notify();
    }
}

impl Render for JamPeople {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(at) = self.at else {
            return div().absolute().into_any_element();
        };
        let Some(jam) = self.devices.read(cx).jam() else {
            return div().absolute().into_any_element();
        };

        let theme = *cx.theme();
        let size = theme.metrics.control_small;
        let devices = self.devices.clone();
        let rows = jam.members.iter().enumerate().map(|(index, member)| {
            let removable = jam.owner && !member.host && !member.you;
            let id = member.id.clone();
            let devices = devices.clone();
            // a profile is opened by the account name, which the id stands in for when missing
            let profile = [&member.username, &member.id]
                .into_iter()
                .find(|id| !id.is_empty())
                .map(|id| SharedString::from(id.clone()));

            MenuItem::new(("jam-member", index), name(member).to_owned())
                .leading(face(member, size))
                .trailing(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(member.host, |this| {
                            this.child(badge(("jam-host", index), t!("devices-jam-host"), theme))
                        })
                        .when(member.you, |this| {
                            this.child(badge(("jam-you", index), t!("devices-jam-you"), theme))
                        })
                        .when(removable, |this| {
                            this.child(
                                Button::new(("jam-remove", index))
                                    .small()
                                    .name(t!("devices-jam-remove-member", name = name(member)))
                                    .label(t!("devices-jam-remove"))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        devices.read(cx).kick(&id)
                                    }),
                            )
                        }),
                )
                .when_none(&profile, |this| this.disabled())
                .when_some(profile, |this, profile| {
                    this.on_click(cx.listener(move |this, _, _, cx| {
                        this.at = None;
                        cx.notify();
                        navigate(Destination::User(profile.clone()), cx);
                    }))
                })
        });

        Popup::new(at, Menu::new("jam-people").w(Picker::WIDE).items(rows))
            .above()
            .on_close(cx.listener(|this, _, _, cx| {
                this.at = None;
                cx.notify();
            }))
            .into_any_element()
    }
}

/// Copies a Jam's link and says so.
pub(crate) fn copy_link(link: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(link.to_owned()));
    Toasts::show(Outcome::Done, "toast-link-copied", cx);
}

/// The name of the person who owns `jam`, when the provider gave one.
pub(crate) fn host_name(jam: &Jam) -> Option<&str> {
    jam.members
        .iter()
        .find(|member| member.host)
        .map(|host| host.name.as_str())
        .filter(|name| !name.is_empty())
}

/// What to call `member`: their name, or the id the provider knows them by when it gave none.
fn name(member: &JamMember) -> &str {
    match member.name.is_empty() {
        true => &member.id,
        false => &member.name,
    }
}

/// A member's round picture at `size`, or while they have none a person glyph on the colour
/// their id picks, so two people without pictures still tell apart.
pub(crate) fn face(member: &JamMember, size: Pixels) -> AnyElement {
    if let Some(image) = &member.image {
        return Avatar::new(Some(image.clone()))
            .size(size)
            .into_any_element();
    }
    let color = color_of(&member.id);
    let glyph = Hsla {
        l: color.l * GLYPH_SHADE,
        ..color
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded_full()
        .bg(color)
        .child(
            svg()
                .path(icons::path("icons/user.svg"))
                .size(size * GLYPH)
                .text_color(glyph),
        )
        .into_any_element()
}

/// The placeholder colour for the person `id` names, the same one on every launch.
fn color_of(id: &str) -> Hsla {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let at = (hasher.finish() % FACE_COLORS.len() as u64) as usize;
    rgb(FACE_COLORS[at]).into()
}

/// The first `limit` members' faces in a row at `size`, with how many more there are after them.
/// At `spread` 0 each face half covers the one before it, and at 1 they stand apart. The faces are
/// placed by hand inside a box as wide as they reach, since flex layout does not count a negative
/// margin into its width and would let the next element cover them. They stand from the box's
/// left edge, which stays put while they spread, and step by one snapped distance, so no face
/// rounds to a different pixel than its neighbours from frame to frame.
pub(crate) fn faces(
    members: &[JamMember],
    limit: usize,
    size: Pixels,
    spread: f32,
    window: &Window,
    theme: Theme,
) -> Div {
    let shown = members.len().min(limit);
    let more = members.len().saturating_sub(limit);
    // every length is a whole number of device pixels, so the layout rounds nothing unevenly
    let size = snapped(size, window);
    let ring = snapped(RING, window);
    let outer = size + ring * 2.;
    let pitch = snapped(
        outer + size * (FACE_GAP * spread - FACE_OVERLAP * (1. - spread)),
        window,
    );
    let last = shown.saturating_sub(1);
    let width = outer + pitch * last as f32;
    // a ring in the strip's colour keeps a face apart from the one it covers, and fades once
    // they spread and no longer overlap
    let ringed = |at: usize, face: AnyElement| {
        div()
            .absolute()
            .top_0()
            .left(pitch * at as f32)
            .size(outer)
            .rounded_full()
            .border(ring)
            .border_color(theme.popover.opacity(1. - spread))
            .child(face)
    };

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(size * FACE_GAP)
        .child(
            div().relative().flex_none().w(width).h(outer).children(
                members
                    .iter()
                    .take(limit)
                    .enumerate()
                    .map(|(at, member)| ringed(at, face(member, size))),
            ),
        )
        .when(more > 0, |this| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(size)
                    .min_w(size)
                    .px_1()
                    .rounded_full()
                    .bg(theme.secondary)
                    .text_size(theme.text(Text::Tiny))
                    .text_color(theme.muted_foreground)
                    .child(t!("devices-jam-more", count = more)),
            )
        })
}

/// A small tag beside a member's name, which screen readers reach under `id`.
fn badge(id: impl Into<ElementId>, label: SharedString, theme: Theme) -> impl IntoElement {
    ui::spoken(id, label)
        .flex_none()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(theme.muted)
        .text_size(theme.text(Text::Tiny))
        .text_color(theme.muted_foreground)
}
