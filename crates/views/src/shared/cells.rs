//! What a table row draws in its cells: the transport at its head, a cover, a title with its
//! marks, links to artists, albums and people, dates, counts and lengths. Each takes the frame
//! [`frame`] gives its cell, so every cell of a row shares the row's hover.

use std::rc::Rc;
use std::time::Duration;

use i18n::t;
use music::{ArtistRef, Contributor};
use router::Destination;
use rune::{Cursor, El, TextStyle, Tx, icon, row, text};
use ui::{
    Artwork, Avatar, Cover, ExplicitBadge, Hsla, InlineLink, InlineLinks, TABULAR, TableCell,
    TextAlign, Theme, Weight, clock,
};

use crate::Ui;
use crate::shared::hero::release_date_label;
use crate::shared::page::rems;

const PLAY: &str = "icons/play.svg";
const PLAYING: &str = "icons/music-2.svg";
const PAUSE: &str = "icons/pause.svg";
const UNAVAILABLE: &str = "icons/play-off.svg";
const PERSON: &str = "icons/user.svg";

/// The width of the number column at the head of a list.
pub(crate) const NUMBER: f32 = 44.;
/// The width of a trailing column of short values, such as a count.
pub(crate) const TRAILING: f32 = 72.;
/// The width of a date column.
pub(crate) const DATE: f32 = 112.;
/// The width of a year column.
pub(crate) const YEAR: f32 = 64.;
/// The side of the square the transport at a row's head answers in.
pub(crate) const HIT: f32 = 18.;
/// The side of a contributor's face beside their name.
pub(crate) const FACE: f32 = 20.;
/// The padding a table puts inside each cell, which a cell's frame reaches back over so the
/// pointer never leaves the row's hover between two cells.
const CELL_PAD: f32 = 8.;
/// The space between a title and the marks after it, in rems.
const MARK_GAP: f32 = 0.375;
/// The space between a face and the name beside it, in rems.
const FACE_GAP: f32 = 0.5;

/// A handler a cell runs when pressed.
pub(crate) type Act = Rc<dyn Fn(&mut Ui, &mut Tx)>;

/// How tall a row's transport glyph stands, from the row height.
pub(crate) fn glyph(theme: &Theme) -> f32 {
    (theme.metrics.row * 0.23).round()
}

/// The frame one cell's content sits in: the whole cell, aligned as its column is, and part of
/// the row's hover group `group`, which every cell of the row shares. Pass the row's key scoped
/// by the table, such as `format!("{table}/{row}")`.
pub(crate) fn frame<F>(cell: &TableCell<F>, group: &str) -> El {
    let frame = row()
        .w(cell.width + CELL_PAD * 2.)
        .h_full()
        .mx(-CELL_PAD)
        .px(CELL_PAD)
        .center_y()
        .min_w(0.)
        .hover_group_shared(group);
    match cell.align {
        TextAlign::Left => frame,
        TextAlign::Center => frame.justify_center(),
        TextAlign::Right => frame.justify_end(),
    }
}

/// A line of body text in `color` that ends in an ellipsis when it runs out of room.
fn line(value: impl Into<String>, color: Hsla, theme: &Theme) -> El {
    text(value)
        .font(TextStyle::new(theme.text(ui::Text::Body), Weight::NORMAL))
        .color(color.color())
        .truncate()
        .shrink()
        .min_w(0.)
}

/// What the transport at the head of a row shows while the pointer is on the row, and what
/// pressing it does. Without a press the glyph says the row cannot play.
pub(crate) struct Transport {
    pub(crate) icon: &'static str,
    pub(crate) color: Hsla,
    pub(crate) press: Option<Act>,
    pub(crate) label: String,
}

/// The transport cell at the head of a row. `number` is the label the row rests at. `playing`
/// is the row's `Playback::control`, `None` for a row that is not the current one.
pub(crate) fn index(
    frame: El,
    playing: Option<bool>,
    playable: bool,
    number: String,
    press: Option<Act>,
    theme: &Theme,
) -> El {
    let faded = theme.muted_foreground.opacity(0.5);
    let resting = match playing {
        Some(true) => icon(icons::get(PLAYING), glyph(theme)).color(theme.foreground.color()),
        Some(false) => icon(icons::get(PAUSE), glyph(theme)).color(theme.muted_foreground.color()),
        None => text(number)
            .font(TextStyle::new(theme.text(ui::Text::Body), Weight::NORMAL))
            .color(match playable {
                true => theme.muted_foreground.color(),
                false => faded.color(),
            }),
    };
    let icon = match (playing == Some(true), playable) {
        (true, _) => PAUSE,
        (false, true) => PLAY,
        (false, false) => UNAVAILABLE,
    };
    let color = match playable {
        true => theme.foreground,
        false => theme.muted_foreground,
    };
    let label = match playing {
        Some(true) => t!("play-pause"),
        Some(false) => t!("play-resume"),
        None => t!("common-play"),
    };
    transport(
        frame,
        resting.key("resting").a11y_hidden(),
        Transport {
            icon,
            color,
            press,
            label: label.into_owned(),
        },
        theme,
    )
}

