//! Effects written once in WGSL under `crates/ui/shaders`. Each source defines only
//! `fn shade(input: ShaderInput) -> vec4<f32>`, and GPUI wraps it in its prelude and runs it on
//! wgpu directly and on Metal and DirectX through naga's translation.

use std::sync::OnceLock;

use gpui::prelude::*;
use gpui::{Corners, Pixels, Shader, ShaderInstance, ShaderSource, canvas};

/// One WGSL effect, validated and registered with GPUI the first time it paints. A source that
/// fails validation is logged by GPUI once and then paints nothing.
pub(crate) struct Effect {
    label: &'static str,
    wgsl: &'static str,
    shader: OnceLock<Option<Shader>>,
}

impl Effect {
    pub(crate) const fn new(label: &'static str, wgsl: &'static str) -> Self {
        Self {
            label,
            wgsl,
            shader: OnceLock::new(),
        }
    }

    fn shader(&self) -> Option<Shader> {
        *self
            .shader
            .get_or_init(|| Shader::new(ShaderSource::new(self.label, self.wgsl)).ok())
    }

    /// A child that fills its positioned parent with one instance of the effect, rounded by
    /// `radius`. The effect animates only by the instance's time, so an animated owner requests
    /// its own frames.
    pub(crate) fn fill(
        &'static self,
        instance: ShaderInstance,
        radius: Pixels,
    ) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some(shader) = self.shader() {
                    window.paint_shader(bounds, Corners::all(radius), &shader, instance);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}
