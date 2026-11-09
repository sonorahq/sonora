//! The block an empty page or list shows: a faint glyph, a muted line, an optional reason under
//! it and an optional action.

use rune::{El, TextStyle, col, icon, text};

use crate::label::{Weight, spoken_vacant, vacant};
use crate::metrics::Text;
use crate::paint::rems;
use crate::theme::Theme;

/// How strongly the glyph shows.
const GLYPH: f32 = 0.35;
/// The glyph's size against a cover, regular and compact.
const GLYPH_SIZE: f32 = 0.5;
const COMPACT_SIZE: f32 = 0.22;
/// How strongly the reason shows against the label.
const DETAIL: f32 = 0.8;
/// How wide the label and its reason may run, in covers.
const WORDS: f32 = 2.5;

pub struct Vacancy {
    label: String,
    detail: Option<String>,
    icon: Option<String>,
    action: Option<El>,
    compact: bool,
    /// The key under which the label reaches screen readers as one line.
    spoken: Option<String>,
}

impl Vacancy {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: None,
            icon: None,
            action: None,
            compact: false,
            spoken: None,
        }
    }

    /// A smaller glyph, for a narrow place like the sidebar.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// A second, quieter line under the label, for the reason behind an empty page.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = Some(path.into());
        self
    }

    /// An element under the text, usually a button that fills the page.
    pub fn action(mut self, action: El) -> Self {
        self.action = Some(action);
        self
    }

    /// Exposes the label to screen readers as one line described by the detail, under `key`.
    pub fn spoken(mut self, key: impl Into<String>) -> Self {
        self.spoken = Some(key.into());
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let glyph = theme.metrics.cover
            * match self.compact {
                true => COMPACT_SIZE,
                false => GLYPH_SIZE,
            };
        let mut vacancy = col().w_full().center();
        if let Some(path) = &self.icon {
            vacancy = vacancy.child(
                icon(icons::get(path), glyph)
                    .mt(match self.compact {
                        true => theme.metrics.pad,
                        false => theme.metrics.inset,
                    })
                    .color(theme.muted_foreground.opacity(GLYPH).color())
                    .a11y_hidden(),
            );
        }
        let words = match (self.detail.clone(), self.spoken) {
            (Some(detail), Some(key)) => caption(self.label.clone(), detail.clone(), true, theme)
                .key(key)
                .role("text")
                .label(self.label)
                .description(detail),
            (Some(detail), None) => caption(self.label, detail, false, theme),
            (None, Some(key)) => spoken_vacant(key, self.label, theme),
            (None, None) => vacant(self.label, theme),
        };
        vacancy.child(words).children(self.action)
    }
}

/// The label and its reason stacked, padded the way `vacant` pads its one line.
fn caption(label: String, detail: String, hidden: bool, theme: &Theme) -> El {
    let hide = |e: El| match hidden {
        true => e.a11y_hidden(),
        false => e,
    };
    col()
        .items_center()
        .gap(rems(theme, 0.25))
        .padding(theme.metrics.pad * 2.)
        .max_w(theme.metrics.cover * WORDS)
        .child(hide(
            text(label)
                .font(TextStyle::new(theme.text(Text::Label), Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .align(rune::Align::Center)
                .wrap(),
        ))
        .child(hide(
            text(detail)
                .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                .color(theme.muted_foreground.opacity(DETAIL).color())
                .align(rune::Align::Center)
                .wrap(),
        ))
}
