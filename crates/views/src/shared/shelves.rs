//! Shelves: a page of sections (home, a genre), each a heading over a sideways rail of cards
//! with paging arrows, or over a few columns of list rows; the single rails artist and album
//! pages draw their recommendations with; and the grid of genre plates.
//!
//! Rails are drawn with [`ui::Shelf`], whose arrows follow the rail's scroll without the view
//! running. Each rail's scroll is kept in a [`Rails`] in the screen's state.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use music::{Genre, GenreItem, GenreSection};
use rune::{El, GrowthSpec, col, row};
use ui::{Card, Mode, ScrollWatch, Shelf, Skeleton, Text, Theme, spoken_heading};

use crate::Ui;
use crate::shared::album_grid::CardLayout;
use crate::shared::cards::{self, Fit, OnItem};
use crate::shared::page::rems;

/// The narrowest a genre plate or a lane of list rows stands, and the most lanes across.
const PLATE: f32 = 260.;
const LANES: usize = 5;
/// How many rows a section in list mode shows per lane.
const ROWS: usize = 3;
/// How many skeleton sections stand in while the shelves load.
const PENDING: usize = 3;
/// The space between a rail's cards, between sections, between lane rows and under a heading.
const RAIL_GAP: f32 = 16.;
const STACK_GAP: f32 = 32.;
const LANE_GAP: f32 = 8.;
/// The space under a heading and between lanes side by side, in rems.
const HEADING_GAP: f32 = 0.75;
const LANE_SPACE: f32 = 0.5;
/// The line height a section heading stands at.
const LEADING: f32 = 1.4;
/// How wide a skeleton heading stands.
const HEADING: f32 = 140.;

/// The scroll of every rail on a page, by the rail's key, kept in the screen's state so the
/// paging arrows find where a rail stands. Rails are made the first time a view asks.
#[derive(Default)]
pub(crate) struct Rails {
    watches: RefCell<HashMap<String, ScrollWatch>>,
}

impl Rails {
    /// The scroll of the rail keyed `key`.
    pub(crate) fn watch(&self, key: &str) -> ScrollWatch {
        self.watches
            .borrow_mut()
            .entry(key.to_owned())
            .or_default()
            .clone()
    }

    /// Forgets every rail, for a page that starts showing other sections.
    pub(crate) fn reset(&mut self) {
        self.watches.get_mut().clear();
    }
}

/// A page's sections, one under the next. The page lays out at `width` with its own inset on
/// both sides, which the rails reach back over so cards slide out to the page's edges.
pub(crate) struct Shelves<'a> {
    key: &'a str,
    sections: &'a [GenreSection],
    rails: &'a Rails,
    mode: Mode,
    width: f32,
    on_item: Option<OnItem>,
}

impl<'a> Shelves<'a> {
    /// `width` is the content's width inside the page's inset.
    pub(crate) fn new(
        key: &'a str,
        sections: &'a [GenreSection],
        rails: &'a Rails,
        width: f32,
    ) -> Self {
        Self {
            key,
            sections,
            rails,
            mode: Mode::Grid,
            width,
            on_item: None,
        }
    }

    /// Rails of tiles (the default) or lanes of list rows.
    pub(crate) fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// A right-click on a card.
    pub(crate) fn on_item(mut self, on_item: OnItem) -> Self {
        self.on_item = Some(on_item);
        self
    }

    pub(crate) fn build(self, ui: &Ui) -> El {
        let theme = &ui.theme;
        let sections = self
            .sections
            .iter()
            .enumerate()
            .map(|(place, section)| match self.mode {
                Mode::Grid => self.rail(ui, place, section),
                Mode::List => self.lane(ui, place, section),
            });
        col()
            .key(self.key.to_owned())
            .gap(STACK_GAP)
            .children(sections)
            .mx(-theme.metrics.inset)
            .w(self.width + theme.metrics.inset * 2.)
    }

    /// Loading sections: skeleton headings over skeleton tiles, through the page's edges.
    pub(crate) fn pending(key: &str, width: f32, theme: &Theme) -> El {
        let layout = CardLayout::new(width);
        let sections = (0..PENDING).map(|shelf| {
            col()
                .key(format!("{key}-pending-{shelf}"))
                .gap(rems(theme, HEADING_GAP))
                .child(
                    row().px(theme.metrics.inset).child(
                        Skeleton::new()
                            .build(theme)
                            .size(HEADING, theme.text(Text::Large))
                            .radius(theme.radius),
                    ),
                )
                .child(
                    row()
                        .px(theme.metrics.inset)
                        .gap(RAIL_GAP)
                        .clip_rounded(0.)
                        .children((0..=layout.columns).map(|place| {
                            Card::skeleton(format!("{key}-pending-{shelf}-{place}"))
                                .tile(layout.card)
                                .build(theme)
                        })),
                )
        });
        col()
            .key(format!("{key}-pending"))
            .w(width + theme.metrics.inset * 2.)
            .mx(-theme.metrics.inset)
            .gap(STACK_GAP)
            .children(sections)
            .a11y_hidden()
    }

