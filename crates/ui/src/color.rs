//! Colours as the theme authors them: sRGB in hue, saturation and lightness, with straight alpha.
//! Rune draws in linear light, so every colour crosses into a [`rune::Color`] at the element.

/// A colour in sRGB HSL, each channel in `0..=1`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hsla {
    pub h: f32,
    pub s: f32,
    pub l: f32,
    pub a: f32,
}

/// A colour in sRGB with straight alpha, each channel in `0..=1`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// An opaque colour from `0xrrggbb`.
pub const fn rgb(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.,
        g: ((hex >> 8) & 0xff) as f32 / 255.,
        b: (hex & 0xff) as f32 / 255.,
        a: 1.,
    }
}

/// A colour from `0xrrggbbaa`.
pub const fn rgba(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 24) & 0xff) as f32 / 255.,
        g: ((hex >> 16) & 0xff) as f32 / 255.,
        b: ((hex >> 8) & 0xff) as f32 / 255.,
        a: (hex & 0xff) as f32 / 255.,
    }
}

pub const fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

pub const fn black() -> Hsla {
    hsla(0., 0., 0., 1.)
}

pub const fn white() -> Hsla {
    hsla(0., 0., 1., 1.)
}

pub const fn transparent() -> Hsla {
    hsla(0., 0., 0., 0.)
}

impl Hsla {
    /// The same colour at `alpha`.
    pub fn opacity(self, alpha: f32) -> Self {
        Self {
            a: self.a * alpha.clamp(0., 1.),
            ..self
        }
    }

    pub fn alpha(self, alpha: f32) -> Self {
        Self {
            a: alpha.clamp(0., 1.),
            ..self
        }
    }

    pub fn is_transparent(self) -> bool {
        self.a <= 0.
    }

    /// `other` drawn over `self` with source-over, in sRGB as a browser blends.
    pub fn blend(self, other: Self) -> Self {
        let (below, above) = (Rgba::from(self), Rgba::from(other));
        let alpha = above.a + below.a * (1. - above.a);
        if alpha <= 0. {
            return transparent();
        }
        let channel =
            |a: f32, b: f32| (b * above.a + a * below.a * (1. - above.a)) / alpha;
        Hsla::from(Rgba {
            r: channel(below.r, above.r),
            g: channel(below.g, above.g),
            b: channel(below.b, above.b),
            a: alpha,
        })
    }

    /// The colour as Rune draws it.
    pub fn color(self) -> rune::Color {
        self.into()
    }
}

impl From<Rgba> for Hsla {
    fn from(color: Rgba) -> Self {
        let Rgba { r, g, b, a } = color;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.;
        if max == min {
            return Hsla { h: 0., s: 0., l, a };
        }
        let d = max - min;
        let s = match l > 0.5 {
            true => d / (2. - max - min),
            false => d / (max + min),
        };
        let h = if max == r {
            (g - b) / d + if g < b { 6. } else { 0. }
        } else if max == g {
            (b - r) / d + 2.
        } else {
            (r - g) / d + 4.
        };
        Hsla { h: h / 6., s, l, a }
    }
}

impl From<Hsla> for Rgba {
    fn from(color: Hsla) -> Self {
        let Hsla { h, s, l, a } = color;
        if s == 0. {
            return Rgba { r: l, g: l, b: l, a };
        }
        let q = match l < 0.5 {
            true => l * (1. + s),
            false => l + s - l * s,
        };
        let p = 2. * l - q;
        let hue = |t: f32| {
            let t = t.rem_euclid(1.);
            if t < 1. / 6. {
                p + (q - p) * 6. * t
            } else if t < 0.5 {
                q
            } else if t < 2. / 3. {
                p + (q - p) * (2. / 3. - t) * 6.
            } else {
                p
            }
        };
        Rgba {
            r: hue(h + 1. / 3.),
            g: hue(h),
            b: hue(h - 1. / 3.),
            a,
        }
    }
}

impl From<Rgba> for rune::Color {
    fn from(color: Rgba) -> Self {
        rune::Color {
            r: rune::color::srgb_decode(color.r.clamp(0., 1.)),
            g: rune::color::srgb_decode(color.g.clamp(0., 1.)),
            b: rune::color::srgb_decode(color.b.clamp(0., 1.)),
            a: color.a.clamp(0., 1.),
        }
    }
}

impl From<Hsla> for rune::Color {
    fn from(color: Hsla) -> Self {
        Rgba::from(color).into()
    }
}

impl From<rune::Color> for Rgba {
    fn from(color: rune::Color) -> Self {
        Rgba {
            r: rune::color::srgb_encode(color.r),
            g: rune::color::srgb_encode(color.g),
            b: rune::color::srgb_encode(color.b),
            a: color.a,
        }
    }
}

/// `t` of the way from `from` to `to`, channel by channel in sRGB.
pub fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (from, to) = (Rgba::from(from), Rgba::from(to));
    let channel = |from: f32, to: f32| from + (to - from) * t;
    Hsla::from(Rgba {
        r: channel(from.r, to.r),
        g: channel(from.g, to.g),
        b: channel(from.b, to.b),
        a: channel(from.a, to.a),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips_through_rgb() {
        for hex in [0x0a0a0a, 0xfafafa, 0x1d4ed8, 0x7f1d1d, 0x909090] {
            let rgb = rgb(hex);
            let back = Rgba::from(Hsla::from(rgb));
            assert!((rgb.r - back.r).abs() < 1e-4, "{hex:x}");
            assert!((rgb.g - back.g).abs() < 1e-4, "{hex:x}");
            assert!((rgb.b - back.b).abs() < 1e-4, "{hex:x}");
        }
    }
}
