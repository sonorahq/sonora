use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, HighlightStyle, InteractiveText,
    Interactivity, Pixels, SharedString, Stateful, StyleRefinement, StyledText, UnderlineStyle,
    Window, div, px, svg,
};

use crate::arrow_ring::ArrowRing;
use crate::button::Button;
use crate::metrics::Text;
use crate::modal::corners;
use crate::spinner::Spinner;
use crate::theme::ActiveTheme as _;

const ICON: Pixels = px(16.);
/// The card's border width, which the height it reports has to count on both edges.
const BORDER: Pixels = px(1.);
/// How thick the line under a name that opens something is.
const LINK_UNDERLINE: Pixels = px(1.);

type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
/// Opens what a toast names.
type Open = Box<dyn Fn(&mut Window, &mut App) + 'static>;
type Measure = Box<dyn Fn(Pixels, &mut Window, &mut App) + 'static>;

/// One toast card: an optional icon, a message with an optional muted line under it, up to two
/// buttons and a close button. The card fills the width it is given, and a caller that sets its
/// height clips the contents, which is how the toast stack piles cards behind the front one. The
/// card does not block the pointer itself, so the container it sits in stays hovered over it and
/// has to occlude what is behind.
#[derive(IntoElement)]
pub struct Toast {
    base: Stateful<Div>,
    id: ElementId,
    message: SharedString,
    strong: Option<SharedString>,
    detail: Option<SharedString>,
    icon: Option<SharedString>,
    loading: bool,
    progress: Option<f32>,
    single_line: bool,
    action: Option<(SharedString, Press)>,
    action_icon: Option<SharedString>,
    cancel: Option<(SharedString, Press)>,
    shown: f32,
    dismiss: Option<Press>,
    open: Option<Open>,
    measure: Option<Measure>,
    inert: bool,
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
            icon: None,
            loading: false,
            progress: None,
            single_line: false,
            action: None,
            action_icon: None,
            cancel: None,
            shown: 1.,
            dismiss: None,
            open: None,
            measure: None,
            inert: false,
        }
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

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    /// Shows a turning spinner in place of the icon.
    pub fn loading(mut self) -> Self {
        self.loading = true;
        self
    }

    /// Shows a download ring filling `done` of the way round, from 0 to 1, in place of the icon,
    /// and how far that is as a percentage after the message.
    pub fn progress(mut self, done: f32) -> Self {
        self.progress = Some(done.clamp(0., 1.));
        self
    }

    /// Keeps the message to one line, cut short with an ellipsis, rather than wrapping it.
    pub fn single_line(mut self) -> Self {
        self.single_line = true;
        self
    }

    /// The outlined button beside the message, such as Undo.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), Box::new(handler)));
        self
    }

    /// An icon after the action's label, such as an arrow on a button that opens a page.
    pub fn action_icon(mut self, path: impl Into<SharedString>) -> Self {
        self.action_icon = Some(path.into());
        self
    }

    /// The quiet button before the action, such as Cancel.
    pub fn cancel(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.cancel = Some((label.into(), Box::new(handler)));
        self
    }

    /// Keeps the buttons drawn but unpressable, for a card on its way out, so nothing reflows and
    /// a press reaches the card under it.
    pub fn inert(mut self) -> Self {
        self.inert = true;
        self
    }

    /// How much of the contents show, from 0 to 1, while the card itself stays. A card stacked
    /// behind another hides its contents.
    pub fn contents(mut self, shown: f32) -> Self {
        self.shown = shown.clamp(0., 1.);
        self
    }

    pub fn on_dismiss(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.dismiss = Some(Box::new(handler));
        self
    }

    pub fn on_open(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.open = Some(Box::new(handler));
        self
    }

    /// Reports the height the card needs to show all of its contents, borders included, on every
    /// frame and whatever height the card is held to. The stack uses it to size the card and the
    /// ones piled behind it.
    pub fn on_measure(mut self, handler: impl Fn(Pixels, &mut Window, &mut App) + 'static) -> Self {
        self.measure = Some(Box::new(handler));
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
            icon,
            loading,
            progress,
            single_line,
            action,
            action_icon,
            cancel,
            shown,
            dismiss,
            open,
            measure,
            inert,
        } = self;

        let theme = *cx.theme();
        let pad = theme.metrics.pad * 2;
        let overrides = std::mem::take(base.style());
        // The card reads as one status named for its message, since the text
        // runs inside carry no nodes of their own.
        let name = message.clone();
        let about = detail.clone();

        let leading = match (progress, loading, icon) {
            (Some(done), _, _) => Some(
                ArrowRing::new((id.clone(), "ring"), Some(done))
                    .color(theme.primary)
                    .into_any_element(),
            ),
            (None, true, _) => Some(
                Spinner::new((id.clone(), "spinner"))
                    .size(ICON)
                    .text_color(theme.popover_foreground)
                    .into_any_element(),
            ),
            (None, false, Some(icon)) => Some(
                svg()
                    .path(icons::path(icon))
                    .size(ICON)
                    .flex_none()
                    .text_color(theme.popover_foreground)
                    .into_any_element(),
            ),
            (None, false, None) => None,
        };

        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(
                div()
                    .min_w_0()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.popover_foreground)
                    .when(single_line, |message| message.truncate())
                    .child(with_name_emphasised(&id, message, strong, open)),
            )
            .when_some(detail, |text, detail| {
                text.child(div().text_color(theme.muted_foreground).child(detail))
            });

        let body = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .w_full()
            .p(pad)
            .opacity(shown)
            .children(leading)
            .child(text)
            .children(progress.map(|done| {
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(i18n::t!(
                        "toast-percent",
                        percent = (done * 100.).floor() as usize
                    ))
            }))
            .when(cancel.is_some() || action.is_some(), |body| {
                body.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1p5()
                        .children(cancel.map(|(label, press)| {
                            Button::new((id.clone(), "cancel"))
                                .ghost()
                                .small()
                                .label(label)
                                .when(!inert, |button| {
                                    button
                                        .on_click(move |event, window, cx| press(event, window, cx))
                                })
                        }))
                        .children(action.map(|(label, press)| {
                            Button::new((id.clone(), "action"))
                                .outline()
                                .small()
                                .label(label)
                                .when_some(action_icon, Button::trailing)
                                .when(!inert, |button| {
                                    button
                                        .on_click(move |event, window, cx| press(event, window, cx))
                                })
                        })),
                )
            })
            .children(dismiss.map(|dismiss| {
                Button::new((id.clone(), "dismiss"))
                    .ghost()
                    .small()
                    .icon("icons/x.svg")
                    .tooltip("common-dismiss")
                    .when(!inert, |button| {
                        button.on_click(move |event, window, cx| dismiss(event, window, cx))
                    })
            }));

        let contents = match measure {
            Some(measure) => div()
                .w_full()
                .on_children_prepainted(move |bounds, window, cx| {
                    if let Some(body) = bounds.first() {
                        measure(body.size.height + BORDER * 2., window, cx);
                    }
                })
                .child(body),
            None => div().w_full().child(body),
        };

        let mut toast = base
            .role(gpui::accesskit::Role::Status)
            .aria_label(name)
            .when_some(about, |this, detail| this.aria_description(detail))
            .w_full()
            .overflow_hidden()
            .rounded(corners(theme.radius))
            .border(BORDER)
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_lg()
            .text_size(theme.text(Text::Label))
            .text_color(theme.popover_foreground)
            .child(contents);
        toast.style().refine(&overrides);
        toast
    }
}

/// The message as one run of text that wraps where it needs to, with the name in it bold, and
/// underlined and clickable when it opens something.
fn with_name_emphasised(
    id: &ElementId,
    message: SharedString,
    strong: Option<SharedString>,
    open: Option<Open>,
) -> AnyElement {
    let named = strong
        .as_ref()
        .and_then(|name| message.find(name.as_ref()).map(|at| at..at + name.len()));
    let Some(named) = named else {
        return message.into_any_element();
    };
    let emphasis = HighlightStyle {
        font_weight: Some(FontWeight::BOLD),
        underline: open.is_some().then_some(UnderlineStyle {
            thickness: LINK_UNDERLINE,
            ..UnderlineStyle::default()
        }),
        ..HighlightStyle::default()
    };
    let text = StyledText::new(message).with_highlights([(named.clone(), emphasis)]);
    let Some(open) = open else {
        return text.into_any_element();
    };
    InteractiveText::new((id.clone(), "link"), text)
        .on_click(vec![named], move |_, window, cx| open(window, cx))
        .into_any_element()
}
