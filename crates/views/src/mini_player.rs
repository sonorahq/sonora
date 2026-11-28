//! Floating mini player (desktop widget).
//!
//! This view only renders playback state and forwards user intent:
//! - playback commands go straight to `state::Playback`;
//! - window intents (close, restore main window) are dispatched as actions
//!   and handled by `crates/sonora`, which owns every window.
//!   `views` must never depend on `sonora`.

use std::time::Duration;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FocusHandle, FontWeight, MouseButton, MouseUpEvent, Pixels, Render,
    SharedString, Subscription, Task, Window, WindowControlArea, div, px,
};
use i18n::t;
use input::{CloseMiniPlayer, ExpandMiniPlayer, MINI_PLAYER_CONTEXT};
use state::{
    AppSettings, Lyrics, LyricsState, MiniPlayerLyricsMode, MiniPlayerLyricsSize,
    MiniPlayerOrientation, MiniPlayerPrefs, MiniPlayerSize, Playback, Repeat, RomanizationScripts,
    Sonora,
};
use ui::{ActiveTheme as _, Artwork, Button, Scrubber, ScrubberState, Theme, clock, tabular};

use crate::chrome::{VerseLook, verse};
use crate::shared::ambient::Ambient;
use crate::shared::transport::{self, end_clock, fraction, steered};
use crate::shared::volume::{RisingVolume, VolumeHost, rising_volume_in};

const RADIUS_SUBTLE: f32 = 8.;
const RADIUS_ROUNDED: f32 = 16.;
const RADIUS_ROUND: f32 = 28.;
const CLOCK_SHORT_EMS: f32 = 3.2;
const CLOCK_LONG_EMS: f32 = 5.0;
const CLOCK_SIGN_EMS: f32 = 0.6;
const ARTWORK_RADIUS_RATIO: f32 = 0.6;
const PROGRESS_REFRESH: Duration = Duration::from_millis(250);
const KARAOKE_REFRESH: Duration = Duration::from_millis(40);
const FULLSCREEN_VISIBLE_VERSES: usize = 6;
const FULLSCREEN_PAST_VERSES: usize = 2;

/// Logical geometry for the mini player at a chosen orientation, preset, and lyrics mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiniPlayerDimensions {
    pub width: f32,
    pub height: f32,
    pub padding: f32,
    pub gap: f32,
    pub artwork: f32,
    pub play_button: f32,
    pub lyrics_height: f32,
    pub left_column_width: f32,
    pub volume_rise: f32,
}

/// Computes the window and internal layout dimensions for `prefs`.
pub fn mini_player_dimensions(prefs: MiniPlayerPrefs) -> MiniPlayerDimensions {
    match (prefs.orientation, prefs.lyrics_mode) {
        (
            MiniPlayerOrientation::Landscape,
            MiniPlayerLyricsMode::Off | MiniPlayerLyricsMode::Bar,
        ) => {
            let (width, base_height, padding, gap, play_button, lyrics_height, volume_rise) =
                match prefs.size {
                    MiniPlayerSize::Compact => (320., 112., 10., 10., 30., 28., 44.),
                    MiniPlayerSize::Standard => (364., 124., 12., 12., 34., 32., 56.),
                    MiniPlayerSize::Large => (428., 144., 14., 14., 38., 36., 74.),
                };
            let has_bar = prefs.lyrics_mode == MiniPlayerLyricsMode::Bar;
            let top_bottom_pad = match has_bar {
                true => padding * 1.5,
                false => padding * 2.,
            };
            MiniPlayerDimensions {
                width,
                height: base_height + if has_bar { lyrics_height } else { 0. },
                padding,
                gap,
                artwork: base_height - top_bottom_pad,
                play_button,
                lyrics_height,
                left_column_width: 0.,
                volume_rise,
            }
        }
        (
            MiniPlayerOrientation::Portrait,
            MiniPlayerLyricsMode::Off | MiniPlayerLyricsMode::Bar,
        ) => {
            let (width, base_height, padding, gap, artwork, play_button, lyrics_height) =
                match prefs.size {
                    MiniPlayerSize::Compact => (232., 306., 12., 8., 136., 32., 28.),
                    MiniPlayerSize::Standard => (268., 346., 14., 10., 164., 36., 32.),
                    MiniPlayerSize::Large => (312., 400., 16., 12., 200., 40., 36.),
                };
            let has_bar = prefs.lyrics_mode == MiniPlayerLyricsMode::Bar;
            MiniPlayerDimensions {
                width,
                height: base_height + if has_bar { lyrics_height } else { 0. },
                padding,
                gap,
                artwork,
                play_button,
                lyrics_height,
                left_column_width: 0.,
                volume_rise: 108.,
            }
        }
        (MiniPlayerOrientation::Landscape, MiniPlayerLyricsMode::Fullscreen) => {
            let (width, height, padding, gap, artwork, play_button, left_column_width) =
                match prefs.size {
                    MiniPlayerSize::Compact => (480., 270., 14., 14., 124., 30., 208.),
                    MiniPlayerSize::Standard => (560., 320., 16., 18., 156., 34., 242.),
                    MiniPlayerSize::Large => (680., 380., 20., 22., 196., 38., 290.),
                };
            MiniPlayerDimensions {
                width,
                height,
                padding,
                gap,
                artwork,
                play_button,
                lyrics_height: 0.,
                left_column_width,
                volume_rise: 108.,
            }
        }
        (MiniPlayerOrientation::Portrait, MiniPlayerLyricsMode::Fullscreen) => {
            let (width, height, padding, gap, artwork, play_button) = match prefs.size {
                MiniPlayerSize::Compact => (272., 424., 12., 8., 96., 30.),
                MiniPlayerSize::Standard => (316., 486., 14., 10., 120., 34.),
                MiniPlayerSize::Large => (372., 556., 16., 12., 148., 38.),
            };
            MiniPlayerDimensions {
                width,
                height,
                padding,
                gap,
                artwork,
                play_button,
                lyrics_height: 0.,
                left_column_width: 0.,
                volume_rise: 108.,
            }
        }
    }
}

