use gpui::prelude::*;
use gpui::{
    AnyElement, App, Axis, Div, ElementId, Entity, Interactivity, MouseButton, MouseMoveEvent,
    Pixels, ScrollWheelEvent, StyleRefinement, Window, div, px,
};

use crate::button::Button;
use crate::glass::{blurring, glass};
use crate::glide::mark_scrolling;
use crate::hold::{Hold, held};
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
    /// How far below the region's top its scrollbar starts, when the caller sets it.
    top_inset: Option<Pixels>,
    /// How tall the band along the top is where content fades out as it scrolls away.
    fade_top: Pixels,
    /// How tall the bands are where content fades at whichever end still has more to scroll to.
    fade_ends: Pixels,
    /// How tall the band along the top grows, as the content scrolls up, where it fades out.
    fade_scrolled: Pixels,
    /// How tall the band along the bottom is, while more lies below, where content fades out.
    fade_remaining: Pixels,
    /// How tall the band along the bottom is where content always fades out.
    fade_bottom: Pixels,
    /// Whether the region holds what is at its top when it changes width, and whether that hold
    /// is active right now.
    hold: Option<bool>,
    /// Whether the held content reflowed this frame without the region changing width.
    reflowed: bool,
    /// The child held where it stands on screen in place of what is at the top.
    keep: Option<usize>,
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
            top_inset: None,
            fade_top: Pixels::ZERO,
            fade_ends: Pixels::ZERO,
            fade_scrolled: Pixels::ZERO,
            fade_remaining: Pixels::ZERO,
            fade_bottom: Pixels::ZERO,
            hold: None,
            reflowed: false,
            keep: None,
        }
    }

    /// Keeps what is at the top of the region in place when the region changes width and its
    /// content reflows. The direct children are anchors by their position, and so are any
    /// `held` items or holding `Deck`s inside them.
    pub fn hold(self) -> Self {
        self.hold_while(true)
    }

    /// Like `hold`, but leaves the scroll alone while `active` is false. It still keeps track
    /// of the top then, so it holds the right place again once active.
    pub fn hold_while(mut self, active: bool) -> Self {
        self.hold = Some(active);
        self
    }

    /// Tells a held region its content reflowed this frame even though its width stayed, as
    /// text wrapped to the width measured a frame earlier does.
    pub fn reflowed(mut self, reflowed: bool) -> Self {
        self.reflowed = reflowed;
        self
    }

    /// Holds the direct child at `place` where it stands on screen, rather than what is at the
    /// top, when a held region reflows. Lyrics keep the line being sung in view this way.
    pub fn keep(mut self, place: Option<usize>) -> Self {
        self.keep = place;
        self
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

    /// Starts the scrollbar `inset` below the region's top, clear of a header pinned there.
    /// Only the bar moves and the content still scrolls up to the top.
    pub fn top_inset(mut self, inset: Pixels) -> Self {
        self.top_inset = Some(inset.max(Pixels::ZERO));
        self
    }

    /// Fades content out across a band of `height` along the top, so what scrolls up under a
    /// control above the region melts away instead of being cut off at a hard line. The content
    /// starts below the band, so nothing is faded while the region is at rest.
    pub fn fade_top(mut self, height: Pixels) -> Self {
        self.fade_top = height.max(Pixels::ZERO);
        self
    }

    /// Fades content out across a band of `height` at each end that has more to scroll to, so a
    /// region too short for what it holds says so. An end with nothing past it stays crisp.
    pub fn fade_ends(mut self, height: Pixels) -> Self {
        self.fade_ends = height.max(Pixels::ZERO);
        self
    }

    /// Fades content out along the top once it scrolls, across a band that grows with the
    /// distance scrolled up to `height`. Unlike [`Self::fade_top`] it adds no room above the
    /// content, so the first item still starts at the very top and nothing fades at rest.
    pub fn fade_scrolled(mut self, height: Pixels) -> Self {
        self.fade_scrolled = height.max(Pixels::ZERO);
        self
    }

    /// The bottom's counterpart of [`Self::fade_scrolled`]: fades content out along the bottom
    /// while more lies below, across a band that shrinks from `height` to nothing as the end
    /// comes into view, so nothing fades once the content is scrolled all the way down.
    pub fn fade_remaining(mut self, height: Pixels) -> Self {
        self.fade_remaining = height.max(Pixels::ZERO);
        self
    }

    /// Fades content out across a band of `height` along the bottom whether or not there is more
    /// to scroll to, for a region whose foot lies under chrome floating over the window's edge.
    pub fn fade_bottom(mut self, height: Pixels) -> Self {
        self.fade_bottom = height.max(Pixels::ZERO);
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
            top_inset,
            fade_top,
            fade_ends,
            fade_scrolled,
            fade_remaining,
            fade_bottom,
            hold,
            reflowed,
            keep,
        } = self;
        let hold = hold.filter(|_| owns_scroll);
        let children: Vec<AnyElement> = match hold {
            Some(_) => children
                .into_iter()
                .enumerate()
                .map(|(place, child)| held(child_key(place), child).into_any_element())
                .collect(),
            None => children,
        };
        let held_id = id.clone();

        bar.update(cx, |bar, cx| {
            bar.set_track_bottom(bottom_inset, cx);
            if let Some(top) = top_inset {
                bar.set_track_top(top, cx);
            }
        });
        let scroll = bar.read(cx).scroll().clone();
        // The offset runs negative as the content moves up, so it sits between minus the furthest
        // it can go and zero. Half a pixel of slack keeps a rounded offset from fading an end.
        let (above, below, scrolled, remaining) = {
            let (at, end) = (-scroll.offset().y, scroll.max_offset().y);
            (
                at > px(0.5),
                end - at > px(0.5),
                at.max(Pixels::ZERO),
                (end - at).max(Pixels::ZERO),
            )
        };
        let end = |more: bool| match more {
            true => fade_ends,
            false => Pixels::ZERO,
        };
        let (fade_top_end, fade_bottom_end) = (
            end(above).max(scrolled.min(fade_scrolled)),
            end(below)
                .max(remaining.min(fade_remaining))
                .max(fade_bottom),
        );
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
                mark_scrolling(cx);
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
        let surface = match hold {
            Some(active) => Hold::new(held_id, Axis::Vertical, &scroll, surface)
                .active(active)
                .reflowed(reflowed)
                .keep(keep.map(child_key))
                .into_any_element(),
            None => surface.into_any_element(),
        };

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
                    .when(
                        (fade_top_end > Pixels::ZERO || fade_bottom_end > Pixels::ZERO)
                            && effects(),
                        |this| this.fade_edges(fade_top_end, fade_bottom_end),
                    )
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

/// The key a held region anchors its direct child at `place` by.
fn child_key(place: usize) -> ElementId {
    ElementId::NamedInteger("scroller-child".into(), place as u64)
}
