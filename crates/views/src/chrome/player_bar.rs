use router::{Destination, Link};
use ui::ActiveTheme as _;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point,
    Render, SharedString, Stateful,
};
use gpui::{Div, Window, deferred, div, px, rems};
use i18n::t;
use input::{ShowMembers, ToggleFullscreen, ToggleLyrics, ToggleQueue};
use state::{AppSettings, Playback, PlayerBarLayout, Queue, SideTab, Sonora};
use ui::{Artwork, Button, Flight, Pinnable, Popup, Room, Scrollbar, ScrubberState};

use crate::chrome::sessions_button::SessionsButton;
use crate::chrome::timeline::Timeline;
use crate::chrome::{Chrome, SidebarRight};
use crate::chrome::{jam, members};
use crate::shared::menus::ItemMenu;
use crate::shared::now_playing::track_details;
use crate::shared::pins::Pinned as _;
use crate::shared::transport::{COVER_FLIGHT, CoverAway, CoverRoom, ShellSwap, moved, transport};
use crate::shared::volume::{RisingVolume, VolumeHost, rising_volume, volume_slider};

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
/// The gap between the cover and the title in a now playing row, in rems.
pub(crate) const COVER_GAP: f32 = 0.75;
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

/// How the bar draws its volume control.
#[derive(Clone, Copy)]
pub(crate) enum Volume {
    /// The mute button with a slider this wide beside it.
    Slider(Pixels),
    /// The mute button alone, with a slider that rises over it on hover.
    Rising,
}

/// What a player bar lays out in every layout but the centred one. The narrow fullscreen player
/// hands its own pieces to [`PlayerBar::arrange`], so its controls stand exactly where the bar's
/// do. `now_playing` fills the room left of the controls and `corner` ends the bar.
pub(crate) struct BarParts {
    pub(crate) now_playing: AnyElement,
    pub(crate) transport: AnyElement,
    pub(crate) seek: AnyElement,
    /// The sessions and panel buttons, which only the bar itself draws. The classic layout
    /// counts them either way, so the controls around them stay put without them.
    pub(crate) tools: Option<AnyElement>,
    /// The volume control, drawn the way [`PlayerBar::volume`] says.
    pub(crate) sound: AnyElement,
    pub(crate) corner: AnyElement,
}

