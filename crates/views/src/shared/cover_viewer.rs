//! The full-size view an album's cover opens: the large artwork at once, the provider's
//! original over it when it lands, zoomed and dragged around and closed with Escape or a
//! click on the backdrop.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Context, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render,
    ScrollWheelEvent, SharedString, Window, anchored, deferred, div, point, px,
};
use input::{COVER_VIEWER_CONTEXT, ZoomIn, ZoomOut, ZoomReset};
use ui::{ActiveTheme as _, Artwork, Button, Dismiss, Motion, Motioned as _, Shield, snapped};

/// How dark the window goes behind the artwork.
const BACKDROP: f32 = 0.85;
/// The widest zoom as a multiple of the fit, enough to read a sticker or a signature on a
/// high-resolution scan.
const MAX_ZOOM: f32 = 4.;
/// One zoom key, button or wheel notch multiplies the zoom by this much.
const ZOOM_STEP: f32 = 1.5;
/// The wheel travel of one zoom step.
const WHEEL_STEP: Pixels = px(20.);

/// A drag in progress: where the pointer grabbed the artwork and where the artwork was.
#[derive(Clone, Copy)]
struct Drag {
    pointer: Point<Pixels>,
    pan: Point<Pixels>,
}

struct Open {
    title: SharedString,
    large: Option<SharedString>,
    full: Option<SharedString>,
    zoom: f32,
    pan: Point<Pixels>,
    drag: Option<Drag>,
    /// Wheel travel gathered towards the next zoom step, so a touchpad's stream of small
    /// deltas zooms at the same pace a wheel does.
    wheel: Pixels,
}

/// The side of the square the artwork fits into unzoomed: the window's short edge, clear of
/// the title bar and the inset.
fn fit_side(window: &Window, cx: &App) -> Pixels {
    let chrome = snapped(cx.theme().metrics.title_bar, window);
    let room = cx.theme().metrics.inset * 2.;
    let viewport = window.viewport_size();
    (viewport.width - room)
        .max(Pixels::ZERO)
        .min((viewport.height - chrome - room).max(Pixels::ZERO))
}

/// Keeps the artwork's edge from crossing the window's: panning is free once a zoomed cover
/// overflows, and pinned at the fit while it does not.
fn clamp_pan(pan: Point<Pixels>, zoom: f32, window: &Window, cx: &App) -> Point<Pixels> {
    let side = fit_side(window, cx) * zoom;
    let viewport = window.viewport_size();
    let chrome = snapped(cx.theme().metrics.title_bar, window);
    let across = ((side - viewport.width) / 2.).max(Pixels::ZERO);
    let down = ((side - (viewport.height - chrome)) / 2.).max(Pixels::ZERO);
    point(pan.x.clamp(-across, across), pan.y.clamp(-down, down))
}

/// The album cover viewer above the detail page, scoped to the album the page shows rather
/// than to whatever happens to be playing.
pub(crate) struct CoverViewer {
    open: Option<Open>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    grab: bool,
}

impl CoverViewer {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            open: None,
            focus: cx.focus_handle(),
            restore: None,
            grab: false,
        }
    }

    /// Opens the viewer on the album's `large` cover, with the provider's original swapped in
    /// once it loads. A provider that cannot name an original keeps the large cover.
    pub(crate) fn open(
        &mut self,
        title: SharedString,
        large: SharedString,
        full: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.open = Some(Open {
            title,
            large: Some(large),
            full,
            zoom: 1.,
            pan: Point::default(),
            drag: None,
            wheel: Pixels::ZERO,
        });
        self.grab = true;
        cx.notify();
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.grab = false;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn zoom_by(&mut self, step: f32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.zoom = (open.zoom * step).clamp(1., MAX_ZOOM);
        open.pan = clamp_pan(open.pan, open.zoom, window, cx);
        cx.notify();
    }

    fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.zoom = 1.;
        open.pan = Point::default();
        cx.notify();
    }
}

