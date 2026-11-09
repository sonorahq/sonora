//! The fullscreen player: the cover large with its title, artists and heart under it, the seek
//! bar, the transport and the volume, over the ambient field and the spectrum, and beside the
//! cover, as the pill picks, the lyrics or the queue. The controls go to sleep after the pointer
//! rests, the cover growing into the room they leave, and come back as it moves.
//!
//! Playback's position never runs this view: the seek bar and its clocks draw from `ui.live`,
//! the spectrum from the visualizer's own thread and the ambient field from Rune's loops. The view
//! runs when the controls wake or sleep, when the track or a setting changes, and on resize.

use std::cell::Cell;
use std::time::{Duration, Instant};

use i18n::t;
use music::Track;
use router::Destination;
use rune::{
    Align, Cursor, Cx, El, Link, LinkPaint, PointerEvent, Side, SpringPair, Transition, Tx,
    WheelEvent, col, el, rich, row, text,
};
use state::{FullscreenControlsAutohide, SideTab};
use ui::{
    Button, Cover, ExplicitBadge, InlineLink, InlineLinks, LEADING, Room, Scrubber, ScrubberState,
    Springs, TabBar, Text, Theme, VeilEdge, Weight,
};

use crate::chrome::{aside, title_bar};
use crate::shared::transport::{self, COVER_FLIGHT, RESTORE};
use crate::shared::visualizer::VisualizerDrive;
use crate::shared::{ambient, dialogs};
use crate::{Ui, act};

/// The share of the window's height and width the cover takes, in a wide window and a tight one,
/// with the controls up and at rest.
const COVER_TALL: f32 = 0.46;
const COVER_WIDE: f32 = 0.34;
const COVER_TALL_TIGHT: f32 = 0.6;
const COVER_WIDE_TIGHT: f32 = 0.86;
const COVER_TALL_REST: f32 = 0.56;
const COVER_WIDE_REST: f32 = 0.4;
const COVER_TALL_TIGHT_REST: f32 = 0.72;
/// The smallest and largest the cover gets, awake and at rest.
const COVER_MIN: f32 = 96.;
const COVER_MAX: f32 = 520.;
const COVER_MAX_REST: f32 = 560.;
/// Room the resting cover keeps from the window's top edge.
const COVER_PAD: f32 = 2.;
/// The page's gaps and bottom padding and the meta block's gap, in rems, so the cover is fitted
/// against the room actually left over.
const COLUMN_GAP: f32 = 1.25;
const PAGE_PAD: f32 = 1.5;
const PAGE_SIDE: f32 = 2.;
const ROW_GAP: f32 = 2.;
const META_GAP: f32 = 0.25;
/// The height the controls take, in player bars, beside the cover and along the bottom.
const DOCK: f32 = 1.15;
const DOCK_FULL: f32 = 1.7;
/// How far the controls, the pill and the leave button sink as they go to sleep.
const SINK: f32 = 24.;
/// How far below a player bar's middle the leave button sits, level with the volume glyph.
const LEAVE_DROP: f32 = 2.;
/// The pill's padding around its tabs.
const PILL_GAP: f32 = 2.;
/// The widest the controls grow.
const SEEK_MAX: f32 = 420.;
/// The volume slider's height, and the room between it and its button the pointer may cross.
const VOLUME_RISE: f32 = 132.;
const VOLUME_ZONE: f32 = 14.;
/// How far the level bubble stands off the volume track, clear of the thumb.
const VOLUME_BUBBLE_LIFT: f32 = 18.;
/// How wide a clock is, in multiples of its text size, for times under an hour and over.
const CLOCK_SHORT: f32 = 3.4;
const CLOCK_LONG: f32 = 5.4;
/// The lowest the spectrum reaches up, whatever the room under the artist line.
const VISUALIZER_MIN: f32 = 160.;
/// How tall the blurred band under the controls is, as a share of the window, and how hard it
/// blurs the spectrum passing under them.
const VEIL: f32 = 0.34;
const VEIL_BLUR: f32 = 12.;
/// How long the pointer rests before the controls sleep, and how soon after a wake another
/// movement counts again.
const REST: Duration = Duration::from_millis(1000);
const WAKE_DEBOUNCE: Duration = Duration::from_millis(400);
/// How far the pointer travels before it wakes the controls, so a resting hand's drift on a
/// trackpad leaves them asleep.
const WAKE_TRAVEL: f32 = 3.;
/// The window lyrics are set for in fullscreen; a larger one grows them, up to this much.
const LYRICS_FRAME: [f32; 2] = [1152., 648.];
const LYRICS_FRAME_MOST: f32 = 2.;
/// How small a new track's cover starts as it springs in.
const COVER_SPRING_FROM: f32 = 0.92;
/// How far the shell zooms as it opens and closes.
const SHELL_ZOOM: f32 = 0.97;
/// The bouncy spring a new cover lands on.
const COVER_SPRING: rune::Spring = rune::Spring::new(0.5, 0.22);
/// The size the shell assumes before the window has been measured.
const UNMEASURED: [f32; 2] = [1280., 800.];

