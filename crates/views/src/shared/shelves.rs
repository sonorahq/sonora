use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, EntityId, FontWeight, MouseDownEvent, Pixels,
    Point, ScrollHandle, ScrollWheelEvent, SharedString, WeakEntity, Window, div, point, px,
};
use std::rc::Rc;

use music::{Album, GenreItem, GenreSection, Playlist};
use router::{Destination, navigate};
use state::Playback;
use ui::{
    ActiveTheme as _, Button, Card, Deck, Glide, Mode, Popup, Skeleton, Text, heading, snapped,
};

use crate::shared::album_grid::CardGrid;
use crate::shared::cards;
use crate::shared::menus::Item;

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

type Rail = (ScrollHandle, Glide);

pub(crate) struct Shelves {
    id: &'static str,
    host: EntityId,
    playback: Entity<Playback>,
    rails: Vec<Rail>,
    context_menu: Option<(Item, Point<Pixels>)>,
}

impl Shelves {
    pub(crate) fn new(id: &'static str, host: EntityId, playback: Entity<Playback>) -> Self {
        Self {
            id,
            host,
            playback,
            rails: Vec::new(),
            context_menu: None,
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
        self.context_menu = None;
    }

    fn popup(&self, cx: &mut Context<Self>) -> Option<Popup> {
        let (item, at) = self.context_menu.clone()?;

        Some(
            Popup::new(at, item.menu(self.playback.clone(), false, cx)).on_close(cx.listener(
                |this, _, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                },
            )),
        )
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
                    Mode::List => shelves.lane(place, section, width, &holder, window, cx),
                }
            });

        div()
            .relative()
            .w_full()
            .child(stack)
            .children(self.popup(cx))
            .into_any_element()
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
        me: &WeakEntity<Self>,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let lanes = lanes(width);
        let cards = section
            .items
            .iter()
            .take(lanes * ROWS)
            .enumerate()
            .map(|(index, item)| self.card(place * 100 + index, item, None, me, cx))
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
                    .child(heading(SharedString::from(section.title.clone()), cx)),
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
                    .child(heading(SharedString::from(section.title.clone()), cx))
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
                            if event.delta.precise() {
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
                                let Some(item) =
                                    feed.get(place).and_then(|section| section.items.get(index))
                                else {
                                    return div().into_any_element();
                                };

                                let holder = view.downgrade();
                                view.read(cx).card(
                                    place * 100 + index,
                                    item,
                                    Some(card),
                                    &holder,
                                    cx,
                                )
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

    fn card(
        &self,
        id: usize,
        item: &GenreItem,
        tile: Option<Pixels>,
        me: &WeakEntity<Self>,
        cx: &App,
    ) -> AnyElement {
        match item {
            GenreItem::Playlist(playlist) => self.playlist_card(id, playlist, tile, me, cx),
            GenreItem::Album(album) => self.album_card(id, album, tile, me, cx),
            GenreItem::Genre(genre) => plate(slot("genre", id), genre, tile, cx),
        }
    }

    fn playlist_card(
        &self,
        id: usize,
        playlist: &Playlist,
        tile: Option<Pixels>,
        me: &WeakEntity<Self>,
        cx: &App,
    ) -> AnyElement {
        cards::playlist_card(slot("playlist", id), playlist, &self.playback, cx)
            .map(|card| dressed(card, tile, cx))
            .menu(opener(me, Item::Playlist(playlist.clone())))
            .into_any_element()
    }

    fn album_card(
        &self,
        id: usize,
        album: &Album,
        tile: Option<Pixels>,
        me: &WeakEntity<Self>,
        cx: &App,
    ) -> AnyElement {
        cards::album_card(slot("album", id), album, &self.playback, cx)
            .map(|card| dressed(card, tile, cx))
            .menu(opener(me, Item::Album(album.clone())))
            .into_any_element()
    }
}

fn opener(
    me: &WeakEntity<Shelves>,
    item: Item,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    let me = me.clone();

    move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
        let at = event.position;
        me.update(cx, |this, cx| {
            this.context_menu = Some((item.clone(), at));
            cx.notify();
        })
        .ok();
    }
}

pub(crate) fn plate(
    id: impl Into<ElementId>,
    genre: &music::Genre,
    tile: Option<Pixels>,
    cx: &App,
) -> AnyElement {
    let opened = SharedString::from(genre.id.clone());

    Card::new(id, SharedString::from(genre.name.clone()))
        .cover(genre.cover.clone())
        .fallback("icons/music.svg")
        .weight(FontWeight::SEMIBOLD)
        .map(|card| dressed(card, tile, cx))
        .press(move |_, _, cx| navigate(Destination::Genre(opened.clone()), cx))
        .into_any_element()
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
            let cells = (first..(first + lanes).min(genres.len()))
                .map(|index| plate((id, index), &genres[index], None, cx));

            div()
                .flex()
                .w_full()
                .gap_2()
                .children(cells.map(|cell| div().flex().flex_col().flex_1().min_w_0().child(cell)))
                .into_any_element()
        })
        .into_any_element()
}

fn lanes(width: Pixels) -> usize {
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

fn slot(kind: &'static str, place: usize) -> ElementId {
    ElementId::NamedInteger(SharedString::new_static(kind), place as u64)
}
