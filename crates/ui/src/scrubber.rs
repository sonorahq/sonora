use crate::metrics::Text;
use crate::theme::ActiveTheme as _;
use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Div, DragMoveEvent, ElementId, Empty, Hsla, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, Point, Render, SharedString, Stateful, StyleRefinement, Window, canvas,
    div, px, relative,
};

const TRACK: f32 = 0.5;
const THUMB: f32 = 1.5;
const HIT: f32 = 3.;
const BUBBLE: f32 = 7.;

type Slide = Box<dyn Fn(&f32, &mut Window, &mut App) + 'static>;
type Release = Box<dyn Fn(&MouseUpEvent, &mut Window, &mut App) + 'static>;

fn track(pad: Pixels) -> Pixels {
    px((pad / px(1.) * TRACK).round())
}

fn thumb(pad: Pixels) -> Pixels {
    px((pad / px(1.) * THUMB).round())
}

fn hit(pad: Pixels) -> Pixels {
    px((pad / px(1.) * HIT).round())
}

#[derive(Clone)]
struct Grab(SharedString);

impl Render for Grab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub struct ScrubberState {
    id: SharedString,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl ScrubberState {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }

    pub fn hovered(&self, position: Point<Pixels>, pad: Pixels) -> Option<f32> {
        self.bounds
            .get()
            .contains(&position)
            .then(|| self.fraction_at(position.x, pad))
    }

    fn fraction_at(&self, x: Pixels, pad: Pixels) -> f32 {
        let bounds = self.bounds.get();
        let pin = thumb(pad);
        let travel = bounds.size.width - pin;
        if travel <= px(0.) {
            return 0.;
        }
        ((x - bounds.origin.x - pin / 2.) / travel).clamp(0., 1.)
    }

    fn climb_at(&self, y: Pixels, pad: Pixels) -> f32 {
        let bounds = self.bounds.get();
        let pin = thumb(pad);
        let travel = bounds.size.height - pin;
        if travel <= px(0.) {
            return 0.;
        }
        1. - ((y - bounds.origin.y - pin / 2.) / travel).clamp(0., 1.)
    }
}

#[derive(IntoElement)]
pub struct Scrubber {
    base: Stateful<Div>,
    id: SharedString,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    fraction: f32,
    filled: Hsla,
    empty: Hsla,
    thumb: Hsla,
    enabled: bool,
    vertical: bool,
    bubble: Option<(f32, SharedString)>,
    lift: Pixels,
    on_move: Option<Slide>,
    on_release: Option<Release>,
}

impl Scrubber {
    pub fn new(state: &ScrubberState, fraction: f32) -> Self {
        Self {
            base: div().id(ElementId::Name(state.id.clone())),
            id: state.id.clone(),
            bounds: state.bounds.clone(),
            fraction: fraction.clamp(0., 1.),
            filled: gpui::white(),
            empty: gpui::black(),
            thumb: gpui::white(),
            enabled: true,
            vertical: false,
            bubble: None,
            lift: px(12.),
            on_move: None,
            on_release: None,
        }
    }

    pub fn bubble(mut self, fraction: f32, text: impl Into<SharedString>) -> Self {
        self.bubble = Some((fraction.clamp(0., 1.), text.into()));
        self
    }

    pub fn lift(mut self, lift: Pixels) -> Self {
        self.lift = lift;
        self
    }