/// The fullscreen player's own view state: whether the controls are awake, what the pointer is
/// over, the volume slider and the spectrum's drive.
pub struct State {
    awake: bool,
    last_moved: Instant,
    /// Where the pointer was when it last woke the controls, which travel is measured from.
    last_pos: Option<[f32; 2]>,
    /// What the pointer rests on: any of it keeps the controls awake.
    over_controls: bool,
    over_pill: bool,
    over_leave: bool,
    over_sound: bool,
    over_zone: bool,
    volume_held: bool,
    over_seek: Option<f32>,
    /// The level before muting, which unmuting brings back.
    muted: Option<f32>,
    volume: ScrubberState,
    /// Whether the last poll found the shell on screen, so entering and leaving act once.
    shown: bool,
    /// Whether opening the shell put the window into OS fullscreen, so closing it leaves.
    os_entered: bool,
    visualizer: VisualizerDrive,
    /// The window's size as the view last read it, for the lyrics' frame outside the view.
    size: Cell<[f32; 2]>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            awake: true,
            last_moved: Instant::now(),
            last_pos: None,
            over_controls: false,
            over_pill: false,
            over_leave: false,
            over_sound: false,
            over_zone: false,
            volume_held: false,
            over_seek: None,
            muted: None,
            volume: ScrubberState::new(),
            shown: false,
            os_entered: false,
            visualizer: VisualizerDrive::default(),
            size: Cell::new(UNMEASURED),
        }
    }
}

impl State {
    /// Wakes the controls, and restarts the clock that puts them to sleep.
    fn stir(&mut self) {
        self.awake = true;
        self.last_moved = Instant::now();
    }

    /// Clears what the pointer rested on. Elements that go away cannot report a leave.
    fn unpark(&mut self) {
        self.over_controls = false;
        self.over_pill = false;
        self.over_leave = false;
        self.over_sound = false;
        self.over_zone = false;
        self.over_seek = None;
    }

    fn volume_open(&self) -> bool {
        self.over_sound || self.over_zone || self.volume_held
    }
}

/// How the shell lays itself out this run.
struct Frame {
    size: [f32; 2],
    room: Room,
    panel: Option<SideTab>,
    split: bool,
    /// Whether the controls are put away, by the pointer resting or by the setting.
    hide: bool,
    /// Whether the pointer has rested, which the leave button and the cursor follow whatever
    /// the setting says about the controls.
    idle: bool,
    frosted: bool,
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let state = &ui.screens.fullscreen;
    // the root is the one element on both sides of the switch, so the size is known at once
    let size = cx
        .layout_size("ui-root")
        .map(|[w, h]| [w.round(), h.round()])
        .unwrap_or(UNMEASURED);
    state.size.set(size);
    let room = Room::of(size[0]);
    let panel = shown_panel(ui);
    let idle = !state.awake;
    let hide = match settings.fullscreen_controls_autohide() {
        FullscreenControlsAutohide::Automatic => idle,
        FullscreenControlsAutohide::AlwaysShown => false,
        FullscreenControlsAutohide::AlwaysHidden => true,
    };
    let frame = Frame {
        size,
        room,
        panel,
        split: room.fits(Room::Wide) && panel.is_some(),
        hide,
        idle,
        frosted: ambient::shown(ui),
    };
    let staged = panel.is_none() || frame.split;
    let corner = title_bar::window_radius(ui);
    let rem = theme.font_size;

    let mut main = row()
        .key("fullscreen-main")
        .grow(1.)
        .min_h(0.)
        .w_full()
        .center_y()
        .justify_between()
        .gap(rem * ROW_GAP);
    if staged {
        main = main.child(stage(ui, &frame, layer));
    }
    if let Some(tab) = panel {
        main = main.child(panel_view(ui, &frame, tab, cx, layer));
    }
    let mut body = col()
        .key("fullscreen-body")
        .grow(1.)
        .min_h(0.)
        .w_full()
        .gap(rem * COLUMN_GAP)
        .px(rem * PAGE_SIDE)
        .pb(rem * PAGE_PAD)
        .child(main);
    if !frame.split && panel.is_some() {
        body = body.child(strip(ui, &frame));
    }
    if !frame.split {
        body = body.children(dock(ui, &frame, layer));
    }

    let bar = title_bar::bar(
        ui,
        cx,
        layer,
        title_bar::Options {
            navigation: false,
            border: false,
            transparent: frame.frosted,
        },
    );

    let mut root = col()
        .key("fullscreen")
        .role("main")
        .label(t!("player-fullscreen"))
        .w_full()
        .h_full()
        .color(theme.foreground.color())
        .on_pointer_move_local(hover)
        .on_wheel(|ui: &mut Ui, wheel: WheelEvent, tx: &mut Tx| {
            // a trackpad reports fingers that only touch it as a scroll that moves nothing
            if wheel.dx != 0. || wheel.dy != 0. {
                poke(ui, tx);
            } else {
                tx.live_only();
            }
            false
        })
        .on_double_press_tx(|_: &mut Ui, tx: &mut Tx| tx.toggle_full_screen())
        .transition(Transition::opacity().combined(Transition::scale(SHELL_ZOOM)));
    if let Some(radius) = corner {
        root = root.radius(radius).clip_rounded(radius);
    }
    root = match frame.frosted {
        true => root.child(ambient::field(ui, size, corner)),
        false => root.fill(theme.background.color()),
    };
    // the pointer resting takes the cursor with it; any motion brings it back
    if idle {
        root = root.cursor(Cursor::Hidden);
    }
    let visualizer = visualizer_on(ui, panel);
    if visualizer {
        root = root.child(spectrum(ui, &frame, corner, cx));
        // straight over the spectrum and under everything else: the bars stop reading through
        // the transport while the title and the artist line keep their own edges
        if !hide && ui::effects() {
            let band = (size[1] * VEIL).round();
            root = root.child(
                el().key("fullscreen-veil")
                    .pin(Some(0.), None, Some(0.), Some(0.))
                    .h(band)
                    .inert()
                    .child(ui::veil(
                        VeilEdge::Bottom,
                        band,
                        VEIL_BLUR,
                        theme.background,
                        corner,
                    )),
            );
        }
    }
    root = root.child(bar).child(body);
    if !idle {
        root = root.child(leave(ui, &frame));
    }
    // the track menu and the dialogs it opens, and a Jam or Session invite, over everything
    dialogs::view(ui, cx, layer);
    crate::chrome::jam::mount(ui, cx, layer);
    crate::chrome::session::mount(ui, cx, layer);
    root
}

