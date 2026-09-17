use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, ClickEvent, Entity, MouseDownEvent, Pixels, SharedString, Window, div};
use music::Track;
use state::Playback;
use ui::{ActiveTheme as _, Button, Card, eyebrow, heading, snapped, vacant};

use crate::shared::track_card::{ContextHandler, StartHandler, TrackCard};

const ROWS: usize = 5;
const MAX_COLUMNS: usize = 3;
const MIN_COLUMN_WIDTH: Pixels = gpui::px(280.);

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(crate) fn column_count(width: Pixels) -> usize {
    ((width / MIN_COLUMN_WIDTH).floor().max(1.) as usize).min(MAX_COLUMNS)
}

#[derive(Clone, Copy)]
pub(crate) struct Shape {
    pub(crate) columns: usize,
    pub(crate) pages: usize,
}

impl Shape {
    pub(crate) fn new(width: Pixels, count: usize) -> Self {
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

#[derive(IntoElement)]
pub(crate) struct Picks {
    id: &'static str,
    title: &'static str,
    eyebrow: Option<&'static str>,
    vacancy: &'static str,
    tracks: Rc<Vec<Track>>,
    playback: Entity<Playback>,
    active: Option<String>,
    width: Pixels,
    page: usize,
    detailed: bool,
    loading: bool,
    on_previous: Option<ClickHandler>,
    on_next: Option<ClickHandler>,
    on_context_menu: Option<ContextHandler>,
    on_start: Option<StartHandler>,
}

impl Picks {
    pub(crate) fn new(
        id: &'static str,
        tracks: Rc<Vec<Track>>,
        playback: Entity<Playback>,
        active: Option<String>,
        width: Pixels,
        page: usize,
    ) -> Self {
        Self {
            id,
            title: "",
            eyebrow: None,
            vacancy: "",
            tracks,
            playback,
            active,
            width,
            page,
            detailed: false,
            loading: false,
            on_previous: None,
            on_next: None,
            on_context_menu: None,
            on_start: None,
        }
    }

    pub(crate) fn title(mut self, key: &'static str) -> Self {
        self.title = key;
        self
    }

    pub(crate) fn eyebrow(mut self, key: &'static str) -> Self {
        self.eyebrow = Some(key);
        self
    }

    pub(crate) fn vacancy(mut self, key: &'static str) -> Self {
        self.vacancy = key;
        self
    }

    pub(crate) fn detailed(mut self) -> Self {
        self.detailed = true;
        self
    }

    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub(crate) fn on_previous(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_previous = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_next(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_next = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_context_menu(
        mut self,
        handler: impl Fn(usize, &MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_context_menu = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_start(mut self, handler: impl Fn(usize, &mut App) + 'static) -> Self {
        self.on_start = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Picks {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let shape = Shape::new(self.width, self.tracks.len());
        let page = self.page.min(shape.pages.saturating_sub(1));
        let start = page * shape.slots();
        let row = snapped(theme.metrics.list_row, window);
        let tracks = self.tracks;
        let empty = tracks.is_empty();
        let barren = empty && !self.loading;
        let id = self.id;
        let on_previous = self.on_previous;
        let on_next = self.on_next;
        let on_context_menu = self.on_context_menu;
        let on_start = self.on_start;

        div()
            .w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .children(self.eyebrow.map(|key| eyebrow(i18n::lookup(key, None), cx)))
                            .child(heading(i18n::lookup(self.title, None), cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!("{id}-previous")))
                                    .small()
                                    .outline()
                                    .icon("icons/chevron-left.svg")
                                    .tooltip("common-previous")
                                    .disabled(empty || page == 0)
                                    .when_some(on_previous, |button, handler| {
                                        button.on_click(move |event, window, cx| {
                                            handler(event, window, cx)
                                        })
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("{id}-next")))
                                    .small()
                                    .outline()
                                    .icon("icons/chevron-right.svg")
                                    .tooltip("common-next")
                                    .disabled(empty || page + 1 >= shape.pages)
                                    .when_some(on_next, |button, handler| {
                                        button.on_click(move |event, window, cx| {
                                            handler(event, window, cx)
                                        })
                                    }),
                            ),
                    ),
            )
            .when(barren, |this| {
                this.child(vacant(i18n::lookup(self.vacancy, None), cx))
            })
            .when(!barren, |this| {
                this.child(div().flex().gap_2().p_2().when_else(
                    self.loading,
                    |this| {
                        this.children((0..shape.columns).map(|column| {
                            column_shell(column, theme.border)
                                .children((0..ROWS).map(|slot| skeleton(id, column * ROWS + slot)))
                        }))
                    },
                    |this| {
                        this.children((0..shape.columns).map(|column| {
                            column_shell(column, theme.border).children((0..ROWS).map(|slot| {
                                let place = start + column * ROWS + slot;
                                match tracks.get(place) {
                                    None => div().flex_none().h(row).into_any_element(),
                                    Some(_) => TrackCard::new(
                                        id,
                                        place,
                                        tracks.clone(),
                                        self.playback.clone(),
                                        self.active.as_deref(),
                                    )
                                    .detailed(self.detailed)
                                    .context(on_context_menu.clone())
                                    .start(on_start.clone())
                                    .render(cx)
                                    .into_any_element(),
                                }
                            }))
                        }))
                    },
                ))
            })
    }
}

fn column_shell(column: usize, border: gpui::Hsla) -> gpui::Div {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .flex_col()
        .gap_1()
        .when(column > 0, |this| {
            this.border_l_1().border_color(border).pl_2()
        })
}

fn skeleton(id: &'static str, place: usize) -> impl IntoElement {
    Card::skeleton((id, place))
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{ROWS, Shape, column_count};

    #[test]
    fn columns_follow_width_and_stop_at_three() {
        assert_eq!(column_count(px(279.)), 1);
        assert_eq!(column_count(px(560.)), 2);
        assert_eq!(column_count(px(840.)), 3);
        assert_eq!(column_count(px(2_000.)), 3);
    }

    #[test]
    fn pages_include_every_track() {
        assert_eq!(Shape::new(px(279.), 30).pages, 6);
        assert_eq!(Shape::new(px(560.), 30).pages, 3);
        assert_eq!(Shape::new(px(840.), 30).pages, 2);
        assert_eq!(Shape::new(px(840.), 0).pages, 1);
    }

    #[test]
    fn the_deck_keeps_its_shape_whatever_it_holds() {
        for count in 0..80 {
            for width in [px(279.), px(560.), px(840.)] {
                let shape = Shape::new(width, count);
                assert_eq!(shape.slots(), shape.columns * ROWS);
                assert!(shape.pages * shape.slots() >= count);
                assert!((shape.pages - 1) * shape.slots() < count.max(1));
            }
        }
    }
}
