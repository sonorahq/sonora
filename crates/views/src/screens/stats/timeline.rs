use gpui::prelude::*;
use gpui::{AnyElement, App, Context, FontWeight, SharedString, div, relative};
use i18n::t;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Unit};
use music::StatsPeriod;
use state::StatsSource;
use ui::{ActiveTheme as _, Button, InfoCard, RangeScrubber, TabBar, Text};

use super::insights::{bar, floor};
use super::{BAR_OFF, BAR_ON, StatsView, stats_tab};

const EPOCH: Date = Date::constant(1970, 1, 1);
/// Calendar presets in days, longest first.
const PRESETS: [(&str, i64); 7] = [
    ("stats-five-years", 1826),
    ("stats-two-years", 730),
    ("stats-year", 365),
    ("stats-six-months", 182),
    ("stats-quarter", 91),
    ("stats-month", 30),
    ("stats-week", 7),
];
/// How far the scrubber's track is inset from its ends, as a multiple of the theme pad.
const TRACK_INSET: f32 = 0.75;

impl StatsView {
    /// The shared date controls, with an activity scrubber when the source has history bounds.
    pub(super) fn timeline(
        &self,
        periods: &[StatsPeriod],
        period: StatsPeriod,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let stats = self.stats.read(cx);
        let (source, history, dates) = (stats.source(), stats.history_bounds(), stats.dates());
        let label = ui::heading(self.window_label(cx), cx);
        let date_row = source.supports_dates().then(|| {
            let days = dates
                .or(history)
                .map(|(from, to)| t!("count-days", count = day_number(to) - day_number(from) + 1));
            self.date_row(days, cx)
        });
        let calendar = date_row.as_ref().and_then(|_| self.calendar(cx));
        let activity = history
            .map(|bounds| self.activity_range(bounds, dates, cx))
            .unwrap_or_default();
        let presets = self.presets(source, periods, period, dates, cx);
        let overview = &self.stats.read(cx).data().overview;
        let hours = overview
            .time
            .filter(|secs| *secs > 0)
            .map(|secs| match secs >= 3600 {
                true => t!("stats-badge-hours", count = secs / 3600),
                false => t!("stats-badge-minutes", count = secs / 60),
            });
        let held = [
            overview.plays_per_day.map(|plays| {
                t!(
                    "stats-badge-per-day",
                    count = plays.round().max(1.) as usize
                )
            }),
            hours,
            overview
                .replay_share
                .map(|share| t!("stats-badge-replays", share = share)),
            overview
                .change
                .map(|change| t!("stats-badge-change", change = format!("{change:+}"))),
        ];
        let badges = self.badges(held.map(|text| text.map(|text| (text, false))), cx);
        InfoCard::new(t!("stats-period-title"))
            .spoken("stats-period")
            .child(label)
            .children(date_row)
            .children(calendar)
            .children(activity)
            .child(presets)
            .when_some(badges, |card, badges| card.trailing(badges))
            .into_any_element()
    }

    /// Labels the selected dates or provider preset.
    pub(super) fn window_label(&self, cx: &App) -> SharedString {
        let stats = self.stats.read(cx);
        match (stats.source(), stats.dates()) {
            (_, Some((from, to))) => t!(
                "stats-selected-dates",
                from = date_label(from),
                to = date_label(to)
            ),
            (StatsSource::Provider, None) => i18n::lookup(period_key(stats.period()), None),
            (_, None) => t!("stats-all-time"),
        }
    }

    /// Keeps the date fields on the selected range, or on all the history, once either changes.
    pub(super) fn sync_dates(&mut self, cx: &mut Context<Self>) {
        let stats = self.stats.read(cx);
        let Some((from, to)) = stats.dates().or_else(|| stats.history_bounds()) else {
            return;
        };
        for (field, seconds) in [(&self.start, from), (&self.end, to)] {
            let label = date_label(seconds);
            field.update(cx, |input, cx| {
                if input.text() != label {
                    input.set_text(label, cx);
                }
            });
        }
    }