/// Follows the shell before a frame: carries the window into and out of OS fullscreen and tells
/// the display lock as the shell opens and closes, follows the panel beside the cover, feeds the
/// spectrum, and puts the controls to sleep once the pointer has rested. True when the view has
/// to run again.
pub(crate) fn poll(ui: &mut Ui, tx: &mut Tx) -> bool {
    let on = matches!(ui.nav.current(), Destination::Fullscreen);
    let state = &mut ui.screens.fullscreen;
    if on != state.shown {
        state.shown = on;
        state.unpark();
        state.volume_held = false;
        state.last_pos = None;
        state.stir();
        match on {
            true => {
                if ui.store.settings.os_fullscreen() {
                    tx.toggle_full_screen();
                    state.os_entered = true;
                }
                tx.wake_after(REST);
            }
            false => {
                // a window put into OS fullscreen by hand stays there
                if std::mem::take(&mut state.os_entered) {
                    tx.toggle_full_screen();
                }
                state.visualizer.stop();
                // the sidebar's panels start where they belong when the workspace returns
                ui.chrome.aside.lyrics.anchor();
                ui.chrome.aside.queue.anchor();
            }
        }
        ui.store.set_fullscreen(on);
    }
    if !on {
        return false;
    }

    let panel = shown_panel(ui);
    let lyrics = aside::lyrics::poll(ui, panel == Some(SideTab::Lyrics), tx);
    let queue = aside::queue::poll(ui, panel == Some(SideTab::Queue), tx);

    let spectrum = match visualizer_on(ui, panel) {
        true => ui
            .store
            .playback
            .spectrum(&ui.store.devices, &ui.store.settings),
        false => None,
    };
    ui.screens.fullscreen.visualizer.follow(spectrum);

    let busy = busy(ui);
    let state = &mut ui.screens.fullscreen;
    let mut slept = false;
    if state.awake {
        let rested = state.last_moved.elapsed();
        match (rested >= REST, busy) {
            (false, _) => tx.wake_after(REST - rested),
            (true, true) => {
                state.last_moved = Instant::now();
                tx.wake_after(REST);
            }
            (true, false) => {
                state.awake = false;
                tx.motion(Springs::SLOW);
                slept = true;
            }
        }
    }
    lyrics | queue | slept
}

/// Wakes the controls for a key press, which shows them as a pointer movement would.
pub(crate) fn stir(ui: &mut Ui, tx: &mut Tx) {
    if matches!(ui.nav.current(), Destination::Fullscreen) {
        poke(ui, tx);
    }
}

/// How much larger than `LYRICS_FRAME` the window is, which grows fullscreen lyrics.
pub(crate) fn lyrics_frame(ui: &Ui) -> f32 {
    let [w, h] = ui.screens.fullscreen.size.get();
    (w / LYRICS_FRAME[0])
        .min(h / LYRICS_FRAME[1])
        .clamp(1., LYRICS_FRAME_MOST)
}

/// The panel beside the cover: the remembered one, or the cover alone while lyrics are off and
/// the remembered one is them.
fn shown_panel(ui: &Ui) -> Option<SideTab> {
    let settings = &ui.store.settings;
    match settings.fullscreen_tab() {
        Some(SideTab::Lyrics) if !settings.lyrics_enabled() => None,
        Some(SideTab::Members) => None,
        tab => tab,
    }
}

/// Whether the spectrum shows: only over the cover alone, with the setting on and playback here.
fn visualizer_on(ui: &Ui, panel: Option<SideTab>) -> bool {
    panel.is_none()
        && ui.store.settings.visualizer_style().shown()
        && ui.store.playback.steered(&ui.store.devices).is_none()
}

/// Whether anything holds the controls awake: the pointer resting on them, a drag on a slider,
/// an open menu or dialog, or an invite the pointer over it never reaches the shell through.
fn busy(ui: &Ui) -> bool {
    let state = &ui.screens.fullscreen;
    state.over_controls
        || state.over_pill
        || state.over_leave
        || state.over_seek.is_some()
        || state.volume_open()
        || ui.chrome.player_bar.seeking.is_some()
        || ui.dialogs.menu.shows()
        || ui.dialogs.any(ui)
        || ui.store.devices.invite().is_some()
}

/// The pointer moved over the shell: once it has travelled far enough it wakes the controls.
/// Nothing the view reads changes unless they were asleep, so most moves never run it.
fn hover(ui: &mut Ui, pointer: PointerEvent, tx: &mut Tx) {
    let state = &mut ui.screens.fullscreen;
    let at = [pointer.window_x, pointer.window_y];
    let travel = state
        .last_pos
        .map_or(f32::INFINITY, |last| (at[0] - last[0]).hypot(at[1] - last[1]));
    if travel < WAKE_TRAVEL {
        tx.live_only();
        return;
    }
    state.last_pos = Some(at);
    poke(ui, tx);
}

