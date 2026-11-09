//! What Sonora adds to Rune's own scrolling, which already brings momentum, scroll bars, glides
//! and virtual rows: a sideways shelf of cards with paging arrows, a perched button that glides a
//! page back to its top, and the scroll position each page is left at.
//!
//! Each follows its viewport through `El::on_scroll_offset` into live values, so a scroll moves
//! the arrows' and the button's look without running the view.

use std::collections::{HashMap, VecDeque};

use i18n::t;
use rune::{El, Live, ScrollPos, Spring, Tx, el, row};

use crate::button::Button;
use crate::glass::{blurring, glass};
use crate::motion::Springs;
use crate::theme::Theme;

/// How far a page has to be scrolled before the trip back is worth a button, in list rows.
const REACH: f32 = 3.;
/// How far a perched control floats off the bottom of its region.
const PERCH: f32 = 12.;
/// How close to an end a shelf counts as at it, so a fraction of a point left over by a glide
/// still dims its arrow.
const STEADY: f32 = 1.;
/// How a dimmed paging arrow shows at a shelf's end.
const DIMMED: f32 = 0.4;
/// The gap between a shelf's cards when the caller names none.
const RAIL_GAP: f32 = 16.;
/// How many pages' scroll positions are remembered.
const PAGES: usize = 64;
/// How far below its perch a hidden return button sits: past the region's bottom edge, where it
/// takes no clicks meant for the rows.
const TUCKED: f32 = 64.;
/// The glide a paging arrow and the return button scroll on.
const GLIDE: Spring = Springs::SLOW;

/// Where a scroll viewport is, kept from its `on_scroll_offset` reports so handlers can read it
/// and live values can follow it. Clone it into the state that owns the viewport; every clone
/// shares one position.
#[derive(Clone)]
pub struct ScrollWatch {
    /// The offset, the furthest it can scroll and the viewport's size, along the watched axis.
    at: Live<[f32; 4]>,
    /// 1 while the viewport is scrolled away from its top by more than `reach`, else 0.
    away: Live<f32>,
    /// Where the return button sits: up on its perch while `away`, else tucked below it.
    lift: Live<[f32; 2]>,
    /// 0.4 at the start of a sideways viewport and 1 elsewhere; `end` likewise at its end.
    start: Live<f32>,
    end: Live<f32>,
}

impl Default for ScrollWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollWatch {
    pub fn new() -> Self {
        Self {
            at: Live::new([0.; 4]),
            away: Live::new(0.),
            lift: Live::new([0., TUCKED]),
            start: Live::new(DIMMED),
            end: Live::new(1.),
        }
    }

    /// Follows the vertical scroll viewport `e`. `reach` is how far it has to scroll before the
    /// return button shows.
    pub fn vertical(&self, e: El, reach: f32) -> El {
        let watch = self.clone();
        e.on_scroll_offset(move |at: ScrollPos| {
            watch.at.set([at.offset[1], at.max[1], at.viewport[1], 0.]);
            let away = at.offset[1] > reach;
            watch.away.set(match away {
                true => 1.,
                false => 0.,
            });
            watch.lift.set(match away {
                true => [0., 0.],
                false => [0., TUCKED],
            });
        })
    }

    /// Follows the sideways scroll viewport `e`, dimming the paging arrows at its ends.
    pub fn horizontal(&self, e: El) -> El {
        let watch = self.clone();
        e.on_scroll_offset(move |at: ScrollPos| {
            let [offset, max, viewport] = [at.offset[0], at.max[0], at.viewport[0]];
            watch.at.set([offset, max, viewport, 0.]);
            watch.start.set(match offset <= STEADY {
                true => DIMMED,
                false => 1.,
            });
            watch.end.set(match offset >= max - STEADY {
                true => DIMMED,
                false => 1.,
            });
        })
    }

    /// How far the viewport is scrolled, in points.
    pub fn offset(&self) -> f32 {
        self.at.get()[0]
    }

