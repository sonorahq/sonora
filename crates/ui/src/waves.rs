use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, Rgba, ShaderInstance, Window};

use crate::shader::Effect;

/// The pace of the morph, two thirds of the one the shader is tuned around.
const SPEED: f32 = 0.67;
/// How far noise and waves push the field around.
const SWELL: f32 = 1.;
/// How far the drifting turn twists the field.
const MORPH: f32 = 1.;
/// The linear luminance bright light is pressed toward, so text over the field stays readable.
const CEILING: f32 = 0.07;

static WAVES: Effect = Effect::new("sonora-waves", include_str!("../shaders/waves.wgsl"));

/// Five colours morphing into one another with no shape that holds, as soft as a heavy blur, for
/// a field that fills a whole surface. The first four span the gradient and the fifth is the shade
/// its troughs sink toward. It fills its positioned parent, is opaque, and dithers itself so the
/// dark gradients do not band.
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
            .params(0, &[SPEED, SWELL, MORPH, CEILING, r, g, b, a])
            .color(0, first)
            .color(1, second)
            .color(2, third)
            .color(3, fourth);
        WAVES.fill(instance, self.radius)
    }
}