/// Wakes the controls or keeps them awake. A run of the view is needed only to wake them.
fn poke(ui: &mut Ui, tx: &mut Tx) {
    let state = &mut ui.screens.fullscreen;
    if state.awake && state.last_moved.elapsed() < WAKE_DEBOUNCE {
        tx.live_only();
        return;
    }
    let woke = !state.awake;
    state.stir();
    tx.wake_after(REST);
    match woke {
        true => tx.motion(Springs::QUICK),
        false => tx.live_only(),
    }
}

/// A hover handler that records what the pointer rests on without running the view.
fn parks(set: fn(&mut State, bool)) -> impl Fn(&mut Ui, bool, &mut Tx) + 'static {
    move |ui: &mut Ui, over: bool, tx: &mut Tx| {
        set(&mut ui.screens.fullscreen, over);
        tx.live_only();
    }
}

/// Shows `panel` beside the cover, or the cover alone, and remembers it.
fn show(ui: &mut Ui, panel: Option<SideTab>) {
    let state = &mut ui.screens.fullscreen;
    // the pill moves between the cover and the panel, and the one left behind reports no leave
    state.over_pill = false;
    state.stir();
    match panel {
        Some(SideTab::Lyrics) => ui.chrome.aside.lyrics.anchor(),
        Some(SideTab::Queue) => ui.chrome.aside.queue.anchor(),
        _ => {}
    }
    ui.store
        .settings
        .set_fullscreen_tab(panel, &mut ui.store.cx);
}

/// The cover, the meta under it and, beside a panel, the controls under both. The cover grows
/// into the room the controls leave as they sleep, on the layout spring.
fn stage(ui: &Ui, frame: &Frame, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let rem = theme.font_size;
    let [vw, vh] = frame.size;
    let wide = frame.room.fits(Room::Wide);
    let (tall, wide_share, ceiling, tall_rest, wide_rest, ceiling_rest) = match wide {
        true => (
            COVER_TALL,
            COVER_WIDE,
            COVER_MAX,
            COVER_TALL_REST,
            COVER_WIDE_REST,
            COVER_MAX_REST,
        ),
        false => (
            COVER_TALL_TIGHT,
            COVER_WIDE_TIGHT,
            vw,
            COVER_TALL_TIGHT_REST,
            COVER_WIDE_TIGHT,
            vw,
        ),
    };
    // Everything the cover shares the column with: the title bar above it, the gaps, the meta
    // and the page's bottom padding. A window too short for all of it shrinks the cover.
    let line = |step: Text| (theme.text(step) * LEADING).round();
    let meta = line(Text::Title) + rem * META_GAP + line(Text::Body);
    let stack = theme.metrics.title_bar + rem * (COLUMN_GAP * 2. + PAGE_PAD) + meta;
    let dock = theme.metrics.player_bar
        * match frame.split {
            true => DOCK,
            false => DOCK_FULL,
        };
    let fit = |tall: f32, wide: f32, reserve: f32, ceiling: f32| {
        (vh * tall)
            .min(vh - reserve)
            .min(vw * wide)
            .min(ceiling)
            .max(COVER_MIN)
    };
    let near = fit(tall, wide_share, stack + dock, ceiling);
    // the resting cover never reaches further up than the room above the awake one
    let slack = (vh - stack - dock - near).max(0.);
    let above = theme.metrics.title_bar + slack / 2. - COVER_PAD;
    let far = fit(tall_rest, wide_rest, stack, ceiling_rest)
        .min(near + above * 2.)
        .max(near);
    let side = match frame.hide {
        true => far,
        false => near,
    }
    .round();

    let mut column = col()
        .key("fullscreen-stage")
        .center()
        .gap(rem * COLUMN_GAP)
        .min_w(0.)
        .motion(Springs::SLOW)
        .child(artwork(ui, side, far.round()))
        .child(meta_view(ui, frame));
    column = match frame.split {
        true => column.grow(1.).h_full(),
        // the cover alone enters as a panel does when the pill brings it back
        false => column.w_full().transition(entrance()),
    };
    if frame.split {
        column = column.children(dock(ui, frame, layer));
    }
    column
}

