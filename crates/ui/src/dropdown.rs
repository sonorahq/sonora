//! A panel that hangs under a field or control while it is open, in a menu's look, such as the
//! suggestions under a search field.

use std::rc::Rc;

use rune::{El, col, row};

use crate::glass::{GLASS_BLUR, blurring, popover_fill};
use crate::overlay::{self, Placement};
use crate::paint::shadow_md;
use crate::theme::Theme;

/// How far below its anchor the panel hangs.
const GAP: f32 = 6.;
const BORDER: f32 = 1.;
const ROW_GAP: f32 = 1.;

type Close<S> = Rc<dyn Fn(&mut S)>;

/// A dropdown. The panel takes no focus, so a field in the anchor keeps the keyboard, and the
/// anchor stays above the press target that closes the panel, so presses on it still reach it.
/// Whether the panel shows is the caller's state.
pub struct Dropdown<S> {
    key: String,
    anchor: El,
    open: bool,
    width: Option<f32>,
    children: Vec<El>,
    close: Option<Close<S>>,
}

impl<S: 'static> Dropdown<S> {
    pub fn new(key: impl Into<String>, anchor: El) -> Self {
        Self {
            key: key.into(),
            anchor,
            open: false,
            width: None,
            children: Vec::new(),
            close: None,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// The panel's width, usually the anchor's. Without one the panel is as wide as its rows.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn child(mut self, child: El) -> Self {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = El>) -> Self {
        self.children.extend(children);
        self
    }

    /// Runs on a press anywhere outside both the anchor and the panel.
    pub fn on_close(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.close = Some(Rc::new(f));
        self
    }

    /// The anchor, with the open panel pushed into `layer`.
    pub fn build(self, theme: &Theme, layer: &mut Vec<El>) -> El {
        let anchor_key = format!("{}-anchor", self.key);
        let anchor = row().key(anchor_key.clone()).child(self.anchor);
        if !self.open {
            return anchor;
        }
        if let Some(close) = self.close {
            layer.push(overlay::scrim(
                format!("{}-scrim", self.key),
                move |s: &mut S| close(s),
            ));
        }
        let mut panel = col()
            .key(format!("{}-panel", self.key))
            .padding(crate::paint::rems(theme, 0.25))
            .gap(ROW_GAP)
            .min_h(0.)
            .radius(theme.radius)
            .border(BORDER, theme.border.color())
            .fill(popover_fill(theme).color())
            .children(self.children);
        panel = shadow_md(panel);
        if blurring(theme) {
            panel = panel.backdrop_blur(GLASS_BLUR);
        }
        if let Some(width) = self.width {
            panel = panel.w(width);
        }
        layer.push(overlay::anchor(
            panel,
            &anchor_key,
            Placement::BottomStart,
            GAP,
        ));
        // over the scrim, so a press on the field keeps the panel and moves the caret
        anchor.raise()
    }
}
