use crate::Text;
use crate::theme::ActiveTheme as _;
use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Div, FontWeight, IntoElement, RenderOnce, SharedString, StyleRefinement, Window, div,
    rems, svg,
};

/// The badge's side, in rems.
const SIDE: f32 = 1.;
/// How much of the side an icon pack glyph takes, which leaves it the margin the drawn "E" has.
const GLYPH: f32 = 0.75;

/// A small filled square beside a title that marks what the item is, such as explicit or a local
/// file, or a short word in a pill of the same height. It carries no name of its own, so the
/// caller wraps it in a node that announces it.
#[derive(IntoElement)]
pub struct Badge {
    base: Div,
    mark: Mark,
}

/// A compact, accessible label for player metadata that stays on one line.
#[derive(IntoElement)]
pub struct AudioQualityBadge {
    base: Div,
    id: &'static str,
    text: SharedString,
    aria_label: SharedString,
    hi_res: bool,
}

impl AudioQualityBadge {
    /// Creates a badge for audio quality, with a localized accessibility label.
    pub fn new(
        id: &'static str,
        text: impl Into<SharedString>,
        aria_label: impl Into<SharedString>,
    ) -> Self {
        Self {
            base: div(),
            id,
            text: text.into(),
            aria_label: aria_label.into(),
            hi_res: false,
        }
    }

    /// Marks audio at 24 bits and above CD sample rates as hi-res.
    pub fn hi_res(mut self) -> Self {
        self.hi_res = true;
        self
    }
}

impl RenderOnce for AudioQualityBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();

        self.base
            .id(self.id)
            .role(Role::Label)
            .aria_label(self.aria_label)
            .h(rems(SIDE))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .whitespace_nowrap()
            .px(theme.metrics.pad / 2.)
            .bg(match self.hi_res {
                true => theme.status_warning.opacity(0.08),
                false => theme.muted,
            })
            .rounded_xs()
            .text_size(theme.text(Text::Tiny))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(match self.hi_res {
                true => theme.status_warning.opacity(0.7),
                false => theme.muted_foreground,
            })
            .child(self.text)
    }
}

/// What a badge shows.
enum Mark {
    /// The drawn "E", which sits in the exact middle of the square whatever font and scale are in
    /// use, an equal gap above and below it.
    Explicit,
    /// A glyph from the active icon pack, inset from the edges.
    Icon(SharedString),
    /// A short word, already resolved from Fluent, that widens the square into a pill.
    Text(SharedString),
}

impl Badge {
    /// The square "E" a track marked explicit carries.
    pub fn explicit() -> Self {
        Self {
            base: div(),
            mark: Mark::Explicit,
        }
    }

    /// A badge showing `icon`, spelled `"icons/<name>.svg"`.
    pub fn icon(icon: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            mark: Mark::Icon(icon.into()),
        }
    }

    /// A badge showing `text`, which the caller resolves from Fluent in its render.
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            mark: Mark::Text(text.into()),
        }
    }
}

impl Styled for Badge {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { mut base, mark } = self;
        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());
        let glyph = |path: SharedString, side: f32| {
            svg()
                .path(path)
                .size(rems(side))
                .text_color(theme.muted_foreground)
        };

        let badge = base
            .h(rems(SIDE))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .bg(theme.muted)
            .rounded_xs();
        let mut badge = match mark {
            Mark::Explicit => badge
                .w(rems(SIDE))
                .child(glyph(icons::path("icons/explicit.svg"), SIDE)),
            Mark::Icon(icon) => badge.w(rems(SIDE)).child(glyph(icons::path(&icon), GLYPH)),
            Mark::Text(text) => badge
                .px(theme.metrics.pad / 2.)
                .text_size(theme.text(Text::Tiny))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text),
        };
        badge.style().refine(&overrides);
        badge
    }
}
