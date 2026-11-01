use std::cell::Cell;
use std::rc::Rc;

use router::{Destination, Link, navigate};
use std::time::Duration;
use ui::ActiveTheme as _;
use ui::Tails as _;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent, SharedString,
};
use gpui::{Window, canvas, div, point, px};
use i18n::t;
use input::{ToggleFullscreen, ToggleLyrics, ToggleQueue};
use state::{AppSettings, Playback, PlayerBarLayout, Queue, SideTab, Sonora};
use ui::{
    Artwork, Button, ExplicitBadge, HoverFade, InlineLink, InlineLinks, Motion, Popup, Room,
    Scrollbar, Scrubber, ScrubberState, clock, tabular,
};

use crate::chrome::devices::DevicePicker;
use crate::chrome::jam::{self, JamPeople};
use crate::chrome::{Chrome, SidebarRight};
use crate::shared::menus::ItemMenu;
use crate::shared::transport::{NOTCH, fraction, like, moved, percent, transport, volume_icon};

const SEEK_MAX: f32 = 560.;
const VOLUME_WIDTH: f32 = 110.;
const VOLUME_TIGHT: f32 = 72.;
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
/// The height of the strip above the bar that names the Jam being listened to, or the other
/// device playback is on.
const BANNER: f32 = 32.;
const BANNER_ICON: f32 = 14.;
/// The width of a member's picture on the strip, and how many of them it shows before it counts
/// the rest.
const BANNER_FACE: f32 = 24.;
const BANNER_FACES: usize = 4;
/// The space round the faces inside their pill, the same on every side so the pill's round ends
/// stay concentric with the faces.
const FACES_PAD: f32 = 2.;

