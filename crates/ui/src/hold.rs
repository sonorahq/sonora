use std::cell::RefCell;
use std::panic::Location;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Axis, Bounds, ElementId, Global, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, ScrollHandle, Size, Window, px,
};

/// How much a scrolling region's width has to change before its content is held in place.
const RESIZED: Pixels = px(0.5);

/// Keeps what sits at the leading edge of a scrolling region in place when the region changes
/// width and its content reflows. Each frame it notes which item or row the edge falls in and how
/// far into it, and on the frame the width changes it scrolls that spot back to the edge, in the
/// same frame and from the new layout. It wraps the element that scrolls and adds no layout of its
/// own. Only `held` items and `Deck`s built with `hold` under it are anchors, and the innermost
/// one under the edge wins. While inactive it keeps taking notes but leaves the scroll alone.
pub struct Hold {
    id: ElementId,
    axis: Axis,
    scroll: ScrollHandle,
    active: bool,
    reflowed: bool,
    /// The anchor held in place of the spot at the edge.
    keep: Option<ElementId>,
    child: AnyElement,
    frame: Option<Rc<RefCell<Frame>>>,
}

/// An item of a held region, anchored by its `key`, which has to stay the same across frames and
/// be unique among the anchors of the region. It adds no layout of its own.
pub struct Held {
    key: ElementId,
    child: AnyElement,
    layout: Option<LayoutId>,
}

/// The rows of a `Deck` that anchors its hold. `keys` names the first item each row starts with,
/// for a grid whose rows regroup their items when it reflows. Without keys a row is its own item.
#[derive(Clone)]
pub struct Run {
    pub(crate) rows: Vec<Pixels>,
    pub(crate) gap: Pixels,
    pub(crate) keys: Option<Rc<[usize]>>,
}

/// What a hold remembers from the frame before: the width it was laid out at and where its edge
/// fell, innermost anchor last.
#[derive(Default)]
struct Memory {
    width: Option<Pixels>,
    marks: Vec<Mark>,
    kept: Option<Kept>,
}

/// Where the kept anchor was laid out: how far into the content its leading edge sat, leaving
/// the scroll out, so a scroll made since that frame is kept rather than undone.
#[derive(Clone)]
struct Kept {
    id: ElementId,
    start: Pixels,
}

/// Where the edge fell in one anchor: which item, and how far into it as a share of its length.
#[derive(Clone)]
struct Mark {
    id: ElementId,
    key: usize,
    share: f32,
}

/// What a hold gathers from its content during one frame: the anchors laid out under it, and
/// the marks they leave while they are prepainted against the edge.
struct Frame {
    axis: Axis,
    edge: Pixels,
    anchors: Vec<Anchor>,
    marks: Vec<Mark>,
}

struct Anchor {
    id: ElementId,
    layout: LayoutId,
    run: Option<Run>,
}

/// The holds an element is being laid out or prepainted under, innermost last.
#[derive(Default)]
struct Holds(Vec<Rc<RefCell<Frame>>>);

impl Global for Holds {}

impl Hold {
    /// Holds the element that scrolls along `axis` through `scroll`. `id` is that element's id,
    /// which keys what the hold remembers between frames.
    pub fn new(
        id: impl Into<ElementId>,
        axis: Axis,
        scroll: &ScrollHandle,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            axis,
            scroll: scroll.clone(),
            active: true,
            reflowed: false,
            keep: None,
            child: child.into_any_element(),
            frame: None,
        }
    }

    /// Leaves the scroll alone while `active` is false, for a region that places itself, such as
    /// lyrics following the line being sung.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Holds this frame even though the width stayed, for content that reflows a frame after
    /// the region resized, such as text the caller wraps to the width it measured last frame.
    pub fn reflowed(mut self, reflowed: bool) -> Self {
        self.reflowed = reflowed;
        self
    }

    /// Holds the `held` item `key` in place of the spot at the edge, for a region that keeps one
    /// item in view, such as lyrics on the line being sung. Only what the reflow moves is taken
    /// back, so a glide under way carries on. Without it, or while that item was not laid out the
    /// frame before, the edge is held.
    pub fn keep(mut self, key: Option<ElementId>) -> Self {
        self.keep = key;
        self
    }
}

