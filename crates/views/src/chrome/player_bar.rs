use router::{Destination, Link, navigate};
use std::time::Duration;
use ui::ActiveTheme as _;

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, Entity, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent, SharedString,
};
use gpui::{Window, div, point, px};
use i18n::t;
use input::{ToggleFullscreen, ToggleLyrics, ToggleQueue};
use state::{AppSettings, Playback, Queue, SideTab, Sonora};
use ui::{
    Artwork, Button, ExplicitBadge, InlineLink, InlineLinks, Popup, Room, Scrollbar, Scrubber,
    ScrubberState, clock, tabular,
};

use crate::chrome::SidebarRight;
use crate::chrome::devices::DevicePicker;
use crate::chrome::jam::{self, JamPeople};
use crate::shared::menus::ItemMenu;
use crate::shared::transport::{NOTCH, fraction, like, moved, percent, transport, volume_icon};

const SEEK_MAX: f32 = 560.;
const VOLUME_WIDTH: f32 = 110.;
const VOLUME_TIGHT: f32 = 72.;
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
/// The height of the strip above the bar that names the Jam being listened to, or the other
/// device playback is on.
const BANNER: f32 = 28.;
const BANNER_ICON: f32 = 14.;
/// The width of a member's picture on the strip, and how many of them it shows before it counts
/// the rest.
const BANNER_FACE: f32 = 20.;
/// How much of the popover colour the strip keeps over its blur, as a menu does, so what it
/// covers does not read through the text.
const BANNER_FILL: f32 = 0.75;
const BANNER_FACES: usize = 4;

pub(crate) struct PlayerBar {
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    settings: Entity<AppSettings>,
    track_menu: ItemMenu,
    devices: Entity<DevicePicker>,
    people: Entity<JamPeople>,
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
        let picker = cx.new(|cx| DevicePicker::new(people.clone(), cx));

        Self {
            playback,
            queue,
            settings,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            devices: picker,
            people,
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
            .child(self.devices.clone())
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

    fn now_playing(&self, room: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let artwork = ui::snapped(theme.metrics.row, window);
        let artists = theme.text(ui::Text::Small);
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let cover = track.and_then(|track| track.cover.clone());
        let explicit = track.is_some_and(|track| track.explicit);
        let like = like(track, &self.playback, cx);

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
                        track.and_then(|track| track.album_id.clone()),
                        |this, album| this.link(Destination::Album(album.into())),
                    )
                    .when(track.is_some(), |this| {
                        this.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                window.prevent_default();
                                this.open_playing_menu(event.position, cx);
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
                                .child(match track {
                                    Some(track) => div()
                                        .id("now-playing-track")
                                        .when_some(track.album_id.clone(), |this, album_id| {
                                            this.hover(|style| style.underline())
                                                .link(Destination::Album(album_id.into()))
                                        })
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            cx.listener(
                                                move |this, event: &MouseDownEvent, window, cx| {
                                                    window.prevent_default();
                                                    this.open_playing_menu(event.position, cx);
                                                },
                                            ),
                                        )
                                        .child(SharedString::from(track.name.clone()))
                                        .min_w_0()
                                        .truncate(),
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
                        .when_some(track, |this, track| {
                            this.child(
                                InlineLinks::new(
                                    "now-playing-artist",
                                    track.artist_refs.iter().map(|artist| {
                                        InlineLink::new(
                                            artist.name.clone(),
                                            artist.id.clone().map(Into::into),
                                        )
                                    }),
                                    track.artists.clone(),
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
    /// How much of the content's bottom edge the strip above the bar covers right now, which
    /// is nothing while no Jam or other device is being shown.
    pub(crate) fn strip(playback: &Playback, cx: &gpui::App) -> Pixels {
        let devices = Sonora::global(cx).devices.read(cx);
        match (devices.jam().is_some() && devices.available()) || playback.steered(cx).is_some() {
            true => px(BANNER),
            false => Pixels::ZERO,
        }
    }

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

        let clock_label = |value: Duration, align_end: bool| {
            div()
                .child(clock(value))
                .w(clock_width)
                .flex_none()
                .whitespace_nowrap()
                .text_size(clock_text)
                .font_features(tabular())
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

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(self.settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        let base = div()
            .flex()
            .w_full()
            .h(height)
            .flex_none()
            .px_5()
            .when(stacked, |this| this.py_2())
            .when_some(radius, |this, radius| this.rounded_b(radius))
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
                        // never narrower than its buttons, or they spill over the seek bar's clock
                        .flex_1()
                        .children(sides)
                        .child(self.sound(px(VOLUME_WIDTH), cx))
                        .child(self.fullscreen_button()),
                ),
        };

        let frosting = ui::blurring(cx);
        let fill = match frosting {
            true => theme.popover.opacity(BANNER_FILL),
            false => theme.popover,
        };
        let devices = Sonora::global(cx).devices.read(cx);
        let strip = |id: &'static str| {
            div()
                .id(id)
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
                .bg(fill)
                .when(frosting, ui::frost)
        };
        let banner = devices.jam().filter(|_| devices.available()).map(|jam| {
            let device = match devices.steered() {
                Some(steered) => steered.device.name,
                None => t!("devices-this-computer").to_string(),
            };
            let text = match jam::host_name(jam) {
                Some(host) => t!("devices-jam-banner", host = host, device = device),
                None => t!("devices-jam-banner-anonymous", device = device),
            };
            // The people list ends where the strip starts, so it rises from the strip's top edge.
            let top = window.viewport_size().height - height - px(BANNER);
            let controls = Sonora::global(cx).devices.clone();
            strip("jam-banner")
                .occlude()
                // the name and faces open the people list, and the buttons beside them act on the Jam
                .child(
                    div()
                        .id("jam-people-open")
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.secondary_hover))
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            let at = point(event.position().x, top);
                            this.people.update(cx, |people, cx| people.show(at, cx));
                        }))
                        .child(
                            gpui::svg()
                                .path(icons::path("icons/user.svg"))
                                .size(px(BANNER_ICON))
                                .flex_none()
                                .text_color(theme.foreground),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(text))
                        .child(jam::faces(
                            &jam.members,
                            BANNER_FACES,
                            px(BANNER_FACE),
                            theme,
                        )),
                )
                .when_some(jam.link.clone(), |this, link| {
                    this.child(
                        Button::new("jam-code")
                            .ghost()
                            .small()
                            .label(t!("devices-jam-code"))
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
                            .label(t!("devices-jam-copy"))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(link.clone()))
                            }),
                    )
                })
                .child(
                    Button::new("jam-leave")
                        .ghost()
                        .small()
                        .label(match jam.owner {
                            true => t!("devices-jam-end"),
                            false => t!("devices-jam-leave"),
                        })
                        .on_click(move |_, _, cx| controls.read(cx).leave()),
                )
        });
        // a Jam's strip names the device already, so the plain one is for when there is no Jam
        let banner = banner.or_else(|| {
            self.playback.read(cx).steered(cx).map(|steered| {
                let devices = Sonora::global(cx).devices.clone();
                strip("elsewhere-banner")
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