    /// The scrubber and annotations over the source's known listening history.
    fn activity_range(
        &self,
        bounds: (i64, i64),
        dates: Option<(i64, i64)>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = *cx.theme();
        let low = day_number(bounds.0);
        let width = (day_number(bounds.1).max(low + 1) - low) as f32;
        let share = |seconds| ((day_number(seconds) - low) as f32 / width).clamp(0., 1.);
        let value = dates.map_or((0., 1.), |(from, to)| (share(from), share(to)));
        let inset = theme.metrics.pad * TRACK_INSET;
        let place = |date: Date| ((date_number(date) - low) as f32 / width).clamp(0., 1.);
        let (busiest, streak) = {
            let stats = self.stats.read(cx);
            let history = stats.history();
            let busiest = history
                .and_then(|history| history.busiest_day)
                .map(|(day, plays)| {
                    (
                        place(day),
                        t!("stats-mark-busiest", day = day.to_string(), count = plays),
                    )
                });
            let streak = history
                .and_then(|history| history.streak)
                .map(|(from, to)| {
                    let days = to
                        .since((Unit::Day, from))
                        .map_or(1, |span| span.get_days() + 1);
                    (
                        (place(from), place(to)),
                        t!("stats-mark-streak", count = days),
                    )
                });
            (busiest, streak)
        };
        let mut parts = vec![
            div()
                .flex()
                .flex_col()
                .children(
                    (!self.stats.read(cx).density().is_empty())
                        .then(|| self.brush(value, cx).px(inset)),
                )
                .child(
                    RangeScrubber::new(&self.range, value)
                        .name(t!("stats-period-title"))
                        .values(
                            date_label(dates.map_or(bounds.0, |dates| dates.0)),
                            date_label(dates.map_or(bounds.1, |dates| dates.1)),
                        )
                        .colors(theme.primary, theme.secondary, theme.foreground)
                        .on_change(cx.listener(move |this, &(a, b): &(f32, f32), _, cx| {
                            let day = |share: f32| low + (share * width).round() as i64;
                            if let Some(dates) = span(day(a), day(b)) {
                                this.set_dates(Some(dates), cx);
                            }
                        })),
                )
                .into_any_element(),
        ];
        parts.extend(
            self.annotations(busiest, streak, cx)
                .map(|lane| lane.px(inset).into_any_element()),
        );
        parts
    }

    /// Highlights the selected range in the whole-history histogram.
    fn brush(&self, selection: (f32, f32), cx: &App) -> gpui::Div {
        let theme = *cx.theme();
        let density = self.stats.read(cx).density();
        let most = density.iter().copied().max().unwrap_or_default().max(1);
        floor(&theme).children(density.iter().enumerate().map(|(index, count)| {
            let at = (index as f32 + 0.5) / density.len() as f32;
            let covered = (selection.0..=selection.1).contains(&at);
            bar(
                &theme,
                *count as f32 / most as f32,
                match covered {
                    true => theme.primary.opacity(BAR_ON),
                    false => theme.muted_foreground.opacity(BAR_OFF),
                },
            )
        }))
    }

