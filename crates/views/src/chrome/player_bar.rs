use router::{Destination, Link, navigate};
use std::time::Duration;
use ui::ActiveTheme as _;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, ScrollWheelEvent, SharedString,
};
use gpui::{Window, div, px};
use i18n::t;
use input::{ToggleFullscreen, ToggleLyrics, ToggleQueue};
use state::{AppSettings, Playback, Queue, SideTab, Sleep, Sonora};
use ui::{
    Artwork, Button, ExplicitBadge, InlineLink, InlineLinks, MenuItem, Picker, Popovers, Popup,
    Room, Scrollbar, Scrubber, ScrubberState, clock,
};

use crate::chrome::SidebarRight;
use crate::shared::menus::ItemMenu;
use crate::shared::transport::{NOTCH, like, moved, percent, transport, volume_icon};

const SEEK_MAX: f32 = 560.;
const VOLUME_WIDTH: f32 = 110.;
const VOLUME_TIGHT: f32 = 72.;
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
const SLEEP: &str = "sleep";
const SLEEP_STEP_MINUTES: u64 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SleepMode {
    Off,
    Custom,
    EndOfTrack,
}

pub(crate) struct PlayerBar {
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    settings: Entity<AppSettings>,
    track_menu: ItemMenu,
    context_menu: Option<(music::Track, Point<Pixels>)>,
    sleep_group: Popovers,
    sleep_mode: ScrubberState,
    sleep_mode_preview: Option<SleepMode>,
    sleep: ScrubberState,
    pending_sleep: Option<Option<Sleep>>,
    seek: ScrubberState,
    volume: ScrubberState,
    pending: Option<f32>,
    over_seek: Option<f32>,
    over_volume: Option<f32>,
    volume_held: bool,
    muted: Option<f32>,
}

