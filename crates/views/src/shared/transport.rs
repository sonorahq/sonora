use gpui::prelude::*;
use gpui::{App, Entity, SharedString, div};
use i18n::t;
use state::{Playback, PlaybackState, Queue, Repeat, Sonora};
use ui::{ActiveTheme as _, Button};

pub(crate) const NOTCH: f32 = 0.05;
const STEP: f32 = 0.004;

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

pub(crate) fn like(track: Option<music::Track>, cx: &App) -> Button {
    let theme = *cx.theme();
    let library = Sonora::global(cx).library.clone();
    let id = track.as_ref().and_then(|track| track.id.as_deref());
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
            if let Some(track) = track.clone() {
                library.update(cx, |library, cx| library.toggle(track, cx));
            }
        })
}

pub(crate) fn transport(
    playback: &Entity<Playback>,
    queue: &Entity<Queue>,
    big: bool,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(shuffle(queue, cx))
        .child(previous(playback, cx))
        .child(seek_back(playback, cx))
        .child(toggle(playback, big, cx))
        .child(seek_forward(playback, cx))
        .child(next(playback, queue, cx))
        .child(repeat(playback, cx))
}

pub(crate) fn toggle(playback: &Entity<Playback>, big: bool, cx: &App) -> Button {
    let held = playback.read(cx);
    let playing = matches!(held.state(), PlaybackState::Playing);
    let idle = held.track().is_none();
    let playback = playback.clone();

    let (id, icon, tooltip) = match playing {
        true => ("pause", "icons/pause.svg", "play-pause"),
        false => ("play", "icons/play.svg", "play-resume"),
    };

    Button::new(id)
        .ghost()
        .when(!big, Button::small)
        .icon(icon)
        .tooltip_above(tooltip)
        .disabled(idle)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.toggle_play(cx));
        })
}

fn shuffle(queue: &Entity<Queue>, cx: &App) -> Button {
    let theme = *cx.theme();
    let on = queue.read(cx).shuffle();
    let queue = queue.clone();

    Button::new("shuffle")
        .ghost()
        .small()
        .icon("icons/shuffle.svg")
        .tooltip_above("player-shuffle")
        .tint(match on {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .on_click(move |_, _, cx| {
            queue.update(cx, |queue, cx| queue.toggle_shuffle(cx));
        })
}

fn repeat(playback: &Entity<Playback>, cx: &App) -> Button {
    let theme = *cx.theme();
    let repeat = playback.read(cx).repeat();
    let playback = playback.clone();

    Button::new("repeat")
        .ghost()
        .small()
        .icon(match repeat {
            Repeat::One => "icons/repeat-one.svg",
            _ => "icons/repeat.svg",
        })
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

fn seek_back(playback: &Entity<Playback>, cx: &App) -> Button {
    let idle = playback.read(cx).track().is_none();
    let playback = playback.clone();

    Button::new("seek-back")
        .ghost()
        .small()
        .icon("icons/rotate-ccw.svg")
        .tooltip_above("player-seek-back")
        .disabled(idle)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.seek_back(cx));
        })
}

fn seek_forward(playback: &Entity<Playback>, cx: &App) -> Button {
    let idle = playback.read(cx).track().is_none();
    let playback = playback.clone();

    Button::new("seek-forward")
        .ghost()
        .small()
        .icon("icons/rotate-cw.svg")
        .tooltip_above("player-seek-forward")
        .disabled(idle)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.seek_forward(cx));
        })
}

fn previous(playback: &Entity<Playback>, cx: &App) -> Button {
    let enabled = playback.read(cx).has_previous(cx);
    let playback = playback.clone();

    Button::new("previous")
        .ghost()
        .small()
        .icon("icons/skip-back.svg")
        .tooltip_above("player-previous")
        .disabled(!enabled)
        .on_click(move |_, _, cx| {
            playback.update(cx, |playback, cx| playback.previous(cx));
        })
}

fn next(playback: &Entity<Playback>, queue: &Entity<Queue>, cx: &App) -> Button {
    let enabled = queue.read(cx).has_next();
    let playback = playback.clone();

    Button::new("next")
        .ghost()
        .small()
        .icon("icons/skip-forward.svg")
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
