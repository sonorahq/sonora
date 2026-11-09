//! The macOS menu bar and Dock menu. Keyboard shortcuts are `views::on_key`'s on every platform;
//! these menus only show them and run the same actions when picked. Rune keeps AppKit's own app
//! menu items (About, Hide, Hide Others, Show All, Quit) and adds Edit and Window with
//! `App::standard_menus`, so Sonora adds Settings, Refresh and Sign Out to the app menu and
//! names Rune's items in the app's language. Other platforms draw no menu bar, so
//! [`publish`] does nothing there.

use crate::tray::Shown;

/// Names Rune's own menu items (Hide, Quit, Edit, Window, …) from Fluent, in whatever language
/// is active when AppKit asks.
pub fn install_labels() {
    #[cfg(target_os = "macos")]
    rune::set_labels(|label| {
        use i18n::t;
        use rune::Label;

        let text = match label {
            Label::Cut => t!("app-cut"),
            Label::Copy => t!("app-copy"),
            Label::Paste => t!("app-paste"),
            Label::SelectAll => t!("app-select-all"),
            Label::Menu("Edit") => t!("app-edit"),
            Label::Menu("Window") => t!("app-window"),
            Label::Minimize => t!("app-minimize"),
            Label::Close => t!("app-close-window"),
            Label::Hide(_) => t!("app-hide"),
            Label::HideOthers => t!("app-hide-others"),
            Label::ShowAll => t!("app-show-all"),
            Label::Quit(_) => t!("app-quit"),
            _ => return None,
        };
        Some(text.to_string())
    });
}

/// Hands the app menu's and the Dock menu's items to AppKit, with the Dock's labels and checks
/// from `shown`. The tray calls it whenever those change, a language switch included.
pub fn publish(shown: &Shown) {
    #[cfg(target_os = "macos")]
    {
        let mut items = app_items();
        items.extend(crate::dock::items(shown));
        if let Err(error) = crate::handle().set_menu(items) {
            log::warn!("menu: cannot set the menu bar: {error}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = shown;
}

/// Settings, Refresh and Sign Out, after the app menu's About item. Their shortcuts reach the
/// window first, so `views::on_key` runs them as it does everywhere and the menu only runs an
/// item picked with the pointer.
#[cfg(target_os = "macos")]
fn app_items() -> Vec<rune::MenuItem<views::Ui>> {
    use i18n::t;
    use rune::{MenuItem, Shortcut};

    let shortcut = |keys: &str| keys.parse::<Shortcut>().ok();
    let with = |item: MenuItem<views::Ui>, keys: &str| match shortcut(keys) {
        Some(keys) => item.key_equivalent(keys).keys_first(),
        None => item,
    };
    vec![
        with(
            MenuItem::new(
                t!("app-settings").to_string(),
                |ui: &mut views::Ui, tx: &mut rune::Tx| {
                    crate::window::show();
                    views::run_action(ui, input::Action::OpenSettings, tx);
                },
            )
            .id("app.settings"),
            "cmd+comma",
        ),
        MenuItem::separator(),
        with(
            MenuItem::new(
                t!("app-refresh").to_string(),
                |ui: &mut views::Ui, tx: &mut rune::Tx| {
                    views::run_action(ui, input::Action::Refresh, tx);
                },
            )
            .id("app.refresh"),
            "cmd+r",
        ),
        MenuItem::new(
            t!("app-sign-out").to_string(),
            |ui: &mut views::Ui, _: &mut rune::Tx| {
                ui.store.sign_out();
                ui.settle();
            },
        )
        .id("app.sign-out"),
        MenuItem::separator(),
    ]
}
