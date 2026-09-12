use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Pixels, Render, ScrollHandle, SharedString,
    WeakEntity, Window, div, px,
};
use i18n::t;
use state::{AppSettings, GenreDetails, Playback, Sonora};
use ui::{ActiveTheme as _, Mode, Popovers, Scrollbar, Scroller, Skeleton, Text, vacant};

use crate::chrome::{Chrome, Toolbar, Tooled, tools};
use crate::shared::cells;
use crate::shared::shelves::Shelves;

const TILE: Pixels = px(220.);
const STEADY: Pixels = px(0.5);
const SECTION: &str = "genre";

pub(crate) struct GenreView {
    detail: Entity<GenreDetails>,
    settings: Entity<AppSettings>,
    shelves: Entity<Shelves>,
    scrollbar: Entity<Scrollbar>,
    toolbar: Entity<Toolbar>,
    popovers: Popovers,
    mode: Mode,
    width: Pixels,
    me: WeakEntity<Self>,
}

impl GenreView {
    pub(crate) fn new(
        detail: Entity<GenreDetails>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let mode = settings.read(cx).view_or(SECTION, Mode::Grid);
        let id = cx.entity_id();
        let shelves = cx.new(|_| Shelves::new("genre-shelf", id, playback.clone()));

        cx.observe(&detail, |this, _, cx| {
            this.shelves.update(cx, |shelves, _| shelves.reset());
            cx.notify();
        })
        .detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        cx.observe(&shelves, |_, _, cx| cx.notify()).detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        let me = cx.entity();
        let toolbar = Toolbar::tooled(&me, cx);

        Self {
            detail,
            settings,
            shelves,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id)),
            toolbar,
            popovers: Popovers::default(),
            mode,
            width: Pixels::ZERO,
            me: me.downgrade(),
        }
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.settings
            .update(cx, |settings, cx| settings.set_view(SECTION, mode, cx));
        cx.notify();
    }
}

impl Tooled for GenreView {
    fn toolbar(&self) -> Entity<Toolbar> {
        self.toolbar.clone()
    }

    fn tools(&self, _cx: &App) -> Vec<AnyElement> {
        let viewed = self.me.clone();

        vec![tools::views(&self.popovers, self.mode, move |mode, cx| {
            viewed.update(cx, |view, cx| view.set_mode(mode, cx)).ok();
        })]
    }
}

impl Render for GenreView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let pad = theme.metrics.inset;
        let room = cells::content_width(window, pad * 2., cx);
        if (room - self.width).abs() >= STEADY {
            self.width = room;
        }

        let detail = self.detail.read(cx);
        let loading = detail.is_loading();
        let title = detail.name().unwrap_or_default().to_owned();
        let error = detail.error().map(str::to_owned);
        let sections = detail.sections();
        let empty = !loading && sections.is_empty();
        let (mode, width) = (self.mode, self.width);
        let shelves = self.shelves.update(cx, |shelves, cx| {
            shelves.render(sections, mode, width, window, cx)
        });

        div().flex().flex_col().size_full().child(
            Scroller::new("genre", &self.scrollbar).p(pad).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .child(
                        div()
                            .text_size(theme.text(Text::Display))
                            .font_weight(FontWeight::BOLD)
                            .child(SharedString::from(title)),
                    )
                    .when(loading, |this| this.child(Skeleton::new().w_full().h(TILE)))
                    .children(error.map(|error| {
                        div()
                            .text_color(theme.danger)
                            .child(SharedString::from(error))
                    }))
                    .when(empty, |this| this.child(vacant(t!("genre-empty"), cx)))
                    .child(shelves),
            ),
        )
    }
}
