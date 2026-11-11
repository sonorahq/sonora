use gpui::prelude::*;
use gpui::{ElementId, Hsla};
use ui::Button;

/// A navigation row as the sidebars draw it: a ghost button with its label at the start, in
/// `tint`, that takes `accent` while hovered or pressed and lets it fade once the pointer
/// leaves. The caller adds the icon and lights the row it considers current with
/// `.lit(true, accent)`.
pub(crate) fn nav_row(
    id: impl Into<ElementId>,
    key: &'static str,
    tint: Hsla,
    accent: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .label(i18n::lookup(key, None))
        .tint(tint)
        .gap_2p5()
        .justify_start()
        .lit(false, accent)
        .active(move |style| style.bg(accent))
}

/// A navigation row reduced to its icon, named by a tooltip beside it.
pub(crate) fn nav_icon(
    id: impl Into<ElementId>,
    key: &'static str,
    icon: &'static str,
    tint: Hsla,
    accent: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .icon(icon)
        .tooltip_trailing(key)
        .tint(tint)
        .w_full()
        .lit(false, accent)
        .active(move |style| style.bg(accent))
}
