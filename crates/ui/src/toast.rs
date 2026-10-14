use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Interactivity, Pixels, SharedString,
    Stateful, StyleRefinement, Window, div, px, svg,
};

use crate::button::Button;
use crate::metrics::Text;
use crate::motion::Rising as _;
use crate::theme::ActiveTheme as _;

const ICON: Pixels = px(16.);
const REACH: Pixels = px(420.);

type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Toast {
    base: Stateful<Div>,
    id: ElementId,
    message: SharedString,
    strong: Option<SharedString>,
    detail: Option<SharedString>,
    failed: bool,
    dismiss: Option<Press>,
    open: Option<Press>,
}

impl Toast {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, message: impl Into<SharedString>) -> Self {
        let id = id.into();

        Self {
            base: div().id(id.clone()),
            id,
            message: message.into(),
            strong: None,
            detail: None,
            failed: false,
            dismiss: None,
            open: None,
        }
    }

    pub fn failed(mut self) -> Self {
        self.failed = true;
        self
    }

    pub fn strong(mut self, name: impl Into<SharedString>) -> Self {
        self.strong = Some(name.into());
        self
    }

    /// A muted line below the message, wrapping rather than truncating, such as why it failed.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn on_dismiss(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.dismiss = Some(Box::new(handler));
        self
    }

    pub fn on_open(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.open = Some(Box::new(handler));
        self
    }
}

impl Styled for Toast {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Toast {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Toast {}

impl RenderOnce for Toast {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            id,
            message,
            strong,
            detail,
            failed,
            dismiss,
            open,
        } = self;

        let theme = *cx.theme();
        let (tint, icon) = match failed {
            true => (theme.danger, "icons/circle-alert.svg"),
            false => (theme.primary, "icons/circle-check.svg"),
        };
        let overrides = std::mem::take(base.style());

        let mut toast = base
            .occlude()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .max_w(REACH)
            .py(theme.metrics.pad)
            .pl(theme.metrics.pad * 2)
            .pr(theme.metrics.pad)
            .rounded(theme.radius)
            .border_1()
            .shadow_md()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_size(theme.text(Text::Small))
            .text_color(theme.foreground)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        svg()
                            .path(icons::path(icon))
                            .size(ICON)
                            .flex_none()
                            .text_color(tint),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(with_name_emphasised(&id, message, strong, open))
                            .when_some(detail, |text, detail| {
                                text.child(div().text_color(theme.muted_foreground).child(detail))
                            }),
                    ),
            )
            .children(dismiss.map(|dismiss| {
                Button::new("dismiss-toast")
                    .ghost()
                    .small()
                    .icon("icons/x.svg")
                    .tooltip("common-dismiss")
                    .on_click(move |event, window, cx| dismiss(event, window, cx))
            }));
        toast.style().refine(&overrides);
        toast.rising((id, "rise"))
    }
}

fn with_name_emphasised(
    id: &ElementId,
    message: SharedString,
    strong: Option<SharedString>,
    open: Option<Press>,
) -> Div {
    let split = strong
        .as_ref()
        .and_then(|name| message.find(name.as_ref()).map(|at| (at, name)));

    let Some((at, name)) = split else {
        return div().min_w_0().truncate().child(message);
    };

    div()
        .flex()
        .min_w_0()
        .child(
            div()
                .flex_none()
                .child(SharedString::from(message[..at].to_owned())),
        )
        .child(name_span(id, name.clone(), open))
        .child(
            div()
                .flex_none()
                .child(SharedString::from(message[at + name.len()..].to_owned())),
        )
}

fn name_span(id: &ElementId, name: SharedString, open: Option<Press>) -> AnyElement {
    let span = div().min_w_0().truncate().font_weight(FontWeight::BOLD);

    let Some(open) = open else {
        return span.child(name).into_any_element();
    };

    span.id((id.clone(), "link"))
        .cursor_pointer()
        .hover(|style| style.underline())
        .on_click(move |event, window, cx| open(event, window, cx))
        .child(name)
        .into_any_element()
}
