use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyView, App, Bounds, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent,
    SharedString, SpringState, Task,
};
use gpui::{Window, canvas, deferred, div, phi, px, relative};
use i18n::t;
use input::{ToggleFullscreen, WORKSPACE_CONTEXT};
use router::{Destination, navigate};
use state::{
    AppSettings, Cover, FullscreenControlsAutohide, Playback, PlaybackState, Queue, SideTab, Sonora,
};
use ui::{
    ActiveTheme as _, Artwork, Button, ExplicitBadge, InlineLink, InlineLinks, Motion,
    Motioned as _, Popup, Room, Scrollbar, Scrubber, ScrubberState, Springs, TabBar, Text,
    Visualizer, clock, glass, snapped,
};

use crate::chrome::{Aside, TitleBarOptions};
use crate::shared::menus::ItemMenu;
use crate::shared::transport::{NOTCH, like, moved, percent, transport, volume_icon};
use crate::shared::veil::{Edge, veil};
use crate::shared::visualizer::VisualizerDrive;
use crate::shared::{self, ambient, starry};
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
/// The page's own `gap_5`, `pb_6` and the meta block's `gap_1`, in rems, so the cover can be
/// fitted against the room actually left over. GPUI's spacing scale is a quarter rem a step.
const COLUMN_GAP: f32 = 1.25;
const PAGE_PAD: f32 = 1.5;
const META_GAP: f32 = 0.25;
const DOCK: f32 = 1.15;
const DOCK_FULL: f32 = 1.7;
const SINK: f32 = 24.;
const LEAVE_DROP: f32 = 2.;
const PILL_GAP: f32 = 2.;
const SEEK_MAX: f32 = 420.;
const VOLUME_RISE: f32 = 132.;
const VOLUME_ZONE: f32 = 14.;
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
const VISUALIZER_MIN: f32 = 160.;
/// How tall the band under the controls is, as a share of the window. It carries the seek bar,
/// the transport and the meta line over a visualizer that reaches the bottom edge, so it runs
/// a good deal taller than the chrome itself and fades out well above it.
const VEIL: f32 = 0.34;
/// How hard that band blurs what passes under it. The settings header has flat page under it
/// and gets by on a pixel; a bar of the visualizer is a hard edge and needs a real radius
/// before it stops reading through the text over it.
const VEIL_BLUR: Pixels = px(12.);
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
    over_transport: bool,
    over_leave: bool,
    volume_held: bool,
    muted: Option<f32>,
    large: Option<SharedString>,
    revision: usize,
    track_menu: ItemMenu,
    context_menu: Option<(music::Track, Point<Pixels>)>,
    last_moved: Instant,
    inside: bool,
    awake: bool,
    hidden: SpringState,
    spring_beat: Instant,
    rest: Option<Task<()>>,
    focus: FocusHandle,
    visualizer: VisualizerDrive,
    stage: starry::Drive,
    stage_clock: starry::Clock,
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
            over_transport: false,
            over_leave: false,
            volume_held: false,
            muted: None,
            large: None,
            revision: 0,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            context_menu: None,
            last_moved: Instant::now(),
            inside: true,
            awake: true,
            hidden: SpringState {
                position: 0.,
                velocity: 0.,
            },
            spring_beat: Instant::now(),
            rest: None,
            focus: cx.focus_handle(),
            visualizer: VisualizerDrive::default(),
            stage: starry::Drive::default(),
            stage_clock: starry::Clock::default(),
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
                if this.busy(cx) {
                    this.stir(cx);
                    return;
                }
                this.flip(false);
                // The pointer going idle takes the cursor with it; any motion
                // brings it back on its own.
                cx.hide_cursor();
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

    /// Whether anything under the pointer should hold the chrome awake. A hover flag only counts
    /// while the pointer is over the window: one set when it left stays set, and a stale flag
    /// would keep the controls up for good.
    fn busy(&self, cx: &App) -> bool {
        let parked = self.over_volume
            || self.over_zone
            || self.over_panel
            || self.over_pill
            || self.over_transport
            || self.over_leave
            || self.over_seek.is_some()
            || self.scrollbar_held(cx);
        (self.inside && parked)
            || self.volume_held
            || self.pending.is_some()
            || self.context_menu.is_some()
    }

    /// Follows the pointer in and out of the window, once a frame. A hover flag clears on an
    /// event the pointer has to be present for, so the ones still set when it left the window are
    /// dropped here instead. Crossing the window edge repaints on its own, so this is never late.
    fn watch(&mut self, window: &Window, cx: &mut Context<Self>) {
        let inside = window.is_window_hovered();
        if inside == self.inside {
            return;
        }
        self.inside = inside;
        if !inside {
            self.over_seek = None;
            self.stir(cx);
        }
    }

    /// Whether the pointer is parked on the lyrics panel's scrollbar. Read at
    /// rest-expiry time, so no subscription is needed: a parked pointer simply
    /// re-arms the timer instead of letting the chrome go idle under it.
    fn scrollbar_held(&self, cx: &App) -> bool {
        self.panel.is_some() && self.aside.read(cx).scrollbar_active(cx)
    }

    fn flip(&mut self, awake: bool) {
        if self.awake == awake {
            return;
        }
        self.awake = awake;
        self.spring_beat = Instant::now();
    }

    /// Steps the idle spring and answers how far the view has sunk toward rest, 0 awake to
    /// 1 asleep. It follows the pointer alone and never the visibility setting, so the leave
    /// button can ride it whatever the setting says about the controls.
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
        let settings = self.settings.read(cx);
        let layout = settings.stage_style();
        let staged = layout.shown();
        let style = settings.visualizer_style();
        let particles = settings.particles();
        let theme = *cx.theme();
        let levels = self.visualizer.levels();

        // Two clocks. The record and everything riding it turn only while
        // sound plays — pausing parks them mid-turn, resuming picks them up —
        // and the particle field fades out with the music and back in with it.
        // Both hold their pose when motion is reduced, and the ring reads the
        // already-eased spectrum levels, so it settles the way the bottom
        // visualizer does.
        let playing = matches!(
            self.playback.read(cx).state(),
            PlaybackState::Playing | PlaybackState::Loading
        );
        let pose = match staged && ui::motion::animates(cx) {
            true => self.stage_clock.tick(playing),
            false => starry::Pose {
                turn: 0.,
                presence: match playing {
                    true => 1.,
                    false => 0.,
                },
            },
        };
        let scene = starry::Stage {
            levels,
            style,
            particles,
            layout,
            elapsed: match staged && ui::motion::animates(cx) {
                true => starry::spin(),
                false => 0.,
            },
            turn: pose.turn,
            presence: pose.presence,
            theme,
        };
        let cover = if staged {
            vec![
                div()
                    .absolute()
                    .top(pad)
                    .left(pad)
                    .child(starry::stage(raster_side, small, waiting, scene))
                    .into_any_element(),
            ]
        } else {
            let mut faces = vec![
                div()
                    .absolute()
                    .top(pad)
                    .left(pad)
                    .child(
                        Artwork::new(small)
                            .size(raster_side)
                            .corner_radius(radius)
                            .soft(waiting),
                    )
                    .into_any_element(),
            ];
            if let Some(url) = large {
                faces.push(
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
                        })
                        .into_any_element(),
                );
            }
            faces
        };

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
                    .child(div().absolute().inset_0().children(cover)),
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
        let frosted = ambient::shown(cx);
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
                // Three-part row with equal flex sides, so the title stays truly centred:
                // the heart and the explicit badge live in the right cell and never shift it.
                // Both sides have to stay styled the same and the room around the title has to
                // come from the row gap, since padding on a side floors that side flex basis
                // and makes it the wider one.
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(div().flex_1().min_w_0())
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
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .when(explicit, |this| {
                                this.child(div().flex_none().child(ExplicitBadge::new()))
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .opacity(1. - hide)
                                    .child(like(track.clone(), cx).when(frosted, Button::frosted)),
                            ),
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
        let frosted = ambient::shown(cx);
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
                                    .child(like(track.clone(), cx).when(frosted, Button::frosted)),
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

    /// The pill, the seek bar and the transport row. Below `Room::Roomy` the row spans the
    /// whole window, so the volume button steps one control in from the right edge to leave the
    /// corner to the leave button.
    fn controls(&self, room: Room, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let inline = self.panel.is_none();
        let clear = match room.fits(Room::Roomy) {
            true => Pixels::ZERO,
            false => theme.metrics.control_small,
        };

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
                    .id("fullscreen-transport")
                    .relative()
                    .w_full()
                    .flex()
                    .justify_center()
                    .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                        this.over_transport = *hovering;
                        cx.notify();
                    }))
                    .child(transport(&self.playback, &self.queue, true, cx))
                    .child(
                        div()
                            .absolute()
                            .right(clear)
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(self.sound(cx)),
                    ),
            )
    }

    fn dock(&self, cap: Pixels, hide: f32, room: Room, cx: &mut Context<Self>) -> impl IntoElement {
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
                        .child(self.controls(room, cx)),
                )
            })
    }

    fn pill(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let frosted = ambient::shown(cx);
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
                .when(frosted, Button::frosted)
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
            .flex_none()
            .on_hover(linger)
            .child(
                TabBar::new("fullscreen-pill-bar")
                    .when(frosted, TabBar::blurred)
                    .rounded(theme.radius + gap)
                    .items([
                        tab(
                            "fullscreen-artwork-tab",
                            "icons/disc-3.svg",
                            "fullscreen-artwork",
                            None,
                        ),
                        tab(
                            "fullscreen-lyrics",
                            "icons/mic-vocal.svg",
                            "lyrics-title",
                            Some(SideTab::Lyrics),
                        ),
                        tab(
                            "fullscreen-queue",
                            "icons/list-music.svg",
                            "queue-title",
                            Some(SideTab::Queue),
                        ),
                    ]),
            )
    }

    fn sound(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let frosted = ambient::shown(cx);
        let hazy = frosted && ui::blurring(cx);
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
                            .when(frosted, Button::frosted)
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
                            // Over the ambient field the panel is glass; over flat paint a
                            // blur shows nothing, so there it keeps the popover fill, and so
                            // does a run with the blur turned off.
                            match hazy {
                                true => glass(div(), cx),
                                false => div().bg(theme.popover),
                            }
                            .id("fullscreen-volume-panel")
                            .occlude()
                            .flex()
                            .justify_center()
                            .p_1()
                            .py_2()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
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
                                        .when_some(bubble, |this, (at, text)| this.bubble(at, text))
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

    /// The leave button in the bottom right corner, centred on a player bar's height at every
    /// width, so the breakpoint that stacks the chrome player bar cannot move it, and nudged
    /// down by `LEAVE_DROP` to sit level with the volume button's glyph. It sinks and
    /// fades with the idle spring rather than with the controls, so a pointer move still brings
    /// it back when the controls are set to stay hidden.
    fn leave(&self, idle: f32, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let frosted = ambient::shown(cx);

        div()
            .id("leave-fullscreen-hover")
            .absolute()
            .bottom(px(SINK) * -idle - px(LEAVE_DROP))
            .right_5()
            .h(snapped(theme.metrics.player_bar, window))
            .flex()
            .flex_col()
            .justify_center()
            .opacity(1. - idle)
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.over_leave = *hovering;
                cx.notify();
            }))
            .child(
                Button::new("leave-fullscreen")
                    .ghost()
                    .when(frosted, Button::frosted)
                    .small()
                    .icon("icons/chevron-down.svg")
                    .tooltip_above("player-fullscreen-leave")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ToggleFullscreen), cx)
                    }),
            )
    }
}

