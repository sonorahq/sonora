use crate::theme::ActiveTheme as _;
use gpui::prelude::*;
use gpui::{App, Svg, Window, rems, svg};

/// The sparkle's side, in rems, the same as the explicit badge so both sit on the title line.
const SIDE: f32 = 1.;

/// The sparkle ahead of a title, marking a song the provider mixed in as a recommendation, as
/// Spotify's smart shuffle does. It follows the active icon pack.
#[derive(IntoElement, Default)]
pub struct Sparkle;

impl Sparkle {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for Sparkle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let sparkle: Svg = svg()
            .path(icons::path("icons/sparkles.svg"))
            .flex_none()
            .size(rems(SIDE))
            .text_color(cx.theme().primary);
        sparkle
    }
}
