//! The full-size view a page hero's cover opens, for an album, a playlist or an artist: the
//! cover flies out of the page into the middle of the window, the provider's original fades in
//! over it when it lands, the wheel and the buttons zoom it, a drag pans it, and Escape or a
//! click on the backdrop flies it back.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Context, DispatchPhase, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Rems, Render, ScrollEvent, SharedString, Size, WeakEntity, Window,
    anchored, canvas, deferred, div, point, px, rems, size,
};
use input::{COVER_VIEWER_CONTEXT, ToggleFullscreen, ZoomIn, ZoomOut, ZoomReset};
use ui::{
    ActiveTheme as _, Artwork, Button, Dismiss, Flight, Motion, Shield, Springs, Sprung, Zoom,
    css_ease_out, snapped,
};

use music::is_local_id;
use state::{SessionEvent, Sonora};

use crate::chrome::Timeline;
use crate::shared::hero::{COVER_FLIGHT, PageHero};

/// How dark the window goes behind the artwork.
const BACKDROP: f32 = 0.85;
/// The widest zoom as a multiple of the fit, enough to read a sticker or a signature on a
/// high-resolution scan.
const MAX_ZOOM: f32 = 4.;
/// One zoom key, button or wheel notch multiplies the zoom by this much.
const ZOOM_STEP: f32 = 1.5;
/// The wheel travel of one zoom step.
const WHEEL_STEP: Pixels = px(20.);
/// How close the backdrop fade comes to its end before it counts as done.
const FADE_REST: f32 = 1e-3;
/// How much smaller than their size the controls start as the viewer opens, zooming up the way
/// the fullscreen player does.
const CHROME_ZOOM: f32 = 0.03;
/// How close the drawn zoom and pan come to the ones asked for before they count as there.
const ZOOM_REST: f32 = 1e-3;
const PAN_REST: f32 = 0.05;
/// The padding and the border of the pill that holds the controls.
const CONTROLS_PAD: Rems = rems(0.25);
const CONTROLS_BORDER: Pixels = px(1.);

/// A drag in progress: where the pointer grabbed the artwork and where the artwork was.
#[derive(Clone, Copy)]
struct Drag {
    pointer: Point<Pixels>,
    pan: Point<Pixels>,
}

struct Open {
    /// The id of the page item the cover belongs to, which the viewer closes once the page shows
    /// another.
    owner: Option<String>,
    title: SharedString,
    large: SharedString,
    /// Whether the page draws the cover as a circle, as an artist's portrait, so the flight
    /// eases its corners from round.
    round: bool,
    full: Option<SharedString>,
    /// When the large cover and the original were first decoded at the viewer's size, which
    /// starts each one fading in over the copy below it.
    large_at: Option<Instant>,
    full_at: Option<Instant>,
    zoom: f32,
    /// How far the artwork's centre sits from the centre of its fit.
    pan: Point<Pixels>,
    /// The zoom and pan drawn this frame, springing after `zoom` and `pan`. The artwork is
    /// laid out at them rather than scaled, so the stage clips it at its real size.
    drawn_zoom: Sprung,
    drawn_x: Sprung,
    drawn_y: Sprung,
    drag: Option<Drag>,
    /// Wheel travel gathered towards the next zoom step, so a touchpad's stream of small
    /// deltas zooms at the same pace a wheel does.
    wheel: Pixels,
    /// Whether the cover has yet to draw its first frame, before its flight reports a scale.
    fresh: bool,
    /// Whether the cover has finished flying in from the page, after which its corners stay
    /// at the viewer's radius through every zoom.
    landed: bool,
    /// Whether the cover is flying back onto the page, with the backdrop fading out under it.
    closing: bool,
}

/// The area under the title bar the viewer lays the artwork out in, and the square the
/// artwork fits into unzoomed: clear of the window's inset and of the controls at the bottom.
#[derive(Clone, Copy)]
struct Stage {
    /// The window y where the stage starts.
    top: Pixels,
    size: Size<Pixels>,
    /// The side of the unzoomed artwork.
    fit: Pixels,
    /// The centre of the unzoomed artwork, from the stage's top left corner.
    center: Point<Pixels>,
    /// The top of the controls, from the stage's top.
    controls: Pixels,
    /// The seek line the controls were centred on, if the player bar had painted one.
    line: Option<Pixels>,
}

