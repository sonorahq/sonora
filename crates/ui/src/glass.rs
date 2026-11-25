use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, px};

use crate::theme::{ActiveTheme as _, BACKDROP_TRANSPARENCY, Theme};

/// How much of the popover colour a glass control keeps over what it blurs.
pub const GLASS_FILL: f32 = 0.2;
/// How hard a glass control blurs what is behind it.
pub const GLASS_BLUR: Pixels = px(8.);
/// Raised surfaces retain contrast even over bright artwork, while a faint backdrop remains visible.
const POPOVER_FILL: f32 = 0.82;
/// How much of the popover colour a frosted bar keeps, and how far it blurs what passes under.
/// Both are thicker than a menu's, since a bar sits over the page for good and its text has to
/// read over whatever scrolls by.
const BAR_FILL: f32 = 0.86;
const BAR_BLUR: Pixels = px(18.);
/// The thinnest bar fill, which a window at the default transparency or clearer gets. The fill
/// stacks on the page wash under the bar, so a thick one would leave the bar far darker than the
/// window around it.
const CLEAR_BAR_FILL: f32 = 0.3;

/// Whether a surface that only frosts for the look of it should frost at all. A backdrop blur
/// is the priciest thing the renderer does per frame, so the setting turns those off and each
/// one falls back to a flat fill. The settings header and the fullscreen chrome frost whatever
/// it answers: their haze is what keeps text readable over the content passing under it.
pub fn blurring(cx: &App) -> bool {
    cx.theme().blur
}

/// Styles an element as frosted glass: a faint popover fill over a hard blur of
/// whatever it floats on. Over a flat colour the blur shows nothing, so the fill is all
/// that separates the control from the page there. A site the blur setting reaches asks
/// `blurring` before calling this, since the thin fill needs the wash to read.
pub fn glass<E: Styled>(element: E, cx: &App) -> E {
    let theme = cx.theme();

    element
        .bg(theme.popover.opacity(GLASS_FILL))
        .backdrop_blur(GLASS_BLUR)
}

/// The fill shared by context menus and raised translucent panels. With blur disabled it
/// becomes opaque, since there is no wash behind the text to preserve its contrast.
pub fn popover_fill(cx: &App) -> Hsla {
    let theme = cx.theme();
    match blurring(cx) {
        true => theme.popover.opacity(POPOVER_FILL),
        false => theme.popover,
    }
}

/// Gives a raised panel the same fill, foreground and backdrop blur as a context menu.
pub fn popover_surface<E: Styled>(element: E, cx: &App) -> E {
    let element = element
        .bg(popover_fill(cx))
        .text_color(cx.theme().popover_foreground);
    match blurring(cx) {
        true => frost(element),
        false => element,
    }
}

/// Styles chrome that stays over scrolling content, the player bar and a table's head: a thick
/// popover fill over a wide blur. The caller asks `blurring` first, since without the blur the
/// thin fill would leave the text on the bare page. A see-through window gets a thinner fill so
/// the bar lets the desktop through about as much as the page does.
pub fn bar_surface<E: Styled>(element: E, cx: &App) -> E {
    let theme = cx.theme();
    element
        .bg(theme.popover.opacity(bar_fill(theme)))
        .backdrop_blur(BAR_BLUR)
}

/// Blurs what is behind an element by the glass width and nothing more, for a hover fill
/// that is already translucent. Only hovers that float over real content go through it;
/// on flat paint the blur shows nothing, so those stay plain fills. The caller asks
/// `blurring` first, since this one has no theme to read.
pub fn frost<E: Styled>(element: E) -> E {
    element.backdrop_blur(GLASS_BLUR)
}

/// How much of the popover colour a bar keeps on `theme`'s window. A fill `f` over a page that
/// lets `clear` of the desktop through hides `f * clear` more of it than the page does, so the
/// fill thins as the window clears, to hide no more than the thin fill does at the default
/// transparency. A window that is barely see-through keeps the full fill.
fn bar_fill(theme: &Theme) -> f32 {
    let clear = (1. - theme.opacity).max(f32::EPSILON);
    (CLEAR_BAR_FILL * BACKDROP_TRANSPARENCY / clear).clamp(CLEAR_BAR_FILL, BAR_FILL)
}