impl Run {
    /// The row that holds the item `key`, or `None` without rows.
    fn index_of(&self, key: usize) -> Option<usize> {
        let last = self.rows.len().checked_sub(1)?;
        match &self.keys {
            Some(keys) => Some(
                keys.partition_point(|first| *first <= key)
                    .saturating_sub(1),
            ),
            None => Some(key.min(last)),
        }
        .map(|index| index.min(last))
    }

    fn key_of(&self, index: usize) -> usize {
        match &self.keys {
            Some(keys) => keys.get(index).copied().unwrap_or(index),
            None => index,
        }
    }
}

/// Anchors `child` to the hold it is laid out under by `key`.
pub fn held(key: impl Into<ElementId>, child: impl IntoElement) -> Held {
    Held {
        key: key.into(),
        child: child.into_any_element(),
        layout: None,
    }
}

/// Registers a deck laid out under a hold along its axis as an anchor of that hold.
pub(crate) fn register_run(id: &ElementId, layout: LayoutId, run: Run, across: bool, cx: &mut App) {
    if let Some(frame) = nearest(Some(axis_of(across)), cx) {
        frame.borrow_mut().anchors.push(Anchor {
            id: id.clone(),
            layout,
            run: Some(run),
        });
    }
}

/// Notes where the edge of the hold a deck is prepainted under falls among its rows, when it
/// falls in them at all. `lead` is where the deck's run begins on screen.
pub(crate) fn mark_run(id: &ElementId, run: &Run, lead: Pixels, across: bool, cx: &mut App) {
    let Some(frame) = nearest(Some(axis_of(across)), cx) else {
        return;
    };
    let mut frame = frame.borrow_mut();
    let depth = frame.edge - lead;
    let mut top = Pixels::ZERO;
    for (index, row) in run.rows.iter().enumerate() {
        if depth >= top && depth < top + *row {
            frame.marks.push(Mark {
                id: id.clone(),
                key: run.key_of(index),
                share: share(depth - top, *row),
            });
            return;
        }
        top += *row + run.gap;
    }
}

