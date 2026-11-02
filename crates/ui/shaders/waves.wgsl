// Waves: a mesh gradient between five colours whose points drift on slow orbits. The gradient is
// read through two layers of sine waves, so the borders between colours bend and roll like light
// on water, and a faint band of the second colour sweeps across it.
//
// It is cheap enough to run over a whole window at the display rate: a dozen sines, five inverse
// distance weights and no noise lookups. The colours mix with their components squared, which is
// close to linear light for soft gradients, and a dither of one 8 bit step keeps the long dark
// gradients from banding.
//
// params[0].x  speed, 1 for the default drift with orbits of half a minute or so
// params[0].y  warp, how far the waves bend the gradient, around 1
// params[0].z  sheen, how bright the sweeping band is, 0 for none
// params[1]    the fifth colour, straight alpha sRGB
// colors[0..3] the first four colours, straight alpha sRGB

fn light(rgb: vec3<f32>) -> vec3<f32> {
    return rgb * rgb;
}

// Where a point stands at time `t`: `base` swung by `reach` on an orbit of angular `rate`, in the
// centred frame where the shorter side spans one unit.
fn orbit(base: vec2<f32>, reach: vec2<f32>, rate: f32, phase: f32, t: f32) -> vec2<f32> {
    let spin = t * rate + phase;
    return base + reach * vec2<f32>(sin(spin), cos(spin * 0.83 + phase * 0.7));
}

// How strongly a colour anchored at `at` claims the pixel at `p`. The weights are normalised
// afterwards, so only their ratios matter, and the softening term keeps each anchor from
// pinching into a point.
fn claim(p: vec2<f32>, at: vec2<f32>) -> f32 {
    let d = p - at;
    let near = dot(d, d) + 0.06;
    return 1.0 / (near * near);
}

fn shade(input: ShaderInput) -> vec4<f32> {
    let t = input.time * input.params[0].x;
    let warp = input.params[0].y;
    let sheen = input.params[0].z;

    let short = max(min(input.size.x, input.size.y), 1.0);
    let half = input.size / short * 0.5;
    let p = input.position / short - half;

    // Two octaves of crossed sines, each reading the other axis, roll the plane like a slow swell.
    var q = p;
    q += warp * 0.16 * vec2<f32>(sin(q.y * 2.3 + t * 0.31), sin(q.x * 1.9 - t * 0.27));
    q += warp * 0.07 * vec2<f32>(sin(q.y * 4.1 - t * 0.23 + 1.3), sin(q.x * 3.6 + t * 0.19 + 0.7));

    let w0 = claim(q, orbit(half * vec2<f32>(-0.56, -0.40), half * vec2<f32>(0.26, 0.20), 0.185, 0.0, t));
    let w1 = claim(q, orbit(half * vec2<f32>(0.60, -0.52), half * vec2<f32>(0.22, 0.26), 0.233, 1.7, t));
    let w2 = claim(q, orbit(half * vec2<f32>(0.04, 0.36), half * vec2<f32>(0.28, 0.18), 0.153, 3.4, t));
    let w3 = claim(q, orbit(half * vec2<f32>(-0.76, 0.56), half * vec2<f32>(0.20, 0.24), 0.262, 5.1, t));
    let w4 = claim(q, orbit(half * vec2<f32>(0.70, 0.44), half * vec2<f32>(0.24, 0.22), 0.203, 2.5, t));

    var color = (light(input.colors[0].rgb) * w0 + light(input.colors[1].rgb) * w1
        + light(input.colors[2].rgb) * w2 + light(input.colors[3].rgb) * w3
        + light(input.params[1].rgb) * w4) / (w0 + w1 + w2 + w3 + w4);

    // A broad diagonal band of the highlight colour drifts across, bent by the same swell.
    let band = 0.5 + 0.5 * sin(dot(q, vec2<f32>(2.2, -1.4)) - t * 0.37);
    color += light(input.colors[1].rgb) * sheen * band * band * band;

    return vec4<f32>(dither(sqrt(color), input.frag_coord), 1.0);
}
