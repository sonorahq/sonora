use gpui::prelude::*;
use gpui::{App, ClipboardItem, FontWeight, SharedString, div};
use state::{Outcome, Toasts};
use ui::ActiveTheme as _;
use ui::{Button, Text, spoken};

/// The code a device sign-in asks the listener to type on the provider's site, centred in
/// whatever holds it, with a button that copies it and says so in a toast. The login screen
/// and the settings dialog both show it.
pub(crate) fn device_code(code: String, cx: &App) -> impl IntoElement {
    let theme = *cx.theme();
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .w_full()
        .child(
            spoken("device-code", SharedString::from(code.clone()))
                .text_size(theme.text(Text::Title))
                .font_weight(FontWeight::BOLD),
        )
        .child(
            Button::new("copy-code")
                .icon("icons/copy.svg")
                .tooltip("app-copy")
                .ghost()
                .small()
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                    Toasts::show(Outcome::Done, "toast-code-copied", cx);
                }),
        )
}
