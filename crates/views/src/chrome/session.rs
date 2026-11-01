//! Sonora Session as the app shows it: the dialog that takes a friend's code, the code of the
//! session the listener started, and the people in it as faces. All of it is `state::Listening`'s
//! to answer, and this only draws it.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, ClipboardItem, Context, Entity, FocusHandle, Render, SharedString, Window, div};
use i18n::t;
use music::connect::JamMember;
use state::{Listening, Outcome, Room, SESSION_CODE_LENGTH, Session, Sonora, Toasts};
use ui::{ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Modal, OtpInput, OtpSlots, Submit};

/// How many digits a code shows in each group, with a dash between the groups.
const GROUP: usize = 3;
/// How wide both session dialogs stand, in covers, so the code keeps a margin on either side.
const WIDTH: f32 = 3.;

/// Takes the code of a session to join, typed or pasted. It shows for as long as `Listening`
/// says the dialog is open, and joining hands the code to `Listening`.
pub(crate) struct SessionJoin {
    listening: Entity<Listening>,
    code: Entity<OtpInput>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    shown: bool,
}

impl SessionJoin {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let listening = Sonora::global(cx).listening.clone();
        cx.observe(&listening, |_, _, cx| cx.notify()).detach();
        let code = cx.new(|cx| {
            OtpInput::new(SESSION_CODE_LENGTH, "devices-session-code", cx).grouped(GROUP)
        });
        cx.observe(&code, |_, _, cx| cx.notify()).detach();
        Self {
            listening,
            code,
            focus: cx.focus_handle(),
            restore: None,
            shown: false,
        }
    }

    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = self.code.read(cx);
        if !code.complete() {
            return;
        }
        let code = code.value().to_owned();
        self.listening
            .update(cx, |listening, cx| listening.join(&code, cx));
        self.close(window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.listening
            .update(cx, |listening, cx| listening.show_join(false, cx));
    }

    /// Hands focus back and empties the code for the next time the dialog opens.
    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.code.update(cx, |code, cx| code.set_value("", cx));
        self.shown = false;
    }
}

impl Render for SessionJoin {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.listening.read(cx).join_shown() {
            if self.shown {
                self.release(window, cx);
            }
            return div().into_any_element();
        }
        if !self.shown {
            self.restore = window.focused(cx);
            self.code.update(cx, |code, cx| code.focus(window, cx));
            self.shown = true;
        }
        let ready = self.code.read(cx).complete();

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
                Modal::new("session-join", t!("devices-session-join-title"))
                    .w(cx.theme().metrics.cover * WIDTH)
                    .detail(t!("devices-session-join-detail"))
                    .child(div().flex().justify_center().child(self.code.clone()))
                    .action(
                        Button::new("cancel-session-join")
                            .ghost()
                            .label(t!("common-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .action(
                        Button::new("confirm-session-join")
                            .primary()
                            .disabled(!ready)
                            .label(t!("devices-session-join-confirm"))
                            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

/// The code of the session the listener is in, for reading out to a friend. It shows
/// for as long as `Listening` says the code is on, which it turns on by itself once a session
/// the listener started is up.
pub(crate) struct SessionCode {
    listening: Entity<Listening>,
    session: Entity<Session>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    shown: bool,
}

impl SessionCode {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let listening = Sonora::global(cx).listening.clone();
        let session = Sonora::global(cx).session.clone();
        cx.observe(&listening, |_, _, cx| cx.notify()).detach();
        Self {
            listening,
            session,
            focus: cx.focus_handle(),
            restore: None,
            shown: false,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release(window, cx);
        self.listening
            .update(cx, |listening, cx| listening.show_code(false, cx));
    }

    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        self.shown = false;
    }
}

impl Render for SessionCode {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let listening = self.listening.read(cx);
        let shown = listening
            .room()
            .filter(|_| listening.code_shown())
            .map(|room| (room.code.clone(), room.provider.clone()));
        let Some((code, provider)) = shown else {
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
        let provider = self
            .session
            .read(cx)
            .account(&provider)
            .map_or(provider, |(name, _)| name.to_owned());
        let spoken = spaced(&code);
        let copied = code.clone();

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
                Modal::new("session-code", t!("devices-session-code-title"))
                    .w(cx.theme().metrics.cover * WIDTH)
                    .detail(t!("devices-session-code-detail", provider = provider))
                    .child(
                        div().flex().justify_center().child(
                            // read out as the digits in their groups, not as one large number
                            div()
                                .id("session-code-digits")
                                .role(Role::Label)
                                .aria_value(spoken)
                                .child(OtpSlots::new(code).grouped(GROUP)),
                        ),
                    )
                    .action(
                        Button::new("copy-session-code")
                            .ghost()
                            .icon("icons/copy.svg")
                            .label(t!("devices-session-copy"))
                            .on_click(move |_, _, cx| copy_code(&copied, cx)),
                    )
                    .action(
                        Button::new("close-session-code")
                            .primary()
                            .label(t!("devices-session-code-close"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

/// Copies a session's code and says so.
pub(crate) fn copy_code(code: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(code.to_owned()));
    Toasts::show(Outcome::Done, "toast-session-code-copied", cx);
}

/// A code with a space between its groups, the way it is read out.
pub(crate) fn spaced(code: &str) -> SharedString {
    let digits: Vec<char> = code.chars().collect();
    digits
        .chunks(GROUP)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
        .into()
}

/// The people in `room` in the shape the Jam's faces are drawn from, the host first.
pub(crate) fn people(room: &Room) -> Vec<JamMember> {
    let mut people: Vec<JamMember> = room
        .members
        .iter()
        .map(|member| JamMember {
            id: member.id.clone(),
            username: String::new(),
            name: member.name.clone(),
            image: member.image.clone(),
            host: member.host,
            you: room.is_you(member),
        })
        .collect();
    people.sort_by_key(|member| !member.host);
    people
}