/// A cell that rests at `resting` and shows the transport in its place while the pointer is on
/// the row.
pub(crate) fn transport(frame: El, resting: El, hover: Transport, theme: &Theme) -> El {
    let Transport {
        icon: glyph_path,
        color,
        press,
        label,
    } = hover;
    let mut button = row()
        .key("transport")
        .role("button")
        .label(label)
        .pin(Some(0.), Some(0.), Some(0.), Some(0.))
        .center()
        .group_opacity(0., 1.)
        .child(icon(icons::get(glyph_path), glyph(theme)).color(color.color()));
    button = match press {
        Some(press) => button
            .focusable(true)
            .cursor(Cursor::Pointer)
            .press_scale(ui::PRESS_SCALE)
            .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| press(ui, tx)),
        None => button.cursor(Cursor::NotAllowed).disabled(true),
    };
    frame.child(
        row()
            .key("transport-hit")
            .min_w(HIT)
            .h(HIT)
            .center()
            .child(resting.group_opacity(1., 0.))
            .child(button),
    )
}

/// A value that opens `to` when pressed, underlined under the pointer.
pub(crate) fn link(
    frame: El,
    key: &str,
    value: impl Into<String>,
    color: Hsla,
    to: Destination,
    theme: &Theme,
) -> El {
    let value = value.into();
    frame.child(
        InlineLinks::new(
            key,
            [InlineLink::new(value.clone(), Some(String::new()))],
            value,
            color,
        )
        .truncate()
        .on_link(move |ui: &mut Ui, _: &str, _: &mut Tx| ui.go(to.clone()))
        .build(theme),
    )
}

/// A track's or a release's artists as comma-separated links to their pages, `fallback` when
/// the provider named none apart.
pub(crate) fn artist_links(
    key: impl Into<String>,
    artists: &[ArtistRef],
    fallback: impl Into<String>,
    color: Hsla,
) -> InlineLinks<Ui> {
    InlineLinks::new(
        key,
        artists
            .iter()
            .map(|artist| InlineLink::new(artist.name.clone(), artist.id.clone())),
        fallback,
        color,
    )
    .on_link(|ui: &mut Ui, id: &str, _: &mut Tx| ui.go(Destination::Artist(id.into())))
}

/// The artists cell.
pub(crate) fn artists(
    frame: El,
    artists: &[ArtistRef],
    fallback: impl Into<String>,
    color: Hsla,
    theme: &Theme,
) -> El {
    frame.child(
        artist_links("artist", artists, fallback, color)
            .truncate()
            .build(theme),
    )
}

/// A line of plain text in `muted`, which screen readers reach by its value under `key`.
pub(crate) fn dim(
    frame: El,
    key: &str,
    value: impl Into<String>,
    muted: Hsla,
    theme: &Theme,
) -> El {
    let value = value.into();
    frame.child(said(line(value.clone(), muted, theme), key, &value))
}

/// A track length in tabular digits, so the column reads as a monospace strip.
pub(crate) fn length(frame: El, value: Duration, muted: Hsla, theme: &Theme) -> El {
    let style = TextStyle::new(theme.text(ui::Text::Body), Weight::NORMAL).features(TABULAR);
    let value = clock(value).into_owned();
    frame.child(said(
        text(value.clone())
            .font(style)
            .color(muted.color())
            .truncate(),
        "length",
        &value,
    ))
}

/// Keys `line` and exposes its text to screen readers. Empty text stays out of the tree.
fn said(line: El, key: &str, value: &str) -> El {
    match value.is_empty() {
        true => line.a11y_hidden(),
        false => line
            .key(key.to_owned())
            .role("text")
            .label(value.to_owned()),
    }
}

/// A date as the app writes one, from Unix seconds; empty without one.
pub(crate) fn stamp(seconds: Option<i64>) -> String {
    seconds
        .and_then(|seconds| jiff::Timestamp::new(seconds, 0).ok())
        .map(|stamp| release_date_label(&stamp.strftime("%Y-%m-%d").to_string()))
        .unwrap_or_default()
}

