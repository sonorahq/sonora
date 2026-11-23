use gpui::prelude::*;
use gpui::{
    App, BorderStyle, Bounds, Corners, Edges, Hsla, PathBuilder, Pixels, Point, Window, canvas,
    fill, point, px, quad, size,
};
use ui::ActiveTheme as _;

use super::Chrome;

/// The width of the sheet's edge, matching the sidebars' borders.
const EDGE: Pixels = px(1.);
/// How many straight pieces the cut outside a corner's curve is made of.
const SEGMENTS: usize = 16;

/// Where the page meets each sidebar: how far in from its own side the sidebar reaches, and
/// how round the corner is there. A side without a sidebar has a width of zero.
#[derive(Clone, Copy)]
struct Side {
    width: Pixels,
    radius: Pixels,
    edge: Hsla,
    /// What shows outside the rounded corner, or nothing on a see-through window, where the
    /// sidebar has no surface of its own.
    backing: Option<Hsla>,
}

/// Draws the page's edges against the chrome: the line under the title bar and the border of
/// each sidebar beside the page, joined by rounded corners. The sidebars and the title bar
/// leave their own borders out for it. Lay it over the row that holds the sidebars and the
/// page, after both, so an opaque window's corners cut whatever the page paints there.
pub(crate) fn sheet_edge(cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let chrome = Chrome::get(cx);
    let corners = Chrome::sheet_corners(cx);
    let opaque = !theme.transparent;
    let left = Side {
        width: chrome.sidebar_left,
        radius: corners.top_left,
        edge: theme.sidebar_border,
        backing: opaque.then_some(theme.sidebar),
    };
    let right = Side {
        width: chrome.sidebar_right,
        radius: corners.top_right,
        edge: theme.border,
        backing: opaque.then_some(theme.background),
    };
    let line = theme.title_bar_border;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint(bounds, left, right, line, window),
    )
    .absolute()
    .top(-EDGE)
    .left_0()
    .right_0()
    .bottom_0()
}

/// The corners a page's own fill takes where it meets the sheet's corners: the sheet's
/// radius less its edge, so the fill's curve runs along the inside of the edge's.
pub(crate) fn inner_corners(cx: &App) -> Corners<Pixels> {
    let corners = Chrome::sheet_corners(cx);
    Corners {
        top_left: (corners.top_left - EDGE).max(Pixels::ZERO),
        top_right: (corners.top_right - EDGE).max(Pixels::ZERO),
        ..Corners::default()
    }
}

/// Paints the top line, then each sidebar's border and the corner it makes with the line.
fn paint(bounds: Bounds<Pixels>, left: Side, right: Side, line: Hsla, window: &mut Window) {
    let top = bounds.top();
    // The outer side of each sidebar's border column, where the corner's square would sit.
    let left_outer = (left.width > Pixels::ZERO).then(|| bounds.left() + left.width - EDGE);
    let right_outer = (right.width > Pixels::ZERO).then(|| bounds.right() - right.width + EDGE);
    let (outer_left, outer_right) = (bounds.left(), bounds.right());
    let start = left_outer.map_or(outer_left, |x| x + left.radius * 2.);
    let end = right_outer.map_or(outer_right, |x| x - right.radius * 2.);
    if end > start {
        window.paint_quad(fill(
            Bounds::from_corners(point(start, top), point(end, top + EDGE)),
            line,
        ));
    }
    if let Some(x) = left_outer {
        side(point(x, top), 1., left, bounds.bottom(), window);
    }
    if let Some(x) = right_outer {
        side(point(x, top), -1., right, bounds.bottom(), window);
    }
}

/// Draws one sidebar's border down to `bottom`, rounding into the top line at `outer`, the
/// corner's square. `toward` is 1 for a corner that opens rightwards and -1 for leftwards.
///
/// The corner is a bordered quad, so its curve is antialiased the way every other rounded
/// border is. A quad rounds at most half its size, so it spans twice the radius and also draws
/// that much of each straight edge, which the lines here leave to it.
fn side(outer: Point<Pixels>, toward: f32, side: Side, bottom: Pixels, window: &mut Window) {
    let reach = side.radius * 2.;
    let column = match toward > 0. {
        true => outer.x,
        false => outer.x - EDGE,
    };
    let below = outer.y + reach.max(EDGE);
    if bottom > below {
        window.paint_quad(fill(
            Bounds::from_corners(point(column, below), point(column + EDGE, bottom)),
            side.edge,
        ));
    }
    if side.radius <= Pixels::ZERO {
        window.paint_quad(fill(
            Bounds::from_corners(point(column, outer.y), point(column + EDGE, below)),
            side.edge,
        ));
        return;
    }

    let radius = side.radius;
    let centre = point(outer.x + radius * toward, outer.y + radius);
    if let Some(backing) = side.backing {
        // What lies outside the curve, cut along the middle of the border so the border's
        // own antialiased edges cover the cut's.
        let along = radius - EDGE / 2.;
        let mut outline = vec![point(outer.x, centre.y), outer, point(centre.x, outer.y)];
        outline.extend((0..=SEGMENTS).rev().map(|step| {
            let angle = std::f32::consts::FRAC_PI_2 * step as f32 / SEGMENTS as f32;
            point(
                centre.x - along * angle.cos() * toward,
                centre.y - along * angle.sin(),
            )
        }));
        let mut cut = PathBuilder::fill();
        cut.add_polygon(&outline, true);
        if let Ok(path) = cut.build() {
            window.paint_path(path, backing);
        }
    }

    let (left, corners, edges) = match toward > 0. {
        true => (
            outer.x,
            Corners {
                top_left: radius,
                ..Corners::default()
            },
            Edges {
                top: EDGE,
                left: EDGE,
                ..Edges::default()
            },
        ),
        false => (
            outer.x - reach,
            Corners {
                top_right: radius,
                ..Corners::default()
            },
            Edges {
                top: EDGE,
                right: EDGE,
                ..Edges::default()
            },
        ),
    };
    window.paint_quad(quad(
        Bounds::new(point(left, outer.y), size(reach, reach)),
        corners,
        // Clear, but in the edge's own colour, so the border's inner antialiasing fades the
        // edge out instead of darkening it towards black, which made the curve look thinner
        // and dimmer than the straight edges it joins.
        side.edge.opacity(0.),
        edges,
        side.edge,
        BorderStyle::Solid,
    ));
}
