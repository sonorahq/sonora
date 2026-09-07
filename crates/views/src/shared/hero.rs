use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, MouseButton, MouseDownEvent, SharedString,
    Window, div, relative,
};
use i18n::t;
use music::Track;
use state::{Origin, Playback, PlaybackState};
use ui::{
    ActiveTheme as _, Artwork, Button, ExplicitBadge, LEADING, Pin, Pinnable as _, TableState,
    Text, upper,
};

use crate::shared::tracks::{self, TrackSource};

pub(crate) fn release_date_label(value: &str) -> SharedString {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 3 {
        return SharedString::from(value.to_owned());
    }
    let month = match parts[1] {
        "01" => t!("month-1"),
        "02" => t!("month-2"),
        "03" => t!("month-3"),
        "04" => t!("month-4"),
        "05" => t!("month-5"),
        "06" => t!("month-6"),
        "07" => t!("month-7"),
        "08" => t!("month-8"),
        "09" => t!("month-9"),
        "10" => t!("month-10"),
        "11" => t!("month-11"),
        "12" => t!("month-12"),
        _ => return SharedString::from(value.to_owned()),
    };
    let day = parts[2].trim_start_matches('0');

    t!("date-full", month = &month, day = day, year = parts[0])
}

#[derive(IntoElement, Default)]
pub(crate) struct HeroMetaStrip {
    items: Vec<AnyElement>,
}

impl HeroMetaStrip {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.items
            .push(div().flex_none().child(text.into()).into_any_element());
        self
    }

    pub(crate) fn item(mut self, item: impl IntoElement) -> Self {
        self.items.push(item.into_any_element());
        self
    }
}

impl RenderOnce for HeroMetaStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let mut strip = div()
            .flex()
            .flex_wrap()
            .items_center()
            .min_w_0()
            .gap_1()
            .text_size(theme.text(Text::Small))
            .text_color(theme.muted_foreground);

        for (index, item) in self.items.into_iter().enumerate() {
            strip = strip.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .when(index > 0, |this| this.child("•"))
                    .child(item),
            );
        }

        strip
    }
}

enum Listing {
    Owned(Vec<Track>),
    Listed(Entity<TableState<TrackSource>>),
}

impl Listing {
    fn first(&self, cx: &App) -> Option<usize> {
        match self {
            Self::Owned(tracks) => tracks.iter().position(|track| track.playable),
            Self::Listed(table) => tracks::first_playable(table, cx),
        }
    }

    fn holds(&self, id: &str, cx: &App) -> bool {
        match self {
            Self::Owned(tracks) => tracks.iter().any(|track| track.id.as_deref() == Some(id)),
            Self::Listed(table) => tracks::holds(table, id, cx),
        }
    }

    fn queue(&self, cx: &App) -> Vec<Track> {
        match self {
            Self::Owned(tracks) => tracks.clone(),
            Self::Listed(table) => tracks::ordered(table, cx),
        }
    }

    fn whence(&self, cx: &App) -> Option<Origin> {
        match self {
            Self::Owned(_) => None,
            Self::Listed(table) => tracks::whence(table, cx),
        }
    }
}

#[derive(IntoElement)]
pub(crate) struct HeroPlayButton {
    id: ElementId,
    label: SharedString,
    listing: Listing,
    from: Option<Origin>,
    playback: Entity<Playback>,
}

impl HeroPlayButton {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        tracks: Vec<Track>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            listing: Listing::Owned(tracks),
            from: None,
            playback,
        }
    }

    pub(crate) fn from(mut self, origin: Option<Origin>) -> Self {
        self.from = origin;
        self
    }

    pub(crate) fn listed(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        table: &Entity<TableState<TrackSource>>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            listing: Listing::Listed(table.clone()),
            from: None,
            playback,
        }
    }
}

