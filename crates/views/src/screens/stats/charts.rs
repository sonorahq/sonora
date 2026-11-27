use gpui::accesskit::Role;
use gpui::prelude::*;

use gpui::{
    AnyElement, App, Bounds, Context, Hsla, PathBuilder, Pixels, Point, SharedString, Window,
    canvas, div, point, px, relative,
};
use i18n::t;
use music::{HourMood, Overview, Ranked, TimeSlice};
use ui::{ActiveTheme as _, Perch, Text, Tipped as _};

use super::StatsView;
use super::timeline::date_label;

const HOUR_MARKS: [usize; 5] = [0, 6, 12, 18, 23];

impl StatsView {
    /// Shows activity by weekday and local hour, with the peak beside the heading.
    pub(super) fn clock(
        &self,
        overview: &Overview,
        cx: &Context<Self>,
    ) -> Option<(AnyElement, Option<AnyElement>)> {
        if overview.clock.is_empty() {
            return None;
        }
        let note = self.badges(
            [overview.peak_hour.map(|hour| {
                (
                    t!("stats-badge-peak", hour = hour_label(usize::from(hour))),
                    false,
                )
            })],
            cx,
        );
        let theme = *cx.theme();
        let most = overview
            .clock
            .iter()
            .copied()
            .max()
            .unwrap_or_default()
            .max(1);
        let gap = theme.metrics.pad / 4.;
        let grid =
            div()
                .id("stats-clock")
                .role(Role::List)
                .aria_label(t!("stats-clock-title"))
                .flex()
                .flex_col()
                .gap(gap)
                .children((0..7).map(|day| {
                    let plays = &overview.clock[day * 24..(day + 1) * 24];
                    let name = weekday(day);
                    let summary = t!(
                        "stats-clock-day",
                        day = name.as_str(),
                        count = plays.iter().sum::<usize>()
                    );
                    div()
                        .id(("stats-clock-day", day))
                        .role(Role::ListItem)
                        .aria_label(summary)
                        .flex()
                        .items_center()
                        .gap(theme.metrics.pad)
                        .child(
                            div()
                                .w(theme.metrics.pad * 4.)
                                .flex_none()
                                .child(ui::faint(cx).child(name.clone())),
                        )
                        .child(div().flex().flex_1().gap(gap).children(
                            plays.iter().enumerate().map(|(hour, count)| {
                                div()
                                    .id(("stats-clock-cell", day * 24 + hour))
                                    .tip_label(
                                        format!("{name} {} · {count}", hour_label(hour)),
                                        Perch::Above,
                                    )
                                    .flex_1()
                                    .h(theme.metrics.pad * 1.75)
                                    .rounded(theme.radius / 2.)
                                    .bg(heat(*count, most, cx))
                            }),
                        ))
                }))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .pl(theme.metrics.pad * 5.)
                        .children(HOUR_MARKS.map(|hour| ui::faint(cx).child(hour_label(hour)))),
                )
                .into_any_element();
        Some((grid, note))
    }
}

fn heat(count: usize, most: usize, cx: &App) -> Hsla {
    let theme = cx.theme();
    match count {
        0 => theme.muted_foreground.opacity(0.1),
        _ => theme
            .primary
            .opacity(0.2 + 0.8 * count as f32 / most as f32),
    }
}

fn weekday(day: usize) -> SharedString {
    t!("stats-weekday", day = day)
}

fn hour_label(hour: usize) -> String {
    format!("{hour:02}:00")
}

/// How far round the colour wheel each successive term moves, so neighbours never look alike.
const HUE_STEP: f32 = 0.618_034;
const ROWS: usize = 2;
/// The height of a row, as a multiple of the theme's row height.
const ROW_SCALE: f32 = 0.9;
const TILE_OPACITY: f32 = 0.8;
/// The smallest share, in percent, that a term needs to get a tile.
const MIN_SHARE: usize = 2;
/// How wide a character of a label is, as a multiple of its text size.
const GLYPH: f32 = 0.65;
const LINE_WIDTH: Pixels = px(2.);
const GUIDE_WIDTH: Pixels = px(1.);
const DOT_RADIUS: Pixels = px(3.);
const DOT_SEGMENTS: usize = 12;

