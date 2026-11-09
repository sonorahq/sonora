//! A titled block of facts or settings on a detail page, and the label and value rows it holds.

use rune::{El, TextStyle, col, row, text};

use crate::label::{Weight, eyebrow};
use crate::metrics::Text;
use crate::paint::rems;
use crate::theme::Theme;

const BORDER: f32 = 1.;
/// How much of the secondary fill shows through the card.
const FILL: f32 = 0.45;
/// How much of the table hover a striped fact shows.
const STRIPE: f32 = 0.35;

/// A card with an eyebrow title over its children.
pub struct InfoCard {
    title: String,
    /// The key under which the card reaches screen readers as a group named by its title.
    spoken: Option<String>,
    children: Vec<El>,
    borderless: bool,
    stretch: bool,
    plain: bool,
    untitled: bool,
    flush: bool,
}

impl InfoCard {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            spoken: None,
            children: Vec::new(),
            borderless: false,
            stretch: false,
            plain: false,
            untitled: false,
            flush: false,
        }
    }

    pub fn child(mut self, child: El) -> Self {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = El>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn borderless(mut self) -> Self {
        self.borderless = true;
        self
    }

    /// Fills the height of its parent, for cards side by side that should end level.
    pub fn stretch(mut self) -> Self {
        self.stretch = true;
        self
    }

    /// Lays the title and the content out as the card does, with no frame, fill or inset around
    /// them, for a page that separates its parts by space alone.
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }

    /// Leaves the eyebrow title out, for a card whose heading sits outside it.
    pub fn untitled(mut self) -> Self {
        self.untitled = true;
        self
    }

    /// Drops the inset and the gap between children for a thin band above and below, so rows that
    /// pad themselves run edge to edge and read as one surface.
    pub fn flush(mut self) -> Self {
        self.flush = true;
        self
    }

    /// Exposes the card to screen readers as a group named by its title, under `key`.
    pub fn spoken(mut self, key: impl Into<String>) -> Self {
        self.spoken = Some(key.into());
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let mut card = col().min_w(0.).clip_rounded(match self.plain {
            true => 0.,
            false => theme.radius,
        });
        if self.stretch {
            card = card.h_full();
        }
        card = match self.flush {
            true => card.py(rems(theme, 0.25)),
            false => card.gap(rems(theme, 1.)),
        };
        if !self.plain {
            if !self.flush {
                card = card.padding(rems(theme, 1.25));
            }
            card = card
                .radius(theme.radius)
                .fill(theme.secondary.opacity(FILL).color());
            if !self.borderless {
                card = card.border(BORDER, theme.border.color());
            }
        }
        if !self.untitled {
            let title = eyebrow(&self.title, theme);
            card = card.child(match self.spoken.is_some() {
                true => title.a11y_hidden(),
                false => title,
            });
        }
        let card = card.children(self.children);
        match self.spoken {
            Some(key) => card.key(key).role("group").label(self.title),
            None => card,
        }
    }
}

/// One fact: a muted label and its value, set against each other across the row.
pub struct Fact {
    label: String,
    value: String,
    /// The key under which the label and the value reach screen readers as one row.
    spoken: Option<String>,
    striped: bool,
}

impl Fact {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            spoken: None,
            striped: false,
        }
    }

    pub fn striped(mut self, striped: bool) -> Self {
        self.striped = striped;
        self
    }

    /// Exposes the fact to screen readers under `key`, named by its label with its value as the
    /// value read after it.
    pub fn spoken(mut self, key: impl Into<String>) -> Self {
        self.spoken = Some(key.into());
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let style = TextStyle::new(theme.text(Text::Body), Weight::NORMAL);
        let spoken = self.spoken.is_some();
        let hide = |e: El| match spoken {
            true => e.a11y_hidden(),
            false => e,
        };
        let mut fact = row()
            .items_start()
            .justify_between()
            .gap(rems(theme, 1.))
            .min_w(0.)
            .px(theme.metrics.pad)
            .py(theme.metrics.pad / 2.)
            .radius(theme.radius)
            .child(hide(
                text(self.label.clone())
                    .font(style)
                    .color(theme.muted_foreground.color())
                    .grow(1.)
                    .shrink()
                    .min_w(0.)
                    .wrap(),
            ))
            .child(hide(
                text(self.value.clone())
                    .font(TextStyle::new(theme.text(Text::Body), Weight::MEDIUM))
                    .color(theme.foreground.color())
                    .align(rune::Align::End)
                    .grow(1.)
                    .shrink()
                    .min_w(0.)
                    .wrap(),
            ));
        if self.striped {
            fact = fact.fill(theme.table_hover.opacity(STRIPE).color());
        }
        match self.spoken {
            Some(key) => fact
                .key(key)
                .role("text")
                .label(self.label)
                .a11y_value(self.value),
            None => fact,
        }
    }
}
