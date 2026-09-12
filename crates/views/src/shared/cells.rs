use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, Entity, Hsla, MouseButton, Pixels, SharedString, Stateful, Task,
    Window, div, px, svg,
};
use i18n::t;
use music::{ArtistRef, Contributor};
use router::{Destination, Link as _, navigate};
use state::{Playback, PlaybackState};
use ui::{
    ActiveTheme as _, Artwork, Avatar, Cell, ExplicitBadge, InlineLink, InlineLinks, ROW_GROUP,
    Theme, clock, tabular,
};

use crate::chrome::Chrome;
use crate::shared::hero::release_date_label;

const PLAY: &str = "icons/play.svg";
const PLAYING: &str = "icons/music-2.svg";
const PAUSE: &str = "icons/pause.svg";
const UNAVAILABLE: &str = "icons/play-off.svg";
const HOVER_PRELOAD_DELAY: Duration = Duration::from_millis(200);
const PERSON: &str = "icons/user.svg";

pub(crate) type Tap = Box<dyn Fn(&mut App)>;

pub(crate) const NUMBER: Pixels = px(44.);
pub(crate) const TRAILING: Pixels = px(72.);
pub(crate) const DATE: Pixels = px(112.);
pub(crate) const YEAR: Pixels = px(64.);
pub(crate) const HIT: Pixels = px(18.);
pub(crate) const FACE: Pixels = px(20.);

pub(crate) fn glyph(theme: &Theme) -> Pixels {
    px((theme.metrics.row / px(1.) * 0.23).round())
}

#[derive(IntoElement)]
struct Glyph {
    icon: &'static str,
    color: Hsla,
}

impl RenderOnce for Glyph {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        svg()
            .path(icons::path(self.icon))
            .size(glyph(cx.theme()))
            .text_color(self.color)
    }
}

#[derive(IntoElement)]
struct Thumb {
    row: usize,
    url: Option<String>,
}

impl RenderOnce for Thumb {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();

        Artwork::new(self.url)
            .id(("artwork", self.row))
            .size(theme.metrics.thumb)
    }
}

#[derive(IntoElement)]
struct Face {
    url: Option<String>,
}

impl RenderOnce for Face {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();

        Avatar::new(self.url).size(theme.metrics.thumb)
    }
}

pub(crate) fn index<F>(
    cell: &Cell<F>,
    state: Option<PlaybackState>,
    playable: bool,
    preload: Option<Tap>,
    press: Option<Tap>,
    cx: &App,
) -> AnyElement {
    let theme = *cx.theme();
    let faded = theme.muted_foreground.opacity(0.5);

    let dimmed = |icon, color| {
        svg()
            .path(icons::path(icon))
            .id(("index-glyph", cell.row))
            .size(glyph(&theme))
            .flex_none()
            .text_color(color)
            .group_hover(ROW_GROUP, |style| style.invisible())
            .into_any_element()
    };
    let resting = match &state {
        Some(PlaybackState::Playing) => dimmed(PLAYING, theme.foreground),
        Some(_) => dimmed(PAUSE, theme.muted_foreground),
        None => div()
            .id(("index-number", cell.row))
            .text_color(match playable {
                true => theme.muted_foreground,
                false => faded,
            })
            .group_hover(ROW_GROUP, |style| style.invisible())
            .child(format!("{}", cell.display + 1))
            .into_any_element(),
    };

    let icon = match (matches!(state, Some(PlaybackState::Playing)), playable) {
        (true, _) => PAUSE,
        (false, true) => PLAY,
        (false, false) => UNAVAILABLE,
    };
    let color = match playable {
        true => theme.foreground,
        false => theme.muted_foreground,
    };

    transport(
        cell,
        resting,
        Transport {
            icon,
            color,
            preload,
            press,
        },
    )
}

pub(crate) fn toggle<F>(
    playback: &Entity<Playback>,
    state: Option<PlaybackState>,
    start: F,
) -> Option<Tap>
where
    F: Fn(&mut Playback, &mut Context<Playback>) + 'static,
{
    let playback = playback.clone();

    Some(Box::new(move |cx: &mut App| {
        playback.update(cx, |playback, cx| match &state {
            Some(PlaybackState::Playing) => playback.pause(cx),
            Some(PlaybackState::Paused) => playback.resume(cx),
            _ => start(playback, cx),
        });
    }))
}