    /// A section as a sideways rail of tiles under its heading.
    fn rail(&self, ui: &Ui, place: usize, section: &GenreSection) -> El {
        let theme = &ui.theme;
        let layout = CardLayout::new(self.width);
        let key = format!("{}-rail-{place}", self.key);
        let watch = self.rails.watch(&key);
        let cards = section.items.iter().enumerate().map(|(index, item)| {
            cards::item_card(
                ui,
                format!("{}-item-{place}-{index}", self.key),
                item,
                Fit::Tile(layout.card),
                self.on_item.as_ref(),
            )
            .flat()
            .build(theme)
        });
        let title = i18n::translate(&section.title).into_owned();
        Shelf::new(key.clone(), &watch)
            .heading(heading(format!("{key}-heading"), title, theme))
            .items(cards)
            .gap(RAIL_GAP)
            .height(Card::tile_height(layout.card, theme))
            .arrows(section.items.len() > layout.columns)
            .build::<Ui>(theme)
    }

    /// A section as lanes of list rows, without sideways scrolling.
    fn lane(&self, ui: &Ui, place: usize, section: &GenreSection) -> El {
        let theme = &ui.theme;
        let lanes = lanes(self.width);
        let total = section.items.len().min(lanes * ROWS);
        let mut columns: Vec<Vec<El>> = (0..lanes).map(|_| Vec::new()).collect();
        for (index, item) in section.items.iter().take(total).enumerate() {
            let card = cards::item_card(
                ui,
                format!("{}-item-{place}-{index}", self.key),
                item,
                Fit::Row,
                self.on_item.as_ref(),
            )
            .build(theme)
            .fill(theme.secondary.color())
            .position_in_set(index + 1, total);
            columns[index % lanes].push(card);
        }
        let title = i18n::translate(&section.title).into_owned();
        col()
            .key(format!("{}-lane-{place}", self.key))
            .px(theme.metrics.inset)
            .gap(rems(theme, HEADING_GAP))
            .child(row().items_end().h(head(theme)).child(heading(
                format!("{}-heading-{place}", self.key),
                title,
                theme,
            )))
            .child(spread(columns, theme))
    }
}

/// One rail outside a shelf page: a heading with paging arrows over a sideways row of cards,
/// with an optional row of tabs between them. Artist and album pages draw their
/// recommendations with it.
pub(crate) struct Rail<'a> {
    key: String,
    title: String,
    rails: &'a Rails,
    tile: f32,
    columns: usize,
    tabs: Option<El>,
    cards: Vec<El>,
}

impl<'a> Rail<'a> {
    /// A rail keyed `key` (unique on the page) under `title`, its cards `tile` wide and
    /// `columns` of them fitting across, which decides whether the arrows show.
    pub(crate) fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        rails: &'a Rails,
        layout: CardLayout,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            rails,
            tile: layout.card,
            columns: layout.columns,
            tabs: None,
            cards: Vec::new(),
        }
    }

    /// The tab row drawn between the heading and the cards.
    pub(crate) fn tabs(mut self, tabs: El) -> Self {
        self.tabs = Some(tabs);
        self
    }

    /// The cards, each a tile as wide as the rail's layout says, keyed by the caller.
    pub(crate) fn cards(mut self, cards: impl IntoIterator<Item = El>) -> Self {
        self.cards = cards.into_iter().collect();
        self
    }

    /// The rail, reaching back over the page's inset so its cards slide out to the page's edges.
    pub(crate) fn build(self, theme: &Theme) -> El {
        let crowded = self.cards.len() > self.columns;
        let watch = self.rails.watch(&self.key);
        let shelf = Shelf::new(self.key.clone(), &watch)
            .heading(heading(format!("{}-heading", self.key), self.title, theme))
            .items(self.cards)
            .gap(RAIL_GAP)
            .height(Card::tile_height(self.tile, theme))
            .arrows(crowded)
            .build::<Ui>(theme);
        let shelf = match self.tabs {
            // The tabs go under the heading, which the shelf draws first.
            Some(tabs) => col()
                .w_full()
                .gap(rems(theme, HEADING_GAP))
                .child(row().px(theme.metrics.inset).child(tabs))
                .child(shelf),
            None => shelf,
        };
        col()
            .key(format!("{}-frame", self.key))
            .mx(-theme.metrics.inset)
            .child(shelf)
    }

    /// The rail while its cards are on their way: the heading over skeleton tiles.
    pub(crate) fn pending(
        key: &str,
        title: impl Into<String>,
        layout: CardLayout,
        theme: &Theme,
    ) -> El {
        let tiles = (0..=layout.columns).map(|place| {
            Card::skeleton(format!("{key}-pending-{place}"))
                .tile(layout.card)
                .build(theme)
        });
        col()
            .key(format!("{key}-pending"))
            .mx(-theme.metrics.inset)
            .gap(rems(theme, HEADING_GAP))
            .child(
                row()
                    .px(theme.metrics.inset)
                    .items_end()
                    .h(head(theme))
                    .child(heading(format!("{key}-heading"), title.into(), theme)),
            )
            .child(
                row()
                    .w_full()
                    .px(theme.metrics.inset)
                    .h(Card::tile_height(layout.card, theme))
                    .gap(RAIL_GAP)
                    .clip_rounded(0.)
                    .children(tiles),
            )
    }
}

