use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, Rgba, ShaderInstance, Window};

use crate::shader::Effect;

/// The drift rate the shader is tuned around.
const SPEED: f32 = 1.;
/// How far the whole field rolls under the ribbons.
const SWELL: f32 = 1.;
/// How far each ribbon of colour curls along its length.
const BEND: f32 = 1.;
/// How soft a ribbon's edge is, from 0 for crisp to 1 for a long falloff.
const EDGE: f32 = 0.55;

static WAVES: Effect = Effect::new("sonora-waves", include_str!("../shaders/waves.wgsl"));

/// Ribbons of colour that drift and bend over a dark stage, for a field that fills a whole
/// surface. The first, second and third colours are the accents, the fourth and fifth the stage
/// beneath. It fills its positioned parent, is opaque, and dithers itself so the stage does not
/// band.
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
            .params(0, &[SPEED, SWELL, BEND, EDGE, r, g, b, a])
            .color(0, first)
            .color(1, second)
            .color(2, third)
            .color(3, fourth);
        WAVES.fill(instance, self.radius)
    }
}
