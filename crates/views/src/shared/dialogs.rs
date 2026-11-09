//! The overlays any screen can open: the context menu, the confirmation, the playlist and tag
//! editors, the artist's About dialog and Widevine's questions, plus the cover tint the theme
//! follows. Their state lives in [`Dialogs`] on `Ui`, and the workspace mounts them all with
//! [`view`] above everything else.

use rune::{Cx, El};

use crate::Ui;
use crate::shared::{about, adaptive, confirm, menus, playlist_editor, tag_editor, widevine};

/// What the window-wide overlays hold while they are open.
#[derive(Default)]
pub struct Dialogs {
    pub(crate) menu: menus::Menus,
    pub(crate) confirm: Option<confirm::Pending>,
    pub(crate) playlist: playlist_editor::Editor,
    pub(crate) tags: tag_editor::Editor,
    pub(crate) about: Option<about::About>,
    /// The cover palette the theme wears and the cover it came from.
    pub(crate) adaptive: adaptive::Adaptive,
}

impl Dialogs {
    /// Whether a dialog (not the context menu) is up.
    pub(crate) fn any(&self, ui: &Ui) -> bool {
        self.confirm.is_some()
            || self.playlist.shows()
            || self.about.is_some()
            || tag_editor::shows(ui)
            || widevine::shows(ui)
    }
}

/// Mounts the open context menu and dialogs into `layer`. The dialogs come after the menu, so
/// one opened from a menu row sits above it while the menu closes.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) {
    let _ = cx;
    menus::view(ui, layer);
    about::view(ui, layer);
    playlist_editor::view(ui, layer);
    tag_editor::view(ui, layer);
    confirm::view(ui, layer);
    widevine::view(ui, layer);
}

/// Closes the topmost overlay, for the window's Escape when no overlay has focus. True when one
/// was open.
pub(crate) fn dismiss(ui: &mut Ui) -> bool {
    let dialogs = &mut ui.dialogs;
    if dialogs.menu.shows() {
        dialogs.menu.close();
    } else if dialogs.confirm.is_some() {
        dialogs.confirm = None;
    } else if dialogs.playlist.shows() {
        dialogs.playlist.close();
    } else if dialogs.about.is_some() {
        dialogs.about = None;
    } else if tag_editor::shows(ui) {
        tag_editor::close(ui);
    } else {
        return false;
    }
    true
}

/// `n` times the theme's text size, the unit every spacing of the design system is set in.
pub(crate) fn rems(theme: &ui::Theme, n: f32) -> f32 {
    theme.font_size * n
}
