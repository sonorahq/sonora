use std::panic::Location;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ElementId, GlobalElementId, InspectorElementId,
    LayoutId, Pixels, Style, StyleRefinement, Window, div, relative, size,
};

type Build = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;
type Visibility = Box<dyn Fn(bool, &mut Window, &mut App)>;

/// A block inside a scrolling region that builds its content only while the content is on
/// screen. Off screen it holds the height the content had when it was last built, so the region
/// keeps its extent. Content that changed height while away is corrected with one redraw on the
/// frame it scrolls back into view.
pub struct Lazy {
    id: ElementId,
    style: StyleRefinement,
    build: Option<Build>,
    revision: u64,
    visibility: Option<Visibility>,
}

/// What a lazy block remembers between frames: how tall its content stood and whether it was
/// on screen, which decides whether the next frame lays the content out in place.
#[derive(Clone, Copy)]
struct Seen {
    height: Pixels,
    shown: bool,
    revision: u64,
}

impl Lazy {
    #[track_caller]
    pub fn new(
        id: impl Into<ElementId>,
        build: impl FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            build: Some(Box::new(build)),
            revision: 0,
            visibility: None,
        }
    }

    /// Remeasures even an off-screen block when its content or layout changes. Keep the
    /// revision stable during scrolling and animation so the cached height can be reused.
    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    /// Reports whether the block is visible after layout, including off-screen measurements.
    /// A block with background work can stop it when outside the scrolling region.
    pub fn on_visibility(
        mut self,
        visibility: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.visibility = Some(Box::new(visibility));
        self
    }
}

impl Styled for Lazy {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Element for Lazy {
    type RequestLayoutState = Option<AnyElement>;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let seen = id.and_then(|id| seen(id, window));
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.flex_shrink = 0.;
        style.refine(&self.style);

        match seen {
            Some(Seen {
                height,
                shown,
                revision,
            }) if !shown && height > Pixels::ZERO && revision == self.revision => {
                style.size.height = height.into();
                (window.request_layout(style, [], cx), None)
            }
            _ => {
                let Some(build) = self.build.take() else {
                    return (window.request_layout(style, [], cx), None);
                };
                let mut content = build(window, cx);
                let child = content.request_layout(window, cx);
                (window.request_layout(style, [child], cx), Some(content))
            }
        }
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        laid: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let shown = window.content_mask().bounds.intersects(&bounds);
        if let Some(visibility) = &self.visibility {
            visibility(shown, window, cx);
        }

        if let Some(mut content) = laid.take() {
            remember(id, bounds.size.height, shown, self.revision, window);
            if !shown {
                return None;
            }
            content.prepaint(window, cx);
            return Some(content);
        }
        if !shown {
            remember(id, bounds.size.height, false, self.revision, window);
            return None;
        }
        let build = self.build.take()?;

        // Laid out on its own here, the content is held to the block's width, as the block
        // holds it when it lays out in place. Left to itself it would shrink to fit its widest
        // line, pulling every control at a row's end in next to its label.
        let mut content = div()
            .w(bounds.size.width)
            .child(build(window, cx))
            .into_any_element();
        let measured = content.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        content.prepaint_at(bounds.origin, window, cx);
        remember(id, measured.height, true, self.revision, window);
        if measured.height != bounds.size.height {
            window.refresh();
        }
        Some(content)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _laid: &mut Self::RequestLayoutState,
        content: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(content) = content {
            content.paint(window, cx);
        }
    }
}

fn seen(id: &GlobalElementId, window: &mut Window) -> Option<Seen> {
    window.with_element_state::<Option<Seen>, _>(id, |seen, _| {
        let seen = seen.flatten();
        (seen, seen)
    })
}

fn remember(
    id: Option<&GlobalElementId>,
    height: Pixels,
    shown: bool,
    revision: u64,
    window: &mut Window,
) {
    let Some(id) = id else {
        return;
    };
    window.with_element_state::<Option<Seen>, _>(id, |_, _| {
        (
            (),
            Some(Seen {
                height,
                shown,
                revision,
            }),
        )
    });
}