    /// Anchors the busiest day and longest streak to the whole history.
    fn annotations(
        &self,
        busiest: Option<(f32, SharedString)>,
        streak: Option<((f32, f32), SharedString)>,
        cx: &App,
    ) -> Option<gpui::Div> {
        let theme = *cx.theme();
        if busiest.is_none() && streak.is_none() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(theme.metrics.pad)
                .children(streak.map(|((from, to), text)| {
                    div().relative().h(theme.metrics.pad * 2.5).child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(from))
                            .w(relative(to - from))
                            .min_w(theme.metrics.cover * 1.5)
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded_full()
                            .bg(theme.primary)
                            .text_size(theme.text(Text::Small))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.primary_foreground)
                            .whitespace_nowrap()
                            .child(text),
                    )
                }))
                .children(busiest.map(|(at, text)| {
                    let label = div()
                        .absolute()
                        .top(theme.metrics.pad * 2.)
                        .whitespace_nowrap()
                        .child(ui::faint(cx).child(text));
                    div()
                        .relative()
                        .h(theme.metrics.pad * 4.5)
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left(relative(at))
                                .w(theme.metrics.pad / 8.)
                                .h(theme.metrics.pad * 1.5)
                                .rounded_full()
                                .bg(theme.foreground),
                        )
                        .child(match at > 0.5 {
                            true => label.right(relative(1. - at)),
                            false => label.left(relative(at)),
                        })
                })),
        )
    }

    fn presets(
        &self,
        source: StatsSource,
        periods: &[StatsPeriod],
        period: StatsPeriod,
        dates: Option<(i64, i64)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tabs = TabBar::new("stats-presets").name(t!("stats-period-title"));
        if !source.supports_dates() {
            return tabs
                .items(
                    periods
                        .iter()
                        .rev()
                        .copied()
                        .enumerate()
                        .map(|(index, window)| {
                            stats_tab(
                                ("stats-window", index),
                                i18n::lookup(period_key(window), None),
                                dates.is_none() && window == period,
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select(source, window, cx)),
                            )
                        }),
                )
                .into_any_element();
        }
        let today = day_number(Timestamp::now().as_second());
        let all = stats_tab("stats-preset-all", t!("stats-all-time"), dates.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.set_dates(None, cx)));
        let recent = PRESETS.into_iter().map(|(key, days)| {
            let first = today - (days - 1);
            let chosen = dates
                .is_some_and(|(from, to)| day_number(from) == first && day_number(to) == today);
            stats_tab(
                ("stats-preset", days as usize),
                i18n::lookup(key, None),
                chosen,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_dates(span(first, today), cx);
            }))
        });
        tabs.items(std::iter::once(all).chain(recent))
            .into_any_element()
    }

    /// Applies typed endpoints once they form a valid range.
    fn date_row(&self, note: Option<SharedString>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let field = |input: &gpui::Entity<ui::Input>, end| {
            let date = input.read(cx).text().parse::<Date>().ok();
            let label = match end {
                true => t!("stats-pick-end"),
                false => t!("stats-pick-start"),
            };
            div()
                .flex()
                .items_center()
                .child(div().w(theme.metrics.cover).child(input.clone()))
                .child(
                    Button::new(("stats-calendar", usize::from(end)))
                        .name(label)
                        .icon("icons/calendar-days.svg")
                        .small()
                        .ghost()
                        .selected(self.calendar.is_some_and(|(open, _)| open == end))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.calendar = match this.calendar {
                                Some((open, _)) if open == end => None,
                                _ => {
                                    let date = date.unwrap_or_else(|| jiff::Zoned::now().date());
                                    date.with().day(1).build().ok().map(|month| (end, month))
                                }
                            };
                            cx.notify();
                        })),
                )
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(theme.metrics.pad)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.metrics.pad / 2.)
                    .child(field(&self.start, false))
                    .child(ui::faint(cx).child("–"))
                    .child(field(&self.end, true)),
            )
            .children(note.map(|note| ui::faint(cx).child(note)))
            .into_any_element()
    }

    /// Selects a day for one endpoint, with month and year navigation.
    fn calendar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (end, month) = self.calendar?;
        let move_by = |(index, months, icon, name)| {
            Button::new(("stats-calendar-step", index as usize))
                .icon(icon)
                .name(name)
                .small()
                .ghost()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Ok(next) = month.checked_add(Span::new().months(months)) {
                        this.calendar = Some((end, next));
                        cx.notify();
                    }
                }))
        };
        let month_name = i18n::lookup(&format!("month-{}", month.month()), None);
        let heading = t!("date-month-year", month = &month_name, year = month.year());
        let back = [
            (0, -12, "icons/skip-back.svg", t!("stats-prev-year")),
            (1, -1, "icons/chevron-left.svg", t!("stats-prev-month")),
        ];
        let forward = [
            (2, 1, "icons/chevron-right.svg", t!("stats-next-month")),
            (3, 12, "icons/skip-forward.svg", t!("stats-next-year")),
        ];
        let offset = month.weekday().to_monday_zero_offset() as usize;
        let range = self
            .stats
            .read(cx)
            .dates()
            .or_else(|| self.stats.read(cx).history_bounds())
            .unwrap_or_else(|| {
                let now = Timestamp::now().as_second();
                (now, now)
            });
        let selected = local_date(if end { range.1 } else { range.0 });
        let column = cx.theme().metrics.cover / 3.;
        let weekdays = (0..7).map(|day| {
            div()
                .w(column)
                .child(ui::faint(cx).child(t!("stats-weekday", day = day)))
        });
        let cells = (0..offset + month.days_in_month() as usize).map(|index| {
            let date = index
                .checked_sub(offset)
                .and_then(|day| month.with().day(day as i8 + 1).build().ok());
            div().w(column).children(date.map(|date| {
                Button::new(("stats-calendar-day", date.day() as usize))
                    .name(date.to_string())
                    .label(date.day().to_string())
                    .small()
                    .ghost()
                    .selected(Some(date) == selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let day = date_number(date);
                        let (first, last) = match end {
                            true => (day_number(range.0).min(day), day),
                            false => (day, day_number(range.1).max(day)),
                        };
                        this.calendar = None;
                        if let Some(dates) = span(first, last) {
                            this.set_dates(Some(dates), cx);
                        }
                    }))
            }))
        });
        Some(
            div()
                .flex()
                .flex_col()
                .w(column * 7.)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .children(back.map(move_by))
                        .child(ui::faint(cx).child(heading))
                        .children(forward.map(move_by)),
                )
                .child(div().flex().children(weekdays))
                .child(div().flex().flex_wrap().children(cells))
                .into_any_element(),
        )
    }

    /// Applies a range of dates, where one covering the whole history counts as all of it.
    fn set_dates(&mut self, dates: Option<(i64, i64)>, cx: &mut Context<Self>) {
        self.rewind();
        let bounds = self.stats.read(cx).history_bounds();
        let whole = |(from, to): (i64, i64)| {
            bounds.is_some_and(|bounds| {
                day_number(from) <= day_number(bounds.0) && day_number(to) >= day_number(bounds.1)
            })
        };
        let dates = dates.filter(|dates| !whole(*dates));
        self.stats
            .update(cx, |stats, cx| stats.set_dates(dates, cx));
        cx.notify();
    }

    /// Validates both endpoints and skips the load for an unchanged range.
    pub(super) fn apply_dates(&mut self, cx: &mut Context<Self>) {
        let typed = parse_date(self.start.read(cx).text(), false)
            .zip(parse_date(self.end.read(cx).text(), true))
            .filter(|(from, to)| from <= to);
        let invalid = typed.is_none();
        if invalid != self.invalid {
            self.invalid = invalid;
            for field in [&self.start, &self.end] {
                field.update(cx, |input, cx| input.set_invalid(invalid, cx));
            }
        }
        let current = {
            let stats = self.stats.read(cx);
            stats.dates().or_else(|| stats.history_bounds())
        };
        let same = |(from, to): (i64, i64)| {
            current.is_some_and(|(now_from, now_to)| {
                day_number(from) == day_number(now_from) && day_number(to) == day_number(now_to)
            })
        };
        if let Some(dates) = typed.filter(|dates| !same(*dates)) {
            self.set_dates(Some(dates), cx);
        }
    }
}