    /// How far the viewport can scroll at most.
    pub fn max(&self) -> f32 {
        self.at.get()[1]
    }

    /// The viewport's length along the watched axis.
    pub fn viewport(&self) -> f32 {
        self.at.get()[2]
    }

    /// Glides the sideways viewport keyed `key` one viewport on or back, clamped to its ends.
    pub fn page(&self, key: &str, forward: bool, tx: &mut Tx) {
        let step = match forward {
            true => self.viewport(),
            false => -self.viewport(),
        };
        let to = (self.offset() + step).clamp(0., self.max().max(0.));
        tx.scroll_to_with(key.to_owned(), to, 0., GLIDE);
    }

    /// Glides the vertical viewport keyed `key` back to its top.
    pub fn rewind(&self, key: &str, tx: &mut Tx) {
        tx.scroll_to_with(key.to_owned(), 0., 0., GLIDE);
    }
}

/// The scroll position each page was left at, so going back lands where the listener was. Pages
/// are named by the caller (a route, say). The oldest are forgotten past a bound.
#[derive(Clone, Debug, Default)]
pub struct ScrollMemory {
    offsets: HashMap<String, f32>,
    order: VecDeque<String>,
}

impl ScrollMemory {
    /// Keeps `offset` for `page`, as the page is left.
    pub fn remember(&mut self, page: impl Into<String>, offset: f32) {
        let page = page.into();
        if !offset.is_finite() {
            return;
        }
        self.order.retain(|kept| *kept != page);
        self.order.push_back(page.clone());
        self.offsets.insert(page, offset.max(0.));
        while self.order.len() > PAGES {
            if let Some(oldest) = self.order.pop_front() {
                self.offsets.remove(&oldest);
            }
        }
    }

    /// Where `page` was left, if it was.
    pub fn recall(&self, page: &str) -> Option<f32> {
        self.offsets.get(page).copied()
    }

    /// Puts the vertical viewport keyed `key` back where `page` was left, without a glide, as a
    /// page comes back.
    pub fn restore(&self, page: &str, key: &str, tx: &mut Tx) {
        if let Some(offset) = self.recall(page) {
            tx.scroll_to_instant(key.to_owned(), 0., offset);
        }
    }

    pub fn forget(&mut self, page: &str) {
        self.offsets.remove(page);
        self.order.retain(|kept| kept != page);
    }
}

/// A sideways rail of cards under a heading, with arrows that glide it a viewport at a time. The
/// arrows dim at the rail's ends as it scrolls, without the view running again.
pub struct Shelf {
    key: String,
    watch: ScrollWatch,
    heading: Option<El>,
    items: Vec<El>,
    gap: f32,
    inset: Option<f32>,
    height: Option<f32>,
    arrows: bool,
}

impl Shelf {
    /// `key` names the rail's scroll viewport, which the arrows glide; `watch` lives in the
    /// state that owns the shelf.
    pub fn new(key: impl Into<String>, watch: &ScrollWatch) -> Self {
        Self {
            key: key.into(),
            watch: watch.clone(),
            heading: None,
            items: Vec::new(),
            gap: RAIL_GAP,
            inset: None,
            height: None,
            arrows: true,
        }
    }

    pub fn heading(mut self, heading: El) -> Self {
        self.heading = Some(heading);
        self
    }

    /// The cards, each keyed by the caller, in order.
    pub fn items(mut self, items: impl IntoIterator<Item = El>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// The room before the first card and after the last, matching the page's side inset.
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = Some(inset);
        self
    }

    /// The rail's height, so a shelf of tiles keeps its room while its cards load.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Whether the paging arrows show: leave them off when every card fits.
    pub fn arrows(mut self, arrows: bool) -> Self {
        self.arrows = arrows;
        self
    }