pub(crate) struct PlayerBar {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    track_menu: ItemMenu,
    devices: Entity<DevicePicker>,
    people: Entity<JamPeople>,
    /// Where the faces on the Jam's strip were last laid out, so the people list opens over them.
    faces_at: Rc<Cell<Option<Bounds<Pixels>>>>,
    context_menu: Option<(music::Track, Point<Pixels>)>,
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
        let devices = Sonora::global(cx).devices.clone();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        cx.observe(&queue, |_, _, cx| cx.notify()).detach();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));
        let people = cx.new(JamPeople::new);
        let picker = cx.new(DevicePicker::new);

        Self {
            playback,
            settings,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            devices: picker,
            people,
            faces_at: Rc::default(),
            context_menu: None,
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

    /// Opens the track menu on whatever is playing when the click lands, so a render does not
    /// have to hold a copy of the track for a menu that is rarely asked for.
    fn open_playing_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(track) = self.playback.read(cx).shown_track(cx) else {
            return;
        };
        self.open_context_menu(track, position, cx);
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
        let level = (self.playback.read(cx).shown_volume(cx) + notch).clamp(0., 1.);
        self.muted = None;
        self.playback
            .update(cx, |playback, cx| playback.set_volume(level, cx));
    }

    fn sound(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let empty = theme.muted_foreground.opacity(0.3);
        let level = self.playback.read(cx).shown_volume(cx);
        let showing = self.over_volume.is_some() || self.volume_held;
        let bubble = showing.then(|| (level, percent(level)));
        let restore = self.muted.unwrap_or(0.7);

        div()
            .id("player-volume")
            .role(Role::Group)
            .aria_label(t!("equalizer-status-volume"))
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
                        .name(t!("equalizer-status-volume"))
                        .value(percent(level))
                        .colors(theme.progress_bar, empty, theme.foreground)
                        .when_some(bubble, |this, (at, text)| this.bubble(at, text))
                        .on_move(cx.listener(|this, fraction: &f32, _, cx| {
                            let level = *fraction;
                            this.volume_held = true;
                            this.muted = None;
                            this.playback
                                .update(cx, |playback, cx| playback.slide_volume(level, cx));
                            cx.notify();
                        }))
                        .on_release(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.volume_held = false;
                            this.playback
                                .update(cx, |playback, cx| playback.end_volume_slide(cx));
                            cx.notify();
                        })),
                ),
            )
    }

    fn side_buttons(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (open, tab) = {
            let settings = self.settings.read(cx);
            (settings.sidebar_right_open(), settings.sidebar_right_tab())
        };

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
            .into_any_element()
    }

    fn fullscreen_button(&self) -> Button {
        Button::new("toggle-fullscreen")
            .ghost()
            .small()
            .icon("icons/maximize.svg")
            .tooltip_above("player-fullscreen")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleFullscreen), cx))
    }

    fn now_playing(
        &self,
        room: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = match self.playback.read(cx).shown_track(cx) {
            Some(track) => SharedString::from(format!("{} — {}", track.name, track.artists)),
            None => t!("player-nothing-playing"),
        };
        div()
            .id("now-playing")
            .role(Role::Group)
            .aria_label(label)
            .flex()
            .items_center()
            .gap_3()
            .flex_1()
            .min_w_0()
            .child(self.artwork(window, cx))
            .when(room, |this| this.child(self.track_details(false, cx)))
    }

    /// The playing cover keeps its album link and context menu in every layout.
    fn artwork(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let layout = self.settings.read(cx).player_bar_layout();
        let artwork = ui::snapped(
            match layout == PlayerBarLayout::Compact && !Self::stacked(window, layout) {
                true => theme.metrics.thumb,
                false => theme.metrics.row,
            },
            window,
        );
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let cover = track.and_then(|track| track.cover.clone());
        let album = track.map(|track| match track.album.is_empty() {
            true => t!("song-view-album"),
            false => SharedString::from(track.album.clone()),
        });
        div()
            .id("now-playing-artwork")
            .when_some(
                track.and_then(|track| track.album_id.clone()),
                |this, album| this.link(Destination::Album(album.into())),
            )
            .when_some(album, |this, album| this.aria_label(album))
            .when(track.is_some(), |this| {
                this.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        this.open_playing_menu(event.position, cx);
                    }),
                )
            })
            .child(Artwork::new(cover).size(artwork))
            .flex_none()
    }

    /// Track and artist links, explicit badge and favorite action, optionally centered without
    /// moving the cover or transport with them.
    fn track_details(&self, centered: bool, cx: &mut Context<Self>) -> gpui::Div {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let artists = theme.text(ui::Text::Small);
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let explicit = track.is_some_and(|track| track.explicit);
        let title = match track {
            Some(track) => div()
                .id("now-playing-track")
                .map(|this| match track.album_id.clone() {
                    Some(album_id) => this
                        .aria_label(SharedString::from(track.name.clone()))
                        .hover(|style| style.underline())
                        .link(Destination::Album(album_id.into())),
                    None => this
                        .role(Role::Label)
                        .aria_value(SharedString::from(track.name.clone())),
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        this.open_playing_menu(event.position, cx);
                    }),
                )
                .child(SharedString::from(track.name.clone()))
                .min_w_0()
                .truncate()
                .tails(),
            None => div()
                .id("now-playing-album")
                .role(Role::Status)
                .aria_label(t!("player-nothing-playing"))
                .child(t!("player-nothing-playing"))
                .min_w_0()
                .text_color(muted)
                .truncate(),
        };
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .when(explicit, |this| {
                this.child(div().flex_none().child(ExplicitBadge::new()))
            })
            .child(like(track, &self.playback, cx));
        let headline = match centered {
            true => {
                // Equal clearance on both sides keeps the title itself centered. Actions sit
                // outside its measured width and remain reachable even for a truncated title.
                let clearance = theme.metrics.control_small
                    * match explicit {
                        true => 2.,
                        false => 1.,
                    }
                    + theme.metrics.pad;
                div().w_full().flex().justify_center().px(clearance).child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .min_w_0()
                        .min_h(theme.text(ui::Text::Body) * ui::LEADING)
                        .child(title)
                        .child(
                            actions
                                .absolute()
                                .left(gpui::relative(1.))
                                .ml(theme.metrics.pad * 0.5)
                                .top_0()
                                .bottom_0(),
                        ),
                )
            }
            false => div()
                .w_full()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1()
                .child(title)
                .child(actions),
        };
        div()
            .flex()
            .flex_col()
            .when(centered, |this| {
                this.items_center().line_height(gpui::relative(ui::LEADING))
            })
            .justify_center()
            .flex_1()
            .min_w_0()
            .child(headline)
            .when_some(track, |this, track| {
                this.child(
                    InlineLinks::new(
                        "now-playing-artist",
                        track.artist_refs.iter().map(|artist| {
                            InlineLink::new(artist.name.clone(), artist.id.clone().map(Into::into))
                        }),
                        track.artists.clone(),
                        muted,
                    )
                    .text_size(artists)
                    .when(centered, |this| this.justify_center())
                    .truncate()
                    .on_click(|id, cx| {
                        navigate(Destination::Artist(id), cx);
                    }),
                )
            })
    }
}

