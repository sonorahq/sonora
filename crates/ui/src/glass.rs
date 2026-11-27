use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, px};

use crate::theme::ActiveTheme as _;

/// How much of the popover colour a glass control keeps over what it blurs.
pub const GLASS_FILL: f32 = 0.2;
/// How hard a glass control blurs what is behind it.
pub const GLASS_BLUR: Pixels = px(8.);
/// Raised surfaces retain contrast even over bright artwork, while a faint backdrop remains visible.
const POPOVER_FILL: f32 = 0.82;
/// How far a frosted bar mixes what passes under it towards the popover colour, how far it
/// blurs it and how much it saturates it first. The fill and the blur are thicker than a
/// menu's, since a bar sits over the page for good and its text has to read over whatever
/// scrolls by. The saturation lets the colours under the fill come through as a glow, so the
/// bar still reads as glass.
const BAR_FILL: f32 = 0.7;
const BAR_BLUR: Pixels = px(18.);
const BAR_SATURATION: f32 = 1.8;

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

/// Styles chrome that stays over scrolling content, the player bar and a table's head: a wide,
/// saturated blur mixed most of the way to the popover colour at the window's own opacity. On a
/// see-through window the bar then hides most of what scrolls under it, and over the page's own
/// ground lets the desktop through as much as the page does. Artwork under it hides the desktop
/// itself, and the bar keeps a little of that density there, as glass would. The caller asks
/// `blurring` first, since without the blur the text would sit on the bare page.
pub fn bar_surface<E: Styled>(element: E, cx: &App) -> E {
    let theme = cx.theme();
    element
        .backdrop_blur(BAR_BLUR)
        .backdrop_saturate(BAR_SATURATION)
        .backdrop_tint(theme.popover.opacity(theme.opacity), BAR_FILL)
}

/// Blurs what is behind an element by the glass width and nothing more, for a hover fill
/// that is already translucent. Only hovers that float over real content go through it;
/// on flat paint the blur shows nothing, so those stay plain fills. The caller asks
/// `blurring` first, since this one has no theme to read.
pub fn frost<E: Styled>(element: E) -> E {
    element.backdrop_blur(GLASS_BLUR)
}