/// Maps a [`ui::Rounding`] preset to the mini player card's corner radius.
pub fn corner_radius(rounding: ui::Rounding) -> Pixels {
    match rounding {
        ui::Rounding::Square => px(0.),
        ui::Rounding::Subtle => px(RADIUS_SUBTLE),
        ui::Rounding::Rounded => px(RADIUS_ROUNDED),
        ui::Rounding::Round => px(RADIUS_ROUND),
    }
}

fn lyrics_scale(size: MiniPlayerLyricsSize) -> f32 {
    match size {
        MiniPlayerLyricsSize::Small => 0.85,
        MiniPlayerLyricsSize::Medium => 1.0,
        MiniPlayerLyricsSize::Large => 1.2,
    }
}

fn fullscreen_verse_sizes(prefs: MiniPlayerPrefs) -> (Pixels, Pixels) {
    let base = match (prefs.orientation, prefs.size) {
        (MiniPlayerOrientation::Portrait, MiniPlayerSize::Compact) => 13.,
        (MiniPlayerOrientation::Portrait, MiniPlayerSize::Standard) => 15.,
        (MiniPlayerOrientation::Portrait, MiniPlayerSize::Large) => 17.,
        (MiniPlayerOrientation::Landscape, MiniPlayerSize::Compact) => 14.,
        (MiniPlayerOrientation::Landscape, MiniPlayerSize::Standard) => 16.,
        (MiniPlayerOrientation::Landscape, MiniPlayerSize::Large) => 19.,
    };
    let verse_size = px(base * lyrics_scale(prefs.lyrics_size));
    (verse_size, verse_size * 0.72)
}

/// Snapshot of what the widget displays, taken once per frame.
#[derive(Clone, Debug, Default)]
struct NowPlaying {
    has_track: bool,
    title: SharedString,
    artist: SharedString,
    artwork: Option<SharedString>,
    position: Duration,
    duration: Duration,
    is_playing: bool,
    can_play: bool,
}

impl NowPlaying {
    fn read(playback: &Playback, cx: &App) -> Self {
        let remote = steered(cx);
        let track = playback.shown_track(cx);
        let (position, duration) = playback.shown_time(cx);
        let is_playing = remote
            .as_ref()
            .map_or_else(|| playback.wants_playing(), |other| other.state.playing);
        let can_play = track.is_some() || remote.is_some() || playback.can_restart_queue(cx);
        let Some(track) = track else {
            return Self {
                is_playing,
                can_play,
                ..Self::default()
            };
        };
        Self {
            has_track: true,
            title: track.name.into(),
            artist: track.artists.into(),
            artwork: track.cover.map(Into::into),
            position,
            duration,
            is_playing,
            can_play,
        }
    }
}

fn seek_target(duration: Duration, progress: f32) -> Duration {
    if !progress.is_finite() {
        return Duration::ZERO;
    }
    duration.mul_f32(progress.clamp(0., 1.))
}

fn clock_width(duration: Duration, time_left: bool, size: Pixels) -> Pixels {
    let base = match duration.as_secs() >= 3600 {
        true => CLOCK_LONG_EMS,
        false => CLOCK_SHORT_EMS,
    };
    let ems = match time_left {
        true => base + CLOCK_SIGN_EMS,
        false => base,
    };
    size * ems
}

