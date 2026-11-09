//! A track as a list row card: cover, name, artists and, in detail, its length, playing its
//! radio (or whatever the list starts) when pressed. Decks and short track lists use it.

use std::rc::Rc;

use music::Track;
use rune::{El, PointerPress, TextStyle, Tx, text};
use ui::{Button, TABULAR, Text, Weight, clock};

use crate::Ui;
use crate::shared::cards::{self, Fit};

/// A right-click on the card at a place in its list.
pub(crate) type ContextHandler = Rc<dyn Fn(&mut Ui, usize, PointerPress, &mut Tx)>;
/// What pressing the card at a place in its list starts, in place of the track's radio.
pub(crate) type StartHandler = Rc<dyn Fn(&mut Ui, usize)>;
/// Removes the track at a place in its list.
pub(crate) type RemoveHandler = Rc<dyn Fn(&mut Ui, usize)>;

/// A track card at `place` in `tracks`, the list it is listed in.
pub(crate) struct TrackCard<'a> {
    key: &'a str,
    place: usize,
    tracks: &'a [Track],
    detailed: bool,
    context: Option<ContextHandler>,
    start: Option<StartHandler>,
    /// The Fluent key naming the remove button, and what it does with the track's place.
    remove: Option<(&'static str, RemoveHandler)>,
}

impl<'a> TrackCard<'a> {
    /// `key` prefixes the card's key, which ends in the track's place.
    pub(crate) fn new(key: &'a str, place: usize, tracks: &'a [Track]) -> Self {
        Self {
            key,
            place,
            tracks,
            detailed: false,
            context: None,
            start: None,
            remove: None,
        }
    }

    /// Shows the track's length at the end, and its artists only when it has more than one.
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

    /// An X the pointer brings out, which removes the track. `tooltip` is its Fluent key.
    pub(crate) fn remove(mut self, tooltip: &'static str, remove: RemoveHandler) -> Self {
        self.remove = Some((tooltip, remove));
        self
    }

    /// The card, built, or nothing when `place` is past the list.
    pub(crate) fn build(self, ui: &Ui) -> Option<El> {
        let theme = &ui.theme;
        let track = self.tracks.get(self.place)?;
        let (current, playing) = cards::track_status(ui, track);
        let place = self.place;

        let artists = (!self.detailed || featured(track))
            .then(|| cards::track_artists("pick-artist", track, theme));
        let length = self.detailed.then(|| {
            text(clock(track.duration).into_owned())
                .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL).features(TABULAR))
                .color(theme.muted_foreground.color())
                .a11y_hidden()
        });
        let remove = self.remove.map(|(tooltip, remove)| {
            Button::new("track-card-remove")
                .ghost()
                .small()
                .icon("icons/x.svg")
                .tooltip(i18n::lookup(tooltip, None))
                .tint(theme.muted_foreground)
                .on_press(move |ui: &mut Ui| remove(ui, place))
                .build(theme)
        });

        let mut card =
            cards::track_card(ui, format!("{}-{place}", self.key), track, Fit::Row, None);
        if let Some(artists) = artists {
            card = card.bare_meta(artists);
        }
        if let Some(length) = length {
            card = card.trailing(length);
        }
        if let Some(remove) = remove {
            card = card.action(remove);
        }
        if let Some(context) = self.context {
            card = card
                .menu(move |ui: &mut Ui, at: PointerPress, tx: &mut Tx| context(ui, place, at, tx));
        }
        let seed = track.clone();
        let started = self.start.clone();
        let pressed = track.clone();
        let start = self.start;
        let card = card
            .play(playing, move |ui: &mut Ui, _: &mut Tx| match current {
                true => {
                    ui.store.toggle_play();
                    ui.settle();
                }
                false => begin(ui, place, &seed, started.as_ref()),
            })
            .on_press(move |ui: &mut Ui| begin(ui, place, &pressed, start.as_ref()));
        Some(
            card.build(theme)
                .min_w(0.)
                .position_in_set(place + 1, self.tracks.len()),
        )
    }
}

/// Whether a track credits more than one artist, which a detailed card still names.
fn featured(track: &Track) -> bool {
    match track.artist_refs.is_empty() {
        false => track.artist_refs.len() > 1,
        true => track.artists.contains(','),
    }
}

/// Starts what pressing the card at `place` starts: the list's own start, or the track's radio.
fn begin(ui: &mut Ui, place: usize, track: &Track, start: Option<&StartHandler>) {
    match start {
        Some(start) => start(ui, place),
        None => ui.store.play_radio(track),
    }
    ui.settle();
}