impl Stage {
    fn measure(window: &Window, cx: &App) -> Self {
        let metrics = cx.theme().metrics;
        let top = snapped(metrics.title_bar, window);
        let viewport = window.viewport_size();
        let size = size(viewport.width, (viewport.height - top).max(Pixels::ZERO));
        let inset = metrics.inset;
        let height = metrics.control_small
            + CONTROLS_PAD.to_pixels(window.rem_size()) * 2.
            + CONTROLS_BORDER * 2.;
        // The controls sit over the player bar's seek line, or one inset above the bottom edge
        // before the bar has painted.
        let line = Timeline::line(cx);
        let middle = line
            .unwrap_or(viewport.height - inset - height / 2.)
            .min(viewport.height - height / 2.);
        let controls = snapped(middle - height / 2. - top, window).max(Pixels::ZERO);
        // The fit runs from one inset below the title bar to one inset above the controls.
        let fit = (size.width - inset * 2.)
            .min(controls - inset * 2.)
            .max(Pixels::ZERO);
        let center = point(size.width / 2., controls / 2.);
        Self {
            top,
            size,
            fit,
            center,
            controls,
            line,
        }
    }

    /// Keeps the artwork's edge from crossing the stage's: panning is free along an axis once
    /// the zoomed cover overflows the stage there, and pinned to the fit while it does not.
    fn clamp(&self, pan: Point<Pixels>, zoom: f32) -> Point<Pixels> {
        let side = self.fit * zoom;
        let axis = |pan: Pixels, length: Pixels, center: Pixels| match side >= length {
            true => pan.clamp(length - side / 2. - center, side / 2. - center),
            false => Pixels::ZERO,
        };
        point(
            axis(pan.x, self.size.width, self.center.x),
            axis(pan.y, self.size.height, self.center.y),
        )
    }

    /// The pan that keeps the point of the artwork under `anchor`, in window coordinates, in
    /// place while the zoom goes `from` one value `to` another.
    fn anchored(
        &self,
        pan: Point<Pixels>,
        from: f32,
        to: f32,
        anchor: Point<Pixels>,
    ) -> Point<Pixels> {
        let reach = point(anchor.x, anchor.y - self.top) - self.center;
        reach - (reach - pan) * (to / from)
    }
}

/// The cover viewer above an album, playlist or artist page, scoped to the item the page shows
/// rather than to whatever happens to be playing.
pub(crate) struct CoverViewer {
    open: Option<Open>,
    /// How far the backdrop and the controls have faded in, from 0 to 1.
    shown: Sprung,
    /// How far the cover's flight scales it, for corners that ease between the page's radius
    /// and the viewer's.
    flight: Rc<Cell<Zoom>>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    grab: bool,
}

