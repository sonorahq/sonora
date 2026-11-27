mod charts;
mod insights;
mod rankings;
mod timeline;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, ElementId, Entity, Render, SharedString, Window, div};
use i18n::t;
use music::{Ranked, StatsPeriod};
use state::{Playback, Sonora, Stats, StatsSource};
use ui::{ActiveTheme as _, Button, Input, RangeState, Scrollbar, Scroller, TabBar, Text};

use crate::chrome::Chrome;
use crate::shared::album_grid::CardGrid;
use crate::shared::shelves::{Rail, STACK_GAP, section};
use crate::shared::trouble;
use rankings::Kind;

/// How much of the secondary colour the stats tiles, rows and badges are filled with.
const TILE_FILL: f32 = 0.6;
/// The inner padding of a tile, as a multiple of the theme pad.
const TILE_PAD: f32 = 1.5;

/// How opaque a bar of a bar chart is where it counts and where it does not, in the primary and
/// the muted colour.
const BAR_ON: f32 = 0.6;
const BAR_OFF: f32 = 0.25;

/// How many years the era of a listener's songs spans.
const ERA: i32 = 5;

/// The room one stats section has, and what its rail needs to repaint the page.
#[derive(Clone, Copy)]
struct Layout<'a> {
    columns: usize,
    tile: gpui::Pixels,
    width: gpui::Pixels,
    window: &'a Window,
    notify: &'a Rc<dyn Fn(&mut App)>,
}

/// Displays rankings and listening history for the selected source and window.
pub(crate) struct StatsView {
    stats: Entity<Stats>,
    playback: Entity<Playback>,
    scrollbar: Entity<Scrollbar>,
    range: RangeState,
    start: Entity<Input>,
    end: Entity<Input>,
    invalid: bool,
    calendar: Option<(bool, jiff::civil::Date)>,
    rails: Vec<Rail>,
    alternate_charts: std::collections::HashSet<&'static str>,
}