impl IntoElement for Hold {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for Hold {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let frame = Rc::new(RefCell::new(Frame {
            axis: self.axis,
            edge: Pixels::ZERO,
            anchors: Vec::new(),
            marks: Vec::new(),
        }));
        cx.default_global::<Holds>().0.push(frame.clone());
        let layout = self.child.request_layout(window, cx);
        cx.default_global::<Holds>().0.pop();
        self.frame = Some(frame);

        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(frame) = self.frame.clone() else {
            self.child.prepaint(window, cx);
            return;
        };
        let axis = self.axis;
        let global = window.with_global_id(self.id.clone(), |global, _| global.clone());
        window.with_element_state::<Memory, _>(&global, |memory, window| {
            let mut memory = memory.unwrap_or_default();
            let resized = memory
                .width
                .is_some_and(|width| self.reflowed || (width - bounds.size.width).abs() >= RESIZED);
            if resized
                && self.active
                && let Some(depth) = depth(
                    &frame.borrow(),
                    &memory,
                    self.keep.as_ref(),
                    bounds,
                    &self.scroll,
                    window,
                )
            {
                let offset = self.scroll.offset();
                self.scroll.set_offset(match axis {
                    Axis::Horizontal => Point::new(-depth, offset.y),
                    Axis::Vertical => Point::new(offset.x, -depth),
                });
            }

            {
                let mut frame = frame.borrow_mut();
                frame.edge = along(bounds.origin, axis);
                frame.marks.clear();
            }
            cx.default_global::<Holds>().0.push(frame.clone());
            self.child.prepaint(window, cx);
            cx.default_global::<Holds>().0.pop();

            memory.width = Some(bounds.size.width);
            memory.marks = std::mem::take(&mut frame.borrow_mut().marks);
            memory.kept = self.keep.clone().and_then(|id| {
                let start = laid_start(&frame.borrow(), &id, bounds, window)?;
                Some(Kept { id, start })
            });
            ((), memory)
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

impl IntoElement for Held {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for Held {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout = self.child.request_layout(window, cx);
        self.layout = Some(layout);
        if let Some(frame) = nearest(None, cx) {
            frame.borrow_mut().anchors.push(Anchor {
                id: self.key.clone(),
                layout,
                run: None,
            });
        }

        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(frame) = nearest(None, cx) {
            let mut frame = frame.borrow_mut();
            let axis = frame.axis;
            let depth = frame.edge - along(bounds.origin, axis);
            let length = length(bounds.size, axis);
            if depth >= Pixels::ZERO && depth < length {
                frame.marks.push(Mark {
                    id: self.key.clone(),
                    key: 0,
                    share: share(depth, length),
                });
            }
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// How far into the content the edge has to be scrolled to stand where the last frame's marks
/// say it stood, using this frame's layout. The innermost mark whose anchor is laid out under
/// this hold again wins, and `None` means none is.
fn place(
    frame: &Frame,
    marks: &[Mark],
    bounds: Bounds<Pixels>,
    window: &mut Window,
) -> Option<Pixels> {
    let axis = frame.axis;
    marks.iter().rev().find_map(|mark| {
        let anchor = frame.anchors.iter().find(|anchor| anchor.id == mark.id)?;
        let laid = window.layout_bounds(anchor.layout);
        let start = along(laid.origin, axis) - along(bounds.origin, axis);
        let (top, length) = match &anchor.run {
            Some(run) => {
                let index = run.index_of(mark.key)?;
                let top = run.rows[..index]
                    .iter()
                    .fold(Pixels::ZERO, |top, row| top + *row + run.gap);
                (top, run.rows[index])
            }
            None => (Pixels::ZERO, length(laid.size, axis)),
        };
        Some((start + top + length * mark.share).max(Pixels::ZERO))
    })
}

/// How far into the content the edge has to be scrolled to hold what the last frame noted, the
/// kept anchor when it is still the one to keep and is laid out again, and the spot at the edge
/// otherwise.
fn depth(
    frame: &Frame,
    memory: &Memory,
    keep: Option<&ElementId>,
    bounds: Bounds<Pixels>,
    scroll: &ScrollHandle,
    window: &mut Window,
) -> Option<Pixels> {
    let kept = memory.kept.as_ref().filter(|kept| Some(&kept.id) == keep);
    let kept = kept.and_then(|kept| {
        let start = laid_start(frame, &kept.id, bounds, window)?;
        let depth = -along(scroll.offset(), frame.axis);
        Some((depth + start - kept.start).max(Pixels::ZERO))
    });
    kept.or_else(|| place(frame, &memory.marks, bounds, window))
}

/// How far into the content the anchor `id` is laid out this frame, leaving the scroll out, or
/// `None` when it is not laid out under this hold.
fn laid_start(
    frame: &Frame,
    id: &ElementId,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) -> Option<Pixels> {
    let anchor = frame.anchors.iter().find(|anchor| &anchor.id == id)?;
    let laid = window.layout_bounds(anchor.layout);
    Some(along(laid.origin, frame.axis) - along(bounds.origin, frame.axis))
}

/// The innermost hold being laid out or prepainted, along `axis` when one is given.
fn nearest(axis: Option<Axis>, cx: &mut App) -> Option<Rc<RefCell<Frame>>> {
    cx.default_global::<Holds>()
        .0
        .iter()
        .rev()
        .find(|frame| axis.is_none_or(|axis| frame.borrow().axis == axis))
        .cloned()
}

fn axis_of(across: bool) -> Axis {
    match across {
        true => Axis::Horizontal,
        false => Axis::Vertical,
    }
}

fn along(point: Point<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Horizontal => point.x,
        Axis::Vertical => point.y,
    }
}

fn length(size: Size<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Horizontal => size.width,
        Axis::Vertical => size.height,
    }
}

fn share(into: Pixels, length: Pixels) -> f32 {
    match length > Pixels::ZERO {
        true => (into / length).clamp(0., 1.),
        false => 0.,
    }
}
