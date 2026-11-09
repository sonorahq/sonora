//! The square "E" a track marked explicit carries.

use rune::{El, icon, row};

use crate::theme::Theme;

/// How large the badge's corners are.
const CORNER: f32 = 2.;

/// The badge. The letter is drawn rather than set in type, so it sits in the exact middle of the
/// square whatever font and scale are in use. Screen readers skip it: a row that shows one says
/// "explicit" in its own description.
#[derive(Default)]
pub struct ExplicitBadge;

impl ExplicitBadge {
    pub fn new() -> Self {
        Self
    }

    pub fn build(self, theme: &Theme) -> El {
        // one rem, as the old badge measured itself
        let side = theme.font_size;
        row()
            .size(side, side)
            .radius(CORNER)
            .fill(theme.muted.color())
            .child(
                icon(icons::get("icons/explicit.svg"), side).color(theme.muted_foreground.color()),
            )
            .a11y_hidden()
    }
}
