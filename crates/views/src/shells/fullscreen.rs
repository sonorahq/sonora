use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, App, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, ScrollWheelEvent, SharedString, SpringConfig, SpringState, Stateful,
    Task, TextShadow,
};
use gpui::{Div, Window, anchored, canvas, deferred, div, phi, point, px, rems};
use i18n::t;
use input::{
    ToggleFullscreen, ToggleLyrics, ToggleQueue, ToggleWindowFullscreen, WORKSPACE_CONTEXT,
};
use router::{Destination, navigate};
use state::{AppSettings, Cover, FullscreenControlsAutohide, Playback, Queue, SideTab, Sonora};
use ui::{
    ActiveTheme as _, Artwork, Badge, Button, Flight, InlineLink, InlineLinks, Motion,
    Motioned as _, Popup, Room, Scrollbar, Scrubber, ScrubberState, Springs, Sprung, TabBar, Text,
    Visualizer, Zoom, clock, entering, snapped,
};

use crate::chrome::{
    Aside, BarParts, COVER_GAP, PlayerBar, SessionsButton, TitleBarOptions, Volume, jam, members,
};
use crate::shared::menus::ItemMenu;
use crate::shared::now_playing::track_details;
use crate::shared::rooted::Rooted;
use crate::shared::transport::{
    COVER_FLIGHT, CoverFade, CoverRoom, ShellSwap, clock_ems, end_clock, fraction, like, moved,
    transport,
};
use crate::shared::visualizer::VisualizerDrive;
use crate::shared::volume::{
    RisingVolume, VOLUME_BUBBLE_LIFT, VOLUME_RISE, VolumeHost, rising_volume, volume_slider,
};
use crate::shared::{self, ambient};
use crate::shells::Shell;
use ui::{VeilEdge, veil};

/// How many body text heights across the face of whoever queued the playing song stands.
const ADDER_FACE: f32 = 1.5;
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
/// The page's own `gap_5`, bottom padding and the meta block's `gap_1`, in rems, so the cover can be
/// fitted against the room actually left over. GPUI's spacing scale is a quarter rem a step.
const COLUMN_GAP: f32 = 1.25;
const PAGE_PAD: f32 = 1.5;
const META_GAP: f32 = 0.25;
/// The page's side padding and the gap between the cover column and a panel from `Room::Wide`
/// up, in rems, which set how wide the split column is.
const PAGE_SIDE: f32 = 2.;
const ROW_GAP: f32 = 2.;
const DOCK: f32 = 1.15;
const DOCK_FULL: f32 = 1.7;
const SINK: f32 = 24.;
/// The name the seek bar with the transport is seen under, so it slides to where a panel
/// moves it.
const DECK_FLIGHT: &str = "fullscreen-deck";
/// The name the bar's cover is seen under below `Room::Wide`, so a panel coming or going flies
/// the big cover into it and back out of it.
const STRIP_FLIGHT: &str = "fullscreen-strip-cover";
/// The space between the pill and the leave button in the corner, and between the transport and
/// the volume where the volume follows it.
const CORNER_GAP: f32 = 8.;
/// A closed panel fading out, a little quicker than one comes in.
const PANEL_LEAVE: SpringConfig = ui::spring(0.24, 0.);
/// How far the page's foot lifts the row along the bottom, which puts it level with the
/// corner.
const DECK_LIFT: f32 = 2.;
const PILL_GAP: f32 = 2.;
/// How far above the bar along the bottom the pill floats below `Room::Wide`.
const PILL_LIFT: f32 = 8.;
const SEEK_MAX: f32 = 420.;
const VISUALIZER_MIN: f32 = 160.;
/// How tall the band under the controls is, as a share of the window. It carries the seek bar,
/// the transport and the meta line over a visualizer that reaches the bottom edge, so it runs
/// a good deal taller than the chrome itself and fades out well above it.
const VEIL: f32 = 0.34;
/// How hard that band blurs what passes under it. The settings header has flat page under it
/// and gets by on a pixel; a bar of the visualizer is a hard edge and needs a real radius
/// before it stops reading through the text over it.
const VEIL_BLUR: Pixels = px(12.);
/// How far ahead of the spectrum that band runs as the two come and go: it is at full strength
/// by the time this share of the spectrum shows. A band that only kept pace would let the bars
/// read sharp through it for as long as either was partly there.
const VEIL_LEAD: f32 = 10.;
/// The blur of each shadow stacked under the title and the artist line. A tight one gives the
/// shadow a core and a wider one softens its edge.
const TEXT_SHADOW: [f32; 2] = [2., 8.];
/// How much of the page colour the title and artist shadows carry. The text reads on the field
/// without them, so they only hold it where something bright passes behind.
const TEXT_SHADOW_STRENGTH: f32 = 0.45;
/// How far the widest text shadow spreads past its glyphs: three of its deviations, as far as the
/// renderer carries a blur. A gliding layer keeps this much room so it never cuts a shadow off.
const SHADOW_BLEED: f32 = TEXT_SHADOW[1] * 3.;
/// How hard the docked controls blur once they are all but hidden. The dock shrinks over them as
/// they leave, so without it the last frames show the tops of the seek row cut off by its edge.
const DOCK_BLUR: Pixels = px(8.);
/// Below this the dock's blur is not worth a layer of its own.
const DOCK_BLUR_LEAST: Pixels = px(0.05);
const REST: Duration = Duration::from_millis(1000);
const WAKE_DEBOUNCE: Duration = Duration::from_millis(400);
/// How far the pointer has to travel, in pixels, before it wakes the controls, so the drift a
/// resting hand gives a trackpad or a sensitive mouse leaves them asleep.
const WAKE_TRAVEL: f64 = 3.;
const SPRING_REST: f32 = 0.001;
const SPRING_STALL: Duration = Duration::from_millis(64);

pub struct FullscreenView {
    playback: Entity<Playback>,
    cover: Entity<Cover>,
    settings: Entity<AppSettings>,
    aside: Entity<Aside>,
    panel: Option<SideTab>,
    /// How much of the content the pill just switched to is still hidden, springing from 1 to 0
    /// at the pace the cover travels, so a panel fades in while the cover slides aside. A spring
    /// steps from the first frame the content draws, so a slow first layout cannot eat the
    /// entrance. The pill itself never takes part.
    switch: Option<Sprung>,
    seek: ScrubberState,
    pending: Option<f32>,
    over_seek: Option<f32>,
    volume: RisingVolume,
    over_pill: bool,
    over_transport: bool,
    over_leave: bool,
    /// Whether the pointer rests on the bar along the bottom below `Room::Wide` or the pill over
    /// it, which keeps the controls awake.
    over_dock: bool,
    large: Option<SharedString>,
    full: Option<SharedString>,
    revision: usize,
    track_menu: ItemMenu,
    context_menu: Option<(music::Track, Point<Pixels>)>,
    last_moved: Instant,
    /// Where the pointer was when it last woke the controls, which motion is measured from.
    last_pos: Option<Point<Pixels>>,
    inside: bool,
    /// Whether the pointer left the window and has not moved inside it since. macOS answers
    /// `is_window_hovered` with whether the window has focus, so only the exit event tells there.
    left: bool,
    awake: bool,
    hidden: SpringState,
    spring_beat: Instant,
    rest: Option<Task<()>>,
    focus: FocusHandle,
    visualizer: VisualizerDrive,
    root_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// How far the cover is scaled as it flies in from the player bar.
    cover_zoom: Rc<Cell<Zoom>>,
    /// Whether the player is the shell on show. While it fades out over the workspace its cover
    /// flies back into the player bar.
    on_show: bool,
    /// The player's own top left corner, which a fading panel is placed against.
    root_origin: Rc<Cell<Point<Pixels>>>,
    /// Where the panel last drew, which is where it fades out once the pill closes it.
    panel_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// How much of a panel the pill just closed is still showing, springing from 1 to 0.
    leaving: Option<Sprung>,
    meta_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where the big cover last drew, which a panel opening below `Room::Wide` flies it down from.
    cover_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Whether the switch under way opened a panel over the cover, which below `Room::Wide` flies
    /// the big cover down onto the bar's.
    landing: bool,
    /// The height the controls take with nothing clamping them, measured a frame ago. The dock
    /// shrinks from it while it fades, so the column moves in step with the cover.
    dock_natural: Rc<Cell<Pixels>>,
    /// The volume slider beside the mute button below `Room::Wide`, where the controls lie the
    /// way the player bar's do, and where the pointer hovers it.
    volume_bar: ScrubberState,
    /// The Sessions picker below `Room::Wide`, where the player bar has its own.
    sessions: Entity<SessionsButton>,
    over_volume: Option<f32>,
}

