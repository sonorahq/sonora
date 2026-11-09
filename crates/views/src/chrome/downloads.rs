//! The title bar's button for what is downloading. It shows a ring filling as the downloads go
//! and how many songs are left, with a cross beside it that cancels them all. Its menu lists the
//! songs still to come under the album, playlist or pick that asked for them, in a list that only
//! builds the rows in view. It draws nothing while no download is under way.

use std::borrow::Cow;
use std::rc::Rc;
use ui::Smooth as _;

use i18n::t;
use router::Destination;
use rune::{Cursor, Cx, El, GrowthSpec, ScrollSpec, col, el, row, text};
use state::{Active, Collection, Pending, Subject};
use ui::{ArrowRing, Button, Cover, Menu, Picker, Text, Theme, Weight};

use crate::chrome::player_bar::rems;
use crate::{Ui, act};

/// The popover key of the downloads menu among the chrome's popovers.
const DOWNLOADS: &str = "downloads";
/// How tall the list of running downloads grows before it scrolls.
const LIST: f32 = 420.;
/// How wide the list of running downloads is.
const WIDTH: f32 = 340.;
/// The size of the cover beside an album's or playlist's name.
const COVER: f32 = 18.;
const BULLET: &str = "·";

/// One line of the list of running downloads.
enum Row {
    /// The name and cover over the songs of an album or playlist, which opens its page and
    /// cancels its download. A `Song` subject heads the songs picked on their own instead.
    Heading {
        subject: Subject,
        id: String,
        name: String,
        cover: Option<String>,
    },
    /// A song still to come, `place` of all the songs in the list. `credit` names its artists
    /// after the title, which an album leaves out since they are the same on every row. `alone`
    /// is set for one picked on its own, which the row cancels.
    Song {
        group: String,
        pending: Pending,
        credit: bool,
        alone: bool,
        place: usize,
    },
}

/// The button and the cross beside it, or nothing while nothing downloads.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let _ = cx;
    let theme = &ui.theme;
    let Some(remaining) = ui.store.downloads.remaining() else {
        return el().key("downloads");
    };
    let open = ui.chrome.popovers.shows(DOWNLOADS);
    let mut menu = Menu::<Ui>::new("downloads-menu").width(WIDTH);
    if open {
        let (rows, songs) = rows(ui.store.downloads.active());
        let mut footer = row()
            .key("downloads-footer")
            .center_y()
            .justify_between()
            .gap(rems(theme, 0.5))
            .px(rems(theme, 0.5))
            .pt(rems(theme, 0.5))
            .pb(rems(theme, 0.25))
            .border_sides([1., 0., 0., 0.], theme.border.color())
            .child(small(
                format!("{}%", (remaining.share * 100.).floor() as usize),
                theme,
            ));
        if let Some(eta) = remaining.eta {
            let minutes = eta.as_secs().div_ceil(60).max(1) as usize;
            footer = footer.child(small(t!("downloads-eta", minutes = minutes).into(), theme));
        }
        menu = menu.header(
            col()
                .gap(rems(theme, 0.25))
                .child(list(theme, rows, songs))
                .child(footer),
        );
    }
    let picker = Picker::plain(
        DOWNLOADS,
        &ui.chrome.popovers,
        |ui: &mut Ui| &mut ui.chrome.popovers,
        t!("downloads-left", count = remaining.songs),
    )
    .ghost()
    .name(t!("downloads-active"))
    .leading(
        ArrowRing::new(Some(remaining.share))
            .color(theme.primary)
            .build(theme),
    )
    .menu(menu)
    .build(theme, layer);
    row().key("downloads").center_y().child(picker).child(
        cancel("downloads-cancel-all", theme)
            .tooltip(t!("downloads-cancel-all"))
            .on_press(act(|ui: &mut Ui| ui.store.cancel_all_downloads()))
            .build(theme),
    )
}

/// The scrolling list of rows, as tall as they are up to `LIST`.
fn list(theme: &Theme, rows: Vec<Row>, songs: usize) -> El {
    let height = theme.metrics.control_small;
    let tall = (height * rows.len() as f32).min(LIST);
    let rows: Rc<[Row]> = rows.into();
    let keys = rows.clone();
    col()
        .key("downloads-rows")
        .w_full()
        .h(tall)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .smooth::<Ui>("downloads-rows")
        .child(
            col()
                .key("downloads-list")
                .role("menu")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    rows.len(),
                    height,
                    move |_: &Ui, at| Cow::Owned(key(&keys[at])),
                    move |ui: &Ui, at| line(ui, &rows[at], songs),
                ),
        )
}

