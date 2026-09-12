use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyView, App, Bounds, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent,
    SharedString, SpringState, Task,
};
use gpui::{Window, canvas, deferred, div, px, relative};
use i18n::t;
use input::{ToggleFullscreen, WORKSPACE_CONTEXT};
use router::{Destination, navigate};
use state::{AppSettings, Cover, Playback, Queue, SideTab, Sonora};
use ui::{
    ActiveTheme as _, Artwork, Button, ExplicitBadge, InlineLink, InlineLinks, Motion,
    Motioned as _, Popup, Room, Scrollbar, Scrubber, ScrubberState, Springs, Text, Visualizer,
    clock, snapped,
};

use crate::chrome::{Aside, PlayerBar, TitleBarOptions};
use crate::shared::menus::ItemMenu;
use crate::shared::transport::{NOTCH, like, moved, percent, transport, volume_icon};
use crate::shared::visualizer::VisualizerDrive;
use crate::shells::Shell;

const COVER_TALL: f32 = 0.46;
const COVER_WIDE: f32 = 0.34;
const COVER_TALL_TIGHT: f32 = 0.6;
const COVER_WIDE_TIGHT: f32 = 0.86;
const COVER_TALL_REST: f32 = 0.56;
const COVER_WIDE_REST: f32 = 0.4;
const COVER_TALL_TIGHT_REST: f32 = 0.72;
const COVER_MIN: f32 = 96.;
const COVER_MAX: f32 = 520.;
const COVER_MAX_REST: f32 = 560.;
const COVER_LAYER_PAD: f32 = 2.;
const RESERVE: f32 = 2.9;
const RESERVE_REST: f32 = 1.3;
const DOCK: f32 = 1.15;
const DOCK_FULL: f32 = 1.7;
const SINK: f32 = 24.;
const PILL_GAP: f32 = 2.;
const SEEK_MAX: f32 = 420.;
const VOLUME_RISE: f32 = 132.;
const VOLUME_ZONE: f32 = 14.;
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
const VISUALIZER_MIN: f32 = 160.;
const REST: Duration = Duration::from_millis(1500);
const WAKE_DEBOUNCE: Duration = Duration::from_millis(400);
const SPRING_REST: f32 = 0.001;
const SPRING_STALL: Duration = Duration::from_millis(64);

pub struct FullscreenView {
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    cover: Entity<Cover>,
    settings: Entity<AppSettings>,
    aside: Entity<Aside>,
    panel: Option<SideTab>,
    seek: ScrubberState,
    pending: Option<f32>,
    over_seek: Option<f32>,
    volume: ScrubberState,
    over_volume: bool,
    over_zone: bool,
    over_panel: bool,
    over_pill: bool,
    volume_held: bool,
    muted: Option<f32>,
    large: Option<SharedString>,
    revision: usize,
    track_menu: ItemMenu,
    context_menu: Option<(music::Track, Point<Pixels>)>,
    last_moved: Instant,
    awake: bool,
    hidden: SpringState,
    spring_beat: Instant,
    rest: Option<Task<()>>,
    focus: FocusHandle,
    visualizer: VisualizerDrive,
    root_bounds: Rc<Cell<Bounds<Pixels>>>,
    artwork_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl FullscreenView {
    pub fn new(playback: Entity<Playback>, queue: Entity<Queue>, cx: &mut Context<Self>) -> Self {
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let cover = Sonora::global(cx).cover.clone();
        cx.observe(&cover, |_, _, cx| cx.notify()).detach();
        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let aside = cx.new(|cx| Aside::new(queue.clone(), playback.clone(), SideTab::Lyrics, cx));
        aside.update(cx, |aside, _| aside.strip());
        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));

