use std::ops::Range;
use std::rc::Rc;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, ElementId, Entity, Pixels, SharedString, UniformListScrollHandle,
    Window, div, px, uniform_list,
};
use i18n::t;
use router::{Destination, navigate};
use state::{Active, Collection, Downloads, Pending, Sonora, Subject};
use ui::{
    ActiveTheme as _, ArrowRing, Artwork, Button, Menu, Picker, Popovers, Scrollbar, Scroller,
    Text, Theme, faint,
};

/// How tall the list of running downloads grows before it scrolls.
const LIST: Pixels = px(420.);
/// How wide the list of running downloads is.
const WIDTH: Pixels = px(340.);
/// The size of the cover beside an album's or playlist's name.
const COVER: Pixels = px(18.);
const BULLET: &str = "·";

/// One line of the list of running downloads.
enum Row {
    /// The name and cover over the songs of an album or playlist, which opens its page and
    /// cancels its download. A `Song` subject heads the songs picked on their own instead.
    Heading {
        subject: Subject,
        id: String,
        name: SharedString,
        cover: Option<String>,
    },
    /// A song still to come, `place` of all the songs in the list. `credit` names its artists
    /// after the title, which an album leaves out since they are the same on every row. `alone`
    /// is set for one picked on its own, which the row cancels.
    Song {
        group: SharedString,
        pending: Pending,
        credit: bool,
        alone: bool,
        place: usize,
    },
}

/// The title bar's button for what is downloading. It shows a ring filling as the downloads go
/// and how many songs are left, with a cross beside it that cancels them all. Its menu lists the
/// songs still to come under the album, playlist or pick that asked for them, in a list that only
/// draws the rows in view. It draws nothing while no download is under way.
pub(crate) struct DownloadsButton {
    downloads: Entity<Downloads>,
    popovers: Popovers,
    /// The rows of the open menu, read again on every draw while it is open.
    rows: Rc<[Row]>,
    /// How many song rows the list holds, which screen readers count them out of.
    songs: usize,
    scroll: UniformListScrollHandle,
    scrollbar: Entity<Scrollbar>,
}

impl DownloadsButton {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let downloads = Sonora::global(cx).downloads.clone();
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        let me = cx.entity_id();
        let scroll = UniformListScrollHandle::new();
        let scrollbar =
            cx.new(|_| Scrollbar::new(scroll.0.borrow().base_handle.clone()).watching(me));

        Self {
            downloads,
            popovers: Popovers::default(),
            rows: Rc::from([]),
            songs: 0,
            scroll,
            scrollbar,
        }
    }

    /// The scrolling list of rows, as tall as they are up to `LIST`.
    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = cx.theme().metrics.control_small;
        let height = (row * self.rows.len() as f32).min(LIST);
        div()
            .w_full()
            .h(height)
            .child(
                Scroller::listing("downloads-rows", &self.scrollbar).child(
                    uniform_list(
                        "downloads-rows",
                        self.rows.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            range
                                .map(|at| this.row(&this.rows[at], row, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                ),
            )
            .into_any_element()
    }

    fn row(&self, row: &Row, height: Pixels, cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let base = |id: String| {
            div()
                .id(ElementId::Name(id.into()))
                .flex()
                .w_full()
                .min_w_0()
                .h(height)
                .items_center()
                .gap_2()
                .px_2()
        };
        match row {
            Row::Heading {
                subject: Subject::Song,
                ..
            } => base("download-heading-songs".into())
                .child(faint(cx).child(t!("downloads-songs")))
                .into_any_element(),
            Row::Heading {
                subject,
                id,
                name,
                cover,
            } => {
                let key = format!("{subject:?}-{id}");
                let kind = match subject {
                    Subject::Album => Collection::Album,
                    _ => Collection::Playlist,
                };
                let destination = match subject {
                    Subject::Album => Destination::Album(id.clone().into()),
                    _ => Destination::Playlist(id.clone().into()),
                };
                let popovers = self.popovers.clone();
                let downloads = self.downloads.clone();
                let id = id.clone();
                base(format!("download-heading-{key}"))
                    .child(
                        div()
                            .id(ElementId::Name(format!("download-open-{key}").into()))
                            .role(Role::Link)
                            .aria_label(name.clone())
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .child(Artwork::new(cover.clone()).size(COVER).flex_none())
                            .child(
                                faint(cx)
                                    .min_w_0()
                                    .truncate()
                                    .hover(|this| this.text_color(theme.foreground))
                                    .child(name.clone()),
                            )
                            .on_click(move |_, _, cx| {
                                popovers.close();
                                navigate(destination.clone(), cx);
                                cx.refresh_windows();
                            }),
                    )
                    .child(cancel(format!("download-cancel-{key}"), move |cx| {
                        downloads.update(cx, |downloads, cx| downloads.cancel_set(kind, &id, cx));
                    }))
                    .into_any_element()
            }
            Row::Song {
                group,
                pending,
                credit,
                alone,
                place,
            } => {
                let key = format!("{group}-{}", pending.id);
                let popovers = self.popovers.clone();
                let song = pending.id.clone();
                base(format!("download-song-{key}"))
                    .role(Role::MenuItem)
                    .aria_label(pending.name.clone())
                    .aria_position_in_set(*place)
                    .aria_size_of_set(self.songs)
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.secondary_hover))
                    .child(div().flex_none().child(ring(&key, pending, &theme)))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_none()
                                    .max_w_full()
                                    .truncate()
                                    .child(pending.name.clone()),
                            )
                            .when(*credit && !pending.artists.is_empty(), |this| {
                                this.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("{BULLET} {}", pending.artists)),
                                )
                            }),
                    )
                    .when(*alone, |this| {
                        let downloads = self.downloads.clone();
                        let id = pending.id.clone();
                        this.child(cancel(format!("download-cancel-{key}"), move |cx| {
                            downloads.update(cx, |downloads, cx| downloads.cancel_song(&id, cx));
                        }))
                    })
                    .on_click(move |_, _, cx| {
                        popovers.close();
                        navigate(Destination::Song(song.clone().into()), cx);
                        cx.refresh_windows();
                    })
                    .into_any_element()
            }
        }
    }
}

