// Waves: five colours morphing into one another with no shape that holds. Nothing in the field
// moves as an object. The plane itself is turned by an angle that drifts with slow noise, pushed
// around by more noise and rolled by crossed sines, and the colours are read off the result as a
// wide four corner gradient. Every term is low frequency, so the field stays as soft as a heavy
// blur, and the accents swell, split and dissolve instead of travelling.
//
// A slow wave of shade runs through it and sinks the troughs toward the fifth colour, and a
// vignette darkens the edges, which keeps the field dark enough for lyrics to read over it. A
// dither of one 8 bit step keeps the dark gradients from banding.
//
// params[0].x  speed, 1 for the default pace
// params[0].y  swell, how far the noise and the sines push the plane, around 1
// params[0].z  morph, how far the drifting turn twists the plane, around 1
// params[1]    the fifth colour, straight alpha sRGB, which the troughs sink toward
// colors[0..3] the corners of the gradient, straight alpha sRGB

fn light(rgb: vec3<f32>) -> vec3<f32> {
    return rgb * rgb;
}

fn shade(input: ShaderInput) -> vec4<f32> {
    let t = input.time * input.params[0].x;
    let swell = input.params[0].y;
    let morph = input.params[0].z;

    let short = max(min(input.size.x, input.size.y), 1.0);
    let half = input.size / short * 0.5;
    let p = input.position / short - half;

    // The turn drifts with time and with x * y, so each quarter of the plane twists its own way
    // and the twist keeps changing.
    let turn = morph * 1.8 * gradient_noise(vec2<f32>(t * 0.045, p.x * p.y * 0.5 + 3.1));
    var q = rotate(p, turn);
    q += swell * 0.42 * vec2<f32>(
        gradient_noise(q * 1.05 + vec2<f32>(0.0, t * 0.06)),
        gradient_noise(q * 1.05 + vec2<f32>(5.2, -t * 0.05)),
    );
    q.x += swell * 0.12 * sin(q.y * 2.2 + t * 0.21);
    q.y += swell * 0.10 * sin(q.x * 2.9 - t * 0.17);

    // A dark stage in the cover's own shadows, leaning from the deep fifth colour at the top to
    // the fourth at the bottom.
    var color = mix(light(input.params[1].rgb), light(input.colors[3].rgb), smoothstep(-0.7, 0.7, q.y)) * 0.6;

    // Three pools of colour, each where its own slow noise runs high. They are thresholds of a
    // field rather than shapes, so they swell, split and dissolve as the plane morphs.
    let first = smoothstep(-0.2, 0.4, gradient_noise(q * 0.8 + vec2<f32>(t * 0.05, 1.3)));
    color = mix(color, light(input.colors[0].rgb) * 1.15, first);
    let second = smoothstep(-0.1, 0.45, gradient_noise(q * 0.75 + vec2<f32>(7.1, -t * 0.045)));
    color = mix(color, light(input.colors[2].rgb), second * 0.95);
    let bright = smoothstep(0.0, 0.5, gradient_noise(q * 1.05 + vec2<f32>(-t * 0.06, 3.7)));
    color = mix(color, light(input.colors[1].rgb) * 1.3, bright * 0.9);

    // A slow wave of shade sinks its troughs a little, so the field breathes in depth too.
    let wave = 0.5 + 0.5 * sin(q.x * 1.6 + q.y * 2.1 + t * 0.27);
    color *= 0.86 + 0.14 * wave;

    let reach = length(p / half);
    color *= 0.8 + 0.2 * smoothstep(1.3, 0.3, reach);

    return vec4<f32>(dither(sqrt(color), input.frag_coord), 1.0);
}
