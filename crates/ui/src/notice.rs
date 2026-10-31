use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Div, ElementId, FontWeight, Interactivity, Pixels, SharedString, StyleRefinement, Window,
    div, px, svg,
};

use crate::metrics::Text;
use crate::theme::ActiveTheme as _;

const ICON: Pixels = px(18.);
const REACH: Pixels = px(520.);

#[derive(IntoElement)]
pub struct Notice {
    base: Div,
    title: SharedString,
    message: SharedString,
    icon: SharedString,
    failed: bool,
    centered: bool,
    /// The id under which the notice reaches screen readers, which it does only once given one.
    spoken: Option<ElementId>,
}

impl Notice {
    pub fn new(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            title: title.into(),
            message: message.into(),
            icon: SharedString::from("icons/circle-alert.svg"),
            failed: false,
            centered: false,
            spoken: None,
        }
    }

    pub fn failed(mut self) -> Self {
        self.failed = true;
        self
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = path.into();
        self
    }

    /// Exposes the notice to screen readers as a group named by its title and described by its
    /// message, under the element id `id`. Without it both are drawn but never read.
    pub fn spoken(mut self, id: impl Into<ElementId>) -> Self {
        self.spoken = Some(id.into());
        self
    }
}

impl Styled for Notice {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Notice {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Notice {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            title,
            message,
            icon,
            failed,
            centered,
            spoken,
        } = self;

        let theme = *cx.theme();
        let tint = match failed {
            true => theme.danger,
            false => theme.muted_foreground,
        };
        let overrides = std::mem::take(base.style());

        let head = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                svg()
                    .path(icons::path(icon))
                    .size(ICON)
                    .flex_none()
                    .text_color(tint),
            )
            .child(
                div()
                    .text_size(theme.text(Text::Body))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child(title.clone()),
            );

        let mut notice = base
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .max_w(REACH)
            .p(theme.metrics.pad * 1.5)
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary.opacity(0.45))
            .text_size(theme.text(Text::Small))
            .when(centered, |this| this.items_center().text_center())
            .child(head)
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(message.clone()),
            );

        notice.style().refine(&overrides);
        match spoken {
            Some(id) => notice
                .id(id)
                .role(Role::Group)
                .aria_label(title)
                .aria_description(message)
                .into_any_element(),
            None => notice.into_any_element(),
        }
    }
}
