use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Div, ElementId, Interactivity, SharedString, Stateful, StyleRefinement,
    Window, div, px,
};

use crate::motion::{Motion, Motioned as _, Movement};
use crate::theme::ActiveTheme as _;

const SCALE: f32 = 0.85;
const INSET: f32 = 2.;
const WIDTH: f32 = 1.75;
const BORDER: f32 = 1.;

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    base: Stateful<Div>,
    checked: bool,
    disabled: bool,
    name: Option<SharedString>,
    on_click: Option<Click>,
}

impl Switch {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        let id = id.into();

        Self {
            base: div().id(id.clone()),
            id,
            checked,
            disabled: false,
            name: None,
            on_click: None,
        }
    }

    /// Names the switch for screen readers. It draws no text of its own, so a switch without a
    /// name is announced as nothing but its state.
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

impl Styled for Switch {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Switch {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            mut base,
            checked,
            disabled,
            name,
            on_click,
        } = self;
        let theme = *cx.theme();
        let height = px((theme.metrics.control_small / px(1.) * SCALE).round());
        let width = px((height / px(1.) * WIDTH).round());
        let thumb = height - px((INSET + BORDER) * 2.);
        let travel = width - height;
        let background = match checked {
            true => theme.primary,
            false => theme.muted,
        };
        let overrides = std::mem::take(base.style());

        let (from, to) = match checked {
            true => (0., 1.),
            false => (1., 0.),
        };
        let (track_was, track_is) = match checked {
            true => (theme.muted, theme.primary),
            false => (theme.primary, theme.muted),
        };
        let (edge_was, edge_is) = match checked {
            true => (theme.border, theme.primary),
            false => (theme.primary, theme.border),
        };
        let (hover_was, hover_is) = match checked {
            true => (theme.secondary_hover, theme.primary_hover),
            false => (theme.primary_hover, theme.secondary_hover),
        };
        let (knob_was, knob_is) = match checked {
            true => (theme.muted_foreground, theme.primary_foreground),
            false => (theme.primary_foreground, theme.muted_foreground),
        };

        let movement = window.use_keyed_state((id, "movement"), cx, |_, _| Movement::new(checked));
        let animates = movement.update(cx, |movement, _| movement.turning(checked));
        let knob = div().size(thumb).flex_none().rounded(thumb / 2.);

        let mut switch = base
            // The role sits on the inner div because the animation wrapper
            // around it never forwards one, while the real tree still nests
            // this node during prepaint.
            .role(gpui::accesskit::Role::Switch)
            .aria_toggled(gpui::accesskit::Toggled::from(checked))
            .when_some(name, |this, name| this.aria_label(name))
            .flex()
            .flex_none()
            .items_center()
            .w(width)
            .h(height)
            .p(px(INSET))
            .rounded(height / 2.)
            .bg(background)
            .border_1()
            .border_color(match checked {
                true => theme.primary,
                false => theme.border,
            })
            .when(disabled, |this| this.opacity(0.4))
            .when(!disabled, |this| this.cursor_pointer())
            .child(match animates {
                true => knob
                    .motion(
                        ("thumb", usize::from(checked)),
                        Motion::Control,
                        move |knob, t| {
                            knob.ml(travel * (from + (to - from) * t))
                                .bg(crate::motion::mix(knob_was, knob_is, t))
                        },
                    )
                    .into_any_element(),
                false => knob.ml(travel * to).bg(knob_is).into_any_element(),
            });

        switch.style().refine(&overrides);
        if !disabled && let Some(handler) = on_click {
            switch = switch.on_click(handler);
        }

        match animates {
            true => switch
                .motion(
                    ("track", usize::from(checked)),
                    Motion::Control,
                    move |track, t| {
                        let hover = crate::motion::mix(hover_was, hover_is, t);
                        track
                            .bg(crate::motion::mix(track_was, track_is, t))
                            .border_color(crate::motion::mix(edge_was, edge_is, t))
                            .when(!disabled, move |this| {
                                this.hover(move |style| style.bg(hover))
                            })
                    },
                )
                .into_any_element(),
            false => switch
                .when(!disabled, move |this| {
                    this.hover(move |style| style.bg(hover_is))
                })
                .into_any_element(),
        }
    }
}
