use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, Rgba, ShaderInstance, Window};

use crate::shader::Effect;

/// The drift rate the shader is tuned around.
const SPEED: f32 = 1.;
/// How far the swell bends the borders between colours.
const WARP: f32 = 1.;
/// How bright the band of light that sweeps across the field is.
const SHEEN: f32 = 0.18;

static WAVES: Effect = Effect::new("sonora-waves", include_str!("../shaders/waves.wgsl"));

/// Five colours drifting on slow orbits and folded into one another by waves, for a field that
/// fills a whole surface. It fills its positioned parent, is opaque, and dithers itself so a wide
/// dark gradient does not band.
#[derive(IntoElement)]
pub struct Waves {
    colors: [Hsla; 5],
    time: f32,
    radius: Pixels,
}

impl Waves {
    pub fn new(colors: [Hsla; 5]) -> Self {
        Self {
            colors,
            time: 0.,
            radius: Pixels::ZERO,
        }
    }

    /// How many seconds the field has drifted. A fixed time paints a still frame, so an
    /// animated owner advances it and requests the next frame itself.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    pub fn corner_radius(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }
}

impl RenderOnce for Waves {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let [first, second, third, fourth, fifth] = self.colors;
        let Rgba { r, g, b, a } = fifth.into();
        let instance = ShaderInstance::at(self.time)
            .params(0, &[SPEED, WARP, SHEEN, 0., r, g, b, a])
            .color(0, first)
            .color(1, second)
            .color(2, third)
            .color(3, fourth);
        WAVES.fill(instance, self.radius)
    }
}