/// The cover, which opens its album. A new track's cover springs in over the last one, and the
/// larger renditions fade in over the thumbnail as they land.
fn artwork(ui: &Ui, side: f32, rest: f32) -> El {
    let theme = &ui.theme;
    let radius = theme.radius * 2.;
    let track = ui.store.playback.shown_track(&ui.store.devices);
    let small = track.as_ref().and_then(|track| track.cover.clone());
    let album = track.as_ref().and_then(|track| track.album_id.clone());
    let local = track
        .as_ref()
        .and_then(|track| track.id.as_deref())
        .is_some_and(music::is_local_id)
        || small.as_deref().is_some_and(|url| url.starts_with("file://"));
    let cover = &ui.store.cover;
    let large = album.as_deref().and_then(|album| cover.large_for(album));
    let full = album.as_deref().and_then(|album| cover.full_for(album));
    // A local cover is the full picture already. Each larger rendition fades in over the one
    // under it, so the thumbnail shows only until the large one lands.
    let upgrades = |url: &&str| !local && small.is_some() && Some(*url) != small.as_deref();
    let large = large.filter(upgrades);
    let full = full.filter(upgrades).filter(|url| Some(*url) != large);
    // every rendition decodes for the resting size, so growing into it stays sharp
    let decode = ui.artwork().scale() * rest / side.max(1.);
    let layer = |url: Option<&str>, overlay: bool| {
        let cover = Cover::new(ui.artwork(), url)
            .size(side)
            .radius(radius)
            .scale(decode)
            .fallback("icons/disc-3.svg");
        match overlay {
            true => cover.overlay(),
            false => cover,
        }
        .build(theme)
        .absolute(0., 0.)
    };
    let identity = track
        .as_ref()
        .and_then(|track| track.id.clone().or_else(|| track.cover.clone()))
        .unwrap_or_default();
    let face = el()
        .key(format!("fullscreen-cover-{identity}"))
        .absolute(0., 0.)
        .size(side, side)
        .child(layer(small.as_deref(), false))
        .children(large.map(|url| layer(Some(url), true)))
        .children(full.map(|url| layer(Some(url), true)))
        .transition(Transition::asymmetric(
            Transition::opacity()
                .combined(Transition::scale(COVER_SPRING_FROM))
                .springs(SpringPair::new(COVER_SPRING, Springs::QUICK)),
            Transition::opacity(),
        ));
    let mut art = el()
        .key("fullscreen-artwork")
        .matched(COVER_FLIGHT)
        .size(side, side)
        .child(face);
    let Some(track) = track else {
        return art.role("img").label(t!("player-nothing-playing"));
    };
    art = art
        .label(album_name(&track))
        .on_secondary_press(transport::open_menu)
        .press_scale(ui::PRESS_SCALE_CARD);
    match album {
        Some(id) => art
            .role("link")
            .focusable(true)
            .cursor(Cursor::Pointer)
            .on_press(act(move |ui: &mut Ui| {
                ui.go(Destination::Album(id.clone().into()))
            })),
        None => art.role("img"),
    }
}

/// The title with the explicit mark and the heart, over the artists. The title stays truly
/// centred: the right cell holds the mark and the heart, and the left one an invisible copy of
/// it, so both sides floor at one width and a long title truncates rather than pushing the
/// heart out of the row.
fn meta_view(ui: &Ui, frame: &Frame) -> El {
    let theme = &ui.theme;
    let rem = theme.font_size;
    let track = ui.store.playback.shown_track(&ui.store.devices);
    let trailing = |track: Option<&Track>, real: bool| {
        let explicit = ui
            .store
            .playback
            .shown_track(&ui.store.devices)
            .is_some_and(|track| track.explicit);
        let mut cell = row().center_y().gap(rem * 0.5);
        if explicit {
            let badge = ExplicitBadge::new().build(theme);
            cell = cell.child(match real {
                true => el()
                    .key("fullscreen-explicit")
                    .role("img")
                    .label(t!("song-explicit"))
                    .child(badge),
                false => badge,
            });
        }
        let heart = match real {
            true => transport::like(ui, track),
            false => el().w(theme.metrics.control_small).h(1.),
        };
        cell.child(
            el().key(match real {
                true => "fullscreen-heart",
                false => "fullscreen-heart-balance",
            })
            .opacity(match frame.hide {
                true => 0.,
                false => 1.,
            })
            .child(heart),
        )
    };
    let title = title(ui, track.as_ref(), Text::Title, "fullscreen-title");
    let headline = row()
        .key("fullscreen-headline")
        .center_y()
        .gap(rem * 0.5)
        .w_full()
        .min_w(0.)
        .child(
            row()
                .key("fullscreen-title-balance")
                .grow(1.)
                .justify_end()
                .opacity(0.)
                .inert()
                .a11y_hidden()
                .child(trailing(None, false)),
        )
        .child(title)
        .child(
            row()
                .key("fullscreen-title-trailing")
                .grow(1.)
                .child(trailing(track.as_ref(), true)),
        );
    let mut meta = col()
        .key("fullscreen-meta")
        .items_center()
        .gap(rem * META_GAP)
        .w_full()
        .min_w(0.)
        .child(headline);
    if let Some(track) = &track {
        meta = meta.child(
            row()
                .w_full()
                .min_w(0.)
                .justify_center()
                .child(artists(track, "fullscreen-artists", Text::Body, theme)),
        );
    }
    meta
}

/// The shown track's title in `step`, opening its album, with the track's menu on a secondary
/// press. With nothing playing it says so.
fn title(ui: &Ui, track: Option<&Track>, step: Text, key: &str) -> El {
    let theme = &ui.theme;
    let font = ui::font(theme, step, Weight::SEMIBOLD);
    let Some(track) = track else {
        return text(t!("player-nothing-playing"))
            .key(key)
            .role("status")
            .label(t!("player-nothing-playing"))
            .font(font)
            .truncate()
            .min_w(0.);
    };
    let name = track.name.clone();
    let line = match track.album_id.clone() {
        Some(album) => rich(name.clone(), Vec::new())
            .link(
                Link::new(0..name.len(), album)
                    .label(name.clone())
                    .hover(LinkPaint::new().underline(theme.foreground.color())),
            )
            .on_link(|ui: &mut Ui, id: &str, _: &mut Tx| {
                ui.go(Destination::Album(id.to_owned().into()));
                ui.settle();
            }),
        None => text(name.clone()).role("text").label(name),
    };
    line.key(key)
        .font(font)
        .color(theme.foreground.color())
        .truncate()
        .shrink()
        .min_w(0.)
        .on_secondary_press(transport::open_menu)
}

/// The shown track's artists, each opening its page.
fn artists(track: &Track, key: &str, step: Text, theme: &Theme) -> El {
    InlineLinks::<Ui>::new(
        key,
        track
            .artist_refs
            .iter()
            .map(|artist| InlineLink::new(artist.name.clone(), artist.id.clone())),
        track.artists.clone(),
        quiet(theme),
    )
    .text_size(theme.text(step))
    .truncate()
    .on_link(|ui: &mut Ui, id: &str, _: &mut Tx| {
        ui.go(Destination::Artist(id.to_owned().into()));
        ui.settle();
    })
    .build(theme)
}

