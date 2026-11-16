// Lattice: a field of fine dots on a stage, lit from beneath by slow pools of colour and by a
// spotlight that follows the pointer. Between the dots the pools show only as a faint haze, so
// the light reads as falling through a grille. A soft beam from the top middle keeps the eye on
// whatever the surface holds in its centre.
//
// params[0].x  speed, 1 for the default pace
// params[0].y  spacing, logical pixels between dot centres
// params[0].z  dot radius in logical pixels
// params[0].w  haze, how much of the pools shows between the dots
// params[1].xy the pointer in logical pixels from the top left, x below -1e4 for none
// params[1].z  spotlight radius in logical pixels
// params[1].w  spotlight strength, 0 for none
// params[2]    the stage, straight alpha sRGB
// params[3].x  glow, how far the pools colour the dots
// params[3].y  ink, 1 raises the dots toward white and 0 presses them toward black
// params[3].z  vignette, how much the corners fade back into the stage
// params[3].w  beam, the strength of the light from the top middle
// colors[0..2] the pool colours, straight alpha sRGB, alpha scaling each pool
// colors[3]    the spotlight colour

// How much of the plane the pool keyed by `seed` covers here, from 0 outside it to 1 at its core.
fn pool(q: vec2<f32>, seed: vec2<f32>, drift: vec2<f32>) -> f32 {
    return smoothstep(0.0, 0.8, gradient_noise(q + seed + drift));
}

fn shade(input: ShaderInput) -> vec4<f32> {
    let t = input.time * input.params[0].x;
    let spacing = max(input.params[0].y, 2.0);
    let radius = input.params[0].z;
    let haze = input.params[0].w;
    let stage = input.params[2].rgb;
    let glow = input.params[3].x;
    let neutral = vec3<f32>(select(0.0, 1.0, input.params[3].y > 0.5));

    // The pools live in a plane scaled by the surface's geometric mean, so they keep their size
    // whatever shape the window takes.
    let p = (input.position - input.size * 0.5) / max(sqrt(input.size.x * input.size.y), 1.0) * 1.7;
    let q = p + 0.3 * vec2<f32>(
        gradient_noise(p * 0.8 + vec2<f32>(0.0, t * 0.05)),
        gradient_noise(p * 0.8 + vec2<f32>(4.1, -t * 0.04)),
    );
    let a = pool(q * 1.1, vec2<f32>(1.7, 0.3), vec2<f32>(t * 0.045, 0.0)) * input.colors[0].a;
    let b = pool(q * 1.25, vec2<f32>(-5.2, 2.6), vec2<f32>(0.0, -t * 0.04)) * input.colors[1].a;
    let c = pool(q * 0.95, vec2<f32>(8.3, -4.1), vec2<f32>(-t * 0.035, t * 0.025)) * input.colors[2].a;
    let total = a + b + c;
    let tint = (input.colors[0].rgb * a + input.colors[1].rgb * b + input.colors[2].rgb * c)
        / max(total, 1e-4);
    let cover = saturate(total);

    // The beam falls from just above the top middle and fades out by the lower third.
    let toward = (input.uv - vec2<f32>(0.5, -0.1)) * vec2<f32>(1.1, 1.0);
    let beam = input.params[3].w * exp(-dot(toward, toward) * 3.2);

    var spot = 0.0;
    let pointer = input.params[1].xy;
    if pointer.x > -1e4 {
        let reach = max(input.params[1].z, 1.0);
        let d = length(input.position - pointer) / reach;
        spot = input.params[1].w * exp(-d * d * 2.0);
    }
    let lamp = input.colors[3].rgb;

    // The dot grid keys off device pixels so every dot lands on the same subpixel phase.
    let step = spacing * input.scale_factor;
    let local = (fract(input.device_position / step) - 0.5) * step / input.scale_factor;
    let swell = radius * (1.0 + 0.7 * spot);
    let speck = coverage(length(local) - swell, input.scale_factor);

    var ground = mix(stage, tint, haze * cover);
    ground = mix(ground, neutral, beam * 0.05);
    ground = mix(ground, lamp, spot * haze * 0.9);

    var ink = mix(stage, neutral, 0.09 + beam * 0.22);
    ink = mix(ink, tint, glow * cover);
    ink = mix(ink, lamp, saturate(spot * 1.1));

    var color = mix(ground, ink, speck);

    let edge = length((input.uv - vec2<f32>(0.5)) * 2.0) * 0.7071;
    color = mix(color, stage, input.params[3].z * smoothstep(0.25, 1.0, edge));

    return vec4<f32>(dither(color, input.frag_coord), 1.0);
}
