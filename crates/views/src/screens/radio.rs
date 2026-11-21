use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, Render, ScrollHandle, SharedString, Window, div};
use i18n::t;
use music::Station;
use router::Destination;
use state::{Playback, Session, Sonora, Stations};
use ui::{ActiveTheme as _, Card, Scrollbar, Scroller, snapped, vacant};

use crate::chrome::Chrome;
use crate::shared::album_grid::CardGrid;
use crate::shared::cards::track_status;
use crate::shared::cells;
use crate::shared::hero::{HeroMetaStrip, PageHero};
use crate::shared::page;

/// The page of internet radio stations, one card each, which plays the station when pressed.
pub(crate) struct RadioView {
    stations: Entity<Stations>,
    playback: Entity<Playback>,
    session: Entity<Session>,
    scrollbar: Entity<Scrollbar>,
}

impl RadioView {
    pub(crate) fn new(
        stations: Entity<Stations>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&stations, |_, _, cx| cx.notify()).detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();
        let session = Sonora::global(cx).session.clone();
        cx.observe(&session, |this, _, cx| this.leave_unless_listed(cx))
            .detach();
        let id = cx.entity_id();

        Self {
            stations,
            playback,
            session,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id)),
        }
    }

    /// Asks for the stations, unless they are already in or on their way.
    pub(crate) fn open(&mut self, cx: &mut Context<Self>) {
        self.leave_unless_listed(cx);
        self.stations.update(cx, |stations, cx| stations.load(cx));
    }

    /// Leaves the page for Home while it is open under a provider that lists no stations, as
    /// after switching to one or going back to the page from another. A session still signing
    /// in has no capabilities yet, so it is waited for.
    fn leave_unless_listed(&self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if session.is_pending() || session.capabilities().stations {
            return;
        }
        if router::trail(cx).read(cx).current() == Destination::Radio {
            router::replace(Destination::Home, cx);
        }
    }

    /// Asks the server for its stations again, for Refresh.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.stations.update(cx, |stations, cx| stations.reload(cx));
    }

    pub(crate) fn is_loading(&self, cx: &App) -> bool {
        self.stations.read(cx).is_loading()
    }

    fn header(&self, cx: &App) -> AnyElement {
        let count = self.stations.read(cx).stations().len();

        PageHero::new("radio-hero", t!("nav-radio"))
            .fallback("icons/radio.svg")
            .accent()
            .meta(HeroMetaStrip::new().text(t!("count-stations", count = count)))
            .into_any_element()
    }

    /// What stands in for the cards when there are none to show, or `None` while they load.
    fn note(&self, cx: &App) -> Option<SharedString> {
        let stations = self.stations.read(cx);
        match () {
            _ if stations.is_loading() => None,
            _ if stations.error().is_some() => Some(t!("stations-not-loaded")),
            _ if stations.stations().is_empty() => Some(t!("stations-empty")),
            _ => None,
        }
    }

    fn card(&self, station: &Station, index: usize, count: usize, cx: &App) -> Card {
        let theme = *cx.theme();
        let track = station.track();
        let (current, playing) = track_status(&track, &self.playback, cx);
        let tint = match current {
            true => theme.primary,
            false => theme.foreground,
        };
        let host = station.homepage.as_deref().and_then(host);
        let (toggled, started) = (self.playback.clone(), self.playback.clone());
        let tuned = track.clone();

        Card::new(
            SharedString::from(format!("radio-station-{}", station.id)),
            SharedString::from(station.name.clone()),
        )
        .fallback("icons/radio.svg")
        .tint(tint)
        .hint()
        .when_some(host, |card, host| {
            card.description(host.clone())
                .meta(SharedString::from(host))
        })
        .play(playing, move |_, _, cx| match current {
            true => toggled.update(cx, |playback, cx| playback.toggle_play(cx)),
            false => toggled.update(cx, |playback, cx| {
                playback.start(vec![track.clone()], 0, None, cx)
            }),
        })
        .press(move |_, _, cx| {
            started.update(cx, |playback, cx| {
                playback.start(vec![tuned.clone()], 0, None, cx)
            })
        })
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
    }

    fn cards(&self, window: &Window, cx: &App) -> AnyElement {
        let inset = cx.theme().metrics.inset;
        let room = cells::content_width(window, page::reserved(inset), cx);
        let layout = CardGrid::layout(room);
        let stations = self.stations.read(cx).stations();
        let count = stations.len();
        let rows = stations
            .chunks(layout.columns)
            .enumerate()
            .map(|(row, chunk)| {
                CardGrid::new(room)
                    .children(chunk.iter().enumerate().map(|(column, station)| {
                        self.card(station, row * layout.columns + column, count, cx)
                            .tile(layout.card)
                            .flat()
                            .into_any_element()
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .id("radio-stations")
            .role(Role::List)
            .aria_label(t!("nav-radio"))
            .flex()
            .flex_col()
            .px(inset)
            .gap(snapped(window.rem_size() * 1.5, window))
            .children(rows)
            .into_any_element()
    }
}

impl Render for RadioView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let inset = cx.theme().metrics.inset;
        let note = self.note(cx);

        Scroller::new("radio-page", &self.scrollbar)
            .bottom_inset(Chrome::bottom(cx))
            .px(Chrome::margin(window, cx))
            .pt(inset)
            .pb(inset)
            .child(div().px(inset).pb(inset).child(self.header(cx)))
            .child(self.cards(window, cx))
            .when_some(note, |this, note| {
                this.child(
                    div()
                        .id("radio-empty")
                        .role(Role::Status)
                        .aria_label(note.clone())
                        .child(vacant(note, cx)),
                )
            })
    }
}

/// The host of a station's website without its `www.`, which is all a card has room for.
fn host(homepage: &str) -> Option<String> {
    let rest = homepage
        .split_once("://")
        .map_or(homepage, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next()?.trim();
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_owned())
}
