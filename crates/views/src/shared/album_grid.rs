use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Pixels, Render, RenderOnce, SharedString, StyleRefinement,
    Window, div, px,
};
use music::Album;
use state::Playback;

use crate::shared::cards;

pub(crate) const CARD_MIN: Pixels = px(130.);
pub(crate) const CARD_MAX: Pixels = px(190.);
/// The least room between two cards in a row. The gap grows past it to spread the room a row
/// of cards leaves.
const CARD_GAP: Pixels = px(16.);
/// How finely a card and its gap are cut, as a fraction of a pixel. Neither rounds to a whole
/// pixel, since a card that grows a pixel at a time moves the columns back and forth while the
/// width changes every frame, as it does while a sidebar slides.
const STEP: f32 = 64.;

#[derive(Clone, Copy)]
pub(crate) struct CardLayout {
    pub(crate) columns: usize,
    pub(crate) card: Pixels,
    pub(crate) gap: Pixels,
}

impl CardLayout {
    fn new(available: Pixels) -> Self {
        let available = available.max(CARD_MIN);
        let columns = (((available + CARD_GAP) / (CARD_MIN + CARD_GAP))
            .floor()
            .max(1.)) as usize;
        let count = columns as f32;
        let spread = available - CARD_GAP * (count - 1.);
        let card = fine((spread / count).min(CARD_MAX));
        let gap = match columns > 1 {
            true => fine((available - card * count) / (count - 1.)),
            false => Pixels::ZERO,
        };

        Self { columns, card, gap }
    }
}

#[derive(IntoElement)]
pub(crate) struct CardGrid {
    layout: CardLayout,
    cards: Vec<AnyElement>,
}

impl CardGrid {
    pub(crate) fn new(available: Pixels) -> Self {
        Self {
            layout: CardLayout::new(available),
            cards: Vec::new(),
        }
    }

    pub(crate) fn layout(available: Pixels) -> CardLayout {
        CardLayout::new(available)
    }

    pub(crate) fn children(mut self, cards: impl IntoIterator<Item = AnyElement>) -> Self {
        self.cards.extend(cards);
        self
    }
}

impl RenderOnce for CardGrid {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .gap_x(self.layout.gap)
            .children(self.cards)
    }
}

/// What an album tile shows. The grid that owns the tile sets all of it, so the tile reads
/// no entity while it renders.
#[derive(Clone, PartialEq)]
pub(crate) struct TileLook {
    pub(crate) album: Album,
    pub(crate) playing: bool,
    /// Whether the card carries the local file badge.
    pub(crate) local: bool,
    pub(crate) width: Pixels,
    /// Where the card sits in its list, from 0, and how many the list holds, for screen readers.
    pub(crate) position: usize,
    pub(crate) count: usize,
}

/// An album card drawn as a view of its own through `cached`, so its hover fade, its press and
/// its cover landing redraw it alone while the cards around it replay. It renders from the
/// look its grid gave it and reads no other entity, `Playback` least of all.
pub(crate) struct AlbumTile {
    /// What the card's element id starts with, the album id following it.
    prefix: &'static str,
    look: TileLook,
    /// Handed to the card's play, warm and menu handlers, and never read while rendering.
    playback: Entity<Playback>,
}

impl AlbumTile {
    /// Shows `look` from the next render, answering whether it differs from what the tile
    /// showed. The grid sets it while it draws, when a notify would only land a frame late, so
    /// the grid draws a changed tile without its cache instead.
    fn show(&mut self, look: TileLook) -> bool {
        if self.look == look {
            return false;
        }
        self.look = look;
        true
    }
}

impl Render for AlbumTile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = &self.look;
        let id = SharedString::from(format!("{}-{}", self.prefix, look.album.id));

        cards::told_album_card(id, &look.album, look.playing, &self.playback, cx)
            .aria_position_in_set(look.position + 1)
            .aria_size_of_set(look.count)
            .tile(look.width)
            .flat()
            .map(|card| cards::marked_local(card, look.local))
    }
}

/// The album tiles of one grid by album id, kept for as long as the grid draws them. A tile
/// the grid left out since the last `sweep` is dropped by the next one.
pub(crate) struct AlbumTiles {
    prefix: &'static str,
    tiles: HashMap<String, Kept>,
    /// Counts the sweeps, so a tile knows whether the grid drew it since the last one.
    turn: u64,
}

