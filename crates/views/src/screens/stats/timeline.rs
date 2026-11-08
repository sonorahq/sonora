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
use super::{BAR_OFF, BAR_ON, StatsView};

const EPOCH: Date = Date::constant(1970, 1, 1);
/// The quick ranges over recorded plays, longest first like the windows of a provider and the
/// scrubber's track: the Fluent key naming each, and its length in days.
const PRESETS: [(&str, i64); 5] = [
    ("stats-year", 365),
    ("stats-six-months", 182),
    ("stats-quarter", 91),
    ("stats-month", 30),
    ("stats-week", 7),
];
/// How far the scrubber's track is inset from its ends, as a multiple of the theme pad.
const TRACK_INSET: f32 = 0.75;

impl StatsView {
    /// The window the rankings cover, and the way to change it: a free range of dates over the
    /// recorded plays, or a pick among the fixed windows of a provider or scrobbler.
    pub(super) fn timeline(
        &self,
        source: StatsSource,
        periods: &[StatsPeriod],
        period: StatsPeriod,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let parts = match source {
            StatsSource::Provider => self.fixed_window(source, periods, period, cx),
            _ => self.dated_window(cx),
        };
        // What the window held sits at the right of the title row, level with the title.
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
            .children(parts)
            .when_some(badges, |card, badges| card.trailing(badges))
            .into_any_element()
    }

    /// Names what the figures and rankings on show cover: the selected dates of the recorded
    /// plays, or the fixed window of any other source.
    pub(super) fn window_label(&self, cx: &App) -> SharedString {
        let stats = self.stats.read(cx);
        match (stats.source(), stats.dates()) {
            (StatsSource::Provider, _) => i18n::lookup(period_key(stats.period()), None),
            (_, Some((from, to))) => t!(
                "stats-selected-dates",
                from = date_label(from),
                to = date_label(to)
            ),
            (_, None) => t!("stats-all-time"),
        }
    }

    /// Keeps the date fields on the selected range, or on all the history, once either changes.
    pub(super) fn sync_dates(&mut self, cx: &mut Context<Self>) {
        let stats = self.stats.read(cx);
        if stats.source() == StatsSource::Provider {
            return;
        }
        let Some((from, to)) = stats.dates().or_else(|| stats.bounds()) else {
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

    /// The fixed windows of a source, laid along one track from the broadest on the left to the
    /// shortest on the right, all ending now. A source with a single window has nothing to pick.
    fn fixed_window(
        &self,
        source: StatsSource,
        periods: &[StatsPeriod],
        period: StatsPeriod,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = *cx.theme();
        let windows: Vec<_> = periods.iter().copied().rev().collect();
        let label = i18n::lookup(period_key(period), None);
        let mut parts = vec![headline(label.clone(), None, cx)];
        if windows.len() < 2 {
            parts.push(
                ui::faint(cx)
                    .child(t!("stats-window-fixed"))
                    .into_any_element(),
            );
            return parts;
        }

        let count = windows.len();
        let middle = |index: usize| (index as f32 + 0.5) / count as f32;
        let selected = windows
            .iter()
            .position(|window| *window == period)
            .unwrap_or(0);
        let picks = windows.clone();
        parts.push(
            RangeScrubber::new(&self.range, (middle(selected), 1.))
                .stops((0..count).map(middle).collect())
                .lock_high()
                .name(t!("stats-period-title"))
                .values(label, t!("stats-now"))
                .colors(theme.primary, theme.secondary, theme.foreground)
                .on_change(cx.listener(move |this, &(low, _): &(f32, f32), _, cx| {
                    let index =
                        ((low * count as f32 - 0.5).round().max(0.) as usize).min(count - 1);
                    let chosen = picks[index];
                    if chosen != this.stats.read(cx).period() {
                        this.select(source, chosen, cx);
                    }
                }))
                .into_any_element(),
        );
        parts.push(
            TabBar::new("stats-windows")
                .name(t!("stats-period-title"))
                .items(windows.into_iter().enumerate().map(|(index, window)| {
                    Button::new(("stats-window", index))
                        .label(i18n::lookup(period_key(window), None))
                        .small()
                        .ghost()
                        .selected(window == period)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.select(source, window, cx)),
                        )
                }))
                .into_any_element(),
        );
        parts
    }

    /// A free range over the recorded plays: a scrubber under the shape of the listening, quick
    /// ranges, and exact dates.
    fn dated_window(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = *cx.theme();
        let (bounds, dates) = {
            let stats = self.stats.read(cx);
            (stats.bounds(), stats.dates())
        };
        let label = self.window_label(cx);
        let Some(bounds) = bounds else {
            return vec![headline(label, None, cx)];
        };

        let low = day_number(bounds.0);
        let width = (day_number(bounds.1).max(low + 1) - low) as f32;
        let share = |seconds| ((day_number(seconds) - low) as f32 / width).clamp(0., 1.);
        let value = dates.map_or((0., 1.), |(from, to)| (share(from), share(to)));
        let days = dates.map_or(width as i64 + 1, |(from, to)| {
            day_number(to) - day_number(from) + 1
        });
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
            self.date_row(Some(t!("count-days", count = days)), cx),
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
        parts.push(self.presets(dates, cx));
        parts
    }

