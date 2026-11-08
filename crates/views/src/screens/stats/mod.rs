mod charts;
mod insights;
mod rankings;
mod timeline;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Render, SharedString, Window, div};
use i18n::t;
use music::StatsPeriod;
use state::{Playback, Sonora, Stats, StatsSource};
use ui::{ActiveTheme as _, Button, Input, RangeState, Scrollbar, Scroller, TabBar, Text};

use crate::chrome::Chrome;
use crate::shared::album_grid::CardGrid;
use crate::shared::shelves::{Rail, STACK_GAP, section};
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
    width: gpui::Pixels,
    window: &'a Window,
    notify: &'a Rc<dyn Fn(&mut App)>,
}

/// Personal rankings from the provider, a linked scrobbler or the plays Sonora recorded, over
/// the window or the dates the listener picks.
pub(crate) struct StatsView {
    stats: Entity<Stats>,
    playback: Entity<Playback>,
    scrollbar: Entity<Scrollbar>,
    range: RangeState,
    start: Entity<Input>,
    end: Entity<Input>,
    invalid: bool,
    rails: Vec<Rail>,
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
            rails: Kind::ALL.iter().map(|_| Rail::new(host)).collect(),
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
        self.rewind();
        self.stats
            .update(cx, |stats, cx| stats.select(source, period, cx));
        cx.notify();
    }

    /// The tab row that picks where the rankings come from.
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
                    StatsSource::Scrobbler("lastfm") => ("lastfm", t!("settings-lastfm")),
                    StatsSource::Scrobbler("listenbrainz") => {
                        ("listenbrainz", t!("settings-listenbrainz"))
                    }
                    StatsSource::Scrobbler(id) => (id, SharedString::from(id)),
                };
                Button::new(SharedString::from(format!("stats-source-{id}")))
                    .label(label)
                    .small()
                    .ghost()
                    .selected(candidate == source)
                    .on_click(cx.listener(move |this, _, _, cx| this.select(candidate, period, cx)))
            }))
            .into_any_element()
    }

    /// What replaces the rankings when the source failed to answer.
    fn failure(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(cx.theme().metrics.pad)
            .child(ui::spoken("stats-failed", t!("stats-failed")))
            .child(
                Button::new("stats-retry")
                    .label(t!("trouble-retry"))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .into_any_element()
    }

    /// The overview and the rankings of the selected source, or whatever stands in for them
    /// while they load.
    fn sections(&self, layout: &Layout<'_>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let stats = self.stats.read(cx);
        if stats.failed() {
            return vec![self.failure(cx)];
        }
        if stats.is_loading() {
            let tile = CardGrid::layout(layout.width).card;
            return Kind::MAIN
                .iter()
                .map(|kind| {
                    Rail::pending(kind.title(), tile, layout.columns.max(1), layout.window, cx)
                })
                .collect();
        }

        let data = stats.data();
        let genres = data.genres.clone();
        // Moods the app names itself are Fluent keys, and the provider's own are plain words.
        let tags = data.tags.clone();
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
        let years = data.years.clone();
        let tempos = data.tempos.clone();
        let milestones = data.milestones.clone();
        let traits = data.traits();
        let times = data.overview.clone();
        let barren = data.tracks.is_empty() && data.artists.is_empty() && data.albums.is_empty();
        let rails: Vec<_> = Kind::ALL
            .iter()
            .filter_map(|kind| self.rail(*kind, layout, cx))
            .collect();
        let years_note = self.badges(
            [
                peak_era(&years)
                    .map(|(from, to)| (t!("stats-badge-era", from = from, to = to), false)),
                times
                    .recent_share
                    .map(|share| (t!("stats-badge-recent", share = share), true)),
            ],
            cx,
        );
        let clock = self.clock(&times, cx);
        // The page reads from how much was listened to, to what the listening was like, to
        // when it happened.
        let mut sections: Vec<AnyElement> = Vec::new();
        let style = self.style(traits, layout.width, cx);
        sections.extend(style.map(|body| {
            section(
                "stats-style",
                t!("stats-style-title"),
                None,
                body,
                layout.window,
                cx,
            )
        }));
        sections.extend(rails);
        let tempo_note = self.badges(
            [times
                .avg_bpm
                .map(|bpm| (t!("stats-badge-bpm", bpm = bpm), false))],
            cx,
        );
        let after = [
            (
                "stats-years",
                t!("stats-years-title"),
                years_note,
                self.years(&years, cx),
            ),
            (
                "stats-tempo",
                t!("stats-tempo-title"),
                tempo_note,
                self.tempo(&tempos, cx),
            ),
        ];
        for (tag, title, note, body) in after {
            sections.extend(body.map(|body| section(tag, title, note, body, layout.window, cx)));
        }
        for (tag, title, terms) in [
            ("stats-top-genres", t!("stats-top-genres"), genres),
            ("stats-top-moods", t!("stats-top-moods"), moods),
            ("stats-top-tags", t!("stats-top-tags"), tags),
        ] {
            if !terms.is_empty() {
                let body = self.distribution(tag, title.clone(), &terms, layout.width, cx);
                sections.push(section(tag, title, None, body, layout.window, cx));
            }
        }
        sections.extend(clock.map(|(grid, note)| {
            section(
                "stats-clock",
                t!("stats-clock-title"),
                note,
                grid,
                layout.window,
                cx,
            )
        }));
        sections.extend(self.milestones(&milestones, cx).map(|body| {
            section(
                "stats-milestones",
                t!("stats-milestones-title"),
                None,
                body,
                layout.window,
                cx,
            )
        }));
        if barren && sections.is_empty() {
            sections.push(ui::vacant(t!("stats-empty"), cx).into_any_element());
        }
        sections
    }
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
        let layout = Layout {
            columns: CardGrid::layout(width).columns,
            width,
            window,
            notify: &notify,
        };

        // The header stays above the page. Once the page has scrolled the timeline away, it also
        // names the window the page is showing.
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
        let timeline = self.timeline(source, &periods, period, cx);
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

/// The five consecutive release years that hold the most of the songs, as the first and last of
/// them, which is the era the listener's songs mostly come from.
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
    let start = (first..=last.max(first))
        .max_by_key(|start| held(*start))
        .filter(|start| held(*start) > 0)?;
    Some((start, start + ERA - 1))
}
