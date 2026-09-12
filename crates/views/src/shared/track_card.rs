use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Entity, MouseDownEvent, SharedString, Window, div};
use music::Track;
use state::{Playback, PlaybackState};
use ui::{ActiveTheme as _, Card, Pinnable, Text, clock, tabular};

use crate::shared::cells;
use crate::shared::pins::Pinned as _;

pub(crate) type ContextHandler = Rc<dyn Fn(usize, &MouseDownEvent, &mut Window, &mut App)>;
pub(crate) type StartHandler = Rc<dyn Fn(usize, &mut App)>;

pub(crate) struct TrackCard {
    id: &'static str,
    place: usize,
    tracks: Rc<Vec<Track>>,
    playback: Entity<Playback>,
    active: Option<String>,
    detailed: bool,
    context: Option<ContextHandler>,
    start: Option<StartHandler>,
}

impl TrackCard {
    pub(crate) fn new(
        id: &'static str,
        place: usize,
        tracks: Rc<Vec<Track>>,
        playback: Entity<Playback>,
        active: Option<&str>,
    ) -> Self {
        Self {
            id,
            place,
            tracks,
            playback,
            active: active.map(str::to_owned),
            detailed: false,
            context: None,
            start: None,
        }
    }

    pub(crate) fn detailed(mut self, detailed: bool) -> Self {
        self.detailed = detailed;
        self
    }

    pub(crate) fn context(mut self, context: Option<ContextHandler>) -> Self {
        self.context = context;
        self
    }

    pub(crate) fn start(mut self, start: Option<StartHandler>) -> Self {
        self.start = start;
        self
    }

    pub(crate) fn render(self, cx: &App) -> Card {
        let theme = *cx.theme();
        let track = &self.tracks[self.place];
        let current = track.id.as_deref() == self.active.as_deref();
        let tint = match current {
            true => theme.primary,
            false => theme.foreground,
        };
        let playing = current && self.playback.read(cx).state() == &PlaybackState::Playing;
        let pin = track.pin();
        let pressed_tracks = self.tracks.clone();
        let pressed_playback = self.playback.clone();
        let pressed_start = self.start.clone();
        let transport_tracks = self.tracks.clone();
        let transport_playback = self.playback.clone();
        let transport_start = self.start.clone();
        let place = self.place;

        let artists = (!self.detailed || featured(track)).then(|| {
            cells::artist_links(
                SharedString::new_static("pick-artist"),
                track.artist_refs.clone(),
                track.artists.clone(),
                theme.muted_foreground,
            )
            .text_size(theme.text(Text::Small))
            .truncate()
        });
        let length = self.detailed.then(|| {
            div()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .font_features(tabular())
                .child(clock(track.duration))
        });

        Card::new((self.id, place), SharedString::from(track.name.clone()))
            .cover(track.cover.clone())
            .tint(tint)
            .when(track.explicit, Card::explicit)
            .when_some(artists, Card::bare_meta)
            .when_some(length, Card::trailing)
            .when_some(self.context, |card, handler| {
                card.menu(move |event, window, cx| handler(place, event, window, cx))
            })
            .play(playing, move |_, _, cx| match current {
                true => transport_playback.update(cx, |playback, cx| playback.toggle_play(cx)),
                false => begin(
                    place,
                    &transport_tracks,
                    &transport_playback,
                    &transport_start,
                    cx,
                ),
            })
            .press(move |_, _, cx| {
                begin(
                    place,
                    &pressed_tracks,
                    &pressed_playback,
                    &pressed_start,
                    cx,
                );
            })
            .when_some(pin, Pinnable::pin)
            .min_w_0()
    }
}

fn featured(track: &Track) -> bool {
    match track.artist_refs.is_empty() {
        false => track.artist_refs.len() > 1,
        true => track.artists.contains(','),
    }
}

fn begin(
    place: usize,
    tracks: &Rc<Vec<Track>>,
    playback: &Entity<Playback>,
    start: &Option<StartHandler>,
    cx: &mut App,
) {
    match start {
        Some(handler) => handler(place, cx),
        None => playback.update(cx, |playback, cx| {
            playback.play_radio(&tracks[place], cx);
        }),
    }
}
