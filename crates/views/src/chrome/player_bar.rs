use router::{Destination, Link, navigate};
use ui::ActiveTheme as _;
use ui::Tails as _;
use ui::Tipped as _;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Render, SharedString, StyledText,
};
use gpui::{Window, deferred, div, px};
use i18n::t;
use input::{ShowMembers, ToggleFullscreen, ToggleLyrics, ToggleQueue};
use state::{AppSettings, Playback, PlayerBarLayout, Queue, SideTab, Sonora};
use ui::{
    Artwork, Button, ExplicitBadge, Flight, InlineLink, InlineLinks, Perch, Pinnable, Popup, Room,
    Scrollbar, Scrubber, ScrubberState, clock, tabular,
};

use crate::chrome::sessions_button::SessionsButton;
use crate::chrome::{Chrome, SidebarRight};
use crate::shared::menus::ItemMenu;
use crate::shared::pins::Pinned as _;
use crate::shared::transport::{
    COVER_FLIGHT, CoverAway, ShellSwap, fraction, like, moved, percent, transport, volume_icon,
};
use crate::shared::volume::{RisingVolume, VolumeHost, rising_volume, turn_volume};

const SEEK_MAX: f32 = 560.;
const VOLUME_WIDTH: f32 = 110.;
/// The least a volume slider that gives way to the seek bar shrinks to.
const VOLUME_TIGHT: f32 = 72.;
/// How wide a small icon button draws: `SMALL_INSET` on each side of a 16 px icon. Every button
/// on the classic bar's right side is one, the sessions picker included.
const ICON_BUTTON: f32 = 32.;
/// The classic bar's spacing in rems: its `px_5` at either end, its `gap_4` between the three
/// columns, the right side's `gap_2` between its groups, the `gap_1` between the panel buttons
/// and the volume group's `gap_1` between the mute button and the slider.
const BAR_PAD: f32 = 1.25;
const COLUMN_GAP: f32 = 1.;
const SIDE_GAP: f32 = 0.5;
const TOOL_GAP: f32 = 0.25;
const VOLUME_GAP: f32 = 0.25;
/// The width the classic bar's middle column holds while the volume slider gives way beside
/// it, the least it shrinks to before the volume folds into a button, and the least it takes
/// once the volume has folded, about the transport's own width over a short seek bar.
const SEEK_KEEP: f32 = 300.;
const SEEK_LEAST: f32 = 260.;
const SEEK_FLOOR: f32 = 220.;
/// The room a side of the classic bar keeps for the track's title once the volume has folded,
/// as a share of the cover metric.
const TITLE_ROOM: f32 = 1.4;
/// The stacked bar's `gap_2` between its two rows, in rems.
const STACK_GAP: f32 = 0.5;
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
/// The extra room both clocks take, in ems, while the right one leads with a minus sign.
const CLOCK_SIGN: f32 = 0.6;

/// How the bar draws its volume control.
#[derive(Clone, Copy)]
enum Volume {
    /// The mute button with a slider this wide beside it.
    Slider(Pixels),
    /// The mute button alone, with a slider that rises over it on hover.
    Rising,
}

pub(crate) struct PlayerBar {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    track_menu: ItemMenu,
    sessions: Entity<SessionsButton>,
    context_menu: Option<(music::Track, Point<Pixels>)>,
    seek: ScrubberState,
    volume: ScrubberState,
    /// The mute memory and the drag both volume controls share, and the slider that rises
    /// over the button where the bar has no room for one beside it.
    rising: RisingVolume,
    pending: Option<f32>,
    over_seek: Option<f32>,
    over_volume: Option<f32>,
}

impl VolumeHost for PlayerBar {
    fn volume(&mut self) -> &mut RisingVolume {
        &mut self.rising
    }

