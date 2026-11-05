use crate::theme::ActiveTheme as _;
use gpui::prelude::*;
use gpui::{App, Div, IntoElement, RenderOnce, StyleRefinement, Window, div, rems, svg};

/// The badge's side, in rems.
const SIDE: f32 = 1.;

/// The square "E" a track marked explicit carries. The letter is drawn rather than set in type,
/// so it sits in the exact middle of the square whatever font and scale are in use, an equal
/// gap above and below it.
#[derive(IntoElement)]
pub struct ExplicitBadge {
    base: Div,
}

impl ExplicitBadge {
    pub fn new() -> Self {
        Self { base: div() }
    }
}

impl Default for ExplicitBadge {
    fn default() -> Self {
        Self::new()
    }
}

impl Styled for ExplicitBadge {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for ExplicitBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { mut base } = self;
        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());

        let mut badge = base
            .size(rems(SIDE))
            .flex_none()
            .bg(theme.muted)
            .rounded_xs()
            .child(
                svg()
                    .path(icons::path("icons/explicit.svg"))
                    .size(rems(SIDE))
                    .text_color(theme.muted_foreground),
            );
        badge.style().refine(&overrides);
        badge
    }
}
