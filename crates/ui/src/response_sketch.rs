use gpui::prelude::*;
use gpui::{App, Bounds, Hsla, Pixels, StyleRefinement, Window, canvas, div, point, px, size};

use crate::response_graph::{ResponseAxes, ResponsePoint, line};
use crate::theme::ActiveTheme as _;

/// How big a sketch stands unless the caller sizes it, small enough to sit before a name.
const WIDTH: Pixels = px(28.);
const HEIGHT: Pixels = px(14.);
/// How thick the curve is drawn, thin enough to keep its shape at this size.
const STROKE: Pixels = px(1.5);

/// A tiny read-only drawing of a response curve, as a sound picker shows before each name. The
/// caller samples the curve in ascending frequency order and picks the axes, which should be the
/// same for every sketch shown together so their shapes compare.
#[derive(IntoElement)]
pub struct ResponseSketch {
    style: StyleRefinement,
    axes: ResponseAxes,
    points: Vec<ResponsePoint>,
    color: Option<Hsla>,
}

impl ResponseSketch {
    pub fn new(axes: ResponseAxes, points: impl IntoIterator<Item = ResponsePoint>) -> Self {
        Self {
            style: StyleRefinement::default(),
            axes,
            points: points.into_iter().collect(),
            color: None,
        }
    }

    /// Strokes the curve in `color` rather than the muted foreground.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl Styled for ResponseSketch {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ResponseSketch {
    fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.color.unwrap_or_else(|| cx.theme().muted_foreground);
        let overrides = std::mem::take(&mut self.style);
        let Self { axes, points, .. } = self;

        let mut sketch = div().flex_none().w(WIDTH).h(HEIGHT).child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    // the stroke stays inside the box, so a curve at the axis's edge is not cut
                    let inset = STROKE / 2.;
                    let plot = Bounds::new(
                        point(bounds.left(), bounds.top() + inset),
                        size(bounds.size.width, bounds.size.height - STROKE),
                    );
                    let curve: Vec<_> = points
                        .iter()
                        .filter(|value| value.frequency_hz.is_finite() && value.gain_db.is_finite())
                        .map(|value| axes.project(*value, plot))
                        .collect();
                    line(&curve, STROKE, color, window);
                },
            )
            .size_full(),
        );
        sketch.style().refine(&overrides);
        sketch
    }
}