fn period_key(period: StatsPeriod) -> &'static str {
    match period {
        StatsPeriod::Weeks => "stats-four-weeks",
        StatsPeriod::Months => "stats-six-months",
        StatsPeriod::Year => "stats-long-term",
    }
}

/// A typed local date as the first second of its day, or as the last when it ends a range.
fn parse_date(value: &str, end: bool) -> Option<i64> {
    let date: Date = value.trim().parse().ok()?;
    match end {
        true => end_of(date),
        false => start_of(date),
    }
}

pub(super) fn local_date(seconds: i64) -> Option<Date> {
    Some(
        Timestamp::from_second(seconds)
            .ok()?
            .to_zoned(TimeZone::system())
            .date(),
    )
}

pub(super) fn date_label(seconds: i64) -> String {
    local_date(seconds)
        .map(|date| date.to_string())
        .unwrap_or_default()
}

/// Resolves local midnight with the offset in effect on that date.
fn start_of(date: Date) -> Option<i64> {
    let midnight = date.at(0, 0, 0, 0).to_zoned(TimeZone::system()).ok()?;
    Some(midnight.timestamp().as_second())
}

fn end_of(date: Date) -> Option<i64> {
    Some(start_of(date.tomorrow().ok()?)? - 1)
}

/// The whole days from the first day to the last, as the seconds that start and end them.
fn span(first: i64, last: i64) -> Option<(i64, i64)> {
    Some((start_of(date_at(first)?)?, end_of(date_at(last)?)?))
}

/// Maps a Unix second to a whole local day relative to the epoch.
fn day_number(seconds: i64) -> i64 {
    local_date(seconds).map_or(0, date_number)
}

fn date_number(date: Date) -> i64 {
    date.since((Unit::Day, EPOCH))
        .map_or(0, |days| i64::from(days.get_days()))
}

fn date_at(day: i64) -> Option<Date> {
    EPOCH.checked_add(Span::new().try_days(day).ok()?).ok()
}