impl StatsView {
    /// Sizes term tiles by share and exposes each percentage in a tooltip.
    pub(super) fn distribution(
        &self,
        tag: &'static str,
        label: SharedString,
        terms: &[Ranked<String>],
        width: Pixels,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let hits = |term: &Ranked<String>| term.count.unwrap_or_default();
        let total = terms.iter().map(hits).sum::<usize>().max(1);
        let terms: Vec<_> = terms
            .iter()
            .filter(|term| hits(term) * 100 >= total * MIN_SHARE)
            .cloned()
            .collect();
        let terms = terms.as_slice();
        let gap = theme.metrics.pad / 2.;
        div()
            .id(SharedString::from(format!("{tag}-distribution")))
            .role(Role::List)
            .aria_label(label)
            .flex()
            .flex_col()
            .gap(gap)
            .children(rows(terms).into_iter().map(|row| {
                let row_hits = row.iter().map(|&at| hits(&terms[at])).sum::<usize>().max(1);
                div()
                    .flex()
                    .w_full()
                    .h(theme.metrics.row * ROW_SCALE)
                    .gap(gap)
                    .children(row.into_iter().map(|at| {
                        let term = &terms[at];
                        let share = hits(term) as f32 / row_hits as f32;
                        let name = term.item.as_str();
                        let summary = t!(
                            "stats-genre-share",
                            genre = name,
                            percent = (hits(term) as f32 / total as f32 * 100.).round() as usize
                        );
                        let room = theme.text(Text::Small) * name.chars().count() as f32 * GLYPH
                            + theme.metrics.pad * 2.;
                        let colour = theme.primary_of(Hsla {
                            h: (theme.primary.h + at as f32 * HUE_STEP).fract(),
                            ..theme.primary
                        });
                        div()
                            .id(SharedString::from(format!("{tag}-term-{name}")))
                            .role(Role::ListItem)
                            .aria_label(summary.clone())
                            .aria_position_in_set(at + 1)
                            .aria_size_of_set(terms.len())
                            .tip_label(summary, Perch::Above)
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w_0()
                            .w(relative(share))
                            .h_full()
                            .px(theme.metrics.pad / 2.)
                            .rounded(theme.radius / 2.)
                            .bg(colour.opacity(TILE_OPACITY))
                            .hover(|tile| tile.bg(colour))
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.primary_foreground)
                            .when(width * share >= room, |tile| {
                                tile.child(div().truncate().child(name.to_owned()))
                            })
                    }))
            }))
            .into_any_element()
    }
}

/// One plotted line in [`StatsView::line_chart`].
struct LineSeries {
    name: SharedString,
    color: Hsla,
    points: Vec<(f32, f32)>,
}

/// A filled min–max band behind a line chart: fill colour and `(x, low, high)` points.
type RangeBand = (Hsla, Vec<(f32, f32, f32)>);

impl StatsView {
    /// Plots positivity (`valence`) and intensity (`energy`) over the listening window.
    pub(super) fn mood_history(
        &self,
        slices: &[TimeSlice],
        cx: &App,
    ) -> Option<(AnyElement, Option<AnyElement>)> {
        if slices.len() < 2 {
            return None;
        }
        let (first_at, last_at, span) = time_span(slices)?;
        let (series, tips) = mood_data(
            slices.iter().map(|slice| {
                (
                    (slice.at - first_at) as f32 / span,
                    date_label(slice.at),
                    [slice.valence, slice.energy],
                    slice.plays,
                )
            }),
            cx,
        );
        let (from, to) = (date_label(first_at), date_label(last_at));
        self.line_chart(
            "stats-mood-history",
            t!(
                "stats-mood-history-label",
                from = from.clone(),
                to = to.clone()
            ),
            series,
            None,
            tips,
            (from, to),
            cx,
        )
    }

