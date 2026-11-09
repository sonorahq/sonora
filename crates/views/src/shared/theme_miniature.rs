//! The theme miniature: the Sonora window drawn as a few soft, flat shapes in a theme's
//! colours, the picture on a theme card and in the dialog that offers a linked theme.

use rune::{El, el};
use ui::Theme;

/// How wide a theme card stands.
pub(crate) const CARD: f32 = 150.;
/// How tall a theme card's picture stands, its border included.
pub(crate) const THUMB: f32 = 96.;
/// How many variants of a family a miniature shows side by side at most.
pub(crate) const SLICES: usize = 4;
/// How thick the card's border is, which the miniature sits inside.
const EDGE: f32 = 1.;
/// The miniature window inside the card's border, in whole points.
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
#[allow(
    dead_code,
    reason = "the dialog that offers a linked theme sizes its card by it"
)]
pub(crate) fn framed(scale: f32) -> (f32, f32) {
    (WIDTH * scale + EDGE * 2., HEIGHT * scale + EDGE * 2.)
}

/// The miniature cut into one vertical slice per palette, left to right, each slice the same
/// window painted in its own palette so the layout runs on across the cuts. The cuts run down
/// `SEAMS` and show only as the change of colour. `radius` is the card's, whose inside the
/// miniature's outer corners follow. `scale` multiplies every shape, so a whole number keeps
/// them on whole points, and the card that holds it measures `framed(scale)`.
pub(crate) fn sliced(palettes: &[Theme], radius: f32, scale: f32) -> El {
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
    let outer = (radius - EDGE).max(0.).min(SIDEBAR * scale / 2.);

    el().key("miniature")
        .absolute(EDGE, EDGE)
        .size(WIDTH * scale, HEIGHT * scale)
        .clip_rounded(outer)
        .a11y_hidden()
        .children(
            palettes
                .iter()
                .zip(edges.windows(2))
                .enumerate()
                // each slice runs one point on under the next, so a cut that lands between
                // pixels while the rail scrolls never opens a seam
                .map(|(index, (palette, edge))| {
                    let left = edge[0];
                    let right = match edge[1] < WIDTH {
                        true => edge[1] + SEAM_OVERLAP,
                        false => WIDTH,
                    };
                    rect([left, 0., right - left, HEIGHT], scale)
                        .key(format!("slice-{index}"))
                        .clip_rounded(0.)
                        .child(miniature(palette, outer, mark, scale).absolute(-left * scale, 0.))
                }),
        )
}

/// The Sonora window in `palette`'s colours as a few soft, flat shapes, `WIDTH` by `HEIGHT`
/// points times `scale`: the sidebar with three entries, a heading over a row of tiles, the
/// first in the accent, and a floating player with its play button. Nothing is outlined.
fn miniature(palette: &Theme, outer: f32, mark: f32, scale: f32) -> El {
    let panel = match palette.sidebar == palette.background {
        true => palette.secondary,
        false => palette.sidebar,
    };
    let bar = |shape: [f32; 4], strength: f32| {
        rect(shape, scale)
            .radius(shape[3] * scale / 2.)
            .fill(palette.foreground.opacity(strength).color())
    };

    el().size(WIDTH * scale, HEIGHT * scale)
        .radius(outer)
        .fill(palette.background.color())
        .child(
            rect([0., 0., SIDEBAR, HEIGHT], scale)
                .corners([outer, 0., 0., outer])
                .fill(panel.color()),
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
                .radius(mark)
                .fill(fill.color())
        }))
        .child(
            rect(PLAYER, scale)
                .radius(PLAYER[3] * scale / 2.)
                .fill(panel.color()),
        )
        .child(
            rect(PLAY, scale)
                .radius(PLAY[3] * scale / 2.)
                .fill(palette.primary.color()),
        )
}

/// A box in the miniature from its left, top, width and height in points, times `scale`.
fn rect([x, y, width, height]: [f32; 4], scale: f32) -> El {
    el().absolute(x * scale, y * scale)
        .size(width * scale, height * scale)
}
