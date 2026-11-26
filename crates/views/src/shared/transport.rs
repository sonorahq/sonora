use std::time::Duration;

use gpui::accesskit::Toggled;
use gpui::prelude::*;
use gpui::{App, Entity, Global, SharedString, div};
use i18n::t;
use state::{Elsewhere, Playback, Repeat, Sonora};
use ui::{ActiveTheme as _, Button, Side, clock};

use crate::shared::ambient;

pub(crate) const NOTCH: f32 = state::VOLUME_STEP;
/// The name the playing cover is seen under, so the fullscreen player's cover flies in from the
/// player bar's.
pub(crate) const COVER_FLIGHT: &str = "playing-cover";

/// Whether the playing cover is in the air between the player bar and the fullscreen player,
/// so the player bar leaves its own undrawn until the flight lands. A narrow player with a panel
/// open keeps the bar's cover in place and sends nothing flying.
pub(crate) struct CoverAway(pub(crate) bool);

impl Global for CoverAway {}

/// How much of the opening fullscreen player shows yet, from 0 to 1, which its cover fades in
/// by while it flies above the fade the rest of the player is under.
pub(crate) struct CoverFade(pub(crate) f32);

impl Global for CoverFade {}

/// How much of its backdrop blurs the opening fullscreen player shows yet, from 0 to 1. The
/// zoom it opens with is a layer that drops every backdrop inside it, so once that lets go they
/// rise over the rest of the zoom rather than snapping on in one frame.
pub(crate) struct Backdrops(pub(crate) f32);

impl Global for Backdrops {}

/// How much room the player bar's cover takes beside the title, from 1 where it shows to 0 where
/// the title has slid over its place. Below `Room::Wide` the fullscreen player's artwork tab
/// tucks it away, since its big cover shows the same art, and the player bar under the player
/// tucks its own along while the player opens and closes, so the two never part.
pub(crate) struct CoverRoom(pub(crate) f32);

impl Global for CoverRoom {}

/// Which way the fullscreen player is moving between the shells, if it is. The player bar's
/// fullscreen button and the player's leave button share one spot, so while the player moves
/// the button of the shell it moves to paints above the fade and the other paints nothing.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ShellSwap {
    #[default]
    Settled,
    Opening,
    Closing,
}

impl Global for ShellSwap {}
const STEP: f32 = 0.004;
/// The width of a seek bar clock in ems, for a track under and over an hour.
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
/// The extra room, in ems, the right clock takes while it leads with a minus sign.
const CLOCK_SIGN: f32 = 0.6;

/// The other device playback is on, which the player shows instead of this app's own.
pub(crate) fn steered(cx: &App) -> Option<Elsewhere> {
    Sonora::global(cx).devices.read(cx).steered()
}

/// How far through `total` `elapsed` is, from 0 to 1.
pub(crate) fn fraction(elapsed: Duration, total: Duration) -> f32 {
    match total.is_zero() {
        true => 0.,
        false => (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.),
    }
}

/// The width of one seek bar clock in ems. It fits a clock for a track of `total`, and adds the
/// minus sign when `time_left` makes the right clock count down.
pub(crate) fn clock_ems(total: Duration, time_left: bool) -> f32 {
    let base = match total.as_secs() >= 3600 {
        true => CLOCK_LONG,
        false => CLOCK_SHORT,
    };
    match time_left {
        true => base + CLOCK_SIGN,
        false => base,
    }
}

/// The clock on the right of a seek bar. The player bar and the fullscreen player both use it,
/// so they count down the same way. With `time_left` it reads the time still to play with a
/// minus sign, and `spoken` holds what a screen reader says for it. Otherwise it reads the
/// track's length and has no spoken form.
pub(crate) struct EndClock {
    pub(crate) text: SharedString,
    pub(crate) spoken: Option<SharedString>,
}

/// The right-hand clock for a track at `elapsed` of `total`. Call it in `render`, since it
/// resolves Fluent strings.
pub(crate) fn end_clock(elapsed: Duration, total: Duration, time_left: bool) -> EndClock {
    match time_left {
        true => {
            let left = clock(total.saturating_sub(elapsed));
            EndClock {
                text: t!("player-time-left", time = &left),
                spoken: Some(t!("player-time-left-spoken", time = &left)),
            }
        }
        false => EndClock {
            text: clock(total),
            spoken: None,
        },
    }
}

pub(crate) fn volume_icon(level: f32) -> &'static str {
    match level {
        level if level <= 0.0001 => "icons/volume-x.svg",
        level if level < 0.25 => "icons/volume.svg",
        level if level < 0.5 => "icons/volume-1.svg",
        _ => "icons/volume-2.svg",
    }
}

pub(crate) fn percent(fraction: f32) -> SharedString {
    t!("unit-percent", value = (fraction * 100.).round())
}

