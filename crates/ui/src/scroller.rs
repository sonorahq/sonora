use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, Interactivity, Pixels, ScrollWheelEvent,
    StyleRefinement, Window, div, px,
};

use crate::button::Button;
use crate::scrollbar::Scrollbar;
use crate::theme::ActiveTheme as _;

/// How far a region has to be scrolled before the trip back is worth a button, in rows.
const REACH: f32 = 3.;
/// How far a perched control floats off the bottom of its region.
const PERCH: Pixels = px(12.);

#[derive(IntoElement)]
pub struct Scroller {
    base: Div,
    id: ElementId,
    bar: Entity<Scrollbar>,
    children: Vec<AnyElement>,
    present_surface: bool,
}

impl Scroller {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, bar: &Entity<Scrollbar>) -> Self {
        Self {
            base: div(),
            id: id.into(),
            bar: bar.clone(),
            children: Vec::new(),
            present_surface: true,
        }
    }

    /// Lets a caller merge the scroll presentation into child transforms, avoiding nested
    /// compositor sampling when those children already have their own spring motion.
    pub fn manual_presentation(mut self) -> Self {
        self.present_surface = false;
        self
    }
}

impl Styled for Scroller {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Scroller {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl ParentElement for Scroller {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Scroller {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            id,
            bar,
            children,
            present_surface,
        } = self;

        let scroll = bar.read(cx).scroll().clone();
        let overrides = std::mem::take(base.style());
        bar.read(cx).sync();
        let presentation = bar.read(cx).presentation();
        let gliding = bar.clone();

        let mut surface = base
            .id(id)
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&scroll)
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                match event.delta.precise() {
                    true => gliding.update(cx, |bar, _| bar.stirred()),
                    false => gliding.update(cx, |bar, _| bar.nudge(window)),
                }
            })
            .children(children);

        surface.style().refine(&overrides);
        if present_surface {
            surface = surface.layer_translate(presentation);
        }

        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(surface)
            .child(bar)
    }
}

/// The shape every control that floats over a scrolling region takes: a round bordered pill,
/// centred along the bottom. It swallows clicks meant for it rather than the rows behind, and
/// still lets the wheel through. The caller places it with `bottom_*`.
pub fn perched(button: Button, cx: &App) -> Div {
    let theme = *cx.theme();

    div()
        .absolute()
        .bottom(PERCH)
        .w_full()
        .flex()
        .justify_center()
        .child(
            div().flex().flex_none().block_mouse_except_scroll().child(
                button
                    .ghost()
                    .small()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover),
            ),
        )
}

/// The room a scrolling region has to keep under its last row for a perched control to float in
/// without covering it.
pub fn perch_room(cx: &App) -> Pixels {
    PERCH * 2. + cx.theme().metrics.control_small
}

/// A perched button that glides a scrolling region back to its top. It stays away until the
/// region has been scrolled far enough for the trip to be worth one. The parent has to be
/// `relative`.
pub fn return_top(id: impl Into<ElementId>, bar: &Entity<Scrollbar>, cx: &App) -> Option<Div> {
    return_to(id, bar, Pixels::ZERO, "nav-return-top", cx)
}

/// A perched button that glides a scrolling region back to a resting offset, in either
/// direction. It stays away until the region has drifted far enough from that spot for the trip
/// to be worth one. `tooltip` is an i18n key. The parent has to be `relative`.
pub fn return_to(
    id: impl Into<ElementId>,
    bar: &Entity<Scrollbar>,
    goal: Pixels,
    tooltip: &'static str,
    cx: &App,
) -> Option<Div> {
    let viewport = bar.read(cx).viewport();
    let reach = (cx.theme().metrics.list_row * REACH).min(viewport / 2.);
    if viewport <= Pixels::ZERO || (bar.read(cx).offset() - goal).abs() < reach {
        return None;
    }
    let bar = bar.clone();

    Some(perched(
        Button::new(id)
            .icon("icons/undo-2.svg")
            .tooltip(tooltip)
            .on_click(move |_, window, cx| {
                bar.update(cx, |bar, _| bar.aim(goal, window));
            }),
        cx,
    ))
}