/// A row's key, unique in the list.
fn key(row: &Row) -> String {
    match row {
        Row::Heading {
            subject: Subject::Song,
            ..
        } => "download-heading-songs".into(),
        Row::Heading { subject, id, .. } => format!("download-heading-{subject:?}-{id}"),
        Row::Song { group, pending, .. } => format!("download-song-{group}-{}", pending.id),
    }
}

fn line(ui: &Ui, entry: &Row, songs: usize) -> El {
    let theme = &ui.theme;
    let base = || {
        row()
            .w_full()
            .min_w(0.)
            .h(theme.metrics.control_small)
            .center_y()
            .gap(rems(theme, 0.5))
            .px(rems(theme, 0.5))
    };
    match entry {
        Row::Heading {
            subject: Subject::Song,
            ..
        } => base().child(ui::faint(t!("downloads-songs"), theme)),
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
            let cancelled = id.clone();
            base()
                .child(
                    row()
                        .key(format!("download-open-{key}"))
                        .role("link")
                        .label(name.clone())
                        .focusable(true)
                        .grow(1.)
                        .shrink()
                        .min_w(0.)
                        .center_y()
                        .gap(rems(theme, 0.5))
                        .cursor(Cursor::Pointer)
                        .hover_group()
                        .press_scale(ui::PRESS_SCALE)
                        .on_press(act(move |ui: &mut Ui| {
                            ui.chrome.popovers.close();
                            ui.go(destination.clone());
                        }))
                        .child(
                            Cover::new(ui.artwork(), cover.as_deref())
                                .size(COVER)
                                .build(theme),
                        )
                        .child(
                            ui::faint(name.clone(), theme)
                                .truncate()
                                .shrink()
                                .min_w(0.)
                                .group_color(theme.foreground.color())
                                .a11y_hidden(),
                        ),
                )
                .child(
                    cancel(&format!("download-cancel-{key}"), theme)
                        .on_press(act(move |ui: &mut Ui| {
                            ui.store.cancel_download_set(kind, &cancelled)
                        }))
                        .build(theme),
                )
        }
        Row::Song {
            group,
            pending,
            credit,
            alone,
            place,
        } => {
            let key = format!("{group}-{}", pending.id);
            let song = pending.id.clone();
            let mut words = row()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .gap(rems(theme, 0.25))
                .child(
                    text(pending.name.clone())
                        .font(ui::font(theme, Text::Body, Weight::NORMAL))
                        .color(theme.foreground.color())
                        .truncate()
                        .shrink()
                        .min_w(0.),
                );
            if *credit && !pending.artists.is_empty() {
                words = words.child(
                    text(format!("{BULLET} {}", pending.artists))
                        .font(ui::font(theme, Text::Body, Weight::NORMAL))
                        .color(theme.muted_foreground.color())
                        .truncate()
                        .grow(1.)
                        .shrink()
                        .min_w(0.),
                );
            }
            let mut line = base()
                .key(format!("download-song-{key}"))
                .role("menuitem")
                .label(pending.name.clone())
                .position_in_set(*place, songs)
                .radius(theme.radius)
                .focusable(true)
                .cursor(Cursor::Pointer)
                .hover(theme.secondary_hover.color())
                .press_scale(ui::PRESS_SCALE)
                .on_press(act(move |ui: &mut Ui| {
                    ui.chrome.popovers.close();
                    ui.go(Destination::Song(song.clone().into()));
                }))
                .child(ring(pending, theme))
                .child(words);
            if *alone {
                let id = pending.id.clone();
                line = line.child(
                    cancel(&format!("download-cancel-{key}"), theme)
                        .on_press(act(move |ui: &mut Ui| ui.store.cancel_download_song(&id)))
                        .build(theme),
                );
            }
            line
        }
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
        let group = format!("{:?}-{}", set.subject, set.id);
        let credit = set.subject != Subject::Album;
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
                credit,
                alone: false,
                place,
            });
        }
    }
    if !alone.is_empty() {
        rows.push(Row::Heading {
            subject: Subject::Song,
            id: String::new(),
            name: String::new(),
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
fn cancel(key: &str, theme: &Theme) -> Button {
    let _ = theme;
    Button::new(key)
        .ghost()
        .small()
        .icon("icons/x.svg")
        .tooltip(t!("downloads-cancel"))
}

/// The ring before a song: filling as it arrives while it runs, and empty while it waits.
fn ring(pending: &Pending, theme: &Theme) -> El {
    match pending.running {
        true => ArrowRing::new(pending.fraction).color(theme.primary),
        false => ArrowRing::new(Some(0.)),
    }
    .build(theme)
}

/// A line in the small muted type the footer reads in.
fn small(words: String, theme: &Theme) -> El {
    text(words)
        .font(ui::font(theme, Text::Small, Weight::NORMAL))
        .color(theme.muted_foreground.color())
}
