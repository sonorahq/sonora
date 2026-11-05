use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, Interactivity, MouseButton, MouseMoveEvent, Pixels,
    ScrollWheelEvent, StyleRefinement, Window, div, px,
};

use crate::button::Button;
use crate::glass::{blurring, glass};
use crate::scrollbar::{Scrollbar, activate_middle_scroll, cancel_middle_scroll};
use crate::theme::ActiveTheme as _;
use crate::veil::effects;

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
    owns_scroll: bool,
    bottom_inset: Pixels,
    /// How tall the band along the top is where content fades out as it scrolls away.
    fade_top: Pixels,
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
            owns_scroll: true,
            bottom_inset: Pixels::ZERO,
            fade_top: Pixels::ZERO,
        }
    }

    /// A region whose child scrolls itself, a `uniform_list` above all. The surface still
    /// carries the wheel, the middle button and the bar, and leaves the offset to the handle
    /// the child tracks.
    #[track_caller]
    pub fn listing(id: impl Into<ElementId>, bar: &Entity<Scrollbar>) -> Self {
        Self {
            owns_scroll: false,
            ..Self::new(id, bar)
        }
    }

    /// Lets a caller merge the scroll presentation into child transforms, avoiding nested
    /// compositor sampling when those children already have their own spring motion.
    pub fn manual_presentation(mut self) -> Self {
        self.present_surface = false;
        self
    }

    /// Adds room after the scrollable content so its final item can rise above an overlaid
    /// control. This is added to caller padding, rather than replacing it. A self-scrolling
    /// listing keeps only its scrollbar clear; its list must provide its own trailing space.
    pub fn bottom_inset(mut self, inset: Pixels) -> Self {
        self.bottom_inset = inset.max(Pixels::ZERO);
        self
    }

    /// Fades content out across a band of `height` along the top, so what scrolls up under a
    /// control above the region melts away instead of being cut off at a hard line. The content
    /// starts below the band, so nothing is faded while the region is at rest.
    pub fn fade_top(mut self, height: Pixels) -> Self {
        self.fade_top = height.max(Pixels::ZERO);
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            id,
            bar,
            children,
            present_surface,
            owns_scroll,
            bottom_inset,
            fade_top,
        } = self;

        bar.update(cx, |bar, cx| bar.set_track_bottom(bottom_inset, cx));
        let scroll = bar.read(cx).scroll().clone();
        let overrides = std::mem::take(base.style());
        bar.read(cx).sync();
        let presentation = bar.read(cx).presentation();
        let gliding = bar.clone();

        let mut surface = middle_scroll(base, &bar)
            .id(id)
            .size_full()
            .when(owns_scroll, |surface| {
                surface
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&scroll)
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                match event.delta.precise() {
                    true => gliding.update(cx, |bar, _| bar.stirred()),
                    false => gliding.update(cx, |bar, _| bar.nudge(window)),
                }
            })
            .children(children);

        surface.style().refine(&overrides);
        if owns_scroll && bottom_inset > Pixels::ZERO {
            let padding = surface
                .style()
                .padding
                .bottom
                .unwrap_or_default()
                .to_pixels(Pixels::ZERO.into(), window.rem_size());
            surface = surface.pb(padding + bottom_inset);
        }
        if fade_top > Pixels::ZERO {
            let padding = surface
                .style()
                .padding
                .top
                .unwrap_or_default()
                .to_pixels(Pixels::ZERO.into(), window.rem_size());
            surface = surface.pt(padding + fade_top);
        }
        if present_surface {
            surface = surface.layer_translate(presentation);
        }

        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .size_full()
                    .when(fade_top > Pixels::ZERO && effects(), |this| {
                        this.fade_edges(fade_top, Pixels::ZERO)
                    })
                    .child(surface),
            )
            .child(bar)
    }
}

/// Adds browser-style middle-button auto-scrolling to a scrollable surface. A middle click
/// turns the mode on until the next press of any button. A middle press that is held and
/// dragged scrolls only while held, and `Root` ends it on the release. A middle press that a
/// child handles itself, such as a text field pasting on Linux, does not start it. A middle
/// press never moves focus or shows a pressed style on the children it passes through.
pub fn middle_scroll(surface: Div, bar: &Entity<Scrollbar>) -> Div {
    surface
        .capture_any_mouse_down(|event, window, cx| {
            if event.button == MouseButton::Middle {
                window.prevent_default();
            } else if cancel_middle_scroll(cx) {
                window.refresh();
                cx.stop_propagation();
            }
        })
        .on_mouse_down(MouseButton::Middle, {
            let gliding = bar.clone();
            move |event, window, cx| {
                let started = gliding.update(cx, |bar, cx| {
                    bar.middle_scroll_start(event.position, window, cx)
                });
                if started {
                    activate_middle_scroll(&gliding, cx);
                } else {
                    cancel_middle_scroll(cx);
                }
                window.refresh();
                cx.stop_propagation();
            }
        })
        .on_mouse_move({
            let gliding = bar.clone();
            move |event: &MouseMoveEvent, window, cx| {
                gliding.update(cx, |bar, cx| {
                    bar.middle_scroll_move(event.position, window, cx)
                });
            }
        })
}

/// The shape every control that floats over a scrolling region takes: a round bordered pill,
/// centred along the bottom. It swallows clicks meant for it rather than the rows behind, and
/// still lets the wheel through. The caller places it with `bottom_*`.
pub fn perched(button: Button, cx: &App) -> Div {
    let theme = *cx.theme();
    let button = match blurring(cx) {
        true => glass(button, cx),
        false => button.bg(theme.popover),
    };

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
                    .border_color(theme.border),
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
/// to be worth one. `goal` is how far down the region should sit, in the positive pixels
/// `Scrollbar::offset` reports, and is turned into gpui's negative offset before it is aimed at.
/// `tooltip` is an i18n key. The parent has to be `relative`.
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
            .secondary()
            .icon("icons/undo-2.svg")
            .tooltip(tooltip)
            .on_click(move |_, window, cx| {
                bar.update(cx, |bar, _| bar.aim(-goal, window));
            }),
        cx,
    ))
}