impl PlayerBar {
    pub fn new(playback: Entity<Playback>, queue: Entity<Queue>, cx: &mut Context<Self>) -> Self {
        let library = Sonora::global(cx).library.clone();
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        cx.observe(&queue, |_, _, cx| cx.notify()).detach();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));

        Self {
            playback,
            queue,
            settings,
            track_menu: ItemMenu::new(playlist_scrollbar),
            context_menu: None,
            sleep_group: Popovers::default(),
            sleep_mode: ScrubberState::new("sleep-mode"),
            sleep_mode_preview: None,
            sleep: ScrubberState::new("sleep"),
            pending_sleep: None,
            seek: ScrubberState::new("seek"),
            volume: ScrubberState::new("volume"),
            pending: None,
            over_seek: None,
            over_volume: None,
            volume_held: false,
            muted: None,
        }
    }

    fn open_context_menu(
        &mut self,
        track: music::Track,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.track_menu.reset(cx);
        self.context_menu = Some((track, position));
        cx.notify();
    }

    fn commit_seek(&mut self, cx: &mut Context<Self>) {
        let Some(fraction) = self.pending.take() else {
            return;
        };
        self.playback
            .update(cx, |playback, cx| playback.seek_fraction(fraction, cx));
    }

    fn hover(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let pad = cx.theme().metrics.pad;
        let seek = self.seek.hovered(event.position, pad);
        let volume = self.volume.hovered(event.position, pad);

        if moved(self.over_seek, seek) || moved(self.over_volume, volume) {
            self.over_seek = seek;
            self.over_volume = volume;
            cx.notify();
        }
    }

    fn turn_volume(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height()).y;
        if delta == Pixels::ZERO {
            return;
        }
        cx.stop_propagation();

        let notch = match delta > Pixels::ZERO {
            true => NOTCH,
            false => -NOTCH,
        };
        let level = (self.playback.read(cx).volume() + notch).clamp(0., 1.);
        self.muted = None;
        self.playback
            .update(cx, |playback, cx| playback.set_volume(level, cx));
    }

    fn sound(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let empty = theme.muted_foreground.opacity(0.3);
        let level = self.playback.read(cx).volume();
        let showing = self.over_volume.is_some() || self.volume_held;
        let bubble = showing.then(|| (level, percent(level)));
        let restore = self.muted.unwrap_or(0.7);

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .on_scroll_wheel(cx.listener(Self::turn_volume))
            .child(
                Button::new("volume")
                    .ghost()
                    .small()
                    .icon(volume_icon(level))
                    .tooltip_above(match level <= 0.001 {
                        true => "player-unmute",
                        false => "player-mute",
                    })
                    .tint(theme.muted_foreground)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let wanted = match level <= 0.001 {
                            true => restore,
                            false => 0.,
                        };
                        this.muted = match wanted {
                            0. => Some(level),
                            _ => None,
                        };
                        this.playback
                            .update(cx, |playback, cx| playback.set_volume(wanted, cx));
                    })),
            )
            .child(
                div().w(width).flex_none().child(
                    Scrubber::new(&self.volume, level)
                        .colors(theme.progress_bar, empty, theme.foreground)
                        .when_some(bubble, |this, (at, text)| this.bubble(at, text))
                        .on_move(cx.listener(|this, fraction: &f32, _, cx| {
                            let level = *fraction;
                            this.volume_held = true;
                            this.muted = None;
                            this.playback
                                .update(cx, |playback, cx| playback.set_volume(level, cx));
                        }))
                        .on_release(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.volume_held = false;
                            cx.notify();
                        })),
                ),
            )
    }

    fn side_buttons(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (open, tab, show_sleep) = {
            let settings = self.settings.read(cx);
            (
                settings.sidebar_right_open(),
                settings.sidebar_right_tab(),
                settings.sleep_timer(),
            )
        };
        let sleep = show_sleep.then(|| self.sleep_button(cx));

        let button = move |id: &'static str, icon: &'static str, hint: &'static str, side| {
            let showing = open && tab == side;

            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip_above(hint)
                .selected(showing)
                .tint(match showing {
                    true => theme.foreground,
                    false => theme.muted_foreground,
                })
                .on_click(move |_, window, cx| {
                    let action: Box<dyn gpui::Action> = match side {
                        SideTab::Queue => Box::new(ToggleQueue),
                        SideTab::Lyrics => Box::new(ToggleLyrics),
                    };
                    window.dispatch_action(action, cx);
                })
        };

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(button(
                "player-lyrics",
                "icons/mic-vocal.svg",
                "lyrics-title",
                SideTab::Lyrics,
            ))
            .child(button(
                "player-queue",
                "icons/list-music.svg",
                "queue-title",
                SideTab::Queue,
            ))
            .children(sleep)
            .into_any_element()
    }

    fn sleep_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let armed = self.playback.read(cx).sleep().is_some();

        Picker::icon(SLEEP, &self.sleep_group, "icons/moon.svg")
            .tooltip_above("player-sleep")
            .selected(armed)
            .tint(match armed {
                true => theme.foreground,
                false => theme.muted_foreground,
            })
            .items(match self.sleep_group.shows(SLEEP) {
                true => self.sleep_items(cx),
                false => Vec::new(),
            })
            .into_any_element()
    }

    fn sleep_items(&mut self, cx: &mut Context<Self>) -> Vec<MenuItem> {
        let theme = *cx.theme();
        let (min_minutes, max_minutes) = {
            let settings = self.settings.read(cx);
            (settings.sleep_min_minutes(), settings.sleep_max_minutes())
        };
        let (active_sleep, remaining) = {
            let playback = self.playback.read(cx);
            (playback.sleep(), playback.sleep_remaining())
        };
        let current = self.pending_sleep.unwrap_or(active_sleep);
        let active_mode = sleep_mode(active_sleep);
        let mode = self
            .sleep_mode_preview
            .unwrap_or_else(|| sleep_mode(current));
        let layout_mode = match self.sleep_mode_preview {
            Some(_) => active_mode,
            None => mode,
        };
        let custom_duration = match current {
            Some(Sleep::After(duration)) => duration,
            _ => Duration::from_secs(min_minutes * 60),
        };
        let displayed_duration = match self.pending_sleep.is_some() {
            true => custom_duration,
            false => remaining.unwrap_or(custom_duration),
        };
        let mode_fraction = sleep_mode_fraction(mode);
        let mode_label = sleep_mode_label(mode);
        let custom_minutes = rounded_sleep_minutes(displayed_duration, min_minutes, max_minutes);
        let custom_fraction = sleep_fraction(custom_minutes, min_minutes, max_minutes);
        let custom_label = match self.pending_sleep.is_some() {
            true => t!("player-sleep-minutes", count = custom_minutes),
            false => remaining
                .map(sleep_remaining_label)
                .unwrap_or_else(|| t!("player-sleep-minutes", count = custom_minutes)),
        };

        let dial = div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .py_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_size(theme.text(ui::Text::Small))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(t!("player-sleep")),
                    )
                    .child(mode_label.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(theme.text(ui::Text::Tiny))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        Scrubber::new(&self.sleep_mode, mode_fraction)
                            .colors(
                                theme.progress_bar,
                                theme.muted_foreground.opacity(0.3),
                                theme.foreground,
                            )
                            .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                                let mode = sleep_mode_at_fraction(*fraction);
                                this.sleep_mode_preview = Some(mode);
                                let sleep = match mode {
                                    SleepMode::Off => None,
                                    SleepMode::Custom => Some(Sleep::After(Duration::from_secs(
                                        custom_minutes.clamp(min_minutes, max_minutes) * 60,
                                    ))),
                                    SleepMode::EndOfTrack => Some(Sleep::EndOfTrack),
                                };
                                this.pending_sleep = Some(sleep);
                                cx.notify();
                            }))
                            .on_release(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                this.commit_sleep(cx);
                            })),
                    ),
            )
            .when(layout_mode == SleepMode::Custom, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .text_size(theme.text(ui::Text::Small))
                                .child(t!("player-sleep-custom"))
                                .child(custom_label.clone()),
                        )
                        .child(
                            Scrubber::new(&self.sleep, custom_fraction)
                                .colors(
                                    theme.progress_bar,
                                    theme.muted_foreground.opacity(0.3),
                                    theme.foreground,
                                )
                                .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                                    let minutes = sleep_minutes_at_fraction(
                                        *fraction,
                                        min_minutes,
                                        max_minutes,
                                    );
                                    this.pending_sleep =
                                        Some(Some(Sleep::After(Duration::from_secs(minutes * 60))));
                                    cx.notify();
                                }))
                                .on_release(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.commit_sleep(cx);
                                })),
                        ),
                )
            });

        vec![MenuItem::new("sleep-dial", "").content(dial)]
    }

    fn commit_sleep(&mut self, cx: &mut Context<Self>) {
        let Some(sleep) = self.pending_sleep.take() else {
            return;
        };
        self.sleep_mode_preview = None;
        self.playback
            .update(cx, |playback, cx| playback.set_sleep(sleep, cx));
    }

    fn fullscreen_button(&self) -> Button {
        Button::new("toggle-fullscreen")
            .ghost()
            .small()
            .icon("icons/maximize.svg")
            .tooltip_above("player-fullscreen")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleFullscreen), cx))
    }

    fn now_playing(&self, room: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let artwork = ui::snapped(theme.metrics.row, window);
        let artists = theme.text(ui::Text::Small);
        let track = self.playback.read(cx).track().cloned();
        let cover = track.as_ref().and_then(|track| track.cover.clone());
        let explicit = track.as_ref().is_some_and(|track| track.explicit);
        let like = like(track.clone(), cx);

        div()
            .flex()
            .items_center()
            .gap_3()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("now-playing-artwork")
                    .when_some(
                        track.as_ref().and_then(|track| track.album_id.clone()),
                        |this, album| this.link(Destination::Album(album.into())),
                    )
                    .when_some(track.clone(), |this, context| {
                        this.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                window.prevent_default();
                                this.open_context_menu(context.clone(), event.position, cx);
                            }),
                        )
                    })
                    .child(Artwork::new(cover).size(artwork)),
            )
            .when(room, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .min_w_0()
                                .items_center()
                                .gap_1()
                                .child(match &track {
                                    Some(track) => {
                                        let context_track = track.clone();
                                        div()
                                            .id("now-playing-track")
                                            .when_some(track.album_id.clone(), |this, album_id| {
                                                this.hover(|style| style.underline())
                                                    .link(Destination::Album(album_id.into()))
                                            })
                                            .on_mouse_down(
                                                MouseButton::Right,
                                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                                    window.prevent_default();
                                                    this.open_context_menu(
                                                        context_track.clone(),
                                                        event.position,
                                                        cx,
                                                    );
                                                }),
                                            )
                                            .child(SharedString::from(track.name.clone()))
                                            .min_w_0()
                                            .truncate()
                                    }
                                    None => div()
                                        .id("now-playing-album")
                                        .child(t!("player-nothing-playing"))
                                        .min_w_0()
                                        .text_color(muted)
                                        .truncate(),
                                })
                                .when(explicit, |this| {
                                    this.child(div().flex_none().child(ExplicitBadge::new()))
                                })
                                .child(like),
                        )
                        .when_some(track.clone(), |this, track| {
                            this.child(
                                InlineLinks::new(
                                    "now-playing-artist",
                                    track.artist_refs.into_iter().map(|artist| {
                                        InlineLink::new(artist.name, artist.id.map(Into::into))
                                    }),
                                    track.artists,
                                    muted,
                                )
                                .text_size(artists)
                                .truncate()
                                .on_click(|id, cx| {
                                    navigate(Destination::Artist(id), cx);
                                }),
                            )
                        }),
                )
            })
    }
}

