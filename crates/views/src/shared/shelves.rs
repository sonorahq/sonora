use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, EntityId, Pixels, Point, ScrollHandle,
    ScrollWheelEvent, SharedString, Stateful, WeakEntity, Window, div, point, px,
};
use std::rc::Rc;

use music::{GenreItem, GenreSection};
use state::Playback;
use ui::{ActiveTheme as _, Button, Card, Deck, Glide, Mode, Skeleton, Text, heading, snapped};

use crate::shared::album_grid::CardGrid;
use crate::shared::cards;

const PLATE: Pixels = px(260.);
const LANES: usize = 5;
const ROWS: usize = 3;
const STEADY: Pixels = px(0.5);
const PENDING: usize = 3;
const RAIL_GAP: Pixels = px(16.);
const STACK_GAP: Pixels = px(32.);
const LANE_GAP: Pixels = px(8.);
const HEADING_GAP: Pixels = px(12.);
const LEADING: f32 = 1.4;
const HEADING: Pixels = px(140.);

pub(crate) struct Shelves {
    id: &'static str,
    host: EntityId,
    playback: Entity<Playback>,
    rails: Vec<(ScrollHandle, Glide)>,
}

impl Shelves {
    pub(crate) fn new(id: &'static str, host: EntityId, playback: Entity<Playback>) -> Self {
        Self {
            id,
            host,
            playback,
            rails: Vec::new(),
        }
    }

    fn tag(&self, kind: &str, place: usize) -> SharedString {
        SharedString::from(format!("{}-{kind}-{place}", self.id))
    }

