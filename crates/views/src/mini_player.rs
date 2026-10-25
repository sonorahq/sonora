use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    App, AvailableSpace, Context, Div, Entity, FocusHandle, MouseButton, MouseMoveEvent, Pixels,
    Render, Task, Window, WindowControlArea, canvas, div, point, size,
};
use i18n::t;
use input::{CloseWindow, ShowMainWindow, WORKSPACE_CONTEXT};
use state::{Cover, Playback, SideTab, Sonora};
use ui::{ActiveTheme as _, Artwork, Button, TabBar, Text};

use crate::chrome::Aside;
use crate::chrome::timeline::{Released, Timeline};
use crate::root::ui_font;
use crate::shared::transport::{next, previous, toggle};
use crate::shared::volume::{RisingVolume, VolumeHost, rising_volume};

const REST: Duration = Duration::from_secs(2);

/// A separate view of the shared player with its own lyrics scroll position.
pub struct MiniPlayer {
    playback: Entity<Playback>,
    cover: Entity<Cover>,
    aside: Entity<Aside>,
    panel: Option<SideTab>,
    timeline: Entity<Timeline>,
    volume: RisingVolume,
    settled: bool,
    rest: Option<Task<()>>,
    focus: FocusHandle,
    grabbed: bool,
}

impl VolumeHost for MiniPlayer {
    fn volume(&mut self) -> &mut RisingVolume {
        &mut self.volume
    }
    fn playback(&self) -> &Entity<Playback> {
        &self.playback
    }
}

