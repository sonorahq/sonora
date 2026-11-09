//! Grids of cards: how many fit across a width and how wide each stands, and a virtualised
//! grid that builds only the rows of cards near the view, so a library of thousands of albums
//! scrolls as cheaply as one of twenty.

use std::borrow::Cow;
use std::rc::Rc;

use music::Album;
use rune::{El, GrowthSpec, col, row};
use ui::{Card, Theme};

use crate::Ui;
use crate::shared::cards::{self, Fit, OnItem};

/// The narrowest and widest a card in a grid stands.
pub(crate) const CARD_MIN: f32 = 130.;
pub(crate) const CARD_MAX: f32 = 190.;
/// The least room between two cards side by side.
const CARD_GAP: f32 = 32.;
/// The room between two rows of cards.
pub(crate) const ROW_GAP: f32 = 24.;

/// Finds a grid's items in the window state.
type ItemsFn<T> = Rc<dyn for<'a> Fn(&'a Ui) -> &'a [T]>;

/// How a row of cards fills a width: how many fit, how wide each is and the room between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CardLayout {
    pub(crate) columns: usize,
    pub(crate) card: f32,
    pub(crate) gap: f32,
}

impl CardLayout {
    /// The layout for `available` points: as many cards as fit at their narrowest with the
    /// least gap, each as wide as the rest allows up to [`CARD_MAX`], the slack going to the gaps.
    pub(crate) fn new(available: f32) -> Self {
        let available = available.max(CARD_MIN);
        let columns = ((available + CARD_GAP) / (CARD_MIN + CARD_GAP))
            .floor()
            .max(1.) as usize;
        let count = columns as f32;
        let spread = available - CARD_GAP * (count - 1.);
        let card = (spread / count).min(CARD_MAX).floor();
        let gap = match columns > 1 {
            true => ((available - card * count) / (count - 1.)).floor(),
            false => 0.,
        };

        Self { columns, card, gap }
    }

    /// How many rows `count` cards take.
    pub(crate) fn rows(&self, count: usize) -> usize {
        count.div_ceil(self.columns.max(1))
    }
}

/// One row of cards laid out to `layout`, not virtualised: for a handful of cards, such as a
/// rail's skeletons or an artist's top releases.
pub(crate) fn card_row(layout: CardLayout, cards: impl IntoIterator<Item = El>) -> El {
    row().w_full().gap(layout.gap).children(cards)
}

/// A virtualised grid of `count` cards. Make the built element a direct child of the page's
/// vertical scroll viewport (a hero may come before it); only the rows near the view are built.
pub(crate) struct CardGrid {
    key: String,
    layout: CardLayout,
    count: usize,
    row_gap: f32,
    bottom: f32,
}

impl CardGrid {
    /// `key` names the grid, unique on the page; `available` is the width it lays out in.
    pub(crate) fn new(key: impl Into<String>, available: f32, count: usize) -> Self {
        Self {
            key: key.into(),
            layout: CardLayout::new(available),
            count,
            row_gap: ROW_GAP,
            bottom: 0.,
        }
    }

    /// How a row of cards fills `available` points.
    pub(crate) fn layout(available: f32) -> CardLayout {
        CardLayout::new(available)
    }

    /// The room under the last row, for the player bar or a perched control.
    pub(crate) fn bottom(mut self, room: f32) -> Self {
        self.bottom = room;
        self
    }

    /// The grid, keying card `i` by `key(ui, i)` (a stable id, unique in the grid) and building
    /// it with `card(ui, i, width)` as a tile `width` wide. Rows are keyed by their first card.
    pub(crate) fn build(
        self,
        theme: &Theme,
        key: impl for<'a> Fn(&'a Ui, usize) -> Cow<'a, str> + 'static,
        card: impl Fn(&Ui, usize, f32) -> El + 'static,
    ) -> El {
        let Self {
            key: grid,
            layout,
            count,
            row_gap,
            bottom,
        } = self;
        let rows = layout.rows(count);
        let height = Card::tile_height(layout.card, theme) + row_gap;
        let columns = layout.columns;
        let key = Rc::new(key);
        let named = key.clone();
        col()
            .key(grid)
            .w_full()
            .pb(bottom)
            .growth(GrowthSpec::default())
            .virtual_rows(
                rows,
                height,
                move |ui: &Ui, at: usize| Cow::Owned(format!("row-{}", named(ui, at * columns))),
                move |ui: &Ui, at: usize| {
                    let first = at * columns;
                    let cards = (first..(first + columns).min(count)).map(|index| {
                        card(ui, index, layout.card).position_in_set(index + 1, count)
                    });
                    let line = card_row(layout, cards);
                    match at + 1 < rows {
                        true => col().w_full().pb(row_gap).child(line),
                        false => line,
                    }
                },
            )
    }
}

/// A virtualised grid of album cards, the albums found by `albums` in the store, each opening
/// its page and playing from its cover. `on_item` opens a card's menu.
pub(crate) fn album_grid(
    ui: &Ui,
    key: &str,
    available: f32,
    albums: impl for<'a> Fn(&'a Ui) -> &'a [Album] + 'static,
    on_item: Option<OnItem>,
) -> El {
    let albums: ItemsFn<Album> = Rc::new(albums);
    let count = albums(ui).len();
    let keyed = albums.clone();
    let tag = key.to_owned();
    CardGrid::new(key, available, count).build(
        &ui.theme,
        move |ui: &Ui, at: usize| {
            keyed(ui)
                .get(at)
                .map_or(Cow::Borrowed(""), |album| Cow::Borrowed(album.id.as_str()))
        },
        move |ui: &Ui, at: usize, width: f32| {
            let Some(album) = albums(ui).get(at) else {
                return Card::skeleton(format!("{tag}-{at}"))
                    .tile(width)
                    .build(&ui.theme);
            };
            cards::album_card(
                ui,
                format!("{tag}-{}", album.id),
                album,
                Fit::Tile(width),
                on_item.as_ref(),
            )
            .flat()
            .build(&ui.theme)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_stays_within_its_bounds() {
        for width in (160..2400).step_by(3) {
            let layout = CardLayout::new(width as f32);
            assert!(layout.card <= CARD_MAX, "{width} yielded {:?}", layout.card);
            assert!(layout.card >= CARD_MIN, "{width} yielded {:?}", layout.card);
        }
    }

    #[test]
    fn a_row_never_outgrows_the_space_it_was_given() {
        for width in (160..2400).step_by(3) {
            let available = width as f32;
            let layout = CardLayout::new(available);
            let count = layout.columns as f32;
            let used = layout.card * count + layout.gap * (count - 1.);
            assert!(used <= available, "{width} packed {used:?}");
        }
    }

    #[test]
    fn cards_never_touch() {
        for width in (160..2400).step_by(3) {
            let layout = CardLayout::new(width as f32);
            if layout.columns > 1 {
                assert!(layout.gap >= CARD_GAP, "{width} yielded {:?}", layout.gap);
            }
        }
    }

    #[test]
    fn slack_goes_to_the_gaps_once_the_cards_are_capped() {
        let layout = CardLayout::new(CARD_MAX * 2. + CARD_GAP * 2.);

        assert_eq!(layout.card, CARD_MAX);
        assert!(layout.gap > CARD_GAP);
    }

    #[test]
    fn a_single_column_has_no_gap() {
        let layout = CardLayout::new(CARD_MIN);
        assert_eq!(layout.gap, 0.);
    }
}
