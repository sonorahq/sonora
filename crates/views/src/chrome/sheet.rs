use gpui::prelude::*;
use gpui::{
    App, BorderStyle, Bounds, Corners, Edges, Hsla, PathBuilder, Pixels, Point, Window, canvas,
    point, quad,
};
use ui::ActiveTheme as _;

use super::Chrome;

/// The width of the sheet's edge, matching the sidebars' borders.
const EDGE: Pixels = gpui::px(1.);
/// How many straight pieces the cut outside a corner's curve is made of.
const SEGMENTS: usize = 16;

/// Draws the page's frame as a sheet lying on the chrome: a rounded edge all the way round and,
/// on an opaque window, the chrome's colour outside each rounded corner so whatever the page
/// paints there is cut along the curve. Lay it over the page as its last child, filling it.
pub(crate) fn sheet_frame(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let radius = Chrome::sheet_radius(cx);
    let edge = theme.sidebar_border;
    let backing = (!theme.transparent).then_some(theme.sidebar);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint(bounds, radius, edge, backing, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .bottom_0()
}

/// The corners a page's own fill takes where it meets the sheet's top corners: the sheet's
/// radius less its edge, so the fill's curve runs along the inside of the edge's.
pub(crate) fn inner_corners(cx: &App) -> Corners<Pixels> {
    let inner = (Chrome::sheet_radius(cx) - EDGE).max(Pixels::ZERO);
    Corners {
        top_left: inner,
        top_right: inner,
        ..Corners::default()
    }
}

/// Cuts each corner with `backing` when there is one, then draws the rounded edge.
fn paint(
    bounds: Bounds<Pixels>,
    radius: Pixels,
    edge: Hsla,
    backing: Option<Hsla>,
    window: &mut Window,
) {
    if let Some(backing) = backing.filter(|_| radius > Pixels::ZERO) {
        for (corner, toward) in [
            (bounds.origin, point(1f32, 1.)),
            (bounds.top_right(), point(-1., 1.)),
            (bounds.bottom_left(), point(1., -1.)),
            (bounds.bottom_right(), point(-1., -1.)),
        ] {
            cut(corner, toward, radius, backing, window);
        }
    }
    window.paint_quad(quad(
        bounds,
        Corners::all(radius),
        // Clear, but in the edge's own colour, so the border's inner antialiasing fades the edge
        // out instead of darkening it towards black, which made the curve look thinner and
        // dimmer than the straight edges it joins.
        edge.opacity(0.),
        Edges::all(EDGE),
        edge,
        BorderStyle::Solid,
    ));
}

/// Fills what lies outside the curve of the corner at `corner`, whose sheet opens towards
/// `toward` (1 or -1 on each axis). The cut runs along the middle of the edge, so the edge's own
/// antialiasing covers the cut's.
fn cut(
    corner: Point<Pixels>,
    toward: Point<f32>,
    radius: Pixels,
    backing: Hsla,
    window: &mut Window,
) {
    let centre = point(corner.x + radius * toward.x, corner.y + radius * toward.y);
    let along = radius - EDGE / 2.;
    let mut outline = vec![point(corner.x, centre.y), corner, point(centre.x, corner.y)];
    outline.extend((0..=SEGMENTS).rev().map(|step| {
        let angle = std::f32::consts::FRAC_PI_2 * step as f32 / SEGMENTS as f32;
        point(
            centre.x - along * angle.cos() * toward.x,
            centre.y - along * angle.sin() * toward.y,
        )
    }));
    let mut path = PathBuilder::fill();
    path.add_polygon(&outline, true);
    if let Ok(path) = path.build() {
        window.paint_path(path, backing);
    }
}