impl RenderOnce for HeroPlayButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = {
            let playback = self.playback.read(cx);
            let current = playback.track().and_then(|track| track.id.as_deref());
            current
                .filter(|current| self.listing.holds(current, cx))
                .map(|_| playback.state().clone())
        };
        let (label, icon, blocked) = match &state {
            Some(PlaybackState::Playing) => (t!("play-pause"), "icons/pause.svg", false),
            Some(PlaybackState::Paused) => (t!("play-resume"), "icons/play.svg", false),
            Some(PlaybackState::Loading) => (t!("play-loading"), "icons/play.svg", true),
            _ => (self.label, "icons/play.svg", false),
        };
        let disabled = self.listing.first(cx).is_none() || blocked;
        let listing = self.listing;
        let from = self.from;
        let playback = self.playback;

        div().flex().child(
            Button::new(self.id)
                .label(label)
                .icon(icon)
                .primary()
                .disabled(disabled)
                .on_click(move |_, _, cx| {
                    playback.update(cx, |playback, cx| match &state {
                        Some(PlaybackState::Playing) => playback.pause(cx),
                        Some(PlaybackState::Paused) => playback.resume(cx),
                        Some(PlaybackState::Loading) => {}
                        _ => {
                            let queued = listing.queue(cx);
                            let from = from.clone().or_else(|| listing.whence(cx));
                            playback.start_any(queued, from, cx)
                        }
                    });
                }),
        )
    }
}

type DragStart = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub(crate) struct PageHero {
    id: ElementId,
    title: SharedString,
    cover: Option<String>,
    fallback: Option<SharedString>,
    accent: bool,
    eyebrow: Option<SharedString>,
    meta: Option<AnyElement>,
    actions: Option<AnyElement>,
    circle: bool,
    explicit: bool,
    drag_start: Option<DragStart>,
    pin: Option<Pin>,
}

impl PageHero {
    pub(crate) fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            cover: None,
            fallback: None,
            accent: false,
            eyebrow: None,
            meta: None,
            actions: None,
            circle: false,
            explicit: false,
            drag_start: None,
            pin: None,
        }
    }

    pub(crate) fn pin(mut self, pin: Option<Pin>) -> Self {
        self.pin = pin;
        self
    }

    pub(crate) fn drag_start(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.drag_start = Some(Box::new(handler));
        self
    }

    pub(crate) fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    pub(crate) fn fallback(mut self, icon: impl Into<SharedString>) -> Self {
        self.fallback = Some(icon.into());
        self
    }

    pub(crate) fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    pub(crate) fn eyebrow(mut self, eyebrow: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    pub(crate) fn meta(mut self, meta: impl IntoElement) -> Self {
        self.meta = Some(meta.into_any_element());
        self
    }

    pub(crate) fn actions(mut self, actions: impl IntoElement) -> Self {
        self.actions = Some(actions.into_any_element());
        self
    }

    pub(crate) fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub(crate) fn explicit(mut self, explicit: bool) -> Self {
        self.explicit = explicit;
        self
    }
}

impl RenderOnce for PageHero {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();

        let art = theme.metrics.cover;
        let drag_start = self.drag_start.map(Rc::from);
        let starter = drag_start.clone();

        div()
            .id(self.id)
            .when_some(self.pin, |hero, pin| hero.pin(pin))
            .flex()
            .flex_none()
            .items_end()
            .gap_5()
            .pb_6()
            .child(
                div()
                    .flex_none()
                    .when_some(starter, |this, drag_start: Rc<DragStart>| {
                        this.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                            drag_start(event, window, cx)
                        })
                    })
                    .child(
                        Artwork::new(self.cover)
                            .size(art)
                            .corner_radius(theme.radius * 1.5)
                            .when(self.circle, Artwork::circle)
                            .when_some(self.fallback, Artwork::fallback)
                            .when(self.accent, Artwork::accent),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h(art)
                    .justify_end()
                    .gap_2()
                    .line_height(relative(LEADING))
                    .children(self.eyebrow.map(|eyebrow| eyebrow_label(eyebrow, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .min_w_0()
                            .text_size(theme.text(Text::Display))
                            .font_weight(FontWeight::BOLD)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .when_some(drag_start, |this, drag_start: Rc<DragStart>| {
                                        this.on_mouse_down(
                                            MouseButton::Right,
                                            move |event, window, cx| drag_start(event, window, cx),
                                        )
                                    })
                                    .child(self.title),
                            )
                            .when(self.explicit, |this| {
                                this.child(div().flex_none().child(ExplicitBadge::new()))
                            }),
                    )
                    .children(self.meta)
                    .children(self.actions.map(|actions| div().pt_1().child(actions))),
            )
    }
}

fn eyebrow_label(label: SharedString, cx: &App) -> Div {
    let theme = cx.theme();

    div()
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .font_weight(FontWeight::SEMIBOLD)
        .child(upper(label))
}