/// Picks the single line shown in the live-lyrics strip together with whether it should be
/// emphasised as an active sung line rather than a status fallback.
fn current_lyric_line(lyrics: &Lyrics, has_track: bool, cx: &App) -> (SharedString, bool) {
    if !has_track {
        return (t!("lyrics-idle"), false);
    }

    if let Some(hit) = lyrics.current() {
        match &hit.lyrics {
            music::Lyrics::Synced { lines } => {
                if let Some(index) = lyrics.active_line(cx)
                    && let Some(line) = lines.get(index)
                {
                    let trimmed = line.text.trim();
                    if !trimmed.is_empty() {
                        return (line.text.clone().into(), true);
                    }
                    return (SharedString::new_static("• • •"), false);
                }
                let upcoming = lines
                    .first()
                    .map(|line| line.text.as_str())
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or("♪");
                return (upcoming.to_owned().into(), false);
            }
            music::Lyrics::Plain { .. } => {
                return (t!("mini-player-lyrics-unsynced"), false);
            }
        }
    }

    (lyrics_status_label(lyrics.state()), false)
}

fn lyrics_status_label(state: &LyricsState) -> SharedString {
    match state {
        LyricsState::Loading | LyricsState::Idle => t!("lyrics-loading"),
        LyricsState::Instrumental => t!("lyrics-instrumental"),
        LyricsState::Missing | LyricsState::Ready => t!("lyrics-missing"),
        LyricsState::Failed(_) => t!("lyrics-failed"),
    }
}

/// Stops left-button mouse downs from bubbling into the card's window-drag handler.
fn no_drag<E: InteractiveElement>(element: E) -> E {
    element.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// Marks a non-interactive zone as a native drag area on platforms that support hit-testing.
fn drag_zone<E: InteractiveElement>(zone: E) -> E {
    zone.window_control_area(WindowControlArea::Drag)
}

/// Identity of the track and device a seek drag started against, so a track or device change
/// cancels any pending release seek.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeekTargetIdentity {
    track_id: Option<String>,
    title: String,
    device_id: Option<String>,
}

#[derive(Clone, Debug)]
struct PendingSeek {
    fraction: f32,
    target: SeekTargetIdentity,
}

fn current_seek_identity(playback: &Playback, cx: &App) -> SeekTargetIdentity {
    let track = playback.shown_track(cx);
    SeekTargetIdentity {
        track_id: track.as_ref().and_then(|t| t.id.clone()),
        title: track.map_or_else(String::new, |t| t.name),
        device_id: steered(cx).map(|other| other.device.id),
    }
}

/// Floating compact playback widget rendered in its own window.
pub struct MiniPlayerView {
    playback: Entity<Playback>,
    lyrics: Entity<Lyrics>,
    settings: Entity<AppSettings>,
    ambient: Entity<Ambient>,
    focus_handle: FocusHandle,
    ticker: Option<(Duration, Task<()>)>,
    scrubber: ScrubberState,
    volume: RisingVolume,
    pending_seek: Option<PendingSeek>,
    _subscriptions: Vec<Subscription>,
}

impl VolumeHost for MiniPlayerView {
    fn volume(&mut self) -> &mut RisingVolume {
        &mut self.volume
    }

    fn playback(&self) -> &Entity<Playback> {
        &self.playback
    }
}

impl MiniPlayerView {
    /// Creates the mini player view, observes playback/lyrics/settings, and persists movement.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sonora = Sonora::global(cx);
        let playback = sonora.playback.clone();
        let devices = sonora.devices.clone();
        let lyrics = sonora.lyrics.clone();
        let settings = sonora.settings.clone();
        let ticks = playback.read(cx).ticks().clone();

        let ambient = cx.new(Ambient::framed);
        let cover = playback.read(cx).shown_track(cx).and_then(|t| t.cover);
        ambient.update(cx, |ambient, cx| ambient.follow(cover, cx));

        let initial_radius = corner_radius(settings.read(cx).mini_player().rounding);
        ambient.update(cx, |ambient, cx| {
            ambient.set_corner_radius(Some(initial_radius), cx)
        });