    pub fn build<S: 'static>(self, theme: &Theme) -> El {
        let Self {
            key,
            watch,
            heading,
            items,
            gap,
            inset,
            height,
            arrows,
        } = self;
        let inset = inset.unwrap_or(theme.metrics.inset);
        let count = items.len();
        let cards = items
            .into_iter()
            .enumerate()
            .map(|(index, item)| item.position_in_set(index + 1, count));
        let content = row().gap(gap).px(inset).children(cards);
        let mut rail = watch.horizontal(
            row()
                .key(key.clone())
                .role("list")
                .w_full()
                .scroll_x()
                .child(content),
        );
        if let Some(height) = height {
            rail = rail.h(height);
        }

        let mut head = row()
            .w_full()
            .px(inset)
            .gap(theme.metrics.pad * 2.)
            .items_end()
            .justify_between();
        if let Some(heading) = heading {
            head = head.child(heading);
        }
        if arrows {
            head = head.child(
                row()
                    .gap(theme.metrics.pad / 2.)
                    .child(arrow::<S>(&key, &watch, false, theme))
                    .child(arrow::<S>(&key, &watch, true, theme)),
            );
        }
        rune::col()
            .w_full()
            .gap(theme.metrics.pad * 1.5)
            .child(head)
            .child(rail)
    }
}

/// One paging arrow of a shelf, dimmed while the rail sits at the end it points to.
fn arrow<S: 'static>(key: &str, watch: &ScrollWatch, forward: bool, theme: &Theme) -> El {
    let (side, glyph, tip, dim) = match forward {
        true => (
            "next",
            "icons/chevron-right.svg",
            t!("common-next"),
            &watch.end,
        ),
        false => (
            "previous",
            "icons/chevron-left.svg",
            t!("common-previous"),
            &watch.start,
        ),
    };
    let rail = key.to_owned();
    let paging = watch.clone();
    let button = Button::new(format!("{key}-{side}"))
        .small()
        .outline()
        .icon(glyph)
        .tooltip(tip.to_string())
        .on_press_tx(move |_: &mut S, tx: &mut Tx| paging.page(&rail, forward, tx))
        .build(theme);
    button.live_opacity(dim.smooth(Springs::QUICK))
}

/// The shape every control that floats over a scrolling region takes: centred along the bottom,
/// `PERCH` points up. Put it in a parent that also holds the region.
pub fn perched(control: El) -> El {
    row()
        .pin(Some(0.), None, Some(0.), Some(PERCH))
        .justify_center()
        .child(control)
}

/// The room a scrolling region keeps under its last row for a perched control to float in without
/// covering it.
pub fn perch_room(theme: &Theme) -> f32 {
    PERCH * 2. + theme.metrics.control_small
}

/// A perched button that glides the vertical viewport keyed `key` back to its top. It slides in
/// once `watch` (following that viewport with [`ScrollWatch::vertical`] and a reach from
/// [`return_reach`]) reports the page scrolled far enough, and out again near the top, without
/// the view running. Put it in a parent that also holds the viewport.
pub fn return_top<S: 'static>(key: &str, watch: &ScrollWatch, theme: &Theme) -> El {
    let rail = key.to_owned();
    let rewind = watch.clone();
    let mut button = Button::new(format!("{key}-return-top"))
        .secondary()
        .small()
        .icon("icons/undo-2.svg")
        .tooltip(t!("nav-return-top").to_string())
        .on_press_tx(move |_: &mut S, tx: &mut Tx| rewind.rewind(&rail, tx))
        .build(theme);
    if blurring(theme) {
        button = glass(button, theme);
    }
    let shown = el()
        .live_opacity(watch.away.smooth(Springs::BASE))
        .live_offset(watch.lift.smooth(Springs::BASE))
        .pixel_snap(false)
        .child(button);
    perched(shown)
}

/// How far a page scrolls before its return button shows: three list rows, or half the viewport
/// when that is less.
pub fn return_reach(theme: &Theme, viewport: f32) -> f32 {
    let reach = theme.metrics.list_row * REACH;
    match viewport > 0. {
        true => reach.min(viewport / 2.),
        false => reach,
    }
}
