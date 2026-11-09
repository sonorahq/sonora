//! The Jam as the app shows it: the question before a shared link is joined, the dialog that
//! takes a link to join, the code a phone scans to join the one the listener started, and the
//! faces of the people in a Jam or a Sonora Session. All of it is `state::Devices`' to answer,
//! and this only draws it.

use std::hash::{DefaultHasher, Hash, Hasher};

use i18n::t;
use music::connect::{Jam, JamMember};
use rune::{Cx, El, TextEdit, Tx, el, icon, row, text};
use state::{Outcome, Toasts};
use ui::{Avatar, Button, Hsla, Input, Modal, Qr, Text, Theme, Weight};

use crate::chrome::player_bar::rems;
use crate::{Ui, act, act_tx};

/// The side of the Jam's QR code. It does not scale with the theme, since a code has to stay a
/// size a camera can read.
const CODE: f32 = 220.;
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
const RING: f32 = 1.;
/// The colours a person without a picture is drawn in, picked by their id the way Spotify picks
/// one. A person keeps their colour whatever the theme is, so these are literals and never theme
/// tokens.
const FACE_COLORS: [u32; 8] = [
    0x509bf5, 0xaf2896, 0xe8115b, 0xff6437, 0xf59b23, 0x27856a, 0x8d67ab, 0x1e90a0,
];
/// How much of a placeholder the person glyph fills, and how far darker than the colour it is.
const GLYPH: f32 = 0.6;
const GLYPH_SHADE: f32 = 0.45;
/// How tall a placeholder's initial stands, as a part of the face, and its colour, white on
/// every face colour whatever the theme is, so a literal like `FACE_COLORS`.
const INITIAL: f32 = 0.45;
const INITIAL_COLOR: u32 = 0xffffff;

/// The Jam dialogs' own state: the link typed into the join dialog.
#[derive(Default)]
pub struct State {
    pub link: TextEdit,
}

/// Mounts whichever Jam dialog `Devices` asks for into `layer`: the invite to confirm, the
/// dialog that takes a link, or the code of the Jam the listener started.
pub(crate) fn mount(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) {
    let _ = cx;
    invite(ui, layer);
    join(ui, layer);
    code(ui, layer);
}

/// Asks whether to join the Jam a pasted link names. Either answer hands the invite back.
fn invite(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let Some(invite) = ui.store.devices.invite() else {
        return;
    };
    let count = invite.jam.members.len();
    let title = match host_name(&invite.jam) {
        Some(host) => t!("devices-jam-invite-title", host = host),
        None => t!("devices-jam-invite-title-anonymous"),
    };
    let mut modal = Modal::<Ui>::new("jam-invite", title)
        .action(
            Button::new("decline-jam")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(decline))
                .build(theme),
        )
        .action(
            Button::new("join-jam")
                .primary()
                .label(t!("devices-jam-invite-join"))
                .on_press(act(|ui: &mut Ui| {
                    ui.store.devices.accept(&mut ui.store.cx);
                }))
                .build(theme)
                .default_button(),
        )
        .on_dismiss(act(decline));
    if count > 0 {
        modal = modal.detail(t!("devices-jam-invite-detail", count = count));
    }
    modal.build(theme, layer);
}

fn decline(ui: &mut Ui) {
    ui.store.devices.dismiss(&mut ui.store.cx);
}

