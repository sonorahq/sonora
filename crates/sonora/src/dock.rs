//! The macOS Dock: its icon, and the right-click menu on it with transport plus shuffle and
//! repeat, mirroring the tray. Hiding the Dock icon while the window is closed to the tray is
//! `AppHandle::set_dock_icon`, in `window`.

/// Sets the Dock icon from the embedded family, including launches outside an app bundle. Rune
/// applies it once AppKit finishes launching.
#[cfg(target_os = "macos")]
pub fn install_icon() {
    if let Err(error) = rune::set_app_icon(crate::assets::MAC_ICON) {
        log::warn!("dock: cannot set the app icon: {error}");
    }
}

/// The Dock menu's items, which `actions::publish` hands over with the app menu's.
#[cfg(target_os = "macos")]
pub fn items(shown: &crate::tray::Shown) -> Vec<rune::MenuItem<views::Ui>> {
    use crate::tray::{Event, run};
    use rune::MenuItem;

    vec![
        MenuItem::new(
            shown.toggle.clone(),
            |ui: &mut views::Ui, tx: &mut rune::Tx| run(ui, Event::Toggle, tx),
        )
        .id("dock.toggle")
        .dock(),
        MenuItem::new(
            shown.previous.clone(),
            |ui: &mut views::Ui, tx: &mut rune::Tx| run(ui, Event::Previous, tx),
        )
        .id("dock.previous")
        .dock(),
        MenuItem::new(
            shown.next.clone(),
            |ui: &mut views::Ui, tx: &mut rune::Tx| run(ui, Event::Next, tx),
        )
        .id("dock.next")
        .dock(),
        MenuItem::separator().dock(),
        MenuItem::new(
            shown.shuffle.clone(),
            |ui: &mut views::Ui, tx: &mut rune::Tx| run(ui, Event::Shuffle, tx),
        )
        .id("dock.shuffle")
        .checked(shown.shuffle_on)
        .dock(),
        MenuItem::new(
            shown.repeat.clone(),
            |ui: &mut views::Ui, tx: &mut rune::Tx| run(ui, Event::Repeat, tx),
        )
        .id("dock.repeat")
        .checked(shown.repeat_on)
        .dock(),
    ]
}