        let subscriptions = vec![
            cx.observe(&playback, |this, playback, cx| {
                let cover = playback.read(cx).shown_track(cx).and_then(|t| t.cover);
                this.ambient
                    .update(cx, |ambient, cx| ambient.follow(cover, cx));
                this.clear_stale_seek(cx);
                this.sync_ticker(cx);
                cx.notify();
            }),
            cx.observe(&devices, |this, _, cx| {
                let cover = this.playback.read(cx).shown_track(cx).and_then(|t| t.cover);
                this.ambient
                    .update(cx, |ambient, cx| ambient.follow(cover, cx));
                this.clear_stale_seek(cx);
                this.sync_ticker(cx);
                cx.notify();
            }),
            cx.observe(&lyrics, |_, _, cx| cx.notify()),
            cx.observe(&ambient, |_, _, cx| cx.notify()),
            cx.observe(&ticks, |_, _, cx| cx.notify()),
            cx.observe(&settings, |this, _, cx| {
                this.sync_ambient_radius(cx);
                this.sync_ticker(cx);
                cx.notify();
            }),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            cx.observe_window_bounds(window, |this, window, cx| {
                let origin = window.window_bounds().get_bounds().origin;
                let x = origin.x / px(1.);
                let y = origin.y / px(1.);
                this.settings.update(cx, |settings, cx| {
                    settings.set_mini_player_origin(Some((x, y)), cx);
                });
            }),
            cx.on_release(|this: &mut Self, cx| {
                this.finish_volume_drag(cx);
            }),
        ];

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let mut this = Self {
            playback,
            lyrics,
            settings,
            ambient,
            focus_handle,
            ticker: None,
            scrubber: ScrubberState::new("mini-player-scrubber"),
            volume: RisingVolume::new("mini-player-volume-slider"),
            pending_seek: None,
            _subscriptions: subscriptions,
        };
        this.sync_ticker(cx);
        this
    }

    /// Finalizes an in-flight local volume slider drag so closing mid-drag still persists gain.
    pub fn finish_volume_drag(&mut self, cx: &mut App) {
        if !self.volume.held {
            return;
        }
        self.volume.held = false;
        self.playback
            .update(cx, |playback, cx| playback.end_volume_slide(cx));
    }

    fn clear_stale_seek(&mut self, cx: &App) {
        let Some(pending) = &self.pending_seek else {
            return;
        };
        if pending.target != current_seek_identity(self.playback.read(cx), cx) {
            self.pending_seek = None;
        }
    }

    fn sync_ambient_radius(&self, cx: &mut Context<Self>) {
        let radius = corner_radius(self.settings.read(cx).mini_player().rounding);
        self.ambient.update(cx, |ambient, cx| {
            ambient.set_corner_radius(Some(radius), cx);
        });
    }

    /// Repaints at karaoke pace in immersive lyrics mode, 250ms while playing, idle on pause.
    fn sync_ticker(&mut self, cx: &mut Context<Self>) {
        let playing = steered(cx).as_ref().map_or_else(
            || self.playback.read(cx).wants_playing(),
            |other| other.state.playing,
        );
        if !playing {
            self.ticker = None;
            return;
        }
        let prefs = self.settings.read(cx).mini_player();
        let interval =
            match prefs.lyrics_mode == MiniPlayerLyricsMode::Fullscreen && prefs.lyrics_karaoke {
                true => KARAOKE_REFRESH,
                false => PROGRESS_REFRESH,
            };
        if self
            .ticker
            .as_ref()
            .is_some_and(|(step, _)| *step == interval)
        {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        self.ticker = Some((interval, task));
    }

    fn window_controls(&self, frosted: bool, theme: &Theme) -> impl IntoElement {
        let icon_color = theme.foreground.opacity(0.85);
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_0p5()
            .child(no_drag(
                Button::new("mini-restore")
                    .ghost()
                    .when(frosted, Button::frosted)
                    .small()
                    .icon("icons/maximize.svg")
                    .tint(icon_color)
                    .tooltip("mini-player-restore")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ExpandMiniPlayer), cx);
                    }),
            ))
            .child(no_drag(
                Button::new("mini-close")
                    .ghost()
                    .when(frosted, Button::frosted)
                    .small()
                    .icon("icons/x.svg")
                    .tint(icon_color)
                    .tooltip("mini-player-close")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(CloseMiniPlayer), cx);
                    }),
            ))
    }

    fn render_header(&self, now: &NowPlaying, frosted: bool, theme: &Theme) -> impl IntoElement {
        let title: SharedString = match now.has_track {
            true => now.title.clone(),
            false => t!("mini-player-nothing-playing"),
        };

        div()
            .flex()
            .items_start()
            .gap_2()
            .child(drag_zone(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        ui::spoken("mini-player-track", title)
                            .text_size(theme.text(ui::Text::Body))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate(),
                    )
                    .when(!now.artist.is_empty(), |this| {
                        this.child(
                            ui::spoken("mini-player-artist", now.artist.clone())
                                .text_size(theme.text(ui::Text::Small))
                                .text_color(theme.foreground.opacity(0.65))
                                .truncate(),
                        )
                    }),
            ))
            .child(self.window_controls(frosted, theme))
    }

    fn render_progress(
        &self,
        now: &NowPlaying,
        time_left: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = theme.foreground.opacity(0.65);
        let duration = now.duration;
        let enabled = now.has_track && !duration.is_zero();
        let current_target = current_seek_identity(self.playback.read(cx), cx);
        let progress = self
            .pending_seek
            .as_ref()
            .filter(|pending| pending.target == current_target)
            .map(|pending| pending.fraction)
            .unwrap_or_else(|| fraction(now.position, duration));
        let text_size = theme.text(ui::Text::Tiny);
        let width = clock_width(duration, time_left, text_size);
        let closing = end_clock(now.position, duration, time_left);
        let playback = self.playback.clone();

        let scrubber = Scrubber::new(&self.scrubber, progress)
            .name(t!("song-position"))
            .value(clock(now.position))
            .colors(
                theme.progress_bar,
                theme.muted_foreground.opacity(0.3),
                theme.foreground,
            )
            .popover(theme.popover, theme.border, theme.popover_foreground)
            .enabled(enabled)
            .on_move(cx.listener(|this, at: &f32, _, cx| {
                let target = current_seek_identity(this.playback.read(cx), cx);
                this.pending_seek = Some(PendingSeek {
                    fraction: *at,
                    target,
                });
                cx.notify();
            }))
            .on_release(cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                if let Some(pending) = this.pending_seek.take()
                    && pending.target == current_seek_identity(playback.read(cx), cx)
                {
                    let target = seek_target(duration, pending.fraction);
                    playback.update(cx, |playback, cx| playback.seek(target, cx));
                }
            }));

        div()
            .id("mini-player-seek")
            .role(Role::Group)
            .aria_label(t!("song-position"))
            .flex()
            .items_center()
            .gap_2()
            .text_size(text_size)
            .font_features(tabular())
            .text_color(muted)
            .child(
                ui::spoken("mini-player-elapsed", clock(now.position))
                    .flex_none()
                    .w(width)
                    .whitespace_nowrap(),
            )
            .child(no_drag(div().flex_1().min_w_0()).child(scrubber))
            .child(
                ui::spoken("mini-player-duration", closing.text)
                    .when_some(closing.spoken, |this, spoken| this.aria_value(spoken))
                    .flex_none()
                    .w(width)
                    .whitespace_nowrap()
                    .text_right(),
            )
    }

    fn render_transport(
        &self,
        now: &NowPlaying,
        prefs: MiniPlayerPrefs,
        dims: MiniPlayerDimensions,
        frosted: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let playback = self.playback.clone();
        let held = self.playback.read(cx);
        let shuffle_on = held.shown_shuffle(cx);
        let repeat_mode = held.shown_repeat(cx);

        let (play_icon, play_tooltip) = match now.is_playing {
            true => ("icons/pause.svg", "play-pause"),
            false => ("icons/play.svg", "play-resume"),
        };

        div()
            .flex()
            .items_center()
            .justify_center()
            .gap_1p5()
            .when(prefs.shuffle_button, |row| {
                row.child(no_drag(
                    transport::shuffle(&self.playback, frosted, cx).tint(match shuffle_on {
                        true => theme.primary,
                        false => theme.muted_foreground,
                    }),
                ))
            })
            .child(no_drag(
                transport::previous(&self.playback, frosted, cx).tint(theme.foreground),
            ))
            .child(no_drag(
                Button::new("mini-play-pause")
                    .primary()
                    .fill(theme.primary, theme.primary_hover)
                    .tint(theme.primary_foreground)
                    .rounded_full()
                    .size(px(dims.play_button))
                    .p_0()
                    .icon(play_icon)
                    .tooltip_above(play_tooltip)
                    .disabled(!now.can_play)
                    .on_click(move |_, _, cx| {
                        playback.update(cx, |playback, cx| playback.toggle_play(cx));
                    }),
            ))
            .child(no_drag(
                transport::next(&self.playback, frosted, cx).tint(theme.foreground),
            ))
            .when(prefs.repeat_button, |row| {
                row.child(no_drag(
                    transport::repeat(&self.playback, frosted, cx).tint(match repeat_mode {
                        Repeat::Off => theme.muted_foreground,
                        _ => theme.primary,
                    }),
                ))
            })
            .when(prefs.volume_button, |row| {
                row.child(no_drag(rising_volume_in(
                    &self.volume,
                    &self.playback,
                    frosted,
                    px(dims.volume_rise),
                    theme,
                    cx,
                )))
            })
    }

    fn render_lyrics_bar(
        &self,
        now: &NowPlaying,
        prefs: MiniPlayerPrefs,
        dims: MiniPlayerDimensions,
        theme: &Theme,
        cx: &App,
    ) -> impl IntoElement {
        let (line, active) = current_lyric_line(self.lyrics.read(cx), now.has_track, cx);
        let color = match active {
            true => theme.foreground,
            false => theme.foreground.opacity(0.55),
        };
        let weight = match active {
            true => FontWeight::MEDIUM,
            false => FontWeight::NORMAL,
        };
        let text_size = theme.text(ui::Text::Small) * lyrics_scale(prefs.lyrics_size);

        drag_zone(
            div()
                .id("mini-player-lyrics")
                .role(Role::Group)
                .aria_label(t!("mini-player-lyrics"))
                .flex_none()
                .w_full()
                .h(px(dims.lyrics_height))
                .px(px(dims.padding))
                .pb(px(dims.padding * 0.45))
                .border_t_1()
                .border_color(theme.border.opacity(0.45))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    ui::spoken("mini-player-lyric-line", line)
                        .max_w_full()
                        .text_size(text_size)
                        .font_weight(weight)
                        .text_color(color)
                        .truncate(),
                ),
        )
    }

    fn render_fullscreen_lyrics(
        &self,
        now: &NowPlaying,
        prefs: MiniPlayerPrefs,
        dims: MiniPlayerDimensions,
        scripts: Option<RomanizationScripts>,
        theme: &Theme,
        cx: &App,
    ) -> impl IntoElement {
        let (verse_size, lane_size) = fullscreen_verse_sizes(prefs);

        if !now.has_track {
            return drag_zone(
                div()
                    .id("mini-player-lyrics-empty")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        ui::spoken("mini-player-lyric-status", t!("lyrics-idle"))
                            .text_size(verse_size)
                            .text_color(theme.foreground.opacity(0.55)),
                    ),
            )
            .into_any_element();
        }

        let lyrics = self.lyrics.read(cx);

        if let Some(hit) = lyrics.current() {
            match &hit.lyrics {
                music::Lyrics::Synced { lines } if !lines.is_empty() => {
                    let active = lyrics.active_line(cx);
                    let focus = active.unwrap_or(0);
                    let visible = match prefs.orientation {
                        MiniPlayerOrientation::Landscape => FULLSCREEN_VISIBLE_VERSES,
                        MiniPlayerOrientation::Portrait => 4,
                    };
                    let past = match prefs.orientation {
                        MiniPlayerOrientation::Landscape => FULLSCREEN_PAST_VERSES,
                        MiniPlayerOrientation::Portrait => 1,
                    };
                    let max_start = lines.len().saturating_sub(visible);
                    let start = focus.saturating_sub(past).min(max_start);
                    let end = (start + visible).min(lines.len());
                    let playback = self.playback.clone();

                    return div()
                        .id("mini-player-lyrics-sheet")
                        .role(Role::Group)
                        .aria_label(t!("mini-player-lyrics"))
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .gap(px(dims.padding * 0.2))
                        .children((start..end).filter_map(|index| {
                            let line = lines.get(index)?;
                            let is_active = active == Some(index);
                            if line.text.trim().is_empty() && !is_active {
                                return None;
                            }
                            let distance = index.abs_diff(focus) as f32;
                            let haze = match (is_active, prefs.lyrics_blur) {
                                (true, _) => 0.,
                                (false, true) => (distance * 0.36).min(1.0),
                                (false, false) => 0.,
                            };
                            let look = VerseLook {
                                active: is_active,
                                size: verse_size,
                                lane: lane_size,
                                karaoke: prefs.lyrics_karaoke,
                                scripts,
                                haze,
                            };
                            let start_at = line.start;
                            let playback = playback.clone();
                            let rendered = match line.text.trim().is_empty() {
                                true => div()
                                    .w_full()
                                    .px(verse_size * 0.45)
                                    .py(verse_size * 0.15)
                                    .text_size(verse_size)
                                    .text_color(theme.foreground.opacity(0.65))
                                    .child("♪"),
                                false => verse(line, look, now.position, theme),
                            };
                            Some(
                                no_drag(div().id(("mini-verse", index)))
                                    .w_full()
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| {
                                        playback.update(cx, |playback, cx| {
                                            playback.seek(start_at, cx);
                                        });
                                    })
                                    .child(rendered),
                            )
                        }))
                        .into_any_element();
                }
                music::Lyrics::Plain { text, .. } if !text.trim().is_empty() => {
                    return div()
                        .id("mini-player-lyrics-plain")
                        .role(Role::Group)
                        .aria_label(t!("mini-player-lyrics"))
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(text.lines().map(|line| {
                            div()
                                .text_size(verse_size * 0.85)
                                .text_color(theme.foreground.opacity(0.85))
                                .child(SharedString::from(line.to_owned()))
                        }))
                        .into_any_element();
                }
                _ => {}
            }
        }

        drag_zone(
            div()
                .id("mini-player-lyrics-status")
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    ui::spoken(
                        "mini-player-lyric-status",
                        lyrics_status_label(lyrics.state()),
                    )
                    .text_size(verse_size * 0.9)
                    .text_color(theme.foreground.opacity(0.55)),
                ),
        )
        .into_any_element()
    }
}

