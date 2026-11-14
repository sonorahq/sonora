use std::time::Duration;

use gpui::accesskit::Toggled;
use gpui::prelude::*;
use gpui::{App, Entity, Global, SharedString, div};
use i18n::t;
use state::{Elsewhere, Playback, Repeat, Sonora};
use ui::{ActiveTheme as _, Button, Side};

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

pub(crate) fn volume_icon(level: f32) -> &'static str {
    match level {
        level if level <= 0.0001 => "icons/volume-x.svg",
        level if level < 0.25 => "icons/volume.svg",
        level if level < 0.5 => "icons/volume-1.svg",
        _ => "icons/volume-2.svg",
    }
}

pub(crate) fn percent(fraction: f32) -> SharedString {
    t!("player-percent", value = (fraction * 100.).round())
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
    let idle = held.track().is_none() && steered.is_none();
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
