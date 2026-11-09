//! A framed deck of list-row cards in up to three columns of five, paged with the arrows in
//! its head: home's Quick picks, an artist's popular songs.

use std::rc::Rc;

use music::{GenreItem, Track};
use rune::{El, Transition, col, row};
use ui::{Button, Card, spoken_eyebrow, spoken_heading, spoken_vacant};

use crate::Ui;
use crate::shared::cards::{self, OnItem};
use crate::shared::track_card::{ContextHandler, StartHandler, TrackCard};

/// How many rows each column holds.
const ROWS: usize = 5;
/// The most columns a deck lays out, and the narrowest one.
const MAX_COLUMNS: usize = 3;
const MIN_COLUMN_WIDTH: f32 = 280.;
/// The deck's padding around its head and its body, and the space between its rows.
const HEAD_PAD: f32 = 16.;
const HEAD_PAD_BOTTOM: f32 = 12.;
const HEAD_GAP: f32 = 16.;
const ARROW_GAP: f32 = 4.;
const BODY_PAD: f32 = 8.;
const ROW_GAP: f32 = 4.;
/// The hairline between the head and the body, and between columns.
const RULE: f32 = 1.;

/// Moves a deck to another page.
pub(crate) type PageHandler = Rc<dyn Fn(&mut Ui, usize)>;

/// How many columns a deck `width` wide lays out.
pub(crate) fn column_count(width: f32) -> usize {
    ((width / MIN_COLUMN_WIDTH).floor().max(1.) as usize).min(MAX_COLUMNS)
}

/// A deck's columns and how many pages its rows take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) columns: usize,
    pub(crate) pages: usize,
}

impl Shape {
    pub(crate) fn new(width: f32, count: usize) -> Self {
        let columns = column_count(width);

        Self {
            columns,
            pages: count.div_ceil(columns * ROWS).max(1),
        }
    }

    fn slots(self) -> usize {
        self.columns * ROWS
    }
}

/// What the deck lists: a run of tracks that queue together, or shelf items of any kind that
/// each stand alone.
enum Rows<'a> {
    Tracks(&'a [Track]),
    Mixed(&'a [GenreItem]),
}

impl Rows<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Tracks(tracks) => tracks.len(),
            Self::Mixed(items) => items.len(),
        }
    }
}

/// The deck. The page it shows lives in the screen's state; [`Picks::on_page`] moves it.
pub(crate) struct Picks<'a> {
    key: &'a str,
    title: &'static str,
    eyebrow: Option<String>,
    vacancy: &'static str,
    rows: Rows<'a>,
    width: f32,
    page: usize,
    detailed: bool,
    loading: bool,
    on_page: Option<PageHandler>,
    on_context: Option<ContextHandler>,
    on_start: Option<StartHandler>,
    on_item: Option<OnItem>,
}

impl<'a> Picks<'a> {
    /// A deck of `tracks`, `width` wide, showing `page`.
    pub(crate) fn new(key: &'a str, tracks: &'a [Track], width: f32, page: usize) -> Self {
        Self {
            key,
            title: "",
            eyebrow: None,
            vacancy: "",
            rows: Rows::Tracks(tracks),
            width,
            page,
            detailed: false,
            loading: false,
            on_page: None,
            on_context: None,
            on_start: None,
            on_item: None,
        }
    }

    /// A deck of shelf items rather than tracks: each row plays or opens on its own, so
    /// `on_start` goes unused, and [`Picks::on_item`] opens a row's menu.
    pub(crate) fn mixed(key: &'a str, items: &'a [GenreItem], width: f32, page: usize) -> Self {
        Self {
            rows: Rows::Mixed(items),
            ..Self::new(key, &[], width, page)
        }
    }

    /// The Fluent key of the deck's heading.
    pub(crate) fn title(mut self, key: &'static str) -> Self {
        self.title = key;
        self
    }

    /// The small line over the title, already worded.
    pub(crate) fn eyebrow(mut self, label: impl Into<String>) -> Self {
        self.eyebrow = Some(label.into());
        self
    }

    /// The Fluent key of what an empty deck says.
    pub(crate) fn vacancy(mut self, key: &'static str) -> Self {
        self.vacancy = key;
        self
    }

    /// Track rows with their lengths, naming the artists only of songs that credit several.
    pub(crate) fn detailed(mut self) -> Self {
        self.detailed = true;
        self
    }

    /// Skeleton rows while the deck has nothing yet.
    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Moves the deck to a page, from the arrows.
    pub(crate) fn on_page(mut self, f: impl Fn(&mut Ui, usize) + 'static) -> Self {
        self.on_page = Some(Rc::new(f));
        self
    }

    /// A right-click on a track row, by its place among the tracks.
    pub(crate) fn on_context(
        mut self,
        f: impl Fn(&mut Ui, usize, rune::PointerPress, &mut rune::Tx) + 'static,
    ) -> Self {
        self.on_context = Some(Rc::new(f));
        self
    }

    /// What pressing a track row starts, by its place, in place of the track's radio.
    pub(crate) fn on_start(mut self, f: impl Fn(&mut Ui, usize) + 'static) -> Self {
        self.on_start = Some(Rc::new(f));
        self
    }

    /// A right-click on a row of a mixed deck.
    pub(crate) fn on_item(mut self, on_item: OnItem) -> Self {
        self.on_item = Some(on_item);
        self
    }