/// Takes the link of a Jam to join, pasted or typed. Joining hands the link to `Devices` to look
/// up, and the invite it names then asks to confirm, with who is in the Jam.
fn join(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    if !ui.store.devices.join_shown() {
        return;
    }
    let ready = wanted(&ui.chrome.jam.link).is_some();
    let field = Input::<Ui>::new(
        "jam-link",
        &ui.chrome.jam.link,
        t!("devices-jam-link-hint"),
        |ui: &mut Ui| &mut ui.chrome.jam.link,
    )
    .name(t!("devices-jam-join-title"))
    .autofocus()
    .on_submit(act_tx(|ui: &mut Ui, _: &mut Tx| confirm_join(ui)))
    .build(theme);
    let paste = Button::new("jam-paste")
        .secondary()
        .icon("icons/clipboard-paste.svg")
        .label(t!("devices-jam-paste"))
        .on_press_tx(|_: &mut Ui, tx: &mut Tx| {
            tx.read_clipboard(|ui: &mut Ui, content, tx: &mut Tx| {
                let pasted = single_line(content.text().trim());
                if pasted.is_empty() {
                    return;
                }
                let link = &mut ui.chrome.jam.link;
                link.replace_text(pasted);
                link.focused = true;
                tx.focus("jam-link", false);
            });
        })
        .build(theme)
        // as tall as the field beside it, so the two read as one row
        .h(theme.metrics.field);
    Modal::<Ui>::new("jam-join", t!("devices-jam-join-title"))
        .width(theme.metrics.cover * JOIN_WIDTH)
        .detail(t!("devices-jam-join-detail"))
        .child(
            row()
                .center_y()
                .gap(rems(theme, 0.5))
                .child(el().grow(1.).min_w(0.).child(field))
                .child(paste),
        )
        .action(
            Button::new("cancel-jam-join")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(close_join))
                .build(theme),
        )
        .action(
            Button::new("confirm-jam-join")
                .primary()
                .disabled(!ready)
                .label(t!("devices-jam-invite-join"))
                .on_press(act(confirm_join))
                .build(theme),
        )
        .on_dismiss(act(close_join))
        .build(theme, layer);
}

/// The link in the field, when it reads as one.
fn wanted(link: &TextEdit) -> Option<String> {
    let text = link.text.trim();
    JAM_LINKS
        .iter()
        .any(|marker| text.contains(marker))
        .then(|| text.to_owned())
}

fn confirm_join(ui: &mut Ui) {
    let Some(link) = wanted(&ui.chrome.jam.link) else {
        return;
    };
    ui.store.devices.peek(&link, &mut ui.store.cx);
    close_join(ui);
}

/// Closes the join dialog and empties the field for the next time it opens.
fn close_join(ui: &mut Ui) {
    ui.chrome.jam.link = TextEdit::default();
    ui.store.devices.show_join(false, &mut ui.store.cx);
}

/// The link to the Jam the listener started as a QR code, for a phone to scan.
fn code(ui: &Ui, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let devices = &ui.store.devices;
    let Some(jam) = devices.jam().filter(|_| devices.code_shown()) else {
        return;
    };
    let Some(link) = jam.link.clone() else {
        return;
    };
    let title = match host_name(jam) {
        Some(host) => t!("devices-jam-code-title", host = host),
        None => t!("devices-jam-code-title-anonymous"),
    };
    let copied = link.clone();
    Modal::<Ui>::new("jam-code", title.clone())
        .child(
            row().w_full().justify_center().child(
                Qr::new(link)
                    .size(CODE)
                    .rounded(theme.radius * 2.)
                    .name(title)
                    .build(),
            ),
        )
        .action(
            Button::new("copy-jam-code")
                .ghost()
                .label(t!("devices-jam-copy"))
                .on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
                    copy_link(ui, tx, &copied)
                }))
                .build(theme),
        )
        .action(
            Button::new("close-jam-code")
                .primary()
                .label(t!("devices-jam-code-close"))
                .on_press(act(close_code))
                .build(theme)
                .default_button(),
        )
        .on_dismiss(act(close_code))
        .build(theme, layer);
}

fn close_code(ui: &mut Ui) {
    ui.store.devices.show_code(false, &mut ui.store.cx);
}

