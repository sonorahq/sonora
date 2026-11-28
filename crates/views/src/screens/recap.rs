use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Hsla, Pixels, Render, ScrollHandle, SharedString,
    Window, div, px,
};
use i18n::t;
use state::{Recap, RecapState, TopAlbum, TopArtist, TopGenre, TopSong, Year};
use ui::{
    ActiveTheme as _, Artwork, MenuItem, Picker, Popovers, Scrollbar, Scroller, Text, runtime,
    spoken_heading, vacant,
};

use crate::chrome::Chrome;
use crate::shared::hero::{HeroMetaStrip, PageHero};
use crate::shared::page;

/// The width of a ranking's rank number.
const RANK: Pixels = px(24.);
/// The height of the monthly chart before the month names below it.
const CHART: Pixels = px(120.);
/// The narrowest a summary figure grows before it wraps to the next row.
const STAT_WIDTH: Pixels = px(180.);

/// The yearly recap screen: a year picker and the selected year's rankings, totals and chart.
pub(crate) struct RecapView {
    recap: Entity<Recap>,
    scrollbar: Entity<Scrollbar>,
    popovers: Popovers,
}

impl RecapView {
    pub(crate) fn new(recap: Entity<Recap>, cx: &mut Context<Self>) -> Self {
        let id = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));
        cx.observe(&recap, |_, _, cx| cx.notify()).detach();
        Self {
            recap,
            scrollbar,
            popovers: Popovers::default(),
        }
    }

    /// Reads the play log again, keeping the year on show when it still has plays.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.recap.update(cx, |recap, cx| recap.refresh(cx));
    }

    fn header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (report, years, selected) = {
            let recap = self.recap.read(cx);
            (
                recap.report().cloned(),
                recap.years().to_vec(),
                recap.selected(),
            )
        };
        let mut strip = HeroMetaStrip::new();
        if let Some(report) = &report {
            strip = strip
                .text(t!("recap-plays", count = report.plays))
                .text(runtime(report.total()));
        }
        let picker = self.year_picker(&years, selected, cx);

        PageHero::new("recap-hero", t!("nav-recap"))
            .fallback("icons/sparkles.svg")
            .accent()
            .eyebrow(t!("recap-eyebrow"))
            .meta(strip)
            .actions(picker)
            .into_any_element()
    }

    fn year_picker(
        &mut self,
        years: &[i32],
        selected: Option<i32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if years.is_empty() {
            return div().into_any_element();
        }
        let current = selected.map(|year| year.to_string()).unwrap_or_default();
        let items = years
            .iter()
            .rev()
            .map(|&year| {
                MenuItem::new(("recap-year", year as usize), year.to_string())
                    .selected(selected == Some(year))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.recap.update(cx, |recap, cx| recap.select(year, cx));
                        this.popovers.close();
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .items_center()
            .child(
                Picker::new("recap-year", &self.popovers, current)
                    .name(t!("recap-year"))
                    .width(Picker::NARROW)
                    .items(items),
            )
            .into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let (state, report) = {
            let recap = self.recap.read(cx);
            (recap.state(), recap.report().cloned())
        };
        match state {
            RecapState::Loading => status("recap-loading", t!("recap-loading"), cx),
            RecapState::Failed => status("recap-failed", t!("recap-failed"), cx),
            RecapState::Ready => match report {
                None => status("recap-empty-all", t!("recap-empty-all"), cx),
                Some(report) if report.plays == 0 => {
                    let label = t!("recap-empty", year = report.year);
                    status("recap-empty-year", label, cx)
                }
                Some(report) => self.report(&report, cx),
            },
        }
    }

    fn report(&self, report: &Year, cx: &mut Context<Self>) -> AnyElement {
        let mut sections = div().flex().flex_col().gap_8();
        sections = sections.child(self.summary(report, cx));
        sections = sections.child(self.songs(&report.top_songs, cx));
        sections = sections.child(self.artists(&report.top_artists, cx));
        sections = sections.child(self.albums(&report.top_albums, cx));
        sections = sections.child(self.genres(report, cx));
        sections = sections.child(self.chart(report, cx));
        sections.into_any_element()
    }

    fn summary(&self, report: &Year, cx: &App) -> AnyElement {
        let plays = report.plays.to_string();
        div()
            .flex()
            .flex_wrap()
            .gap_3()
            .child(stat(
                "recap-stat-time",
                t!("recap-total-time"),
                runtime(report.total()),
                cx,
            ))
            .child(stat(
                "recap-stat-plays",
                t!("recap-plays-title"),
                plays.into(),
                cx,
            ))
            .into_any_element()
    }

    fn songs(&self, songs: &[TopSong], cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let total = songs.len();
        let rows = songs.iter().enumerate().map(|(index, song)| {
            let plays = t!("recap-plays", count = song.plays);
            let label = t!(
                "recap-song-label",
                rank = index + 1,
                name = &song.name,
                artists = &song.artists,
                plays = &plays,
            );
            div()
                .id(row_id("recap-top-song", &song.id))
                .role(Role::ListItem)
                .aria_label(label)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(total)
                .flex()
                .items_center()
                .gap_3()
                .py_2()
                .child(rank(index, theme.muted_foreground))
                .child(Artwork::new(song.cover.clone()).size(theme.metrics.thumb))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(song.name.clone())
                        .child(
                            div()
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground)
                                .child(song.artists.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(plays),
                )
        });
        let list = div()
            .id("recap-top-songs-list")
            .role(Role::List)
            .aria_label(t!("recap-top-songs"))
            .flex()
            .flex_col()
            .children(rows);
        section("recap-top-songs-heading", t!("recap-top-songs"), list, cx)
    }

    fn artists(&self, artists: &[TopArtist], cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let total = artists.len();
        let rows = artists.iter().enumerate().map(|(index, artist)| {
            let plays = t!("recap-plays", count = artist.plays);
            let label = t!(
                "recap-artist-label",
                rank = index + 1,
                name = &artist.name,
                plays = &plays,
            );
            div()
                .id(row_id(
                    "recap-top-artist",
                    artist.id.as_deref().unwrap_or(artist.name.as_str()),
                ))
                .role(Role::ListItem)
                .aria_label(label)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(total)
                .flex()
                .items_center()
                .gap_3()
                .py_2()
                .child(rank(index, theme.muted_foreground))
                .child(div().flex_1().min_w_0().child(artist.name.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(plays),
                )
        });
        let list = div()
            .id("recap-top-artists-list")
            .role(Role::List)
            .aria_label(t!("recap-top-artists"))
            .flex()
            .flex_col()
            .children(rows);
        section(
            "recap-top-artists-heading",
            t!("recap-top-artists"),
            list,
            cx,
        )
    }

    fn albums(&self, albums: &[TopAlbum], cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let total = albums.len();
        let rows = albums.iter().enumerate().map(|(index, album)| {
            let plays = t!("recap-plays", count = album.plays);
            let label = t!(
                "recap-album-label",
                rank = index + 1,
                name = &album.name,
                artists = &album.artists,
                plays = &plays,
            );
            div()
                .id(row_id(
                    "recap-top-album",
                    album.id.as_deref().unwrap_or(album.name.as_str()),
                ))
                .role(Role::ListItem)
                .aria_label(label)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(total)
                .flex()
                .items_center()
                .gap_3()
                .py_2()
                .child(rank(index, theme.muted_foreground))
                .child(Artwork::new(album.cover.clone()).size(theme.metrics.thumb))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(album.name.clone())
                        .child(
                            div()
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground)
                                .child(album.artists.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(plays),
                )
        });
        let list = div()
            .id("recap-top-albums-list")
            .role(Role::List)
            .aria_label(t!("recap-top-albums"))
            .flex()
            .flex_col()
            .children(rows);
        section("recap-top-albums-heading", t!("recap-top-albums"), list, cx)
    }

    fn genres(&self, report: &Year, cx: &App) -> AnyElement {
        let Some(genres) = &report.top_genres else {
            return section(
                "recap-top-genres-heading",
                t!("recap-top-genres"),
                status(
                    "recap-genres-unavailable",
                    t!("recap-genres-unavailable"),
                    cx,
                ),
                cx,
            );
        };
        let theme = *cx.theme();
        let total = genres.len();
        let rows = genres
            .iter()
            .enumerate()
            .map(|(index, genre)| genre_row(index, genre, total, &theme));
        let list = div()
            .id("recap-top-genres-list")
            .role(Role::List)
            .aria_label(t!("recap-top-genres"))
            .flex()
            .flex_col()
            .children(rows);
        section("recap-top-genres-heading", t!("recap-top-genres"), list, cx)
    }

    fn chart(&self, report: &Year, cx: &App) -> AnyElement {
        let theme = *cx.theme();
        let most = report.months.iter().copied().max().unwrap_or(0).max(1);
        let bars = (0..12).map(|index| {
            let month = month_name(index);
            let time = runtime(report.month(index));
            let label = t!("recap-month-label", month = &month, time = &time);
            let height = px(CHART.as_f32() * (report.months[index] as f32 / most as f32));
            div()
                .id(("recap-month", index))
                .role(Role::Image)
                .aria_label(label)
                .flex()
                .flex_1()
                .flex_col()
                .items_center()
                .justify_end()
                .gap_1()
                .h_full()
                .child(
                    div()
                        .w_full()
                        .h(height)
                        .rounded(theme.radius)
                        .bg(theme.primary),
                )
                .child(
                    div()
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground)
                        .child(month),
                )
        });
        let chart = div()
            .id("recap-months")
            .role(Role::Group)
            .aria_label(t!("recap-chart"))
            .flex()
            .items_end()
            .gap_1()
            .h(CHART + theme.text(Text::Small) + theme.metrics.pad)
            .children(bars);
        section("recap-chart-heading", t!("recap-chart"), chart, cx)
    }
}

impl Render for RecapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inset = cx.theme().metrics.inset;
        let page = Scroller::new("recap-page", &self.scrollbar)
            .bottom_inset(Chrome::track_bottom(cx))
            .top_inset(page::below_head(window, cx))
            .px(Chrome::margin(window, cx))
            .pt(inset)
            .pb(inset)
            .child(div().px(inset).child(self.header(cx)))
            .child(div().px(inset).pt_6().child(self.body(cx)));

        div().size_full().child(page)
    }
}

