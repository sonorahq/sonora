use std::mem;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Pixels, Window,
};

use crate::Theme;

/// Draws its child in another theme than the window's, for a subtree that keeps its own colours
/// while the rest of the window already wears different ones. Whatever reads `cx.theme()` while
/// the child lays out, prepaints and paints sees `theme`. Deferred elements and tooltips draw
/// after the wrapper lets go, so they see the window's theme.
pub struct Themed {
    theme: Theme,
    child: AnyElement,
}

impl Themed {
    pub fn new(theme: Theme, child: impl IntoElement) -> Self {
        Self {
            theme,
            child: child.into_any_element(),
        }
    }
}

impl Element for Themed {
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
        let child = &mut self.child;
        let layout = wearing(&self.theme, cx, |cx| child.request_layout(window, cx));
        (layout, ())
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
        let child = &mut self.child;
        wearing(&self.theme, cx, |cx| child.prepaint(window, cx));
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
        let child = &mut self.child;
        wearing(&self.theme, cx, |cx| child.paint(window, cx));
    }
}

impl IntoElement for Themed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Runs `f` with `theme` as the global theme and puts the window's own back afterwards.
fn wearing<R>(theme: &Theme, cx: &mut App, f: impl FnOnce(&mut App) -> R) -> R {
    let worn = mem::replace(cx.global_mut::<Theme>(), *theme);
    let result = f(cx);
    *cx.global_mut::<Theme>() = worn;
    result
}