    fn playback(&self) -> &Entity<Playback> {
        &self.playback
    }
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
        cx.observe_global::<ShellSwap>(|_, cx| cx.notify()).detach();

        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));
        let sessions = cx.new(SessionsButton::new);

        Self {
            playback,
            settings,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            sessions,
            context_menu: None,
            seek: ScrubberState::new("seek"),
            volume: ScrubberState::new("volume"),
            pending: None,
            over_seek: None,
            over_volume: None,
            rising: RisingVolume::new("volume-rise"),
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

    /// The volume control, drawn the way `volume` says.
    fn sound(&self, volume: Volume, cx: &mut Context<Self>) -> AnyElement {
        match volume {
            Volume::Slider(width) => self.slider(width, cx).into_any_element(),
            Volume::Rising => {
                rising_volume(&self.rising, &self.playback, false, cx).into_any_element()
            }
        }
    }

    /// The mute button with a volume slider `width` wide beside it.
    fn slider(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let empty = theme.muted_foreground.opacity(0.3);
        let level = self.playback.read(cx).shown_volume(cx);
        let showing = self.over_volume.is_some() || self.rising.held;
        let bubble = showing.then(|| (level, percent(level)));
        let restore = self.rising.muted.unwrap_or(0.7);

        div()
            .id("player-volume")
            .role(Role::Group)
            .aria_label(t!("equalizer-status-volume"))
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .on_scroll_wheel(cx.listener(turn_volume))
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
                        this.rising.muted = match wanted {
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
                            this.rising.held = true;
                            this.rising.muted = None;
                            this.playback
                                .update(cx, |playback, cx| playback.slide_volume(level, cx));
                            cx.notify();
                        }))
                        .on_release(cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.rising.held = false;
                            this.playback
                                .update(cx, |playback, cx| playback.end_volume_slide(cx));
                            cx.notify();
                        })),
                ),
            )
    }

    fn side_buttons(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let (open, tab, lyrics) = {
            let settings = self.settings.read(cx);
            (
                settings.sidebar_right_open(),
                settings.sidebar_right_tab(),
                settings.lyrics_enabled(),
            )
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
                        SideTab::Members => Box::new(ShowMembers),
                    };
                    window.dispatch_action(action, cx);
                })
        };

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .when(lyrics, |this| {
                this.child(button(
                    "player-lyrics",
                    "icons/mic-vocal.svg",
                    "lyrics-title",
                    SideTab::Lyrics,
                ))
            })
            .child(button(
                "player-queue",
                "icons/list-music.svg",
                "queue-title",
                SideTab::Queue,
            ))
            .into_any_element()
    }

    /// The button that opens the fullscreen player. It takes over from the player's leave
    /// button the moment the player starts closing and paints above its fade, and it paints
    /// nothing while the player opens over it.
    fn fullscreen_button(&self, cx: &gpui::App) -> AnyElement {
        let button = Button::new("toggle-fullscreen")
            .ghost()
            .small()
            .icon("icons/maximize.svg")
            .tooltip_above("player-fullscreen")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleFullscreen), cx));
        match cx.try_global::<ShellSwap>().copied().unwrap_or_default() {
            ShellSwap::Closing => deferred(button).into_any_element(),
            ShellSwap::Opening => button.invisible().into_any_element(),
            ShellSwap::Settled => button.into_any_element(),
        }
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
        let away = cx.try_global::<CoverAway>().is_some_and(|away| away.0);
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let cover = track.and_then(|track| track.cover.clone());
        let album = track.map(|track| match track.album.is_empty() {
            true => t!("song-view-album"),
            false => SharedString::from(track.album.clone()),
        });
        div()
            .id("now-playing-artwork")
            .when_some(track.and_then(|track| track.pin()), Pinnable::pin)
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
            .child(
                // The fullscreen player's cover flies to and from this one, which stays undrawn
                // while that one is in the air.
                Flight::new(
                    "now-playing-cover",
                    div()
                        .when(away, |this| this.invisible())
                        .child(Artwork::new(cover).size(artwork)),
                )
                .seen_as(COVER_FLIGHT),
            )
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
            Some(track) => {
                let name = SharedString::from(track.name.clone());
                let text = StyledText::new(name.clone());
                let layout = text.layout().clone();
                div()
                    .id("now-playing-track")
                    .when_some(track.pin(), Pinnable::pin)
                    .map(|this| match track.album_id.clone() {
                        Some(album_id) => this
                            .aria_label(name.clone())
                            .hover(|style| style.underline())
                            .link(Destination::Album(album_id.into())),
                        None => this.role(Role::Label).aria_value(name.clone()),
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.prevent_default();
                            this.open_playing_menu(event.position, cx);
                        }),
                    )
                    .child(text)
                    .tip_clipped(name, layout, Perch::Above)
                    .min_w_0()
                    .truncate()
                    .tails()
            }
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
    /// Splits the classic bar's row: how wide the seek bar's column in the middle is, and how
    /// the volume draws. The two sides always share what is left evenly, so the middle stays
    /// centred. While the middle has `SEEK_KEEP` the row splits in even thirds. Narrower, the
    /// volume slider gives way first, down to `VOLUME_TIGHT`, then the middle, down to
    /// `SEEK_LEAST`, and only then does the volume fold into a button whose slider rises on
    /// hover. From there the middle gives the track's title `TITLE_ROOM`, down to
    /// `SEEK_FLOOR`. `tools` counts the icon buttons before the volume.
    fn classic(window: &Window, tools: usize, cx: &gpui::App) -> (Pixels, Volume) {
        let title = cx.theme().metrics.cover * TITLE_ROOM;
        let rem = window.rem_size();
        let button = px(ICON_BUTTON);
        let row = window.viewport_size().width - rem * (BAR_PAD + COLUMN_GAP) * 2.;
        let tools = button * tools as f32 + rem * TOOL_GAP * tools.saturating_sub(1) as f32;
        // The right side without its slider: the buttons, the mute and fullscreen buttons and
        // the gaps between them.
        let rising = tools + button * 2. + rem * SIDE_GAP * 2.;
        let around = rising + rem * VOLUME_GAP;
        let keep = (row / 3.).max(px(SEEK_KEEP)).min(px(SEEK_MAX));
        let slider = (row - keep) / 2. - around;
        if slider >= px(VOLUME_TIGHT) {
            return (keep, Volume::Slider(slider.min(px(VOLUME_WIDTH))));
        }
        let middle = row - (around + px(VOLUME_TIGHT)) * 2.;
        if middle >= px(SEEK_LEAST) {
            return (middle, Volume::Slider(px(VOLUME_TIGHT)));
        }
        let middle = (row - title * 2.)
            .max(px(SEEK_FLOOR))
            .min(keep)
            .min(row - rising * 2.);
        (middle, Volume::Rising)
    }

    /// The volume slider keeps its full width until the window is as narrow as the one that
    /// stacks the classic bar, so a layout that stacks sooner still matches it.
    fn volume_width(window: &Window) -> Pixels {
        match Chrome::window_room(window).fits(Room::Roomy) {
            true => px(VOLUME_WIDTH),
            false => px(VOLUME_TIGHT),
        }
    }

    /// Centered needs room for its three columns; smaller windows keep every control reachable
    /// in the shared responsive layout. Only the whole window's width determines the choice.
    fn stacked(window: &Window, layout: PlayerBarLayout) -> bool {
        let room = match layout {
            PlayerBarLayout::Centered => Room::Wide,
            _ => Room::Roomy,
        };
        !Chrome::window_room(window).fits(room)
    }

    /// How far below its place in a full-height bar the fullscreen button sits, which is where
    /// a stacked or compact bar puts it. The fullscreen player drops its own corner by as
    /// much, so the button stays under the pointer when the player opens and closes.
    pub(crate) fn fullscreen_drop(window: &Window, cx: &gpui::App) -> Pixels {
        let theme = *cx.theme();
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        let spare = theme.metrics.player_bar - Self::height(window, cx);
        let drop = match Self::stacked(window, layout) {
            // The button's row is centred under the cover's row and the gap between the two.
            true => spare + theme.metrics.row + window.rem_size() * STACK_GAP,
            false => spare,
        };
        ui::snapped((drop / 2.).max(Pixels::ZERO), window)
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
        // the Sessions button stands with the panel buttons, one gap apart like them, and takes no
        // gap of its own while it draws nothing or the listener hid it
        let picker = self.settings.read(cx).sessions_button()
            && (Sonora::global(cx).devices.read(cx).available()
                || Sonora::global(cx).listening.read(cx).available(cx));
        let tools = usize::from(picker)
            + match sides.is_some() {
                true => 1 + usize::from(self.settings.read(cx).lyrics_enabled()),
                false => 0,
            };
        let buttons = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .when(picker, |this| this.child(self.sessions.clone()))
            .children(sides);

        let playback = self.playback.read(cx);
        let seekable = playback.shown_track(cx).is_some();
        let (elapsed, total) = playback.shown_time(cx);
        let progress = self.pending.unwrap_or_else(|| fraction(elapsed, total));
        let time_left = self.settings.read(cx).player_bar_time_left();
        let clock_width = clock_text
            * (match total.as_secs() >= 3600 {
                true => CLOCK_LONG,
                false => CLOCK_SHORT,
            } + match time_left {
                true => CLOCK_SIGN,
                false => 0.,
            });

        let seek_bubble = self
            .over_seek
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));

        let clock_label = |id: &'static str, text: SharedString, align_end: bool| {
            ui::spoken(id, text)
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
            .child(clock_label("player-elapsed", clock(elapsed), true))
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
            .child(match time_left {
                true => {
                    let left = clock(total.saturating_sub(elapsed));
                    clock_label(
                        "player-time-left",
                        t!("player-time-left", time = &left),
                        false,
                    )
                    .aria_value(t!("player-time-left-spoken", time = &left))
                }
                false => clock_label("player-duration", clock(total), false),
            })
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
                        .child(buttons)
                        .child(self.sound(Volume::Slider(Self::volume_width(window)), cx))
                        .child(self.fullscreen_button(cx)),
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
                        .child(buttons)
                        .child(self.sound(Volume::Slider(px(VOLUME_WIDTH)), cx))
                        .child(self.fullscreen_button(cx)),
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
                .child(buttons)
                .child(self.sound(Volume::Slider(px(VOLUME_WIDTH)), cx))
                .child(self.fullscreen_button(cx)),
            // The two sides share what the middle leaves them evenly, so the transport and the
            // seek bar stay centred. `classic` works out how wide the middle is and how the
            // volume draws.
            _ => {
                let (middle, volume) = Self::classic(window, tools, cx);
                base.items_center()
                    .gap_4()
                    .child(self.now_playing(show_track, window, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .flex_none()
                            .w(middle)
                            .child(transport(&self.playback, false, cx))
                            .child(seek),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(buttons)
                            .child(self.sound(volume, cx))
                            .child(self.fullscreen_button(cx)),
                    )
            }
        };

        content
            .relative()
            .when_some(context_menu, |this, menu| this.child(menu))
    }
}
