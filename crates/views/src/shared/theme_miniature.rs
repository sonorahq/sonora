//! The theme miniature: the Sonora window drawn as a few soft, flat shapes in a theme's
//! colours, the picture on a theme card and in the dialog that offers a linked theme.

use gpui::prelude::*;
use gpui::{Div, Pixels, Size, div, px, size};
use ui::Theme;

/// How wide a theme card stands.
pub(crate) const CARD: Pixels = px(150.);
/// How tall a theme card's picture stands, its border included.
pub(crate) const THUMB: Pixels = px(96.);
/// How thick the card's border is, which the miniature sits inside.
const EDGE: Pixels = px(1.);
/// The miniature window inside the card's border, in whole pixels.
const WIDTH: f32 = 148.;
const HEIGHT: f32 = 94.;
/// How much of the theme's corner radius the miniature's tiles keep.
const RADIUS_SCALE: f32 = 0.3;
/// How strongly the bars show against what they sit on, the first bar of the sidebar standing
/// for the active entry, and how much of its strength the accent tile keeps.
const BAR: f32 = 0.2;
const ACTIVE_BAR: f32 = 0.4;
const ACCENT: f32 = 0.9;

/// The sidebar's width. It runs the full height, so it alone carries the left corners.
const SIDEBAR: f32 = 38.;
/// The columns a cut between two palettes may run down: the sidebar's edge and the middle of
/// each gap between two tiles, so a cut never splits a tile or the play button.
const SEAMS: [f32; 3] = [SIDEBAR, 76., 110.];
/// How far a slice runs on under the slice after it.
const SEAM_OVERLAP: f32 = 1.;
/// The sidebar's entries and the content's heading, as left, top, width and height.
const ENTRIES: [[f32; 4]; 3] = [[8., 14., 22., 5.], [8., 25., 16., 5.], [8., 36., 19., 5.]];
const HEADING: [f32; 4] = [45., 14., 34., 5.];
/// The row of tiles under the heading by their left edges, the first one in the accent.
const TILES: [f32; 3] = [45., 79., 113.];
const TILE_TOP: f32 = 27.;
const TILE: f32 = 28.;
/// The player floating at the foot of the content, and its play button.
const PLAYER: [f32; 4] = [45., 66., 96., 14.];
const PLAY: [f32; 4] = [50., 70., 6., 6.];

/// The size of the card that holds a miniature drawn at `scale`, its border included. At a scale
/// of 1 it is `CARD` by `THUMB`.
pub(crate) fn framed(scale: f32) -> Size<Pixels> {
    size(
        px(WIDTH * scale) + EDGE * 2.,
        px(HEIGHT * scale) + EDGE * 2.,
    )
}

/// The miniature cut into one vertical slice per palette, left to right, each slice the same
/// window painted in its own palette so the layout runs on across the cuts. The cuts run down
/// `SEAMS` and show only as the change of colour. `radius` is the card's, whose inside the
/// miniature's outer corners follow. `scale` multiplies every shape, so a whole number keeps
/// them on whole pixels, and the card that holds it measures `framed(scale)`.
pub(crate) fn sliced(palettes: &[Theme], radius: Pixels, scale: f32) -> Div {
    let count = palettes.len().max(1);
    let cuts = (1..count).map(|index| {
        let even = WIDTH * index as f32 / count as f32;
        SEAMS
            .into_iter()
            .min_by(|a, b| (a - even).abs().total_cmp(&(b - even).abs()))
            .unwrap_or(even)
    });
    let edges: Vec<f32> = std::iter::once(0.)
        .chain(cuts)
        .chain(std::iter::once(WIDTH))
        .collect();
    let mark = (radius * RADIUS_SCALE * scale).floor();
    // the card does not clip to its corners, so the miniature rounds its own inside the border
    let outer = (radius - EDGE)
        .max(Pixels::ZERO)
        .min(px(SIDEBAR * scale / 2.));

    div().relative().size_full().children(
        palettes
            .iter()
            .zip(edges.windows(2))
            // each slice runs one pixel on under the next, so a cut that lands between pixels
            // while the rail scrolls never opens a seam, but stops short of the far corners,
            // whose soft edges only the last slice draws
            .map(|(palette, edge)| {
                let left = edge[0];
                let right = match edge[1] < WIDTH {
                    true => edge[1] + SEAM_OVERLAP,
                    false => WIDTH,
                };
                rect([left, 0., right - left, HEIGHT], scale)
                    .overflow_hidden()
                    .child(miniature(palette, outer, mark, scale).left(px(-left * scale)))
            }),
    )
}

/// The Sonora window in `palette`'s colours as a few soft, flat shapes, `WIDTH` by `HEIGHT`
/// pixels times `scale`: the sidebar with three entries, a heading over a row of tiles, the first
/// in the accent, and a floating player with its play button. Nothing is outlined. The window
/// and the sidebar round the outer corners by `outer`, and `mark` rounds the tiles.
fn miniature(palette: &Theme, outer: Pixels, mark: Pixels, scale: f32) -> Div {
    let panel = match palette.sidebar == palette.background {
        true => palette.secondary,
        false => palette.sidebar,
    };
    let bar = |shape: [f32; 4], strength: f32| {
        rect(shape, scale)
            .rounded_full()
            .bg(palette.foreground.opacity(strength))
    };

    div()
        .absolute()
        .top_0()
        .w(px(WIDTH * scale))
        .h(px(HEIGHT * scale))
        .rounded(outer)
        .bg(palette.background)
        .child(
            rect([0., 0., SIDEBAR, HEIGHT], scale)
                .rounded_l(outer)
                .bg(panel),
        )
        .children(ENTRIES.into_iter().enumerate().map(|(index, entry)| {
            bar(
                entry,
                match index {
                    0 => ACTIVE_BAR,
                    _ => BAR,
                },
            )
        }))
        .child(bar(HEADING, ACTIVE_BAR))
        .children(TILES.into_iter().enumerate().map(|(index, left)| {
            let fill = match index {
                0 => palette.primary.opacity(ACCENT),
                _ => palette.muted,
            };
            rect([left, TILE_TOP, TILE, TILE], scale)
                .rounded(mark)
                .bg(fill)
        }))
        .child(rect(PLAYER, scale).rounded_full().bg(panel))
        .child(rect(PLAY, scale).rounded_full().bg(palette.primary))
}

/// A box in the miniature from its left, top, width and height in pixels, times `scale`.
fn rect([x, y, width, height]: [f32; 4], scale: f32) -> Div {
    div()
        .absolute()
        .left(px(x * scale))
        .top(px(y * scale))
        .w(px(width * scale))
        .h(px(height * scale))
}