    pub fn colors(mut self, filled: Hsla, empty: Hsla, thumb: Hsla) -> Self {
        self.filled = filled;
        self.empty = empty;
        self.thumb = thumb;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    pub fn on_move(mut self, handler: impl Fn(&f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_move = Some(Box::new(handler));
        self
    }

    pub fn on_release(
        mut self,
        handler: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_release = Some(Box::new(handler));
        self
    }
}

impl Styled for Scrubber {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Scrubber {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pad = cx.theme().metrics.pad;
        let line = track(pad);
        let pin = thumb(pad);
        let reach = hit(pad);
        let bubble_width = px((pad / px(1.) * BUBBLE).round());
        let popover = cx.theme().popover;
        let popover_border = cx.theme().border;
        let popover_text = cx.theme().popover_foreground;
        let text_size = cx.theme().text(Text::Tiny);

        let Self {
            mut base,
            id,
            bounds,
            fraction,
            filled,
            empty,
            thumb,
            enabled,
            vertical,
            bubble,
            lift,
            on_move,
            on_release,
        } = self;
        let overrides = std::mem::take(base.style());

        let state = Rc::new(ScrubberState {
            id: id.clone(),
            bounds: bounds.clone(),
        });
        let on_move = on_move.map(Rc::new);
        let on_release = on_release.map(Rc::new);

        let at = move |state: &ScrubberState, position: Point<Pixels>| match vertical {
            true => state.climb_at(position.y, pad),
            false => state.fraction_at(position.x, pad),
        };

        let down = {
            let state = state.clone();
            let on_move = on_move.clone();
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                if let Some(handler) = on_move.as_ref() {
                    handler(&at(&state, event.position), window, cx);
                }
            }
        };

        let dragged = {
            let state = state.clone();
            let on_move = on_move.clone();
            let mine = id.clone();
            move |event: &DragMoveEvent<Grab>, window: &mut Window, cx: &mut App| {
                if event.drag(cx).0 != mine {
                    return;
                }
                if let Some(handler) = on_move.as_ref() {
                    handler(&at(&state, event.event.position), window, cx);
                }
            }
        };

        let released = move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
            if let Some(handler) = on_release.as_ref() {
                handler(event, window, cx);
            }
        };

        let inset = match enabled {
            true => pin / 2.,
            false => Pixels::ZERO,
        };

        let bar = match vertical {
            true => div()
                .relative()
                .h_full()
                .w(line)
                .rounded_full()
                .bg(empty)
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(inset)
                        .bottom(inset)
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(Pixels::ZERO - inset)
                                .top(relative(1. - fraction))
                                .rounded_full()
                                .bg(filled),
                        )
                        .when(enabled, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .left((line - pin) / 2.)
                                    .bottom(relative(fraction))
                                    .mb(Pixels::ZERO - pin / 2.)
                                    .size(pin)
                                    .rounded_full()
                                    .bg(thumb),
                            )
                        })
                        .when_some(bubble, |this, (at, text)| {
                            this.child(
                                div()
                                    .absolute()
                                    .map(|this| {
                                        if lift >= Pixels::ZERO {
                                            this.right(line + lift)
                                        } else {
                                            this.left(line - lift)
                                        }
                                    })
                                    .bottom(relative(at))
                                    .mb(Pixels::ZERO - pin / 2.)
                                    .h(pin)
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .px_1p5()
                                            .rounded_md()
                                            .bg(popover)
                                            .border_1()
                                            .border_color(popover_border)
                                            .text_color(popover_text)
                                            .text_size(text_size)
                                            .whitespace_nowrap()
                                            .child(text),
                                    ),
                            )
                        }),
                ),
            false => div()
                .relative()
                .w_full()
                .h(line)
                .rounded_full()
                .bg(empty)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(inset)
                        .right(inset)
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(Pixels::ZERO - inset)
                                .right(relative(1. - fraction))
                                .rounded_full()
                                .bg(filled),
                        )
                        .when(enabled, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top((line - pin) / 2.)
                                    .left(relative(fraction))
                                    .ml(Pixels::ZERO - pin / 2.)
                                    .size(pin)
                                    .rounded_full()
                                    .bg(thumb),
                            )
                        })
                        .when_some(bubble, |this, (at, text)| {
                            this.child(
                                div()
                                    .absolute()
                                    .map(|this| {
                                        if lift >= Pixels::ZERO {
                                            this.bottom(lift)
                                        } else {
                                            this.top(Pixels::ZERO - lift)
                                        }
                                    })
                                    .left(relative(at))
                                    .ml(Pixels::ZERO - bubble_width / 2.)
                                    .w(bubble_width)
                                    .flex()
                                    .justify_center()
                                    .child(
                                        div()
                                            .px_1p5()
                                            .rounded_md()
                                            .bg(popover)
                                            .border_1()
                                            .border_color(popover_border)
                                            .text_color(popover_text)
                                            .text_size(text_size)
                                            .child(text),
                                    ),
                            )
                        }),
                ),
        };

        let mut scrubber = base
            .role(gpui::accesskit::Role::Slider)
            .aria_numeric_value(f64::from(fraction))
            .aria_min_numeric_value(0.)
            .aria_max_numeric_value(1.)
            .aria_orientation(match vertical {
                true => gpui::accesskit::Orientation::Vertical,
                false => gpui::accesskit::Orientation::Horizontal,
            })
            .relative()
            .flex()
            .when_else(
                vertical,
                |this| this.justify_center().h_full().w(reach),
                |this| this.items_center().w_full().h(reach),
            )
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .when(enabled, |this| {
                this.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, down)
                    .on_drag(Grab(id.clone()), |grab, _, _, cx| cx.new(|_| grab.clone()))
                    .on_drag_move(dragged)
                    .on_mouse_up(MouseButton::Left, released.clone())
                    .on_mouse_up_out(MouseButton::Left, released)
            })
            .child(bar);
        scrubber.style().refine(&overrides);
        scrubber
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, point, px, size};

    use super::{ScrubberState, hit, thumb, track};

    const PAD: gpui::Pixels = px(8.);
    const ORIGIN: f32 = 20.;
    const WIDTH: f32 = 300.;

    fn measured() -> ScrubberState {
        let state = ScrubberState::new("test");
        state.bounds.set(Bounds {
            origin: point(px(ORIGIN), px(0.)),
            size: size(px(WIDTH), hit(PAD)),
        });
        state
    }

    #[test]
    fn the_thumb_lands_under_the_pointer() {
        let state = measured();
        let pin = thumb(PAD);
        let travel = px(WIDTH) - pin;

        for step in 0..=100 {
            let fraction = step as f32 / 100.;
            let center = px(ORIGIN) + pin / 2. + travel * fraction;
            let read = state.fraction_at(center, PAD);
            assert!((read - fraction).abs() < 1e-4, "{fraction} read as {read}");
        }
    }

    #[test]
    fn the_ends_clamp() {
        let state = measured();

        assert_eq!(state.fraction_at(px(0.), PAD), 0.);
        assert_eq!(state.fraction_at(px(ORIGIN), PAD), 0.);
        assert_eq!(state.fraction_at(px(ORIGIN + WIDTH), PAD), 1.);
        assert_eq!(state.fraction_at(px(9999.), PAD), 1.);
    }

    #[test]
    fn an_unmeasured_track_reads_zero() {
        let state = ScrubberState::new("test");

        assert_eq!(state.fraction_at(px(50.), PAD), 0.);
    }

    #[test]
    fn the_bubble_shows_exactly_where_the_grab_lands() {
        let state = measured();
        let reach = hit(PAD);
        let x = px(ORIGIN + WIDTH / 2.);

        assert!(reach > track(PAD) * 4.);
        assert!(state.hovered(point(x, px(0.)), PAD).is_some());
        assert!(state.hovered(point(x, reach - px(1.)), PAD).is_some());
        assert!(state.hovered(point(x, px(-1.)), PAD).is_none());
        assert!(state.hovered(point(x, reach), PAD).is_none());
    }
}
