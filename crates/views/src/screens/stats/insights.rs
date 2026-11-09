use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, FontWeight, Hsla, Pixels, SharedString, div, px, relative, svg,
};
use i18n::t;
use music::{Milestone, Trait};
use ui::{ActiveTheme as _, Fact, Perch, Text, Theme, Tipped as _};

use super::timeline::date_label;
use super::{BAR_OFF, BAR_ON, StatsView, TILE_FILL, TILE_PAD, peak_era};

/// The size of a tile's icon, as a multiple of the theme pad, which is about the height of the
/// tile, and how faint a caption on the accent is.
const ICON_SIZE: f32 = 3.;
const ICON_QUIET: f32 = 0.8;
/// The narrowest a listening style tile is, as a multiple of the cover size.
const TRAIT_WIDTH: f32 = 2.6;
/// What a tile gives up of its share of a row, so that rounding never pushes one onto the next.
const ROUNDING: Pixels = px(1.);

impl StatsView {
    /// How the listener listens, in a few labels worked out from the figures, or nothing when
    /// the source has too few to say.
    pub(super) fn style(
        &self,
        traits: Vec<Trait>,
        width: Pixels,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let theme = *cx.theme();
        // As many to a row as the width holds, shared out so that the rows come out even and
        // each tile grows to fill its row.
        let gap = theme.metrics.pad;
        let narrowest = theme.metrics.cover * TRAIT_WIDTH;
        let fit = (((width + gap) / (narrowest + gap)) as usize).max(1);
        let rows = traits.len().div_ceil(fit).max(1);
        let columns = traits.len().div_ceil(rows).max(1);
        let each = (width - gap * (columns as f32 - 1.)) / columns as f32;
        (!traits.is_empty()).then(|| {
            div()
                .id("stats-style")
                .role(Role::List)
                .aria_label(t!("stats-style-title"))
                .flex()
                .flex_wrap()
                .gap(theme.metrics.pad)
                .children(traits.into_iter().map(|which| {
                    let (icon, name, note) = wording(which);
                    div()
                        .id(SharedString::from(format!("stats-trait-{name}")))
                        .role(Role::ListItem)
                        .aria_label(SharedString::from(format!("{name}. {note}")))
                        .flex_1()
                        .min_w(each - ROUNDING)
                        .child(tile(&theme, icon, name, note))
                }))
                .into_any_element()
        })
    }

    /// Small rounded labels for the figures that go with a section, the lit ones in the accent
    /// colour. Nothing shows when there are none.
    pub(super) fn badges(
        &self,
        items: impl IntoIterator<Item = Option<(SharedString, bool)>>,
        cx: &App,
    ) -> Option<AnyElement> {
        let theme = *cx.theme();
        let items: Vec<_> = items.into_iter().flatten().collect();
        (!items.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(theme.metrics.pad / 2.)
                .children(items.into_iter().map(|(text, lit)| {
                    div()
                        .flex_none()
                        .px(theme.metrics.pad)
                        .py(theme.metrics.pad / 8.)
                        .rounded_full()
                        .text_size(theme.text(Text::Small))
                        .font_weight(FontWeight::MEDIUM)
                        .map(|this| match lit {
                            true => this
                                .bg(theme.primary.opacity(0.18))
                                .text_color(theme.primary),
                            false => this
                                .bg(theme.secondary.opacity(TILE_FILL))
                                .text_color(theme.muted_foreground),
                        })
                        .child(text)
                }))
                .into_any_element()
        })
    }

    /// The plays of the whole history that reached a round number, oldest first, as the same
    /// label and value rows the song page lists its details in.
    pub(super) fn milestones(&self, milestones: &[Milestone]) -> Option<AnyElement> {
        (!milestones.is_empty()).then(|| {
            div()
                .id("stats-milestones")
                .role(Role::List)
                .aria_label(t!("stats-milestones-title"))
                .flex()
                .flex_col()
                .children(milestones.iter().enumerate().map(|(index, milestone)| {
                    let label = match milestone.number {
                        1 => t!("stats-milestone-first"),
                        count => t!("stats-milestone-nth", count = count),
                    };
                    let (title, artist) = (&milestone.title, &milestone.artist);
                    let value = format!("{title} · {artist} · {}", date_label(milestone.at));
                    Fact::new(label, value)
                        .spoken(("stats-milestone", milestone.number))
                        .striped(index % 2 == 1)
                }))
                .into_any_element()
        })
    }

    /// Bars for how many of the songs came out in each year, from the oldest to the newest, with
    /// the years between that none came out in left empty. The years of the peak era stand out.
    pub(super) fn years(&self, years: &[(i32, usize)], cx: &App) -> Option<AnyElement> {
        let (first, last) = (years.first()?.0, years.last()?.0);
        let era = peak_era(years);
        let bars = (first..=last)
            .map(|year| {
                let count = years
                    .iter()
                    .find(|(at, _)| *at == year)
                    .map_or(0, |(_, count)| *count);
                let in_era = era.is_some_and(|(from, to)| (from..=to).contains(&year));
                (
                    t!("stats-year-songs", year = year, count = count),
                    count,
                    in_era,
                )
            })
            .collect();
        Some(self.histogram(
            "stats-years",
            t!("stats-years-label", from = first, to = last),
            bars,
            (first.to_string(), last.to_string()),
            cx,
        ))
    }

    /// Bars for how many of the songs play at each tempo, slowest to fastest in bands of ten
    /// beats per minute, with the busiest band standing out.
    pub(super) fn tempo(&self, bands: &[(u16, usize)], cx: &App) -> Option<AnyElement> {
        let (first, last) = (bands.first()?.0, bands.last()?.0);
        let most = bands.iter().map(|(_, count)| *count).max()?;
        let bars = (first..=last)
            .step_by(10)
            .map(|band| {
                let count = bands
                    .iter()
                    .find(|(at, _)| *at == band)
                    .map_or(0, |(_, count)| *count);
                (
                    t!(
                        "stats-tempo-songs",
                        from = band,
                        to = band + 9,
                        count = count
                    ),
                    count,
                    count == most,
                )
            })
            .collect();
        Some(self.histogram(
            "stats-tempo",
            t!("stats-tempo-label", from = first, to = last + 9),
            bars,
            (
                t!("stats-tempo-bpm", bpm = first).to_string(),
                t!("stats-tempo-bpm", bpm = last + 9).to_string(),
            ),
            cx,
        ))
    }

    /// Bars standing on a floor line, each with its tooltip, its count and whether it stands out,
    /// under the labels of the first and the last. The release years and the tempos share it.
    fn histogram(
        &self,
        id: &'static str,
        label: SharedString,
        bars: Vec<(SharedString, usize, bool)>,
        ends: (String, String),
        cx: &App,
    ) -> AnyElement {
        let theme = *cx.theme();
        let most = bars
            .iter()
            .map(|(_, count, _)| *count)
            .max()
            .unwrap_or_default()
            .max(1);
        div()
            .id(id)
            .role(Role::Group)
            .aria_label(label)
            .flex()
            .flex_col()
            .gap(theme.metrics.pad / 2.)
            .child(
                floor(&theme).children(bars.into_iter().enumerate().map(
                    |(at, (tip, count, stands_out))| {
                        let fill = match (count, stands_out) {
                            (0, _) => theme.muted_foreground.opacity(BAR_OFF),
                            (_, true) => theme.primary,
                            (_, false) => theme.primary.opacity(BAR_ON),
                        };
                        bar(&theme, count as f32 / most as f32, fill)
                            .id((id, at))
                            .tip_label(tip, Perch::Above)
                    },
                )),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(ui::faint(cx).child(ends.0))
                    .child(ui::faint(cx).child(ends.1)),
            )
            .into_any_element()
    }
}

