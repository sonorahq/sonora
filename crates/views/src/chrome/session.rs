//! Sonora Session as the app shows it: the dialog that takes a friend's code, the code of the
//! session the listener started, and the people in it in the shape the faces are drawn from.
//! All of it is `state::Listening`'s to answer, and this only draws it.

use i18n::t;
use music::connect::JamMember;
use rune::{Cx, El, TextEdit, Tx, row};
use state::{Outcome, Room, SESSION_CODE_LENGTH, Toasts};
use ui::{Button, Modal, OtpInput, OtpSlots};

use crate::{Ui, act, act_tx};

/// How many digits a code shows in each group, with a dash between the groups.
const GROUP: usize = 3;
/// How wide both session dialogs stand, in covers, so the code keeps a margin on either side.
const WIDTH: f32 = 3.;

/// The session dialogs' own state: the code typed into the join dialog.
#[derive(Default)]
pub struct State {
    pub code: TextEdit,
}

/// Mounts whichever session dialog `Listening` asks for into `layer`.
pub(crate) fn mount(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) {
    let _ = cx;
    join(ui, layer);
    code(ui, layer);
}

/// Takes the code of a session to join, typed or pasted, and hands it to `Listening`.
fn join(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    if !ui.store.listening.join_shown() {
        return;
    }
    let edit = &ui.chrome.session.code;
    let ready = complete(edit);
    let input = OtpInput::<Ui>::new(
        "session-code-input",
        edit,
        SESSION_CODE_LENGTH,
        t!("devices-session-code"),
        |ui: &mut Ui| &mut ui.chrome.session.code,
    )
    .grouped(GROUP)
    .build(theme);
    Modal::<Ui>::new("session-join", t!("devices-session-join-title"))
        .width(theme.metrics.cover * WIDTH)
        .detail(t!("devices-session-join-detail"))
        .child(row().w_full().justify_center().child(input))
        .action(
            Button::new("cancel-session-join")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(close_join))
                .build(theme),
        )
        .action(
            Button::new("confirm-session-join")
                .primary()
                .disabled(!ready)
                .label(t!("devices-session-join-confirm"))
                .on_press(act(confirm_join))
                .build(theme)
                .default_button(),
        )
        .on_dismiss(act(close_join))
        .build(theme, layer);
}

/// Opens the join dialog with its field focused, as the Sessions menu does.
pub(crate) fn open_join(ui: &mut Ui) {
    ui.chrome.session.code = TextEdit::default();
    ui.chrome.session.code.focused = true;
    ui.store.listening.show_join(true);
}

/// Whether the field holds a whole code.
fn complete(code: &TextEdit) -> bool {
    code.text.chars().filter(char::is_ascii_digit).count() == SESSION_CODE_LENGTH
}

fn confirm_join(ui: &mut Ui) {
    if !complete(&ui.chrome.session.code) {
        return;
    }
    let code = ui.chrome.session.code.text.clone();
    ui.store.join_listening(&code);
    close_join(ui);
}

/// Closes the join dialog and empties the code for the next time it opens.
fn close_join(ui: &mut Ui) {
    ui.chrome.session.code = TextEdit::default();
    ui.store.listening.show_join(false);
}

/// The code of the session the listener is in, for reading out to a friend.
fn code(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let listening = &ui.store.listening;
    let Some(room) = listening.room().filter(|_| listening.code_shown()) else {
        return;
    };
    let provider = ui
        .store
        .session
        .account(&room.provider)
        .map_or(room.provider.clone(), |(name, _)| name.to_owned());
    let copied = room.code.clone();
    // read out as the digits in their groups, not as one large number
    let digits = row()
        .key("session-code-digits")
        .role("text")
        .label(spaced(&room.code))
        .child(OtpSlots::new(room.code.clone()).grouped(GROUP).build(theme).a11y_hidden());
    Modal::<Ui>::new("session-code", t!("devices-session-code-title"))
        .width(theme.metrics.cover * WIDTH)
        .detail(t!("devices-session-code-detail", provider = provider))
        .child(row().w_full().justify_center().child(digits))
        .action(
            Button::new("copy-session-code")
                .ghost()
                .icon("icons/copy.svg")
                .label(t!("devices-session-copy"))
                .on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
                    copy_code(ui, tx, &copied)
                }))
                .build(theme),
        )
        .action(
            Button::new("close-session-code")
                .primary()
                .label(t!("devices-session-code-close"))
                .on_press(act(close_code))
                .build(theme)
                .default_button(),
        )
        .on_dismiss(act(close_code))
        .build(theme, layer);
}

fn close_code(ui: &mut Ui) {
    ui.store.listening.show_code(false);
}

/// Copies a session's code and says so.
pub(crate) fn copy_code(ui: &mut Ui, tx: &mut Tx, code: &str) {
    tx.copy(code.to_owned());
    Toasts::show(Outcome::Done, "toast-session-code-copied", &mut ui.store.cx);
}

/// A code with a space between its groups, the way it is read out.
pub(crate) fn spaced(code: &str) -> String {
    let digits: Vec<char> = code.chars().collect();
    digits
        .chunks(GROUP)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
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
