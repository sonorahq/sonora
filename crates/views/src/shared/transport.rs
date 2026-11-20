use std::time::Duration;

use gpui::accesskit::Toggled;
use gpui::prelude::*;
use gpui::{App, Context, Entity, Global, MouseUpEvent, Pixels, Point, SharedString, div};
use i18n::t;
use state::{Playback, Repeat, Sonora};
use ui::{ActiveTheme as _, Button, Scrubber, ScrubberState, Side, clock};

use crate::shared::ambient;

pub(crate) const NOTCH: f32 = state::VOLUME_STEP;
/// The name the playing cover is seen under, so the fullscreen player's cover flies in from the
/// player bar's.
pub(crate) const COVER_FLIGHT: &str = "playing-cover";

/// Whether the playing cover is in the air between the player bar and the fullscreen player,
/// so the player bar leaves its own undrawn until the flight lands.
pub(crate) struct CoverAway(pub(crate) bool);

impl Global for CoverAway {}

/// How much of the opening fullscreen player shows yet, from 0 to 1, which its cover fades in
/// by while it flies above the fade the rest of the player is under.
pub(crate) struct CoverFade(pub(crate) f32);

impl Global for CoverFade {}

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

/// The heart that saves or drops `track`. A click toggles the track `playback` holds by then,
/// which is the one shown, so callers only lend the track to read its state.
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
            if let Some(track) = playback.read(cx).track().cloned() {
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
        .child(skip(playback, false, frosted, cx))
        .child(toggle(playback, big, frosted, cx))
        .child(skip(playback, true, frosted, cx))
        .child(repeat(playback, frosted, cx))
}

pub(crate) fn toggle(playback: &Entity<Playback>, big: bool, frosted: bool, cx: &App) -> Button {
    let held = playback.read(cx);
    let playing = held.wants_playing();
    let idle = held.track().is_none();
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
    let on = playback.read(cx).shuffle(cx);
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
    let repeat = playback.read(cx).repeat();
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

fn skip(playback: &Entity<Playback>, forward: bool, frosted: bool, cx: &App) -> Button {
    let held = playback.read(cx);
    let sections = &held.queue().read(cx).sections;
    let (id, icon, side, tooltip, available, step) = match forward {
        true => (
            "next",
            "icons/skip-forward.svg",
            Side::Right,
            "player-next",
            sections.upcoming > 0,
            Playback::next as fn(&mut Playback, &mut gpui::Context<Playback>),
        ),
        false => (
            "previous",
            "icons/skip-back.svg",
            Side::Left,
            "player-previous",
            sections.current || sections.past > 0,
            Playback::previous as fn(&mut Playback, &mut gpui::Context<Playback>),
        ),
    };
    let enabled = held.steers_device(cx) || available;
    let playback = playback.clone();
    Button::new(id)
        .ghost()
        .when(frosted, Button::frosted)
        .small()
        .icon(icon)
        .belt(side)
        .tooltip_above(tooltip)
        .disabled(!enabled)
        .on_click(move |_, _, cx| playback.update(cx, step))
}

/// Shared drag and hover state for the desktop player's two timelines.
pub(crate) struct SeekControl {
    slider: ScrubberState,
    pending: Option<f32>,
    pub(crate) over: Option<f32>,
}

impl SeekControl {
    pub(crate) fn new(id: &'static str) -> Self {
        Self {
            slider: ScrubberState::new(id),
            pending: None,
            over: None,
        }
    }

    pub(crate) fn dragging(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn travel(&self, pad: Pixels) -> Pixels {
        self.slider.travel(pad)
    }

    pub(crate) fn progress(&self, elapsed: Duration, total: Duration) -> f32 {
        self.pending.unwrap_or_else(|| fraction(elapsed, total))
    }

    /// Returns whether the hover bubble moved enough to redraw.
    pub(crate) fn hover(&mut self, at: Point<Pixels>, pad: Pixels) -> bool {
        let over = self.slider.hovered(at, pad);
        if !moved(self.over, over) {
            return false;
        }
        self.over = over;
        true
    }

    /// Builds a slider and attaches its shared drag behavior through the view's state.
    pub(crate) fn slider<T: 'static>(
        &self,
        playback: &Entity<Playback>,
        (elapsed, total): (Duration, Duration),
        cx: &Context<T>,
        seek: fn(&mut T) -> &mut Self,
    ) -> Scrubber {
        let seekable = playback.read(cx).track().is_some();
        let playback = playback.clone();
        let theme = cx.theme();
        let bubble = self
            .over
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));
        Scrubber::new(&self.slider, self.progress(elapsed, total))
            .name(t!("song-position"))
            .value(clock(elapsed))
            .colors(
                theme.progress_bar,
                theme.muted_foreground.opacity(0.3),
                theme.foreground,
            )
            .ranges(
                Sonora::global(cx).sponsorblock.read(cx).ranges(cx),
                theme.foreground.opacity(0.4),
            )
            .enabled(seekable)
            .when_some(bubble, |this, (at, text)| this.bubble(at, text))
            .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                seek(this).pending = Some(*fraction);
                cx.notify();
            }))
            .on_release(cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if let Some(fraction) = seek(this).pending.take() {
                    playback.update(cx, |playback, cx| playback.seek_fraction(fraction, cx));
                }
            }))
    }
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
