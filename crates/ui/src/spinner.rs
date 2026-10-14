use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, App, ElementId, Pixels, StyleRefinement, Svg, Transformation,
    Window, percentage, px, svg,
};

use crate::motion;
use crate::theme::ActiveTheme as _;

/// How long one turn takes.
const TURN: Duration = Duration::from_millis(900);
/// The spinner's size, the same as a button's icon.
const SIZE: Pixels = px(16.);

/// A turning arrow for work that has started and not answered yet. It holds still when the
/// system asks for less motion.
#[derive(IntoElement)]
pub struct Spinner {
    id: ElementId,
    base: Svg,
}

impl Spinner {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            base: svg(),
        }
    }
}

impl Styled for Spinner {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, mut base } = self;
        let overrides = std::mem::take(base.style());

        let mut icon = base
            .path(icons::path("icons/loader-circle.svg"))
            .size(SIZE)
            .flex_none()
            .text_color(cx.theme().muted_foreground);
        icon.style().refine(&overrides);
        match motion::animates(cx) {
            true => icon
                .with_animation(id, Animation::new(TURN).repeat(), |icon, turned| {
                    icon.with_transformation(Transformation::rotate(percentage(turned)))
                })
                .into_any_element(),
            false => icon.into_any_element(),
        }
    }
}