pub(crate) struct PlayerBar {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    track_menu: ItemMenu,
    sessions: Entity<SessionsButton>,
    context_menu: Option<(music::Track, Point<Pixels>)>,
    /// The clocks and the seek bar, which follow the position without redrawing the bar.
    timeline: Entity<Timeline>,
    volume: ScrubberState,
    /// The mute memory and the drag both volume controls share, and the slider that rises
    /// over the button where the bar has no room for one beside it.
    rising: RisingVolume,
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
        cx.observe_global::<CoverRoom>(|_, cx| cx.notify()).detach();

        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));
        let sessions = cx.new(SessionsButton::new);
        let timeline = cx.new(|cx| Timeline::new(playback.clone(), cx));

        Self {
            playback,
            settings,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            sessions,
            context_menu: None,
            timeline,
            volume: ScrubberState::new("volume"),
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

    fn hover(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let pad = cx.theme().metrics.pad;
        self.timeline
            .update(cx, |timeline, cx| timeline.hover(event.position, cx));
        let volume = self.volume.hovered(event.position, pad);

        if moved(self.over_volume, volume) {
            self.over_volume = volume;
            cx.notify();
        }
    }

    /// The volume control, drawn the way `volume` says.
    fn sound(&self, volume: Volume, cx: &mut Context<Self>) -> AnyElement {
        match volume {
            Volume::Slider(width) => volume_slider(
                &self.volume,
                self.over_volume,
                self.rising.held,
                width,
                &self.playback,
                cx,
            )
            .into_any_element(),
            Volume::Rising => {
                rising_volume(&self.rising, &self.playback, false, cx).into_any_element()
            }
        }
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

    fn now_playing(&self, room: bool, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = match self.playback.read(cx).shown_track(cx) {
            Some(track) => match members::picked_by(cx) {
                Some(added) => SharedString::from(format!(
                    "{} — {}, {}",
                    track.name,
                    track.artists,
                    t!("queue-added-by", name = added.name.as_str())
                )),
                None => SharedString::from(format!("{} — {}", track.name, track.artists)),
            },
            None => t!("player-nothing-playing"),
        };
        div()
            .id("now-playing")
            .role(Role::Group)
            .aria_label(label)
            .flex()
            .items_center()
            .gap(rems(COVER_GAP))
            .flex_1()
            .min_w_0()
            .child(Self::tuck(
                self.artwork(window, cx),
                Self::cover_room(cx),
                window,
                cx,
            ))
            .when(room, |this| this.child(self.details(cx)))
    }

    /// The title and artist block beside the cover.
    fn details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.playback.read(cx).shown_track(cx);
        track_details(shown.as_ref(), &self.playback, false, Self::menu_at, cx)
    }

    fn menu_at(&mut self, position: Point<Pixels>, _: &mut Window, cx: &mut Context<Self>) {
        self.open_playing_menu(position, cx);
    }

    /// The playing cover keeps its album link and context menu in every layout.
    fn artwork(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let artwork = Self::cover_side(window, cx);
        let away = cx.try_global::<CoverAway>().is_some_and(|away| away.0);
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let cover = track.and_then(|track| track.cover.clone());
        let album = track.map(|track| match track.album.is_empty() {
            true => t!("song-view-album"),
            false => SharedString::from(track.album.clone()),
        });
        let stamp = members::picked_by(cx).filter(|_| !away).map(|added| {
            let ring = match self.settings.read(cx).player_bar_translucent() {
                true => theme.popover,
                false => theme.secondary,
            };
            jam::stamp("now-playing-adder", &added, artwork, ring, window)
        });
        div()
            .id("now-playing-artwork")
            .relative()
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
            .children(stamp)
            .flex_none()
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

    /// How many icon buttons stand before the volume: the sessions picker while it shows and
    /// the panel buttons while the window has room for the right panel.
    fn tool_count(window: &Window, cx: &App) -> usize {
        usize::from(Self::picker_shown(cx)) + Self::panel_buttons(window, cx)
    }

    /// Whether the bar shows its Sessions picker.
    fn picker_shown(cx: &App) -> bool {
        Sonora::global(cx).settings.read(cx).sessions_button()
            && (Sonora::global(cx).devices.read(cx).available()
                || Sonora::global(cx).listening.read(cx).available(cx))
    }

    /// How many panel buttons the bar shows, none while the window has no room for the panel.
    fn panel_buttons(window: &Window, cx: &App) -> usize {
        match SidebarRight::available(window) {
            true => 1 + usize::from(Sonora::global(cx).settings.read(cx).lyrics_enabled()),
            false => 0,
        }
    }

    /// The tools for a view below `Room::Wide` that copies the bar's layout: `sessions` where the
    /// bar shows its picker. The bar has no panel buttons there, and its tools take their place
    /// in the row even when they draw nothing, so the copy's do too and its seek bar ends where
    /// the bar's does.
    pub(crate) fn tools_like(sessions: Entity<SessionsButton>, cx: &App) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .when(Self::picker_shown(cx), |this| this.child(sessions))
    }

    /// How the bar draws its volume control at this width, in whatever layout it has.
    pub(crate) fn volume(window: &Window, cx: &App) -> Volume {
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        match (Self::stacked(window, layout), layout) {
            // A narrow bar keeps its seek bar long and folds the volume into the mute button.
            (true, _) => Volume::Rising,
            (false, PlayerBarLayout::Classic) => {
                Self::classic(window, Self::tool_count(window, cx), cx).1
            }
            (false, _) => Volume::Slider(px(VOLUME_WIDTH)),
        }
    }

    /// How big the playing cover draws at this width: a thumbnail in an unstacked compact bar
    /// and a row's height everywhere else.
    pub(crate) fn cover_side(window: &Window, cx: &App) -> Pixels {
        let theme = cx.theme();
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        let side = match layout == PlayerBarLayout::Compact && !Self::stacked(window, layout) {
            true => theme.metrics.thumb,
            false => theme.metrics.row,
        };
        ui::snapped(side, window)
    }

    /// How far above the bar's middle a stacked bar's cover and title row sits, half of the seek
    /// row and the gap over it. The narrow fullscreen player lowers that row by as much while
    /// its controls sleep, so it rests centred in the bar. An unstacked bar is already centred.
    pub(crate) fn title_lift(window: &Window, cx: &App) -> Pixels {
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        match Self::stacked(window, layout) {
            true => {
                let lift = (window.rem_size() * STACK_GAP + cx.theme().metrics.control_small) / 2.;
                ui::snapped(lift, window)
            }
            false => Pixels::ZERO,
        }
    }

    /// Whether the title and artist show beside the cover, which needs a window from
    /// `Room::Snug` up.
    pub(crate) fn shows_track(window: &Window) -> bool {
        Chrome::window_room(window).fits(Room::Snug)
    }

    /// Gives `base` the bar's ground: the frosted surface or the secondary colour the settings
    /// pick, the border along its top and the window's bottom corners. The narrow fullscreen
    /// player stands its own bar on the same ground, so the two draw alike to the pixel.
    pub(crate) fn surface(base: Stateful<Div>, window: &Window, cx: &App) -> Stateful<Div> {
        let theme = *cx.theme();
        let transparent = Sonora::global(cx)
            .settings
            .read(cx)
            .player_bar_translucent();
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = {
            let _ = window;
            None
        };
        base.when_some(radius, |this, radius| this.rounded_b(radius))
            .when_else(
                transparent,
                |this| ui::bar_surface(this, cx).text_color(theme.popover_foreground),
                |this| this.when(!theme.transparent, |this| this.bg(theme.secondary)),
            )
            .border_t_1()
            .border_color(theme.border)
    }

    /// How much room the cover takes beside the title right now, which [`CoverRoom`] sets.
    pub(crate) fn cover_room(cx: &App) -> f32 {
        cx.try_global::<CoverRoom>()
            .map_or(1., |room| room.0.clamp(0., 1.))
    }

    /// Tucks the cover that starts a now playing row away by `room`. It keeps its size and spot
    /// and fades, while what follows it slides over its place, so a cover a flight targets never
    /// changes its bounds. With no room left it answers no pointer either.
    pub(crate) fn tuck(
        cover: Stateful<Div>,
        room: f32,
        window: &Window,
        cx: &App,
    ) -> Stateful<Div> {
        if room >= 1. {
            return cover;
        }
        let taken = Self::cover_side(window, cx) + window.rem_size() * COVER_GAP;
        cover
            .mr(Pixels::ZERO - taken * (1. - room))
            .opacity(room)
            .when(room <= 0., |this| this.invisible())
    }

    /// Gives `base` the bar's height and padding for the layout it has at this width.
    fn frame(base: Stateful<Div>, window: &Window, cx: &App) -> Stateful<Div> {
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        base.flex()
            .w_full()
            .h(Self::height(window, cx))
            .flex_none()
            .px_5()
            .when(Self::stacked(window, layout), |this| this.py_2())
    }

    /// Lays `parts` out in `base` the way the bar is laid out at this width, for every layout
    /// but an unstacked centred one, which the bar draws itself. `base` is sized and padded here.
    pub(crate) fn arrange(
        base: Stateful<Div>,
        parts: BarParts,
        window: &Window,
        cx: &App,
    ) -> Stateful<Div> {
        let theme = *cx.theme();
        let layout = Sonora::global(cx).settings.read(cx).player_bar_layout();
        let base = Self::frame(base, window, cx);
        let BarParts {
            now_playing,
            transport,
            seek,
            tools,
            sound,
            corner,
        } = parts;
        let now_playing = div().flex().flex_1().min_w_0().child(now_playing);
        match (Self::stacked(window, layout), layout) {
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
                        .child(now_playing)
                        .child(transport),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .w_full()
                        .child(div().flex_1().min_w_0().child(seek))
                        .children(tools)
                        .child(sound)
                        .child(corner),
                ),
            (false, PlayerBarLayout::Compact) => base
                .items_center()
                .gap_3()
                .child(now_playing.max_w(theme.metrics.cover * 1.6))
                .child(transport)
                .child(div().flex_1().min_w_0().child(seek))
                .children(tools)
                .child(sound)
                .child(corner),
            // The two sides share what the middle leaves them evenly, so the transport and the
            // seek bar stay centred. `classic` works out how wide the middle is.
            _ => {
                let (middle, _) = Self::classic(window, Self::tool_count(window, cx), cx);
                base.items_center()
                    .gap_4()
                    .child(now_playing)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .flex_none()
                            .w(middle)
                            .child(transport)
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
                            .children(tools)
                            .child(sound)
                            .child(corner),
                    )
            }
        }
    }
}

