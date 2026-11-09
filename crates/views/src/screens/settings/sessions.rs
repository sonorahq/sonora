//! The Sessions group of the Playback section: whether the player bar shows the Sessions
//! button, and which server Sonora Sessions go through.

use i18n::t;
use rune::{Cx, El, TextEdit, Tx, row};
use ui::{Button, Input, Picker};

use super::{Row, group, switch};
use crate::{Ui, act};

/// The field that takes the address of the listener's own server.
#[derive(Default)]
pub struct State {
    pub edit: TextEdit,
    /// Whether the field has been filled from the saved address yet.
    seeded: bool,
}

/// The group. The address row stands only while the official server is off. The Playback page
/// lays it out among its own groups.
pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut rows = vec![
        switch(
            theme,
            "sessions-button",
            t!("settings-sessions-button"),
            settings.sessions_button(),
            |ui, on| ui.store.settings.set_sessions_button(on, &mut ui.store.cx),
        ),
        switch(
            theme,
            "sessions-official",
            t!("settings-sessions-official"),
            settings.sessions_official(),
            |ui, on| {
                ui.store
                    .settings
                    .set_sessions_official(on, &mut ui.store.cx);
                // turning the official server off is asking for an address, so the field that
                // takes it gets the keys
                if !on {
                    seed(ui);
                    ui.screens.settings.sessions_server.edit.focused = true;
                }
            },
        ),
    ];
    if !settings.sessions_official() {
        rows.push(server(ui));
    }
    group(
        theme,
        Some((
            "settings-group-sessions",
            "icons/users-round-collaboration.svg",
        )),
        None,
        rows,
    )
}

/// Fills the field from the saved address the first time it shows.
fn seed(ui: &mut Ui) {
    let state = &mut ui.screens.settings.sessions_server;
    if !state.seeded {
        state.edit = TextEdit::new(ui.store.settings.sessions_server());
        state.seeded = true;
    }
}

/// The address of the listener's own server and the button that saves it. Saving checks the
/// server first and keeps it only once it answers, and a toast says how it went. The field
/// wears a red border from a save that was turned down until one goes through. On a narrow
/// page the field narrows and the label stays whole.
fn server(ui: &Ui) -> El {
    let theme = &ui.theme;
    let listening = &ui.store.listening;
    let saving = listening.saving();
    let field = Input::new(
        "settings-sessions-server-field",
        &ui.screens.settings.sessions_server.edit,
        t!("settings-sessions-server-hint"),
        |ui: &mut Ui| &mut ui.screens.settings.sessions_server.edit,
    )
    .name(t!("settings-sessions-server"))
    .compact()
    .invalid(listening.rejected())
    .on_submit(|ui: &mut Ui, _: &mut Tx| save(ui))
    .build(theme);
    let control = row()
        .key("sessions-server")
        .grow(1.)
        .min_w(0.)
        .center_y()
        .justify_end()
        .gap(theme.metrics.pad)
        .child(
            row()
                .key("sessions-server-slot")
                .grow(1.)
                .shrink()
                .min_w(0.)
                .max_w(Picker::<Ui>::WIDE)
                .child(field),
        )
        .child(
            Button::new("sessions-server-save")
                .secondary()
                .small()
                .label(t!("settings-sessions-save"))
                .disabled(saving)
                .on_press(act(save))
                .build(theme),
        );
    Row::new("sessions-server", t!("settings-sessions-server"), control)
        .yielding()
        .build(theme)
        .on_layout(|ui: &mut Ui, _: [f32; 4]| seed(ui))
}

fn save(ui: &mut Ui) {
    let typed = ui.screens.settings.sessions_server.edit.text.clone();
    ui.store.listening.save_server(&typed, &mut ui.store.cx);
}
