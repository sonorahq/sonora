//! Frosted surfaces: a translucent popover fill over a backdrop blur of what the app drew under
//! it, or a plain fill when the blur setting is off.
//!
//! Rune blurs at most `rune::MAX_BACKDROP_BLURS` elements a frame and draws the fill alone for
//! the rest, so a frost belongs on surfaces, never on every row or button.

use rune::El;

use crate::color::Hsla;
use crate::theme::Theme;

/// How much of the popover colour a glass control keeps over what it blurs.
pub const GLASS_FILL: f32 = 0.2;
/// How hard a glass control blurs what is behind it, as a Gaussian sigma in points.
pub const GLASS_BLUR: f32 = 8.;
/// Raised surfaces keep contrast even over bright artwork, while a faint backdrop shows.
const POPOVER_FILL: f32 = 0.82;
/// How much of the popover colour a frosted bar keeps, and how far it blurs what passes under.
/// Both are thicker than a menu's, since a bar sits over the page for good and its text has to
/// read over whatever scrolls by.
const BAR_FILL: f32 = 0.86;
const BAR_BLUR: f32 = 18.;
/// The bar's fill on a see-through window, where the page wash already sits under the bar.
const CLEAR_BAR_FILL: f32 = 0.3;

/// Whether a surface that frosts only for the look of it should frost at all. The setting turns
/// the blurs off, and each one falls back to a flat fill.
pub fn blurring(theme: &Theme) -> bool {
    theme.blur
}

/// Styles `e` as frosted glass: a faint popover fill over a hard blur of what it floats on. A
/// site the blur setting reaches asks [`blurring`] first, since the thin fill needs the blur.
pub fn glass(e: El, theme: &Theme) -> El {
    e.fill(theme.popover.opacity(GLASS_FILL).color())
        .backdrop_blur(GLASS_BLUR)
}

/// The fill shared by menus and raised translucent panels: opaque without the blur, since there
/// is no wash behind the text to keep its contrast.
pub fn popover_fill(theme: &Theme) -> Hsla {
    match blurring(theme) {
        true => theme.popover.opacity(POPOVER_FILL),
        false => theme.popover,
    }
}

/// Gives a raised panel the fill and backdrop blur of a menu.
pub fn popover_surface(e: El, theme: &Theme) -> El {
    let e = e.fill(popover_fill(theme).color());
    match blurring(theme) {
        true => frost(e),
        false => e,
    }
}

/// Styles chrome that stays over scrolling content, such as the player bar or a table's head: a
/// thick popover fill over a wide blur. The caller asks [`blurring`] first. A see-through window
/// gets a thinner fill so the bar lets the desktop through about as much as the page does.
pub fn bar_surface(e: El, theme: &Theme) -> El {
    let fill = match theme.transparent {
        true => CLEAR_BAR_FILL,
        false => BAR_FILL,
    };
    e.fill(theme.popover.opacity(fill).color())
        .backdrop_blur(BAR_BLUR)
}

/// Blurs what is behind `e` by the glass width and nothing more, for a fill that is already
/// translucent.
pub fn frost(e: El) -> El {
    e.backdrop_blur(GLASS_BLUR)
}
