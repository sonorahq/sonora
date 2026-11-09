//! A hairline between two parts of a surface, in the theme's border colour.

use rune::{El, el};

use crate::theme::Theme;

const THICKNESS: f32 = 1.;

/// A one point rule that never shrinks. It is decoration, so screen readers skip it.
pub struct Separator {
    vertical: bool,
}

impl Separator {
    /// A rule across the width of its parent.
    pub fn horizontal() -> Self {
        Self { vertical: false }
    }

    /// A rule down the height of its parent.
    pub fn vertical() -> Self {
        Self { vertical: true }
    }

    pub fn build(self, theme: &Theme) -> El {
        let rule = el().fill(theme.border.color()).a11y_hidden();
        match self.vertical {
            true => rule.w(THICKNESS).h_full(),
            false => rule.h(THICKNESS).w_full(),
        }
    }
}
