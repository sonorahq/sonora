use std::cell::Cell;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, DragMoveEvent, ElementId, Empty, Interactivity, Pixels, Stateful,
    StyleRefinement, Window, deferred, div, px,
};

use crate::metrics::snapped;

const GRIP: Pixels = px(12.);
const GRIP_INSET: Pixels = px(-6.);
const GRIP_PRIORITY: usize = 0;

type Resize = Box<dyn Fn(&Pixels, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}

struct Grab {
    panel: ElementId,
    width: Pixels,
    origin: Cell<Pixels>,
}

#[derive(IntoElement)]
pub struct Panel {
    base: Stateful<Div>,
    key: ElementId,
    side: Side,
    width: Pixels,
    min: Pixels,
    max: Pixels,
    reach: Option<Pixels>,
    fill: bool,
    /// How far above the panel's foot the resize grip ends.
    grip_bottom: Pixels,
    resize: Option<Resize>,
    children: Vec<AnyElement>,
}

impl Panel {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, side: Side, width: Pixels) -> Self {
        let key: ElementId = id.into();
        Self {
            base: div().id(key.clone()),
            key,
            side,
            width,
            min: Pixels::ZERO,
            max: Pixels::MAX,
            reach: None,
            fill: false,
            grip_bottom: Pixels::ZERO,
            resize: None,
            children: Vec::new(),
        }
    }

    pub fn limits(mut self, min: Pixels, max: Pixels) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn reach(mut self, reach: Pixels) -> Self {
        self.reach = Some(reach);
        self
    }

    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Ends the resize grip `bottom` above the panel's foot, so chrome floating over the panel's
    /// lower edge keeps its own pointer instead of the grip's, which is drawn over everything.
    pub fn grip_above(mut self, bottom: Pixels) -> Self {
        self.grip_bottom = bottom;
        self
    }

    pub fn on_resize(mut self, resize: impl Fn(&Pixels, &mut Window, &mut App) + 'static) -> Self {
        self.resize = Some(Box::new(resize));
        self
    }
}

impl Styled for Panel {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Panel {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Panel {}

impl ParentElement for Panel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Panel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            key,
            side,
            width,
            min,
            max,
            reach,
            fill,
            grip_bottom,
            resize,
            children,
        } = self;
        let overrides = std::mem::take(base.style());
        let width = width.clamp(min, max);
        let handle = (!fill).then_some(resize).flatten().map(|resize| {
            grip(key, side, width, min, reach.unwrap_or(max), resize).bottom(grip_bottom)
        });
        let handle = handle.map(|handle| deferred(handle).with_priority(GRIP_PRIORITY));

        let mut panel = base
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .map(|panel| match side {
                Side::Left => panel.border_r_1(),
                Side::Right => panel.border_l_1(),
            })
            .map(|panel| match fill {
                true => panel.flex_1().min_w_0(),
                false => panel.flex_none().w(width),
            });
        panel.style().refine(&overrides);

        panel.children(children).children(handle)
    }
}

fn grip(
    key: ElementId,
    side: Side,
    width: Pixels,
    min: Pixels,
    max: Pixels,
    resize: Resize,
) -> Stateful<Div> {
    let dragged = key.clone();

    div()
        .id("panel-grip")
        .occlude()
        .absolute()
        .top_0()
        .w(GRIP)
        .cursor_col_resize()
        .map(|handle| match side {
            Side::Left => handle.right(GRIP_INSET),
            Side::Right => handle.left(GRIP_INSET),
        })
        .on_drag_move(move |event: &DragMoveEvent<Grab>, window, cx| {
            let grab = event.drag(cx);
            if grab.panel != dragged {
                return;
            }
            let (start, origin) = (grab.width, grab.origin.get());
            let travel = event.event.position.x - origin;
            let dragged = match side {
                Side::Left => start + travel,
                Side::Right => start - travel,
            };
            let width = snapped(dragged.clamp(min, max), window);
            resize(&width, window, cx);
        })
        .on_drag(
            Grab {
                panel: key,
                width,
                origin: Cell::new(Pixels::ZERO),
            },
            |grab, _, window, cx| {
                grab.origin.set(window.mouse_position().x);
                cx.new(|_| Empty)
            },
        )
}