/// The heart that shows whether `track` is saved. A click toggles the track shown at click time.
pub(crate) fn like(track: Option<&music::Track>, playback: &Entity<Playback>, cx: &App) -> Button {
    let theme = *cx.theme();
    let library = Sonora::global(cx).library.clone();
    let playback = playback.clone();
    let id = track.and_then(|track| track.id.as_deref());
    let saved = id.is_some_and(|id| library.read(cx).saved(id));

    Button::new("toggle-liked-track")
        .ghost()
        .backgroundless()
        .small()
        .icon(match saved {
            true => "icons/heart-filled.svg",
            false => "icons/heart.svg",
        })
        .tooltip_above(match saved {
            true => "menu-remove-from-library",
            false => "menu-add-to-library",
        })
        .tint(match saved {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .disabled(id.is_none())
        .on_click(move |_, _, cx| {
            // `track` always returns the local track, which is not the currently played track
            // if remote playback has taken over. Use `shown_track` instead.
            if let Some(track) = playback.read(cx).shown_track(cx) {
                library.update(cx, |library, cx| library.toggle(track, cx));
            }
        })
}

/// `big` is only true in fullscreen, where these buttons float over the ambient
/// background and so blur their hover fills. Without that background, and on
/// the flat player bar, the blur would show nothing.
pub(crate) fn transport(playback: &Entity<Playback>, big: bool, cx: &App) -> impl IntoElement {
    let frosted = big && ambient::shown(cx);

    div()
        .flex()
        .items_center()
        .gap_2()
        .child(shuffle(playback, frosted, cx))
        .child(previous(playback, frosted, cx))
        .child(toggle(playback, big, frosted, cx))
        .child(next(playback, frosted, cx))
        .child(repeat(playback, frosted, cx))
}

pub(crate) fn toggle(playback: &Entity<Playback>, big: bool, frosted: bool, cx: &App) -> Button {
    let held = playback.read(cx);
    let steered = steered(cx);
    let playing = steered
        .as_ref()
        .map_or_else(|| held.wants_playing(), |remote| remote.state.playing);
    let idle = held.track().is_none() && steered.is_none() && !held.can_restart_queue(cx);
    let playback = playback.clone();

    let (icon, tooltip) = match playing {
        true => ("icons/pause.svg", "play-pause"),
        false => ("icons/play.svg", "play-resume"),
    };

    // One id for both states, so the button keeps its press and swaps one icon for the other.
    Button::new("play-pause")
        .ghost()
        .when(frosted, Button::frosted)
        .when(!big, Button::small)
        .icon(icon)
        .tooltip_above(tooltip)
        .disabled(idle)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.toggle_play(cx));
        })
}

fn shuffle(playback: &Entity<Playback>, frosted: bool, cx: &App) -> Button {
    let theme = *cx.theme();
    let on = playback.read(cx).shown_shuffle(cx);
    let playback = playback.clone();

    Button::new("shuffle")
        .ghost()
        .when(frosted, Button::frosted)
        .small()
        .icon("icons/shuffle.svg")
        .indicator(on)
        .aria_toggled(Toggled::from(on))
        .tooltip_above("player-shuffle")
        .tint(match on {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.toggle_shuffle(cx));
        })
}

fn repeat(playback: &Entity<Playback>, frosted: bool, cx: &App) -> Button {
    let theme = *cx.theme();
    let repeat = playback.read(cx).shown_repeat(cx);
    let playback = playback.clone();

    Button::new("repeat")
        .ghost()
        .when(frosted, Button::frosted)
        .small()
        .icon(match repeat {
            Repeat::One => "icons/repeat-one.svg",
            _ => "icons/repeat.svg",
        })
        .blending()
        .indicator(repeat != Repeat::Off)
        .tooltip_above(match repeat {
            Repeat::Off => "player-repeat",
            Repeat::All => "player-repeat-all",
            Repeat::One => "player-repeat-one",
        })
        .tint(match repeat {
            Repeat::Off => theme.muted_foreground,
            _ => theme.primary,
        })
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.cycle_repeat(cx));
        })
}

fn previous(playback: &Entity<Playback>, frosted: bool, cx: &App) -> Button {
    let queue = playback.read(cx).shown_queue(cx);
    let enabled = !queue.editable() || queue.sections.current || queue.sections.past > 0;
    let playback = playback.clone();

    Button::new("previous")
        .ghost()
        .when(frosted, Button::frosted)
        .small()
        .icon("icons/skip-back.svg")
        .belt(Side::Left)
        .tooltip_above("player-previous")
        .disabled(!enabled)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.previous(cx));
        })
}

fn next(playback: &Entity<Playback>, frosted: bool, cx: &App) -> Button {
    let queue = playback.read(cx).shown_queue(cx);
    let enabled = !queue.editable() || queue.sections.upcoming > 0;
    let playback = playback.clone();

    Button::new("next")
        .ghost()
        .when(frosted, Button::frosted)
        .small()
        .icon("icons/skip-forward.svg")
        .belt(Side::Right)
        .tooltip_above("player-next")
        .disabled(!enabled)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.next(cx));
        })
}

pub(crate) fn moved(before: Option<f32>, after: Option<f32>) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => (before - after).abs() > STEP,
        (before, after) => before.is_some() != after.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_width_adds_the_sign_only_when_counting_down() {
        let short = Duration::from_secs(225);
        let long = Duration::from_secs(3600);

        assert!((clock_ems(short, false) - CLOCK_SHORT).abs() < 1e-6);
        assert!((clock_ems(short, true) - (CLOCK_SHORT + CLOCK_SIGN)).abs() < 1e-6);
        assert!((clock_ems(long, false) - CLOCK_LONG).abs() < 1e-6);
        assert!((clock_ems(long, true) - (CLOCK_LONG + CLOCK_SIGN)).abs() < 1e-6);
    }

    #[test]
    fn right_clock_counts_down_the_time_still_to_play() {
        let left = clock(Duration::from_secs(160));
        let end = end_clock(Duration::from_secs(65), Duration::from_secs(225), true);

        assert_eq!(end.text, t!("player-time-left", time = &left));
        assert_eq!(
            end.spoken,
            Some(t!("player-time-left-spoken", time = &left))
        );
    }

    #[test]
    fn right_clock_shows_the_length_when_not_counting_down() {
        let end = end_clock(Duration::from_secs(65), Duration::from_secs(225), false);

        assert_eq!(end.text, clock(Duration::from_secs(225)));
        assert_eq!(end.spoken, None);
    }
}
