//! Sonora's screens and window chrome on Rune. [`view`] builds the whole window from [`Ui`];
//! every handler changes `Ui` and Rune runs the view again.

mod actions;
mod app;
mod chrome;
mod screens;
mod shared;
mod shells;

pub use app::{Lives, Ui, act, act_tx};

use router::Destination;
use rune::{Cx, El, KeyEvent, Tx, col};
use state::SessionState;
use ui::Room;

/// The whole window: the title bar over the workspace (or the sign-in page, or the fullscreen
/// player), with every open overlay mounted above.
pub fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let theme = &ui.theme;
    if let Some(artwork) = &ui.artwork {
        artwork.set_scale(cx.env().scale);
    }
    cx.help_look(ui::help_look(theme));
    if let Some(still) = ui.store.settings.stillness().still() {
        cx.reduced_motion(still);
    }
    let mut layer = Vec::new();
    // An account that could not be reached is still an account, so nothing about a lost
    // network puts the sign-in page up: the workspace stays, on what the library kept and on
    // the local files.
    let signed_out = match ui.store.session.state() {
        SessionState::SignedOut | SessionState::Failed(_) => true,
        SessionState::Restoring | SessionState::SignedIn(_) | SessionState::Offline(_) => false,
        SessionState::Authorizing(_) => ui.chrome.signing_in,
    };
    let base = match (ui.nav.current(), signed_out) {
        (Destination::Fullscreen, _) => shells::fullscreen::view(ui, cx, &mut layer),
        (_, true) => col()
            .key("window")
            .w_full()
            .h_full()
            .fill(theme.background.color())
            .color(theme.foreground.color())
            .child(chrome::title_bar::view(ui, cx, &mut layer))
            .child(screens::login::view(ui, cx, &mut layer)),
        (_, false) => col()
            .key("window")
            .w_full()
            .h_full()
            .fill(theme.background.color())
            .color(theme.foreground.color())
            .child(chrome::title_bar::view(ui, cx, &mut layer))
            .child(shells::workspace::view(ui, cx, &mut layer)),
    };
    ui::root(base, layer)
}

/// Feeds live values before a frame. True when something the view reads changed outside a
/// handler, so the view runs again.
pub fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    ui.poll(tx)
}

/// Shortcuts that act on the app, whatever has focus.
pub fn on_key(ui: &mut Ui, event: &KeyEvent, tx: &mut Tx) -> bool {
    ui.key(event, tx)
}

/// Runs a shortcut's action as its key would, for the host's native menus.
pub fn run_action(ui: &mut Ui, action: input::Action, tx: &mut Tx) {
    actions::run(ui, action, tx);
}

/// The width class of the content area between the sidebars, re-running the view only when it
/// crosses a step of the ladder.
pub(crate) fn content_room<S>(cx: &Cx<S>) -> Room {
    cx.layout_with("content", |rect| Room::of(rect[2]))
        .unwrap_or(Room::Roomy)
}

/// The width class of the whole window.
pub(crate) fn window_room<S>(cx: &Cx<S>) -> Room {
    cx.layout_with("window", |rect| Room::of(rect[2]))
        .unwrap_or(Room::Roomy)
}