    pub(crate) fn build(self, ui: &Ui) -> El {
        let theme = &ui.theme;
        let key = self.key;
        let count = self.rows.len();
        let shape = Shape::new(self.width, count);
        let page = self.page.min(shape.pages.saturating_sub(1));
        let start = page * shape.slots();
        let empty = count == 0;
        let barren = empty && !self.loading;
        let row_height = theme.metrics.list_row.round();

        let arrow = |side: &str, glyph: &str, tip: &str, disabled: bool, to: usize| {
            let mut button = Button::new(format!("{key}-{side}"))
                .small()
                .outline()
                .icon(glyph)
                .tooltip(i18n::lookup(tip, None))
                .disabled(disabled);
            if let Some(on_page) = self.on_page.clone() {
                button = button.on_press(move |ui: &mut Ui| on_page(ui, to));
            }
            button.build(theme)
        };
        let arrows = row()
            .key("arrows")
            .center_y()
            .gap(ARROW_GAP)
            .pb(ROW_GAP)
            .child(arrow(
                "previous",
                "icons/chevron-left.svg",
                "common-previous",
                empty || page == 0,
                page.saturating_sub(1),
            ))
            .child(arrow(
                "next",
                "icons/chevron-right.svg",
                "common-next",
                empty || page + 1 >= shape.pages,
                (page + 1).min(shape.pages.saturating_sub(1)),
            ));
        let title = i18n::lookup(self.title, None).into_owned();
        let heading = col()
            .key("titles")
            .min_w(0.)
            .children(
                self.eyebrow
                    .map(|label| spoken_eyebrow(format!("{key}-eyebrow"), label, theme)),
            )
            .child(spoken_heading(format!("{key}-heading"), title, 2, theme));
        let head = row()
            .key("head")
            .w_full()
            .items_end()
            .justify_between()
            .gap(HEAD_GAP)
            .px(HEAD_PAD)
            .pt(HEAD_PAD)
            .pb(HEAD_PAD_BOTTOM)
            .border_sides([0., 0., RULE, 0.], theme.border.color())
            .child(heading)
            .child(arrows);

        let mut deck = col()
            .key(key.to_owned())
            .role("group")
            .w_full()
            .radius(theme.radius)
            .border(RULE, theme.border.color())
            .clip_rounded(theme.radius)
            .child(head);
        if barren {
            let vacancy = i18n::lookup(self.vacancy, None).into_owned();
            return deck.child(spoken_vacant(format!("{key}-vacancy"), vacancy, theme));
        }

        let column = |column: usize, rows: Vec<El>| {
            let shell = col()
                .key(format!("column-{column}"))
                .grow(1.)
                .shrink()
                .min_w(0.)
                .gap(ROW_GAP)
                .children(rows);
            match column > 0 {
                true => shell
                    .border_sides([RULE, 0., 0., 0.], theme.border.color())
                    .pl(BODY_PAD),
                false => shell,
            }
        };
        let columns: Vec<El> = (0..shape.columns)
            .map(|at| {
                let rows = (0..ROWS)
                    .map(|slot| {
                        let place = start + at * ROWS + slot;
                        match empty {
                            true => Card::skeleton(format!("{key}-skeleton-{place}"))
                                .build(theme),
                            false => self
                                .row(ui, place)
                                .unwrap_or_else(|| row().key(format!("gap-{slot}")).h(row_height)),
                        }
                    })
                    .collect();
                column(at, rows)
            })
            .collect();
        // Each page is its own element, so turning one cross-fades the rows.
        let body = row()
            .key(format!("page-{page}"))
            .w_full()
            .gap(BODY_PAD)
            .padding(BODY_PAD)
            .children(columns)
            .transition(Transition::opacity());
        deck = deck.child(body);
        deck
    }

    /// The row at `place`, or nothing past the end.
    fn row(&self, ui: &Ui, place: usize) -> Option<El> {
        let key = self.key;
        match &self.rows {
            Rows::Tracks(tracks) => TrackCard::new(key, place, tracks)
                .detailed(self.detailed)
                .context(self.on_context.clone())
                .start(self.on_start.clone())
                .build(ui),
            Rows::Mixed(items) => {
                let item = items.get(place)?;
                Some(
                    cards::listed(ui, format!("{key}-{place}"), item, self.on_item.as_ref())
                        .build(&ui.theme)
                        .min_w(0.)
                        .position_in_set(place + 1, items.len()),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ROWS, Shape, column_count};

    #[test]
    fn columns_follow_width_and_stop_at_three() {
        assert_eq!(column_count(279.), 1);
        assert_eq!(column_count(560.), 2);
        assert_eq!(column_count(840.), 3);
        assert_eq!(column_count(2_000.), 3);
    }

    #[test]
    fn pages_include_every_track() {
        assert_eq!(Shape::new(279., 30).pages, 6);
        assert_eq!(Shape::new(560., 30).pages, 3);
        assert_eq!(Shape::new(840., 30).pages, 2);
        assert_eq!(Shape::new(840., 0).pages, 1);
    }

    #[test]
    fn the_deck_keeps_its_shape_whatever_it_holds() {
        for count in 0..80 {
            for width in [279., 560., 840.] {
                let shape = Shape::new(width, count);

                assert_eq!(shape.slots(), shape.columns * ROWS);
                assert!(shape.pages * shape.slots() >= count);
                assert!((shape.pages - 1) * shape.slots() < count.max(1));
            }
        }
    }
}
