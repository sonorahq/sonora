//! Lines of text in the app's voice: the faint eyebrow over a section, a heading, the muted line
//! an empty list shows, and their `spoken_*` twins that screen readers reach.

use rune::{El, TextStyle, col, text};

use crate::metrics::Text;
use crate::theme::Theme;

/// Font weights as GPUI named them.
pub struct Weight;

impl Weight {
    pub const NORMAL: f32 = 400.;
    pub const MEDIUM: f32 = 500.;
    pub const SEMIBOLD: f32 = 600.;
    pub const BOLD: f32 = 700.;
}

/// The type of one step of the theme's ladder at `weight`.
pub fn font(theme: &Theme, step: Text, weight: f32) -> TextStyle {
    TextStyle::new(theme.text(step), weight)
}

/// `label` in capitals, as an eyebrow draws it.
pub fn upper(label: impl AsRef<str>) -> String {
    label.as_ref().to_uppercase()
}

/// A small semibold muted line, the style an eyebrow and a section's caption share.
pub fn faint(label: impl Into<String>, theme: &Theme) -> El {
    text(label)
        .font(font(theme, Text::Small, Weight::SEMIBOLD))
        .color(theme.muted_foreground.color())
}

/// The capitals over a section. Read as plain text in capitals; [`spoken_eyebrow`] keys it
/// and reads it as written.
pub fn eyebrow(label: impl AsRef<str>, theme: &Theme) -> El {
    faint(upper(label), theme)
}

/// A page or section title in the title step, semibold. Read as plain text; [`spoken_heading`]
/// lists it among the page's headings.
pub fn heading(label: impl Into<String>, theme: &Theme) -> El {
    text(label)
        .font(font(theme, Text::Title, Weight::SEMIBOLD))
        .color(theme.foreground.color())
}

/// The muted, centred line an empty list or page shows, padded off its edges.
pub fn vacant(label: impl Into<String>, theme: &Theme) -> El {
    col()
        .w_full()
        .center()
        .padding(theme.metrics.pad * 2.)
        .child(vacant_line(label.into(), theme))
}

/// A line of static text that screen readers reach under `key`.
pub fn spoken(key: impl Into<String>, label: impl Into<String>, theme: &Theme) -> El {
    let label = label.into();
    text(label.clone())
        .key(key)
        .role("text")
        .label(label)
        .font(font(theme, Text::Body, Weight::NORMAL))
        .color(theme.foreground.color())
}

/// A [`heading`] that screen readers reach under `key` and list among the page's headings at
/// `level`: one for a page title, two for a section.
pub fn spoken_heading(
    key: impl Into<String>,
    label: impl Into<String>,
    level: u32,
    theme: &Theme,
) -> El {
    let label = label.into();
    text(label.clone())
        .key(key)
        .role("heading")
        .level(level)
        .label(label)
        .font(font(theme, Text::Title, Weight::SEMIBOLD))
        .color(theme.foreground.color())
}

/// An [`eyebrow`] that screen readers reach under `key`. It reads the label as written rather
/// than in the capitals it is drawn in, which some readers would spell out letter by letter.
pub fn spoken_eyebrow(key: impl Into<String>, label: impl Into<String>, theme: &Theme) -> El {
    let label = label.into();
    text(upper(&label))
        .key(key)
        .role("text")
        .label(label)
        .font(font(theme, Text::Small, Weight::SEMIBOLD))
        .color(theme.muted_foreground.color())
}

/// A [`vacant`] line that screen readers reach under `key`, for an empty list or page that would
/// otherwise say nothing at all.
pub fn spoken_vacant(key: impl Into<String>, label: impl Into<String>, theme: &Theme) -> El {
    let label = label.into();
    col()
        .key(key)
        .role("text")
        .label(label.clone())
        .w_full()
        .center()
        .padding(theme.metrics.pad * 2.)
        .child(vacant_line(label, theme).a11y_hidden())
}

/// The muted line itself, wrapping and centred.
fn vacant_line(label: String, theme: &Theme) -> El {
    text(label)
        .font(font(theme, Text::Label, Weight::NORMAL))
        .color(theme.muted_foreground.color())
        .align(rune::Align::Center)
        .wrap()
}
