use gpui::prelude::*;
use gpui::{AnyElement, App, Context, div};
use i18n::t;
use state::Sonora;
use ui::{Button, Picker, Switch};

use super::{Group, SettingsView, Slot};

impl SettingsView {
    /// The Sessions group: whether the player bar shows the Sessions button, and which server
    /// Sonora Sessions go through. The address row stands only while the official server is off.
    pub(super) fn sessions_groups(&self, cx: &App) -> Vec<Group> {
        let mut slots = vec![Slot::SessionsButton, Slot::SessionsOfficial];
        if !self.settings.read(cx).sessions_official() {
            slots.push(Slot::SessionsServer);
        }
        vec![
            Group::new(
                "settings-group-sessions",
                "icons/users-round-collaboration.svg",
            )
            .rows(slots),
        ]
    }

    pub(super) fn sessions_button_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).sessions_button();
        self.row(
            t!("settings-sessions-button"),
            Switch::new("sessions-button", on)
                .name(t!("settings-sessions-button"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_sessions_button(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn sessions_official_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).sessions_official();
        self.row(
            t!("settings-sessions-official"),
            Switch::new("sessions-official", on)
                .name(t!("settings-sessions-official"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_sessions_official(!on, cx));
                    // turning the official server off is asking for an address, so the field
                    // that takes it gets the keys
                    if on {
                        this.sessions_server
                            .update(cx, |input, cx| input.focus(window, cx));
                    }
                }))
                .into_any_element(),
        )
    }

    /// The address of the listener's own server and the button that saves it. Saving checks the
    /// server first and keeps it only once it answers, and a toast says how it went. The field is
    /// compact and the button small, so the two stand the same height. On a narrow page the field
    /// narrows and the label stays whole.
    pub(super) fn sessions_server_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let saving = Sonora::global(cx).listening.read(cx).saving();
        let field = self.sessions_server.clone();

        self.yielding_row(
            t!("settings-sessions-server"),
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(Picker::WIDE)
                        .child(self.sessions_server.clone()),
                )
                .child(
                    Button::new("sessions-server-save")
                        .flex_none()
                        .secondary()
                        .small()
                        .label(t!("settings-sessions-save"))
                        .disabled(saving)
                        .on_click(move |_, _, cx| {
                            let typed = field.read(cx).text().to_owned();
                            Sonora::global(cx)
                                .listening
                                .clone()
                                .update(cx, |listening, cx| listening.save_server(&typed, cx));
                        }),
                )
                .into_any_element(),
        )
    }
}
