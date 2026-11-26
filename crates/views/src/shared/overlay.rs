//! The dialogs, menus and cards the root and the workspace draw over everything else, which
//! show nothing nearly all the time. Their host renders on every frame anything below it
//! animates, so a closed one is embedded as a cached view and costs nothing on those frames.

use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, StyleRefinement};

/// A view drawn over a shell that draws nothing while it is closed.
pub(crate) trait Overlay: Render {
    /// Whether the overlay draws anything or still has a transition to play. It reads the same
    /// state the overlay renders from, so it turns true on the frame the overlay opens. While it
    /// is false the overlay's render must draw a bare `div()`.
    fn showing(&self, cx: &App) -> bool;
}

/// Embeds `overlay` in its host's render. A closed overlay replays its empty frame until it
/// notifies, which it does when it opens, and is laid out as the bare `div()` it draws. An open
/// one renders with its host as any child does, so what it reads without observing stays current.
pub(crate) fn overlaid<V: Overlay>(overlay: &Entity<V>, cx: &App) -> AnyElement {
    match overlay.read(cx).showing(cx) {
        true => overlay.clone().into_any_element(),
        false => overlay
            .clone()
            .cached(StyleRefinement::default())
            .into_any_element(),
    }
}