/// The small cover, title and artists across the foot of a panel in a window too narrow to show
/// the cover beside it.
fn strip(ui: &Ui, frame: &Frame) -> El {
    let theme = &ui.theme;
    let rem = theme.font_size;
    let track = ui.store.playback.shown_track(&ui.store.devices);
    let cover = track.as_ref().and_then(|track| track.cover.as_deref());
    let mut art = el().key("strip-artwork").child(
        Cover::new(ui.artwork(), cover)
            .size(theme.metrics.row)
            .fallback("icons/disc-3.svg")
            .build(theme),
    );
    if let Some(track) = &track {
        art = art.label(album_name(track)).press_scale(ui::PRESS_SCALE_CARD);
        art = match track.album_id.clone() {
            Some(id) => art
                .role("link")
                .focusable(true)
                .cursor(Cursor::Pointer)
                .on_press(act(move |ui: &mut Ui| {
                    ui.go(Destination::Album(id.clone().into()))
                })),
            None => art.role("img"),
        };
    }
    let explicit = track.as_ref().is_some_and(|track| track.explicit);
    let mut headline = row()
        .center_y()
        .gap(rem * 0.5)
        .min_w(0.)
        .child(title(ui, track.as_ref(), Text::Body, "strip-title"));
    if explicit {
        headline = headline.child(
            el().key("strip-explicit")
                .role("img")
                .label(t!("song-explicit"))
                .child(ExplicitBadge::new().build(theme)),
        );
    }
    headline = headline.child(
        el().key("strip-heart")
            .opacity(match frame.hide {
                true => 0.,
                false => 1.,
            })
            .child(transport::like(ui, track.as_ref())),
    );
    let mut details = col()
        .justify_center()
        .grow(1.)
        .min_w(0.)
        .child(headline);
    if let Some(track) = &track {
        details = details.child(artists(track, "strip-artists", Text::Small, theme));
    }
    row()
        .key("fullscreen-strip")
        .center_y()
        .gap(rem * 0.75)
        .w_full()
        .min_w(0.)
        .child(art)
        .child(details)
}

/// The lyrics or the queue beside the cover, with the pill floating over its foot. A switch
/// plays the entrance a page plays.
fn panel_view(ui: &Ui, frame: &Frame, tab: SideTab, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let content = match tab {
        SideTab::Lyrics => aside::lyrics::view(ui, cx, layer),
        _ => aside::queue::view(ui, cx, layer),
    };
    let mut panel = col()
        .key("fullscreen-panel")
        .grow(1.)
        .min_w(0.)
        .min_h(0.)
        .h_full()
        .child(
            col()
                .key(match tab {
                    SideTab::Lyrics => "fullscreen-panel-lyrics",
                    _ => "fullscreen-panel-queue",
                })
                .role("complementary")
                .label(aside::title(tab))
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .min_h(0.)
                .transition(entrance())
                .child(content),
        );
    if !frame.hide {
        panel = panel.child(
            row()
                .key("fullscreen-floating-pill")
                .pin(Some(0.), None, Some(0.), Some(theme.font_size * 0.75))
                .justify_center()
                .transition(sinking())
                .child(pill(ui, frame)),
        );
    }
    panel
}

/// The entrance a page plays, which a panel and the cover alone play as the pill switches.
fn entrance() -> Transition {
    Transition::asymmetric(
        Transition::opacity()
            .combined(Transition::scale(0.99))
            .combined(Transition::blur(1.5)),
        Transition::opacity(),
    )
}

/// How the controls come and go as they wake and sleep: fading as they rise from and sink back
/// toward the bottom edge.
fn sinking() -> Transition {
    Transition::opacity()
        .combined(Transition::offset(0., SINK))
        .springs(SpringPair::new(Springs::QUICK, Springs::SLOW))
}

/// The cover, lyrics and queue tabs. Its selected fill flies between the tabs.
fn pill(ui: &Ui, frame: &Frame) -> El {
    let theme = &ui.theme;
    let lyrics = ui.store.settings.lyrics_enabled();
    let tab = |key: &str, icon: &str, hint: &str, target: Option<SideTab>| {
        let showing = frame.panel == target;
        let button = Button::new(key)
            .ghost()
            .small()
            .icon(icon)
            .tooltip_above(i18n::lookup(hint, None).to_string())
            .selected(showing)
            .tint(match showing {
                true => theme.foreground,
                false => theme.muted_foreground,
            })
            .on_press(act(move |ui: &mut Ui| show(ui, target)));
        match frame.frosted {
            true => button.frosted(),
            false => button,
        }
    };
    let mut items = vec![tab(
        "fullscreen-artwork-tab",
        "icons/disc-3.svg",
        "fullscreen-artwork",
        None,
    )];
    if lyrics {
        items.push(tab(
            "fullscreen-lyrics",
            "icons/mic-vocal.svg",
            "lyrics-title",
            Some(SideTab::Lyrics),
        ));
    }
    items.push(tab(
        "fullscreen-queue",
        "icons/list-music.svg",
        "queue-title",
        Some(SideTab::Queue),
    ));
    let mut bar = TabBar::new("fullscreen-pill-bar")
        .name(t!("fullscreen-panels"))
        .radius(theme.radius + PILL_GAP)
        .items(items);
    if frame.frosted {
        bar = bar.blurred();
    }
    el().key("fullscreen-pill")
        .on_hover_change(|ui: &mut Ui, over: bool, tx: &mut Tx| {
            ui.screens.fullscreen.over_pill = over;
            match over {
                true => poke(ui, tx),
                false => tx.live_only(),
            }
        })
        .child(bar.build(theme))
}

