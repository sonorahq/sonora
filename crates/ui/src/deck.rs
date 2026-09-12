use std::ops::Range;
use std::panic::Location;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ElementId, GlobalElementId, InspectorElementId,
    LayoutId, Pixels, Style, StyleRefinement, Window, div, point, relative, size,
};

use crate::table::Viewport;

type Draw = Box<dyn Fn(usize, &mut Window, &mut App) -> AnyElement>;
type Measure = Box<dyn Fn(Pixels, &mut Window, &mut App)>;

/// A run of rows of known heights inside a scrolling region, of which only the ones in view are
/// built. The element takes the extent of the whole run, so the region scrolls and its
/// scrollbar measures as if every row were there, and it finds the rows on screen from its own
/// bounds and the region's clip while it is laid out, so it is never a frame behind the scroll
/// and needs nothing measured by its caller. `across` turns the run sideways for a rail.
pub struct Deck {
    id: ElementId,
    style: StyleRefinement,
    rows: Vec<Pixels>,
    gap: Pixels,
    across: bool,
    draw: Option<Draw>,
    measure: Option<Measure>,
}

impl Deck {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            rows: Vec::new(),
            gap: Pixels::ZERO,
            across: false,
            draw: None,
            measure: None,
        }
    }

    pub fn across(mut self) -> Self {
        self.across = true;
        self
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = Pixels>) -> Self {
        self.rows = rows.into_iter().collect();
        self
    }

    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap;
        self
    }

    /// Builds the row at an index. Asked only for the rows in view, every frame they are.
    pub fn draw(
        mut self,
        draw: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.draw = Some(Box::new(draw));
        self
    }

    /// Reports where the run's leading edge landed, in window coordinates, once it is laid
    /// out. A caller that aims a scroll at one of the rows needs that to know how far down the
    /// region the run begins.
    pub fn on_measure(mut self, measure: impl Fn(Pixels, &mut Window, &mut App) + 'static) -> Self {
        self.measure = Some(Box::new(measure));
        self
    }

    pub fn tops(rows: &[Pixels], gap: Pixels) -> Vec<Pixels> {
        tops(rows, gap)
    }

    pub fn at(rows: &[Pixels], gap: Pixels, offset: Pixels) -> (usize, Pixels) {
        if rows.is_empty() {
            return (0, Pixels::ZERO);
        }

        let tops = tops(rows, gap);
        let index = passed(&tops, rows, offset).min(rows.len() - 1);
        (index, (offset - tops[index]).max(Pixels::ZERO))
    }
}

impl Styled for Deck {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Deck {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Deck {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        let reach = extent(&self.rows, self.gap);
        let mut style = Style::default();
        match self.across {
            true => {
                style.size.width = reach.into();
                style.size.height = relative(1.).into();
            }
            false => {
                style.size.width = relative(1.).into();
                style.size.height = reach.into();
            }
        }
        style.flex_shrink = 0.;
        style.refine(&self.style);

        (window.request_layout(style, [], cx), ())
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
        let across = self.across;
        let lead = match across {
            true => bounds.origin.x,
            false => bounds.origin.y,
        };
        if let Some(measure) = &self.measure {
            measure(lead, window, cx);
        }
        let Some(draw) = &self.draw else {
            return Vec::new();
        };

        // The region clips to its own bounds, so the mask is the part of the run on screen.
        let view = window.content_mask().bounds;
        let viewport = match across {
            true => Viewport {
                top: view.origin.x - lead,
                height: view.size.width,
            },
            false => Viewport {
                top: view.origin.y - lead,
                height: view.size.height,
            },
        };
        let tops = tops(&self.rows, self.gap);
        let shown = span(&tops, &self.rows, viewport);