/// A rank number for a ranking row.
fn rank(index: usize, color: Hsla) -> AnyElement {
    div()
        .flex_none()
        .w(RANK)
        .text_color(color)
        .child(format!("{}", index + 1))
        .into_any_element()
}

/// One genre row of the ranking.
fn genre_row(index: usize, genre: &TopGenre, total: usize, theme: &ui::Theme) -> AnyElement {
    let plays = t!("recap-plays", count = genre.plays);
    let label = t!(
        "recap-genre-label",
        rank = index + 1,
        name = &genre.name,
        plays = &plays,
    );
    div()
        .id(row_id("recap-top-genre", &genre.name))
        .role(Role::ListItem)
        .aria_label(label)
        .aria_position_in_set(index + 1)
        .aria_size_of_set(total)
        .flex()
        .items_center()
        .gap_3()
        .py_2()
        .child(rank(index, theme.muted_foreground))
        .child(div().flex_1().min_w_0().child(genre.name.clone()))
        .child(
            div()
                .flex_none()
                .text_color(theme.muted_foreground)
                .child(plays),
        )
        .into_any_element()
}

/// A titled section: a heading screen readers reach, then its list or chart.
fn section(id: &'static str, title: SharedString, body: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(spoken_heading(id, title, 2, cx))
        .child(body)
        .into_any_element()
}