impl PlayerBar {
    pub(crate) fn height(window: &Window, cx: &gpui::App) -> Pixels {
        let theme = *cx.theme();
        match !Room::of(window.viewport_size().width).fits(Room::Roomy) {
            true => ui::snapped(theme.metrics.player_bar + theme.metrics.pad * 3., window),
            false => ui::snapped(theme.metrics.player_bar, window),
        }
    }
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let empty = muted.opacity(0.3);
        let span = Room::of(window.viewport_size().width);
        let stacked = !span.fits(Room::Roomy);
        let height = Self::height(window, cx);
        let clock_text = theme.text(ui::Text::Tiny);

        let show_track = span.fits(Room::Snug);
        let sides = match SidebarRight::available(window) {
            true => Some(self.side_buttons(cx)),
            false => None,
        };

        let playback = self.playback.read(cx);
        let seekable = playback.track().is_some();
        let progress = self.pending.unwrap_or_else(|| playback.progress());
        let elapsed = playback.position();
        let total = playback
            .track()
            .map(|track| track.duration)
            .unwrap_or(Duration::ZERO);
        let clock_width = clock_text
            * match total.as_secs() >= 3600 {
                true => CLOCK_LONG,
                false => CLOCK_SHORT,
            };

        let seek_bubble = self
            .over_seek
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));

        let clock_label = |value: Duration, align_end: bool| {
            div()
                .child(clock(value))
                .w(clock_width)
                .flex_none()
                .whitespace_nowrap()
                .text_size(clock_text)
                .text_color(muted)
                .when_else(align_end, |this| this.text_right(), |this| this.text_left())
        };

        let seek = div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(clock_label(elapsed, true))
            .child(
                div().flex_1().min_w_0().child(
                    Scrubber::new(&self.seek, progress)
                        .colors(theme.progress_bar, empty, theme.foreground)
                        .enabled(seekable)
                        .when_some(seek_bubble, |this, (at, text)| this.bubble(at, text))
                        .on_move(cx.listener(|this, fraction: &f32, _, cx| {
                            this.pending = Some(*fraction);
                            cx.notify();
                        }))
                        .on_release(
                            cx.listener(|this, _: &MouseUpEvent, _, cx| this.commit_seek(cx)),
                        ),
                ),
            )
            .child(clock_label(total, false))
            .into_any_element();

        let base = div()
            .flex()
            .w_full()
            .h(height)
            .flex_none()
            .px_5()
            .when(stacked, |this| this.py_2())
            .when(!theme.transparent, |this| this.bg(theme.secondary))
            .border_t_1()
            .border_color(theme.border)
            .on_mouse_move(cx.listener(Self::hover));

        let context_menu = self.context_menu.clone().map(|(track, position)| {
            Popup::new(position, self.track_menu.for_track(&track, cx)).on_close(cx.listener(
                |this, _, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                },
            ))
        });

        let content = match stacked {
            true => base
                .flex_col()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .child(self.now_playing(show_track, window, cx))
                        .child(transport(&self.playback, &self.queue, false, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .w_full()
                        .child(div().flex_1().min_w_0().child(seek))
                        .children(sides)
                        .child(self.sound(px(VOLUME_TIGHT), cx))
                        .child(self.fullscreen_button()),
                ),
            false => base
                .items_center()
                .gap_4()
                .child(self.now_playing(show_track, window, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .flex_1()
                        .min_w_0()
                        .max_w(px(SEEK_MAX))
                        .child(transport(&self.playback, &self.queue, false, cx))
                        .child(seek),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .flex_1()
                        .min_w_0()
                        .children(sides)
                        .child(self.sound(px(VOLUME_WIDTH), cx))
                        .child(self.fullscreen_button()),
                ),
        };

        content.when_some(context_menu, |this, menu| this.child(menu))
    }
}

