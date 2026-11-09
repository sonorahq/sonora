//! A band that blurs what passes under chrome floating over content, at full strength along one
//! edge and fading to nothing across its height.

use rune::El;

use crate::color::Hsla;

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

/// The band. It fills its parent, which has to be `height` tall: a backdrop blur of `blur`
/// points under a soft mask that runs from whole at `edge` to clear at the other side. With
/// effects off there is no blur to fade, and the band is `background` instead. `corner` rounds
/// the two corners along `edge`, for a band flush with a rounded window. A frame blurs a few
/// backdrops at most, so a page takes one veil per edge.
pub fn veil(edge: VeilEdge, height: f32, blur: f32, background: Hsla, corner: Option<f32>) -> El {
    let radius = corner.unwrap_or(0.);
    let corners = match edge {
        VeilEdge::Top => [radius, radius, 0., 0.],
        VeilEdge::Bottom => [0., 0., radius, radius],
    };
    let band = rune::el()
        .pin(Some(0.), Some(0.), Some(0.), Some(0.))
        .corners(corners)
        .a11y_hidden();
    if !effects() {
        return band.fill(background.color());
    }
    let (top, bottom) = match edge {
        VeilEdge::Top => (0., height),
        VeilEdge::Bottom => (height, 0.),
    };
    band.clip_fade_linear(0., top, 0., bottom).child(
        rune::el()
            .w_full()
            .h_full()
            .corners(corners)
            .backdrop_blur(blur),
    )
}