        let mut built = Vec::with_capacity(shown.len());
        for index in shown {
            let (width, height) = match across {
                true => (self.rows[index], bounds.size.height),
                false => (bounds.size.width, self.rows[index]),
            };
            let mut element = div()
                .overflow_hidden()
                .w(width)
                .h(height)
                .child(draw(index, window, cx))
                .into_any_element();
            element.layout_as_root(
                size(
                    AvailableSpace::Definite(width),
                    AvailableSpace::Definite(height),
                ),
                window,
                cx,
            );
            let origin = bounds.origin
                + match across {
                    true => point(tops[index], Pixels::ZERO),
                    false => point(Pixels::ZERO, tops[index]),
                };
            element.prepaint_at(origin, window, cx);
            built.push(element);
        }
        built
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        built: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for element in built {
            element.paint(window, cx);
        }
    }
}

fn tops(rows: &[Pixels], gap: Pixels) -> Vec<Pixels> {
    let mut top = Pixels::ZERO;
    rows.iter()
        .map(|height| {
            let start = top;
            top += *height + gap;
            start
        })
        .collect()
}

fn extent(rows: &[Pixels], gap: Pixels) -> Pixels {
    match rows.is_empty() {
        true => Pixels::ZERO,
        false => {
            rows.iter()
                .fold(Pixels::ZERO, |total, height| total + *height)
                + gap * (rows.len() - 1) as f32
        }
    }
}

/// The rows that overlap the viewport, `top` being how far into the run the viewport starts.
fn span(tops: &[Pixels], rows: &[Pixels], viewport: Viewport) -> Range<usize> {
    if tops.is_empty() {
        return 0..0;
    }

    let bottom = viewport.top + viewport.height;
    let first = passed(tops, rows, viewport.top);
    let last = tops.partition_point(|top| *top < bottom);

    first..last.max(first)
}

fn passed(tops: &[Pixels], rows: &[Pixels], top: Pixels) -> usize {
    let mut low = 0;
    let mut high = tops.len();
    while low < high {
        let mid = (low + high) / 2;
        match tops[mid] + rows[mid] <= top {
            true => low = mid + 1,
            false => high = mid,
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{Deck, extent, span, tops};
    use crate::table::Viewport;

    fn heights(count: usize, height: f32) -> Vec<gpui::Pixels> {
        (0..count).map(|_| px(height)).collect()
    }

    #[test]
    fn tops_stack_rows_with_gaps() {
        let rows = heights(3, 100.);

        assert_eq!(tops(&rows, px(10.)), [px(0.), px(110.), px(220.)]);
    }

    #[test]
    fn extent_covers_every_row_and_the_gaps_between() {
        assert_eq!(extent(&heights(3, 100.), px(10.)), px(320.));
        assert_eq!(extent(&heights(1, 100.), px(10.)), px(100.));
        assert_eq!(extent(&[], px(10.)), px(0.));
    }

    #[test]
    fn span_covers_the_visible_rows() {
        let rows = heights(20, 100.);
        let tops = tops(&rows, px(0.));

        let shown = span(
            &tops,
            &rows,
            Viewport {
                top: px(500.),
                height: px(200.),
            },
        );

        assert!(shown.contains(&5));
        assert!(shown.contains(&6));
        assert!(!shown.contains(&9));
    }

    #[test]
    fn span_starts_at_the_top_of_a_short_deck() {
        let rows = heights(2, 100.);
        let tops = tops(&rows, px(0.));

        assert_eq!(
            span(
                &tops,
                &rows,
                Viewport {
                    top: px(0.),
                    height: px(800.),
                }
            ),
            0..2
        );
    }

    #[test]
    fn at_finds_the_row_under_an_offset() {
        let rows = heights(4, 100.);

        assert_eq!(Deck::at(&rows, px(10.), px(0.)), (0, px(0.)));
        assert_eq!(Deck::at(&rows, px(10.), px(150.)), (1, px(40.)));
        assert_eq!(Deck::at(&rows, px(10.), px(105.)), (1, px(0.)));
        assert_eq!(Deck::at(&rows, px(10.), px(9000.)), (3, px(8670.)));
        assert_eq!(Deck::at(&[], px(10.), px(50.)), (0, px(0.)));
    }

    #[test]
    fn span_is_empty_without_rows() {
        assert_eq!(
            span(
                &[],
                &[],
                Viewport {
                    top: px(0.),
                    height: px(800.),
                }
            ),
            0..0
        );
    }
}
