//! The Scrobbling page: one row per service plays are reported to, and the dialog that links
//! one with its key, token or server.

use i18n::t;
use music::scrobble::{Link, Secret};
use rune::{Cx, El, TextEdit, Transition, Tx, col, row};
use state::ScrobbleState;
use ui::{Button, Input, Modal, Text, Weight, font};

use super::{Layout, Row, group, toggle};
use crate::{Ui, act, act_tx};

/// How wide the link dialog stands.
const WIDTH: f32 = 560.;
/// How wide a service's status may run before it is cut, in covers.
const STATUS_WIDTH: f32 = 1.5;

/// What the Scrobbling page keeps between frames: the service whose link dialog is up and its
/// two fields.
#[derive(Default)]
pub struct State {
    /// The service whose link dialog is open, by id.
    prompt: Option<&'static str>,
    pub first: TextEdit,
    pub second: TextEdit,
    /// What each field asks for, as Fluent keys, and whether it holds a secret drawn as dots.
    first_field: (&'static str, bool),
    second_field: (&'static str, bool),
}

fn st(ui: &mut Ui) -> &mut State {
    &mut ui.screens.settings.scrobbling
}

/// One field of the link dialog: which hint it carries, what it starts with, and whether it
/// holds a secret.
struct Field {
    hint: &'static str,
    value: String,
    masked: bool,
}

/// The Scrobbling page: one row per service, in the order `music::scrobble` lists them.
pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>, _layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let count = ui.store.scrobbling.rows().len();
    if let Some(service) = ui.screens.settings.scrobbling.prompt {
        modal(ui, service, layer);
    }
    vec![group(
        theme,
        None,
        None,
        (0..count).map(|index| scrobble_row(ui, index)),
    )]
}

/// A service's name and where its link stands, which the row shows and a search reads.
pub(super) fn scrobble_meta(ui: &Ui, index: usize) -> Option<(String, String)> {
    let row = ui.store.scrobbling.rows().get(index)?;
    let service = row.id();
    let detail = match row.state() {
        ScrobbleState::Off => t!("settings-scrobble-off"),
        ScrobbleState::Linking => t!("settings-scrobble-waiting"),
        ScrobbleState::On(name) => match name.is_empty() {
            true => t!("settings-scrobble-on"),
            false => t!("settings-scrobble-as", name = name.to_string()),
        },
        ScrobbleState::Failed(key) => i18n::lookup(key, None),
    };
    Some((
        i18n::lookup(&format!("settings-{service}"), None).into_owned(),
        detail.into_owned(),
    ))
}

/// A service: its name, where its link stands unless the Connect button says so already, and
/// the controls for its state.
fn scrobble_row(ui: &Ui, index: usize) -> El {
    let theme = &ui.theme;
    let (title, detail) = scrobble_meta(ui, index).unwrap_or_default();
    let row_ = &ui.store.scrobbling.rows()[index];
    let service = row_.id();
    let link = row_.link();
    let enabled = row_.enabled();
    let status = (!matches!(row_.state(), ScrobbleState::Off)).then_some(detail);

    let action = match (row_.linked(), row_.linking()) {
        (true, _) => row()
            .center_y()
            .gap(theme.font_size * 0.5)
            .child(toggle(
                theme,
                &format!("scrobble-on-{service}"),
                title.clone(),
                enabled,
                false,
                move |ui, on| ui.store.set_scrobble_enabled(service, on),
            ))
            .child(
                Button::new(format!("scrobble-unlink-{service}"))
                    .label(t!("settings-scrobble-disconnect"))
                    .small()
                    .outline()
                    .on_press(act(move |ui: &mut Ui| {
                        ui.store.disconnect_scrobbler(service)
                    }))
                    .build(theme),
            ),
        (false, true) => Button::new(format!("scrobble-cancel-{service}"))
            .label(t!("common-cancel"))
            .small()
            .outline()
            .on_press(act(|ui: &mut Ui| ui.store.cancel_scrobble_link()))
            .build(theme),
        (false, false) => Button::new(format!("scrobble-link-{service}"))
            .label(t!("settings-scrobble-connect"))
            .small()
            .outline()
            .on_press(act(move |ui: &mut Ui| open(ui, service, link)))
            .build(theme),
    };
    let mut control = row().center_y().gap(theme.font_size * 0.75);
    if let Some(status) = status {
        control = control.child(
            ui::spoken(format!("scrobble-status-{service}"), status, theme)
                .max_w(theme.metrics.cover * STATUS_WIDTH)
                .truncate()
                .font(font(theme, Text::Label, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .transition(Transition::opacity()),
        );
    }
    control = control.child(action.transition(Transition::opacity()));
    Row::new(format!("scrobble-{service}"), title, control).build(theme)
}

/// Starts a link. A service with nothing to type goes straight to the browser; the rest put up
/// the dialog with whatever was stored last time already in the fields.
fn open(ui: &mut Ui, service: &'static str, link: Link) {
    let account = ui.store.settings.account(service);
    let (first, second) = match link {
        Link::Browser => return ui.store.connect_scrobbler(service, Secret::None),
        Link::Keys => (
            Field {
                hint: "settings-scrobble-key",
                value: account.key,
                masked: false,
            },
            Some(Field {
                hint: "settings-scrobble-secret",
                value: account.secret,
                masked: true,
            }),
        ),
        Link::Token => (
            Field {
                hint: "settings-scrobble-token",
                value: account.session,
                masked: true,
            },
            None,
        ),
        Link::Server => (
            Field {
                hint: "settings-scrobble-server",
                value: account.server,
                masked: false,
            },
            Some(Field {
                hint: "settings-scrobble-key",
                value: account.session,
                masked: true,
            }),
        ),
    };
    let state = st(ui);
    state.first.replace_text(first.value);
    state.first.focused = true;
    state.first_field = (first.hint, first.masked);
    match second {
        Some(second) => {
            state.second.replace_text(second.value);
            state.second_field = (second.hint, second.masked);
        }
        None => state.second.clear(),
    }
    state.prompt = Some(service);
}

/// Links `service` with what the dialog's fields hold.
fn submit(ui: &mut Ui, service: &'static str, link: Link) {
    let state = st(ui);
    let first = state.first.text.clone();
    let second = state.second.text.clone();
    let secret = match link {
        Link::Browser => Secret::None,
        Link::Keys => Secret::Keys {
            key: first,
            secret: second,
        },
        Link::Token => Secret::Token(first),
        Link::Server => Secret::Server {
            url: first,
            key: second,
        },
    };
    close(ui);
    ui.store.connect_scrobbler(service, secret);
}

fn close(ui: &mut Ui) {
    let state = st(ui);
    state.prompt = None;
    state.first.clear();
    state.second.clear();
}

/// The link dialog for `service`: its fields, the page where a key is asked for when the
/// service has one, and Connect.
fn modal(ui: &Ui, service: &'static str, layer: &mut Vec<El>) {
    let theme = &ui.theme;
    let state = &ui.screens.settings.scrobbling;
    let row_ = ui
        .store
        .scrobbling
        .rows()
        .iter()
        .find(|row| row.id() == service);
    let link = row_.map(|row| row.link()).unwrap_or(Link::Token);
    let signup = row_.and_then(|row| row.signup());
    let title = i18n::lookup(&format!("settings-{service}"), None).into_owned();
    let request = match link {
        Link::Token => "settings-scrobble-token-request",
        _ => "settings-scrobble-request",
    };
    let field = |key: &str,
                 edit: &TextEdit,
                 (hint, masked): (&str, bool),
                 access: fn(&mut Ui) -> &mut TextEdit| {
        Input::new(key, edit, i18n::lookup(hint, None), access)
            .masked(masked)
            .on_submit(act_tx(move |ui: &mut Ui, _: &mut Tx| {
                submit(ui, service, link)
            }))
            .build(theme)
    };
    let mut fields = col().w_full().gap(theme.font_size * 0.5).child(field(
        "settings-scrobble-first",
        &state.first,
        state.first_field,
        |ui| &mut st(ui).first,
    ));
    if link != Link::Token {
        fields = fields.child(field(
            "settings-scrobble-second",
            &state.second,
            state.second_field,
            |ui| &mut st(ui).second,
        ));
    }
    let mut dialog = Modal::<Ui>::new(
        "settings-scrobble-prompt",
        t!("settings-scrobble-title", service = title.as_str()),
    )
    .width(WIDTH)
    .detail(i18n::lookup(&format!("settings-{service}-detail"), None))
    .child(fields);
    if let Some(url) = signup {
        dialog = dialog.action(
            Button::new("settings-scrobble-request")
                .ghost()
                .label(i18n::lookup(request, None))
                .on_press_tx(move |_: &mut Ui, tx: &mut Tx| {
                    if let Err(error) = tx.open_url(url) {
                        log::warn!("settings: cannot open {url}: {error}");
                    }
                })
                .build(theme),
        );
    }
    dialog
        .action(
            Button::new("settings-scrobble-cancel")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(act(close))
                .build(theme),
        )
        .action(
            Button::new("settings-scrobble-submit")
                .primary()
                .label(t!("settings-scrobble-connect"))
                .on_press(act(move |ui: &mut Ui| submit(ui, service, link)))
                .build(theme),
        )
        .on_dismiss(act(close))
        .build(theme, layer);
}