/// A genre's plate, a list-row card on the secondary fill, opening the genre.
pub(crate) fn plate(ui: &Ui, key: impl Into<String>, genre: &Genre) -> El {
    cards::genre_card(ui, key, genre, Fit::Row)
        .build(&ui.theme)
        .fill(ui.theme.secondary.color())
}

/// A virtualised grid of genre plates, the genres found by `genres` in the store. Make it a
/// direct child of the page's scroll viewport.
pub(crate) fn grid(
    ui: &Ui,
    key: &str,
    width: f32,
    genres: impl for<'a> Fn(&'a Ui) -> &'a [Genre] + 'static,
) -> El {
    let theme = &ui.theme;
    let genres: Rc<dyn for<'a> Fn(&'a Ui) -> &'a [Genre]> = Rc::new(genres);
    let lanes = lanes(width);
    let count = genres(ui).len();
    let rows = count.div_ceil(lanes);
    let height = theme.metrics.list_row.round() + LANE_GAP;
    let keyed = genres.clone();
    let tag = key.to_owned();
    col()
        .key(key.to_owned())
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            rows,
            height,
            move |ui: &Ui, at: usize| {
                Cow::Owned(format!(
                    "row-{}",
                    keyed(ui)
                        .get(at * lanes)
                        .map_or("", |genre| genre.id.as_str())
                ))
            },
            move |ui: &Ui, at: usize| {
                let all = genres(ui);
                let first = at * lanes;
                let cells = (first..(first + lanes).min(all.len())).map(|index| {
                    col().grow(1.).shrink().min_w(0.).child(
                        plate(ui, format!("{tag}-{}", all[index].id), &all[index])
                            .position_in_set(index + 1, all.len()),
                    )
                });
                // Short rows keep their plates as wide as the full rows'.
                let fillers = (all.len().min(first + lanes)..first + lanes)
                    .map(|_| col().grow(1.).shrink().min_w(0.));
                let line = row()
                    .w_full()
                    .gap(rems(&ui.theme, LANE_SPACE))
                    .children(cells)
                    .children(fillers);
                match at + 1 < rows {
                    true => col().w_full().pb(LANE_GAP).child(line),
                    false => line,
                }
            },
        )
}

/// How many columns a grid of genre plates or a section's lanes lay out `width` wide.
pub(crate) fn lanes(width: f32) -> usize {
    ((width / PLATE).floor().max(1.) as usize).min(LANES)
}

/// Deals list rows into lanes side by side.
fn spread(columns: Vec<Vec<El>>) -> El {
    row()
        .w_full()
        .gap(LANE_GAP)
        .children(columns.into_iter().map(|column| {
            col()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .gap(LANE_GAP)
                .children(column)
        }))
}

/// How tall a section's heading row stands.
fn head(theme: &Theme) -> f32 {
    (theme.text(Text::Title) * LEADING)
        .round()
        .max(theme.metrics.control_small)
}

/// A shelf or rail title as a level two heading, so screen readers can jump from one shelf to
/// the next. `key` has to be unique on the page.
fn heading(key: String, title: String, theme: &Theme) -> El {
    spoken_heading(key, title, 2, theme)
}

/// The tile of a shelf item `width` wide, flat on the page, as a rail draws it.
pub(crate) fn tile(
    ui: &Ui,
    key: impl Into<String>,
    item: &GenreItem,
    width: f32,
    on_item: Option<&OnItem>,
) -> El {
    cards::item_card(ui, key, item, Fit::Tile(width), on_item)
        .flat()
        .build(&ui.theme)
}