    /// Bars for the plays across the whole history, in colour where the selection covers them,
    /// standing on a floor line.
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

    /// Marks the busiest day and the longest streak of the whole history under the scrubber, so
    /// they stay put as the window moves. The streak is a bar across its days with its length
    /// inside, and the busiest day a line up to its day with its label below. Each is a share of
    /// the track's width. Nothing shows when the source has neither.
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

    /// The whole history, and quick ranges ending today, longest first.
    fn presets(&self, dates: Option<(i64, i64)>, cx: &mut Context<Self>) -> AnyElement {
        let today = day_number(Timestamp::now().as_second());
        let all = Button::new("stats-preset-all")
            .label(t!("stats-all-time"))
            .small()
            .ghost()
            .selected(dates.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.set_dates(None, cx)));
        let recent = PRESETS.into_iter().map(|(key, days)| {
            let first = today - (days - 1);
            let chosen = dates
                .is_some_and(|(from, to)| day_number(from) == first && day_number(to) == today);
            Button::new(("stats-preset", days as usize))
                .label(i18n::lookup(key, None))
                .small()
                .ghost()
                .selected(chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_dates(span(first, today), cx);
                }))
        });
        TabBar::new("stats-presets")
            .name(t!("stats-period-title"))
            .items(std::iter::once(all).chain(recent))
            .into_any_element()
    }

    /// The selected dates as two fields that take the window the moment they hold dates in
    /// order, with a quieter note beside them.
    fn date_row(&self, note: Option<SharedString>, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let field =
            |input: &gpui::Entity<ui::Input>| div().w(theme.metrics.cover).child(input.clone());
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
                    .child(field(&self.start))
                    .child(ui::faint(cx).child("–"))
                    .child(field(&self.end)),
            )
            .children(note.map(|note| ui::faint(cx).child(note)))
            .into_any_element()
    }

    /// Applies a range of dates, where one covering the whole history counts as all of it.
    fn set_dates(&mut self, dates: Option<(i64, i64)>, cx: &mut Context<Self>) {
        self.rewind();
        let bounds = self.stats.read(cx).bounds();
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

    /// Applies the typed dates once they are dates in order, and flags the fields while they
    /// are not. Typing a range that is already selected changes nothing.
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
            stats.dates().or_else(|| stats.bounds())
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

/// The selected window as a heading, with a quieter note beside it.
fn headline(label: SharedString, note: Option<SharedString>, cx: &App) -> AnyElement {
    div()
        .flex()
        .items_baseline()
        .justify_between()
        .gap(cx.theme().metrics.pad)
        .child(ui::heading(label, cx))
        .children(note.map(|note| ui::faint(cx).child(note)))
        .into_any_element()
}

fn period_key(period: StatsPeriod) -> &'static str {
    match period {
        StatsPeriod::Weeks => "stats-four-weeks",
        StatsPeriod::Months => "stats-six-months",
        StatsPeriod::Year => "stats-year",
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

/// The local calendar date of a Unix second.
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

/// The first second of a local date. Local midnight is looked up for the date itself, so a day
/// with a daylight saving change is still whole.
fn start_of(date: Date) -> Option<i64> {
    let midnight = date.at(0, 0, 0, 0).to_zoned(TimeZone::system()).ok()?;
    Some(midnight.timestamp().as_second())
}

/// The last second of a local date.
fn end_of(date: Date) -> Option<i64> {
    Some(start_of(date.tomorrow().ok()?)? - 1)
}

/// The whole days from the first day to the last, as the seconds that start and end them.
fn span(first: i64, last: i64) -> Option<(i64, i64)> {
    Some((start_of(date_at(first)?)?, end_of(date_at(last)?)?))
}

/// The count of days from 1970-01-01 to the local date of a Unix second, which makes a slider
/// handle land on whole days.
fn day_number(seconds: i64) -> i64 {
    local_date(seconds).map_or(0, date_number)
}

/// The count of days from 1970-01-01 to a date.
fn date_number(date: Date) -> i64 {
    date.since((Unit::Day, EPOCH))
        .map_or(0, |days| i64::from(days.get_days()))
}

/// The date `day_number` counted `day` days from 1970-01-01 for.
fn date_at(day: i64) -> Option<Date> {
    EPOCH.checked_add(Span::new().try_days(day).ok()?).ok()
}
