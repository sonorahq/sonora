use std::cell::Cell;
use std::panic::Location;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    Anchor, AnyElement, App, Bounds, Context, DispatchPhase, Display, ElementId, GlobalElementId,
    InspectorElementId, LayoutId, MouseMoveEvent, Pixels, Position, SharedString, Style, Window,
    anchored, canvas, div, point, px,
};

use crate::metrics::Text;
use crate::theme::ActiveTheme as _;

const MARGIN: Pixels = px(8.);
const OFFSET: Pixels = px(6.);

/// Where a tooltip sits relative to the element that owns it.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Perch {
    /// Centred below the element, or above it when the window has no room below.
    #[default]
    Below,
    /// Centred above the element, or below it when the window has no room above.
    Above,
    /// Below and to the left of the pointer, shown the moment the pointer arrives and moved
    /// along with it.
    Follow,
}

/// The bubble a hovered element shows. It is placed against the element's bounds as they were
/// when the pointer arrived, so it lands in the same spot wherever the pointer entered.
pub struct Tooltip {
    text: SharedString,
    raw: bool,
    perch: Perch,
    /// The owning element in window coordinates. A following tooltip ignores it.
    host: Bounds<Pixels>,
}

/// Attaches a tooltip to an element with the show delay its perch calls for. A tooltip perched
/// below or above adds an invisible child that records where the element sits.
pub trait Tipped: Sized {
    /// Shows the Fluent string `key` on hover.
    fn tip(self, key: impl Into<SharedString>, perch: Perch) -> Self;
    /// Shows `text` on hover as it is, without a Fluent lookup.
    fn tip_label(self, text: impl Into<SharedString>, perch: Perch) -> Self;
}

/// Places a tooltip bubble centred against its host, on the preferred side when the window has
/// room there and on the other side otherwise, and keeps it inside the window margin.
struct Beside {
    host: Bounds<Pixels>,
    above: bool,
    child: AnyElement,
}

impl<E: StatefulInteractiveElement + ParentElement> Tipped for E {
    fn tip(self, key: impl Into<SharedString>, perch: Perch) -> Self {
        attach(self, key.into(), false, perch)
    }

    fn tip_label(self, text: impl Into<SharedString>, perch: Perch) -> Self {
        attach(self, text.into(), true, perch)
    }
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let follow = self.perch == Perch::Follow;
        let bubble = div()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_size(theme.text(Text::Small))
            .text_color(theme.popover_foreground)
            .child(match self.raw {
                true => self.text.clone(),
                false => i18n::lookup(&self.text, None),
            })
            .when(follow, |this| this.child(tracker()));

        match follow {
            true => anchored()
                .position(window.mouse_position() + point(-OFFSET, OFFSET))
                .anchor(Anchor::TopRight)
                .snap_to_window_with_margin(MARGIN)
                .child(bubble)
                .into_any_element(),
            false => Beside {
                host: self.host,
                above: self.perch == Perch::Above,
                child: bubble.into_any_element(),
            }
            .into_any_element(),
        }
    }
}

impl IntoElement for Beside {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Beside {
    type RequestLayoutState = LayoutId;
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
        let child = self.child.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            display: Display::Flex,
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let size = window.layout_bounds(*child).size;
        let viewport = window.viewport_size();
        let margin = MARGIN + window.client_inset().unwrap_or_default();

        let below = self.host.bottom() + OFFSET;
        let above = self.host.top() - OFFSET - size.height;
        let fits_below = below + size.height <= viewport.height - margin;
        let fits_above = above >= margin;
        let up = match self.above {
            true => fits_above || !fits_below,
            false => fits_above && !fits_below,
        };
        let y = match up {
            true => above,
            false => below,
        };
        let x = self.host.center().x - size.width / 2.;
        let origin = point(
            inside(x, size.width, viewport.width, margin).round(),
            inside(y, size.height, viewport.height, margin).round(),
        );

        window.with_element_offset(origin - bounds.origin, |window| {
            self.child.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// Wires a tooltip into `element`. A perched tooltip also lays a recorder over the element so
/// the tooltip knows where the element stood, and a following one drops the hover delay.
fn attach<E>(element: E, text: SharedString, raw: bool, perch: Perch) -> E
where
    E: StatefulInteractiveElement + ParentElement,
{
    match perch {
        Perch::Follow => element
            .tooltip_show_delay(Duration::ZERO)
            .tooltip(move |_, cx| {
                let text = text.clone();
                cx.new(|_| Tooltip {
                    text,
                    raw,
                    perch,
                    host: Bounds::default(),
                })
                .into()
            }),
        Perch::Below | Perch::Above => {
            let host = Rc::new(Cell::new(None));
            element
                .child(recorder(host.clone()))
                .tooltip(move |window, cx| {
                    let text = text.clone();
                    let host = host.get().unwrap_or_else(|| {
                        Bounds::new(window.mouse_position(), Default::default())
                    });
                    cx.new(|_| Tooltip {
                        text,
                        raw,
                        perch,
                        host,
                    })
                    .into()
                })
        }
    }
}

/// Covers its parent without painting anything and stores the parent's bounds each frame.
fn recorder(host: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| host.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .inset_0()
}

/// Moves a span starting at `start` so it stays `margin` inside `0..limit`, keeping the start
/// edge visible when the span is too long to fit.
fn inside(start: Pixels, len: Pixels, limit: Pixels, margin: Pixels) -> Pixels {
    start.min(limit - margin - len).max(margin)
}

/// Redraws the window on every pointer move so a following tooltip renders at the new position.
fn tracker() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            window.on_mouse_event(|_: &MouseMoveEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble {
                    window.refresh();
                }
            })
        },
    )
    .absolute()
}
