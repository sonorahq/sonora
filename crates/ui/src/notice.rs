//! A framed message with an icon and a title, for something the page has to say about itself:
//! a failed load, a provider that is signed out.

use rune::{El, TextStyle, col, icon, row, text};

use crate::label::Weight;
use crate::metrics::Text;
use crate::paint::rems;
use crate::theme::Theme;

const ICON: f32 = 18.;
const REACH: f32 = 520.;
const BORDER: f32 = 1.;
/// How much of the secondary fill shows through the frame.
const FILL: f32 = 0.45;

pub struct Notice {
    title: String,
    message: String,
    icon: String,
    failed: bool,
    centered: bool,
    /// The key under which the notice reaches screen readers as one group.
    spoken: Option<String>,
}

impl Notice {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            icon: "icons/circle-alert.svg".to_owned(),
            failed: false,
            centered: false,
            spoken: None,
        }
    }

    /// Tints the icon in the danger colour, for something that went wrong.
    pub fn failed(mut self) -> Self {
        self.failed = true;
        self
    }

    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = path.into();
        self
    }

    /// Exposes the notice to screen readers as one group named by its title and described by
    /// its message, under `key`. Without it the title and message are read as plain text.
    pub fn spoken(mut self, key: impl Into<String>) -> Self {
        self.spoken = Some(key.into());
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let tint = match self.failed {
            true => theme.danger,
            false => theme.muted_foreground,
        };
        let spoken = self.spoken.is_some();
        let hide = |e: El| match spoken {
            true => e.a11y_hidden(),
            false => e,
        };
        let align = match self.centered {
            true => rune::Align::Center,
            false => rune::Align::Start,
        };
        let head = row()
            .center_y()
            .gap(rems(theme, 0.5))
            .child(
                icon(icons::get(&self.icon), ICON)
                    .color(tint.color())
                    .a11y_hidden(),
            )
            .child(hide(
                text(self.title.clone())
                    .font(TextStyle::new(theme.text(Text::Body), Weight::MEDIUM))
                    .color(theme.foreground.color()),
            ));
        let mut notice = col()
            .gap(rems(theme, 0.5))
            .min_w(0.)
            .max_w(REACH)
            .padding(theme.metrics.pad * 1.5)
            .radius(theme.radius)
            .border(BORDER, theme.border.color())
            .fill(theme.secondary.opacity(FILL).color())
            .child(head)
            .child(hide(
                text(self.message.clone())
                    .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .align(align)
                    .wrap(),
            ));
        if self.centered {
            notice = notice.items_center();
        }
        match self.spoken {
            Some(key) => notice
                .key(key)
                .role("group")
                .label(self.title)
                .description(self.message),
            None => notice,
        }
    }
}
