use gpui::prelude::*;
use gpui::{Div, Hsla, Pixels, Window, div};

use crate::snapped;

/// How many strips the blur fades in through. They cost nothing beyond a quad each, so more of
/// them only make the crossfade smoother.
const STRIPS: usize = 64;
/// How steeply the haze rises from the clear edge, as the exponent in `1 - (1 - t)^RISE`. The
/// first strip starts near zero, so no line shows where the blur begins, and the haze still
/// reaches working strength within the first half of the band.
const RISE: f32 = 2.5;

/// Which side of a veil holds the full blur. The other one is clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VeilEdge {
    Top,
    Bottom,
}

/// Whether backdrop blurs are drawn at all. `SONORA_BLUR=0` turns them off for a renderer that
/// cannot afford them, and every haze falls back to its flat colour.
pub fn effects() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SONORA_BLUR").as_deref() != Ok("0"))
}

/// A band that blurs what passes under it, at full strength along `edge` and fading to nothing
/// across `height`, so chrome floating over content stays readable without a hard line where
/// the treatment stops. It fills its parent, which has to be `height` tall and `relative`.
///
/// The renderer blurs a run of consecutive backdrops once, by the widest radius among them, and
/// honours each one's opacity, so the strips cost one blur pass a frame. Interleaving anything
/// between them splits that into a pass each, which lags. A backdrop inside a layer that carries
/// a paint filter is dropped, so the band only blurs while nothing above it scales or fades. With
/// effects off there is no blur to fade and the band is the plain colour instead. The Blur UI
/// setting never reaches it: a band is there to keep what floats over it readable, not for the
/// look of it.
///
/// `corner` rounds the two corners along `edge`, for a band flush with a rounded window.
pub fn veil(
    edge: VeilEdge,
    height: Pixels,
    blur: Pixels,
    background: Hsla,
    corner: Option<Pixels>,
    window: &Window,
) -> Div {
    let round = move |strip: Div, radius: Pixels| match edge {
        VeilEdge::Top => strip.rounded_t(radius),
        VeilEdge::Bottom => strip.rounded_b(radius),
    };
    let mut edges: Vec<Pixels> = (0..=STRIPS)
        .map(|slice| snapped(height * (slice as f32 / STRIPS as f32), window))
        .collect();
    // A strip clamps its radius to half its own height, so the strips along the edge merge
    // into one tall enough to carry the corner.
    if let Some(radius) = corner {
        let end = edges[STRIPS];
        let merged = snapped(radius * 2., window).min(end);
        match edge {
            VeilEdge::Top => {
                edges.retain(|at| *at == Pixels::ZERO || *at >= merged);
                edges.insert(1, merged);
            }
            VeilEdge::Bottom => {
                edges.retain(|at| *at <= end - merged || *at == end);
                edges.insert(edges.len() - 1, end - merged);
            }
        }
    }
    let rounded = match edge {
        VeilEdge::Top => 0,
        VeilEdge::Bottom => edges.len() - 2,
    };
    let strips = edges
        .windows(2)
        .enumerate()
        .filter_map(move |(slice, span)| {
            let cut = span[1] - span[0];
            let across = (span[0] + cut / 2.) / height;
            let held = match edge {
                VeilEdge::Top => 1. - across,
                VeilEdge::Bottom => across,
            };
            (cut > Pixels::ZERO).then(|| {
                div()
                    .flex_none()
                    .w_full()
                    .h(cut)
                    .when_some(corner.filter(|_| slice == rounded), round)
                    .opacity(1. - (1. - held).powf(RISE))
                    .backdrop_blur(blur)
            })
        })
        .collect::<Vec<_>>();

    match effects() {
        true => div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .children(strips),
        false => div()
            .absolute()
            .inset_0()
            .when_some(corner, round)
            .bg(background),
    }
}
