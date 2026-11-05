use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Div, ElementId, Interactivity, SharedString, Stateful, StyleRefinement,
    Window, div, px, svg,
};

use crate::metrics::Text;
use crate::motion::{Motion, Motioned as _, Movement, mix};
use crate::theme::ActiveTheme as _;

const SCALE: f32 = 0.66;
const MARK: f32 = 0.68;
const GAP: f32 = 8.;

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    base: Stateful<Div>,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    /// What a screen reader calls the checkbox in place of its label.
    name: Option<SharedString>,
    on_click: Option<Click>,
}

impl Checkbox {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        let id = id.into();

        Self {
            base: div().id(id.clone()),
            id,
            checked,
            disabled: false,
            label: None,
            name: None,
            on_click: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Names the checkbox for screen readers in place of its label, for one whose label sits
    /// elsewhere in the row or that has none.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl Styled for Checkbox {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Checkbox {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            mut base,
            checked,
            disabled,
            label,
            name,
            on_click,
        } = self;
        let theme = *cx.theme();
        let side = px((theme.metrics.control_small / px(1.) * SCALE).round());
        let radius = theme.radius.min(side / 3.);
        let overrides = std::mem::take(base.style());

        let (box_was, box_is) = match checked {
            true => (theme.muted, theme.primary),
            false => (theme.primary, theme.muted),
        };
        let (edge_was, edge_is) = match checked {
            true => (theme.border, theme.primary),
            false => (theme.primary, theme.border),
        };
        let (mark_was, mark_is) = match checked {
            true => (theme.muted, theme.primary_foreground),
            false => (theme.primary_foreground, theme.muted),
        };

        let movement = window.use_keyed_state((id, "movement"), cx, |_, _| Movement::new(checked));
        let animates = movement.update(cx, |movement, _| movement.turning(checked));
        let mark = svg()
            .path(icons::path("icons/check.svg"))
            .size(px((side / px(1.) * MARK).round()))
            .flex_none();
        let mark = match animates {
            true => mark
                .motion(
                    ("mark", usize::from(checked)),
                    Motion::Control,
                    move |mark, t| mark.text_color(mix(mark_was, mark_is, t)),
                )
                .into_any_element(),
            false => mark.text_color(mark_is).into_any_element(),
        };
        let square = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(side)
            .rounded(radius)
            .border_1()
            .child(mark);

        let square = match animates {
            true => square
                .motion(
                    ("box", usize::from(checked)),
                    Motion::Control,
                    move |square, t| {
                        square
                            .bg(mix(box_was, box_is, t))
                            .border_color(mix(edge_was, edge_is, t))
                    },
                )
                .into_any_element(),
            false => square.bg(box_is).border_color(edge_is).into_any_element(),
        };

        let mut checkbox = base
            .role(gpui::accesskit::Role::CheckBox)
            .aria_toggled(gpui::accesskit::Toggled::from(checked))
            .when_some(name.or_else(|| label.clone()), |this, name| {
                this.aria_label(name)
            })
            .flex()
            .items_center()
            .gap(px(GAP))
            .text_size(theme.text(Text::Small))
            .text_color(theme.foreground)
            .when(disabled, |this| this.opacity(0.4))
            .when(!disabled, |this| this.cursor_pointer())
            .child(square)
            .children(label.map(|label| div().child(label)));

        checkbox.style().refine(&overrides);
        if !disabled && let Some(handler) = on_click {
            checkbox = checkbox.on_click(handler);
        }
        checkbox
    }
}