/// A summary figure, named with its label and its value for screen readers.
fn stat(id: &'static str, label: SharedString, value: SharedString, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    let spoken = t!("recap-stat", label = &label, value = &value);
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(spoken)
        .flex()
        .flex_col()
        .gap_1()
        .min_w(STAT_WIDTH)
        .p_4()
        .rounded(theme.radius)
        .bg(theme.secondary.opacity(0.45))
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_size(theme.text(Text::Title))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
        .into_any_element()
}

/// A status line for a recap that is loading, failed or has nothing to show.
fn status(id: &'static str, label: SharedString, cx: &App) -> AnyElement {
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(label.clone())
        .child(vacant(label, cx))
        .into_any_element()
}

/// A stable id for a ranking row, unique within its list.
fn row_id(kind: &str, id: &str) -> gpui::ElementId {
    gpui::ElementId::Name(format!("{kind}-{id}").into())
}

/// The name of a month, January being `0`.
fn month_name(index: usize) -> SharedString {
    match index {
        0 => t!("month-1"),
        1 => t!("month-2"),
        2 => t!("month-3"),
        3 => t!("month-4"),
        4 => t!("month-5"),
        5 => t!("month-6"),
        6 => t!("month-7"),
        7 => t!("month-8"),
        8 => t!("month-9"),
        9 => t!("month-10"),
        10 => t!("month-11"),
        _ => t!("month-12"),
    }
}