    /// Plots mean tempo and the minimum–maximum band for each time slice.
    pub(super) fn tempo_history(
        &self,
        slices: &[TimeSlice],
        cx: &App,
    ) -> Option<(AnyElement, Option<AnyElement>)> {
        if slices.iter().filter_map(|s| s.bpm).count() < 2 {
            return None;
        }
        let theme = *cx.theme();
        let (first_at, last_at, span) = time_span(slices)?;
        let min_bpm = slices
            .iter()
            .filter_map(|s| s.bpm_min.or(s.bpm))
            .fold(f32::INFINITY, f32::min);
        let max_bpm = slices
            .iter()
            .filter_map(|s| s.bpm_max.or(s.bpm))
            .fold(f32::NEG_INFINITY, f32::max);
        let range = (max_bpm - min_bpm).max(20.);
        let floor_bpm = (min_bpm - range * 0.1).max(0.);
        let ceil_bpm = max_bpm + range * 0.1;
        let denom = (ceil_bpm - floor_bpm).max(1.);
        let norm = |bpm: f32| (bpm - floor_bpm) / denom;
        let series = vec![LineSeries {
            name: t!("stats-tempo-title"),
            color: theme.primary,
            points: slices
                .iter()
                .filter_map(|s| {
                    s.bpm
                        .map(|bpm| ((s.at - first_at) as f32 / span, norm(bpm)))
                })
                .collect(),
        }];
        let band = slices
            .iter()
            .filter_map(|s| {
                let low = s.bpm_min.or(s.bpm)?;
                let high = s.bpm_max.or(s.bpm)?;
                Some(((s.at - first_at) as f32 / span, norm(low), norm(high)))
            })
            .collect::<Vec<_>>();
        let band = (band.len() >= 2).then_some((theme.primary.opacity(0.16), band));
        let tips = slices
            .iter()
            .map(|s| {
                let mut parts = vec![date_label(s.at)];
                if let Some(bpm) = s.bpm {
                    parts.push(t!("stats-tempo-bpm", bpm = bpm.round() as u16).to_string());
                }
                if let (Some(low), Some(high)) = (s.bpm_min, s.bpm_max)
                    && (high - low).round() >= 1.
                {
                    parts.push(format!("{}–{}", low.round() as u16, high.round() as u16));
                }
                parts.push(t!("stats-badge-plays", count = s.plays, value = s.plays).to_string());
                SharedString::from(parts.join(" · "))
            })
            .collect();
        let (from, to) = (date_label(first_at), date_label(last_at));
        self.line_chart(
            "stats-tempo-history",
            t!(
                "stats-tempo-history-label",
                from = from.clone(),
                to = to.clone()
            ),
            series,
            band,
            tips,
            (from, to),
            cx,
        )
    }

    /// Plots positivity and energy across local hours of the day (`00:00`–`23:00`).
    pub(super) fn hourly_mood(
        &self,
        hours: &[HourMood],
        cx: &App,
    ) -> Option<(AnyElement, Option<AnyElement>)> {
        if hours.len() != 24 {
            return None;
        }
        let (series, tips) = mood_data(
            hours.iter().enumerate().map(|(hour, mood)| {
                (
                    hour as f32 / 23.,
                    hour_label(hour),
                    [mood.valence, mood.energy],
                    mood.plays,
                )
            }),
            cx,
        );
        self.line_chart(
            "stats-hourly-mood",
            t!("stats-hourly-mood-label"),
            series,
            None,
            tips,
            (hour_label(0), hour_label(23)),
            cx,
        )
    }

