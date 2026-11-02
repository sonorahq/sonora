use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, Rgba, ShaderInstance, Window};

use crate::shader::Effect;

/// The pace the shader is tuned around.
const SPEED: f32 = 1.;
/// How far noise and waves push the field around.
const SWELL: f32 = 1.;
/// How far the drifting turn twists the field.
const MORPH: f32 = 1.;
/// How much of its colour a pool lays over what is under it at its core.
const PEAK: f32 = 0.72;
/// How many times more of the field fits across than the shader is tuned for, so the waves lie as
/// close as they would on a window that much wider and read as one woven sheet.
const DENSITY: f32 = 2.2;
/// How wide the edge of a pool runs against the shader's own, a light blur over the transitions.
const SOFTNESS: f32 = 1.8;
/// How much darker the corners are than the middle, a light vignette that frames the field.
const VIGNETTE: f32 = 0.25;

static WAVES: Effect = Effect::new("sonora-waves", include_str!("../shaders/waves.wgsl"));

/// Five soft pools of colour morphing over a stage, with no shape that holds and nothing sharper
/// than a heavy blur, for a field that fills a whole surface. Each pool is laid over the ones
/// before it, so the last colour reads on top. It fills its positioned parent, is opaque, and
/// dithers itself so the dark gradients do not band.
#[derive(IntoElement)]
pub struct Waves {
    colors: [Hsla; 5],
    stage: Hsla,
    time: f32,
    radius: Pixels,
}

impl Waves {
    pub fn new(colors: [Hsla; 5]) -> Self {
        Self {
            colors,
            stage: Hsla::black(),
            time: 0.,
            radius: Pixels::ZERO,
        }
    }

    /// The colour the pools lie on, black unless set.
    pub fn stage(mut self, stage: Hsla) -> Self {
        self.stage = stage;
        self
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
        let channels = |color: Hsla| {
            let Rgba { r, g, b, a } = color.into();
            [r, g, b, a]
        };
        let instance = ShaderInstance::at(self.time)
            .params(0, &[SPEED, SWELL, MORPH, PEAK])
            .params(4, &channels(fifth))
            .params(8, &channels(self.stage))
            .params(12, &[DENSITY, SOFTNESS, VIGNETTE])
            .color(0, first)
            .color(1, second)
            .color(2, third)
            .color(3, fourth);
        WAVES.fill(instance, self.radius)
    }
}
