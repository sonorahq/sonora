// Waves: five soft pools of colour morphing over a stage, with no shape that holds. Nothing in
// the field moves as an object. The plane itself is turned by an angle that drifts with slow
// noise, pushed around by more noise and rolled by crossed sines, and each colour pools where its
// own slow noise runs high on the result. Every term is low frequency, so the field stays as soft
// as a heavy blur, and the pools swell, split and dissolve instead of travelling.
//
// The pools are laid over the stage one after another, each covering what is under it by up to
// `peak` in plain sRGB, which is how stacked translucent quads blend. A dither of one 8 bit step
// keeps the dark gradients from banding.
//
// params[0].x  speed, 1 for the default pace
// params[0].y  swell, how far the noise and the sines push the plane, around 1
// params[0].z  morph, how far the drifting turn twists the plane, around 1
// params[0].w  peak, how much of its colour a pool lays down at its core
// params[1]    the fifth colour, straight alpha sRGB
// params[2]    the stage the pools lie on, straight alpha sRGB
// params[3].x  density, how many units of the field fit in the geometric mean of the surface's
//              sides. The field takes the shape of the surface rather than being cut from a
//              square: a wide one holds more waves across and fewer down, a tall one the other
//              way, and the waves themselves stay round
// params[3].y  softness, how wide the edge of a pool runs, 1 for the default and more for a
//              blur over the transitions
// params[3].z  vignette, how much darker the corners are than the middle, 0 for none
// params[3].w  ceiling, the sRGB luma above which light is pressed back, 0 for none
// colors[0..3] the first four colours, straight alpha sRGB

// How much of the plane the pool keyed by `seed` covers here, from 0 outside it to 1 at its core.
// `soft` widens the edge around the same middle, which reads as a blur over the transition.
fn pool(q: vec2<f32>, scale: f32, seed: vec2<f32>, drift: vec2<f32>, soft: f32) -> f32 {
    let reach = 0.4 * soft;
    return smoothstep(0.2 - reach, 0.2 + reach, gradient_noise(q * scale + seed + drift));
}

fn shade(input: ShaderInput) -> vec4<f32> {
    let t = input.time * input.params[0].x;
    let swell = input.params[0].y;
    let morph = input.params[0].z;
    let peak = input.params[0].w;
    let density = max(input.params[3].x, 0.1);
    let soft = max(input.params[3].y, 0.1);

    let p = (input.position - input.size * 0.5) / max(sqrt(input.size.x * input.size.y), 1.0) * density;

    // The plane twists by an angle read off slow noise across it, about a pivot that sits off the
    // middle and wanders, so no two parts of the surface turn alike and nothing mirrors across the
    // centre.
    let pivot = vec2<f32>(0.23 + 0.3 * sin(t * 0.031), -0.17 + 0.25 * cos(t * 0.023));
    let turn = morph * 1.2 * gradient_noise(p * 0.55 + vec2<f32>(t * 0.045, 3.1));
    var q = rotate(p - pivot, turn) + pivot;
    q += swell * 0.26 * vec2<f32>(
        gradient_noise(q * 1.05 + vec2<f32>(0.0, t * 0.06)),
        gradient_noise(q * 1.05 + vec2<f32>(5.2, -t * 0.05)),
    );
    q.x += swell * 0.07 * sin(q.y * 2.2 + t * 0.21);
    q.y += swell * 0.06 * sin(q.x * 2.9 - t * 0.17);

    var color = input.params[2].rgb;
    color = mix(color, input.colors[0].rgb, peak * pool(q, 1.1, vec2<f32>(1.3, 0.0), vec2<f32>(t * 0.05, 0.0), soft));
    color = mix(color, input.colors[1].rgb, peak * pool(q, 1.3, vec2<f32>(-4.7, 2.2), vec2<f32>(0.0, -t * 0.055), soft));
    color = mix(color, input.colors[2].rgb, peak * pool(q, 1.0, vec2<f32>(7.1, -3.4), vec2<f32>(-t * 0.045, 0.0), soft));
    color = mix(color, input.colors[3].rgb, peak * pool(q, 1.2, vec2<f32>(-2.6, -6.3), vec2<f32>(0.0, t * 0.04), soft));
    color = mix(color, input.params[1].rgb, peak * pool(q, 1.15, vec2<f32>(9.4, 5.8), vec2<f32>(t * 0.035, -t * 0.03), soft));

    // The vignette reads the distance from the middle before the density packs the plane, so it
    // keeps to the edges of the surface whatever the waves do. 1 is a corner, so the middle
    // stays untouched and only the outer reach darkens.
    let edge = length((input.uv - vec2<f32>(0.5)) * 2.0) * 0.7071;
    color *= 1.0 - input.params[3].z * smoothstep(0.15, 1.0, edge);

    // A soft ceiling: light under it passes untouched and light over it keeps only a third of
    // the excess, so a vivid cover cannot wash out the text laid over the field.
    let ceiling = input.params[3].w;
    let level = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    if ceiling > 0.0 && level > ceiling {
        color *= (ceiling + (level - ceiling) * 0.35) / level;
    }

    return vec4<f32>(dither(color, input.frag_coord), 1.0);
}