impl VolumeHost for FullscreenView {
    fn volume(&mut self) -> &mut RisingVolume {
        &mut self.volume
    }

    fn playback(&self) -> &Entity<Playback> {
        &self.playback
    }
}

impl FullscreenView {
    /// The height the dock along the bottom is given, which the toasts hang above.
    pub(crate) fn dock_height(cx: &App) -> Pixels {
        cx.theme().metrics.player_bar * DOCK_FULL
    }

    pub fn new(playback: Entity<Playback>, queue: Entity<Queue>, cx: &mut Context<Self>) -> Self {
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let sponsorblock = Sonora::global(cx).sponsorblock.clone();
        cx.observe(&sponsorblock, |_, _, cx| cx.notify()).detach();
        // the seek bar and the lyrics follow the position as it moves
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |_, _, cx| cx.notify()).detach();
        let cover = Sonora::global(cx).cover.clone();
        cx.observe(&cover, |_, _, cx| cx.notify()).detach();
        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        // The meta drops its text shadows while the player opens or closes.
        cx.observe_global::<ShellSwap>(|_, cx| cx.notify()).detach();
        cx.observe_global::<CoverRoom>(|_, cx| cx.notify()).detach();
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |this, settings, cx| {
            if this.panel == Some(SideTab::Lyrics) && !settings.read(cx).lyrics_enabled() {
                this.panel = None;
            }
            cx.notify();
        })
        .detach();
        let panel = settings.read(cx).fullscreen_tab();
        let shown = panel.unwrap_or_else(|| match settings.read(cx).lyrics_enabled() {
            true => SideTab::Lyrics,
            false => SideTab::Queue,
        });
        let aside = cx.new(|cx| Aside::new(queue.clone(), playback.clone(), shown, cx));
        aside.update(cx, |aside, _| aside.strip());
        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));

        Self {
            playback,
            cover,
            settings,
            aside,
            panel,
            switch: None,
            seek: ScrubberState::new("fullscreen-seek"),
            pending: None,
            over_seek: None,
            volume: RisingVolume::new("fullscreen-volume-slider"),
            over_pill: false,
            over_transport: false,
            over_leave: false,
            over_dock: false,
            large: None,
            full: None,
            revision: 0,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            context_menu: None,
            last_moved: Instant::now(),
            last_pos: None,
            inside: true,
            left: false,
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
            cover_zoom: Rc::default(),
            on_show: false,
            root_origin: Rc::default(),
            panel_bounds: Rc::new(Cell::new(Bounds::default())),
            leaving: None,
            meta_bounds: Rc::new(Cell::new(Bounds::default())),
            cover_bounds: Rc::default(),
            landing: false,
            dock_natural: Rc::new(Cell::new(Pixels::ZERO)),
            volume_bar: ScrubberState::new("fullscreen-volume"),
            sessions: cx.new(SessionsButton::new),
            over_volume: None,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Starts the idle clock as the view opens and stops it as the view closes, so it never
    /// hides the cursor over the workspace. Either way the view next shows with its controls up.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.on_show = shown;
        match shown {
            true => {
                self.flip(true);
                self.hidden = SpringState {
                    position: 0.,
                    velocity: 0.,
                };
                self.unpark();
                self.last_pos = None;
                self.stir(cx);
            }
            false => self.rest = None,
        }
        cx.notify();
    }

    /// Selects the fullscreen panel and restarts the controls' idle timer.
    fn show(&mut self, panel: Option<SideTab>, cx: &mut Context<Self>) {
        if panel != self.panel && !cx.reduce_motion() {
            let visible = self
                .switch
                .as_ref()
                .map_or(1., |switch| 1. - switch.position().clamp(0., 1.));
            self.switch = Some(Sprung::at(1.));
            self.landing = self.panel.is_none() && panel.is_some();
            self.leaving = panel.is_none().then(|| Sprung::at(visible));
        }
        self.panel = panel;
        // The transport may move between the dock and the strip, and the row it leaves never
        // says the pointer left it.
        self.over_transport = false;
        self.settings
            .update(cx, |settings, cx| settings.set_fullscreen_tab(panel, cx));
        if let Some(tab) = panel {
            self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        }
    }

    /// Flips the fullscreen panel to `tab` from a key press. The cover comes back when `tab`
    /// is already showing.
    fn toggle_panel(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if tab == SideTab::Lyrics && !self.settings.read(cx).lyrics_enabled() {
            return;
        }
        let panel = match self.panel == Some(tab) {
            true => None,
            false => Some(tab),
        };
        self.show(panel, cx);
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

    fn hover(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !hovered(window) {
            return;
        }
        let pad = cx.theme().metrics.pad;
        let seek = self.seek.hovered(event.position, pad);
        if moved(self.over_seek, seek) {
            self.over_seek = seek;
            cx.notify();
        }
        let volume = self.volume_bar.hovered(event.position, pad);
        if moved(self.over_volume, volume) {
            self.over_volume = volume;
            cx.notify();
        }
        if self.left {
            self.left = false;
            cx.notify();
        }
        let travel = self
            .last_pos
            .map_or(f64::INFINITY, |last| (event.position - last).magnitude());
        if travel < WAKE_TRAVEL {
            if !self.awake {
                cx.hide_cursor();
            }
            return;
        }
        self.last_pos = Some(event.position);
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
    /// would keep the controls up for good. A Jam invite or a panel's track menu drawn over the
    /// view holds it too, since the pointer over either never reaches the view and the cursor
    /// would hide under it.
    fn busy(&self, cx: &App) -> bool {
        let parked = self.volume.parked()
            || self.over_pill
            || self.over_transport
            || self.over_leave
            || self.over_dock
            || self.over_seek.is_some()
            || self.over_volume.is_some()
            || self.scrollbar_held(cx);
        (self.inside && parked)
            || self.volume.held
            || self.pending.is_some()
            || self.context_menu.is_some()
            || (self.panel.is_some() && self.aside.read(cx).menu_open())
            || Sonora::global(cx).devices.read(cx).invite().is_some()
    }

    /// Clears hover state when the fullscreen view opens or closes. Its elements report their
    /// current hover state when they are mounted again.
    fn unpark(&mut self) {
        self.volume.unpark();
        self.over_pill = false;
        self.over_transport = false;
        self.over_leave = false;
        self.over_dock = false;
        self.over_seek = None;
        self.over_volume = None;
    }

    /// Whether the player bar's cover flies to or from this player as it opens or closes, for the
    /// bar to leave its own undrawn meanwhile. Below `Room::Wide` a panel leaves the cover in the
    /// bar along the bottom where the player bar has it, so there the bar's only flies to and
    /// from the artwork tab.
    pub(crate) fn cover_flies(&self, window: &Window) -> bool {
        Room::of(window.viewport_size().width).fits(Room::Wide) || self.panel.is_none()
    }

    /// Whether the bar along the bottom gives its cover's place to the title, which it does
    /// below `Room::Wide` on the artwork tab, where the big cover shows the same art.
    pub(crate) fn tucks_cover(&self, window: &Window) -> bool {
        !Room::of(window.viewport_size().width).fits(Room::Wide) && self.panel.is_none()
    }

    /// How much of its place beside the title the bar's cover keeps, given how much of the tab
    /// just shown is still `hidden`. It slides over as a panel comes or goes, follows the player
    /// bar's under it while the player opens or closes, and otherwise rests where the tab puts it.
    /// The player moving between the shells wins over a switch still running, so the two bars
    /// never part.
    fn cover_room(&self, hidden: f32, cx: &App) -> f32 {
        if cx.try_global::<ShellSwap>().copied().unwrap_or_default() != ShellSwap::Settled {
            return PlayerBar::cover_room(cx);
        }
        match self.panel {
            Some(_) if self.landing => 1. - hidden,
            Some(_) => 1.,
            None if self.switch.is_some() => hidden,
            None => 0.,
        }
    }

    /// Follows the pointer in and out of the window, once a frame. A hover flag clears on an
    /// event the pointer has to be present for, so the ones still set when it left the window are
    /// dropped here instead. Crossing the window edge repaints on its own, so this is never late.
    fn watch(&mut self, window: &Window, cx: &mut Context<Self>) {
        let inside = hovered(window) && !self.left;
        if inside == self.inside {
            return;
        }
        self.inside = inside;
        if !inside {
            self.unpark();
            self.last_pos = None;
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

    /// How much of the tab just shown is still hidden, from 1 as the switch starts to 0 once it
    /// ends. It asks for another frame while the switch runs.
    fn switching(&mut self, window: &mut Window, cx: &App) -> f32 {
        let Some(switch) = self.switch.as_mut() else {
            return 0.;
        };
        let hidden = switch.follow(0., Springs::FLIGHT, SPRING_REST, window, cx);
        if hidden <= 0. {
            self.switch = None;
            self.landing = false;
        }
        hidden.clamp(0., 1.)
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
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The cover leaves the player bar with that cover's tight corners, which round out to
        // its own as it lands, and tighten again on the way back. A panel coming or going only
        // scales it between two sizes of its own, so it keeps its own corners the whole way.
        // Below `Room::Wide` a panel shrinks it into the corner of the bar, so there it comes
        // back from that cover's tight corners too.
        let (tight, round) = (Artwork::rounding(cx), cx.theme().radius * 2.);
        let zoom = self.cover_zoom.get();
        let radius = match (self.on_show, self.switch.is_some()) {
            (true, true) if !narrow => zoom.radius(round, round),
            (true, _) => zoom.radius(tight, round),
            (false, _) => zoom.radius(round, tight),
        };
        let pad = px(COVER_LAYER_PAD);
        let inset = (layout_side - raster_side) / 2. - pad;
        let (album, small, local_id, named) = match self.playback.read(cx).shown_track(cx) {
            Some(track) => {
                let local = track.id.as_deref().is_some_and(music::is_local_id);
                let named = album_name(&track);
                (track.album_id, track.cover, local, named)
            }
            None => (None, None, false, SharedString::default()),
        };
        let cover = self.cover.read(cx);
        let cover_large = album.as_deref().and_then(|album| cover.large_for(album));
        let cover_full = album.as_deref().and_then(|album| cover.full_for(album));
        let local = local_id || small.as_ref().is_some_and(|url| url.starts_with("file://"));
        // Only upgrade resolution for non-local tracks, since a local cover is always
        // the full resolution picture. The track's own embedded cover is prioritized
        // over the cover of its album. Each larger cover fades in over the one below, so the
        // thumbnail shows only until the large cover lands.
        let upgrades = |url: &&str| !local && small.is_some() && Some(*url) != small.as_deref();
        let large = cover_large.filter(upgrades);
        let full = cover_full
            .filter(upgrades)
            .filter(|url| Some(*url) != large);

        if self.large.as_deref() != large || self.full.as_deref() != full {
            self.large = large.map(SharedString::from);
            self.full = full.map(SharedString::from);
            self.revision += 1;
        }
        let large = self.large.clone();
        let full = self.full.clone();
        let revision = self.revision;
        let waiting = !local && album.is_some() && cover_large.is_none();

        // As the player opens, its cover comes into view with the rest of it while it flies. Below
        // `Room::Wide` it is the bar's own cover rising whole out of the bar beside controls that
        // stay put.
        let fade = match narrow {
            true => 1.,
            false => cx.try_global::<CoverFade>().map_or(1., |fade| fade.0),
        };
        let cover = div()
            .id("fullscreen-artwork")
            .when(fade < 1., |this| this.opacity(fade))
            .relative()
            .size(layout_side)
            .flex_none()
            .child({
                let bounds = self.cover_bounds.clone();
                canvas(move |at, _, _| bounds.set(at), |_, _, _, _| {})
                    .absolute()
                    .size_full()
            })
            .when_some(album, |this, album| {
                this.role(Role::Link)
                    .aria_label(named)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| open_album(&album, cx))
            })
            .child(
                div()
                    .absolute()
                    .top(inset)
                    .left(inset)
                    .size(raster_side + pad * 2.)
                    .child(
                        div().absolute().top(pad).left(pad).child(
                            Artwork::new(small)
                                .size(raster_side)
                                .corner_radius(radius)
                                .soft(waiting),
                        ),
                    )
                    .children(
                        [("cover-large", large), ("cover-full", full)]
                            .into_iter()
                            .filter_map(|(layer, url)| {
                                let url = url?;
                                Some(
                                    div()
                                        .absolute()
                                        .top(pad)
                                        .left(pad)
                                        .child(
                                            Artwork::new(Some(url))
                                                .size(raster_side)
                                                .corner_radius(radius)
                                                .overlay(),
                                        )
                                        .motion((layer, revision), Motion::Slow, |art, t| {
                                            art.opacity(t)
                                        }),
                                )
                            }),
                    ),
            );
        // The cover flies in from the player bar as fullscreen opens, and slides and grows to
        // its new place when a panel comes or goes, above everything while it travels.
        // The controls falling asleep move the cover on a spring of their own, so layout only
        // sends it flying while a panel is coming or going. The raster is drawn at its resting
        // size through the flight's own layer, which keeps room for how far it reaches past
        // its layout, so a flying cover takes one layer and not two.
        // Below `Room::Wide` the cover that a closing panel brings back grows out of the one the
        // bar along the bottom held, and a panel opening shrinks it back in there.
        Flight::new("fullscreen-cover", cover)
            .transfer_spring(Springs::ARTWORK_FLIGHT)
            .lands_from(match narrow && self.switch.is_some() {
                true => STRIP_FLIGHT,
                false => COVER_FLIGHT,
            })
            // Once the player closes, its cover flies back down onto the player bar's.
            .heads_to((!self.on_show).then_some(COVER_FLIGHT))
            .reports(self.cover_zoom.clone())
            .follows(self.switch.is_some())
            .reach((raster_side - layout_side) / 2. + pad)
            .presents_at(presentation_scale)
            .above()
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

    fn meta(
        &self,
        hide: f32,
        lift: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let frosted = ambient::shown(cx);
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let title = match track {
            Some(track) => SharedString::from(track.name.clone()),
            None => t!("player-nothing-playing"),
        };
        let album = track.and_then(|track| track.album_id.clone());
        let opens = album.is_some();
        let explicit = track.is_some_and(|track| track.explicit);
        let meta_bounds = self.meta_bounds.clone();
        // Both lines carry a shadow in the page colour, so they stay legible where a visualizer
        // peak or bright ambient art passes behind them. The title ellipsizes without
        // `truncate`, whose `overflow_hidden` would clip the shadow to the line box. Every shadow
        // is a blur layer of its own, so they wait out the player opening or closing, while the
        // meta is still fading and every frame already carries the most layers.
        let shadowed = shared::effects()
            && cx.try_global::<ShellSwap>().copied().unwrap_or_default() == ShellSwap::Settled;
        let shadow = TEXT_SHADOW.map(|blur| {
            TextShadow::new(theme.background.opacity(TEXT_SHADOW_STRENGTH)).blur(px(blur))
        });
        let trailing = |track: Option<&music::Track>, cx: &App| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .when(explicit, |this| {
                    this.child(explicit_badge("fullscreen-explicit", track.is_some()))
                })
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .opacity(1. - hide)
                        .child(like(track, &self.playback, cx).when(frosted, Button::frosted)),
                )
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
            .w_full()
            .min_w_0()
            .map(|this| glide(this, lift, px(SHADOW_BLEED), window))
            .child(
                canvas(move |bounds, _, _| meta_bounds.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(SHADOW_BLEED))
                    .bottom(px(SHADOW_BLEED)),
            )
            .child(
                // Three-part row with equal flex sides, so the title stays truly centred:
                // the heart and the explicit badge live in the right cell and never shift it.
                // The left cell holds an invisible copy of the right one, so both sides floor
                // at the same width and a long title truncates instead of pushing the heart out
                // of the row. The room around the title comes from the row gap, since padding
                // on a side would make that side the wider one.
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .id("fullscreen-title-balance")
                            .flex()
                            .flex_1()
                            .justify_end()
                            .invisible()
                            .child(trailing(None, cx)),
                    )
                    .child(
                        div()
                            .id("fullscreen-title")
                            .map(|this| match opens {
                                true => this.role(Role::Link).aria_label(title.clone()),
                                false => this.role(Role::Label).aria_value(title.clone()),
                            })
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_ellipsis()
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
                                    let Some(track) = this.playback.read(cx).shown_track(cx) else {
                                        return;
                                    };
                                    window.prevent_default();
                                    this.open_context_menu(track, event.position, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .when(shadowed, |this| this.text_shadow(shadow))
                            .child(title),
                    )
                    .child(div().flex().flex_1().child(trailing(track, cx))),
            )
            .when_some(track, |this, track| {
                this.child(
                    div().flex().w_full().min_w_0().justify_center().child(
                        InlineLinks::new(
                            "fullscreen-artists",
                            track.artist_refs.iter().map(|artist| {
                                InlineLink::new(
                                    artist.name.clone(),
                                    artist.id.clone().map(Into::into),
                                )
                            }),
                            track.artists.clone(),
                            shared::quiet(&theme),
                        )
                        .text_size(theme.text(Text::Body))
                        .truncate()
                        .when(shadowed, |this| this.text_shadow(shadow))
                        .on_click(|id, cx| navigate(Destination::Artist(id), cx)),
                    ),
                )
            })
            .children(self.adder(cx))
    }

    /// The button that names whoever queued the playing song in a Jam and opens their profile.
    fn adder(&self, cx: &mut Context<Self>) -> Option<Button> {
        let theme = *cx.theme();
        let added = members::picked_by(cx)?;
        let id = added.id.clone();
        Some(
            Button::new("fullscreen-adder")
                .ghost()
                .small()
                .mt_1()
                .leading(jam::adder_face(&added, theme.text(Text::Body) * ADDER_FACE))
                .label(t!("queue-added-by", name = added.name.as_str()))
                .tint(shared::quiet(&theme))
                .when(ambient::shown(cx), Button::frosted)
                .on_click(move |_, _, cx| navigate(Destination::User(id.clone().into()), cx)),
        )
    }

    /// Opens the track menu on the playing song at `position`, from a right click on its title.
    fn menu_at(&mut self, position: Point<Pixels>, _: &mut Window, cx: &mut Context<Self>) {
        let Some(track) = self.playback.read(cx).shown_track(cx) else {
            return;
        };
        self.open_context_menu(track, position, cx);
        cx.stop_propagation();
    }

    /// The bar's cover below `Room::Wide`, `side` wide like the player bar's. It stays `hidden`
    /// wherever the big cover shows or flies into it or out of it, since growing this small raster
    /// would show its pixels. It only marks where it stands and never flies itself,
    /// so it moves with the title when the bar lowers them.
    fn strip_cover(&self, track: Option<&music::Track>, side: Pixels, hidden: bool) -> Flight {
        Flight::new(
            "fullscreen-strip-cover",
            div()
                .when(hidden, |this| this.invisible())
                .child(Artwork::new(track.and_then(|track| track.cover.clone())).size(side)),
        )
        .seen_as(STRIP_FLIGHT)
        .follows(false)
    }

    /// The big cover flying down onto the bar's while a panel opens below `Room::Wide`, from
    /// where it stood last frame. The column that held it is gone by then, so this copy stands
    /// in for it, and it draws only while the switch runs.
    fn landing(&self, cx: &App) -> Option<impl IntoElement> {
        let at = self.cover_bounds.get();
        if at.size.width <= Pixels::ZERO {
            return None;
        }
        let origin = at.origin - self.root_origin.get();
        let url = self.large.clone().or_else(|| {
            let track = self.playback.read(cx).shown_track(cx)?;
            track.cover.map(SharedString::from)
        });
        let zoom = self.cover_zoom.get();
        Some(
            div().absolute().left(origin.x).top(origin.y).child(
                Flight::new(
                    "fullscreen-cover-landing",
                    Artwork::new(url)
                        .size(at.size.width)
                        .corner_radius(zoom.radius(cx.theme().radius * 2., Artwork::rounding(cx))),
                )
                .transfer_spring(Springs::ARTWORK_FLIGHT)
                .heads_to(Some(STRIP_FLIGHT))
                .reports(self.cover_zoom.clone())
                .above(),
            ),
        )
    }

    /// The controls below `Room::Wide`, laid out by the player bar's own arrangement for this
    /// width, so the cover, the title, the transport, the seek bar, the volume and the leave
    /// button stand exactly where the bar's do and stay put as the player opens or closes, on
    /// every tab. The controls sink and fade with `hide` and the leave button with `idle`, while
    /// the cover and the title sink into the middle of the bar they leave. The cover keeps `room`
    /// of its place beside the title, which the artwork tab gives over to the title.
    fn bar(
        &self,
        hide: f32,
        idle: f32,
        room: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = *cx.theme();
        // Asleep, the controls keep their room but leave the pointer to nothing. The styles go on
        // the parts themselves, since a wrapper of its own would round them apart from the bar's.
        let shown = self.playback.read(cx).shown_track(cx);
        let track = shown.as_ref();
        let label = match track {
            Some(track) => SharedString::from(format!("{} — {}", track.name, track.artists)),
            None => t!("player-nothing-playing"),
        };
        let side = PlayerBar::cover_side(window, cx);
        let album = track.and_then(|track| track.album_id.clone());
        // A tucked cover leaves no link or Jam stamp behind for a screen reader to find.
        let album = album.filter(|_| room > 0.);
        let named = track.map(album_name).unwrap_or_default();
        let cover =
            div()
                .id("strip-artwork")
                .flex_none()
                .relative()
                .when_some(album, |this, album| {
                    this.role(Role::Link)
                        .aria_label(named)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| open_album(&album, cx))
                })
                .child(self.strip_cover(track, side, self.landing || self.panel.is_none()))
                .children(members::picked_by(cx).filter(|_| room > 0.).map(|added| {
                    jam::stamp("strip-adder", &added, side, theme.background, window)
                }));
        let cover = PlayerBar::tuck(cover, room, window, cx);
        let details = PlayerBar::shows_track(window)
            .then(|| track_details(track, &self.playback, false, Self::menu_at, cx));
        let sound = match PlayerBar::volume(window, cx) {
            Volume::Slider(width) => dozing(
                volume_slider(
                    &self.volume_bar,
                    self.over_volume,
                    self.volume.held,
                    width,
                    &self.playback,
                    cx,
                ),
                hide,
            )
            .into_any_element(),
            Volume::Rising => dozing(rising_volume(&self.volume, &self.playback, false, cx), hide)
                .into_any_element(),
        };
        let parts = BarParts {
            now_playing: div()
                .id("fullscreen-now-playing")
                .role(Role::Group)
                .aria_label(label)
                .relative()
                .top(snapped(PlayerBar::title_lift(window, cx) * hide, window))
                .flex()
                .items_center()
                .gap(rems(COVER_GAP))
                .flex_1()
                .min_w_0()
                .child(cover)
                .children(details)
                .into_any_element(),
            transport: dozing(self.transport_row("fullscreen-transport", false, cx), hide)
                .into_any_element(),
            seek: dozing(self.seek(cx), hide).into_any_element(),
            tools: Some(
                dozing(PlayerBar::tools_like(self.sessions.clone(), cx), hide).into_any_element(),
            ),
            sound: sound.into_any_element(),
            corner: div()
                .flex_none()
                .when_some(drowse(idle), |this, blur| this.blur(blur))
                .child(self.leave(idle, false, true, cx))
                .into_any_element(),
        };
        let base = div()
            .id("fullscreen-bar")
            .occlude()
            // The bar occludes the page, whose own listener would never hear the pointer here.
            .on_mouse_move(cx.listener(Self::hover))
            .on_click(|_, _, cx| cx.stop_propagation());
        // It stands on the player's own ground. The border along the bar's top keeps its width
        // here and draws nothing, so the controls inside stand where the bar has them.
        let base = base.border_t_1().border_color(gpui::transparent_black());
        PlayerBar::arrange(base, parts, window, cx)
    }

    fn transport_row(
        &self,
        id: &'static str,
        big: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.over_transport = *hovering;
                cx.notify();
            }))
            .child(transport(&self.playback, big, cx))
    }

    fn seek(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let empty = muted.opacity(0.3);
        let text = theme.text(Text::Tiny);
        let playback = self.playback.read(cx);
        let seekable = playback.shown_track(cx).is_some();
        let (elapsed, total) = playback.shown_time(cx);
        let progress = self.pending.unwrap_or_else(|| fraction(elapsed, total));
        let time_left = self.settings.read(cx).player_bar_time_left();
        let end = end_clock(elapsed, total, time_left);
        let width = text * clock_ems(total, time_left);

        let bubble = self
            .over_seek
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));

        let label = move |id: &'static str, value: SharedString, align_end: bool| {
            ui::spoken(id, value)
                .w(width)
                .flex_none()
                .whitespace_nowrap()
                .text_size(text)
                .font_features(ui::tabular())
                .text_color(muted)
                .when_else(align_end, |this| this.text_right(), |this| this.text_left())
        };

        div()
            .id("fullscreen-seek")
            .role(Role::Group)
            .aria_label(t!("song-position"))
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(label("fullscreen-elapsed", clock(elapsed), true))
            .child(
                div().flex_1().min_w_0().child(
                    Scrubber::new(&self.seek, progress)
                        .name(t!("song-position"))
                        .value(clock(elapsed))
                        .colors(theme.progress_bar, empty, theme.foreground)
                        .ranges(
                            Sonora::global(cx).sponsorblock.read(cx).ranges(cx),
                            theme.foreground.opacity(0.4),
                        )
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
            .child(
                label("fullscreen-total", end.text, false)
                    .when_some(end.spoken, |this, spoken| this.aria_value(spoken)),
            )
    }

    /// The seek bar over the transport, with the volume at the transport row's right end. They
    /// stand centred under the cover from `Room::Wide` up, at most `deck` wide, so they keep one
    /// width whether they sit in the column or at the bottom.
    fn controls(&self, deck: Pixels, hide: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let stack = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w_full()
            .child(self.seek(cx))
            .child(
                self.transport_row("fullscreen-transport", true, cx)
                    .relative()
                    .w_full()
                    .justify_center()
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
            );

        div()
            .id("fullscreen-controls")
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w_full()
            .max_w(deck)
            .flex_none()
            .on_click(|_, _, cx| cx.stop_propagation())
            // The seek bar and the transport travel as one when a panel moves them between
            // the bottom of the window and the column under the cover.
            .child(travelling(
                "fullscreen-deck",
                DECK_FLIGHT,
                self.switch.is_some(),
                self.on_show,
                // The volume slider rises out of the transport and carries a bubble of its
                // own, which is the farthest anything here paints past the deck.
                px(VOLUME_RISE) + Scrubber::bubble_reach(VOLUME_BUBBLE_LIFT, cx),
                stack.opacity(1. - hide),
            ))
    }

    /// The transport controls under the cover or along the bottom. A dock that `collapses` gives
    /// its height back to the page as it hides. One beside the cover keeps its footprint and
    /// only fades, since the column around it settles by `glide` instead of through layout.
    fn dock(
        &self,
        cap: Pixels,
        hide: f32,
        collapses: bool,
        deck: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The clamp runs from the controls' own height once it is known. From a taller cap the
        // dock would hold still through the start of the fade and catch up after. A dock that
        // keeps its footprint reserves it only once the controls are gone, because the height is
        // a frame old and a switch between the full and split layouts would stretch the column
        // by the difference until something renders again.
        let natural = self.dock_natural.get();
        let cap = match natural > Pixels::ZERO {
            true => natural,
            false => cap,
        };
        let measured = self.dock_natural.clone();
        div()
            .flex()
            .flex_none()
            .w_full()
            .items_start()
            .justify_center()
            .map(|this| match collapses {
                true => this.when(hide > 0., |this| {
                    this.max_h(cap * (1. - hide)).overflow_hidden()
                }),
                false => this.when(hide >= 1., |this| this.min_h(cap)),
            })
            .when_some(drowse(hide), |this, blur| this.blur(blur))
            .when(hide < 1., |this| {
                this.child(
                    div()
                        .relative()
                        .w_full()
                        .flex()
                        .justify_center()
                        .top(px(SINK) * hide)
                        .child(
                            canvas(
                                move |bounds, _, _| measured.set(bounds.size.height),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        )
                        .child(self.controls(deck, hide, cx)),
                )
            })
    }

    fn pill(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let frosted = ambient::shown(cx);
        let lyrics = self.settings.read(cx).lyrics_enabled();
        let gap = px(PILL_GAP);
        let linger = cx.listener(|this: &mut Self, hovering: &bool, window, cx| {
            this.over_pill = *hovering;
            if *hovering && hovered(window) && !this.left {
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
                    .name(t!("fullscreen-panels"))
                    .when(frosted, TabBar::blurred)
                    .rounded(theme.radius + gap)
                    .items(
                        [
                            Some(tab(
                                "fullscreen-artwork-tab",
                                "icons/disc-3.svg",
                                "fullscreen-artwork",
                                None,
                            )),
                            lyrics.then(|| {
                                tab(
                                    "fullscreen-lyrics",
                                    "icons/mic-vocal.svg",
                                    "lyrics-title",
                                    Some(SideTab::Lyrics),
                                )
                            }),
                            Some(tab(
                                "fullscreen-queue",
                                "icons/list-music.svg",
                                "queue-title",
                                Some(SideTab::Queue),
                            )),
                        ]
                        .into_iter()
                        .flatten(),
                    ),
            )
    }

    fn sound(&self, cx: &mut Context<Self>) -> impl IntoElement {
        rising_volume(&self.volume, &self.playback, ambient::shown(cx), cx)
    }

    /// Covers the whole view while the controls sleep and the cursor is hidden, so nothing under
    /// the pointer stays hovered and a click only wakes the controls instead of landing blind.
    /// Scrolling passes through, and the layer takes over the root's own pointer listeners, which
    /// no longer count as hovered beneath it.
    fn dormant(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .block_mouse_except_scroll()
            .on_mouse_move(cx.listener(Self::hover))
            .on_mouse_exit(cx.listener(|this, _: &MouseExitEvent, _, cx| {
                this.left = true;
                cx.notify();
            }))
            .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, _, cx| this.rouse(cx)))
    }

    /// Wakes the sleeping controls from a press and shows the cursor, keeping the press from
    /// whatever sits under the hidden pointer.
    fn rouse(&mut self, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.show_cursor();
        self.stir(cx);
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

    /// The bottom right corner: the pill that picks what shows beside the cover, and the leave
    /// button, in one row that never moves with the layout, so a panel coming or going
    /// leaves the pill under the pointer that switched it. The row is centred on a player bar's
    /// height, lowered by as much as the chrome player bar lowers its own fullscreen button in
    /// the layout it has at this width, so that button and the leave button share one spot. The pill sinks and fades
    /// with the controls. The leave button follows the idle spring instead, so a
    /// pointer move still brings it back when the controls are set to stay hidden. While the
    /// player opens the leave button paints above its fade and zoom, and while it closes the
    /// player bar's button paints there instead. The row occludes, so hovering it never
    /// reaches whatever sits behind it.
    /// The button that leaves the player, where the player bar has its fullscreen button. It
    /// sinks and fades with the idle spring rather than with the controls, so a pointer move
    /// still brings it back when the controls are set to stay hidden. While the player opens it
    /// paints above the fade and zoom, and while it closes the bar's button paints there instead.
    /// A `frosted` one floats over the ambient field, and one in the bar along the bottom is plain
    /// like the bar's own.
    /// One that `holds` keeps its room while asleep, so the controls beside it never move.
    fn leave(
        &self,
        idle: f32,
        frosted: bool,
        holds: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let leave = Button::new("leave-fullscreen")
            .ghost()
            .when(frosted, Button::frosted)
            .small()
            .icon("icons/minimize.svg")
            .tooltip_above("player-fullscreen-leave")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleFullscreen), cx));
        let leave = match cx.try_global::<ShellSwap>().copied().unwrap_or_default() {
            ShellSwap::Opening => deferred(leave).into_any_element(),
            ShellSwap::Closing => leave.invisible().into_any_element(),
            ShellSwap::Settled => leave.into_any_element(),
        };
        div()
            .id("leave-fullscreen-hover")
            .flex_none()
            .relative()
            .top(px(SINK) * idle)
            .opacity(1. - idle)
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.over_leave = *hovering;
                cx.notify();
            }))
            .map(|this| match (idle < 1., holds) {
                (true, _) => this.child(leave),
                (false, true) => this.child(div().invisible().child(leave)),
                (false, false) => this,
            })
    }

    /// The bottom right corner from `Room::Wide` up: the pill that picks what shows beside the
    /// cover, and the leave button, in one row that never moves with the layout, so a panel
    /// coming or going leaves the pill under the pointer that switched it. The row is centred on
    /// a player bar's height, lowered by as much as the chrome player bar lowers its own
    /// fullscreen button in the layout it has at this width, so that button and the leave
    /// button share one spot. The pill sinks and fades with the controls. The row occludes, so
    /// hovering it never reaches whatever sits behind it.
    fn corner(
        &self,
        hide: f32,
        idle: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let drop = PlayerBar::fullscreen_drop(window, cx);

        div()
            .id("fullscreen-corner")
            .occlude()
            .absolute()
            .bottom(-drop)
            .right_5()
            .h(snapped(theme.metrics.player_bar, window))
            .flex()
            .items_center()
            .gap(px(CORNER_GAP))
            .when(hide < 1., |this| {
                this.child(
                    div()
                        .relative()
                        .top(px(SINK) * hide)
                        .opacity(1. - hide)
                        .child(self.pill(cx)),
                )
            })
            .child(self.leave(idle, ambient::shown(cx), false, cx))
    }

    /// The pill below `Room::Wide`, floating centred over whatever lies above the bar along the
    /// bottom. It sinks and fades with the controls.
    fn floating(&self, hide: f32, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(PlayerBar::height(window, cx) + px(PILL_LIFT))
            .flex()
            .justify_center()
            .child(
                div()
                    .relative()
                    .top(px(SINK) * hide)
                    .opacity(1. - hide)
                    // The panel scrolls on under the pill, but a click on it stays with the pill.
                    .block_mouse_except_scroll()
                    .child(self.pill(cx)),
            )
    }

    /// The stretch along the bottom below `Room::Wide` that holds the bar and the `pill` tall
    /// pill over it, where a resting pointer keeps the controls awake. It goes over both and only
    /// listens for hover, so it never takes a click or a scroll from them.
    fn dock_zone(&self, pill: Pixels, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("fullscreen-dock-zone")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(PlayerBar::height(window, cx) + pill + px(PILL_LIFT))
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                this.over_dock = *hovering;
                cx.notify();
            }))
    }
}