    /// Draws series, range bands, legends and hover values on common axes.
    #[allow(clippy::too_many_arguments)]
    fn line_chart(
        &self,
        id: &'static str,
        label: SharedString,
        series: Vec<LineSeries>,
        band: Option<RangeBand>,
        tips: Vec<SharedString>,
        ends: (String, String),
        cx: &App,
    ) -> Option<(AnyElement, Option<AnyElement>)> {
        let active: Vec<LineSeries> = series.into_iter().filter(|s| s.points.len() >= 2).collect();
        if active.is_empty() {
            return None;
        }
        let theme = *cx.theme();
        let guide_color = theme.muted_foreground.opacity(0.2);
        let legend = (active.len() > 1).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(theme.metrics.pad)
                .children(
                    active
                        .iter()
                        .map(|s| legend_item(s.color, s.name.clone(), cx)),
                )
                .into_any_element()
        });
        let painted: Vec<(Hsla, Vec<(f32, f32)>)> =
            active.into_iter().map(|s| (s.color, s.points)).collect();

        let graph = div()
            .id(id)
            .role(Role::Group)
            .aria_label(label)
            .flex()
            .flex_col()
            .gap(theme.metrics.pad / 2.)
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(theme.metrics.cover * 0.75)
                    .border_b_1()
                    .border_color(theme.muted_foreground.opacity(0.25))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                paint_lines(bounds, &painted, band.as_ref(), guide_color, window);
                            },
                        )
                        .size_full(),
                    )
                    .child(div().absolute().inset_0().flex().children(
                        tips.into_iter().enumerate().map(|(at, tip)| {
                            div()
                                .id((id, at))
                                .flex_1()
                                .h_full()
                                .tip_label(tip, Perch::Above)
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(ui::faint(cx).child(ends.0))
                    .child(ui::faint(cx).child(ends.1)),
            )
            .into_any_element();

        Some((graph, legend))
    }
}

fn time_span(slices: &[TimeSlice]) -> Option<(i64, i64, f32)> {
    let first = slices.first()?.at;
    let last = slices.last()?.at;
    Some((first, last, (last - first).max(1) as f32))
}

/// Builds the shared mood lines and tooltips for either time scale.
fn mood_data(
    samples: impl Iterator<Item = (f32, String, [Option<f32>; 2], usize)>,
    cx: &App,
) -> (Vec<LineSeries>, Vec<SharedString>) {
    let theme = cx.theme();
    let energy = theme.primary_of(Hsla {
        h: (theme.primary.h + HUE_STEP).fract(),
        ..theme.primary
    });
    let mut series = [
        (t!("stats-mood-history-valence"), theme.primary),
        (t!("stats-mood-history-energy"), energy),
    ]
    .map(|(name, color)| LineSeries {
        name,
        color,
        points: Vec::new(),
    });
    let tips = samples
        .map(|(x, label, values, plays)| {
            let mut parts = vec![label];
            for (line, value) in series.iter_mut().zip(values) {
                if let Some(value) = value {
                    line.points.push((x, value));
                    parts.push(format!("{} {}%", line.name, (value * 100.).round() as u8));
                }
            }
            parts.push(t!("stats-badge-plays", count = plays, value = plays).to_string());
            SharedString::from(parts.join(" · "))
        })
        .collect();
    (Vec::from(series), tips)
}

fn legend_item(color: Hsla, label: SharedString, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    div()
        .flex()
        .items_center()
        .gap(theme.metrics.pad / 2.)
        .child(
            div()
                .size(theme.metrics.pad * 0.75)
                .rounded_full()
                .bg(color),
        )
        .child(ui::faint(cx).child(label))
        .into_any_element()
}

fn paint_lines(
    bounds: Bounds<Pixels>,
    series: &[(Hsla, Vec<(f32, f32)>)],
    band: Option<&RangeBand>,
    guide_color: Hsla,
    window: &mut Window,
) {
    let inset = DOT_RADIUS;
    let width = (bounds.size.width - inset * 2.).max(px(1.));
    let height = (bounds.size.height - inset * 2.).max(px(1.));
    let left = bounds.left() + inset;
    let top = bounds.top() + inset;
    let project = |x: f32, y: f32| {
        point(
            left + width * x.clamp(0., 1.),
            top + height * (1. - y.clamp(0., 1.)),
        )
    };

    let mid_y = top + height * 0.5;
    let mut guide = PathBuilder::stroke(GUIDE_WIDTH);
    guide.add_polygon(&[point(left, mid_y), point(left + width, mid_y)], false);
    if let Ok(path) = guide.build() {
        window.paint_path(path, guide_color);
    }

    if let Some((fill_color, raw)) = band
        && raw.len() >= 2
    {
        let mut polygon: Vec<Point<Pixels>> =
            raw.iter().map(|&(x, _, high)| project(x, high)).collect();
        polygon.extend(raw.iter().rev().map(|&(x, low, _)| project(x, low)));
        let mut area = PathBuilder::fill();
        area.add_polygon(&polygon, true);
        if let Ok(path) = area.build() {
            window.paint_path(path, *fill_color);
        }
    }

    for (color, raw) in series.iter().rev() {
        let pts: Vec<Point<Pixels>> = raw.iter().map(|&(x, y)| project(x, y)).collect();
        paint_series(&pts, *color, window);
    }
}

fn paint_series(points: &[Point<Pixels>], color: Hsla, window: &mut Window) {
    if points.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(LINE_WIDTH);
    path.add_polygon(points, false);
    if let Ok(built) = path.build() {
        window.paint_path(built, color);
    }
    for &center in points {
        let ring: Vec<Point<Pixels>> = (0..DOT_SEGMENTS)
            .map(|step| {
                let angle = step as f32 / DOT_SEGMENTS as f32 * std::f32::consts::TAU;
                point(
                    center.x + DOT_RADIUS * angle.cos(),
                    center.y + DOT_RADIUS * angle.sin(),
                )
            })
            .collect();
        let mut dot = PathBuilder::fill();
        dot.add_polygon(&ring, true);
        if let Ok(built) = dot.build() {
            window.paint_path(built, color);
        }
    }
}

/// Assigns each term to the row containing its cumulative weight midpoint.
fn rows(terms: &[Ranked<String>]) -> Vec<Vec<usize>> {
    let weight = |term: &Ranked<String>| term.count.unwrap_or_default().max(1) as f64;
    let total: f64 = terms.iter().map(weight).sum();
    let count = terms.len().min(ROWS);
    let mut rows = vec![Vec::new(); count];
    let mut running = 0.;
    for (at, term) in terms.iter().enumerate() {
        let middle = running + weight(term) / 2.;
        rows[((middle / total * count as f64) as usize).min(count - 1)].push(at);
        running += weight(term);
    }
    rows.retain(|row| !row.is_empty());
    rows
}
