//! The ambient background: a flowing colour field behind fullscreen, five colours folded into
//! one another by slow breaths in the `ui::Waves` field, darkened so lyrics stay readable. The
//! fullscreen shell paints it under everything, title bar included, and paints no background of
//! its own beneath it. The field stands in for the page and carries the window's own opacity, so
//! a see-through window stays see-through in fullscreen.
//!
//! `ambient` decides whether it is painted at all, and with it whether the fullscreen controls
//! frost what they float over. `ambient_motion` decides whether the field breathes.
//!
//! The colours are the theme's `tint` and `tint_secondary`, which fullscreen samples off the
//! cover whatever the adaptive theme setting says (see `adaptive::wear`), resolved through
//! `Theme::accent`. Art with no colour leaves no tint and the stage stays neutral. Rune springs
//! the gradients' colours when they change, so a cover change washes in rather than cuts, and
//! the breathing runs on Rune's loops: neither ever runs the view.

use rune::El;
use ui::{Hsla, Rgba, Springs, Theme};

use crate::Ui;

/// How many colours make up the field.
const COLORS: usize = 5;
/// How much of itself the field keeps under the dark overlay that holds lyrics readable over
/// it. The overlay is folded into the colours rather than painted as a sheet of its own, which
/// over an opaque field is the same picture and leaves a see-through window's alpha alone.
const SHADE: f32 = 0.55;

/// Whether the ambient background is on. Fullscreen reads it for the frosted glass on its
/// controls as well, which only has the field to blur.
pub(crate) fn shown(ui: &Ui) -> bool {
    ui.store.settings.ambient()
}

/// The field, `size` points across, behind the whole window. `corner` rounds it into the
/// window's own corners where the app draws them. Build it only while [`shown`].
pub(crate) fn field(ui: &Ui, size: [f32; 2], corner: Option<f32>) -> El {
    let theme = &ui.theme;
    // Without motion the field is a still gradient and asks for no frames. Either the system
    // preference (Rune holds loops still under it) or the setting of its own is enough.
    let still = !ui.store.settings.ambient_motion();
    // The field stands in for the page rather than sitting on it, so it carries the window's
    // own opacity.
    let clarity = match theme.transparent {
        true => theme.background.a,
        false => 1.,
    };
    let waves = ui::Waves::new(layered(colors(theme)).map(shaded))
        .stage(shaded(Hsla {
            a: 1.,
            ..theme.background
        }))
        .still(still)
        .corner_radius(corner.unwrap_or(0.))
        .build(theme, size);
    rune::el()
        .key("ambient")
        .absolute(0., 0.)
        .w_full()
        .h_full()
        .opacity(clarity)
        // a new cover's colours wash in over a second rather than cutting
        .motion(Springs::SLOW)
        .inert()
        .a11y_hidden()
        .child(waves)
}

/// The five field colours from the theme: the cover's leading hue carried from a light
/// highlight down to a dark shadow, plus its runner-up where the art names one. Art with no
/// colour leaves no tints behind, and then the stage is quiet neutrals.
pub(crate) fn colors(theme: &Theme) -> [Hsla; COLORS] {
    let neutral = |light: f32| Hsla {
        h: 0.06,
        s: 0.12,
        l: light,
        a: 1.,
    };
    // A dark theme gets a dark stage; lift the floor a little so the pools stay apart from the
    // stage.
    let floor = (0.13 + theme.background.l * 0.8).clamp(0.13, 0.30);

    let Some(tint) = theme.tint else {
        return [
            neutral(floor),
            neutral(floor + 0.025),
            neutral(floor + 0.05),
            neutral(floor + 0.015),
            neutral(floor + 0.04),
        ];
    };
    let accent = theme.accent(tint);
    let base = Hsla {
        h: accent.h,
        s: (accent.s * 0.9 + 0.04).clamp(0.45, 0.75),
        l: (0.40 + (accent.l - 0.45) * 0.5).clamp(0.28, 0.52),
        a: 1.,
    };
    let second = match theme.tint_secondary {
        Some(second) => Hsla {
            h: second.h,
            s: (second.s * 0.9).clamp(0.4, 0.72),
            l: (0.38 + (second.l - 0.45) * 0.5).clamp(0.28, 0.5),
            a: 1.,
        },
        None => neutral(floor + 0.03),
    };
    [
        base,
        Hsla {
            l: (base.l + 0.14).clamp(0.3, 0.62),
            s: (base.s - 0.08).clamp(0.4, 0.75),
            ..base
        },
        second,
        Hsla {
            l: (base.l - 0.13).clamp(0.18, 0.45),
            s: (base.s + 0.04).clamp(0.45, 0.8),
            ..base
        },
        Hsla {
            h: (base.h + 0.97).rem_euclid(1.),
            l: (base.l - 0.16).clamp(0.16, 0.4),
            s: (base.s - 0.02).clamp(0.4, 0.78),
            ..base
        },
    ]
}

/// The colours in the order the field lays them down, the runner-up last. Each pool covers the
/// ones under it, so drawn third it sank beneath the two shadows and a cover's second colour
/// barely showed.
fn layered(colors: [Hsla; COLORS]) -> [Hsla; COLORS] {
    let [base, highlight, second, shadow, deep] = colors;
    [base, highlight, shadow, deep, second]
}

/// A colour as the dark overlay leaves it. Black at `SHADE` over an opaque colour is that
/// colour's channels cut by what the sheet lets through, so cutting them here is the same
/// picture with one layer fewer to spend the window's alpha on.
fn shaded(color: Hsla) -> Hsla {
    let rgba = Rgba::from(color);
    let kept = 1. - SHADE;
    Hsla::from(Rgba {
        r: rgba.r * kept,
        g: rgba.g * kept,
        b: rgba.b * kept,
        a: rgba.a,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tinted(hue: f32) -> Theme {
        let mut theme = Theme::dark();
        theme.tint = Some(Hsla {
            h: hue,
            s: 0.6,
            l: 0.4,
            a: 1.,
        });
        theme.tint_secondary = Some(Hsla {
            h: (hue + 0.5).rem_euclid(1.),
            s: 0.6,
            l: 0.4,
            a: 1.,
        });
        theme
    }

    #[test]
    fn accent_theme_yields_its_family() {
        let colors = colors(&tinted(0.78));

        for (index, color) in colors.iter().enumerate() {
            // The runner-up sits in slot two; everything else is family.
            if index == 2 {
                let gap = (color.h - 0.28).abs().min(1. - (color.h - 0.28).abs());
                assert!(gap < 0.06, "second hue was {}", color.h);
                continue;
            }
            let gap = (color.h - 0.78).abs().min(1. - (color.h - 0.78).abs());
            assert!(gap < 0.06, "hue was {}", color.h);
        }
        assert!(colors[1].l > colors[0].l, "highlight lifts");
        assert!(colors[3].l < colors[0].l, "shadow sinks");
    }

    #[test]
    fn untinted_theme_yields_neutrals() {
        let colors = colors(&Theme::dark());

        assert!(colors.iter().all(|color| color.s < 0.2));
    }
}
