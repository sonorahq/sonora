use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Refineable as _, Size, Style,
    StyleRefinement, Styled, Window,
};

/// Lays its child out on its own, from the device pixel its own box lands on, the way a cached
/// view lays out what it holds. Laid out inline, the same content rounds from wherever the
/// fractions of its position fall, so a copy that has to match a cached view to the pixel, as
/// the narrow fullscreen player's bar matches the player bar, stands in one of these. Its own box
/// takes the size its style gives, since the child is not measured.
pub(crate) struct Rooted {
    style: StyleRefinement,
    child: AnyElement,
}

impl Rooted {
    pub(crate) fn new(child: impl IntoElement) -> Self {
        Self {
            style: StyleRefinement::default(),
            child: child.into_any_element(),
        }
    }
}

impl Styled for Rooted {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for Rooted {
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
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child
            .layout_as_root(Size::<AvailableSpace>::from(bounds.size), window, cx);
        self.child.prepaint_at(bounds.origin, window, cx);
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
        self.child.paint(window, cx);
    }
}
