//! The code a device sign-in asks the listener to type on the provider's site.

use rune::{El, Tx, row};
use state::{Outcome, Toasts};
use ui::{Button, Text, Theme, Weight};

use crate::{Ui, act_tx};

/// The code a device sign-in asks the listener to type on the provider's site, centred in
/// whatever holds it, with a button that copies it and says so in a toast. The login screen
/// and the settings dialog both show it.
pub(crate) fn device_code(code: &str, theme: &Theme) -> El {
    let copied = code.to_owned();
    row()
        .w_full()
        .center()
        .gap(theme.font_size * 0.5)
        .child(
            ui::spoken("device-code", code, theme)
                .font(ui::font(theme, Text::Title, Weight::BOLD))
                .selectable(true),
        )
        .child(
            Button::new("copy-code")
                .icon("icons/copy.svg")
                .tooltip(i18n::t!("app-copy"))
                .ghost()
                .small()
                .on_press_tx(act_tx(move |ui: &mut Ui, tx: &mut Tx| {
                    tx.copy(copied.clone());
                    Toasts::show(Outcome::Done, "toast-code-copied", &mut ui.store.cx);
                }))
                .build(theme),
        )
}
