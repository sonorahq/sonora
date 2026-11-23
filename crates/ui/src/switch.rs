use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Div, ElementId, Hsla, Interactivity, SharedString, Stateful, StyleRefinement,
    Window, div, px,
};

use crate::motion::{Motion, Motioned as _, Movement, glow_fill, glowing};
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
    pending: bool,
    disabled: bool,
    name: Option<SharedString>,
    label: Option<SharedString>,
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
            pending: false,
            disabled: false,
            name: None,
            label: None,
            on_click: None,
        }
    }

    /// Names the switch for screen readers.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Shows a short label beside the thumb inside the track.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Keeps the switch operable while its checked state is taking effect.
    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
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
            pending,
            disabled,
            name,
            label,
            on_click,
        } = self;
        let theme = *cx.theme();
        let height = px((theme.metrics.control_small / px(1.) * SCALE).round());
        let width = px((height / px(1.) * WIDTH).round());
        let thumb = height - px((INSET + BORDER) * 2.);
        let travel = if label.is_some() {
            px(0.)
        } else {
            width - height
        };
        let (on_track, on_hover, on_knob) = match pending {
            true => (
                theme.secondary_active,
                theme.secondary_hover,
                theme.foreground,
            ),
            false => (theme.primary, theme.primary_hover, theme.primary_foreground),
        };
        let overrides = std::mem::take(base.style());

        let (from, to) = match checked {
            true => (0., 1.),
            false => (1., 0.),
        };
        let (track_was, track_is) = match checked {
            true => (theme.muted, on_track),
            false => (on_track, theme.muted),
        };
        let (edge_was, edge_is) = match checked {
            true => (theme.border, theme.primary),
            false => (theme.primary, theme.border),
        };
        let (hover_was, hover_is) = match checked {
            true => (theme.secondary_hover, on_hover),
            false => (on_hover, theme.secondary_hover),
        };
        let (knob_was, knob_is) = match checked {
            true => (theme.muted_foreground, on_knob),
            false => (on_knob, theme.muted_foreground),
        };

        let movement =
            window.use_keyed_state((id.clone(), "movement"), cx, |_, _| Movement::new(checked));
        let animates = movement.update(cx, |movement, _| movement.turning(checked));
        let knob = div().size(thumb).flex_none().rounded(thumb / 2.);
        let name = name.or_else(|| label.clone());
        let label = label.map(|text| {
            div()
                .px(theme.metrics.pad / 2.)
                .whitespace_nowrap()
                .text_size(theme.text(crate::Text::Tiny))
                .text_color(knob_is)
                .child(text)
        });
        let labeled = label.is_some();
        let (before, after) = if checked {
            (label, None)
        } else {
            (None, label)
        };

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
            .when(!labeled, |this| this.w(width))
            .when(labeled, |this| this.gap_1())
            .h(height)
            .p(px(INSET))
            .rounded(height / 2.)
            .bg(track_is)
            .border_1()
            .border_color(edge_is)
            .when(disabled, |this| this.opacity(0.4))
            .when(!disabled, |this| this.cursor_pointer())
            .children(before)
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
            })
            .children(after);

        switch.style().refine(&overrides);
        if !disabled && let Some(handler) = on_click {
            switch = switch.on_click(handler);
        }

        let (switch, level) = match disabled {
            true => (switch, 0.),
            false => glowing(switch, &id, false, 1., window, cx),
        };
        // The track under the pointer, on its way between its own colour and the hover.
        let lit =
            move |rest: Hsla, hover: Hsla| glow_fill(Some(rest), hover, level).unwrap_or(rest);

        match animates {
            true => switch
                .motion(
                    ("track", usize::from(checked)),
                    Motion::Control,
                    move |track, t| {
                        track
                            .bg(lit(
                                crate::motion::mix(track_was, track_is, t),
                                crate::motion::mix(hover_was, hover_is, t),
                            ))
                            .border_color(crate::motion::mix(edge_was, edge_is, t))
                    },
                )
                .into_any_element(),
            false => switch.bg(lit(track_is, hover_is)).into_any_element(),
        }
    }
}