impl CoverViewer {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let session = Sonora::global(cx).session.clone();
        cx.subscribe(&session, |this, _, event, cx| {
            // These sign events clear and reload a provider page under a new account, possibly
            // within one frame, so the page's stale check can miss it. A local item's art does
            // not depend on the account.
            if matches!(event, SessionEvent::SignedOut | SessionEvent::SignedIn)
                && this
                    .open
                    .as_ref()
                    .is_some_and(|open| !open.owner.as_deref().is_some_and(is_local_id))
            {
                this.force_close(None, cx);
            }
        })
        .detach();
        Self {
            open: None,
            shown: Sprung::at(0.),
            flight: Rc::default(),
            focus: cx.focus_handle(),
            restore: None,
            grab: false,
        }
    }

    /// Opens the viewer on the `large` cover of the page item `owner`, with the provider's
    /// original swapped in once it loads. A provider that cannot name an original for the url
    /// keeps the large cover, and a local item's cover already is the original. The cover flies
    /// in from the page hero's, which is a circle when `round`.
    pub(crate) fn open(
        &mut self,
        owner: Option<String>,
        title: SharedString,
        large: String,
        round: bool,
        cx: &mut Context<Self>,
    ) {
        let session = Sonora::global(cx).session.read(cx);
        let client = match owner.as_deref() {
            Some(id) if is_local_id(id) => session.local_client(),
            _ => session.client(),
        };
        let full = client
            .and_then(|client| client.cover_original(&large))
            .filter(|full| full != &large)
            .map(SharedString::from);
        let large = SharedString::from(large);
        if self.open.as_ref().is_none_or(|open| !open.closing) {
            self.shown = Sprung::at(0.);
        }
        self.open = Some(Open {
            owner,
            title,
            large,
            round,
            full,
            large_at: None,
            full_at: None,
            zoom: 1.,
            pan: Point::default(),
            drawn_zoom: Sprung::at(1.),
            drawn_x: Sprung::at(0.),
            drawn_y: Sprung::at(0.),
            drag: None,
            wheel: Pixels::ZERO,
            fresh: true,
            landed: false,
            closing: false,
        });
        self.grab = true;
        cx.notify();
    }

    /// Whether the viewer shows a cover, including while it flies back onto the page.
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Drops the viewer at once when the page under it no longer shows the item `owner` with the
    /// cover `cover`, after navigation or a provider switch that signs the cover url anew.
    pub(crate) fn close_if_stale(
        &mut self,
        owner: Option<&str>,
        cover: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stale = self.open.as_ref().is_some_and(|open| {
            open.owner.as_deref() != owner || Some(open.large.as_ref()) != cover
        });
        if stale {
            self.force_close(Some(window), cx);
        }
    }

    /// Drops the viewer at once, without flying the cover back, for a page whose item changed
    /// under it. Focus goes back where it was when a `window` is at hand.
    pub(crate) fn force_close(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.open.is_none() {
            return;
        }
        self.open = None;
        self.grab = false;
        if let (Some(window), Some(focus)) = (window, self.restore.take()) {
            window.focus(&focus, cx);
        }
        self.restore = None;
        cx.notify();
    }

    /// Flies the cover back onto the page while the backdrop fades out, and hands focus back.
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut().filter(|open| !open.closing) else {
            return;
        };
        open.closing = true;
        open.drag = None;
        // The cover lands back on the page from its fit, whole, with the flight taking over
        // from wherever the zoom had it, so the stage never clips what flies.
        open.zoom = 1.;
        open.pan = Point::default();
        open.drawn_zoom.jump(1.);
        open.drawn_x.jump(0.);
        open.drawn_y.jump(0.);
        self.grab = false;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Sets the zoom, keeping the point under `anchor` where it is, or the middle of the fit
    /// without one.
    fn zoom_to(
        &mut self,
        zoom: f32,
        anchor: Option<Point<Pixels>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.open.as_mut().filter(|open| !open.closing) else {
            return;
        };
        let stage = Stage::measure(window, cx);
        let zoom = zoom.clamp(1., MAX_ZOOM);
        let pan = match anchor {
            Some(anchor) => stage.anchored(open.pan, open.zoom, zoom, anchor),
            None => open.pan * (zoom / open.zoom),
        };
        open.zoom = zoom;
        open.pan = stage.clamp(pan, zoom);
        cx.notify();
    }

    fn zoom_by(&mut self, step: f32, window: &Window, cx: &mut Context<Self>) {
        let Some(zoom) = self.open.as_ref().map(|open| open.zoom) else {
            return;
        };
        self.zoom_to(zoom * step, None, window, cx);
    }

    /// Turns wheel travel over the stage into zoom steps about the pointer.
    fn wheel(&mut self, event: &ScrollEvent, window: &Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut().filter(|open| !open.closing) else {
            return;
        };
        open.wheel += event.delta.pixel_delta(WHEEL_STEP).y;
        let mut zoom = open.zoom;
        while open.wheel >= WHEEL_STEP {
            open.wheel -= WHEEL_STEP;
            zoom *= ZOOM_STEP;
        }
        while open.wheel <= -WHEEL_STEP {
            open.wheel += WHEEL_STEP;
            zoom /= ZOOM_STEP;
        }
        self.zoom_to(zoom, Some(event.position), window, cx);
    }

    fn drag_to(&mut self, pointer: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some(drag) = open.drag else {
            return;
        };
        let stage = Stage::measure(window, cx);
        open.pan = stage.clamp(drag.pan + (pointer - drag.pointer), open.zoom);
        open.drawn_x.jump(f32::from(open.pan.x));
        open.drawn_y.jump(f32::from(open.pan.y));
        cx.notify();
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut().filter(|open| open.drag.is_some()) {
            open.drag = None;
            cx.notify();
        }
    }

    /// Listens to the pointer across the whole window while the viewer is open, so a drag
    /// keeps panning off the artwork and the wheel zooms wherever it turns. It also draws the
    /// viewer again when the seek line its controls sit over has moved since `line`, since the
    /// player bar paints after the page has laid the viewer out.
    fn tracker(viewer: WeakEntity<Self>, line: Option<Pixels>) -> impl IntoElement {
        canvas(
            move |_, window, cx| {
                if Timeline::line(cx) != line {
                    window.request_animation_frame();
                }
            },
            move |_, _, window, _| {
                let moved = viewer.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        moved
                            .update(cx, |this, cx| this.drag_to(event.position, window, cx))
                            .ok();
                    }
                });
                let released = viewer.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        released.update(cx, |this, cx| this.release(cx)).ok();
                    }
                });
                let wheeled = viewer;
                window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        cx.stop_propagation();
                        wheeled
                            .update(cx, |this, cx| this.wheel(event, window, cx))
                            .ok();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

