//! Floating surfaces that fade out after their owner stops drawing them. A menu or a dialog
//! leaves the tree the moment it closes, so nothing is left to animate. A [`Linger`] names
//! what the surface paints, and the host from [`lingers`] paints that last frame again while
//! it fades, once the surface is gone.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Deferred, Element, ElementId, Global, GlobalElementId,
    InspectorElementId, LayerFilter, LayoutId, Pixels, Position, SpanId, Style, Window, WindowId,
    deferred, px,
};

use crate::motion::{ENTRANCE_BLUR, ENTRANCE_ZOOM, ease_out_expo, entrance_span};

/// How far past its bounds a leaving surface may paint, which is room for its drop shadow.
const REACH: Pixels = px(32.);

/// How a surface looks on its way out.
#[derive(Clone, Copy)]
enum Leave {
    /// It fades where it stands, for a scrim that fills the window.
    Fade,
    /// It fades, shrinks and blurs by as much as [`Rising`](crate::Rising) undid on the way in.
    Recede,
}

/// A surface the host watches, and how far through leaving it is.
struct Leaving {
    leave: Leave,
    /// When the surface was first missing from a frame, `None` while its owner draws it.
    since: Option<Instant>,
    /// How much of its opacity the last frame kept.
    shown: f32,
}

/// Every surface that may linger, by the window it is in and the span it paints under.
#[derive(Default)]
struct Lingers {
    watched: HashMap<(WindowId, SpanId), Leaving>,
    /// The windows that draw a host. A surface anywhere else is never watched, since nothing
    /// would ever let go of it.
    hosted: HashSet<WindowId>,
}

impl Global for Lingers {}

/// Wraps a floating surface so it fades out once its owner stops drawing it, rather than
/// vanishing. The owner changes nothing: it drops the surface as before. Keep the id stable
/// across frames. Nothing lingers unless the window draws [`lingers`].
pub struct Linger {
    id: ElementId,
    child: AnyElement,
    leave: Leave,
    turn: u64,
}

impl Linger {
    /// A surface that leaves the way [`Rising`](crate::Rising) brought it in, played backwards
    /// over the same span and curve.
    pub fn new(id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            child: child.into_any_element(),
            leave: Leave::Recede,
            turn: 0,
        }
    }

    /// Leaves by fading alone, for a surface too large to shrink without showing its edges.
    pub fn fading(mut self) -> Self {
        self.leave = Leave::Fade;
        self
    }

    /// Tells one showing of the surface from the next. A surface drawn with another `turn`
    /// counts as a new one, so the one before it still fades out where it stood.
    pub fn turn(mut self, turn: u64) -> Self {
        self.turn = turn;
        self
    }
}

impl Element for Linger {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(id) = id else {
            self.child.paint(window, cx);
            return;
        };
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        self.turn.hash(&mut hasher);
        let span = hasher.finish();
        let here = window.window_handle().window_id();
        let lingers = cx.default_global::<Lingers>();
        if lingers.hosted.contains(&here) {
            lingers
                .watched
                .entry((here, span))
                .or_insert(Leaving {
                    leave: self.leave,
                    since: None,
                    shown: 1.,
                })
                .leave = self.leave;
        }
        window.with_paint_span(span, bounds, |window| self.child.paint(window, cx));
    }
}

impl IntoElement for Linger {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Paints the surfaces of this window that are on their way out, over everything else. The
/// root view draws one, anywhere in its tree. It takes no room and answers no input.
pub fn lingers() -> Deferred {
    deferred(Host).with_priority(usize::MAX)
}

/// The element behind [`lingers`]. It paints last, so by then every surface still alive has
/// painted its span into the frame and the ones missing are the ones that left.
struct Host;

impl Element for Host {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let here = window.window_handle().window_id();
        let still = cx.reduce_motion();
        let span = entrance_span().as_secs_f32().max(f32::EPSILON);
        let lingers = cx.default_global::<Lingers>();
        lingers.hosted.insert(here);
        let mut watched = std::mem::take(&mut lingers.watched);
        watched.retain(|(at, id), leaving| {
            if *at != here {
                return true;
            }
            if window.span_painted(*id) {
                leaving.since = None;
                leaving.shown = 1.;
                return true;
            }
            let Some(bounds) = window.rendered_span(*id).filter(|_| !still) else {
                return false;
            };
            let since = *leaving.since.get_or_insert_with(Instant::now);
            let progress = since.elapsed().as_secs_f32() / span;
            if progress >= 1. {
                return false;
            }
            let gone = ease_out_expo(progress);
            let shown = 1. - gone;
            // The last frame already carries the fade it was painted with, so this one only
            // adds the difference.
            let fade = (shown / leaving.shown.max(f32::EPSILON)).min(1.);
            leaving.shown = shown;
            match leaving.leave {
                Leave::Fade => {
                    window.replay_span(*id, fade);
                }
                Leave::Recede => {
                    let filter = LayerFilter {
                        scale: Some(1. - ENTRANCE_ZOOM * gone),
                        blur: Some(ENTRANCE_BLUR * gone),
                        ..LayerFilter::default()
                    };
                    window.with_filter(bounds, filter, REACH, |window| {
                        window.replay_span(*id, fade);
                    });
                }
            }
            window.request_animation_frame();
            true
        });
        cx.default_global::<Lingers>().watched = watched;
    }
}

impl IntoElement for Host {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
