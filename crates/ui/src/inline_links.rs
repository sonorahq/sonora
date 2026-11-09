//! A line of comma-separated names, each pressable when it leads somewhere, such as a track's
//! artists. It is one text with links in it, so it wraps or ends in an ellipsis as a whole.

use std::rc::Rc;

use rune::{El, Link, LinkPaint, TextStyle, Tx, rich, text};

use crate::color::Hsla;
use crate::metrics::Text;
use crate::theme::Theme;

const SEPARATOR: &str = ", ";

type LinkFn<S> = Rc<dyn Fn(&mut S, &str, &mut Tx)>;

/// One name in the line, and what pressing it opens. A name without a value is plain text.
#[derive(Clone, Debug)]
pub struct InlineLink {
    pub label: String,
    pub value: Option<String>,
}

impl InlineLink {
    pub fn new(label: impl Into<String>, value: Option<String>) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }
}

/// The names joined by commas, the ones with values underlined under the pointer and reported to
/// `on_link` when pressed. With no names it shows `fallback`.
pub struct InlineLinks<S: 'static> {
    key: String,
    items: Vec<InlineLink>,
    fallback: String,
    color: Hsla,
    size: Option<f32>,
    clip: bool,
    on_link: Option<LinkFn<S>>,
}

impl<S: 'static> InlineLinks<S> {
    pub fn new(
        key: impl Into<String>,
        items: impl IntoIterator<Item = InlineLink>,
        fallback: impl Into<String>,
        color: Hsla,
    ) -> Self {
        Self {
            key: key.into(),
            items: items.into_iter().collect(),
            fallback: fallback.into(),
            color,
            size: None,
            clip: false,
            on_link: None,
        }
    }

    /// The text size, in place of the body size.
    pub fn text_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Keeps the line to one line, ending in an ellipsis when it does not fit.
    pub fn truncate(mut self) -> Self {
        self.clip = true;
        self
    }

    /// The value of a pressed name.
    pub fn on_link(mut self, f: impl Fn(&mut S, &str, &mut Tx) + 'static) -> Self {
        self.on_link = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let Self {
            key,
            items,
            fallback,
            color,
            size,
            clip,
            on_link,
        } = self;
        let font = TextStyle::new(size.unwrap_or_else(|| theme.text(Text::Body)), 400.);
        let line = match items.is_empty() {
            true => text(fallback),
            false => {
                let mut content = String::new();
                let mut links = Vec::new();
                for (index, item) in items.into_iter().enumerate() {
                    if index > 0 {
                        content.push_str(SEPARATOR);
                    }
                    let start = content.len();
                    content.push_str(&item.label);
                    if let (Some(value), true) = (item.value, on_link.is_some()) {
                        links.push(
                            Link::new(start..content.len(), value)
                                .label(item.label)
                                .hover(LinkPaint::new().underline(color.color())),
                        );
                    }
                }
                let mut line = rich(content, Vec::new());
                for link in links {
                    line = line.link(link);
                }
                match on_link {
                    Some(on_link) => line
                        .on_link(move |s: &mut S, value: &str, tx: &mut Tx| on_link(s, value, tx)),
                    None => line,
                }
            }
        };
        let line = line.key(key).font(font).color(color.color()).min_w(0.);
        match clip {
            true => line.truncate().shrink(),
            false => line,
        }
    }
}