/// Copies a Jam's link and says so.
pub(crate) fn copy_link(ui: &mut Ui, tx: &mut Tx, link: &str) {
    tx.copy(link.to_owned());
    Toasts::show(Outcome::Done, "toast-link-copied", &mut ui.store.cx);
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
pub(crate) fn name(member: &JamMember) -> &str {
    match member.name.is_empty() {
        true => &member.id,
        false => &member.name,
    }
}

/// A member's round picture at `size`, or while they have none the first letter of their name
/// on the colour their id picks, so two people without pictures still tell apart. A name with no
/// letter to show gets a person glyph instead.
pub(crate) fn face(ui: &Ui, member: &JamMember, size: f32) -> El {
    let theme = &ui.theme;
    if let Some(image) = &member.image {
        return Avatar::new(ui.artwork(), Some(image)).size(size).build(theme);
    }
    let color = color_of(&member.id);
    let disc = row()
        .size(size, size)
        .radius(size / 2.)
        .fill(color.color())
        .center()
        .a11y_hidden();
    match initial(name(member)) {
        Some(letter) => disc.child(
            text(letter)
                .font(rune::TextStyle::new(size * INITIAL, Weight::SEMIBOLD))
                .color(Hsla::from(ui::rgb(INITIAL_COLOR)).color()),
        ),
        None => disc.child(
            icon(icons::get("icons/user.svg"), size * GLYPH).color(
                Hsla {
                    l: color.l * GLYPH_SHADE,
                    ..color
                }
                .color(),
            ),
        ),
    }
}

/// The first `limit` members' faces in a row at `size`, with how many more there are after
/// them. At `spread` 0 each face half covers the one before it, and at 1 they stand apart. The
/// faces are placed by hand inside a box as wide as they reach, so the next element never covers
/// them, and stand from the box's left edge, which stays put while they spread.
pub(crate) fn faces(ui: &Ui, members: &[JamMember], limit: usize, size: f32, spread: f32) -> El {
    let theme = &ui.theme;
    let shown = members.len().min(limit);
    let more = members.len().saturating_sub(limit);
    let size = size.round();
    let outer = size + RING * 2.;
    let pitch = (outer + size * (FACE_GAP * spread - FACE_OVERLAP * (1. - spread))).round();
    let width = outer + pitch * shown.saturating_sub(1) as f32;
    // a ring in the strip's colour keeps a face apart from the one it covers, and fades once
    // they spread and no longer overlap
    let ring = theme.popover.opacity(1. - spread).color();
    let strip = el().size(width, outer).children(
        members.iter().take(limit).enumerate().map(|(at, member)| {
            row()
                .key(format!("face-{}", member.id))
                .absolute(pitch * at as f32, 0.)
                .size(outer, outer)
                .radius(outer / 2.)
                .border(RING, ring)
                .center()
                .child(face(ui, member, size))
        }),
    );
    let mut line = row()
        .center_y()
        .gap(size * FACE_GAP)
        .a11y_hidden()
        .child(strip);
    if more > 0 {
        line = line.child(more_label(
            t!("devices-jam-more", count = more).into_owned(),
            size,
            theme,
        ));
    }
    line
}

/// The pill after the faces that counts the people past them.
fn more_label(label: String, size: f32, theme: &Theme) -> El {
    row()
        .h(size)
        .min_w(size)
        .px(rems(theme, 0.25))
        .radius(size / 2.)
        .fill(theme.secondary.color())
        .center()
        .child(
            text(label)
                .font(ui::font(theme, Text::Tiny, Weight::NORMAL))
                .color(theme.muted_foreground.color()),
        )
}

/// A small tag beside a member's name, which screen readers reach under `key`.
pub(crate) fn badge(key: impl Into<String>, label: impl Into<String>, theme: &Theme) -> El {
    let label = label.into();
    row()
        .key(key)
        .role("text")
        .label(label.clone())
        .px(rems(theme, 0.5))
        .py(rems(theme, 0.125))
        .radius(9999.)
        .fill(theme.muted.color())
        .child(
            text(label)
                .font(ui::font(theme, Text::Tiny, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .a11y_hidden(),
        )
}

/// The capital of the first letter or digit in `name`, if it has one.
fn initial(name: &str) -> Option<String> {
    let first = name.chars().find(|c| c.is_alphanumeric())?;
    Some(first.to_uppercase().collect())
}

/// The placeholder colour for the person `id` names, the same one on every launch.
fn color_of(id: &str) -> Hsla {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let at = (hasher.finish() % FACE_COLORS.len() as u64) as usize;
    Hsla::from(ui::rgb(FACE_COLORS[at]))
}

/// `text` on one line: line breaks and tabs become spaces, as pasting into a one-line field does.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
