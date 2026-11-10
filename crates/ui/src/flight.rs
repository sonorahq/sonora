//! Elements that travel: one that layout moves or resizes slides and scales from where it last
//! drew to its new place, and one that appears can start where another was last seen, the way
//! the playing cover flies from the player bar into the fullscreen player.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Element, ElementId, Global, GlobalElementId, InspectorElementId,
    LayerFilter, LayoutId, Pixels, Point, SharedString, Size, SpringConfig, Window, point, px,
};

use crate::motion::{Springs, Sprung};

/// How far a flight may sit from its place, in pixels and in scale, and still count as landed.
const OFFSET_REST: f32 = 0.01;
const SCALE_REST: f32 = 1e-4;
/// How far layout has to move an element, in pixels, before it flies there rather than jumps.
const MOVED: f32 = 0.5;

/// Where a flight is drawn this frame relative to its layout: shifted by `offset` and scaled by
/// `scale` about its centre. Shared with the inner element that paints it.
type Pose = Rc<Cell<(Point<Pixels>, f32)>>;

/// Carries its child from where it last drew to where layout puts it, sliding and scaling it
/// through the compositor so it moves by fractions of a pixel. Keep the id stable across frames.
/// Scaling is uniform, by the change in width, so it suits square things such as a cover.
pub struct Flight {
    id: ElementId,
    inner: Option<AnyElement>,
    pose: Pose,
    seen_as: Option<SharedString>,
    lands_from: Option<SharedString>,
    above: bool,
    scales: bool,
    spring: SpringConfig,
}

/// What a flight remembers between frames.
struct Course {
    /// Its layout bounds last frame.
    drawn: Option<Bounds<Pixels>>,
    /// The window size last frame: a window that resizes moves everything, and nothing flies.
    viewport: Size<Pixels>,
    x: Sprung,
    y: Sprung,
    scale: Sprung,
}

/// Where each named flight was last seen, for another to start from as it appears.
#[derive(Default)]
struct Sightings(HashMap<SharedString, Bounds<Pixels>>);

impl Global for Sightings {}

/// The child itself, painted at the pose its flight chose.
struct Posed {
    child: AnyElement,
    pose: Pose,
}

impl Flight {
    pub fn new(id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        let pose: Pose = Rc::new(Cell::new((Point::default(), 1.)));
        Self {
            id: id.into(),
            inner: Some(
                Posed {
                    child: child.into_any_element(),
                    pose: pose.clone(),
                }
                .into_any_element(),
            ),
            pose,
            seen_as: None,
            lands_from: None,
            above: false,
            scales: true,
            spring: Springs::FLIGHT,
        }
    }

    /// Notes where this element draws under `name` every frame it prepaints, for a flight that
    /// [`lands_from`](Self::lands_from) the same name. An element in a cached view keeps the
    /// place it was last seen at, which is where it still draws.
    pub fn seen_as(mut self, name: impl Into<SharedString>) -> Self {
        self.seen_as = Some(name.into());
        self
    }

    /// Starts this element, the first frame it draws, where the element seen as `name` was last
    /// seen, and flies it to its own place.
    pub fn lands_from(mut self, name: impl Into<SharedString>) -> Self {
        self.lands_from = Some(name.into());
        self
    }

    /// Draws the element above the whole window while it flies, outside the fades and clips of
    /// what holds it, so it stays whole while the screen around it changes.
    pub fn above(mut self) -> Self {
        self.above = true;
        self
    }

    /// Slides the element without scaling it, for text, which keeps its size wherever layout
    /// puts it.
    pub fn unscaled(mut self) -> Self {
        self.scales = false;
        self
    }

    pub fn spring(mut self, spring: SpringConfig) -> Self {
        self.spring = spring;
        self
    }
}