impl Render for CoverViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.grab {
            self.restore = window.focused(cx);
            window.focus(&self.focus, cx);
            self.grab = false;
        }
        let Some(closing) = self.open.as_ref().map(|open| open.closing) else {
            return div().into_any_element();
        };
        let target = match closing {
            true => 0.,
            false => 1.,
        };
        // The backdrop and the controls fade in at the pace the fullscreen player opens and fade
        // out at the pace its cover flies back.
        let spring = match closing {
            true => Springs::ARTWORK_FLIGHT,
            false => Springs::shell(),
        };
        let shown = self.shown.follow(target, spring, FADE_REST, window, cx);
        let stage = Stage::measure(window, cx);
        if let Some(open) = self.open.as_mut().filter(|open| open.fresh) {
            // The flight reports its scale only once it has drawn, so the first frame starts
            // the corners from the hero cover's size, where the cover takes off.
            open.fresh = false;
            let side = stage.fit * open.zoom;
            let from = match side > Pixels::ZERO {
                true => cx.theme().metrics.cover / side,
                false => 1.,
            };
            self.flight.set(Zoom {
                now: from,
                from,
                to: 1.,
            });
        }
        let flight = self.flight.get();
        let resting = flight.now == flight.to;
        if closing && shown == 0. && resting {
            // A notify made while drawing is lost, so the viewer lets go on the next frame,
            // where the page hears it and draws its own cover again. This frame still draws the
            // landed cover, so no frame shows neither.
            cx.on_next_frame(window, |this, _, cx| {
                if this.open.as_ref().is_some_and(|open| open.closing) {
                    this.open = None;
                    cx.notify();
                }
            });
            window.request_animation_frame();
        }
        let tracker = (!closing).then(|| Self::tracker(cx.entity().downgrade(), stage.line));
        let Some(open) = self.open.as_mut() else {
            return div().into_any_element();
        };
        if !closing && resting && flight.to == 1. {
            open.landed = true;
        }

        let theme = *cx.theme();
        let title = open.title.clone();
        let zoom = open.zoom;
        let drawn = open
            .drawn_zoom
            .follow(zoom, Springs::RESIZE, ZOOM_REST, window, cx);
        let pan = point(
            px(open
                .drawn_x
                .follow(f32::from(open.pan.x), Springs::RESIZE, PAN_REST, window, cx)),
            px(open
                .drawn_y
                .follow(f32::from(open.pan.y), Springs::RESIZE, PAN_REST, window, cx)),
        );
        let side = stage.fit * drawn;
        let origin = stage.center + pan - point(side / 2., side / 2.);
        let page = match open.round {
            true => theme.metrics.cover / 2.,
            false => PageHero::cover_radius(&theme),
        };
        let radius = match (open.landed, closing) {
            (true, false) => theme.radius,
            (false, false) => flight.radius(page, theme.radius),
            (_, true) => flight.radius(theme.radius, page),
        };
        let dragging = open.drag.is_some();
        // The page's own decode of the cover shows at once, stretched, and the sharper copies
        // fade in over it as each one is decoded for the viewer's size.
        let large = open.large.clone();
        let full = open.full.clone();
        let large_in = fade_in(
            &mut open.large_at,
            Artwork::decoded(&large, side, window, cx),
            window,
            cx,
        );
        let full_in = full.as_ref().map_or(0., |full| {
            fade_in(
                &mut open.full_at,
                Artwork::decoded(full, side, window, cx),
                window,
                cx,
            )
        });
        let layer = |url: SharedString, opacity: f32| {
            div().absolute().top_0().left_0().opacity(opacity).child(
                Artwork::new(Some(url))
                    .size(side)
                    .contain()
                    .corner_radius(radius)
                    .overlay(),
            )
        };

        let grab = cx.listener(|this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            let Some(open) = this.open.as_mut().filter(|open| !open.closing) else {
                return;
            };
            open.drag = Some(Drag {
                pointer: event.position,
                pan: open.pan,
            });
            cx.notify();
        });
        let artwork = div()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .size(side)
            .when(!closing, |this| {
                this.when(zoom > 1., |this| this.cursor_grab())
                    .when(dragging, |this| this.cursor_grabbing())
                    .on_mouse_down(MouseButton::Left, grab)
            })
            .child(
                Artwork::new(Some(large.clone()))
                    .size(side)
                    .sampled_at(theme.metrics.cover)
                    .contain()
                    .corner_radius(radius),
            )
            .child(layer(large, large_in))
            .children(full.map(|full| layer(full, full_in)));
        // The cover flies out of the page hero's as the viewer opens and back into it as the
        // viewer closes, the way the playing cover flies into the fullscreen player. Zoom and
        // pan move the layout itself, since the flight would scale a copy the stage had already
        // clipped. Closing snaps the layout back to the fit, which the flight picks up from
        // wherever the zoom had the cover. The flight composites inside the stage, so a cover
        // flying to or from a hero scrolled under the title bar slides beneath it.
        let artwork = Flight::new("cover-viewer-art", artwork)
            .transfer_spring(Springs::ARTWORK_FLIGHT)
            .lands_from(COVER_FLIGHT)
            .heads_to(closing.then_some(COVER_FLIGHT))
            .follows(closing)
            .reports(self.flight.clone());

        let button = |id: &'static str, icon: &'static str, tooltip: &'static str| {
            Button::new(id).icon(icon).small().tooltip(tooltip)
        };
        let controls = div()
            .absolute()
            .left_0()
            .right_0()
            .top(stage.controls)
            .flex()
            .justify_center()
            .opacity(shown)
            .when(!closing && shown < 1., |this| {
                this.layer_scale(1. - CHROME_ZOOM * (1. - shown))
            })
            .child(
                div()
                    .when(!closing, |this| this.occlude())
                    .flex()
                    .items_center()
                    .gap_1()
                    .p(CONTROLS_PAD)
                    .rounded_full()
                    .border(CONTROLS_BORDER)
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_md()
                    .child(
                        button("cover-zoom-out", "icons/minus.svg", "cover-zoom-out")
                            .disabled(zoom <= 1.)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom_by(1. / ZOOM_STEP, window, cx)
                            })),
                    )
                    .child(
                        button("cover-zoom-in", "icons/plus.svg", "cover-zoom-in")
                            .disabled(zoom >= MAX_ZOOM)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom_by(ZOOM_STEP, window, cx)
                            })),
                    )
                    .child(
                        button("cover-zoom-fit", "icons/maximize.svg", "cover-zoom-fit")
                            .disabled(zoom <= 1.)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom_to(1., None, window, cx)
                            })),
                    )
                    .child(
                        button("cover-close", "icons/x.svg", "common-dismiss")
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    ),
            );

        // The backdrop dims the whole window, title bar included. Once closing, the viewer only
        // paints: the page under it scrolls and clicks while the cover is still in the air.
        let viewport = window.viewport_size();
        let frame = div()
            .relative()
            .w(viewport.width)
            .h(viewport.height)
            .child(match closing {
                false => Shield::new("cover-viewer-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(theme.background.opacity(BACKDROP * shown))
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(window, cx);
                        }),
                    )
                    .into_any_element(),
                true => div()
                    .absolute()
                    .inset_0()
                    .bg(theme.background.opacity(BACKDROP * shown))
                    .into_any_element(),
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(stage.top)
                    .w(stage.size.width)
                    .h(stage.size.height)
                    .overflow_hidden()
                    .child(artwork)
                    .child(controls)
                    .children(tracker),
            );

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
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| {
                cx.stop_propagation();
                this.zoom_to(1., None, window, cx);
            }))
            // The fullscreen player would open under the viewer, so its key does nothing here.
            .on_action(|_: &ToggleFullscreen, _, cx| cx.stop_propagation())
            .child(deferred(anchored().position(Point::default()).child(frame)))
            .into_any_element()
    }
}

/// How far a sharper copy of the cover has faded in: nothing until `ready` first holds, then
/// rising over the slow motion span from that moment.
fn fade_in(at: &mut Option<Instant>, ready: bool, window: &Window, cx: &App) -> f32 {
    if ready && at.is_none() {
        *at = Some(Instant::now());
    }
    let Some(at) = *at else {
        return 0.;
    };
    if cx.reduce_motion() {
        return 1.;
    }
    let span = Motion::Slow.span().as_secs_f32().max(f32::EPSILON);
    let progress = (at.elapsed().as_secs_f32() / span).min(1.);
    if progress < 1. {
        window.request_animation_frame();
    }
    css_ease_out(progress)
}
