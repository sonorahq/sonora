use gpui::prelude::*;
use gpui::{App, Div, FontWeight, Rems, SharedString, div, rems};

use crate::metrics::Text;
use crate::theme::ActiveTheme as _;

/// How far the tails of letters may hang below a clipped line, as a share of the font size. The
/// Arabic fallback font draws ر and ز lower than the line box leaves room for.
const TAILS: Rems = rems(0.25);
const TAILS_RETURNED: Rems = rems(-0.25);

/// Room below a clipped line of text for the tails of its letters.
pub trait Tails: Styled + Sized {
    /// The padding is inside the element's clip, so a tail that hangs past the line box shows, and
    /// the margin takes the same room back out of the layout, so nothing around the line moves.
    fn tails(self) -> Self {
        self.pb(TAILS).mb(TAILS_RETURNED)
    }
}

impl<T: Styled> Tails for T {}

pub fn eyebrow(label: impl Into<SharedString>, cx: &App) -> Div {
    faint(cx).child(upper(label))
}

pub fn faint(cx: &App) -> Div {
    let theme = cx.theme();

    div()
        .flex_none()
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .font_weight(FontWeight::SEMIBOLD)
}

pub fn upper(label: impl Into<SharedString>) -> SharedString {
    label.into().to_uppercase().into()
}

pub fn vacant(label: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();

    div()
        .flex()
        .w_full()
        .items_center()
        .justify_center()
        .p(theme.metrics.pad * 2.)
        .text_align(gpui::TextAlign::Center)
        .text_size(theme.text(Text::Label))
        .text_color(theme.muted_foreground)
        .child(div().min_w_0().child(label.into()))
}

pub fn heading(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .text_size(cx.theme().text(Text::Title))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.into())
}