pub(crate) struct Transport {
    pub(crate) icon: &'static str,
    pub(crate) color: Hsla,
    pub(crate) preload: Option<Tap>,
    pub(crate) press: Option<Tap>,
}

pub(crate) fn transport<F>(cell: &Cell<F>, resting: AnyElement, hover: Transport) -> AnyElement {
    let Transport {
        icon,
        color,
        preload,
        press,
    } = hover;
    let enabled = press.is_some();
    let preload = preload.map(Rc::<dyn Fn(&mut App)>::from);
    let hover_task = Rc::new(RefCell::new(None::<Task<()>>));

    cell.frame()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id(("transport-hit", cell.row))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .size(HIT)
                .when_some(preload.clone(), |this, preload| {
                    let hover_task = hover_task.clone();
                    this.on_hover(move |hovered: &bool, _, cx| {
                        let mut task = hover_task.borrow_mut();
                        *task = if *hovered {
                            let preload = preload.clone();
                            Some(cx.spawn(async move |cx| {
                                cx.background_executor().timer(HOVER_PRELOAD_DELAY).await;
                                cx.update(|cx| preload(cx));
                            }))
                        } else {
                            None
                        };
                    })
                })
                .child(resting)
                .child(
                    div()
                        .id(("transport", cell.row))
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .invisible()
                        .group_hover(ROW_GROUP, |style| style.visible())
                        .child(Glyph { icon, color })
                        .when_else(
                            enabled,
                            |this| this.cursor_pointer(),
                            |this| this.cursor_not_allowed(),
                        )
                        .when_some(press, |this, press| {
                            let hover_task = hover_task.clone();
                            this.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                hover_task.borrow_mut().take();
                                if let Some(preload) = preload.as_ref() {
                                    preload(cx);
                                }
                            })
                            .on_click(move |_, _, cx| press(cx))
                        }),
                ),
        )
        .into_any_element()
}

pub(crate) fn link<F>(
    cell: &Cell<F>,
    id: &'static str,
    value: impl Into<SharedString>,
    color: Hsla,
    to: Destination,
) -> AnyElement {
    line(cell, Some(color))
        .id((id, cell.row))
        .hover(|style| style.underline())
        .link(to)
        .child(value.into())
        .into_any_element()
}

pub(crate) fn artist_links(
    id: impl Into<SharedString>,
    artists: Vec<ArtistRef>,
    fallback: impl Into<SharedString>,
    color: Hsla,
) -> InlineLinks {
    InlineLinks::new(
        id,
        artists
            .into_iter()
            .map(|artist| InlineLink::new(artist.name, artist.id.map(Into::into))),
        fallback,
        color,
    )
    .on_click(|id, cx| navigate(Destination::Artist(id), cx))
}

pub(crate) fn artists<F>(
    cell: &Cell<F>,
    artists: Vec<ArtistRef>,
    fallback: impl Into<SharedString>,
    color: Hsla,
) -> AnyElement {
    cell.frame()
        .child(
            artist_links(SharedString::new_static("artist"), artists, fallback, color).truncate(),
        )
        .into_any_element()
}

fn line<F>(cell: &Cell<F>, color: Option<Hsla>) -> Div {
    cell.frame()
        .when_some(color, |this, color| this.text_color(color))
}

pub(crate) fn dim<F>(cell: &Cell<F>, value: impl Into<SharedString>, muted: Hsla) -> AnyElement {
    line(cell, Some(muted))
        .child(value.into())
        .into_any_element()
}

/// A track length in tabular digits, so the column reads as a monospace strip.
pub(crate) fn length<F>(cell: &Cell<F>, value: Duration, muted: Hsla) -> AnyElement {
    line(cell, Some(muted))
        .font_features(tabular())
        .child(clock(value))
        .into_any_element()
}

pub(crate) fn stamp(seconds: Option<i64>) -> SharedString {
    seconds
        .and_then(|seconds| jiff::Timestamp::new(seconds, 0).ok())
        .map(|stamp| release_date_label(&stamp.strftime("%Y-%m-%d").to_string()))
        .unwrap_or_default()
}

