use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, ElementId, Entity, MouseDownEvent, SharedString, Window, div};
use music::Track;
use state::{Library, Playback};
use ui::{ActiveTheme as _, Button, Card, Text, clock, tabular};

use crate::shared::cards;

pub(crate) type ContextHandler = Rc<dyn Fn(usize, &MouseDownEvent, &mut Window, &mut App)>;
pub(crate) type StartHandler = Rc<dyn Fn(usize, &mut App)>;

pub(crate) struct TrackCard {
    id: ElementId,
    favorites: Option<Entity<Library>>,
    place: usize,
    tracks: Rc<Vec<Track>>,
    playback: Entity<Playback>,
    detailed: bool,
    context: Option<ContextHandler>,
    start: Option<StartHandler>,
}

impl TrackCard {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        place: usize,
        tracks: Rc<Vec<Track>>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            id: id.into(),
            favorites: None,
            place,
            tracks,
            playback,
            detailed: false,
            context: None,
            start: None,
        }
    }

    /// Adds a visible heart that toggles the track's favorite state.
    pub(crate) fn with_favorites(mut self, library: Entity<Library>) -> Self {
        self.favorites = Some(library);
        self
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
        let (current, playing) = cards::track_status(track, &self.playback, cx);
        let pressed_tracks = self.tracks.clone();
        let pressed_playback = self.playback.clone();
        let pressed_start = self.start.clone();
        let transport_tracks = self.tracks.clone();
        let transport_playback = self.playback.clone();
        let transport_start = self.start.clone();
        let place = self.place;

        let artists = (!self.detailed || featured(track))
            .then(|| cards::track_artists(SharedString::new_static("pick-artist"), track, &theme));
        let length = self.detailed.then(|| {
            div()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .font_features(tabular())
                .child(clock(track.duration))
        });

        let favorite = self.favorites.and_then(|library| {
            let id = track.id.as_deref()?;
            let saved = library.read(cx).saved(id);
            let pending = library.read(cx).pending(id);
            let track = track.clone();
            Some(
                Button::new("track-favorite")
                    .small()
                    .ghost()
                    .backgroundless()
                    .icon(match saved {
                        true => "icons/heart-filled.svg",
                        false => "icons/heart.svg",
                    })
                    .tooltip(match saved {
                        true => "menu-remove-from-library",
                        false => "menu-add-to-library",
                    })
                    .tint(match saved {
                        true => theme.primary,
                        false => theme.muted_foreground,
                    })
                    .aria_toggled(saved.into())
                    .disabled(pending)
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        library.update(cx, |library, cx| library.toggle(track.clone(), cx));
                    }),
            )
        });
        let trailing = (length.is_some() || favorite.is_some()).then(|| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(length)
                .children(favorite)
        });
        let total = self.tracks.len();

        cards::track_card(self.id, track, &self.playback, cx)
            .aria_position_in_set(place + 1)
            .aria_size_of_set(total)
            .when_some(artists, Card::bare_meta)
            .when_some(trailing, Card::trailing)
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
