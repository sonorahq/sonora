//! What the elements share to paint and wire themselves: the handlers a builder holds until it
//! knows whether it is enabled, the press overlay, and the shadows of GPUI's scale.

use rune::{Color, El, Shadow, Tx};

/// A handler waiting to be put on the element a builder makes.
type Apply = Box<dyn FnOnce(El) -> El>;

/// The handlers a builder was given, kept until `build` knows whether the element takes input.
/// A disabled element drops them, since Rune's `disabled` only greys it for assistive technology.
#[derive(Default)]
pub(crate) struct Press(Vec<Apply>);

impl Press {
    pub(crate) fn press<S: 'static>(&mut self, f: impl Fn(&mut S) + 'static) {
        self.0.push(Box::new(move |e: El| e.on_press(f)));
    }

    pub(crate) fn press_tx<S: 'static>(&mut self, f: impl Fn(&mut S, &mut Tx) + 'static) {
        self.0.push(Box::new(move |e: El| e.on_press_tx(f)));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Puts the handlers on `e` when `enabled`, and drops them otherwise.
    pub(crate) fn apply(self, mut e: El, enabled: bool) -> El {
        if enabled {
            for f in self.0 {
                e = f(e);
            }
        }
        e
    }
}

/// The overlay that, laid over the hover overlay the runtime already shows, gives `pressed` over
/// the fill: the press swaps hover for pressed rather than stacking the two.
pub(crate) fn press_over(hover: Color, pressed: Color) -> Color {
    if hover.a >= 0.999 {
        return pressed;
    }
    let a = ((pressed.a - hover.a) / (1. - hover.a)).clamp(0., 1.);
    pressed.alpha(a)
}

/// GPUI's `shadow_sm`: one soft line under the element.
pub(crate) fn shadow_sm(e: El) -> El {
    e.box_shadow(shadow(1., 2., 0., 0.05))
}

/// GPUI's `shadow_md`, for surfaces that float over the page: menus, popovers, dialogs.
pub(crate) fn shadow_md(e: El) -> El {
    e.box_shadow(shadow(4., 6., -1., 0.1))
        .box_shadow(shadow(2., 4., -2., 0.1))
}

/// GPUI's `shadow_lg`, for a toast standing over everything.
pub(crate) fn shadow_lg(e: El) -> El {
    e.box_shadow(shadow(10., 15., -3., 0.1))
        .box_shadow(shadow(4., 6., -4., 0.1))
}

fn shadow(y: f32, blur: f32, spread: f32, alpha: f32) -> Shadow {
    Shadow {
        y,
        blur,
        spread,
        color: Color::BLACK.alpha(alpha),
        ..Shadow::default()
    }
}

/// `n` rems: the old layout's spacing unit, which followed the theme's font size.
pub(crate) fn rems(theme: &crate::theme::Theme, n: f32) -> f32 {
    theme.font_size * n
}