/// A tile and the sweep turn the grid last drew it on.
struct Kept {
    tile: Entity<AlbumTile>,
    turn: u64,
}

impl AlbumTiles {
    pub(crate) fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            tiles: HashMap::new(),
            turn: 0,
        }
    }

    /// Drops the tiles the grid did not draw since the last sweep, which are the ones that
    /// left the rows on screen and the row prefetched past them. The owner sweeps each time it
    /// renders, before its grid draws again.
    pub(crate) fn sweep(&mut self) {
        let turn = self.turn;
        self.tiles.retain(|_, kept| kept.turn == turn);
        self.turn += 1;
    }

    /// A row of cards showing `looks` in a grid across `available`, each one `height` tall,
    /// from the tiles kept for those albums or new ones for the albums without.
    pub(crate) fn row(
        &mut self,
        available: Pixels,
        height: Pixels,
        looks: Vec<TileLook>,
        playback: &Entity<Playback>,
        cx: &mut App,
    ) -> AlbumGrid {
        let prefix = self.prefix;
        let turn = self.turn;
        let tiles = looks
            .into_iter()
            .map(|look| {
                let made = |look: TileLook, cx: &mut App| {
                    cx.new(|_| AlbumTile {
                        prefix,
                        look,
                        playback: playback.clone(),
                    })
                };
                match self.tiles.get_mut(&look.album.id) {
                    // An album listed twice gets a tile of its own each time, since one view
                    // cannot be drawn in two places.
                    Some(kept) if kept.turn == turn => (made(look, cx), false),
                    Some(kept) => {
                        kept.turn = turn;
                        let changed = kept.tile.update(cx, |tile, _| tile.show(look));
                        (kept.tile.clone(), changed)
                    }
                    None => {
                        let id = look.album.id.clone();
                        let tile = made(look, cx);
                        self.tiles.insert(
                            id,
                            Kept {
                                tile: tile.clone(),
                                turn,
                            },
                        );
                        (tile, false)
                    }
                }
            })
            .collect();

        AlbumGrid {
            layout: CardLayout::new(available),
            height,
            tiles,
        }
    }
}

/// One row of album tiles, each drawn through its view's cache unless it just changed.
#[derive(IntoElement)]
pub(crate) struct AlbumGrid {
    layout: CardLayout,
    height: Pixels,
    /// The tiles in order, each with whether its look changed this frame.
    tiles: Vec<(Entity<AlbumTile>, bool)>,
}

impl RenderOnce for AlbumGrid {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            layout,
            height,
            tiles,
        } = self;
        // A cached view is laid out without rendering, which would load no covers for a row
        // that is only prefetched, and would replay a changed tile's last frame.
        let plain = ui::prefetching();
        let cards = tiles
            .into_iter()
            .map(|(tile, changed)| match plain || changed {
                true => div()
                    .flex_none()
                    .w(layout.card)
                    .h(height)
                    .child(tile)
                    .into_any_element(),
                false => tile
                    .cached(
                        StyleRefinement::default()
                            .flex_none()
                            .w(layout.card)
                            .h(height),
                    )
                    .into_any_element(),
            });

        div()
            .flex()
            .flex_wrap()
            .w_full()
            .gap_x(layout.gap)
            .gap_y_6()
            .children(cards)
    }
}

/// Cuts `length` down to the next `STEP`th of a pixel, so the cards and gaps of a row never add
/// up to more than the row.
fn fine(length: Pixels) -> Pixels {
    px((length / px(1.) * STEP).floor() / STEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_stays_within_its_bounds() {
        for width in (160..2400).step_by(3) {
            let layout = CardLayout::new(px(width as f32));
            assert!(layout.card <= CARD_MAX, "{width} yielded {:?}", layout.card);
            assert!(layout.card >= CARD_MIN, "{width} yielded {:?}", layout.card);
        }
    }

    #[test]
    fn a_row_never_outgrows_the_space_it_was_given() {
        for width in (160..2400).step_by(3) {
            let available = px(width as f32);
            let layout = CardLayout::new(available);
            let count = layout.columns as f32;
            let used = layout.card * count + layout.gap * (count - 1.);
            assert!(used <= available, "{width} packed {used:?}");
        }
    }

    #[test]
    fn cards_never_touch() {
        for width in (160..2400).step_by(3) {
            let layout = CardLayout::new(px(width as f32));
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
        assert_eq!(layout.gap, Pixels::ZERO);
    }
}
