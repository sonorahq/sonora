use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Div, ElementId, Pixels, ScrollHandle,
    ScrollWheelEvent, SharedString, StyleRefinement, Window, div, point, px, relative, svg,
};

use std::time::Duration;

use i18n::t;

use crate::glide::Glide;
use crate::motion::{ease_out_cubic, paced};
use crate::theme::ActiveTheme as _;

/// How far the items fade out towards a side of the rail, and how far the rail reaches past
/// its column on either side to make room for that fade.
const FADE: Pixels = px(48.);
/// The diameter of the buttons that move the items along.
const ARROW: Pixels = px(28.);
/// How large the chevron draws inside an arrow.
const CHEVRON: Pixels = px(14.);
/// How long an arrow takes to fade in or out.
const ARROW_FADE: Duration = Duration::from_millis(160);
/// How much of the rail's width one arrow press moves it.
const STEP: f32 = 0.75;
/// How close to an end the rail has to be to count as resting there.
const EDGE: Pixels = px(0.5);
/// The height a wheel line is measured at, only to tell which way the wheel turned.
const LINE: Pixels = px(1.);

/// A row of items that scrolls sideways, with a touchpad, Shift and the wheel, or the round
/// arrow over either end. It reaches past its column by the width of its fade on both sides and
/// pads its items back in by as much, so at rest the fades lie over empty room and an item dims
/// gradually as it scrolls into one. An arrow stays out until it is first needed and then fades
/// in and out as the rail reaches or leaves its end. Items keep their own width.
#[derive(IntoElement)]
pub struct Rail {
    id: ElementId,
    base: Div,
    children: Vec<AnyElement>,
    arrows_at: Option<Pixels>,
}

/// What a rail keeps between frames: where it is scrolled to, the glide that eases the wheel and
/// the arrows, and which arrows have been needed so far.
struct RailState {
    scroll: ScrollHandle,
    glide: Glide,
    seen: [bool; 2],
}

impl Rail {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            base: div(),
            children: Vec::new(),
            arrows_at: None,
        }
    }

    /// Centres the arrows `y` below the rail's top edge, such as on the middle of a card's
    /// picture rather than of the card with its name. They sit in the middle of the rail unless
    /// set.
    pub fn arrows_at(mut self, y: Pixels) -> Self {
        self.arrows_at = Some(y);
        self
    }
}

impl ParentElement for Rail {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Rail {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Rail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            mut base,
            children,
            arrows_at,
        } = self;

        let view = window.current_view();
        let state = window.use_keyed_state((id.clone(), "rail"), cx, |_, _| {
            let mut glide = Glide::default();
            glide.watch(view);
            RailState {
                scroll: ScrollHandle::new(),
                glide,
                seen: [false; 2],
            }
        });
        let (scroll, glide) = {
            let state = state.read(cx);
            (state.scroll.clone(), state.glide.clone())
        };
        glide.sync(&scroll);

        // Before the rail is first laid out its width is unknown, so neither arrow is wanted
        // yet and the next frame asks again.
        let measured = scroll.bounds().size.width > Pixels::ZERO;
        if !measured {
            window.request_animation_frame();
        }
        let hidden = -scroll.offset().x;
        let wanted = [
            measured && hidden > EDGE,
            measured && scroll.max_offset().x - hidden > EDGE,
        ];
        let seen = state.update(cx, |state, _| {
            for (seen, wanted) in state.seen.iter_mut().zip(wanted) {
                *seen |= wanted;
            }
            state.seen
        });

        let overrides = std::mem::take(base.style());
        let (wheeled, gliding) = (scroll.clone(), glide.clone());
        let mut row = base
            .id(id.clone())
            .flex()
            .gap_3()
            .mx(-FADE)
            .px(FADE)
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&scroll)
            // A touchpad already moves smoothly. A wheel jumps a whole step, which the glide
            // eases out.
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, _| {
                match event.delta.precise() {
                    true => gliding.jump(&wheeled, wheeled.offset()),
                    false if event.delta.pixel_delta(LINE).x != Pixels::ZERO => {
                        gliding.nudge(&wheeled, window)
                    }
                    false => {}
                }
            })
            .fade_sides(FADE, FADE)
            .children(
                children
                    .into_iter()
                    .map(|child| div().flex_none().child(child)),
            );
        row.style().refine(&overrides);

        let arrows = [false, true]
            .into_iter()
            .filter(|forward| seen[usize::from(*forward)])
            .map(|forward| {
                let wanted = wanted[usize::from(forward)];
                arrow(&id, forward, wanted, arrows_at, &scroll, &glide, cx)
            });

        div().relative().child(row).children(arrows)
    }
}

/// The round button over one end of a rail that glides it most of a view further, towards the
/// end it sits on. It is centred on the column's edge, over the inner half of the fade. While
/// not `wanted` it fades out and lets the pointer through to the items under it.
fn arrow(
    rail: &ElementId,
    forward: bool,
    wanted: bool,
    at: Option<Pixels>,
    scroll: &ScrollHandle,
    glide: &Glide,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    // the active wash is translucent, so it is laid over the button's own fill to keep the
    // cards under the arrow hidden
    let hover = theme.secondary.blend(theme.secondary_active);
    let (side, icon) = match forward {
        true => ("forward", "icons/chevron-right.svg"),
        false => ("back", "icons/chevron-left.svg"),
    };
    let (scroll, glide) = (scroll.clone(), glide.clone());
    let inset = -ARROW / 2.;

    let button = div()
        .id((rail.clone(), side))
        .absolute()
        .map(|this| match at {
            Some(y) => this.top(y - ARROW / 2.),
            None => this.top(relative(0.5)).mt(-ARROW / 2.),
        })
        .when(forward, |this| this.right(inset))
        .when(!forward, |this| this.left(inset))
        .size(ARROW)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.secondary)
        .border_1()
        .border_color(theme.border)
        .shadow_md()
        .child(
            svg()
                .path(icons::path(icon))
                .size(CHEVRON)
                .text_color(theme.foreground),
        )
        // a hidden arrow takes no clicks, so only one that is showing reaches screen readers
        .when(wanted, |this| {
            this.role(gpui::accesskit::Role::Button)
                .aria_label(match forward {
                    true => t!("rail-forward"),
                    false => t!("rail-back"),
                })
                .cursor_pointer()
                .occlude()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, window, _| {
                    let step = scroll.bounds().size.width * STEP;
                    let goal = glide.goal(&scroll);
                    let x = match forward {
                        true => goal.x - step,
                        false => goal.x + step,
                    };
                    glide.aim(&scroll, point(x, goal.y), window);
                })
        });

    if cx.reduce_motion() {
        return button
            .opacity(f32::from(u8::from(wanted)))
            .into_any_element();
    }
    // a new id on each change starts the fade again
    let fade = SharedString::from(format!("{side}-{}", if wanted { "in" } else { "out" }));
    button
        .with_animation(
            (rail.clone(), fade),
            Animation::new(paced(ARROW_FADE)).with_easing(ease_out_cubic),
            move |button, delta| match wanted {
                true => button.opacity(delta),
                false => button.opacity(1. - delta),
            },
        )
        .into_any_element()
}
