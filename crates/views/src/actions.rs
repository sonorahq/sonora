//! What each keyboard shortcut does.

use input::Action;
use router::Destination;
use rune::Tx;

use crate::Ui;

pub(crate) fn run(ui: &mut Ui, action: Action, tx: &mut Tx) {
    match action {
        Action::Quit => {
            ui.store.outputs.on_quit();
            tx.quit();
        }
        Action::Refresh => crate::chrome::title_bar::refresh(ui),
        Action::TogglePlayback => ui.store.toggle_play(),
        Action::SongPrevious => ui.store.previous_track(),
        Action::SongNext => ui.store.next_track(),
        Action::NavigateBack => ui.back(),
        Action::NavigateForward => ui.forward(),
        Action::OpenFilter => crate::shells::workspace::open_filter(ui, tx),
        Action::OpenSearch => ui.go(Destination::Search),
        Action::OpenSettings => ui.open_settings(),
        Action::ToggleFullscreen => match ui.nav.current() {
            Destination::Fullscreen => ui.back(),
            _ => ui.go(Destination::Fullscreen),
        },
        Action::ToggleWindowFullscreen => tx.toggle_full_screen(),
        Action::CloseWindow => tx.close_window(),
        Action::MinimizeWindow => tx.minimize(),
        Action::IncreaseFontSize => step_font(ui, 1.),
        Action::DecreaseFontSize => step_font(ui, -1.),
        Action::ResetFontSize => step_font(ui, 0.),
        Action::Undo | Action::Redo => {}
        Action::Dismiss => crate::shells::workspace::dismiss(ui),
    }
    ui.settle();
}

/// Steps the text size by `delta` points, or back to the default for zero.
fn step_font(ui: &mut Ui, delta: f32) {
    let size = match delta == 0. {
        true => ui::Theme::dark().font_size,
        false => ui.store.settings.font_size() + delta,
    };
    let size = size.clamp(ui::MIN_FONT, ui::MAX_FONT);
    ui.store.settings.set_font_size(size, &mut ui.store.cx);
}