        let mut this = Self {
            playback,
            queue,
            cover,
            settings,
            aside,
            panel: Some(SideTab::Lyrics),
            seek: ScrubberState::new("fullscreen-seek"),
            pending: None,
            over_seek: None,
            volume: ScrubberState::new("fullscreen-volume-slider"),
            over_volume: false,
            over_zone: false,
            over_panel: false,
            over_pill: false,
            volume_held: false,
            muted: None,
            large: None,
            revision: 0,
            track_menu: ItemMenu::new(playlist_scrollbar),
            context_menu: None,
            last_moved: Instant::now(),
            awake: true,
            hidden: SpringState {
                position: 0.,
                velocity: 0.,
            },
            spring_beat: Instant::now(),
            rest: None,
            focus: cx.focus_handle(),
            visualizer: VisualizerDrive::default(),
            root_bounds: Rc::new(Cell::new(Bounds::default())),
            artwork_bounds: Rc::new(Cell::new(Bounds::default())),
        };
        this.stir(cx);
        this
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn show(&mut self, panel: Option<SideTab>, cx: &mut Context<Self>) {
        self.panel = panel;
        if let Some(tab) = panel {
            self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        }
        self.stir(cx);
    }

    fn stir(&mut self, cx: &mut Context<Self>) {
        if !self.awake {
            self.flip(true);
            cx.notify();
        }
        self.last_moved = Instant::now();
        self.rest = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REST).await;
            this.update(cx, |this, cx| {
                if this.last_moved.elapsed() < REST {
                    return;
                }
                if this.busy() {
                    this.stir(cx);
                    return;
                }
                this.flip(false);
                cx.notify();
            })
            .ok();
        }));
    }

    fn hover(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let pad = cx.theme().metrics.pad;
        let seek = self.seek.hovered(event.position, pad);
        if moved(self.over_seek, seek) {
            self.over_seek = seek;
            cx.notify();
        }
        self.poke(cx);
    }

    fn poke(&mut self, cx: &mut Context<Self>) {
        if self.awake && self.last_moved.elapsed() < WAKE_DEBOUNCE {
            return;
        }
        self.stir(cx);
    }

    fn busy(&self) -> bool {
        self.volume_open()
            || self.over_pill
            || self.pending.is_some()
            || self.context_menu.is_some()
    }

    fn flip(&mut self, awake: bool) {
        if self.awake == awake {
            return;
        }
        self.awake = awake;
        self.spring_beat = Instant::now();
    }

    fn hidden(&mut self, window: &mut Window, cx: &App) -> f32 {
        let target = match self.awake {
            true => 0.,
            false => 1.,
        };
        if cx.reduce_motion() {
            self.hidden = SpringState {
                position: target,
                velocity: 0.,
            };
            self.spring_beat = Instant::now();
            return target;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.spring_beat).min(SPRING_STALL);
        self.spring_beat = now;
        self.hidden = Springs::RESPONSIVE.step(self.hidden, target, elapsed.as_secs_f32());
        if Springs::RESPONSIVE.is_settled(self.hidden, target, SPRING_REST) {
            self.hidden = SpringState {
                position: target,
                velocity: 0.,
            };
        } else {
            window.request_animation_frame();
        }
        self.hidden.position.clamp(0., 1.)
    }

    fn volume_open(&self) -> bool {
        self.over_volume || self.over_zone || self.over_panel || self.volume_held
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

    fn commit_seek(&mut self, cx: &mut Context<Self>) {
        let Some(fraction) = self.pending.take() else {
            return;
        };
        self.playback
            .update(cx, |playback, cx| playback.seek_fraction(fraction, cx));
    }

    fn artwork(
        &mut self,
        layout_side: Pixels,
        raster_side: Pixels,
        presentation_scale: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let radius = cx.theme().radius * 2.;
        let pad = px(COVER_LAYER_PAD);
        let inset = (layout_side - raster_side) / 2. - pad;
        let track = self.playback.read(cx).track().cloned();
        let album = track.as_ref().and_then(|track| track.album_id.clone());
        let small = track.as_ref().and_then(|track| track.cover.clone());
        let cover_large = self.cover.read(cx).large();
        let large = cover_large
            .filter(|url| Some(*url) != small.as_deref())
            .map(SharedString::from);

        if self.large != large {
            self.large = large.clone();
            self.revision += 1;
        }
        let revision = self.revision;
        let local = track
            .as_ref()
            .and_then(|track| track.id.as_deref())
            .is_some_and(music::is_local_id)
            || small.as_ref().is_some_and(|url| url.starts_with("file://"));
        let waiting = !local && album.is_some() && cover_large.is_none();
        let artwork_bounds = self.artwork_bounds.clone();

        div()
            .id("fullscreen-artwork")
            .relative()
            .size(layout_side)
            .flex_none()
            .when_some(album, |this, album| {
                this.cursor_pointer()
                    .on_click(move |_, _, cx| open_album(&album, cx))
            })
            .child(
                canvas(
                    move |bounds, _, _| artwork_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top(inset)
                    .left(inset)
                    .size(raster_side + pad * 2.)
                    .layer_scale(presentation_scale)
                    .child(
                        div().absolute().top(pad).left(pad).child(
                            Artwork::new(small)
                                .size(raster_side)
                                .corner_radius(radius)
                                .soft(waiting),
                        ),
                    )
                    .when_some(large, |this, url| {
                        this.child(
                            div()
                                .absolute()
                                .top(pad)
                                .left(pad)
                                .child(
                                    Artwork::new(Some(url))
                                        .size(raster_side)
                                        .corner_radius(radius),
                                )
                                .motion(("cover-large", revision), Motion::Slow, |art, t| {
                                    art.opacity(t)
                                }),
                        )
                    }),
            )
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

    fn meta(&self, hide: f32, lift: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let track = self.playback.read(cx).track().cloned();
        let title = match &track {
            Some(track) => SharedString::from(track.name.clone()),
            None => t!("player-nothing-playing"),
        };
        let album = track.as_ref().and_then(|track| track.album_id.clone());
        let explicit = track.as_ref().is_some_and(|track| track.explicit);
        let held = track.clone();

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
            .w_full()
            .min_w_0()
            .top(lift)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .when(explicit, |this| this.child(div().size_4().flex_none()))
                    .child(div().w(theme.metrics.control_small).flex_none())
                    .child(
                        div()
                            .id("fullscreen-title")
                            .min_w_0()
                            .truncate()
                            .text_size(theme.text(Text::Title))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when_some(album, |this, album| {
                                this.cursor_pointer()
                                    .hover(|style| style.underline())
                                    .on_click(move |_, _, cx| open_album(&album, cx))
                            })
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    let Some(track) = held.clone() else {
                                        return;
                                    };
                                    window.prevent_default();
                                    this.open_context_menu(track, event.position, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .child(title),
                    )
                    .when(explicit, |this| {
                        this.child(div().flex_none().child(ExplicitBadge::new()))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .opacity(1. - hide)
                            .child(like(track.clone(), cx)),
                    ),
            )
            .when_some(track, |this, track| {
                this.child(
                    div().flex().w_full().min_w_0().justify_center().child(
                        InlineLinks::new(
                            "fullscreen-artists",
                            track.artist_refs.into_iter().map(|artist| {
                                InlineLink::new(artist.name, artist.id.map(Into::into))
                            }),
                            track.artists,
                            theme.muted_foreground,
                        )
                        .text_size(theme.text(Text::Body))
                        .truncate()
                        .on_click(|id, cx| navigate(Destination::Artist(id), cx)),
                    ),
                )
            })
    }

    fn strip(&self, hide: f32, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let cover = ui::snapped(theme.metrics.row, window);
        let track = self.playback.read(cx).track().cloned();
        let title = match &track {
            Some(track) => SharedString::from(track.name.clone()),
            None => t!("player-nothing-playing"),
        };
        let album = track.as_ref().and_then(|track| track.album_id.clone());
        let explicit = track.as_ref().is_some_and(|track| track.explicit);
        let held = track.clone();

        div()
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .id("strip-artwork")
                    .when_some(album.clone(), |this, album| {
                        this.cursor_pointer()
                            .on_click(move |_, _, cx| open_album(&album, cx))
                    })
                    .child(Artwork::new(track.as_ref().and_then(|t| t.cover.clone())).size(cover)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .id("strip-title")
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .when_some(album, |this, album| {
                                        this.cursor_pointer()
                                            .hover(|style| style.underline())
                                            .on_click(move |_, _, cx| open_album(&album, cx))
                                    })
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, window, cx| {
                                                let Some(track) = held.clone() else {
                                                    return;
                                                };
                                                window.prevent_default();
                                                this.open_context_menu(track, event.position, cx);
                                                cx.stop_propagation();
                                            },
                                        ),
                                    )
                                    .child(title),
                            )
                            .when(explicit, |this| {
                                this.child(div().flex_none().child(ExplicitBadge::new()))
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .opacity(1. - hide)
                                    .child(like(track.clone(), cx)),
                            ),
                    )
                    .when_some(track, |this, track| {
                        this.child(
                            InlineLinks::new(
                                "strip-artists",
                                track.artist_refs.into_iter().map(|artist| {
                                    InlineLink::new(artist.name, artist.id.map(Into::into))
                                }),
                                track.artists,
                                theme.muted_foreground,
                            )
                            .text_size(theme.text(Text::Small))
                            .truncate()
                            .on_click(|id, cx| navigate(Destination::Artist(id), cx)),
                        )
                    }),
            )
    }

    fn seek(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let empty = muted.opacity(0.3);
        let text = theme.text(Text::Tiny);
        let playback = self.playback.read(cx);
        let seekable = playback.track().is_some();
        let progress = self.pending.unwrap_or_else(|| playback.progress());
        let elapsed = playback.position();
        let total = playback
            .track()
            .map(|track| track.duration)
            .unwrap_or(Duration::ZERO);
        let width = text
            * match total.as_secs() >= 3600 {
                true => CLOCK_LONG,
                false => CLOCK_SHORT,
            };

        let bubble = self
            .over_seek
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));

        let label = move |value: Duration, align_end: bool| {
            div()
                .child(clock(value))
                .w(width)
                .flex_none()
                .whitespace_nowrap()
                .text_size(text)
                .text_color(muted)
                .when_else(align_end, |this| this.text_right(), |this| this.text_left())
        };

        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(label(elapsed, true))
            .child(
                div().flex_1().min_w_0().child(
                    Scrubber::new(&self.seek, progress)
                        .colors(theme.progress_bar, empty, theme.foreground)
                        .enabled(seekable)
                        .when_some(bubble, |this, (at, text)| this.bubble(at, text))
                        .on_move(cx.listener(|this, fraction: &f32, _, cx| {
                            this.pending = Some(*fraction);
                            cx.notify();
                        }))
                        .on_release(
                            cx.listener(|this, _: &MouseUpEvent, _, cx| this.commit_seek(cx)),
                        ),
                ),
            )
            .child(label(total, false))
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let inline = self.panel.is_none();

        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w_full()
            .max_w(px(SEEK_MAX))
            .flex_none()
            .when(inline, |this| this.child(self.pill(cx)))
            .child(self.seek(cx))
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(transport(&self.playback, &self.queue, true, cx))
                    .child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(self.sound(cx)),
                    ),
            )
    }

    fn dock(&self, cap: Pixels, hide: f32, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .w_full()
            .justify_center()
            .when(hide > 0., |this| {
                this.max_h(cap * (1. - hide))
                    .overflow_hidden()
                    .opacity(1. - hide)
            })
            .when(hide < 1., |this| {
                this.child(
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .top(px(SINK) * hide)
                        .child(self.controls(cx)),
                )
            })
    }

    fn pill(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let gap = px(PILL_GAP);
        let linger = cx.listener(|this: &mut Self, hovering: &bool, _, cx| {
            this.over_pill = *hovering;
            if *hovering {
                this.poke(cx);
            }
            cx.notify();
        });
        let tab = move |id: &'static str, icon: &'static str, hint: &'static str, panel| {
            let showing = self.panel == panel;

            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip_above(hint)
                .selected(showing)
                .rounded(theme.radius)
                .tint(match showing {
                    true => theme.foreground,
                    false => theme.muted_foreground,
                })
                .on_click(cx.listener(move |this, _, _, cx| this.show(panel, cx)))
        };

        div()
            .id("fullscreen-pill")
            .flex()
            .flex_none()
            .items_center()
            .gap(gap)
            .p(gap)
            .rounded(theme.radius + gap)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .on_hover(linger)
            .child(tab(
                "fullscreen-artwork-tab",
                "icons/disc-3.svg",
                "fullscreen-artwork",
                None,
            ))
            .child(tab(
                "fullscreen-lyrics",
                "icons/mic-vocal.svg",
                "lyrics-title",
                Some(SideTab::Lyrics),
            ))
            .child(tab(
                "fullscreen-queue",
                "icons/list-music.svg",
                "queue-title",
                Some(SideTab::Queue),
            ))
    }

    fn sound(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let zone = px(VOLUME_ZONE);
        let level = self.playback.read(cx).volume();
        let empty = theme.muted_foreground.opacity(0.3);
        let restore = self.muted.unwrap_or(0.7);
        let span = theme.metrics.control_small + zone * 2.;
        let bubble = (self.over_panel || self.volume_held).then(|| (level, percent(level)));

        div()
            .relative()
            .flex()
            .flex_none()
            .on_scroll_wheel(cx.listener(Self::turn_volume))
            .child(
                div()
                    .id("fullscreen-volume-hover")
                    .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                        this.over_volume = *hovering;
                        cx.notify();
                    }))
                    .child(
                        Button::new("fullscreen-volume")
                            .ghost()
                            .small()
                            .icon(volume_icon(level))
                            .tint(match self.volume_open() {
                                true => theme.foreground,
                                false => theme.muted_foreground,
                            })
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
                    ),
            )
            .when(self.volume_open(), |this| {
                this.child(deferred(
                    div()
                        .id("fullscreen-volume-zone")
                        .absolute()
                        .bottom_0()
                        .left(relative(0.5))
                        .ml(Pixels::ZERO - span / 2.)
                        .w(span)
                        .pt(zone)
                        .pb(theme.metrics.control_small + px(PILL_GAP) * 2.)
                        .flex()
                        .justify_center()
                        .on_scroll_wheel(cx.listener(Self::turn_volume))
                        .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                            this.over_zone = *hovering;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .id("fullscreen-volume-panel")
                                .occlude()
                                .flex()
                                .justify_center()
                                .p_1()
                                .py_2()
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .on_scroll_wheel(cx.listener(Self::turn_volume))
                                .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                                    this.over_panel = *hovering;
                                    cx.notify();
                                }))
                                .child(
                                    div().h(px(VOLUME_RISE)).flex().child(
                                        Scrubber::new(&self.volume, level)
                                            .vertical()
                                            .colors(theme.progress_bar, empty, theme.foreground)
                                            .when_some(bubble, |this, (at, text)| {
                                                this.bubble(at, text)
                                            })
                                            .on_move(cx.listener(|this, fraction: &f32, _, cx| {
                                                let level = *fraction;
                                                this.volume_held = true;
                                                this.muted = None;
                                                this.playback.update(cx, |playback, cx| {
                                                    playback.set_volume(level, cx)
                                                });
                                            }))
                                            .on_release(cx.listener(
                                                |this, _: &MouseUpEvent, _, cx| {
                                                    this.volume_held = false;
                                                    cx.notify();
                                                },
                                            )),
                                    ),
                                ),
                        ),
                ))
            })
    }

    fn floating(&self, hide: f32, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .bottom_3()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .block_mouse_except_scroll()
                    .opacity(1. - hide)
                    .top(px(SINK) * hide)
                    .child(self.pill(cx)),
            )
    }

    fn menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (track, position) = self.context_menu.clone()?;

        Some(
            Popup::new(position, self.track_menu.for_track(&track, cx)).on_close(cx.listener(
                |this, _, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                },
            )),
        )
    }

    fn leave(&self) -> Button {
        Button::new("leave-fullscreen")
            .ghost()
            .small()
            .icon("icons/chevron-down.svg")
            .tooltip_above("player-fullscreen-leave")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleFullscreen), cx))
    }
}