/// When something happened, as a listener says it: just now, minutes ago, today or yesterday
/// at a time, or a date and a time.
pub(crate) fn relative_stamp(seconds: Option<i64>) -> String {
    let Some(stamp) = seconds.and_then(|seconds| jiff::Timestamp::new(seconds, 0).ok()) else {
        return String::new();
    };
    let zone = jiff::tz::TimeZone::system();
    let now = jiff::Timestamp::now();
    let played = stamp.to_zoned(zone.clone());
    let today = now.to_zoned(zone).date();
    let minutes = (now.as_second() - stamp.as_second()) / 60;
    let time = played.strftime("%H:%M").to_string();

    let said = match (minutes, played.date()) {
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
    };
    said.into_owned()
}

/// A count with its thousands grouped as the language groups them.
pub(crate) fn count(value: u64) -> String {
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
    grouped
}

/// What a title cell shows: the name, in `color` (the foreground when `None`), the explicit
/// badge, a status mark such as a download's ring, and the heart. With `press` the name is a
/// link.
pub(crate) struct Title {
    pub(crate) name: String,
    pub(crate) color: Option<Hsla>,
    pub(crate) explicit: bool,
    pub(crate) status: Option<El>,
    pub(crate) liked: Option<El>,
    pub(crate) press: Option<Act>,
}

/// The title cell.
pub(crate) fn title(frame: El, title: Title, theme: &Theme) -> El {
    let Title {
        name,
        color,
        explicit,
        status,
        liked,
        press,
    } = title;
    let mut label = line(name.clone(), color.unwrap_or(theme.foreground), theme)
        .key("track-title")
        .label(name);
    label = match press {
        Some(press) => label
            .role("link")
            .focusable(true)
            .cursor(Cursor::Pointer)
            .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| press(ui, tx)),
        None => label.role("text"),
    };
    let badge = explicit.then(|| {
        row()
            .key("explicit-badge")
            .role("image")
            .label(t!("song-explicit").into_owned())
            .child(ExplicitBadge::new().build(theme))
    });
    frame
        .gap(rems(theme, MARK_GAP))
        .child(label)
        .children(badge)
        .children(status)
        .children(liked)
}

/// A cover thumbnail, as the cover column draws it.
pub(crate) fn artwork(frame: El, artwork: &Artwork, url: Option<&str>, theme: &Theme) -> El {
    frame.child(
        Cover::new(artwork, url)
            .size(theme.metrics.thumb)
            .build(theme)
            .key("artwork"),
    )
}

/// A round face thumbnail, for a person or an artist.
pub(crate) fn avatar(frame: El, artwork: &Artwork, url: Option<&str>, theme: &Theme) -> El {
    frame.child(
        Avatar::new(artwork, url)
            .size(theme.metrics.thumb)
            .build(theme)
            .key("avatar"),
    )
}

/// The face of whoever added a song, which opens their page.
fn portrait(added: &Contributor, artwork: &Artwork, theme: &Theme) -> El {
    let to = Destination::User(added.id.as_str().into());
    row()
        .key("added-face")
        .role("link")
        .label(added.name.clone())
        .focusable(true)
        .cursor(Cursor::Pointer)
        .press_scale(ui::PRESS_SCALE)
        .on_press(move |ui: &mut Ui| ui.go(to.clone()))
        .child(
            Cover::new(artwork, added.avatar.as_deref())
                .circle()
                .size(FACE)
                .fallback(PERSON)
                .build(theme),
        )
}

/// Who added a song: their face and their name, both opening their page.
pub(crate) fn added_by(
    frame: El,
    added: &Contributor,
    color: Hsla,
    artwork: &Artwork,
    theme: &Theme,
) -> El {
    let to = Destination::User(added.id.as_str().into());
    let name = InlineLinks::new(
        "added-name",
        [InlineLink::new(added.name.clone(), Some(added.id.clone()))],
        added.name.clone(),
        color,
    )
    .truncate()
    .on_link(move |ui: &mut Ui, _: &str, _: &mut Tx| ui.go(to.clone()))
    .build(theme);
    frame
        .gap(rems(theme, FACE_GAP))
        .child(portrait(added, artwork, theme))
        .child(name)
}

/// A date with the face of whoever added the song before it, when someone did.
pub(crate) fn credited(
    frame: El,
    added: Option<&Contributor>,
    value: impl Into<String>,
    color: Hsla,
    artwork: &Artwork,
    theme: &Theme,
) -> El {
    let Some(added) = added else {
        return dim(frame, "credited", value, color, theme);
    };
    let value = value.into();
    frame
        .gap(rems(theme, FACE_GAP))
        .child(portrait(added, artwork, theme))
        .child(said(line(value.clone(), color, theme), "credited", &value))
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
