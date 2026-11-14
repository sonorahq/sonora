use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Pixels, RenderOnce, Window, div, px};
use music::Album;
use state::{Playback, Shelf};
use ui::Card;

use crate::shared::cards;

pub(crate) const CARD_MIN: Pixels = px(130.);
pub(crate) const CARD_MAX: Pixels = px(190.);
const CARD_GAP: Pixels = px(32.);

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
        let card = (spread / count).min(CARD_MAX).floor();
        let gap = match columns > 1 {
            true => ((available - card * count) / (count - 1.)).floor(),
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

#[derive(IntoElement)]
pub(crate) struct AlbumGrid {
    id: &'static str,
    layout: CardLayout,
    albums: Vec<(usize, Album)>,
    playback: Entity<Playback>,
    /// Whether local albums carry the local file badge.
    marking: bool,
}

impl AlbumGrid {
    pub(crate) fn new(
        id: &'static str,
        available: Pixels,
        albums: impl IntoIterator<Item = (usize, Album)>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            id,
            layout: CardLayout::new(available),
            albums: albums.into_iter().collect(),
            playback,
            marking: false,
        }
    }

    /// Badges the local albums, for a library page that mixes both shelves.
    pub(crate) fn marking(mut self, marking: bool) -> Self {
        self.marking = marking;
        self
    }
}

impl RenderOnce for AlbumGrid {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            layout,
            albums,
            playback,
            marking,
        } = self;
        let cards = albums.into_iter().map(|(index, album)| {
            let local = marking && Shelf::of(&album.id).local();
            album_card(id, index, &album, &playback, layout.card, cx)
                .map(|card| cards::marked_local(card, local))
                .into_any_element()
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

fn album_card(
    id: &'static str,
    index: usize,
    album: &Album,
    playback: &Entity<Playback>,
    width: Pixels,
    cx: &App,
) -> Card {
    cards::album_card((id, index), album, playback, cx)
        .tile(width)
        .flat()
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
