// Waves: ribbons of colour that drift over a dark stage and bend as they go. Each ribbon is an
// ellipse whose long axis is curled by a travelling wave, read through a slow swell of the whole
// plane, so the accents keep a clear edge while they roll and fold into one another.
//
// It is cheap enough to run over a whole window at the display rate: a few sines per ribbon and
// no noise lookups. The colours mix with their components squared, which is close to linear light
// for soft edges, and a dither of one 8 bit step keeps the dark stage from banding.
//
// params[0].x  speed, 1 for the default drift with orbits of half a minute or so
// params[0].y  swell, how far the whole plane rolls, around 1
// params[0].z  bend, how far each ribbon curls along its length, around 1
// params[0].w  edge, how soft a ribbon's edge is, from 0 for crisp to 1 for a long falloff
// params[1]    the fifth colour, straight alpha sRGB
// colors[0..3] the first four colours, straight alpha sRGB
//
// colors[0] and colors[2] take the two large ribbons, colors[1] the bright one, the fifth colour
// a dark one that cuts between them, and colors[3] the stage beneath.

fn light(rgb: vec3<f32>) -> vec3<f32> {
    return rgb * rgb;
}

// Where a ribbon's centre stands at time `t`: `base` swung by `reach` on an orbit of angular
// `rate`, in the centred frame where the shorter side spans one unit.
fn orbit(base: vec2<f32>, reach: vec2<f32>, rate: f32, phase: f32, t: f32) -> vec2<f32> {
    let spin = t * rate + phase;
    return base + reach * vec2<f32>(sin(spin), cos(spin * 0.83 + phase * 0.7));
}

// How much of the ribbon centred at `at` covers `p`. The ribbon is `span` long and `girth` thick,
// turned by `angle`, and a wave of `bend` travels along it so it curls rather than sitting
// straight. The cover is 1 in the core and falls to 0 at the edge over `edge`.
fn ribbon(
    p: vec2<f32>,
    at: vec2<f32>,
    angle: f32,
    span: f32,
    girth: f32,
    bend: f32,
    edge: f32,
    t: f32,
    phase: f32,
) -> f32 {
    var local = rotate(p - at, angle);
    local.y += bend * girth * 1.4 * sin(local.x / span * 3.0 + t * 0.42 + phase);
    let d = length(local / vec2<f32>(span, girth));
    return 1.0 - smoothstep(1.0 - edge, 1.0, d);
}

fn shade(input: ShaderInput) -> vec4<f32> {
    let t = input.time * input.params[0].x;
    let swell = input.params[0].y;
    let bend = input.params[0].z;
    let edge = clamp(input.params[0].w, 0.05, 1.0);

    let short = max(min(input.size.x, input.size.y), 1.0);
    let half = input.size / short * 0.5;
    let p = input.position / short - half;

    // Two octaves of crossed sines, each reading the other axis, roll the plane like a slow swell.
    var q = p;
    q += swell * 0.20 * vec2<f32>(sin(q.y * 2.1 + t * 0.29), sin(q.x * 1.7 - t * 0.25));
    q += swell * 0.08 * vec2<f32>(sin(q.y * 4.3 - t * 0.21 + 1.3), sin(q.x * 3.8 + t * 0.17 + 0.7));

    let stage_low = light(input.colors[3].rgb);
    let stage_high = light(input.params[1].rgb);
    var color = mix(stage_high, stage_low, smoothstep(-0.6, 0.6, q.y)) * 0.55;

    let first = ribbon(q, orbit(half * vec2<f32>(-0.50, -0.30), half * vec2<f32>(0.28, 0.22), 0.185, 0.0, t),
        0.35 + t * 0.045, 0.78, 0.26, bend, edge, t, 0.0);
    color = mix(color, light(input.colors[0].rgb), first * 0.92);

    let second = ribbon(q, orbit(half * vec2<f32>(0.52, 0.34), half * vec2<f32>(0.24, 0.26), 0.153, 3.4, t),
        -0.55 - t * 0.038, 0.72, 0.24, bend, edge, t, 2.1);
    color = mix(color, light(input.colors[2].rgb), second * 0.9);

    let shadow = ribbon(q, orbit(half * vec2<f32>(0.10, -0.05), half * vec2<f32>(0.30, 0.24), 0.233, 1.7, t),
        1.2 + t * 0.052, 0.66, 0.16, bend, edge, t, 4.2);
    color = mix(color, stage_high * 0.6, shadow * 0.75);

    let bright = ribbon(q, orbit(half * vec2<f32>(0.38, -0.46), half * vec2<f32>(0.26, 0.20), 0.262, 5.1, t),
        -1.0 - t * 0.06, 0.52, 0.15, bend, edge, t, 1.1);
    color = mix(color, light(input.colors[1].rgb), bright * 0.85);

    return vec4<f32>(dither(sqrt(color), input.frag_coord), 1.0);
}
