//! Soft pools of colour breathing over a stage, for a field that fills a whole surface such as
//! the fullscreen view's ambient background.

use rune::{Color, El, LoopSpec, el};

use crate::color::Hsla;
use crate::theme::Theme;

/// How much of its colour a pool lays over what is under it at its core.
const PEAK: f32 = 0.72;
/// Where the five pools sit, in fractions of the surface, and how far each reaches against the
/// surface's longer side.
const POOLS: [([f32; 2], f32); 5] = [
    ([0.18, 0.22], 0.75),
    ([0.82, 0.18], 0.7),
    ([0.5, 0.55], 0.8),
    ([0.15, 0.85], 0.7),
    ([0.85, 0.82], 0.75),
];
/// How long each pool takes to breathe out and in, and when in its breath it starts, so the
/// field never holds one shape.
const BREATHS: [(f64, f64); 5] = [(9., 0.), (11., 2.5), (13., 5.), (10., 7.5), (12., 1.5)];
/// The faintest a pool breathes to.
const LOW: f32 = 0.35;
/// How much darker the corners are than the middle, a light vignette that frames the field.
const VIGNETTE: f32 = 0.6;
/// How soft the pools' edges blur, in points.
const SOFTNESS: f32 = 24.;

/// The field. Each pool is laid over the ones before it, so the last colour reads on top. It
/// fills the size it is built at; Rune breathes it on its own clock and holds it still when the
/// system asks for less motion.
pub struct Waves {
    colors: [Hsla; 5],
    stage: Option<Hsla>,
    radius: f32,
}

impl Waves {
    pub fn new(colors: [Hsla; 5]) -> Self {
        Self {
            colors,
            stage: None,
            radius: 0.,
        }
    }

    /// The colour the pools lie on, black unless set.
    pub fn stage(mut self, stage: Hsla) -> Self {
        self.stage = Some(stage);
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// The field at `size`, the surface's `[width, height]` in points.
    pub fn build(self, _theme: &Theme, size: [f32; 2]) -> El {
        let [w, h] = size;
        let long = w.max(h).max(1.);
        let stage = self.stage.map(Hsla::color).unwrap_or(Color::BLACK);
        let pools = self
            .colors
            .into_iter()
            .zip(POOLS)
            .zip(BREATHS)
            .enumerate()
            .map(|(index, ((color, (center, reach)), (period, delay)))| {
                let core = color.color().alpha(color.a * PEAK);
                el().key(format!("pool-{index}"))
                    .absolute(0., 0.)
                    .size(w, h)
                    .radial_gradient(
                        center,
                        long * reach / 2.,
                        [(0., core), (1., core.alpha(0.))],
                    )
                    .looping(LoopSpec::opacity(period, delay, LOW, 1.))
            });
        el().size(w, h)
            .radius(self.radius)
            .clip_rounded(self.radius)
            .fill(stage)
            .child(
                el().absolute(0., 0.)
                    .size(w, h)
                    .blur(SOFTNESS)
                    .children(pools),
            )
            .child(el().absolute(0., 0.).size(w, h).radial_gradient(
                [0.5, 0.5],
                [w * 0.75, h * 0.75],
                [
                    (0.5, Color::BLACK.alpha(0.)),
                    (1., Color::BLACK.alpha(VIGNETTE)),
                ],
            ))
            .a11y_hidden()
    }
}