impl MiniPlayer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sonora = Sonora::global(cx);
        let playback = sonora.playback.clone();
        let queue = sonora.queue.clone();
        let cover = sonora.cover.clone();
        let settings = sonora.settings.clone();
        let devices = sonora.devices.clone();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        cx.observe(&queue, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&devices, |_, _, cx| cx.notify()).detach();
        cx.observe(&cover, |_, _, cx| cx.notify()).detach();
        let aside = cx.new(|cx| {
            let mut lyrics = Aside::new(queue.clone(), playback.clone(), SideTab::Lyrics, cx);
            lyrics.strip();
            lyrics
        });
        let timeline = cx.new(|cx| Timeline::new(playback.clone(), cx));
        cx.subscribe(&timeline, |this, _, _: &Released, cx| this.wake(cx))
            .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut player = Self {
            playback,
            cover,
            aside,
            panel: None,
            timeline,
            volume: RisingVolume::new("mini-volume"),
            settled: false,
            rest: None,
            focus,
            grabbed: false,
        };
        player.wake(cx);
        player
    }

    /// Restarts the artwork's idle delay. An active scrubber or volume popup keeps controls up.
    fn wake(&mut self, cx: &mut Context<Self>) {
        if self.settled {
            self.settled = false;
            cx.notify();
        }
        self.rest = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REST).await;
            this.update(cx, |this, cx| {
                this.rest = None;
                if this.panel.is_none()
                    && !this.timeline.read(cx).dragging()
                    && !this.grabbed
                    && !this.volume.open()
                {
                    this.settled = true;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn track_details(&self, cx: &App) -> Div {
        let theme = *cx.theme();
        let track = self.playback.read(cx).shown_track(cx);
        let title = track.as_ref().map_or_else(
            || t!("player-nothing-playing"),
            |track| track.name.clone().into(),
        );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(div().truncate().child(title))
            .when_some(track, |this, track| {
                this.child(
                    div()
                        .truncate()
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground)
                        .child(track.artists),
                )
            })
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .p_2()
            .child(
                div()
                    .id("mini-drag")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h(theme.metrics.control_small)
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.grabbed = true),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.grabbed = false;
                            this.wake(cx);
                        }),
                    )
                    .on_mouse_down_out(cx.listener(|this, _, _, _| this.grabbed = false))
                    .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, window, _| {
                        if this.grabbed {
                            this.grabbed = false;
                            window.start_window_move();
                        }
                    }))
                    .child(self.track_details(cx)),
            )
            .child(
                Button::new("mini-main")
                    .ghost()
                    .small()
                    .icon("icons/maximize.svg")
                    .tooltip("mini-player-main")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowMainWindow), cx)),
            )
            .child(
                Button::new("mini-close")
                    .ghost()
                    .small()
                    .icon("icons/x.svg")
                    .tooltip("app-close-window")
                    .on_click(|_, window, _| window.remove_window()),
            )
    }

    fn modes(&self, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let tab = |id, icon, hint, panel| {
            let selected = self.panel == panel;
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip_above(hint)
                .selected(selected)
                .rounded(theme.radius)
                .tint(match selected {
                    true => theme.foreground,
                    false => theme.muted_foreground,
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.panel = panel;
                    if let Some(tab) = panel {
                        this.aside.update(cx, |aside, cx| aside.show(tab, cx));
                    }
                    this.wake(cx);
                    cx.notify();
                }))
        };
        div().flex().flex_none().justify_center().p_2().child(
            TabBar::new("mini-modes")
                .name(t!("fullscreen-panels"))
                .rounded(theme.radius + theme.metrics.pad * 0.25)
                .items([
                    tab(
                        "mini-artwork",
                        "icons/disc-3.svg",
                        "fullscreen-artwork",
                        None,
                    ),
                    tab(
                        "mini-lyrics",
                        "icons/mic-vocal.svg",
                        "lyrics-title",
                        Some(SideTab::Lyrics),
                    ),
                    tab(
                        "mini-queue",
                        "icons/list-music.svg",
                        "queue-title",
                        Some(SideTab::Queue),
                    ),
                ]),
        )
    }

    /// Uses the same large cover as Now Playing while preserving embedded local artwork.
    fn artwork(&self, fill: bool, cx: &App) -> impl IntoElement {
        let theme = *cx.theme();
        let cover = self.playback.read(cx).shown_track(cx).and_then(|track| {
            let local = track.id.as_deref().is_some_and(music::is_local_id)
                || track
                    .cover
                    .as_deref()
                    .is_some_and(|url| url.starts_with("file://"));
            match local {
                true => track.cover,
                false => track
                    .album_id
                    .as_deref()
                    .and_then(|album| self.cover.read(cx).large_for(album))
                    .map(str::to_owned)
                    .or(track.cover),
            }
        });
        canvas(
            move |bounds, window, cx| {
                let side = match fill {
                    true => bounds.size.width.max(bounds.size.height),
                    false => (bounds.size.width.min(bounds.size.height) - theme.metrics.pad * 2.)
                        .max(Pixels::ZERO),
                };
                let mut artwork = Artwork::new(cover)
                    .size(side)
                    .flex_none()
                    .when(fill, |this| this.corner_radius(Pixels::ZERO))
                    .into_any_element();
                artwork.layout_as_root(
                    size(
                        AvailableSpace::Definite(side),
                        AvailableSpace::Definite(side),
                    ),
                    window,
                    cx,
                );
                artwork.prepaint_at(
                    bounds.origin
                        + point(
                            (bounds.size.width - side) / 2.,
                            (bounds.size.height - side) / 2.,
                        ),
                    window,
                    cx,
                );
                artwork
            },
            |_, mut artwork, window, cx| artwork.paint(window, cx),
        )
        .size_full()
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap_3()
            .p_2()
            .child(previous(&self.playback, false, cx))
            .child(toggle(&self.playback, false, false, cx))
            .child(next(&self.playback, false, cx))
            .child(rising_volume(&self.volume, &self.playback, false, cx))
    }
}

impl Render for MiniPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        window.set_rem_size(theme.font_size);
        let font = ui_font(Sonora::global(cx).settings.read(cx).font());
        let artwork_only = self.panel.is_none() && self.settled;
        let root = div()
            .id("mini-player")
            .relative()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font(font)
            .text_size(theme.font_size)
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_hover(cx.listener(|this, _: &bool, _, cx| this.wake(cx)))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.timeline
                    .update(cx, |timeline, cx| timeline.hover(event.position, cx));
                this.wake(cx);
            }))
            .on_key_down(cx.listener(|this, _, _, cx| this.wake(cx)))
            .when(artwork_only, |this| this.child(self.artwork(true, cx)))
            .when(!artwork_only, |this| {
                this.child(self.header(cx))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .when(self.panel.is_some(), |this| this.child(self.aside.clone()))
                            .when(self.panel.is_none(), |this| {
                                this.child(self.artwork(false, cx))
                            }),
                    )
                    .child(self.modes(cx))
                    .child(div().px_3().child(self.timeline.clone()))
                    .child(self.controls(cx))
            });
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let root = root.child(ui::WindowFrame::new());
        root
    }
}
