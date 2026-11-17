use gpui::accesskit::Role;
use gpui::prelude::*;

use gpui::{AnyElement, App, Context, Hsla, Pixels, SharedString, div, relative};
use i18n::t;
use music::{Overview, Ranked};
use ui::{ActiveTheme as _, Perch, Text, Tipped as _};

use super::StatsView;

/// The hours labelled under the listening clock.
const HOUR_MARKS: [usize; 5] = [0, 6, 12, 18, 23];

impl StatsView {
    /// A grid of the plays in each hour of each weekday, darker where there were more, with the
    /// peak hour beside the heading. Nothing shows for a source without play times.
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

/// The colour of a cell that holds `count` of at most `most`.
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
/// How many rows the terms are packed into.
const ROWS: usize = 2;
/// The height of a row, as a multiple of the theme's row height.
const ROW_SCALE: f32 = 0.9;
/// How opaque a tile's colour is until it is hovered.
const TILE_OPACITY: f32 = 0.8;
/// The smallest share, in percent, that a term needs to get a tile.
const MIN_SHARE: usize = 2;
/// How wide a character of a label is, as a multiple of its text size.
const GLYPH: f32 = 0.65;

impl StatsView {
    /// Packs the ranked terms into rounded tiles whose widths follow their shares. A name shows
    /// on a tile that has room for it, and the share is in the tooltip.
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

/// Splits the terms into up to `ROWS` consecutive rows of nearly equal total weight, each term
/// going to the row its midpoint falls in.
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
