//! A navigation row as the sidebars draw it.

use rune::El;
use ui::{Button, Hsla};

/// The space between a navigation row's icon and its label.
const GAP: f32 = 10.;

/// A navigation row: a ghost button with its label (the Fluent message `label`) in `tint`. The
/// caller adds the icon and the press, builds it, and passes the built row to [`nav_lay`].
pub(crate) fn nav_row(key: impl Into<String>, label: &str, tint: Hsla) -> Button {
    Button::new(key)
        .ghost()
        .label(i18n::lookup(label, None))
        .tint(tint)
}

/// Lays a built navigation row out as the sidebars want it: its label at the start, and the
/// `accent` fill while hovered or pressed, the same fill the caller lights the current row with.
pub(crate) fn nav_lay(row: El, accent: Hsla) -> El {
    row.justify_start().gap(GAP).hover(accent.color())
}
