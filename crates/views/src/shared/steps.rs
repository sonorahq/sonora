use gpui::prelude::*;
use gpui::{App, FontWeight, IntoElement, Pixels, RenderOnce, SharedString, Window, div, px};
use ui::{ActiveTheme as _, Text};

const BADGE: Pixels = px(28.);

#[derive(IntoElement)]
pub(crate) struct Steps {
    items: Vec<SharedString>,
}

pub(crate) fn steps(items: impl IntoIterator<Item = impl Into<SharedString>>) -> Steps {
    Steps {
        items: items.into_iter().map(Into::into).collect(),
    }
}

impl RenderOnce for Steps {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .children(self.items.into_iter().enumerate().map(|(i, text)| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .min_w_0()
                    .rounded(theme.radius)
                    .p_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_none()
                            .size(BADGE)
                            .rounded_full()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.secondary)
                            .text_color(theme.foreground)
                            .text_size(theme.text(Text::Small))
                            .font_weight(FontWeight::SEMIBOLD)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child((i + 1).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(text),
                    )
            }))
    }
}