fn open_album(album: &str, cx: &mut App) {
    navigate(Destination::Album(album.into()), cx);
}

impl Shell for FullscreenView {
    fn title_bar(&self, _content: Option<AnyView>, cx: &App) -> TitleBarOptions {
        TitleBarOptions {
            navigation: false,
            sidebar_open: false,
            sidebar_right: None,
            offset: Pixels::ZERO,
            border: false,
            content: None,
            transparent: ambient::shown(cx),
        }
    }
}

impl Render for FullscreenView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let viewport = window.viewport_size();
        let room = Room::of(viewport.width);
        let split = room.fits(Room::Wide) && self.panel.is_some();
        self.watch(window, cx);
        let idle = self.hidden(window, cx);
        let hide = match self.settings.read(cx).fullscreen_controls_autohide() {
            FullscreenControlsAutohide::Automatic => idle,
            FullscreenControlsAutohide::AlwaysShown => 0.,
            FullscreenControlsAutohide::AlwaysHidden => 1.,
        };
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
        // Everything the cover shares the column with, measured rather than guessed: the title
        // bar above it, the gap down to the meta line, the meta itself, the gap under it and
        // the page's bottom padding. Neither meta row sets a line height, so both stand at
        // GPUI's own leading. A window too short for all of it has to shrink the cover, since
        // the column centres what it cannot fit and the top edge is what goes.
        let rem = window.rem_size();
        let line = |step: Text| phi().to_pixels(theme.text(step).into(), rem).round();
        let meta = line(Text::Title) + rem * META_GAP + line(Text::Body);
        let stack = theme.metrics.title_bar + rem * (COLUMN_GAP * 2. + PAGE_PAD) + meta;
        // The dock reserves the cap it clamps itself to while fading, and nothing at all once
        // it is hidden, which is the whole difference between the awake and the resting fit.
        let dock = theme.metrics.player_bar
            * match split {
                true => DOCK,
                false => DOCK_FULL,
            };
        let fit = |tall: f32, wide: f32, reserve: Pixels, ceiling: Pixels| {
            (viewport.height * tall)
                .min(viewport.height - reserve)
                .min(viewport.width * wide)
                .min(ceiling)
                .max(px(COVER_MIN))
        };
        let near = fit(tall, wide, stack + dock, ceiling);
        // The resting cover is the raster, scaled down while the controls are up, so the layer
        // the compositor scales is that much wider than the cover on screen and reaches well
        // past it on every side. A layer is clipped to the window before it is scaled, so one
        // that overhangs the top edge loses a strip off the cover itself, which is the whole
        // reason the resting size can never reach further up than the room above the cover.
        let slack = (viewport.height - stack - dock - near).max(Pixels::ZERO);
        let above = theme.metrics.title_bar + slack / 2. - px(COVER_LAYER_PAD);
        let far = fit(tall_rest, wide_rest, stack, ceiling_rest)
            .min(near + above * 2.)
            .max(near);
        let presented_side = near + (far - near) * hide;
        // The flex item must never change size when the idle state flips: even a one-frame
        // near/far swap makes the centred column relayout. Keep its awake footprint forever and
        // animate only the fixed large raster surface in the compositor.
        let side = snapped(near, window);
        let raster_side = snapped(far, window);
        let cover_scale = presentation_scale(presented_side, raster_side);
        let lift = (presented_side - side) / 2.;
        let staged = self.panel.is_none() || split;
        let layout = self.settings.read(cx).stage_style();
        let starry = staged && layout.shown();

        let style = self.settings.read(cx).visualizer_style();
        let visualizer_on = self.panel.is_none() && style.shown();
        match (visualizer_on || starry)
            .then(|| self.playback.read(cx).spectrum())
            .flatten()
        {
            Some(spectrum) => self.visualizer.show(cx.entity_id(), spectrum, window),
            None => self.visualizer.hide(),
        }
        // The ring rides the spectrum drive above, which only runs while sound
        // is being made. Everything else on the stage asks for frames of its
        // own — the record's turn, the sheen, the particles fading in and out
        // around the music. Once a paused stage has faded to nothing there is
        // nothing left to draw, and the loop winds down until it is asked for
        // again; playback starting wakes the view on its own.
        let playing = matches!(
            self.playback.read(cx).state(),
            PlaybackState::Playing | PlaybackState::Loading
        );
        self.stage.run(
            cx.entity_id(),
            starry && ui::motion::animates(cx) && self.stage_clock.moving(playing),
            window,
        );
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
            // Capture phase: every click stirs the idle timer, even one a
            // control underneath swallows for itself.
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| this.poke(cx)))
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
                        .style_kind(style)
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0(),
                )
            })
            // Straight over the visualizer and under everything else: a backdrop blurs only
            // what is painted before it, so the bars stop reading through the transport while
            // the title and the artist line keep their own edges. The band fades with the
            // controls, so a hidden set takes it along.
            .when(visualizer_on && shown && shared::effects(), |this| {
                let band = viewport.height * VEIL;
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(band)
                        .opacity(1. - hide)
                        .child(veil(
                            Edge::Bottom,
                            band,
                            VEIL_BLUR,
                            theme.background,
                            window,
                        )),
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
                                    this.child(self.dock(
                                        theme.metrics.player_bar * DOCK,
                                        hide,
                                        room,
                                        cx,
                                    ))
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
                this.child(self.dock(theme.metrics.player_bar * DOCK_FULL, hide, room, cx))
            })
            .when(idle < 1., |this| this.child(self.leave(idle, window, cx)))
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
