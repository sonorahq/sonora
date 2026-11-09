//! A tiny read-only drawing of a response curve, as a sound picker shows before each name.

use rune::{El, Shape, canvas};

use crate::color::Hsla;
use crate::response_graph::{ResponseAxes, ResponsePoint, projected};
use crate::theme::Theme;

/// How big a sketch stands unless the caller sizes it, small enough to sit before a name.
const WIDTH: f32 = 28.;
const HEIGHT: f32 = 14.;
/// How thick the curve is drawn, thin enough to keep its shape at this size.
const STROKE: f32 = 1.5;

/// A response curve drawn small and still. The caller samples it in ascending frequency order and
/// picks the axes, which should be the same for every sketch shown together so their shapes
/// compare.
pub struct ResponseSketch {
    axes: ResponseAxes,
    points: Vec<ResponsePoint>,
    color: Option<Hsla>,
    size: (f32, f32),
}

impl ResponseSketch {
    pub fn new(axes: ResponseAxes, points: impl IntoIterator<Item = ResponsePoint>) -> Self {
        Self {
            axes,
            points: points.into_iter().collect(),
            color: None,
            size: (WIDTH, HEIGHT),
        }
    }

    /// Strokes the curve in `color` rather than the muted foreground.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = (width, height);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let (width, height) = self.size;
        let color = self.color.unwrap_or(theme.muted_foreground);
        // the stroke stays inside the box, so a curve at the axis's edge is not cut
        let plot = [0., STROKE / 2., width, (height - STROKE).max(0.)];
        let curve = projected(&self.points, self.axes, plot);
        let shape = match curve.len() >= 2 {
            true => Shape::new().stroke(curve, STROKE, color.color()),
            false => Shape::new(),
        };
        canvas(shape).size(width, height).a11y_hidden()
    }
}
