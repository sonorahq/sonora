use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, Global, MouseButton, Point,
    ScrollWheelEvent, SharedString, StyleRefinement, Window, div,
};

use crate::metrics::Text;
use crate::motion::Rising as _;
use crate::scrollbar::Scrollbar;
use crate::shield::Shield;
use crate::theme::ActiveTheme as _;

const BACKDROP: f32 = 0.8;
const WIDTH: f32 = 2.4;

type Dismiss = Rc<dyn Fn(&(), &mut Window, &mut App)>;

#[derive(Default)]
struct Bars(HashMap<ElementId, Entity<Scrollbar>>);

impl Global for Bars {}

fn bar(id: &ElementId, cx: &mut App) -> Entity<Scrollbar> {
    if let Some(known) = cx.try_global::<Bars>().and_then(|bars| bars.0.get(id)) {
        return known.clone();
    }
    let bar = cx.new(|_| Scrollbar::inset());
    cx.default_global::<Bars>()
        .0
        .insert(id.clone(), bar.clone());
    bar
}

#[derive(IntoElement)]
pub struct Modal {
    base: Div,
    id: ElementId,
    title: SharedString,
    detail: Option<SharedString>,
    body: Vec<AnyElement>,
    actions: Vec<AnyElement>,
    dismiss: Option<Dismiss>,
}

impl Modal {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            id: id.into(),
            title: title.into(),
            detail: None,
            body: Vec::new(),
            actions: Vec::new(),
            dismiss: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn child(mut self, body: impl IntoElement) -> Self {
        self.body.push(body.into_any_element());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Rc::new(handler));
        self
    }

    /// Sends the body of an open modal back to the top, for a caller that swapped its content.
    pub fn rewind(id: impl Into<ElementId>, cx: &mut App) {
        let id = id.into();
        let Some(bar) = cx
            .try_global::<Bars>()
            .and_then(|bars| bars.0.get(&id).cloned())
        else {
            return;
        };
        bar.read(cx).scroll().set_offset(Point::default());
    }
}

impl Styled for Modal {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Modal {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let pad = theme.metrics.pad;
        let room = pad * 2.;
        let Self {
            mut base,
            id,
            title,
            detail,
            body,
            actions,
            dismiss,
        } = self;
        let outside = dismiss.clone();
        let overrides = std::mem::take(base.style());
        let scroller = bar(&id, cx);
        scroller.read(cx).sync();
        let body_id = SharedString::from(format!("modal-body-{id:?}"));

        div()
            .absolute()
            .inset_0()
            .p(theme.metrics.inset)
            .flex()
            .items_center()
            .justify_center()
            .child(
                Shield::new(id)
                    .absolute()
                    .inset_0()
                    .bg(theme.background.opacity(BACKDROP))
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        if let Some(outside) = &outside {
                            outside(&(), window, cx);
                        }
                    }),
            )
            .child({
                let mut panel = base
                    .relative()
                    .occlude()
                    .w(theme.metrics.cover * WIDTH)
                    .max_w_full()
                    .max_h_full()
                    .flex()
                    .flex_col()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_md()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .flex_col()
                            .gap_1()
                            .px(room)
                            .pt(room)
                            .pb(pad)
                            .child(
                                div()
                                    .text_size(theme.text(Text::Large))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .when_some(detail, |this, detail| {
                                this.child(
                                    div()
                                        .text_size(theme.text(Text::Small))
                                        .text_color(theme.muted_foreground)
                                        .child(detail),
                                )
                            }),
                    )
                    .when(!body.is_empty(), |this| {
                        this.child(
                            div()
                                .relative()
                                .flex()
                                .flex_1()
                                .w_full()
                                .min_h_0()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .id(body_id.clone())
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .w_full()
                                        .min_w_0()
                                        .min_h_0()
                                        .gap(pad)
                                        .px(room)
                                        .py(pad)
                                        .overflow_y_scroll()
                                        .track_scroll(scroller.read(cx).scroll())
                                        .on_scroll_wheel({
                                            let gliding = scroller.clone();
                                            move |event: &ScrollWheelEvent, window, cx| {
                                                if event.delta.precise() {
                                                    return;
                                                }
                                                gliding.update(cx, |bar, _| bar.nudge(window));
                                            }
                                        })
                                        .children(body),
                                )
                                .child(scroller.clone()),
                        )
                    })
                    .when(!actions.is_empty(), |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_none()
                                .justify_end()
                                .gap_2()
                                .p(room)
                                .pt(pad)
                                .children(actions),
                        )
                    });
                panel.style().refine(&overrides);
                panel.rising("modal-rise")
            })
    }
}
