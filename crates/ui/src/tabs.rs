//! Tabs: a pill of segment buttons whose selected fill flies from tab to tab, and the indented
//! list of sub-pages under a sidebar entry.

use rune::{El, col, el, row};

use crate::button::Button;
use crate::glass::{blurring, glass};
use crate::paint::{rems, shadow_sm};
use crate::theme::Theme;

const BORDER: f32 = 1.;
/// The bar's padding in rems; the items sit this far inside its curve.
const PAD: f32 = 0.25;
/// How wide the fade is at an edge with more tabs behind it.
const FADE: f32 = 32.;
/// The vertical list's indent, the guide line's width and where it runs.
const LINE: f32 = 1.;
const INDENT: f32 = 16.;
const ICON: f32 = 20.;

/// A row of segment buttons in one pill. The bar is as wide as its items until given a
/// [`TabBar::max_width`], where it stops and scrolls them sideways, each edge with tabs behind
/// it fading them out. The selected tab's fill is one element that flies to the next tab when
/// the selection moves. To bring a tab chosen elsewhere into view, call
/// `tx.scroll_into_view(key)` with its button's key.
pub struct TabBar {
    key: String,
    name: Option<String>,
    items: Vec<Button>,
    blurred: bool,
    max_width: Option<f32>,
    radius: Option<f32>,
}

impl TabBar {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: None,
            items: Vec::new(),
            blurred: false,
            max_width: None,
            radius: None,
        }
    }

    /// Names the bar for screen readers, read on entering it before the tab in focus.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The tabs, one button each; the one marked `selected` is the current tab.
    pub fn items(mut self, items: impl IntoIterator<Item = Button>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    /// A bar that floats over content: it casts a soft shadow, and draws as glass while the blur
    /// setting is on.
    pub fn blurred(mut self) -> Self {
        self.blurred = true;
        self
    }

    /// Caps the bar's width; tabs that do not fit scroll sideways.
    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }

    /// The bar's corner radius in place of the theme's. The tabs stay concentric inside it.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let rounding = self.radius.unwrap_or(theme.radius).max(0.);
        let pad = rems(theme, PAD);
        let inner = (rounding - pad).max(0.);
        let count = self.items.len();
        let indicator = format!("{}-indicator", self.key);
        let tabs = self.items.into_iter().enumerate().map(|(index, mut item)| {
            if item.selected {
                item.underlay = Some(
                    el().key("indicator")
                        .matched(indicator.clone())
                        .absolute(0., 0.)
                        .w_full()
                        .h_full()
                        .radius(inner)
                        .fill(theme.secondary_active.color())
                        .a11y_hidden(),
                );
            }
            item.tab()
                .build(theme)
                .radius(inner)
                .position_in_set(index + 1, count)
        });
        let mut list = row()
            .key(format!("{}-tabs", self.key))
            .role("tablist")
            .focus_group()
            .center_y()
            .gap(rems(theme, 0.25))
            .children(tabs);
        if let Some(name) = self.name {
            list = list.label(name);
        }
        let list = match self.max_width {
            Some(width) => row()
                .key(format!("{}-scroll", self.key))
                .max_w(width - pad * 2. - BORDER * 2.)
                .min_w(0.)
                .scroll_x()
                .scroll_edge_fade(FADE)
                .child(list),
            None => list,
        };
        let mut bar = row()
            .key(self.key)
            .padding(pad)
            .radius(rounding)
            .border(BORDER, theme.border.color())
            .child(list);
        bar = match self.blurred && blurring(theme) {
            true => glass(bar, theme),
            false => bar.fill(theme.secondary.color()),
        };
        if self.blurred {
            bar = shadow_sm(bar);
        }
        bar
    }
}

/// The sub-pages under a sidebar entry: indented rows hanging off a thin guide line that runs
/// down from the entry's icon.
#[derive(Default)]
pub struct Tabs {
    items: Vec<El>,
}

impl Tabs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(mut self, items: impl IntoIterator<Item = El>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let height = theme.metrics.control;
        col()
            .gap(rems(theme, 0.25))
            .ml(INDENT)
            .child(
                el().pin(Some(ICON - INDENT), Some(0.), None, Some(height / 2.))
                    .w(LINE)
                    .fill(theme.sidebar_border.color())
                    .a11y_hidden(),
            )
            .children(self.items.into_iter().map(|item| {
                row()
                    .center_y()
                    .h(height)
                    .pl(rems(theme, 0.75))
                    .child(row().grow(1.).child(item))
            }))
    }
}