impl StatsView {
    pub(crate) fn new(
        stats: Entity<Stats>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();
        cx.observe(&stats, |this, _, cx| {
            this.sync_dates(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let session = Sonora::global(cx).session.clone();
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let host = cx.entity_id();
        let field = |name, cx: &mut Context<Self>| {
            cx.new(|cx| Input::new("stats-date-hint", cx).compact().name(name))
        };
        let (start, end) = (field("range-from", cx), field("range-to", cx));
        for input in [&start, &end] {
            cx.observe(input, |this, _, cx| this.apply_dates(cx))
                .detach();
        }
        Self {
            stats,
            playback,
            scrollbar: cx.new(|_| Scrollbar::inset().watching(host)),
            range: RangeState::new("stats-date-range"),
            start,
            end,
            invalid: false,
            calendar: None,
            rails: Kind::ALL.iter().map(|_| Rail::new(host)).collect(),
            alternate_charts: Default::default(),
        }
    }

    /// Loads the page when it has nothing current, so that coming back to it is instant.
    pub(crate) fn ensure(&mut self, cx: &mut Context<Self>) {
        self.stats.update(cx, |stats, cx| stats.ensure(cx));
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.rewind();
        self.stats.update(cx, |stats, cx| stats.refresh(cx));
    }

    /// Brings every rail back to its first card, since the cards behind it are about to change.
    fn rewind(&self) {
        for rail in &self.rails {
            rail.rewind();
        }
    }

    fn select(&mut self, source: StatsSource, period: StatsPeriod, cx: &mut Context<Self>) {
        self.calendar = None;
        self.rewind();
        self.stats
            .update(cx, |stats, cx| stats.select(source, period, cx));
        cx.notify();
    }

    fn source_tabs(
        &self,
        source: StatsSource,
        period: StatsPeriod,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider = SharedString::from(
            Sonora::global(cx)
                .session
                .read(cx)
                .provider_name()
                .unwrap_or_default()
                .to_owned(),
        );
        let sources = self.stats.read(cx).sources(cx);
        TabBar::new("stats-source-tabs")
            .name(t!("stats-source"))
            .items(sources.into_iter().map(|candidate| {
                let (id, label) = match candidate {
                    StatsSource::Recorded => ("recorded", t!("stats-recorded")),
                    StatsSource::Provider => ("provider", provider.clone()),
                    StatsSource::Scrobbler(id) => {
                        (id, i18n::lookup(&format!("settings-{id}"), None))
                    }
                };
                stats_tab(
                    SharedString::from(format!("stats-source-{id}")),
                    label,
                    candidate == source,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select(candidate, period, cx)))
            }))
            .into_any_element()
    }

    fn failure(&self, cx: &Context<Self>) -> AnyElement {
        let detail = t!("stats-failed");
        let retry = cx.listener(|this, _, _, cx| this.refresh(cx));
        let vacancy = trouble::lost(
            "stats-lost",
            t!("trouble-not-loaded"),
            Some(detail.as_str()),
            retry,
            cx,
        );
        match state::Network::offline(cx) {
            true => vacancy,
            false => vacancy.detail(detail),
        }
        .into_any_element()
    }

    /// Builds available sections in reading order, with placeholders during loading.
    fn sections(&self, layout: &Layout<'_>, cx: &Context<Self>) -> Vec<AnyElement> {
        let stats = self.stats.read(cx);
        if stats.failed() {
            return vec![self.failure(cx)];
        }
        if stats.is_loading() {
            return Kind::MAIN
                .iter()
                .map(|kind| {
                    Rail::pending(kind.title(), layout.tile, layout.columns, layout.window, cx)
                })
                .collect();
        }

        let data = stats.data();
        let barren = data.tracks.is_empty() && data.artists.is_empty() && data.albums.is_empty();
        let times = &data.overview;
        // Moods the app names itself are Fluent keys, and a provider's own are plain words.
        let moods: Vec<_> = data
            .moods
            .iter()
            .cloned()
            .map(|mut mood| {
                if mood.item.starts_with("stats-mood-") {
                    mood.item = i18n::lookup(&mood.item, None).to_string();
                }
                mood
            })
            .collect();
        let era = self.badges(
            [
                peak_era(&data.years)
                    .map(|(from, to)| (t!("stats-badge-era", from = from, to = to), false)),
                times
                    .recent_share
                    .map(|share| (t!("stats-badge-recent", share = share), true)),
            ],
            cx,
        );
        let bpm = self.badges(
            [times
                .avg_bpm
                .map(|bpm| (t!("stats-badge-bpm", bpm = bpm), false))],
            cx,
        );
        let frame = |tag, title, note, body: Option<AnyElement>| {
            body.map(|body| section(tag, title, note, body, layout.window, cx))
        };
        let terms = |tag, title: SharedString, rows: &[Ranked<String>]| {
            let body = (!rows.is_empty())
                .then(|| self.distribution(tag, title.clone(), rows, layout.width, cx));
            frame(tag, title, None, body)
        };
        let (clock_body, clock_note) = self.chart(
            "stats-clock-view",
            (t!("stats-view-activity"), t!("stats-view-mood")),
            self.clock(times, cx)
                .map_or((None, None), |(grid, note)| (Some(grid), note)),
            self.hourly_mood(&data.hourly_mood, cx),
            cx,
        );

        let tempo_history = self.tempo_history(&data.timeline, cx);
        let (tempo_body, tempo_note) = self.chart(
            "stats-tempo-view",
            (t!("stats-view-bands"), t!("stats-view-over-time")),
            (self.tempo(&data.tempos, cx), bpm),
            tempo_history,
            cx,
        );

        let mood_strip = (!moods.is_empty()).then(|| {
            self.distribution(
                "stats-top-moods",
                t!("stats-top-moods"),
                &moods,
                layout.width,
                cx,
            )
        });
        let (mood_body, mood_note) = self.chart(
            "stats-mood-view",
            (t!("stats-view-breakdown"), t!("stats-view-over-time")),
            (mood_strip, None),
            self.mood_history(&data.timeline, cx),
            cx,
        );
        let mut sections: Vec<_> = frame(
            "stats-style",
            t!("stats-style-title"),
            None,
            self.style(data.traits(), layout.width, cx),
        )
        .into_iter()
        .collect();
        sections.extend(
            Kind::ALL
                .into_iter()
                .filter_map(|kind| self.rail(kind, layout, cx)),
        );
        sections.extend(
            [
                frame(
                    "stats-years",
                    t!("stats-years-title"),
                    era,
                    self.years(&data.years, cx),
                ),
                frame(
                    "stats-tempo",
                    t!("stats-tempo-title"),
                    tempo_note,
                    tempo_body,
                ),
                terms("stats-top-genres", t!("stats-top-genres"), &data.genres),
                frame(
                    "stats-top-moods",
                    t!("stats-top-moods"),
                    mood_note,
                    mood_body,
                ),
                terms("stats-top-tags", t!("stats-top-tags"), &data.tags),
                frame(
                    "stats-clock",
                    t!("stats-clock-title"),
                    clock_note,
                    clock_body,
                ),
                frame(
                    "stats-milestones",
                    t!("stats-milestones-title"),
                    None,
                    self.milestones(stats.milestones()),
                ),
            ]
            .into_iter()
            .flatten(),
        );
        if barren && sections.is_empty() {
            sections.push(ui::vacant(t!("stats-empty"), cx).into_any_element());
        }
        sections
    }

    /// Publishes the scrobbler chart before resolving catalogue links.
    fn chart(
        &self,
        id: &'static str,
        labels: (SharedString, SharedString),
        base: (Option<AnyElement>, Option<AnyElement>),
        alternative: Option<(AnyElement, Option<AnyElement>)>,
        cx: &Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let Some((graph, note)) = alternative else {
            return base;
        };
        let active = self.alternate_charts.contains(id);
        let (body, extra) = if active {
            (Some(graph), note.or(base.1))
        } else {
            base
        };
        let tabs = TabBar::new(id)
            .name(id)
            .items(
                [(false, labels.0), (true, labels.1)].map(|(choice, label)| {
                    stats_tab((id, usize::from(choice)), label, choice == active).on_click(
                        cx.listener(move |this, _, _, cx| {
                            if choice {
                                this.alternate_charts.insert(id);
                            } else {
                                this.alternate_charts.remove(id);
                            }
                            cx.notify();
                        }),
                    )
                }),
            );
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(cx.theme().metrics.pad)
            .child(tabs)
            .children(extra);
        (body, Some(header.into_any_element()))
    }
}

/// A selectable tab shared by the source, date range and chart controls.
fn stats_tab(id: impl Into<ElementId>, label: SharedString, selected: bool) -> Button {
    Button::new(id)
        .label(label)
        .small()
        .ghost()
        .selected(selected)
}

impl Render for StatsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let (source, period, periods) = {
            let stats = self.stats.read(cx);
            (stats.source(), stats.period(), stats.periods(cx))
        };
        let width =
            (Chrome::content(window, cx) - theme.metrics.inset * 2.).max(theme.metrics.cover);
        for rail in &self.rails {
            rail.sync();
        }
        let weak = cx.entity().downgrade();
        let notify: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            weak.update(cx, |_, cx| cx.notify()).ok();
        });
        let grid = CardGrid::layout(width, window);
        let layout = Layout {
            columns: grid.columns.max(1),
            tile: grid.card,
            width,
            window,
            notify: &notify,
        };

        let scrolled = self.scrollbar.read(cx).offset() > theme.metrics.cover;
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.inset)
            .px(theme.metrics.inset)
            .pt(theme.metrics.inset)
            .pb(theme.metrics.inset / 2.)
            .border_b_1()
            .border_color(match scrolled {
                true => theme.border,
                false => theme.border.opacity(0.),
            })
            .child(
                ui::spoken_heading("stats-title", t!("nav-stats"), 1, cx)
                    .text_size(theme.text(Text::Display)),
            )
            .child(self.source_tabs(source, period, cx))
            .when(scrolled, |header| {
                header.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_right()
                        .child(ui::faint(cx).child(self.window_label(cx))),
                )
            });
        let timeline = self.timeline(&periods, period, cx);
        let sections = self.sections(&layout, cx);
        div().flex().flex_col().size_full().child(header).child(
            Scroller::new("stats-page", &self.scrollbar)
                .w_full()
                .flex_1()
                .min_h_0()
                .bottom_inset(Chrome::bottom(cx))
                .p(theme.metrics.inset)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(STACK_GAP)
                        .child(timeline)
                        .children(sections),
                ),
        )
    }
}

/// Finds the busiest five consecutive release years.
pub(super) fn peak_era(years: &[(i32, usize)]) -> Option<(i32, i32)> {
    let first = years.first()?.0;
    let last = years.last()?.0;
    let held = |start: i32| -> usize {
        years
            .iter()
            .filter(|(year, _)| (start..start + ERA).contains(year))
            .map(|(_, count)| *count)
            .sum()
    };
    // Only windows that end by the last year with data, so the era never reaches into the future.
    let start = (first - ERA + 1..=last - ERA + 1)
        .max_by_key(|start| held(*start))
        .filter(|start| held(*start) > 0)?;
    Some((start, start + ERA - 1))
}