impl Render for CoverViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.grab {
            self.restore = window.focused(cx);
            window.focus(&self.focus, cx);
            self.grab = false;
        }
        let Some(open) = self.open.as_ref() else {
            return div().into_any_element();
        };
        let theme = *cx.theme();
        let title = open.title.clone();
        let zoom = open.zoom;
        let pan = open.pan;

        let chrome = snapped(theme.metrics.title_bar, window);
        let viewport = window.viewport_size();
        let stage = point(viewport.width, viewport.height - chrome);
        let side = fit_side(window, cx) * zoom;
        let origin = point((stage.x - side) / 2. + pan.x, (stage.y - side) / 2. + pan.y);

        let artwork = div()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .size(side)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let Some(open) = this.open.as_mut() else {
                        return;
                    };
                    open.drag = Some(Drag {
                        pointer: event.position,
                        pan: open.pan,
                    });
                }),
            )
            .child(
                Artwork::new(open.large.clone())
                    .size(side)
                    .contain()
                    .corner_radius(theme.radius),
            )
            .children(open.full.clone().map(|full| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(
                        Artwork::new(Some(full))
                            .size(side)
                            .contain()
                            .corner_radius(theme.radius)
                            .overlay(),
                    )
                    .motion("cover-full", Motion::Slow, |art, t| art.opacity(t))
                    .into_any_element()
            }));

        let button = |id: &'static str, icon: &'static str, tooltip: &'static str| {
            Button::new(id).icon(icon).small().tooltip(tooltip)
        };
        let controls = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(theme.metrics.inset)
            .flex()
            .justify_center()
            .child(
                div()
                    .occlude()
                    .flex()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_md()
                    .child(
                        button("cover-zoom-out", "icons/minimize.svg", "cover-zoom-out").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.zoom_by(1. / ZOOM_STEP, window, cx)
                            }),
                        ),
                    )
                    .child(
                        button("cover-zoom-in", "icons/plus.svg", "cover-zoom-in").on_click(
                            cx.listener(|this, _, window, cx| this.zoom_by(ZOOM_STEP, window, cx)),
                        ),
                    )
                    .child(
                        button("cover-zoom-fit", "icons/maximize.svg", "cover-zoom-fit")
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_fit(cx))),
                    )
                    .child(
                        button("cover-close", "icons/x.svg", "common-dismiss")
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    ),
            );

        let frame = div()
            .w(stage.x)
            .h(stage.y)
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let Some(open) = this.open.as_mut() else {
                    return;
                };
                open.wheel += event.delta.pixel_delta(WHEEL_STEP).y;
                while open.wheel >= WHEEL_STEP {
                    open.wheel -= WHEEL_STEP;
                    open.zoom = (open.zoom * ZOOM_STEP).min(MAX_ZOOM);
                }
                while open.wheel <= -WHEEL_STEP {
                    open.wheel += WHEEL_STEP;
                    open.zoom = (open.zoom / ZOOM_STEP).max(1.);
                }
                open.pan = clamp_pan(open.pan, open.zoom, window, cx);
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let Some(open) = this.open.as_mut() else {
                    return;
                };
                let Some(drag) = open.drag else {
                    return;
                };
                open.pan = clamp_pan(
                    drag.pan + (event.position - drag.pointer),
                    open.zoom,
                    window,
                    cx,
                );
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if let Some(open) = this.open.as_mut() {
                        open.drag = None;
                    }
                }),
            )
            .child(
                Shield::new("cover-viewer-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(theme.background.opacity(BACKDROP))
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(window, cx);
                        }),
                    ),
            )
            .child(artwork)
            .child(controls);

        div()
            .id("cover-viewer")
            .role(Role::Dialog)
            .aria_label(title)
            .absolute()
            .inset_0()
            .key_context(COVER_VIEWER_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                cx.stop_propagation();
                this.zoom_by(ZOOM_STEP, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                cx.stop_propagation();
                this.zoom_by(1. / ZOOM_STEP, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                cx.stop_propagation();
                this.zoom_fit(cx);
            }))
            .child(deferred(
                anchored()
                    .position(point(Pixels::ZERO, chrome))
                    .child(frame),
            ))
            .into_any_element()
    }
}