struct RenderLook {
    prefs: MiniPlayerPrefs,
    dims: MiniPlayerDimensions,
    radius: Pixels,
    frosted: bool,
    time_left: bool,
    scripts: Option<RomanizationScripts>,
    theme: Theme,
}

impl MiniPlayerView {
    fn render_centered_meta(&self, now: &NowPlaying, theme: &Theme) -> impl IntoElement {
        let title: SharedString = match now.has_track {
            true => now.title.clone(),
            false => t!("mini-player-nothing-playing"),
        };

        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .child(
                ui::spoken("mini-player-track", title)
                    .max_w_full()
                    .text_size(theme.text(ui::Text::Body))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_center()
                    .truncate(),
            )
            .when(!now.artist.is_empty(), |this| {
                this.child(
                    ui::spoken("mini-player-artist", now.artist.clone())
                        .max_w_full()
                        .text_size(theme.text(ui::Text::Small))
                        .text_color(theme.foreground.opacity(0.65))
                        .text_center()
                        .truncate(),
                )
            })
    }

    fn render_portrait_header(&self, frosted: bool, theme: &Theme) -> impl IntoElement {
        div()
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .child(drag_zone(div().flex_1().h(px(22.))))
            .child(self.window_controls(frosted, theme))
    }

    fn render_fullscreen_body(
        &self,
        now: &NowPlaying,
        look: &RenderLook,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let RenderLook {
            prefs,
            dims,
            radius,
            frosted,
            time_left,
            scripts,
            ref theme,
        } = *look;

        if prefs.orientation == MiniPlayerOrientation::Portrait {
            return div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(dims.gap))
                .p(px(dims.padding))
                .child(self.render_portrait_header(frosted, theme))
                .child(drag_zone(
                    div()
                        .flex_none()
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(dims.padding * 0.45))
                        .child(
                            Artwork::new(now.artwork.clone())
                                .size(px(dims.artwork))
                                .corner_radius(radius * ARTWORK_RADIUS_RATIO),
                        )
                        .child(self.render_centered_meta(now, theme)),
                ))
                .child(self.render_fullscreen_lyrics(now, prefs, dims, scripts, theme, cx))
                .child(
                    div()
                        .flex_none()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(dims.padding * 0.35))
                        .child(self.render_progress(now, time_left, theme, cx))
                        .child(self.render_transport(now, prefs, dims, frosted, theme, cx)),
                )
                .into_any_element();
        }

        let left_column = div()
            .flex_none()
            .w(px(dims.left_column_width))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .child(drag_zone(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(dims.padding * 0.5))
                    .child(
                        Artwork::new(now.artwork.clone())
                            .size(px(dims.artwork))
                            .corner_radius(radius * ARTWORK_RADIUS_RATIO),
                    )
                    .child(self.render_centered_meta(now, theme)),
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(dims.padding * 0.35))
                    .child(self.render_progress(now, time_left, theme, cx))
                    .child(self.render_transport(now, prefs, dims, frosted, theme, cx)),
            );

        let right_column = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(dims.padding * 0.3))
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .flex()
                    .items_center()
                    .child(drag_zone(div().flex_1().h(px(20.))))
                    .child(self.window_controls(frosted, theme)),
            )
            .child(self.render_fullscreen_lyrics(now, prefs, dims, scripts, theme, cx));

        div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .items_stretch()
            .gap(px(dims.gap))
            .p(px(dims.padding))
            .child(left_column)
            .child(right_column)
            .into_any_element()
    }

    fn render_portrait_body(
        &self,
        now: &NowPlaying,
        look: &RenderLook,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let RenderLook {
            prefs,
            dims,
            radius,
            frosted,
            time_left,
            ref theme,
            ..
        } = *look;
        let has_bar = prefs.lyrics_mode == MiniPlayerLyricsMode::Bar;

        let main = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_between()
            .p(px(dims.padding))
            .pb(px(match has_bar {
                true => dims.padding * 0.75,
                false => dims.padding,
            }))
            .child(self.render_portrait_header(frosted, theme))
            .child(drag_zone(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(dims.gap))
                    .child(
                        Artwork::new(now.artwork.clone())
                            .size(px(dims.artwork))
                            .corner_radius(radius * ARTWORK_RADIUS_RATIO),
                    )
                    .child(self.render_centered_meta(now, theme)),
            ))
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(dims.gap * 0.75))
                    .child(self.render_progress(now, time_left, theme, cx))
                    .child(self.render_transport(now, prefs, dims, frosted, theme, cx)),
            );

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(main)
            .when(has_bar, |card| {
                card.child(self.render_lyrics_bar(now, prefs, dims, theme, cx))
            })
    }

    fn render_compact_body(
        &self,
        now: &NowPlaying,
        look: &RenderLook,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let RenderLook {
            prefs,
            dims,
            radius,
            frosted,
            time_left,
            ref theme,
            ..
        } = *look;
        let has_bar = prefs.lyrics_mode == MiniPlayerLyricsMode::Bar;
        let top_bottom_pad = match has_bar {
            true => dims.padding * 0.75,
            false => dims.padding,
        };

        let main_row = div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(dims.gap))
            .px(px(dims.padding))
            .pt(px(dims.padding))
            .pb(px(top_bottom_pad))
            .child(drag_zone(
                div().flex_none().child(
                    Artwork::new(now.artwork.clone())
                        .size(px(dims.artwork))
                        .corner_radius(radius * ARTWORK_RADIUS_RATIO),
                ),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(self.render_header(now, frosted, theme))
                    .child(self.render_progress(now, time_left, theme, cx))
                    .child(self.render_transport(now, prefs, dims, frosted, theme, cx)),
            );

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(main_row)
            .when(has_bar, |card| {
                card.child(self.render_lyrics_bar(now, prefs, dims, theme, cx))
            })
    }
}