impl PlayerBar {
    /// Centered needs room for its three columns; smaller windows keep every control reachable
    /// in the shared responsive layout. Only the whole window's width determines the choice.
    fn stacked(window: &Window, layout: PlayerBarLayout) -> bool {
        let room = match layout {
            PlayerBarLayout::Centered => Room::Wide,
            _ => Room::Roomy,
        };
        !Chrome::window_room(window).fits(room)
    }

    /// The volume slider keeps its full width until the window is as narrow as the one that
    /// stacks the classic bar, so a layout that stacks sooner still matches it.
    fn volume_width(window: &Window) -> Pixels {
        match Chrome::window_room(window).fits(Room::Roomy) {
            true => px(VOLUME_WIDTH),
            false => px(VOLUME_TIGHT),
        }
    }

    /// How much of the content's bottom edge the strip above the bar covers right now, which
    /// is nothing while no Jam or other device is being shown.
    pub(crate) fn strip(playback: &Playback, cx: &gpui::App) -> Pixels {
        let devices = Sonora::global(cx).devices.read(cx);
        match (devices.jam().is_some() && devices.available()) || playback.steered(cx).is_some() {
            true => px(BANNER),
            false => Pixels::ZERO,
        }
    }

    /// The height the workspace reserves, shared by the overlay's bottom scroll clearance.
    pub(crate) fn height(window: &Window, cx: &gpui::App) -> Pixels {
        let theme = *cx.theme();
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        let height = match (Self::stacked(window, layout), layout) {
            (true, _) => theme.metrics.player_bar + theme.metrics.pad * 3.,
            (false, PlayerBarLayout::Compact) => theme.metrics.row + theme.metrics.pad * 2.,
            _ => theme.metrics.player_bar,
        };
        ui::snapped(height, window)
    }
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let empty = muted.opacity(0.3);
        let span = Chrome::window_room(window);
        let layout = self.settings.read(cx).player_bar_layout();
        let stacked = Self::stacked(window, layout);
        let transparent = self.settings.read(cx).player_bar_translucent();
        let height = Self::height(window, cx);
        let clock_text = theme.text(ui::Text::Tiny);
        // Equal wings keep the title centered while reserving the original volume slider width.
        let centered_wing = theme.metrics.control * 4. + px(VOLUME_WIDTH) + theme.metrics.pad * 3.;

        let show_track = span.fits(Room::Snug);
        let sides = match SidebarRight::available(window) {
            true => Some(self.side_buttons(cx)),
            false => None,
        };

        let playback = self.playback.read(cx);
        let seekable = playback.shown_track(cx).is_some();
        let (elapsed, total) = playback.shown_time(cx);
        let progress = self.pending.unwrap_or_else(|| fraction(elapsed, total));
        let clock_width = clock_text
            * match total.as_secs() >= 3600 {
                true => CLOCK_LONG,
                false => CLOCK_SHORT,
            };