impl Course {
    /// Where the element showed last frame: its layout then, moved and scaled as it was drawn.
    fn shown(&self, last: Bounds<Pixels>) -> Bounds<Pixels> {
        posed(
            last,
            point(px(self.x.position()), px(self.y.position())),
            self.scale.position(),
        )
    }
}

impl Element for Flight {
    type RequestLayoutState = ();
    type PrepaintState = bool;

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
        let layout_id = self
            .inner
            .as_mut()
            .expect("a flight lays out once")
            .request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let viewport = window.viewport_size();
        let sighting = self
            .lands_from
            .as_ref()
            .and_then(|name| cx.try_global::<Sightings>()?.0.get(name).copied());
        let (spring, scales) = (self.spring, self.scales);
        let (offset, scale) = window.with_element_state(
            id.expect("a flight has an id"),
            |course: Option<Course>, window| {
                let mut course = course.unwrap_or_else(|| Course {
                    drawn: None,
                    viewport,
                    x: Sprung::at(0.),
                    y: Sprung::at(0.),
                    scale: Sprung::at(1.),
                });
                let start = match course.drawn {
                    Some(last) if course.viewport == viewport && moved(last, bounds) => {
                        Some(course.shown(last))
                    }
                    Some(_) => None,
                    None => sighting,
                };
                if course.viewport != viewport {
                    course.x.jump(0.);
                    course.y.jump(0.);
                    course.scale.jump(1.);
                }
                if let Some(start) = start.filter(|_| bounds.size.width > Pixels::ZERO) {
                    let from = start.center() - bounds.center();
                    course.x.jump(f32::from(from.x));
                    course.y.jump(f32::from(from.y));
                    if scales {
                        course.scale.jump(start.size.width / bounds.size.width);
                    }
                }
                course.drawn = Some(bounds);
                course.viewport = viewport;
                let x = course.x.follow(0., spring, OFFSET_REST, window, cx);
                let y = course.y.follow(0., spring, OFFSET_REST, window, cx);
                let scale = course.scale.follow(1., spring, SCALE_REST, window, cx);
                ((point(px(x), px(y)), scale), course)
            },
        );
        self.pose.set((offset, scale));
        if let Some(name) = &self.seen_as {
            cx.default_global::<Sightings>()
                .0
                .insert(name.clone(), posed(bounds, offset, scale));
        }

        let flying = offset != Point::default() || scale != 1.;
        let mut inner = self.inner.take().expect("a flight prepaints once");
        if flying && self.above {
            let element_offset = window.element_offset();
            window.defer_draw(inner, element_offset, 0, None);
            return false;
        }
        inner.prepaint(window, cx);
        self.inner = Some(inner);
        true
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        inline: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *inline && let Some(inner) = self.inner.as_mut() {
            inner.paint(window, cx);
        }
    }
}

impl IntoElement for Flight {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Posed {
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
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let (offset, scale) = self.pose.get();
        if offset == Point::default() && scale == 1. {
            self.child.paint(window, cx);
            return;
        }
        let filter = LayerFilter {
            translate: Some(offset),
            scale: Some(scale),
            ..LayerFilter::default()
        };
        window.with_filter(bounds, filter, Pixels::ZERO, |window| {
            self.child.paint(window, cx)
        });
    }
}

impl IntoElement for Posed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// `bounds` shifted by `offset` and scaled by `scale` about their centre.
fn posed(bounds: Bounds<Pixels>, offset: Point<Pixels>, scale: f32) -> Bounds<Pixels> {
    let size = bounds.size.map(|side| side * scale);
    Bounds::centered_at(bounds.center() + offset, size)
}

/// Whether layout moved or resized an element far enough to fly it.
fn moved(last: Bounds<Pixels>, now: Bounds<Pixels>) -> bool {
    let far = |a: Pixels, b: Pixels| f32::from(a - b).abs() > MOVED;
    far(last.origin.x, now.origin.x)
        || far(last.origin.y, now.origin.y)
        || far(last.size.width, now.size.width)
        || far(last.size.height, now.size.height)
}
