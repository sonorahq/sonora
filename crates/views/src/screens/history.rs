use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Pixels, Render, ScrollHandle, SharedString, Window, div,
};
use i18n::t;
use music::Track;
use state::{History, HistoryState, Playback};
use ui::{
    ActiveTheme as _, Button, Listing as _, Modal, Scrollbar, Scroller, TableDelegate, TableEvent,
    TableState, clock, table, vacant,
};

use crate::chrome::{Searchable, Toolbar, Tooled};
use crate::shared::cells;
use crate::shared::hero::{HeroMetaStrip, PageHero};
use crate::shared::page;
use crate::shared::tracks::{HISTORY_COLUMNS, TrackSource, Tracks, drop_picked};

struct HistoryTracks(Entity<History>);

impl Tracks for HistoryTracks {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track] {
        self.0.read(cx).tracks()
    }

    fn is_loading(&self, cx: &App) -> bool {
        matches!(self.0.read(cx).state(), HistoryState::Loading)
    }
}

pub(crate) struct HistoryView {
    history: Entity<History>,
    playback: Entity<Playback>,
    width: Pixels,
    scrollbar: Entity<Scrollbar>,
    table: Entity<TableState<TrackSource>>,
    toolbar: Entity<Toolbar>,
    clearing: bool,
}

impl HistoryView {
    pub(crate) fn new(
        history: Entity<History>,
        playback: Entity<Playback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let width = cells::content_width(window, Pixels::ZERO, cx);
        let id = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));
        let scroll = scrollbar.read(cx).scroll().clone();
        let table = cx.new(|cx| {
            let menu = cx.new(|_| Scrollbar::inset().watching(id));
            let source = TrackSource::new(
                HISTORY_COLUMNS,
                HistoryTracks(history.clone()),
                playback.clone(),
                menu,
            )
            .with_history(history.clone())
            .table(cx.weak_entity());
            TableState::new(TableDelegate::new(source, width, cx), cx).follow(scroll)
        });

        cx.observe(&history, |this, _, cx| {
            this.table.rebuild(cx);
            cx.notify();
        })
        .detach();
        cx.observe(&playback, |this, _, cx| {
            this.table.refresh(cx);
            cx.notify();
        })
        .detach();
        cx.subscribe(&table, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) => {
                page::play(&this.table, &this.playback, *display, cx);
            }
            TableEvent::Activated(display) => {
                page::play_or_toggle(&this.table, &this.playback, *display, cx);
            }
            TableEvent::Removed => drop_picked(&this.table, cx),
            _ => {}
        })
        .detach();

        let toolbar = Toolbar::searchable(&cx.entity(), cx);
        Self {
            history,
            playback,
            width,
            scrollbar,
            table,
            toolbar,
            clearing: false,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.history.update(cx, |history, cx| history.refresh(cx));
    }

    fn note(&self, cx: &App) -> Option<SharedString> {
        let history = self.history.read(cx);
        match history.state() {
            HistoryState::Loading => None,
            HistoryState::Failed => Some(t!("history-not-loaded")),
            _ if self.table.row_count(cx) > 0 => None,
            _ if self.table.filtering(cx) => Some(t!("library-no-matches")),
            _ => Some(t!("history-empty")),
        }
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let (count, duration) = {
            let history = self.history.read(cx);
            let tracks = history.tracks();
            let duration: std::time::Duration = tracks.iter().map(|track| track.duration).sum();
            (tracks.len(), duration)
        };
        let mut strip = HeroMetaStrip::new().text(t!("count-songs", count = count));
        if !duration.is_zero() {
            strip = strip.text(clock(duration));
        }

        PageHero::new("history-hero", t!("nav-history"))
            .fallback("icons/rotate-ccw-clock.svg")
            .accent()
            .eyebrow(t!("detail-playlist"))
            .meta(strip)
            .actions(
                div().flex().items_center().child(
                    Button::new("clear-history")
                        .outline()
                        .icon("icons/trash-2.svg")
                        .label(t!("history-clear"))
                        .disabled(count == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clearing = true;
                            cx.notify();
                        })),
                ),
            )
            .into_any_element()
    }

    fn confirmation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Modal::new("clear-history", t!("history-clear-title"))
            .detail(t!("history-clear-confirm"))
            .action(
                Button::new("cancel-clear-history")
                    .ghost()
                    .label(t!("common-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
            )
            .action(
                Button::new("apply-clear-history")
                    .danger()
                    .label(t!("common-delete"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.history.update(cx, |history, cx| history.clear(cx));
                        this.dismiss(cx);
                    })),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.dismiss(cx)))
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.clearing = false;
        cx.notify();
    }
}

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.table.claim(cx);
        let inset = cx.theme().metrics.inset;
        let width = cells::content_width(window, Pixels::ZERO, cx);
        if (width - self.width).abs() >= gpui::px(0.5) {
            self.width = width;
            self.table.set_width(width, cx);
        }

        let scroll = self.scrollbar.read(cx).scroll().clone();
        let viewport = page::viewport(&scroll, inset, window);
        self.table
            .update(cx, |table, _| table.set_viewport(viewport));

        let note = self.note(cx);
        let page = Scroller::new("history-page", &self.scrollbar)
            .pt(inset)
            .pb(inset)
            .child(div().px(inset).child(self.header(cx)))
            .child(table(&self.table))
            .when_some(note, |this, note| this.child(vacant(note, cx)));

        div()
            .size_full()
            .child(page)
            .when(self.clearing, |this| this.child(self.confirmation(cx)))
    }
}

impl Searchable for HistoryView {
    fn search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.table.set_query(query, cx);
        cx.notify();
    }

    fn hint() -> SharedString {
        "filter-history".into()
    }
}

impl Tooled for HistoryView {
    fn toolbar(&self) -> Entity<Toolbar> {
        self.toolbar.clone()
    }

    fn tools(&self, _cx: &App) -> Vec<AnyElement> {
        Vec::new()
    }
}