        let seek_bubble = self
            .over_seek
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));

        let clock_label = |id: &'static str, value: Duration, align_end: bool| {
            ui::spoken(id, clock(value))
                .w(clock_width)
                .flex_none()
                .whitespace_nowrap()
                .text_size(clock_text)
                .font_features(tabular())
                .text_color(muted)
                .when_else(align_end, |this| this.text_right(), |this| this.text_left())
        };

        let seek = div()
            .id("player-seek")
            .role(Role::Group)
            .aria_label(t!("song-position"))
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(clock_label("player-elapsed", elapsed, true))
            .child(
                div().flex_1().min_w_0().child(
                    Scrubber::new(&self.seek, progress)
                        .name(t!("song-position"))
                        .value(clock(elapsed))
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
            .child(clock_label("player-duration", total, false))
            .into_any_element();

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(self.settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        let base = div()
            .id("player-bar")
            .role(Role::Toolbar)
            .aria_label(t!("settings-page-player-bar"))
            .flex()
            .w_full()
            .h(height)
            .flex_none()
            .px_5()
            .when(stacked, |this| this.py_2())
            .when_some(radius, |this, radius| this.rounded_b(radius))
            .when_else(
                transparent,
                |this| ui::bar_surface(this, cx).text_color(theme.popover_foreground),
                |this| this.when(!theme.transparent, |this| this.bg(theme.secondary)),
            )
            .border_t_1()
            .border_color(theme.border)
            .occlude()
            .on_mouse_move(cx.listener(Self::hover));

        let context_menu = self.context_menu.clone().map(|(track, position)| {
            Popup::new(position, self.track_menu.for_track(&track, cx)).on_close(cx.listener(
                |this, _, _, cx| {
                    this.context_menu = None;
                    cx.notify();
                },
            ))
        });

        let content = match (stacked, layout) {
            (true, _) => base
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
                        .child(transport(&self.playback, false, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .w_full()
                        .child(div().flex_1().min_w_0().child(seek))
                        .children(sides)
                        .child(self.sound(Self::volume_width(window), cx))
                        .child(self.devices.clone())
                        .child(self.fullscreen_button()),
                ),
            (false, PlayerBarLayout::Centered) => base
                .items_center()
                .py(theme.metrics.pad)
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w(centered_wing)
                        .h_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(self.artwork(window, cx))
                        .child(transport(&self.playback, false, cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(px(SEEK_MAX))
                        .h_full()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .child(self.track_details(true, cx).w_full())
                        .child(div().flex_none().w_full().child(seek)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(centered_wing)
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(self.devices.clone())
                        .children(sides)
                        .child(self.sound(px(VOLUME_WIDTH), cx))
                        .child(self.fullscreen_button()),
                ),
            (false, PlayerBarLayout::Compact) => base
                .items_center()
                .gap_3()
                .child(
                    self.now_playing(show_track, window, cx)
                        .max_w(theme.metrics.cover * 1.6),
                )
                .child(transport(&self.playback, false, cx))
                .child(div().flex_1().min_w_0().child(seek))
                .child(self.devices.clone())
                .children(sides)
                .child(self.sound(px(VOLUME_WIDTH), cx))
                .child(self.fullscreen_button()),
            _ => base
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
                        .child(transport(&self.playback, false, cx))
                        .child(seek),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        // never narrower than its buttons, or they spill over the seek bar's clock
                        .flex_1()
                        .child(self.devices.clone())
                        .children(sides)
                        .child(self.sound(px(VOLUME_WIDTH), cx))
                        .child(self.fullscreen_button()),
                ),
        };

        // the faces on the Jam's strip spread apart under the pointer
        let faces =
            window.use_keyed_state("jam-faces", cx, |_, _| HoverFade::new(Motion::Quick.span()));
        let spread = faces.update(cx, |faces, cx| faces.level(window, cx));
        let frosting = ui::blurring(cx);
        let devices = Sonora::global(cx).devices.read(cx);
        // A strip floats over the page's last rows, so it keeps the pointer from reaching them.
        let strip = |id: &'static str| {
            div()
                .id(id)
                .occlude()
                .absolute()
                .left_0()
                .bottom(height)
                .w_full()
                .h(px(BANNER))
                .px_5()
                .flex()
                .items_center()
                .gap_2()
                .text_size(theme.text(ui::Text::Small))
                .text_color(theme.foreground)
                .when_else(
                    frosting,
                    |this| ui::bar_surface(this, cx),
                    |this| this.bg(theme.popover),
                )
        };
        let banner = devices.jam().filter(|_| devices.available()).map(|jam| {
            let text = match jam::host_name(jam) {
                Some(host) => t!("devices-jam-banner", host = host),
                None => t!("devices-jam-banner-anonymous"),
            };
            // The people list rises from the strip's top edge, over the faces that open it.
            let top = window.viewport_size().height - height - px(BANNER);
            let controls = Sonora::global(cx).devices.clone();
            let anchor = self.faces_at.clone();
            let measured = self.faces_at.clone();
            strip("jam-banner")
                .role(Role::Status)
                .aria_label(text.clone())
                .child(
                    gpui::svg()
                        .path(icons::path("icons/users-round-collaboration.svg"))
                        .size(px(BANNER_ICON))
                        .flex_none()
                        .text_color(theme.foreground),
                )
                .child(div().min_w_0().truncate().child(text.clone()))
                .child(
                    div()
                        .id("jam-people-open")
                        .role(Role::Button)
                        .aria_label(t!("devices-jam-people", count = jam.members.len()))
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .p(ui::snapped(px(FACES_PAD), window))
                        .rounded_full()
                        .cursor_pointer()
                        .bg(theme.secondary_hover.opacity(spread))
                        .on_hover({
                            let faces = faces.clone();
                            move |hovered, window, cx| {
                                faces.update(cx, |faces, _| faces.set(*hovered));
                                window.refresh();
                            }
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let left = anchor.get().map_or(Pixels::ZERO, |bounds| bounds.left());
                            this.people
                                .update(cx, |people, cx| people.show(point(left, top), cx));
                        }))
                        .child(jam::faces(
                            &jam.members,
                            BANNER_FACES,
                            px(BANNER_FACE),
                            spread,
                            window,
                            theme,
                        ))
                        .child(
                            canvas(
                                move |bounds, _, _| measured.set(Some(bounds)),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        ),
                )
                // the faces follow the Jam's name, and its buttons keep to the right edge
                .child(div().flex_1())
                .when_some(jam.link.clone(), |this, link| {
                    this.child(
                        Button::new("jam-code")
                            .ghost()
                            .small()
                            .icon("icons/qr-code.svg")
                            .tooltip_above("devices-jam-code")
                            .on_click({
                                let devices = controls.clone();
                                move |_, _, cx| {
                                    devices.update(cx, |devices, cx| devices.show_code(true, cx))
                                }
                            }),
                    )
                    .child(
                        Button::new("jam-copy")
                            .ghost()
                            .small()
                            .icon("icons/copy.svg")
                            .tooltip_above("devices-jam-copy")
                            .on_click(move |_, _, cx| jam::copy_link(&link, cx)),
                    )
                })
                .child(
                    Button::new("jam-leave")
                        .ghost()
                        .small()
                        .icon("icons/log-out.svg")
                        .label(match jam.owner {
                            true => t!("devices-jam-end"),
                            false => t!("devices-jam-leave"),
                        })
                        .on_click(move |_, _, cx| {
                            controls.update(cx, |devices, cx| devices.leave(cx))
                        }),
                )
        });
        // a Jam's strip names the device already, so the plain one is for when there is no Jam
        let banner = banner.or_else(|| {
            self.playback.read(cx).steered(cx).map(|steered| {
                let devices = Sonora::global(cx).devices.clone();
                strip("elsewhere-banner")
                    .role(Role::Status)
                    .aria_label(t!(
                        "devices-elsewhere-banner",
                        device = steered.device.name.clone()
                    ))
                    .child(
                        gpui::svg()
                            .path(icons::path("icons/monitor-speaker.svg"))
                            .size(px(BANNER_ICON))
                            .flex_none()
                            .text_color(theme.foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(t!("devices-elsewhere-banner", device = steered.device.name)),
                    )
                    .child(
                        Button::new("elsewhere-here")
                            .ghost()
                            .small()
                            .label(t!("devices-elsewhere-here"))
                            .on_click(move |_, _, cx| devices.read(cx).transfer_here()),
                    )
            })
        });

        content
            .relative()
            .when_some(banner, |this, banner| this.child(banner))
            .when_some(context_menu, |this, menu| this.child(menu))
            .child(self.people.clone())
    }
}