    pub(crate) fn pending(&self, width: Pixels, cx: &App) -> Vec<AnyElement> {
        let theme = *cx.theme();
        let layout = CardGrid::layout(width);

        (0..PENDING)
            .map(|shelf| {
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        Skeleton::new()
                            .w(HEADING)
                            .h(theme.text(Text::Large))
                            .rounded(theme.radius),
                    )
                    .child(div().flex().w_full().gap_4().overflow_hidden().children(
                        (0..layout.columns).map(|place| {
                            Card::skeleton(self.tag("pending", shelf * 100 + place))
                                .tile(layout.card)
                        }),
                    ))
                    .into_any_element()
            })
            .collect()
    }

    pub(crate) fn reset(&mut self) {
        self.rails.clear();
    }

    pub(crate) fn render(
        &mut self,
        sections: Rc<Vec<GenreSection>>,
        mode: Mode,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        while self.rails.len() < sections.len() {
            let mut glide = Glide::default();
            glide.watch(self.host);
            self.rails.push((ScrollHandle::new(), glide));
        }
        for (scroll, glide) in &self.rails {
            glide.sync(scroll);
        }

        let heights: Vec<Pixels> = sections
            .iter()
            .map(|section| self.height(section, mode, width, window, cx))
            .collect();
        let me = cx.entity().downgrade();

        let stack = Deck::new(self.tag("stack", 0))
            .rows(heights)
            .gap(STACK_GAP)
            .draw(move |place, window, cx| {
                let Some(view) = me.upgrade() else {
                    return div().into_any_element();
                };
                let Some(section) = sections.get(place) else {
                    return div().into_any_element();
                };
                let holder = view.downgrade();
                let shelves = view.read(cx);

                match mode {
                    Mode::Grid => shelves.rail(place, &sections, width, &holder, window, cx),
                    Mode::List => shelves.lane(place, section, width, window, cx),
                }
            });

        div().relative().w_full().child(stack).into_any_element()
    }

    fn height(
        &self,
        section: &GenreSection,
        mode: Mode,
        width: Pixels,
        window: &Window,
        cx: &App,
    ) -> Pixels {
        let theme = *cx.theme();
        let head = head(window, cx) + HEADING_GAP;
        let body = match mode {
            Mode::Grid => Card::tile_height(CardGrid::layout(width).card, window, cx),
            Mode::List => {
                let lanes = lanes(width);
                let rows = section.items.len().min(lanes * ROWS).div_ceil(lanes);
                let row = snapped(theme.metrics.list_row, window);
                row * rows as f32 + LANE_GAP * rows.saturating_sub(1) as f32
            }
        };

        head + body
    }

    fn lane(
        &self,
        place: usize,
        section: &GenreSection,
        width: Pixels,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let lanes = lanes(width);
        let total = section.items.len().min(lanes * ROWS);
        let cards = section
            .items
            .iter()
            .take(total)
            .enumerate()
            .map(|(index, item)| {
                self.card(place * 100 + index, item, None, cx)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(total)
                    .into_any_element()
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_end()
                    .h(head(window, cx))
                    .child(shelf_heading(
                        self.tag("heading", place),
                        i18n::translate(&section.title),
                        cx,
                    )),
            )
            .child(spread(cards, lanes))
            .into_any_element()
    }

    fn rail(
        &self,
        place: usize,
        sections: &Rc<Vec<GenreSection>>,
        width: Pixels,
        me: &WeakEntity<Self>,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let Some(section) = sections.get(place) else {
            return div().into_any_element();
        };
        let layout = CardGrid::layout(width);
        let (handle, glide) = self.rails[place].clone();
        let crowded = section.items.len() > layout.columns;
        let feed = sections.clone();
        let drawn = me.clone();
        let card = layout.card;
        let tall = Card::tile_height(card, window, cx);

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .h(head(window, cx))
                    .child(shelf_heading(
                        self.tag("heading", place),
                        i18n::translate(&section.title),
                        cx,
                    ))
                    .when(crowded, |this| {
                        this.child(self.arrows(place, &handle, &glide, me))
                    }),
            )
            .child(
                div()
                    .id((self.id, place))
                    .w_full()
                    .h(tall)
                    .overflow_x_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&handle)
                    .on_scroll_wheel({
                        let scroll = handle.clone();
                        let glide = glide.clone();
                        move |event: &ScrollWheelEvent, window, _| {
                            if !sideways(event) {
                                return;
                            }
                            glide.nudge(&scroll, window);
                        }
                    })
                    .child(
                        Deck::new(self.tag("rail", place))
                            .across()
                            .rows(section.items.iter().map(|_| card))
                            .gap(RAIL_GAP)
                            .draw(move |index, _, cx| {
                                let Some(view) = drawn.upgrade() else {
                                    return div().into_any_element();
                                };
                                let Some(items) = feed.get(place).map(|section| &section.items)
                                else {
                                    return div().into_any_element();
                                };
                                let Some(item) = items.get(index) else {
                                    return div().into_any_element();
                                };

                                view.read(cx)
                                    .card(place * 100 + index, item, Some(card), cx)
                                    .aria_position_in_set(index + 1)
                                    .aria_size_of_set(items.len())
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }

    fn arrows(
        &self,
        place: usize,
        handle: &ScrollHandle,
        glide: &Glide,
        me: &WeakEntity<Self>,
    ) -> AnyElement {
        let at = glide.goal(handle).x;
        let reach = handle.max_offset().x;

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                self.arrow(self.tag("previous", place), false, handle, glide, me)
                    .disabled(at >= -STEADY),
            )
            .child(
                self.arrow(self.tag("next", place), true, handle, glide, me)
                    .disabled(reach > Pixels::ZERO && at <= STEADY - reach),
            )
            .into_any_element()
    }

    fn arrow(
        &self,
        id: impl Into<ElementId>,
        forward: bool,
        handle: &ScrollHandle,
        glide: &Glide,
        me: &WeakEntity<Self>,
    ) -> Button {
        let handle = handle.clone();
        let glide = glide.clone();
        let me = me.clone();

        Button::new(id)
            .small()
            .outline()
            .icon(match forward {
                true => "icons/chevron-right.svg",
                false => "icons/chevron-left.svg",
            })
            .tooltip(match forward {
                true => "common-next",
                false => "common-previous",
            })
            .on_click(move |_, window, cx| {
                slide(&handle, &glide, forward, window);
                me.update(cx, |_, cx| cx.notify()).ok();
            })
    }

    fn card(&self, id: usize, item: &GenreItem, tile: Option<Pixels>, cx: &App) -> Card {
        cards::item_card(slot("item", id), item, &self.playback, cx)
            .map(|card| dressed(card, tile, cx))
    }
}

pub(crate) fn plate(
    id: impl Into<ElementId>,
    genre: &music::Genre,
    tile: Option<Pixels>,
    cx: &App,
) -> Card {
    cards::genre_card(id, genre).map(|card| dressed(card, tile, cx))
}

pub(crate) fn grid(
    id: &'static str,
    genres: Rc<Vec<music::Genre>>,
    width: Pixels,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let lanes = lanes(width);
    let row = snapped(cx.theme().metrics.list_row, window);
    let rows = genres.len().div_ceil(lanes);

    Deck::new(id)
        .rows((0..rows).map(|_| row))
        .gap(LANE_GAP)
        .draw(move |place, _, cx| {
            let first = place * lanes;
            let cells = (first..(first + lanes).min(genres.len())).map(|index| {
                plate((id, index), &genres[index], None, cx)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(genres.len())
            });

            div()
                .flex()
                .w_full()
                .gap_2()
                .children(cells.map(|cell| div().flex().flex_col().flex_1().min_w_0().child(cell)))
                .into_any_element()
        })
        .into_any_element()
}

/// How many columns a grid of genre plates `width` wide lays out.
pub(crate) fn lanes(width: Pixels) -> usize {
    ((width / PLATE).floor().max(1.) as usize).min(LANES)
}

fn spread(cards: Vec<AnyElement>, lanes: usize) -> Div {
    let mut columns: Vec<Vec<AnyElement>> = (0..lanes).map(|_| Vec::new()).collect();
    for (place, card) in cards.into_iter().enumerate() {
        columns[place % lanes].push(card);
    }

    div()
        .flex()
        .w_full()
        .gap_2()
        .children(columns.into_iter().map(|column| {
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .flex_col()
                .gap_2()
                .children(column)
        }))
}

fn head(window: &Window, cx: &App) -> Pixels {
    let theme = *cx.theme();

    snapped(theme.text(Text::Title) * LEADING, window).max(theme.metrics.control_small)
}

fn dressed(card: Card, tile: Option<Pixels>, cx: &App) -> Card {
    match tile {
        Some(width) => card.tile(width).flat(),
        None => card.bg(cx.theme().secondary),
    }
}

fn slide(handle: &ScrollHandle, glide: &Glide, forward: bool, window: &mut Window) {
    let page = handle.bounds().size.width;
    let at = glide.goal(handle);
    let next = match forward {
        true => at.x - page,
        false => at.x + page,
    };

    glide.aim(handle, point(next, at.y), window);
}

/// One horizontal card rail outside a shelf page: a heading with paging arrows over a
/// sideways scroller. Artist and album pages draw their recommendations with these, so
/// neither rebuilds the scroll plumbing a shelf keeps for its sections.
pub(crate) struct Rail {
    scroll: ScrollHandle,
    glide: Glide,
}

impl Rail {
    /// A rail that reports its scrolls to `host`, the page holding it.
    pub(crate) fn new(host: EntityId) -> Self {
        let mut glide = Glide::default();
        glide.watch(host);
        Self {
            scroll: ScrollHandle::new(),
            glide,
        }
    }

    /// Catches the glide up with its scroller before the frame draws it.
    pub(crate) fn sync(&self) {
        self.glide.sync(&self.scroll);
    }

    /// Puts the rail back at its first card at once, for when it starts showing other cards.
    pub(crate) fn rewind(&self) {
        self.glide.jump(&self.scroll, Point::default());
    }

    /// The rail under its heading: the title with paging arrows beside it while the cards
    /// overflow, the kind pills when the rail has kinds to narrow by, and the cards in a
    /// sideways scroller. `notify` repaints the page holding the rail, which the arrows
    /// need to refresh themselves.
    pub(crate) fn render(
        &self,
        spec: RailSpec,
        window: &Window,
        cx: &mut App,
        notify: &Rc<dyn Fn(&mut App)>,
        draw: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> AnyElement {
        let RailSpec {
            tag,
            place,
            title,
            count,
            tile,
            columns,
            tabs,
        } = spec;
        let tall = Card::tile_height(tile, window, cx);
        let scrolled = self.scroll.clone();
        let glided = self.glide.clone();
        let arrows = match count > columns {
            true => Some(arrows(tag, place, &self.scroll, &self.glide, notify)),
            false => None,
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .h(head(window, cx))
                    .child(shelf_heading(
                        SharedString::from(format!("{tag}-heading-{place}")),
                        title,
                        cx,
                    ))
                    .children(arrows),
            )
            .children(tabs)
            .child(
                div()
                    .id((tag, place))
                    .w_full()
                    .h(tall)
                    .overflow_x_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(move |event: &ScrollWheelEvent, window, _| {
                        if !sideways(event) {
                            return;
                        }
                        glided.nudge(&scrolled, window);
                    })
                    .child(
                        Deck::new(SharedString::from(format!("{tag}-rail-{place}")))
                            .across()
                            .rows(std::iter::repeat_n(tile, count))
                            .gap(RAIL_GAP)
                            .draw(draw),
                    ),
            )
            .into_any_element()
    }

    /// The rail while its cards are still on their way: the heading over skeleton tiles.
    pub(crate) fn pending(
        title: SharedString,
        tile: Pixels,
        columns: usize,
        window: &Window,
        cx: &mut App,
    ) -> AnyElement {
        let tall = Card::tile_height(tile, window, cx);

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_end()
                    .h(head(window, cx))
                    .child(shelf_heading(
                        SharedString::from(format!("rail-pending-heading-{title}")),
                        title,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .w_full()
                    .h(tall)
                    .gap(RAIL_GAP)
                    .overflow_hidden()
                    .children((0..columns).map(|place| {
                        Card::skeleton(("rail-pending", place))
                            .tile(tile)
                            .into_any_element()
                    })),
            )
            .into_any_element()
    }
}

/// What a rail draws: its heading and the cards behind it. `tile` is how wide one card
/// stands and `columns` how many fit across, which is what decides whether the paging
/// arrows show. `tabs` is the tab row, drawn between the heading and the cards.
pub(crate) struct RailSpec {
    pub tag: &'static str,
    pub place: usize,
    pub title: SharedString,
    pub count: usize,
    pub tile: Pixels,
    pub columns: usize,
    pub tabs: Option<AnyElement>,
}

/// The paging arrows of a rail head, which is what tells a crowded rail from a short one.
fn arrows(
    tag: &'static str,
    place: usize,
    scroll: &ScrollHandle,
    glide: &Glide,
    notify: &Rc<dyn Fn(&mut App)>,
) -> AnyElement {
    let at = glide.goal(scroll).x;
    let reach = scroll.max_offset().x;

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(
            arrow(
                SharedString::from(format!("{tag}-previous-{place}")),
                false,
                scroll,
                glide,
                notify,
            )
            .disabled(at >= -STEADY),
        )
        .child(
            arrow(
                SharedString::from(format!("{tag}-next-{place}")),
                true,
                scroll,
                glide,
                notify,
            )
            .disabled(reach > Pixels::ZERO && at <= STEADY - reach),
        )
        .into_any_element()
}

fn arrow(
    id: SharedString,
    forward: bool,
    scroll: &ScrollHandle,
    glide: &Glide,
    notify: &Rc<dyn Fn(&mut App)>,
) -> Button {
    let scroll = scroll.clone();
    let glide = glide.clone();
    let notify = notify.clone();

    Button::new(id)
        .small()
        .outline()
        .icon(match forward {
            true => "icons/chevron-right.svg",
            false => "icons/chevron-left.svg",
        })
        .tooltip(match forward {
            true => "common-next",
            false => "common-previous",
        })
        .on_click(move |_, window, cx| {
            slide(&scroll, &glide, forward, window);
            notify(cx);
        })
}

fn slot(kind: &'static str, place: usize) -> ElementId {
    ElementId::NamedInteger(SharedString::new_static(kind), place as u64)
}

/// Whether a wheel event should glide a rail. Only a notched wheel moving sideways does, since a
/// trackpad scrolls the rail itself and a vertical notch belongs to the page.
fn sideways(event: &ScrollWheelEvent) -> bool {
    !event.delta.precise() && event.delta.pixel_delta(px(1.)).x != Pixels::ZERO
}

/// A shelf or rail title as a level two heading, so screen readers can jump from one shelf to
/// the next. `id` has to be unique on the page.
fn shelf_heading(id: impl Into<ElementId>, title: SharedString, cx: &App) -> Stateful<Div> {
    heading(title.clone(), cx)
        .id(id)
        .role(Role::Heading)
        .aria_level(2)
        .aria_label(title)
}