impl Render for PlayerBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let layout = self.settings.read(cx).player_bar_layout();
        let stacked = Self::stacked(window, layout);
        // Equal wings keep the title centered while reserving the original volume slider width.
        let centered_wing = theme.metrics.control * 4. + px(VOLUME_WIDTH) + theme.metrics.pad * 3.;
        let sides = match SidebarRight::available(window) {
            true => Some(self.side_buttons(cx)),
            false => None,
        };
        // the Sessions button stands with the panel buttons, one gap apart like them, and takes no
        // gap of its own while it draws nothing or the listener hid it
        let picker = Self::picker_shown(cx);
        let buttons = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .when(picker, |this| this.child(self.sessions.clone()))
            .children(sides);

        let seek = self.timeline.clone();

        let base = Self::surface(
            div()
                .id("player-bar")
                .role(Role::Toolbar)
                .aria_label(t!("settings-page-player-bar")),
            window,
            cx,
        )
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
            (false, PlayerBarLayout::Centered) => Self::frame(base, window, cx)
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
                        .child({
                            let shown = self.playback.read(cx).shown_track(cx);
                            track_details(shown.as_ref(), &self.playback, true, Self::menu_at, cx)
                                .w_full()
                        })
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
            _ => {
                let parts = BarParts {
                    now_playing: self
                        .now_playing(Self::shows_track(window), window, cx)
                        .into_any_element(),
                    // The wrapper matches the one the fullscreen player hears the pointer through, so
                    // the two round to the same pixels.
                    transport: div()
                        .flex()
                        .items_center()
                        .child(transport(&self.playback, false, cx))
                        .into_any_element(),
                    seek: seek.into_any_element(),
                    tools: Some(buttons.into_any_element()),
                    sound: self.sound(Self::volume(window, cx), cx),
                    corner: self.fullscreen_button(cx),
                };
                Self::arrange(base, parts, window, cx)
            }
        };

        content
            .relative()
            .when_some(context_menu, |this, menu| this.child(menu))
    }
}