impl Render for DownloadsButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(remaining) = self.downloads.read(cx).remaining() else {
            self.popovers.close();
            self.rows = Rc::from([]);
            return div().into_any_element();
        };
        let theme = *cx.theme();
        if self.popovers.any() {
            let (rows, songs) = rows(self.downloads.read(cx).active());
            self.rows = rows.into();
            self.songs = songs;
        }
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_2()
            .pt_2()
            .pb_1()
            .border_t_1()
            .border_color(theme.border)
            .text_size(theme.text(Text::Small))
            .text_color(theme.muted_foreground)
            .child(format!("{}%", (remaining.share * 100.).floor() as usize))
            .when_some(remaining.eta, |this, eta| {
                let minutes = eta.as_secs().div_ceil(60).max(1) as usize;
                this.child(t!("downloads-eta", minutes = minutes))
            });
        let menu = Menu::new("downloads-menu").w(WIDTH).header(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(self.list(cx))
                .child(footer),
        );
        let downloads = self.downloads.clone();

        div()
            .flex()
            .flex_none()
            .items_center()
            .child(
                Picker::plain(
                    "downloads",
                    &self.popovers,
                    t!("downloads-left", count = remaining.songs),
                )
                .ghost()
                .name(t!("downloads-active"))
                .leading(
                    ArrowRing::new("downloads-ring", Some(remaining.share)).color(theme.primary),
                )
                .menu(menu),
            )
            .child(
                cancel("downloads-cancel-all".into(), move |cx| {
                    downloads.update(cx, |downloads, cx| downloads.cancel_all(cx));
                })
                .tooltip("downloads-cancel-all"),
            )
            .into_any_element()
    }
}

/// The rows of the list, an album or playlist at a time and the songs picked alone last, each
/// under its heading, with how many song rows there are.
fn rows(active: Vec<Active>) -> (Vec<Row>, usize) {
    let (sets, alone): (Vec<Active>, Vec<Active>) = active
        .into_iter()
        .partition(|active| active.subject != Subject::Song);
    let mut rows = Vec::new();
    let mut place = 0;
    for set in sets {
        let group = SharedString::from(format!("{:?}-{}", set.subject, set.id));
        rows.push(Row::Heading {
            subject: set.subject,
            id: set.id,
            name: set.name,
            cover: set.cover,
        });
        for pending in set.left {
            place += 1;
            rows.push(Row::Song {
                group: group.clone(),
                pending,
                credit: set.subject != Subject::Album,
                alone: false,
                place,
            });
        }
    }
    if !alone.is_empty() {
        rows.push(Row::Heading {
            subject: Subject::Song,
            id: String::new(),
            name: SharedString::default(),
            cover: None,
        });
        for pending in alone.into_iter().flat_map(|song| song.left) {
            place += 1;
            rows.push(Row::Song {
                group: "songs".into(),
                pending,
                credit: true,
                alone: true,
                place,
            });
        }
    }
    (rows, place)
}

/// A small cross that cancels a download.
fn cancel(id: String, press: impl Fn(&mut App) + 'static) -> Button {
    Button::new(ElementId::Name(id.into()))
        .ghost()
        .small()
        .icon("icons/x.svg")
        .tooltip("downloads-cancel")
        .on_click(move |_, _, cx| press(cx))
}

/// The ring before a song: filling as it arrives while it runs, and empty while it waits.
fn ring(key: &str, pending: &Pending, theme: &Theme) -> ArrowRing {
    let id = ElementId::Name(format!("download-ring-{key}").into());
    match pending.running {
        true => ArrowRing::new(id, pending.fraction).color(theme.primary),
        false => ArrowRing::new(id, Some(0.)),
    }
}