fn sleep_mode(sleep: Option<Sleep>) -> SleepMode {
    match sleep {
        None => SleepMode::Off,
        Some(Sleep::After(_)) => SleepMode::Custom,
        Some(Sleep::EndOfTrack) => SleepMode::EndOfTrack,
    }
}

fn sleep_mode_at_fraction(fraction: f32) -> SleepMode {
    match fraction.clamp(0., 1.) {
        fraction if fraction < 0.25 => SleepMode::Off,
        fraction if fraction < 0.75 => SleepMode::Custom,
        _ => SleepMode::EndOfTrack,
    }
}

fn sleep_mode_fraction(mode: SleepMode) -> f32 {
    match mode {
        SleepMode::Off => 0.,
        SleepMode::Custom => 0.5,
        SleepMode::EndOfTrack => 1.,
    }
}

fn sleep_mode_label(mode: SleepMode) -> SharedString {
    match mode {
        SleepMode::Off => t!("player-sleep-off"),
        SleepMode::Custom => t!("player-sleep-custom"),
        SleepMode::EndOfTrack => t!("player-sleep-end-of-track"),
    }
}

fn rounded_sleep_minutes(duration: Duration, min_minutes: u64, max_minutes: u64) -> u64 {
    let minutes = duration.as_secs().div_ceil(60);
    sleep_minutes(minutes, min_minutes, max_minutes)
}

fn sleep_fraction(minutes: u64, min_minutes: u64, max_minutes: u64) -> f32 {
    if min_minutes >= max_minutes {
        return 0.;
    }
    ((minutes.saturating_sub(min_minutes)) as f32 / (max_minutes - min_minutes) as f32)
        .clamp(0., 1.)
}

fn sleep_minutes_at_fraction(fraction: f32, min_minutes: u64, max_minutes: u64) -> u64 {
    let raw = min_minutes as f32 + fraction.clamp(0., 1.) * (max_minutes - min_minutes) as f32;
    let snapped = (raw / SLEEP_STEP_MINUTES as f32).round() as u64 * SLEEP_STEP_MINUTES;
    sleep_minutes(snapped, min_minutes, max_minutes)
}

fn sleep_minutes(minutes: u64, min_minutes: u64, max_minutes: u64) -> u64 {
    minutes.clamp(min_minutes, max_minutes)
}

fn sleep_remaining_label(remaining: Duration) -> SharedString {
    let seconds = remaining.as_secs().max(1);
    match seconds >= 60 {
        true => t!("player-sleep-minutes-left", count = seconds / 60),
        false => t!("player-sleep-seconds-left", count = seconds),
    }
}