impl Render for MiniPlayerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base_theme = *cx.theme();
        let settings = self.settings.read(cx);
        let prefs = settings.mini_player();
        let time_left = settings.player_bar_time_left();
        let scripts = settings
            .romanized_lyrics()
            .then(|| settings.romanization_scripts());
        let dims = mini_player_dimensions(prefs);
        let now = NowPlaying::read(self.playback.read(cx), cx);
        let radius = corner_radius(prefs.rounding);
        self.sync_ambient_radius(cx);

        let track_theme = self.ambient.read(cx).theme(cx);
        let field = prefs.ambient.then(|| self.ambient.clone());
        let frosted = field.is_some();
        let theme = match &field {
            Some(_) => track_theme,
            None => match (track_theme.tint, base_theme.tint == track_theme.tint) {
                (Some(tint), false) if now.has_track => base_theme.tinted(tint),
                _ => base_theme,
            },
        };

        let look = RenderLook {
            prefs,
            dims,
            radius,
            frosted,
            time_left,
            scripts,
            theme,
        };

        let body = match (prefs.lyrics_mode, prefs.orientation) {
            (MiniPlayerLyricsMode::Fullscreen, _) => self
                .render_fullscreen_body(&now, &look, cx)
                .into_any_element(),
            (
                MiniPlayerLyricsMode::Off | MiniPlayerLyricsMode::Bar,
                MiniPlayerOrientation::Portrait,
            ) => self
                .render_portrait_body(&now, &look, cx)
                .into_any_element(),
            (
                MiniPlayerLyricsMode::Off | MiniPlayerLyricsMode::Bar,
                MiniPlayerOrientation::Landscape,
            ) => self.render_compact_body(&now, &look, cx).into_any_element(),
        };

        let card = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .rounded(radius)
            .border_1()
            .border_color(theme.border)
            .text_color(theme.foreground)
            .when(field.is_none(), |this| this.bg(theme.background))
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                window.start_window_move();
            })
            .children(field)
            .child(body);

        div()
            .id("mini-player")
            .role(Role::Region)
            .aria_label(t!("mini-player-title"))
            .key_context(MINI_PLAYER_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_target_clamps_invalid_or_out_of_range_fractions() {
        let total = Duration::from_secs(200);
        assert_eq!(seek_target(total, f32::NAN), Duration::ZERO);
        assert_eq!(seek_target(total, f32::INFINITY), Duration::ZERO);
        assert_eq!(seek_target(total, -0.5), Duration::ZERO);
        assert_eq!(seek_target(total, 1.5), total);
        assert_eq!(seek_target(total, 0.5), Duration::from_secs(100));
    }

    #[test]
    fn clock_width_expands_for_long_tracks_and_time_left() {
        let size = px(12.);
        let short = Duration::from_secs(180);
        let long = Duration::from_secs(3700);
        assert!(clock_width(short, true, size) > clock_width(short, false, size));
        assert!(clock_width(long, false, size) > clock_width(short, false, size));
    }

    #[test]
    fn mini_player_dimensions_scale_with_orientation_size_and_lyrics_mode() {
        let base = MiniPlayerPrefs::default();
        let compact = mini_player_dimensions(MiniPlayerPrefs {
            size: MiniPlayerSize::Compact,
            ..base
        });
        let standard = mini_player_dimensions(base);
        let large = mini_player_dimensions(MiniPlayerPrefs {
            size: MiniPlayerSize::Large,
            ..base
        });
        let portrait = mini_player_dimensions(MiniPlayerPrefs {
            orientation: MiniPlayerOrientation::Portrait,
            ..base
        });
        let bar = mini_player_dimensions(MiniPlayerPrefs {
            lyrics_mode: MiniPlayerLyricsMode::Bar,
            ..base
        });
        let fullscreen = mini_player_dimensions(MiniPlayerPrefs {
            lyrics_mode: MiniPlayerLyricsMode::Fullscreen,
            ..base
        });

        assert!(compact.width < standard.width && standard.width < large.width);
        assert!(compact.height < standard.height && standard.height < large.height);
        assert!(portrait.height > portrait.width);
        assert_eq!(bar.height, standard.height + bar.lyrics_height);
        assert!(fullscreen.width > standard.width);
        assert!(fullscreen.height > bar.height);
    }
}