pub(crate) fn relative_stamp(seconds: Option<i64>) -> SharedString {
    let Some(stamp) = seconds.and_then(|seconds| jiff::Timestamp::new(seconds, 0).ok()) else {
        return SharedString::default();
    };
    let zone = jiff::tz::TimeZone::system();
    let now = jiff::Timestamp::now();
    let played = stamp.to_zoned(zone.clone());
    let today = now.to_zoned(zone).date();
    let minutes = (now.as_second() - stamp.as_second()) / 60;
    let time = played.strftime("%H:%M").to_string();

    match (minutes, played.date()) {
        (..=0, _) => t!("date-just-now"),
        (1, _) => t!("date-minute-ago"),
        (..=59, _) => t!("date-minutes-ago", count = minutes),
        (_, day) if day == today => t!("date-today", time = &time),
        (_, day) if today.yesterday().is_ok_and(|past| past == day) => {
            t!("date-yesterday", time = &time)
        }
        _ => {
            let date = release_date_label(&played.strftime("%Y-%m-%d").to_string());
            t!("date-time", date = &date, time = &time)
        }
    }
}

pub(crate) fn count(value: u64) -> SharedString {
    let group = t!("number-group");
    let digits = value.to_string();
    let first = match digits.len() % 3 {
        0 => 3,
        remainder => remainder,
    };
    let mut grouped = digits[..first].to_owned();
    for chunk in digits.as_bytes()[first..].chunks(3) {
        grouped.push_str(&group);
        grouped.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    SharedString::from(grouped)
}

pub(crate) fn title<F>(
    cell: &Cell<F>,
    value: impl Into<SharedString>,
    color: Option<Hsla>,
    explicit: bool,
    press: Option<Tap>,
    is_liked: Option<AnyElement>,
) -> AnyElement {
    let text = div()
        .id(("track-title", cell.row))
        .min_w_0()
        .truncate()
        .child(value.into())
        .when_some(press, |this, press| {
            this.cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    press(cx);
                })
        });

    line(cell, color)
        .flex()
        .items_center()
        .gap_1p5()
        .child(text)
        .when(explicit, |this| {
            this.child(div().flex_none().child(ExplicitBadge::new()))
        })
        .when_some(is_liked, |this, is_liked| this.child(is_liked))
        .into_any_element()
}

pub(crate) fn artwork<F>(cell: &Cell<F>, url: Option<String>) -> AnyElement {
    cell.middle()
        .child(Thumb { row: cell.row, url })
        .into_any_element()
}

pub(crate) fn avatar<F>(cell: &Cell<F>, url: Option<String>) -> AnyElement {
    cell.middle().child(Face { url }).into_any_element()
}

fn portrait(added: &Contributor, row: usize) -> Stateful<Div> {
    div()
        .id(("added-face", row))
        .flex_none()
        .cursor_pointer()
        .link(Destination::User(added.id.clone().into()))
        .child(
            Artwork::new(added.avatar.clone())
                .circle()
                .size(FACE)
                .fallback(PERSON),
        )
}

pub(crate) fn added_by<F>(cell: &Cell<F>, added: &Contributor, color: Hsla) -> AnyElement {
    line(cell, Some(color))
        .flex()
        .items_center()
        .gap_2()
        .child(portrait(added, cell.row))
        .child(
            div()
                .id(("added-name", cell.row))
                .min_w_0()
                .truncate()
                .hover(|style| style.underline())
                .link(Destination::User(added.id.clone().into()))
                .child(SharedString::from(added.name.clone())),
        )
        .into_any_element()
}

pub(crate) fn credited<F>(
    cell: &Cell<F>,
    added: Option<&Contributor>,
    value: impl Into<SharedString>,
    color: Hsla,
) -> AnyElement {
    let Some(added) = added else {
        return dim(cell, value, color);
    };

    line(cell, Some(color))
        .flex()
        .items_center()
        .gap_2()
        .child(portrait(added, cell.row))
        .child(div().min_w_0().truncate().child(value.into()))
        .into_any_element()
}

pub(crate) fn blank<F>(cell: &Cell<F>) -> AnyElement {
    cell.frame().into_any_element()
}

pub(crate) fn content_width(window: &Window, inset: Pixels, cx: &App) -> Pixels {
    (Chrome::content(window, cx) - inset).max(ui::MIN_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::count;

    #[test]
    fn groups_count() {
        assert_eq!(count(999), "999");
        assert_eq!(count(1_234), "1,234");
        assert_eq!(count(12_345_678), "12,345,678");
    }
}