/// The row that bars stand in, resting on a floor line.
pub(super) fn floor(theme: &Theme) -> Div {
    div()
        .flex()
        .items_end()
        .gap(theme.metrics.pad / 8.)
        .h(theme.metrics.cover / 2.)
        .border_b_1()
        .border_color(theme.muted_foreground.opacity(BAR_OFF))
}

/// A bar `share` as tall as the row, rounded at the top only, so that it stands on the floor.
pub(super) fn bar(theme: &Theme, share: f32, fill: Hsla) -> Div {
    div()
        .flex_1()
        .h(relative(share))
        .min_h(theme.metrics.pad / 4.)
        .rounded_t(theme.radius / 2.)
        .bg(fill)
}

/// The icon of a trait, its name and a sentence on what it means. The name and the sentence are
/// the Fluent messages `stats-trait-<slug>` and `stats-trait-<slug>-note`.
fn wording(which: Trait) -> (&'static str, SharedString, SharedString) {
    let (icon, slug) = match which {
        Trait::EarlyBird => ("icons/sunrise.svg", "early-bird"),
        Trait::Daytime => ("icons/sun.svg", "daytime"),
        Trait::Evening => ("icons/sunset.svg", "evening"),
        Trait::NightOwl => ("icons/moon.svg", "night-owl"),
        Trait::Weekender => ("icons/calendar-days.svg", "weekender"),
        Trait::Weekday => ("icons/briefcase.svg", "weekday"),
        Trait::Explorer => ("icons/compass.svg", "explorer"),
        Trait::Loyalist => ("icons/heart.svg", "loyalist"),
        Trait::Mainstream => ("icons/trending-up.svg", "mainstream"),
        Trait::DeepDiver => ("icons/gem.svg", "deep-diver"),
        Trait::Throwback => ("icons/rotate-ccw-clock.svg", "throwback"),
        Trait::FreshEars => ("icons/sparkles.svg", "fresh-ears"),
        Trait::Collaborator => ("icons/users-round-collaboration.svg", "collaborator"),
        Trait::Superfan => ("icons/star.svg", "superfan"),
        Trait::Daily => ("icons/flame.svg", "daily"),
        Trait::Rising => ("icons/rocket.svg", "rising"),
        Trait::FastLane => ("icons/zap.svg", "fast-lane"),
        Trait::SlowBurner => ("icons/snail.svg", "slow-burner"),
    };
    let key = format!("stats-trait-{slug}");
    (
        icon,
        i18n::lookup(&key, None),
        i18n::lookup(&format!("{key}-note"), None),
    )
}

/// A tile of the accent colour with a large icon beside a name over a quieter sentence.
fn tile(theme: &Theme, icon: &'static str, title: SharedString, caption: SharedString) -> Div {
    div()
        .flex()
        .items_center()
        .gap(theme.metrics.pad * TILE_PAD)
        .w_full()
        .px(theme.metrics.pad * TILE_PAD)
        .py(theme.metrics.pad)
        .rounded(theme.radius)
        .bg(theme.primary)
        .child(
            svg()
                .path(icons::path(icon))
                .size(theme.metrics.pad * ICON_SIZE)
                .flex_none()
                .text_color(theme.primary_foreground),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .text_size(theme.text(Text::Large))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.primary_foreground)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.primary_foreground.opacity(ICON_QUIET))
                        .child(caption),
                ),
        )
}
