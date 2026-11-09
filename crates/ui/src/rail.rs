//! A row of items that scrolls sideways, with a touchpad, Shift and the wheel, or the round
//! arrows over either end, its items fading out towards each edge with more behind it.

use std::cell::Cell;
use std::rc::Rc;

use rune::{Cursor, El, Live, ScrollPos, Tx, icon, row};

use crate::motion::{PRESS_SCALE, Springs};
use crate::paint::{rems, shadow_md};
use crate::theme::Theme;

/// How far the items fade out towards a side of the rail, and how far the rail reaches past its
/// column on either side to make room for that fade.
const FADE: f32 = 48.;
/// The arrows' diameter and their chevrons' size.
const ARROW: f32 = 28.;
const CHEVRON: f32 = 14.;
/// How much of the rail's width one arrow press moves it.
const STEP: f32 = 0.75;
/// How close to an end the rail has to be to count as resting there.
const EDGE: f32 = 0.5;
const BORDER: f32 = 1.;

/// A rail. It reaches past its column by the width of its fade on both sides and pads its items
/// back in by as much, so at rest the fades lie over empty room. An arrow shows over an end
/// with items behind it and fades away when the rail reaches that end.
pub struct Rail {
    key: String,
    children: Vec<El>,
    arrows_at: Option<f32>,
}

impl Rail {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            children: Vec::new(),
            arrows_at: None,
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

    /// Centres the arrows `y` below the rail's top edge, such as on the middle of a card's
    /// picture rather than of the card with its name.
    pub fn arrows_at(mut self, y: f32) -> Self {
        self.arrows_at = Some(y);
        self
    }

    pub fn build<S: 'static>(self, theme: &Theme) -> El {
        let scroll_key = format!("{}-scroll", self.key);
        // Where the row is, as the last frame scrolled it: the arrows read it when pressed and
        // fade by it without the view running again.
        // (offset, furthest offset, viewport width) along the row
        let reach: Rc<Cell<(f32, f32, f32)>> = Rc::new(Cell::new((0., 0., 0.)));
        let shown = [Live::new(0.), Live::new(0.)];
        let watched = reach.clone();
        let fades = shown.clone();
        let items = row()
            .key(scroll_key.clone())
            .mx(-FADE)
            // a flex item is as wide as its content unless told it may shrink, which would
            // leave nothing to scroll
            .grow(1.)
            .min_w(0.)
            .scroll_x()
            .scroll_edge_fade(FADE)
            .on_scroll_offset(move |at: ScrollPos| {
                let x = at.offset[0];
                watched.set((x, at.max[0], at.viewport[0]));
                fades[0].set(f32::from(u8::from(x > EDGE)));
                fades[1].set(f32::from(u8::from(at.max[0] - x > EDGE)));
            })
            .child(
                row()
                    .gap(rems(theme, 0.75))
                    .px(FADE)
                    .children(self.children),
            );
        let rail = self.key.clone();
        let arrows_at = self.arrows_at;
        let arrows = [false, true].into_iter().map(|forward| {
            let reach = reach.clone();
            let key = scroll_key.clone();
            let (side, glyph, name) = match forward {
                true => (
                    "forward",
                    "icons/chevron-right.svg",
                    i18n::t!("rail-forward"),
                ),
                false => ("back", "icons/chevron-left.svg", i18n::t!("rail-back")),
            };
            let top = arrows_at.map(|y| y - ARROW / 2.);
            let inset = -ARROW / 2.;
            let placed = match forward {
                true => row().pin(None, top, Some(inset), None),
                false => row().pin(Some(inset), top, None, None),
            };
            let placed = match top {
                Some(_) => placed,
                None => placed.self_center(),
            };
            let name = name.into_owned();
            let arrow = placed
                .key(format!("{rail}-{side}"))
                .role("button")
                .label(name.clone())
                .help(name)
                .center()
                .size(ARROW, ARROW)
                .radius(ARROW / 2.)
                .fill(theme.secondary.color())
                .border(BORDER, theme.border.color())
                .hover(theme.secondary_active.color())
                .cursor(Cursor::Pointer)
                .press_scale(PRESS_SCALE)
                .live_opacity(shown[usize::from(forward)].smooth(Springs::BASE))
                .child(
                    icon(icons::get(glyph), CHEVRON)
                        .color(theme.foreground.color())
                        .a11y_hidden(),
                )
                .on_press_tx(move |_: &mut S, tx: &mut Tx| {
                    let (at, furthest, viewport) = reach.get();
                    let step = viewport * STEP;
                    let x = match forward {
                        true => at + step,
                        false => at - step,
                    };
                    tx.scroll_to(key.clone(), x.clamp(0., furthest), 0.);
                });
            shadow_md(arrow)
        });
        row()
            .key(self.key)
            .center_y()
            .min_w(0.)
            .child(items)
            .children(arrows)
    }
}