/// The controls: the pill while the cover shows alone, the seek bar and the transport with the
/// volume at its end. They leave as the controls sleep and come back as they wake.
fn dock(ui: &Ui, frame: &Frame, layer: &mut Vec<El>) -> Option<El> {
    if frame.hide {
        return None;
    }
    let theme = &ui.theme;
    let rem = theme.font_size;
    // below Roomy the row spans the window, so the volume steps in clear of the leave button
    let clear = match frame.room.fits(Room::Roomy) {
        true => 0.,
        false => theme.metrics.control_small,
    };
    let mut controls = col()
        .key("fullscreen-controls")
        .items_center()
        .gap(rem * 0.75)
        .w_full()
        .max_w(SEEK_MAX)
        .on_hover_change(parks(|state, over| state.over_controls = over));
    if frame.panel.is_none() {
        controls = controls.child(pill(ui, frame));
    }
    controls = controls.child(seek(ui)).child(
        row()
            .key("fullscreen-transport")
            .w_full()
            .justify_center()
            .child(transport::transport(ui, true))
            .child(
                row()
                    .key("fullscreen-sound-slot")
                    .pin(None, Some(0.), Some(clear), Some(0.))
                    .center_y()
                    .child(sound(ui, frame, layer)),
            ),
    );
    Some(
        row()
            .key("fullscreen-dock")
            .w_full()
            .justify_center()
            .transition(sinking())
            .child(controls),
    )
}

/// The elapsed time, the seek bar and the length. The bar and both clocks follow playback through
/// live values, so a playing track never runs the view.
fn seek(ui: &Ui) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.fullscreen;
    let (_, total) = ui.store.playback.shown_time(&ui.store.devices);
    let seekable = ui.store.playback.shown_track(&ui.store.devices).is_some();
    let size = theme.text(Text::Tiny);
    let width = size
        * match total.as_secs() >= 3600 {
            true => CLOCK_LONG,
            false => CLOCK_SHORT,
        };
    let clock = |key: &str, live: &rune::LiveText, end: bool| {
        text("0:00")
            .key(key)
            .role("text")
            .live_text(live)
            .font(ui::font(theme, Text::Tiny, Weight::NORMAL).features(ui::TABULAR))
            .color(theme.muted_foreground.color())
            .align(match end {
                true => rune::Align::End,
                false => rune::Align::Start,
            })
            .w(width)
    };
    let mut bar = Scrubber::<Ui>::live("fullscreen-seek-bar", &ui.live.seek)
        .name(t!("song-position"))
        .colors(
            theme.progress_bar,
            theme.muted_foreground.opacity(0.3),
            theme.foreground,
        )
        .enabled(seekable)
        // the player bar's hold on the bar is the one `Ui::poll` respects
        .on_move(|ui: &mut Ui, fraction: f32, _: &mut Tx| {
            ui.chrome.player_bar.seeking = Some(fraction);
        })
        .on_release(|ui: &mut Ui, _: &mut Tx| {
            if let Some(fraction) = ui.chrome.player_bar.seeking.take() {
                ui.store.seek_fraction(fraction);
                ui.settle();
            }
        })
        .on_hover(|ui: &mut Ui, at: Option<f32>, tx: &mut Tx| {
            let state = &mut ui.screens.fullscreen;
            match transport::moved(state.over_seek, at) {
                true => state.over_seek = at,
                false => tx.live_only(),
            }
        });
    if let Some(at) = state.over_seek.or(ui.chrome.player_bar.seeking) {
        bar = bar.bubble(at, ui::clock(total.mul_f32(at)));
    }
    row()
        .key("fullscreen-seek")
        .role("group")
        .label(t!("song-position"))
        .center_y()
        .gap(theme.font_size * 0.5)
        .w_full()
        .child(clock("fullscreen-elapsed", &ui.live.elapsed, true))
        .child(col().grow(1.).min_w(0.).child(bar.build(theme)))
        .child(clock("fullscreen-total", &ui.live.total, false))
}

/// The mute button, with the volume slider rising over it while the pointer is anywhere over
/// the two. The wheel over either turns the volume.
fn sound(ui: &Ui, frame: &Frame, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.fullscreen;
    let level = ui.store.playback.shown_volume(&ui.store.devices);
    let muted = level <= 0.001;
    let open = state.volume_open();
    let mut button = Button::new("fullscreen-volume")
        .ghost()
        .small()
        .icon(transport::volume_icon(level))
        // no tooltip: it would sit over the slider rising from the same spot
        .name(match muted {
            true => t!("player-unmute"),
            false => t!("player-mute"),
        })
        .tint(match open {
            true => theme.foreground,
            false => theme.muted_foreground,
        })
        .on_press(act(move |ui: &mut Ui| {
            let state = &mut ui.screens.fullscreen;
            let wanted = match muted {
                true => state.muted.unwrap_or(RESTORE),
                false => 0.,
            };
            state.muted = (!muted).then_some(level);
            ui.store.set_volume(wanted);
        }));
    if frame.frosted {
        button = button.frosted();
    }
    if open {
        layer.push(volume_panel(ui, frame, level));
    }
    el().key("fullscreen-sound")
        .on_wheel(turn_volume)
        .on_hover_change(|ui: &mut Ui, over: bool, _: &mut Tx| {
            ui.screens.fullscreen.over_sound = over;
        })
        .child(button.build(theme))
}