fn open_album(album: &str, cx: &mut App) {
    navigate(Destination::Album(album.into()), cx);
}

impl Shell for FullscreenView {
    fn title_bar(&self, _content: Option<AnyView>, _cx: &App) -> TitleBarOptions {
        TitleBarOptions {
            navigation: false,
            sidebar_open: false,
            sidebar_right: None,
            offset: Pixels::ZERO,
            border: false,
            content: None,
        }
    }
}

impl Render for FullscreenView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let viewport = window.viewport_size();
        let room = Room::of(viewport.width);
        let split = room.fits(Room::Wide) && self.panel.is_some();
        let hide = self.hidden(window, cx);
        let shown = hide < 1.;
        let (tall, wide, ceiling, tall_rest, wide_rest, ceiling_rest) = match room.fits(Room::Wide)
        {
            true => (
                COVER_TALL,
                COVER_WIDE,
                px(COVER_MAX),
                COVER_TALL_REST,
                COVER_WIDE_REST,
                px(COVER_MAX_REST),
            ),
            false => (
                COVER_TALL_TIGHT,
                COVER_WIDE_TIGHT,
                viewport.width,
                COVER_TALL_TIGHT_REST,
                COVER_WIDE_TIGHT,
                viewport.width,
            ),
        };
        let fit = |tall: f32, wide: f32, reserve: f32, ceiling: Pixels| {
            (viewport.height * tall)
                .min(viewport.height - theme.metrics.title_bar - theme.metrics.player_bar * reserve)
                .min(viewport.width * wide)
                .min(ceiling)
                .max(px(COVER_MIN))
        };
        let near = fit(tall, wide, RESERVE, ceiling);
        let far = fit(tall_rest, wide_rest, RESERVE_REST, ceiling_rest);
        let presented_side = near + (far - near) * hide;
        // The flex item must never change size when the idle state flips: even a one-frame
        // near/far swap makes the centred column relayout. Keep its awake footprint forever and
        // animate only the fixed large raster surface in the compositor.
        let side = snapped(near, window);
        let raster_side = snapped(far, window);
        let cover_scale = presentation_scale(presented_side, raster_side);
        let lift = (presented_side - side) / 2.;
        let staged = self.panel.is_none() || split;

        let visualizer_on = self.panel.is_none() && self.settings.read(cx).visualizer();
        match visualizer_on
            .then(|| self.playback.read(cx).spectrum())
            .flatten()
        {
            Some(spectrum) => self.visualizer.show(cx.entity_id(), spectrum, window),
            None => self.visualizer.hide(),
        }
        let bottom = |bounds: Bounds<Pixels>| bounds.origin.y + bounds.size.height;
        let visualizer_max = (bottom(self.root_bounds.get()) - bottom(self.artwork_bounds.get()))
            .max(px(VISUALIZER_MIN));
        let root_bounds = self.root_bounds.clone();

        div()
            .id("fullscreen")
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_5()
            .px_8()
            .pb_6()
            .on_mouse_move(cx.listener(Self::hover))
            .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| this.poke(cx)))
            .on_scroll_wheel(cx.listener(|this, _: &ScrollWheelEvent, _, cx| this.poke(cx)))
            .on_key_down(cx.listener(|this, _: &KeyDownEvent, _, cx| this.poke(cx)))
            .child(
                canvas(move |bounds, _, _| root_bounds.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .when(visualizer_on, |this| {
                this.child(
                    Visualizer::new(self.visualizer.levels(), visualizer_max)
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0(),
                )
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_8()
                    .when(staged, |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_5()
                                .min_w_0()
                                .when_else(
                                    split,
                                    |this| this.flex_1().h_full(),
                                    |this| this.w_full(),
                                )
                                .child(self.artwork(side, raster_side, cover_scale, cx))
                                .child(self.meta(hide, lift, cx))
                                .when(split, |this| {
                                    this.child(self.dock(theme.metrics.player_bar * DOCK, hide, cx))
                                }),
                        )
                    })
                    .when(self.panel.is_some(), |this| {
                        this.child(
                            div()
                                .relative()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .h_full()
                                .child(self.aside.clone())
                                .when(shown, |this| this.child(self.floating(hide, cx))),
                        )
                    }),
            )
            .when(!split && self.panel.is_some(), |this| {
                this.child(self.strip(hide, window, cx))
            })
            .when(!split, |this| {
                this.child(self.dock(theme.metrics.player_bar * DOCK_FULL, hide, cx))
            })
            .when(shown, |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom(px(SINK) * -hide)
                        .right_5()
                        .h(PlayerBar::height(window, cx))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .when(!room.fits(Room::Roomy), |this| {
                            // Match the second row of the compact player bar.
                            this.py_2()
                                .gap_2()
                                .child(div().h(snapped(theme.metrics.row, window)).flex_none())
                        })
                        .opacity(1. - hide)
                        .child(self.leave()),
                )
            })
            .children(self.menu(cx))
    }
}

fn presentation_scale(presented: Pixels, layout: Pixels) -> f32 {
    presented.as_f32() / layout.as_f32()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_spring_preserves_a_subpixel_size() {
        let presented = px(200.25);
        let layout = px(200.);
        let scale = presentation_scale(presented, layout);

        assert!((layout.as_f32() * scale - presented.as_f32()).abs() < 0.001);
    }
}