fn open_album(album: &str, cx: &mut App) {
    navigate(Destination::Album(album.into()), cx);
}

/// What a cover that opens the track's album is called: the album's name, or View album when
/// the track does not name one.
fn album_name(track: &music::Track) -> SharedString {
    match track.album.is_empty() {
        true => t!("song-view-album"),
        false => SharedString::from(track.album.clone()),
    }
}

/// The explicit mark beside a title. `named` puts it in the accessibility tree, so the invisible
/// copy that only balances the title row can stay out of it.
fn explicit_badge(id: &'static str, named: bool) -> impl IntoElement {
    let badge = div().flex_none().child(Badge::explicit());
    match named {
        true => badge
            .id(id)
            .role(Role::Image)
            .aria_label(t!("song-explicit"))
            .into_any_element(),
        false => badge.into_any_element(),
    }
}

impl Shell for FullscreenView {
    fn title_bar(&self, _content: Option<AnyView>, cx: &App) -> TitleBarOptions {
        TitleBarOptions {
            navigation: false,
            sidebar_open: false,
            sidebar_right: None,
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
        let narrow = !room.fits(Room::Wide);
        let split = !narrow && self.panel.is_some();
        self.watch(window, cx);
        // The zone along the bottom is gone from `Room::Wide` up and never says the pointer left.
        if !narrow {
            self.over_dock = false;
        }
        let idle = self.hidden(window, cx);
        let switching = self.switching(window, cx);
        let leaving = self.leaving.as_mut().map(|leaving| {
            leaving
                .follow(0., PANEL_LEAVE, SPRING_REST, window, cx)
                .clamp(0., 1.)
        });
        if leaving.is_none_or(|left| left <= 0.) {
            self.leaving = None;
        }
        let leaving = leaving.filter(|left| *left > 0.);
        let hide = match self.settings.read(cx).fullscreen_controls_autohide() {
            FullscreenControlsAutohide::Automatic => idle,
            FullscreenControlsAutohide::AlwaysShown => 0.,
            FullscreenControlsAutohide::AlwaysHidden => 1.,
        };
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
        // Below `Room::Wide` the title lives in the bar along the bottom instead.
        let meta = match narrow {
            true => Pixels::ZERO,
            false => line(Text::Title) + rem * META_GAP + line(Text::Body),
        };
        // The controls fill the split column, so beside no panel they stop at the column's width
        // too and a panel coming or going never stretches them while they fly.
        let column = (viewport.width - rem * (PAGE_SIDE * 2. + ROW_GAP)) / 2.;
        let deck = px(SEEK_MAX).min(column).max(Pixels::ZERO);
        // The page ends as much nearer the window's bottom edge as the corner sits lower, so
        // the transport along the bottom stays level with the corner. Below `Room::Wide` it ends
        // where the bar along the bottom starts, and over the pill floating on it while the
        // cover is on show.
        let pill = theme.metrics.control_small + px(PILL_GAP * 2. + 2.);
        let foot = match (narrow, self.panel.is_some()) {
            (false, _) => rem * PAGE_PAD + px(DECK_LIFT) - PlayerBar::fullscreen_drop(window, cx),
            (true, true) => PlayerBar::height(window, cx),
            (true, false) => PlayerBar::height(window, cx) + pill + px(PILL_LIFT * 2.),
        };
        let stack = theme.metrics.title_bar + rem * (COLUMN_GAP * 2.) + foot + meta;
        // The dock reserves the cap it clamps itself to while fading, and nothing at all once
        // it is hidden, which is the whole difference between the awake and the resting fit.
        let dock = match (narrow, split) {
            (true, _) => Pixels::ZERO,
            (false, true) => theme.metrics.player_bar * DOCK,
            (false, false) => theme.metrics.player_bar * DOCK_FULL,
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
        // Below `Room::Wide` the bar keeps its footprint while the controls sleep, so the cover
        // has nothing to grow into.
        let far = match narrow {
            true => near,
            false => fit(tall_rest, wide_rest, stack, ceiling_rest)
                .min(near + above * 2.)
                .max(near),
        };
        let presented_side = near + (far - near) * hide;
        // The flex item must never change size when the idle state flips: even a one-frame
        // near/far swap makes the centred column relayout. Keep its awake footprint forever and
        // animate only the fixed large raster surface in the compositor.
        let side = snapped(near, window);
        let raster_side = snapped(far, window);
        let cover_scale = presentation_scale(presented_side, raster_side);
        let staged = self.panel.is_none() || split;
        // The column settles by half of what the dock gives up and the meta by half of what the
        // cover grows, both straight along `hide` to a rest that sits on the device grid, so
        // neither steps a whole pixel at a time through layout in the slow tail of the spring.
        let settle = |rest: Pixels| snapped(rest, window) * hide;
        let shift = match staged && !narrow {
            true => settle(self.dock_natural.get() / 2.),
            false => Pixels::ZERO,
        };
        let lift = settle((far - side) / 2.);
        // The cover grows past the column's top and the meta sinks past its bottom while the
        // column glides, so its layer keeps room for both. A split column already fills the
        // height and has the slack around its content to grow into.
        let bleed = match split {
            true => Pixels::ZERO,
            false => snapped((far - side) / 2., window) + px(COVER_LAYER_PAD + SHADOW_BLEED),
        };

        let style = self.settings.read(cx).visualizer_style();
        let visualizer_on = self
            .settings
            .read(cx)
            .visualizer_screens()
            .contains(self.panel)
            && style.shown()
            && self.playback.read(cx).steered(cx).is_none();
        match visualizer_on
            .then(|| self.playback.read(cx).spectrum(cx))
            .flatten()
        {
            Some(spectrum) => self.visualizer.show(cx.entity_id(), spectrum, window),
            None => self.visualizer.hide(),
        }
        // The spectrum fades in and out, and the band over it with it, so both are drawn for
        // as long as any of the spectrum shows rather than for as long as it is wanted.
        let visualizer_presence = self.visualizer.presence();
        let bottom = |bounds: Bounds<Pixels>| bounds.origin.y + bounds.size.height;
        // The peaks stop an inset short of the artist line, so the glow never reaches the text.
        let visualizer_max =
            (bottom(self.root_bounds.get()) - bottom(self.meta_bounds.get()) - theme.metrics.inset)
                .max(px(VISUALIZER_MIN));
        let root_bounds = self.root_bounds.clone();
        // The visualizer and its veil sit flush with the window's bottom corners.
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let corner = crate::chrome::window_radius(self.settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let corner: Option<Pixels> = None;
        let visualizer_tint = match ambient::shown(cx) {
            true => theme.tint.map(|tint| theme.primary_of(tint)),
            false => None,
        };
        // Over a panel the visualizer covers the panel's text, so it is drawn between the panel
        // and the cover column, which keeps the controls in that column above it. There it is
        // anchored to the root's own place in the window, which no panel moves, so it stands
        // where it stood behind. The canvas behind `root_bounds` has no insets of its own and
        // starts inside the root's padding, so only its size is the root's and the corner
        // comes from `root_origin`.
        let root = Bounds::new(self.root_origin.get(), self.root_bounds.get().size);
        let over =
            self.panel.is_some() && visualizer_presence > 0. && root.size.height > Pixels::ZERO;
        let mut spectrum: Vec<AnyElement> = Vec::new();
        if visualizer_presence > 0. {
            spectrum.push(
                Visualizer::new(self.visualizer.levels(), visualizer_max)
                    .opacity(visualizer_presence)
                    .style_kind(style)
                    .when_some(visualizer_tint, |this, tint| this.tint(tint))
                    .when_some(corner, |this, radius| this.corner_radius(radius))
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .into_any_element(),
            );
        }
        // Straight over the visualizer and under the controls: a backdrop blurs only what is
        // painted before it, so the bars stop reading through the transport while the title
        // and the artist line keep their own edges. The band fades with the controls, so a
        // hidden set takes it along, and with the spectrum, so it lasts for as long as bars
        // still show under the controls. Below `Room::Wide` the bar keeps its cover and title in
        // place while the controls sleep, so there the band stays under them.
        let veiled = match narrow {
            true => 1.,
            false => 1. - hide,
        };
        if visualizer_presence > 0. && veiled > 0. && shared::effects() {
            let band = viewport.height * VEIL;
            spectrum.push(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(band)
                    .opacity(veiled * (visualizer_presence * VEIL_LEAD).min(1.))
                    .child(veil(
                        VeilEdge::Bottom,
                        band,
                        VEIL_BLUR,
                        theme.background,
                        corner,
                        window,
                    ))
                    .into_any_element(),
            );
        }
        let (behind, between) = match over {
            true => (
                Vec::new(),
                Some(
                    anchored().position(root.origin).snap_to_window().child(
                        div()
                            .relative()
                            .w(root.size.width)
                            .h(root.size.height)
                            .children(spectrum),
                    ),
                ),
            ),
            false => (spectrum, None),
        };
        let perches = match self.panel.is_some() {
            true => self.aside.update(cx, |aside, cx| aside.perches(window, cx)),
            false => Vec::new(),
        };

        div()
            .id("fullscreen")
            .role(Role::Main)
            .aria_label(t!("player-fullscreen"))
            .key_context(super::shell_context(WORKSPACE_CONTEXT))
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_5()
            // Below `Room::Wide` the page keeps the player bar's own side padding, which is the
            // corner's too, so the strip and the row along the bottom start and end where the
            // bar's do.
            .when_else(
                room.fits(Room::Wide),
                |this| this.px_8(),
                |this| this.px_5(),
            )
            .pb(foot)
            .on_click(|event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    window.dispatch_action(Box::new(ToggleWindowFullscreen), cx);
                }
            })
            .on_mouse_move(cx.listener(Self::hover))
            .on_mouse_exit(cx.listener(|this, _: &MouseExitEvent, _, cx| {
                this.left = true;
                cx.notify();
            }))
            // Capture phase: every click stirs the idle timer, even one a
            // control underneath swallows for itself. Asleep, it only wakes, which covers the
            // frame before the dormant layer first paints.
            .capture_any_mouse_down(cx.listener(
                |this, _: &MouseDownEvent, _, cx| match this.awake {
                    true => this.poke(cx),
                    false => this.rouse(cx),
                },
            ))
            // A trackpad reports fingers that only touch it as a scroll that moves nothing.
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                if event.delta.pixel_delta(window.line_height()) != Point::default() {
                    if !this.awake {
                        cx.show_cursor();
                    }
                    this.poke(cx);
                }
            }))
            .on_key_down(cx.listener(|this, _: &KeyDownEvent, _, cx| this.poke(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleLyrics, _, cx| this.toggle_panel(SideTab::Lyrics, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleQueue, _, cx| this.toggle_panel(SideTab::Queue, cx)),
            )
            .child(
                canvas(move |bounds, _, _| root_bounds.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child({
                let root_origin = self.root_origin.clone();
                canvas(
                    move |bounds, _, _| root_origin.set(bounds.origin),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
            })
            .children(behind)
            .child({
                let column = staged.then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_5()
                        .min_w_0()
                        .when_else(split, |this| this.flex_1().h_full(), |this| this.w_full())
                        .map(|this| glide(this, shift, bleed, window))
                        // A split column carries the pill in its dock, and its cover
                        // holds still while the panel beside it enters.
                        .when(!split, |this| entering(this, switching))
                        .child(self.artwork(side, raster_side, cover_scale, narrow, cx))
                        // The title and artist travel with the cover when a panel
                        // comes or goes. Below `Room::Wide` the bar along the bottom carries
                        // them, and an empty line under the cover only marks where the
                        // visualizer stops.
                        .map(|this| match narrow {
                            false => this.child(lifted(
                                Flight::new("fullscreen-meta", self.meta(hide, lift, window, cx))
                                    .unscaled()
                                    .follows(self.switch.is_some()),
                                // The column's own layer would clip the line while it
                                // is still outside the column's new bounds.
                                self.on_show,
                            )),
                            true => this.child({
                                let meta_bounds = self.meta_bounds.clone();
                                div().relative().w_full().child(
                                    canvas(
                                        move |bounds, _, _| meta_bounds.set(bounds),
                                        |_, _, _, _| {},
                                    )
                                    .absolute()
                                    .size_full(),
                                )
                            }),
                        })
                        .when(split, |this| {
                            this.child(self.dock(
                                theme.metrics.player_bar * DOCK,
                                hide,
                                false,
                                deck,
                                cx,
                            ))
                        })
                        .into_any_element()
                });
                let panel = self.panel.is_some().then(|| {
                    div()
                        .id("fullscreen-panel")
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .h_full()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child({
                            let panel_bounds = self.panel_bounds.clone();
                            canvas(
                                move |bounds, _, _| panel_bounds.set(bounds),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full()
                        })
                        .child(entering(
                            div().size_full().min_h_0().child(self.aside.clone()),
                            switching,
                        ))
                        .into_any_element()
                });
                let row = div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_8();
                // Children paint in order, so the reversed row lists the panel first and still
                // stands the column on the left.
                match over {
                    false => row.children(column).children(panel),
                    true => row
                        .flex_row_reverse()
                        .children(panel)
                        .children(between)
                        .children(column),
                }
            })
            // A panel the pill closed fades out where it stood while the cover takes the middle.
            .when_some(leaving.filter(|_| self.panel.is_none()), |this, left| {
                let at = self.panel_bounds.get();
                let origin = at.origin - self.root_origin.get();
                this.child(
                    div()
                        .absolute()
                        .left(origin.x)
                        .top(origin.y)
                        .w(at.size.width)
                        .h(at.size.height)
                        .opacity(left)
                        .child(self.aside.clone()),
                )
            })
            // The panel's floating buttons stand over it here, above the visualizer and the band
            // over it, which would otherwise cover and blur them.
            .when(!perches.is_empty(), |this| {
                let at = self.panel_bounds.get();
                let origin = at.origin - self.root_origin.get();
                this.child(entering(
                    div()
                        .absolute()
                        .left(origin.x)
                        .top(origin.y)
                        .w(at.size.width)
                        .h(at.size.height)
                        .children(perches),
                    switching,
                ))
            })
            .map(|this| match narrow {
                // The bar along the bottom spans the window edge to edge the way the player
                // bar does, over the page's side padding. It lays out on its own like the
                // player bar's cached view, so the two round to the same pixels.
                true => this
                    .child(
                        Rooted::new(self.bar(
                            hide,
                            idle,
                            self.cover_room(switching, cx),
                            window,
                            cx,
                        ))
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(PlayerBar::height(window, cx)),
                    )
                    .when(hide < 1., |this| {
                        this.child(self.floating(hide, window, cx))
                    })
                    .child(self.dock_zone(pill, window, cx))
                    // After the bar, so the cover it flies down onto has been seen this frame.
                    .when(self.landing, |this| this.children(self.landing(cx))),
                false => this
                    .when(!split, |this| {
                        this.child(self.dock(
                            theme.metrics.player_bar * DOCK_FULL,
                            hide,
                            !staged,
                            deck,
                            cx,
                        ))
                    })
                    .when(idle < 1. || hide < 1., |this| {
                        this.child(self.corner(hide, idle, window, cx))
                    }),
            })
            .when(!self.awake && self.on_show, |this| {
                this.child(self.dormant(cx))
            })
            .children(self.menu(cx))
    }
}

/// Wraps a control that a panel moves to another part of the layout, so it slides there from
/// where the one it replaces just stood rather than jumping. It keeps its size as it goes and
/// draws above the layers it crosses while the player is `on_show`. It flies only while
/// `switching`, as a panel comes or goes. Any other move is the controls' own, falling asleep
/// or waking, and follows layout at once. `reach` is how far its bubbles and pop-overs paint
/// past it, which the flying layer keeps room for until the flight has fully landed.
fn travelling(
    id: &'static str,
    name: &'static str,
    switching: bool,
    on_show: bool,
    reach: Pixels,
    element: impl IntoElement,
) -> Flight {
    lifted(
        Flight::new(id, element)
            .seen_as(name)
            .lands_from(name)
            .recent()
            .follows(switching)
            .unscaled()
            .reach(reach),
        on_show,
    )
}

/// Draws a flying control above the window only while the player is `on_show`. A player that
/// is closing fades out as one, and a control drawn above it would stay behind at full strength
/// until its flight landed.
fn lifted(flight: Flight, on_show: bool) -> Flight {
    match on_show {
        true => flight.above(),
        false => flight,
    }
}

/// Sinks, fades and blurs `element` by `by`, from 0 awake to 1 asleep, the way the docked
/// controls leave from `Room::Wide` up. Awake it takes no style at all, since even a zero offset
/// would round it apart from the same element without one.
fn asleep<E: Styled>(element: E, by: f32) -> E {
    if by <= 0. {
        return element;
    }
    let element = element.relative().top(px(SINK) * by).opacity(1. - by);
    match drowse(by) {
        Some(blur) => element.blur(blur),
        None => element,
    }
}

/// Puts `element` `by` of the way to sleep, and out of the pointer's way once it is all the way.
fn dozing<E: Styled + FluentBuilder>(element: E, by: f32) -> E {
    asleep(element, by).when(by >= 1., |this| this.invisible())
}

/// How hard controls that are `by` of the way asleep blur, if enough to be worth a layer.
fn drowse(by: f32) -> Option<Pixels> {
    let blur = DOCK_BLUR * by;
    (blur > DOCK_BLUR_LEAST && by < 1. && shared::effects()).then_some(blur)
}

/// Places `element` `offset` below where layout puts it: the whole device pixels through layout
/// and the fraction through the compositor, so a slow offset glides rather than stepping a pixel
/// at a time. An offset that sits on the device grid takes no layer. The layer clips to the
/// element's bounds, so `bleed` pads them above and below by as far as the children paint past
/// them, and matching negative margins give that room back to layout. The padding stays at rest
/// too, so nothing moves when the layer comes and goes.
fn glide<E: Styled>(element: E, offset: Pixels, bleed: Pixels, window: &Window) -> E {
    let grid = snapped(offset, window);
    let element = element.py(bleed).my(Pixels::ZERO - bleed).top(grid);
    match offset == grid {
        true => element,
        false => element.layer_translate(point(px(0.), offset - grid)),
    }
}

fn presentation_scale(presented: Pixels, layout: Pixels) -> f32 {
    presented.as_f32() / layout.as_f32()
}

/// Whether the pointer is over the window, focused or not, so hovering an unfocused player still
/// wakes its controls. macOS answers `is_window_hovered` with focus, so there the view's exit
/// event alone tells.
fn hovered(window: &Window) -> bool {
    cfg!(target_os = "macos") || window.is_window_hovered()
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
