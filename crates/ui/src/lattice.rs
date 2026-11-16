use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, Point, Rgba, ShaderInstance, Window, px};

use crate::shader::Effect;

/// The pace the pools drift at.
const SPEED: f32 = 1.;
/// How far apart the dots stand and how large each one draws.
const SPACING: Pixels = px(18.);
const DOT: Pixels = px(1.1);
/// How much of the pools shows between the dots, and how far they colour the dots themselves.
const HAZE: f32 = 0.16;
const GLOW: f32 = 0.85;
/// How wide the pointer's spotlight reaches and how strongly it lights the dots under it.
const SPOT_REACH: Pixels = px(220.);
const SPOT: f32 = 0.75;
/// How much the corners fade back into the stage.
const VIGNETTE: f32 = 0.55;
/// The strength of the soft light from the top middle.
const BEAM: f32 = 0.9;

static LATTICE: Effect = Effect::new("sonora-lattice", include_str!("../shaders/lattice.wgsl"));

/// A field of fine dots lit from beneath by three slow pools of colour and by a spotlight that
/// follows the pointer, for a backdrop that fills a whole surface behind frosted content. It
/// fills its positioned parent and is opaque.
#[derive(IntoElement)]
pub struct Lattice {
    pools: [Hsla; 3],
    lamp: Hsla,
    stage: Hsla,
    dark: bool,
    pointer: Option<Point<Pixels>>,
    time: f32,
    radius: Pixels,
}

impl Lattice {
    /// A lattice lit by `pools`. A pool's alpha scales how much light it casts.
    pub fn new(pools: [Hsla; 3]) -> Self {
        Self {
            pools,
            lamp: Hsla::white(),
            stage: Hsla::black(),
            dark: true,
            pointer: None,
            time: 0.,
            radius: Pixels::ZERO,
        }
    }

    /// The colour the dots lie on. A light stage turns the dots dark, so they read on either.
    pub fn stage(mut self, stage: Hsla) -> Self {
        self.stage = stage;
        self.dark = stage.l < 0.5;
        self
    }

    /// The colour of the spotlight, which an owner can warm to whatever lies under the pointer.
    pub fn lamp(mut self, lamp: Hsla) -> Self {
        self.lamp = lamp;
        self
    }

    /// Where the pointer is, in window coordinates. `None` leaves the lattice without a spotlight.
    pub fn pointer(mut self, pointer: Option<Point<Pixels>>) -> Self {
        self.pointer = pointer;
        self
    }

    /// How many seconds the pools have drifted. An animated owner advances it and requests the
    /// next frame itself.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    pub fn corner_radius(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }
}

impl RenderOnce for Lattice {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let channels = |color: Hsla| {
            let Rgba { r, g, b, a } = color.into();
            [r, g, b, a]
        };
        let [first, second, third] = self.pools;
        let ink = match self.dark {
            true => 1.,
            false => 0.,
        };
        let (pointer, lamp, stage, time) = (self.pointer, self.lamp, self.stage, self.time);
        LATTICE.fill_with(self.radius, move |bounds| {
            let spot = pointer.map(|pointer| pointer - bounds.origin);
            let (x, y, strength) = match spot {
                Some(spot) => (f32::from(spot.x), f32::from(spot.y), SPOT),
                None => (-1e5, -1e5, 0.),
            };
            ShaderInstance::at(time)
                .params(0, &[SPEED, f32::from(SPACING), f32::from(DOT), HAZE])
                .params(4, &[x, y, f32::from(SPOT_REACH), strength])
                .params(8, &channels(stage))
                .params(12, &[GLOW, ink, VIGNETTE, BEAM])
                .color(0, first)
                .color(1, second)
                .color(2, third)
                .color(3, lamp)
        })
    }
}