/// The vertical volume slider over the mute button, inside a zone that reaches down over the
/// button so the pointer crossing between the two keeps it open. It rises the way a menu does.
fn volume_panel(ui: &Ui, frame: &Frame, level: f32) -> El {
    let theme = &ui.theme;
    let pad = theme.metrics.pad;
    let span = theme.metrics.control_small + VOLUME_ZONE * 2.;
    let mut slider = Scrubber::<Ui>::new("fullscreen-volume-slider", &ui.screens.fullscreen.volume, level)
        .vertical()
        .name(t!("equalizer-status-volume"))
        .value(transport::percent(level))
        .colors(
            theme.progress_bar,
            theme.muted_foreground.opacity(0.3),
            theme.foreground,
        )
        .lift(VOLUME_BUBBLE_LIFT)
        .on_move(|ui: &mut Ui, level: f32, _: &mut Tx| {
            let state = &mut ui.screens.fullscreen;
            state.volume_held = true;
            state.muted = None;
            ui.store.slide_volume(level);
            ui.settle();
        })
        .on_release(|ui: &mut Ui, _: &mut Tx| {
            ui.screens.fullscreen.volume_held = false;
            ui.store.end_volume_slide();
            ui.settle();
        });
    slider = slider.bubble(level, transport::percent(level));
    let mut panel = col()
        .key("fullscreen-volume-panel")
        .role("group")
        .label(t!("equalizer-status-volume"))
        .items_center()
        .px(pad * 0.5)
        .py(pad)
        .radius(theme.radius)
        .border(1., theme.border.color())
        .on_wheel(turn_volume)
        .child(el().h(VOLUME_RISE).child(slider.build(theme)));
    panel = match frame.frosted && ui::blurring(theme) {
        true => ui::glass(panel, theme),
        false => panel.fill(theme.popover.color()),
    };
    col()
        .key("fullscreen-volume-zone")
        .w(span)
        .items_center()
        .pt(VOLUME_ZONE)
        .pb(theme.metrics.control_small + PILL_GAP * 2.)
        .anchored(
            "fullscreen-volume",
            Side::Top,
            Align::Center,
            -theme.metrics.control_small,
        )
        .on_hover_change(|ui: &mut Ui, over: bool, _: &mut Tx| {
            ui.screens.fullscreen.over_zone = over;
        })
        .transition(ui::rise(Side::Top, Align::Center))
        .child(panel)
}

/// Turns the volume a notch for each step of the wheel over the volume control.
fn turn_volume(ui: &mut Ui, wheel: WheelEvent, _: &mut Tx) -> bool {
    if wheel.dy == 0. {
        return false;
    }
    let level = transport::turned(ui.store.playback.shown_volume(&ui.store.devices), wheel.dy);
    ui.screens.fullscreen.muted = None;
    ui.store.set_volume(level);
    ui.settle();
    true
}

/// The spectrum along the window's foot, reaching up to an inset short of the artist line.
fn spectrum(ui: &Ui, frame: &Frame, corner: Option<f32>, cx: &mut Cx<Ui>) -> El {
    let theme = &ui.theme;
    let [vw, vh] = frame.size;
    let meta = cx
        .layout_with("fullscreen-meta", |rect| (rect[1] + rect[3]).round())
        .unwrap_or(vh / 2.);
    let max = (vh - meta - theme.metrics.inset).max(VISUALIZER_MIN).round();
    let mut view = ui
        .screens
        .fullscreen
        .visualizer
        .visualizer()
        .view(vw, max)
        .style(ui.store.settings.visualizer_style());
    if frame.frosted
        && let Some(tint) = theme.tint
    {
        view = view.tint(theme.primary_of(tint));
    }
    if let Some(radius) = corner {
        view = view.corner_radius(radius);
    }
    view.build(theme)
        .key("fullscreen-spectrum")
        .pin(Some(0.), None, Some(0.), Some(0.))
        .inert()
}

/// The button in the bottom right corner that leaves fullscreen, centred on a player bar's
/// height. It sinks and fades as the pointer rests, whatever the setting says about the
/// controls, so moving the pointer always brings it back.
fn leave(ui: &Ui, frame: &Frame) -> El {
    let theme = &ui.theme;
    let mut button = Button::new("leave-fullscreen")
        .ghost()
        .small()
        .icon("icons/minimize.svg")
        .tooltip_above(t!("player-fullscreen-leave"))
        .on_press(act(|ui: &mut Ui| ui.back()));
    if frame.frosted {
        button = button.frosted();
    }
    col()
        .key("leave-fullscreen-slot")
        .pin(
            None,
            None,
            Some(theme.font_size * 1.25),
            Some(-LEAVE_DROP),
        )
        .h(theme.metrics.player_bar.round())
        .justify_center()
        .on_hover_change(parks(|state, over| state.over_leave = over))
        .transition(sinking())
        .child(button.build(theme))
}

/// What a cover that opens the track's album is called: the album's name, or View album when
/// the track does not name one.
fn album_name(track: &Track) -> String {
    match track.album.is_empty() {
        true => t!("song-view-album").into_owned(),
        false => track.album.clone(),
    }
}

/// The foreground thinned, for text over the ambient field: translucent light takes on the
/// colours behind it where a flat grey would stand apart.
fn quiet(theme: &Theme) -> ui::Hsla {
    theme.foreground.opacity(0.56)
}
