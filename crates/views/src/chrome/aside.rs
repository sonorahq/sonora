use std::cell::Cell;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::rc::Rc;
use std::{collections::HashMap, ops::Range};

use gpui::prelude::*;

use gpui::accesskit::{Role, Toggled};
use gpui::{
    Animation, AnimationExt as _, App, Bounds, Context, Div, DragMoveEvent, Entity, FontWeight,
    MouseDownEvent, MouseUpEvent, Pixels, Point, Render, ScrollHandle, ScrollStrategy,
    SharedString, Size, SpringConfig, SpringState, Task, UniformListScrollHandle, Window, canvas,
    div, ease_in_out, px, relative, size, svg, uniform_list,
};
use i18n::t;
use music::{Contributor, Shape, Track, Voice};
use router::{Destination, LibraryTab, Link as _};
use state::{
    AppSettings, Lyrics, LyricsState, Network, Playback, PlaybackState, Queue, QueuePosition,
    QueueSections as Sections, QueueSlot as Slot, QueueView, RomanizationScripts, Shelf, SideTab,
    Sonora, Whence,
};
use ui::{
    ActiveTheme as _, Avatar, Button, Card, DraggedPin, Edge, Initials, Motion, Motioned as _,
    PREFETCH_REACH, Pin, Pinnable as _, Popup, Scrollbar, Scroller, Spot, Springs, Tails as _,
    Text, Vacancy, drop_gap, drop_marker, ease_out_cubic, ease_out_expo, eyebrow, faint, mix,
    prefetch, snapped, vacant,
};
use unicode_bidi::{BidiClass, Direction};

use crate::chrome::{Chrome, section_label};
use crate::shared::effects;
use crate::shared::menus::ItemMenu;
use crate::shared::pins::Pinned as _;

const QUEUE: &str = "queue";
/// How many text heights across the picture of whoever queued a song is.
const ADDER_FACE: f32 = 1.4;
const BULLET: SharedString = SharedString::new_static("·");
const FADE: f32 = 96.;
const REST: f32 = FADE * 0.75;
/// Blank rows after the last queued song, on top of the ones that clear the player bar.
const TAIL_ROWS: usize = 1;
const BLUR: f32 = 0.13;
const BACKGROUND_SINGING_BLUR: Pixels = px(0.75);
const VEIL: f32 = 0.3;
const HAZE: f32 = 0.45;
const VERSE_FADE: f32 = 1.25;
const PAST: f32 = 0.4;
const AHEAD: f32 = 0.6;
const REVEAL: f32 = 0.6;
const ACTIVE_VERSE_GROWTH: Pixels = px(2.);
const FULLSCREEN_VERSE_GROWTH: Pixels = px(3.);
/// How much larger than the lane text unsynced lyrics are set, while staying below a synced verse.
const PLAIN_LYRICS_GROWTH: f32 = 1.15;
const LYRICS_HORIZONTAL_INSET_REM: f32 = 1.5;
// The largest window fullscreen lyrics keep their own size in. A larger one grows them by its
// share of this one, so a verse keeps its size relative to the window.
const LYRICS_FRAME: Size<Pixels> = size(px(1152.), px(648.));
// the most a large window grows fullscreen lyrics by
const LYRICS_FRAME_MOST: f32 = 2.;
const PINNED_SHARE: f32 = 0.25;
const PIN: f32 = 0.3;
// how far a row falls behind, in verse sizes
const LAG: f32 = 24.;
// never past this share
const LAG_SHARE: f32 = 0.28;
// movement the last row skips
const LAG_TRAIL: f32 = 0.9;
// The first row's physical spring. Rows farther along the viewport keep the same damping ratio but
// use a lower natural frequency, producing the cascading iMessage-like settle.
const LAG_STAGGER: f32 = 0.35;
const LAG_LEAST: Pixels = px(0.05);
const LAG_STALL: f32 = 0.064;
// Below this a blur is not worth a layer of its own.
const HAZE_LEAST: Pixels = px(0.05);
/// The step a row's blur is rounded to. Neighbouring rows that land on the same step share one
/// offscreen layer in the renderer, where each distinct blur costs a layer of its own.
const HAZE_STEP: Pixels = px(0.25);
// How far a verse sinks while it is held.
const PRESSED: f32 = 0.955;
// The widest a line of lyrics is set, in multiples of its own size. Left to fill
// a fullscreen panel, a lead verse and a background one end up at opposite edges.
const REACH: f32 = 24.;
// What a sheet settling on the best answer comes in through: it blurs and fades
// on the way, once, on a curve that is the same going in as coming out.
const RESOLVE_BLUR: f32 = 0.2;
const RESOLVE_FADE: f32 = 0.5;
const SETTLE: std::time::Duration = std::time::Duration::from_secs(4);
const INSTRUMENTAL_BREAK: std::time::Duration = std::time::Duration::from_secs(5);
const SWEEP_LEAST: std::time::Duration = std::time::Duration::from_millis(180);
// karaoke sweep ceiling
const KARAOKE_HZ: u32 = 45;
const KARAOKE_FRAME: std::time::Duration =
    std::time::Duration::from_nanos(1_000_000_000 / KARAOKE_HZ as u64);
const SWEEP_STRETCH: f32 = 1.4;
const SWEPT: f32 = 0.98;
const LANDING: f32 = 0.2;
// what a lane row actually takes, plus the gaps between lanes
const LANE_GAP_REM: f32 = 0.25;
const LANE_SLACK: f32 = 0.25;

/// A place in the sheet the pointer can be over: a verse, or the melody break
/// above it. They share a line index, so they need telling apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Warm {
    Verse(usize),
    Break(usize),
}

/// How near the pointer a spot is, and how far it has been pressed.
#[derive(Clone, Copy, Default)]
struct Touch {
    warmth: f32,
    depth: f32,
    waking: bool,
    settling: bool,
}

#[derive(Clone, Copy)]
struct Sung {
    karaoke: bool,
    lane: Pixels,
    scripts: Option<RomanizationScripts>,
    theme: ui::Theme,
    karaoke_tint: gpui::Hsla,
    lift: f32,
    from: gpui::Point<f32>,
    /// Whether the lyrics mostly read right to left, which puts the lead voice on the right.
    mirrored: bool,
}

/// How one verse is drawn outside the sheet, for a still picture of the lyrics such as the
/// settings preview. The sizes are the sheet's own, before an active verse grows.
#[derive(Clone, Copy)]
pub(crate) struct VerseLook {
    pub(crate) active: bool,
    /// The verse's text size and its romanized lane's.
    pub(crate) size: Pixels,
    pub(crate) lane: Pixels,
    pub(crate) karaoke: bool,
    pub(crate) scripts: Option<RomanizationScripts>,
    /// How far out of focus the verse sits, from 0 for sharp to 1 at the sheet's edge.
    pub(crate) haze: f32,
}

#[derive(Clone, Copy)]
struct RowLook {
    playing: bool,
    drop_line: Option<Edge>,
    /// Whether the row is a song someone queued by hand, which names who did when it is known.
    queued: bool,
    /// Whether the row is on another device's queue, which this app shows but cannot change.
    remote: bool,
}

/// The credit under a sheet, resolved once for the answer it names and the language it was read
/// in, since the sheet redraws every frame while a song plays.
struct Credit {
    source: &'static str,
    writers: Vec<String>,
    language: i18n::Language,
    lines: (SharedString, Option<SharedString>),
}

/// What the height of a settled lyrics row depends on besides its own words. A row off screen is
/// drawn as a spacer of the height it was last laid out at, and only while none of this has
/// changed since then.
#[derive(Clone, Copy, PartialEq)]
struct SheetLayout {
    viewport: Size<Pixels>,
    width: Pixels,
    verse: Pixels,
    lane: Pixels,
    scripts: Option<RomanizationScripts>,
    rem: Pixels,
    scale: f32,
    font: u64,
}

#[derive(Clone)]
struct ContextMenuState {
    track: Track,
    revision: u64,
    position: Point<Pixels>,
}

pub(crate) struct Aside {
    queue: Entity<Queue>,
    playback: Entity<Playback>,
    lyrics: Entity<Lyrics>,
    settings: Entity<AppSettings>,
    tab: SideTab,
    verse_bar: Entity<Scrollbar>,
    followed: Option<usize>,
    nudges: u64,
    pinned: bool,
    nudged: Option<std::time::Instant>,
    verse_of: Option<String>,
    verse_take: u64,
    placing: bool,
    context_menu: Option<ContextMenuState>,
    track_menu: ItemMenu,
    drop_gap: Option<usize>,
    scroll: UniformListScrollHandle,
    scrollbar: Entity<Scrollbar>,
    past_len: usize,
    /// The id of the steered device's track the list was last pinned for, or `None` over this
    /// app's own queue. A change pins the now-playing row again.
    queue_track: Option<String>,
    anchor: bool,
    titled: bool,
    aiming: bool,
    rested: Option<Pixels>,
    since: std::time::Instant,
    over: Option<Warm>,
    hovered: Option<Warm>,
    fading: Option<Warm>,
    linger: Option<Task<()>>,
    previous_active_line: Option<usize>,
    departing_line: Option<usize>,
    departed: std::time::Instant,
    arrived: std::time::Instant,
    arrival: u64,
    departure: u64,
    lyrics_wrap_width: Option<Pixels>,
    lyrics_wrap_size: Option<Pixels>,
    lyrics_wraps: HashMap<usize, Wrapped>,
    lane_rooms: HashMap<usize, Pixels>,
    seen: Pixels,
    flying: bool,
    flew: bool,
    slid: std::time::Instant,
    drifts: HashMap<usize, SpringState>,
    pinning: Option<usize>,
    held: Option<Warm>,
    rising: Option<Warm>,
    sank: std::time::Instant,
    sinking: Option<Task<()>>,
    swept_frame: std::time::Instant,
    sweeping: Option<Task<()>>,
    showed: bool,
    resolving: bool,
    credit: Option<Credit>,
    /// Set once the sheet has been laid out, and taken by the next render, so rows are only stood
    /// in for while the bounds the scroll handle holds are the last frame's.
    laid: Rc<Cell<bool>>,
    sheet_layout: Option<SheetLayout>,
    /// Which rows this frame drew with nothing about them in motion, by row.
    settled: Vec<bool>,
    /// The same for the frame before, which the scroll handle's bounds describe.
    was_settled: Vec<bool>,
}

impl Aside {
    pub(crate) fn new(
        queue: Entity<Queue>,
        playback: Entity<Playback>,
        tab: SideTab,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&queue, |this, queue, cx| {
            let revision = queue.read(cx).revision();
            if this
                .context_menu
                .as_ref()
                .is_some_and(|menu| menu.revision != revision)
            {
                this.track_menu.reset(cx);
                this.context_menu = None;
            }
            cx.notify();
        })
        .detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        let me = cx.entity_id();
        let scroll = UniformListScrollHandle::new();
        let scrollbar =
            cx.new(|_| Scrollbar::new(scroll.0.borrow().base_handle.clone()).watching(me));
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));
        let lyrics = Sonora::global(cx).lyrics.clone();
        cx.observe(&lyrics, |_, _, cx| cx.notify()).detach();
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let verse_bar = cx.new(|_| {
            Scrollbar::new(ScrollHandle::new())
                .spring(Springs::LYRICS_SCROLL)
                .watching(me)
        });

        Self {
            queue,
            playback,
            lyrics,
            settings,
            tab,
            verse_bar,
            followed: None,
            nudges: 0,
            pinned: true,
            nudged: None,
            verse_of: None,
            verse_take: 0,
            placing: false,
            context_menu: None,
            track_menu: ItemMenu::new(playlist_scrollbar, cx),
            drop_gap: None,
            scroll,
            scrollbar,
            past_len: 0,
            queue_track: None,
            anchor: true,
            titled: true,
            aiming: false,
            rested: None,
            since: std::time::Instant::now(),
            over: None,
            hovered: None,
            fading: None,
            linger: None,
            previous_active_line: None,
            departing_line: None,
            departed: std::time::Instant::now(),
            arrived: std::time::Instant::now(),
            arrival: 0,
            departure: 0,
            lyrics_wrap_width: None,
            lyrics_wrap_size: None,
            lyrics_wraps: HashMap::new(),
            lane_rooms: HashMap::new(),
            seen: px(0.),
            flying: false,
            flew: true,
            slid: std::time::Instant::now(),
            drifts: HashMap::new(),
            pinning: None,
            held: None,
            rising: None,
            sank: std::time::Instant::now(),
            sinking: None,
            swept_frame: std::time::Instant::now(),
            sweeping: None,
            showed: false,
            resolving: false,
            credit: None,
            laid: Rc::new(Cell::new(false)),
            sheet_layout: None,
            settled: Vec::new(),
            was_settled: Vec::new(),
        }
    }

    pub(crate) fn strip(&mut self) {
        self.titled = false;
    }

    pub(crate) fn tab(&self) -> SideTab {
        self.tab
    }

    /// Whether the pointer is parked on the panel's scrollbar. Fullscreen
    /// reads this to keep its chrome awake while the reader holds it.
    pub(crate) fn scrollbar_active(&self, cx: &App) -> bool {
        match self.tab {
            SideTab::Lyrics => self.verse_bar.read(cx).hovered(),
            SideTab::Queue => self.scrollbar.read(cx).hovered(),
        }
    }

    pub(crate) fn show(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.forget_verse();
            self.anchor_verse();
        }
        self.anchor = true;
        cx.notify();
    }

    pub(crate) fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.track_menu.reset(cx);
        self.context_menu = None;
        cx.notify();
    }

    /// How far a verse has sunk under the pointer, or risen back after being let
    /// go of.
    fn sink_progress(&self, window: &mut Window) -> f32 {
        let span = Motion::Quick.span().as_secs_f32().max(f32::EPSILON);
        let progress = (self.sank.elapsed().as_secs_f32() / span).clamp(0., 1.);
        if progress < 1. {
            window.request_animation_frame();
        }
        ease_in_out(progress)
    }

    fn touch(&self, spot: Warm, sharpen: f32, sink: f32) -> Touch {
        let waking = self.hovered == Some(spot);
        let settling = self.fading == Some(spot);
        Touch {
            warmth: match (waking, settling) {
                (true, _) => sharpen,
                (_, true) => 1. - sharpen,
                _ => 0.,
            },
            depth: match (self.held == Some(spot), self.rising == Some(spot)) {
                (true, _) => sink,
                (_, true) => 1. - sink,
                _ => 0.,
            },
            waking,
            settling,
        }
    }

    /// Whether the pointer has anything to do with a spot, over it, warming it, holding it or letting
    /// it go. Such a row keeps its listeners, so it hears the pointer leave.
    fn handled(&self, spot: Warm) -> bool {
        [self.over, self.hovered, self.fading, self.held, self.rising].contains(&Some(spot))
    }

    fn press_verse(&mut self, spot: Warm, down: bool, cx: &mut Context<Self>) {
        match down {
            true => {
                if self.held == Some(spot) {
                    return;
                }
                self.held = Some(spot);
                self.rising = None;
                self.sinking = None;
                self.sank = std::time::Instant::now();
            }
            false => {
                if self.held != Some(spot) {
                    return;
                }
                self.held = None;
                self.rising = Some(spot);
                self.sank = std::time::Instant::now();
                self.sinking = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Motion::Quick.span()).await;
                    this.update(cx, |this, cx| {
                        if this.rising != Some(spot) {
                            return;
                        }
                        this.rising = None;
                        cx.notify();
                    })
                    .ok();
                }));
            }
        }
        cx.notify();
    }

    /// Lets go of whichever verse or break is held. Any left release in the window does it, so
    /// the sheet listens once for every row rather than each row listening for its own.
    fn release_verse(&mut self, cx: &mut Context<Self>) {
        if let Some(spot) = self.held {
            self.press_verse(spot, false, cx);
        }
    }

    fn sweep_karaoke(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.sweeping.is_some() {
            return;
        }
        let saved = match window.is_window_active() {
            true => None,
            false => self.settings.read(cx).saver().interval(),
        };
        let interval = KARAOKE_FRAME.max(saved.unwrap_or_default());
        let wait = interval.saturating_sub(self.swept_frame.elapsed());
        self.sweeping = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.swept_frame = std::time::Instant::now();
                this.sweeping = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn sharpen_progress(&self, window: &mut Window) -> f32 {
        let span = Motion::Quick.span().as_secs_f32().max(f32::EPSILON);
        let progress = (self.since.elapsed().as_secs_f32() / span).clamp(0., 1.);
        if progress < 1. {
            window.request_animation_frame();
        }
        ease_in_out(progress)
    }

    fn forget_verse(&mut self) {
        self.flying = false;
        self.pinning = None;
        self.previous_active_line = None;
        self.departing_line = None;
        self.placing = true;
        self.forget_measurements();
    }

    fn forget_measurements(&mut self) {
        self.lyrics_wraps.clear();
        self.lane_rooms.clear();
        self.drifts.clear();
        self.sheet_layout = None;
        self.settled.clear();
        self.was_settled.clear();
    }

    // the panel took the wheel
    fn flown(&mut self, goal: Pixels, from: Pixels) {
        self.flying = true;
        self.flew = goal <= from;
    }

    // only automatic scrolls
    fn lagged(
        &mut self,
        scroll: &ScrollHandle,
        presentation: Pixels,
        verse: Pixels,
        nudges: u64,
    ) -> Drag {
        let now = std::time::Instant::now();
        let beat = now.duration_since(self.slid).as_secs_f32().min(LAG_STALL);
        self.slid = now;

        // follow the seen position
        let offset = scroll.offset().y + presentation;
        let step = offset - self.seen;
        self.seen = offset;
        if nudges != self.nudges {
            self.flying = false;
        }

        Drag {
            step: match self.flying {
                true => step,
                false => px(0.),
            },
            beat,
            downward: self.flew,
            most: (verse * LAG).min(scroll.bounds().size.height * LAG_SHARE),
        }
    }

    /// Steps the physical spring that makes a row lag behind the sheet, fed the inverse scroll
    /// delta and keeping its velocity so a retarget does not restart it. Returns how far the row
    /// is drawn from its place while it still moves, and `None` once it rests, so the caller asks
    /// for one more frame per render however many rows are moving.
    fn dragged(&mut self, row: usize, along: f32, drag: Drag) -> Option<Pixels> {
        let held = self.drifts.get(&row).copied();
        if held.is_none() && drag.step == px(0.) {
            return None;
        }
        let mut state = held.unwrap_or_default();
        state.position = (px(state.position) - drag.step * (LAG_TRAIL * along))
            .clamp(-drag.most, drag.most)
            .as_f32();
        let spring = lag_spring(along);
        state = spring.step(state, 0., drag.beat);
        if spring.is_settled(state, 0., LAG_LEAST.as_f32()) {
            self.drifts.remove(&row);
            return None;
        }
        self.drifts.insert(row, state);
        Some(px(state.position))
    }

    fn set_hovered(&mut self, spot: Warm, over: bool, cx: &mut Context<Self>) {
        if !over {
            if self.over == Some(spot) {
                self.over = None;
            }
            if self.hovered == Some(spot) {
                self.hovered = None;
                self.fading = Some(spot);
                self.since = std::time::Instant::now();
                self.linger = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Motion::Quick.span()).await;
                    this.update(cx, |this, cx| {
                        if this.fading != Some(spot) {
                            return;
                        }
                        this.fading = None;
                        cx.notify();
                    })
                    .ok();
                }));
                cx.notify();
            }
            return;
        }

        self.over = Some(spot);
        if self.hovered == Some(spot) {
            return;
        }
        self.fading = None;
        self.linger = Some(cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                if this.over != Some(spot) {
                    return;
                }
                this.hovered = Some(spot);
                cx.notify();
            })
            .ok();
        }));
    }

    fn enqueue(&mut self, pin: &Pin, gap: Option<usize>, cx: &mut Context<Self>) {
        self.playback
            .update(cx, |playback, cx| playback.enqueue_pin(pin, gap, cx));
    }

    /// Applies a drop anywhere on the queue at the gap the marker shows, since the marker reaches
    /// past a row into the header and spacing around it. A track moved within the queue stays put
    /// when no gap is marked, and a track dragged in from elsewhere then goes to the end.
    fn land(&mut self, dragged: &DraggedPin, cx: &mut Context<Self>) {
        let gap = self.drop_gap.take();
        match (dragged.spot(QUEUE), gap) {
            (Some(held), Some(gap)) => self.queue.update(cx, |queue, cx| {
                if queue.revision() == held.revision {
                    queue.move_upcoming_to_gap(held.index, gap, cx);
                }
            }),
            (Some(_), None) => {}
            (None, gap) => self.enqueue(&dragged.pin, gap, cx),
        }
        cx.notify();
    }

    fn dismiss_menu(&mut self, cx: &mut Context<Self>) {
        self.track_menu.reset(cx);
        self.context_menu = None;
        cx.notify();
    }

    fn row(
        track: Track,
        index: usize,
        position: QueuePosition,
        queue_revision: u64,
        look: RowLook,
        (place, total): (usize, usize),
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let RowLook {
            playing,
            drop_line,
            queued,
            remote,
        } = look;
        let theme = *cx.theme();
        let queue_index = position.upcoming().filter(|_| !remote);
        let uid = Sonora::global(cx)
            .queue
            .read(cx)
            .uid(position)
            .map(str::to_owned);
        let play_uid = uid.clone();
        let press_uid = uid.clone();
        let current = position == QueuePosition::Current;
        let title = match position {
            QueuePosition::Past(_) => theme.muted_foreground,
            QueuePosition::Current => theme.primary,
            QueuePosition::Upcoming(_) => theme.foreground,
        };
        let pin = track.pin();
        let menu_track = track.clone();
        // A queue entry keeps its uid while it moves, so its nodes stay put for screen readers.
        let key = match (uid.as_deref(), position) {
            (Some(uid), QueuePosition::Past(_)) => format!("past-{uid}"),
            (Some(uid), QueuePosition::Current) => format!("now-{uid}"),
            (Some(uid), QueuePosition::Upcoming(_)) => format!("next-{uid}"),
            (None, _) => index.to_string(),
        };
        let added = track
            .added_by
            .as_deref()
            .filter(|added| queued && !added.name.is_empty() && added.name != added.id);
        let about = match added {
            Some(added) => SharedString::from(format!(
                "{}, {}",
                track.artists,
                t!("queue-added-by", name = added.name.as_str())
            )),
            None => SharedString::from(track.artists.clone()),
        };

        let card = Card::new(
            SharedString::from(format!("queue-track-{key}")),
            SharedString::from(track.name.clone()),
        )
        .aria_description(about)
        .cover(track.cover.clone())
        .bare_meta(
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(
                    crate::shared::cells::artist_links(
                        SharedString::from(format!("queue-track-artist-{key}")),
                        track.artist_refs.clone(),
                        track.artists.clone(),
                        theme.muted_foreground,
                    )
                    .text_size(theme.text(Text::Small))
                    .truncate(),
                )
                .when_some(added, |this, added| this.child(adder(added, theme))),
        )
        .tint(title)
        .when(track.explicit, Card::explicit)
        .when(!remote || current, |card| {
            card.play(
                playing,
                cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| match (remote, play_uid.as_deref()) {
                            (true, _) => playback.toggle_play(cx),
                            (false, Some(uid)) => playback.play_queue(uid, cx),
                            _ => {}
                        });
                }),
            )
        })
        .menu(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            this.track_menu.reset(cx);
            this.context_menu = Some(ContextMenuState {
                track: menu_track.clone(),
                revision: queue_revision,
                position: event.position,
            });
            cx.notify();
        }))
        .when(!remote && !current, |card| {
            card.press(cx.listener(move |this, _, _, cx| {
                if let Some(uid) = press_uid.as_deref() {
                    this.playback
                        .update(cx, |playback, cx| playback.play_queue(uid, cx));
                }
            }))
        })
        .when(queue_index.is_some(), |card| {
            card.action(
                Button::new(SharedString::from(format!("remove-queued-track-{key}")))
                    .ghost()
                    .small()
                    .mr_1()
                    .icon("icons/x.svg")
                    .tooltip("menu-remove-from-queue")
                    .tint(theme.muted_foreground)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.queue.update(cx, |queue, cx| {
                            if let Some(position) =
                                uid.as_deref().and_then(|uid| queue.position(uid))
                            {
                                queue.remove(position, cx);
                            }
                        });
                    })),
            )
        })
        .when_some(queue_index, |this, target| {
            this.on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DraggedPin>, _, cx| {
                    let Some(gap) = drop_gap(event.bounds, event.event.position, target) else {
                        return;
                    };
                    let gap = match event.drag(cx).spot(QUEUE) {
                        Some(held) => (gap != held.index && gap != held.index + 1).then_some(gap),
                        None => Some(gap),
                    };
                    if this.drop_gap != gap {
                        this.drop_gap = gap;
                        cx.notify();
                    }
                }),
            )
            .on_drop(cx.listener(|this, dragged: &DraggedPin, _, cx| this.land(dragged, cx)))
        })
        .when_some(pin, |this, pin| match queue_index {
            Some(index) => this.pin_from(pin, Spot::new(QUEUE, index).revision(queue_revision)),
            None => this.pin(pin),
        });

        div()
            .id(SharedString::from(format!("queue-track-container-{key}")))
            .role(Role::ListItem)
            .aria_position_in_set(place)
            .aria_size_of_set(total)
            .relative()
            .min_w_0()
            .child(card)
            .when_some(drop_line, |this, edge| this.child(drop_marker(edge, cx)))
    }

    fn menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let ContextMenuState {
            track, position, ..
        } = self.context_menu.clone()?;

        Some(
            Popup::new(position, self.track_menu.for_track(&track, cx))
                .on_close(cx.listener(|this, _, _, cx| this.dismiss_menu(cx))),
        )
    }

    /// The panel's title and, over this app's own queue, the controls that change it. They go
    /// while another device has playback, since its queue is shown read only.
    fn header(
        &self,
        sections: &Sections,
        remote: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();

        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap_2()
            .h(snapped(theme.metrics.header, window))
            .px_2()
            .when(self.titled, |this| {
                this.border_b_1().border_color(theme.border).child(eyebrow(
                    match self.tab {
                        SideTab::Queue => t!("queue-title"),
                        SideTab::Lyrics => t!("lyrics-title"),
                    },
                    cx,
                ))
            })
            .when(!self.titled, |this| {
                this.justify_end().pr(theme.metrics.control + px(8.))
            })
            .when(self.tab == SideTab::Queue && !remote, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        // Only where a station exists to keep the queue going.
                        .when(
                            Sonora::global(cx).session.read(cx).capabilities().radio,
                            |this| {
                                this.child(
                                    Button::new("toggle-radio")
                                        .ghost()
                                        .small()
                                        .icon("icons/radio.svg")
                                        .tooltip("queue-radio")
                                        .aria_toggled(Toggled::from(self.playback.read(cx).radio()))
                                        .tint(match self.playback.read(cx).radio() {
                                            true => theme.primary,
                                            false => theme.muted_foreground,
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.playback.update(cx, |playback, cx| {
                                                playback.toggle_radio(cx)
                                            });
                                        })),
                                )
                            },
                        )
                        .child(
                            Button::new("reset-queue")
                                .ghost()
                                .small()
                                .label(t!("queue-reset"))
                                .tint(theme.muted_foreground)
                                .disabled(!self.queue.read(cx).reordered())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.queue.update(cx, |queue, cx| queue.reset(cx));
                                })),
                        )
                        .child(
                            Button::new("clear-queue")
                                .ghost()
                                .small()
                                .label(t!("queue-clear"))
                                .tint(theme.muted_foreground)
                                .disabled(sections.upcoming == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.queue.update(cx, |queue, cx| queue.clear_upcoming(cx));
                                })),
                        ),
                )
            })
    }

    fn follow(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.tab != SideTab::Lyrics || self.pinned {
            return None;
        }

        Some(
            self.raised(
                ui::perched(
                    Button::new("resume-pin")
                        .secondary()
                        .icon("icons/undo-2.svg")
                        .tooltip("lyrics-follow")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.anchor_verse();
                            cx.notify();
                        })),
                    cx,
                ),
                cx,
            ),
        )
    }

    /// The trip back to the now-playing row, or to the top while nothing is playing, once the
    /// queue has drifted far enough from it to want one.
    fn recall(&self, sections: &Sections, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        if self.tab != SideTab::Queue {
            return None;
        }
        let goal = self.resting_offset(sections, window, cx)?;
        let tooltip = match sections.current {
            true => "queue-return-playing",
            false => "nav-return-top",
        };
        let perch = ui::return_to("queue-return-top", &self.scrollbar, goal, tooltip, cx)?;

        Some(self.raised(perch, cx))
    }

    /// Keeps floating controls above the workspace player bar and the strip over it, or above
    /// the fullscreen transport.
    fn raised(&self, perch: Div, cx: &App) -> Div {
        match self.titled {
            true => match self.bottom(cx) > Pixels::ZERO {
                true => perch.bottom(self.bottom(cx) + cx.theme().metrics.pad * 1.5),
                false => perch,
            },
            false => perch.bottom_16(),
        }
    }

    /// Only workspace asides sit behind the player bar; fullscreen keeps its own clearance.
    fn bottom(&self, cx: &App) -> Pixels {
        match self.titled {
            true => Chrome::bottom(cx),
            false => Pixels::ZERO,
        }
    }

    fn verses(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let playback = self.playback.read(cx);
        let (position, singing) = match playback.steered(cx) {
            Some(remote) => (remote.state.live_position(), remote.state.playing),
            None => (
                playback.live_position(),
                matches!(playback.state(), PlaybackState::Playing),
            ),
        };
        let lyrics = self.lyrics.read(cx);
        let state = lyrics.state().clone();
        let shown = lyrics.current().is_some();
        let mirrored = lyrics
            .current()
            .is_some_and(|hit| reads_right_to_left(&hit.lyrics));
        let lines = match (&state, lyrics.current().map(|hit| &hit.lyrics)) {
            (LyricsState::Ready, Some(music::Lyrics::Synced { lines })) => Some(lines.clone()),
            _ => None,
        };
        if let Some(hit) = lyrics.current() {
            let language = i18n::language();
            let fresh = self.credit.as_ref().is_some_and(|credit| {
                credit.source == hit.source
                    && credit.writers == hit.writers
                    && credit.language == language
            });
            if !fresh {
                self.credit = Some(Credit {
                    source: hit.source,
                    writers: hit.writers.clone(),
                    language,
                    lines: credit_lines(hit.source, &hit.writers),
                });
            }
        }
        let following = (self.verse_of.as_deref() != lyrics.following())
            .then(|| lyrics.following().map(str::to_owned));
        let take = lyrics.revision();
        let (karaoke_lyrics, romanization_scripts) = {
            let settings = self.settings.read(cx);
            (
                settings.karaoke_lyrics(),
                settings
                    .romanized_lyrics()
                    .then(|| settings.romanization_scripts()),
            )
        };
        let karaoke_effects = karaoke_lyrics && effects();
        let scale = match self.titled {
            true => self.settings.read(cx).panel_lyrics_scale(),
            false => {
                self.settings.read(cx).fullscreen_lyrics_scale()
                    * lyrics_frame_scale(window.viewport_size())
            }
        };
        let (verse, lane_size) = verse_sizes(self.titled, scale, &theme);
        let sung = Sung {
            karaoke: karaoke_effects,
            lane: lane_size,
            scripts: romanization_scripts,
            theme,
            karaoke_tint: theme.foreground,
            lift: 1.,
            from: gpui::point(0., 0.5),
            mirrored,
        };

        if let Some(following) = following {
            self.verse_of = following;
            self.verse_take = take;
            self.forget_verse();
            self.anchor_verse();
            let scroll = self.verse_bar.read(cx).scroll().clone();
            scroll.set_offset(gpui::point(scroll.offset().x, px(0.)));
            self.verse_bar
                .update(cx, |bar, _| bar.remember_offset(scroll.offset().y));
        } else if self.verse_take != take {
            self.verse_take = take;
            // Nothing was on screen before, so there is no change to play: the
            // sheet is simply put up.
            self.resolving = self.showed;
            self.forget_verse();
            self.anchor_verse();
        }
        self.showed = shown;

        let empty = |key: &'static str, cx: &mut Context<Self>| {
            let text = i18n::lookup(key, None);
            vacant(text.clone(), cx)
                .id(key)
                .role(Role::Label)
                .aria_value(text)
                .flex_1()
                .into_any_element()
        };

        // aim before reading
        if let Some(lines) = &lines {
            let live = active_lyrics_row(lines, position);
            let focus = match self.pinning {
                Some(row) if Some(row) != live => Some(row),
                _ => {
                    self.pinning = None;
                    live
                }
            };
            self.pin_verse(focus, window, cx);
        }

        let reach = verse * REACH;
        let wrap_size = active_verse_size(verse);
        let scroll = self.verse_bar.read(cx).scroll().clone();
        let (nudges, presentation) = {
            let bar = self.verse_bar.read(cx);
            (bar.nudges(), bar.presentation().y)
        };
        let animations = ui::motion::animates(cx);
        if !animations {
            self.drifts.clear();
        }
        let drag = match (lines.is_some(), animations) {
            (true, true) => self.lagged(&scroll, presentation, verse, nudges),
            _ => Drag::default(),
        };
        let inset = window.rem_size() * LYRICS_HORIZONTAL_INSET_REM;
        let wrap_width = (scroll.bounds().size.width - inset)
            .min(reach - inset)
            .max(px(0.));
        if self.lyrics_wrap_width != Some(wrap_width) || self.lyrics_wrap_size != Some(wrap_size) {
            self.lyrics_wrap_width = Some(wrap_width);
            self.lyrics_wrap_size = Some(wrap_size);
            self.forget_measurements();
            window.request_animation_frame();
        }

        let layout = SheetLayout {
            viewport: window.viewport_size(),
            width: scroll.bounds().size.width,
            verse,
            lane: lane_size,
            scripts: romanization_scripts,
            rem: window.rem_size(),
            scale: window.scale_factor(),
            font: font_key(self.settings.read(cx).font()),
        };
        let measured = self.laid.replace(false) && self.sheet_layout == Some(layout);
        self.sheet_layout = Some(layout);
        std::mem::swap(&mut self.settled, &mut self.was_settled);
        self.settled.clear();

        let mut drifting = false;
        let mut body: Vec<gpui::AnyElement> = match (&lines, &state) {
            (Some(lines), _) => {
                let active_line = sung_line(lines, position);
                if singing
                    && karaoke_effects
                    && lines.iter().enumerate().any(|(index, line)| {
                        line.worded()
                            && primary_karaoke_visible(line, Some(index) == active_line, position)
                    })
                {
                    self.sweep_karaoke(window, cx);
                }
                if self.previous_active_line != active_line {
                    if self.previous_active_line.is_some() {
                        self.departing_line = self.previous_active_line;
                        self.departure = self.departure.wrapping_add(1);
                        self.departed = std::time::Instant::now();
                    }
                    if active_line.is_some() {
                        self.arrival = self.arrival.wrapping_add(1);
                        self.arrived = std::time::Instant::now();
                    }
                    self.previous_active_line = active_line;
                }
                if self.departing_line.is_some() && self.departed.elapsed() >= Motion::Base.span() {
                    self.departing_line = None;
                }
                let instrumental_line = active_instrumental(lines, position);
                let hazing = effects() && self.settings.read(cx).blur_lyrics() && self.pinned;
                let blur = verse * BLUR;
                let sharpen = self.sharpen_progress(window);
                // with motion turned down a press is simply on or off
                let sink = match animations {
                    true => self.sink_progress(window),
                    false => 1.,
                };
                let view = scroll.bounds();
                if hazing && scroll.bounds_for_item(0).is_none() {
                    window.request_animation_frame();
                }
                let mut rendered = Vec::with_capacity(lyric_row_count(lines) + 1);

                for (index, line) in lines.iter().enumerate() {
                    let seek = line.start;
                    let gap = instrumental_gap_before(lines, index);
                    let instrumental_start = line.start.saturating_sub(gap);
                    let has_instrumental = gap >= INSTRUMENTAL_BREAK;
                    let instrumental_progress = if has_instrumental {
                        progress_between(instrumental_start, line.start, position)
                    } else {
                        0.
                    };
                    let instrumental_has_passed = position >= line.start;

                    let verse_touch = self.touch(Warm::Verse(index), sharpen, sink);
                    let notes_touch = self.touch(Warm::Break(index), sharpen, sink);
                    let warmth = verse_touch.warmth;
                    let depth = verse_touch.depth;
                    // whatever the pointer rests on comes back into focus
                    let clearing = |touch: Touch, depth: f32| match (touch.waking, touch.settling) {
                        (true, _) => depth * (1. - sharpen),
                        (false, true) => depth * sharpen,
                        (false, false) => depth,
                    };
                    let haze = |depth: f32| clearing(verse_touch, depth);
                    if has_instrumental {
                        let notes_row = rendered.len();
                        self.settled.push(false);
                        if singing && instrumental_line == Some(index) {
                            window.request_animation_frame();
                        }
                        let notes_along = viewport_along(&scroll, notes_row, view, drag.downward);
                        let notes_drift = self.dragged(notes_row, notes_along, drag);
                        drifting |= notes_drift.is_some();
                        let notes_translation = presentation + notes_drift.unwrap_or_default();
                        let softness = match hazing && instrumental_line != Some(index) {
                            true => clearing(
                                notes_touch,
                                viewport_haze(&scroll, notes_row, view, blur, notes_translation),
                            ),
                            false => 0.,
                        };
                        let notes = instrumental_row(
                            instrumental_progress,
                            instrumental_has_passed,
                            verse,
                            &theme,
                        )
                        .id(("instrumental", index))
                        .role(Role::Button)
                        .aria_label(t!("lyrics-instrumental-break"))
                        .when(instrumental_line == Some(index), |this| {
                            this.aria_selected(true)
                        })
                        .w_full()
                        .max_w(reach)
                        .px_2()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .when(notes_touch.warmth > 0., |this| {
                            this.bg(theme.table_hover.opacity(notes_touch.warmth))
                        })
                        .when(notes_touch.depth > 0., |this| {
                            this.layer_scale(1. - (1. - PRESSED) * notes_touch.depth)
                        })
                        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                            this.set_hovered(Warm::Break(index), *over, cx)
                        }))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.press_verse(Warm::Break(index), true, cx)
                            }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.seek_verse(notes_row, instrumental_start, cx);
                        }))
                        .when(softness > 0., |this| this.opacity(1. - VEIL * softness))
                        .map(|this| match stepped(blur * softness) {
                            soft if soft > HAZE_LEAST => this.blur(soft),
                            _ => this,
                        });
                        rendered.push(adrift(notes, notes_translation, false, window));
                    }

                    let row = rendered.len();
                    let along = viewport_along(&scroll, row, view, drag.downward);
                    let drift = self.dragged(row, along, drag);
                    drifting |= drift.is_some();
                    let translation = presentation + drift.unwrap_or_default();
                    let active = Some(index) == active_line;
                    let departing = Some(index) == self.departing_line;
                    let karaoke = karaoke_effects
                        && line.worded()
                        && primary_karaoke_visible(line, active, position);
                    let primary_karaoke = karaoke && line.words.is_some();
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        self.lyrics_wraps.entry(index)
                    {
                        let parts = lyrics_parts(&line.text, line.words.as_deref());
                        if let Some(wrapped) =
                            lyrics_wrap_rows(&parts, wrap_size, wrap_width, window)
                        {
                            slot.insert(wrapped);
                        }
                    }
                    // A settled row far off screen is only a spacer of the height it had last frame,
                    // which is the height it would have now.
                    let settled =
                        !active && !departing && !karaoke && !self.handled(Warm::Verse(index));
                    let spared = (measured && settled && self.was_settled.get(row) == Some(&true))
                        .then(|| viewport_place(&scroll, row, view, translation))
                        .flatten()
                        .filter(|place| {
                            place.top + place.height < -view.size.height
                                || place.top > view.size.height * 2.
                        });
                    self.settled.push(settled);
                    if let Some(place) = spared {
                        rendered.push(
                            div()
                                .flex_none()
                                .w_full()
                                .h(place.height)
                                .into_any_element(),
                        );
                        continue;
                    }
                    let wrapped = self.lyrics_wraps.get(&index);
                    let line_has_ended = active_line.is_some_and(|active| index < active)
                        || line_has_passed(line, position);
                    let worded = karaoke_effects && line.worded() && line.words.is_some();
                    let shade = |singing: bool| match (singing, line_has_ended) {
                        (true, _) if worded => theme.muted_foreground,
                        (true, _) => theme.foreground,
                        (false, true) => theme.muted_foreground.opacity(PAST),
                        (false, false) => theme.muted_foreground.opacity(AHEAD),
                    };
                    let tint = shade(Some(index) == active_line);

                    let dimming = (animations && departing).then_some(self.departure);
                    let growing =
                        animations && active && self.arrived.elapsed() < Motion::Base.span();
                    let shrinking = dimming.is_some();
                    let active_size = active_verse_size(verse);
                    let small = verse / active_size;
                    let big = active_size / verse;
                    // both ways land on 1
                    let lift = match (growing, shrinking) {
                        (true, _) => small + (1. - small) * ramp(self.arrived, window),
                        (_, true) => big - (big - 1.) * ramp(self.departed, window),
                        _ => 1.,
                    };
                    let paint = match (growing, shrinking) {
                        (true, _) => mix(shade(false), tint, ramp(self.arrived, window)),
                        (_, true) => mix(shade(true), tint, ramp(self.departed, window)),
                        _ => tint,
                    };
                    let sung = Sung {
                        karaoke_tint: mix(
                            theme.foreground,
                            tint,
                            primary_karaoke_fade(line, active, position),
                        ),
                        lift,
                        from: match hugs_end(line.voice, sung.mirrored) {
                            true => gpui::point(1., 0.5),
                            false => gpui::point(0., 0.5),
                        },
                        ..sung
                    };

                    let primary = match (primary_karaoke, line.words.as_ref(), wrapped) {
                        (true, Some(words), Some(plan)) => {
                            karaoke_lane(plan, line.start, words, position, verse, line.voice, sung)
                                .into_any_element()
                        }
                        (_, _, Some(plan)) => {
                            fixed_lyrics_lane(&plan.text, line.voice, sung).into_any_element()
                        }
                        _ => div()
                            .child(SharedString::from(line.text.clone()))
                            .into_any_element(),
                    };
                    let fade = match (line.secondary.is_empty(), active, departing) {
                        (true, _, _) => None,
                        (_, true, _) => Some(("lane-in", self.arrival, growing)),
                        (_, _, true) => Some(("lane-out", self.departure, animations)),
                        _ => None,
                    };
                    let room = fade.map(|_| match self.lane_rooms.get(&index) {
                        Some(room) => *room,
                        None => {
                            let room = lanes_room(
                                &line.secondary,
                                romanization_scripts,
                                lane_size,
                                active_verse_size(verse) * ui::LEADING,
                                wrap_width,
                                window,
                            );
                            self.lane_rooms.insert(index, room);
                            room
                        }
                    });
                    // gpui wraps a lane by assuming each glyph sits right of the one before
                    // it, which right-to-left text breaks, so such a lane is measured here
                    let lane_plans = match fade.is_some() {
                        true => line
                            .secondary
                            .iter()
                            .map(|lane| {
                                has_right_to_left(&lane.text)
                                    .then(|| {
                                        let parts = lyrics_parts(&lane.text, lane.words.as_deref());
                                        lyrics_wrap_rows(&parts, lane_size, wrap_width, window)
                                    })
                                    .flatten()
                            })
                            .collect::<Vec<_>>(),
                        false => Vec::new(),
                    };
                    let lanes = fade.zip(room).map(|((tag, take, animated), room)| {
                        let arriving = tag == "lane-in";
                        let group = div().flex().flex_col().gap_1().children(
                            line.secondary.iter().zip(&lane_plans).map(|(lane, plan)| {
                                let sung_by_end = line
                                    .sung_end()
                                    .is_some_and(|end| secondary_lane_started(lane, end));
                                secondary_lyrics_lane(
                                    lane,
                                    plan.as_ref(),
                                    line_has_ended,
                                    position,
                                    dimming.filter(|_| sung_by_end),
                                    line.voice,
                                    sung,
                                )
                            }),
                        );
                        match animated {
                            true => group
                                .overflow_hidden()
                                .with_animation(
                                    (tag, take as usize),
                                    Animation::new(Motion::Base.span()).with_easing(ease_in_out),
                                    move |this, t| {
                                        let shown = match arriving {
                                            true => t,
                                            false => 1. - t,
                                        };
                                        this.opacity(shown).max_h(room * shown)
                                    },
                                )
                                .into_any_element(),
                            false => group.into_any_element(),
                        }
                    });
                    let content = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(hugs_end(line.voice, sung.mirrored), |this| {
                            this.items_end().text_right()
                        })
                        .child(primary)
                        .when_some(
                            selected_romanization(&line.romanized, romanization_scripts),
                            |this, text| this.child(romanized_lyrics_lane(text, lane_size, &theme)),
                        )
                        .children(lanes)
                        .when(depth > 0., |this| {
                            this.layer_scale(1. - (1. - PRESSED) * depth)
                        });

                    let softness = match hazing && Some(index) != active_line {
                        true => haze(viewport_haze(&scroll, row, view, blur, translation)),
                        false => 0.,
                    };
                    let row_blur = match background_line_singing(line, active, position) {
                        true => BACKGROUND_SINGING_BLUR,
                        false => blur,
                    };
                    let traded = index
                        .checked_sub(1)
                        .is_some_and(|previous| lines[previous].voice != line.voice);
                    // the romanization and the backing vocals under a line are read after it
                    let lanes_spoken = selected_romanization(&line.romanized, romanization_scripts)
                        .into_iter()
                        .chain(line.secondary.iter().map(|lane| lane.text.clone()))
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let verse_line = div()
                        .id(("verse", index))
                        .role(Role::Button)
                        .aria_label(SharedString::from(line.text.clone()))
                        .when(!lanes_spoken.is_empty(), |this| {
                            this.aria_description(SharedString::from(lanes_spoken))
                        })
                        .when(active, |this| this.aria_selected(true))
                        .aria_position_in_set(index + 1)
                        .aria_size_of_set(lines.len())
                        .w_full()
                        .max_w(reach)
                        .px_2()
                        .py_1()
                        .when(traded, |this| this.mt_2())
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .when(warmth > 0., |this| {
                            this.bg(theme.table_hover.opacity(warmth))
                        })
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.press_verse(Warm::Verse(index), true, cx)
                            }),
                        )
                        .text_size(verse)
                        .line_height(active_verse_size(verse) * ui::LEADING)
                        .text_color(tint)
                        .font_weight(FontWeight::SEMIBOLD)
                        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                            this.set_hovered(Warm::Verse(index), *over, cx)
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.seek_verse(row, seek, cx);
                        }))
                        .child(content);

                    let verse_line = verse_line
                        .when(softness > 0., |this| this.opacity(1. - VEIL * softness))
                        .map(|this| match stepped(row_blur * softness) {
                            soft if soft > HAZE_LEAST => this.blur(soft),
                            _ => this,
                        });
                    let verse_line = match (growing, shrinking, active) {
                        (_, true, false) => verse_line.text_color(paint),
                        (true, _, _) | (_, _, true) => {
                            verse_line.text_size(active_size).text_color(paint)
                        }
                        _ => verse_line,
                    };
                    rendered.push(adrift(verse_line, translation, traded, window));
                }

                rendered
            }
            (None, LyricsState::Ready) => {
                match self.lyrics.read(cx).current().map(|hit| &hit.lyrics) {
                    Some(music::Lyrics::Plain { text, romanized }) => {
                        let romanized = selected_romanization(romanized, romanization_scripts);
                        let romanized_lines = romanized
                            .as_deref()
                            .map_or_else(Vec::new, |text| text.lines().collect::<Vec<_>>());
                        let plain_size = (lane_size * PLAIN_LYRICS_GROWTH).round();
                        // gpui wraps text by assuming each glyph sits right of the one before it,
                        // which right-to-left text breaks, so such lyrics are wrapped here
                        let rows = has_right_to_left(text).then(|| {
                            text.lines()
                                .map(|line| {
                                    lyrics_wrap_rows(
                                        &lyrics_parts(line, None),
                                        plain_size,
                                        wrap_width,
                                        window,
                                    )
                                    .filter(|plan| !plan.text.is_empty())
                                    .map_or_else(
                                        || vec![SharedString::from(line.to_owned())],
                                        |plan| plan.text,
                                    )
                                })
                                .collect::<Vec<_>>()
                        });
                        vec![
                            div()
                                .id("plain-lyrics")
                                .role(Role::Paragraph)
                                .aria_label(SharedString::from(text.clone()))
                                .w_full()
                                .max_w(reach)
                                .px_2()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .text_size(plain_size)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground)
                                .when(sung.mirrored, |this| this.text_right())
                                .map(|this| match (&rows, romanized.is_some()) {
                                    (None, false) => this.child(SharedString::from(text.clone())),
                                    _ => this.children(text.lines().enumerate().map(
                                        |(index, line)| {
                                            let spoken = match &rows {
                                                Some(rows) => fixed_lyrics_lane(
                                                    &rows[index],
                                                    Voice::Lead,
                                                    sung,
                                                ),
                                                None => {
                                                    div().child(SharedString::from(line.to_owned()))
                                                }
                                            };
                                            div().flex().flex_col().child(spoken).when_some(
                                                romanized_lines
                                                    .get(index)
                                                    .filter(|line| !line.is_empty()),
                                                |this, line| {
                                                    this.child(
                                                        romanized_lyrics_lane(
                                                            (*line).to_owned(),
                                                            lane_size,
                                                            &theme,
                                                        )
                                                        .font_weight(FontWeight::NORMAL),
                                                    )
                                                },
                                            )
                                        },
                                    )),
                                })
                                .into_any_element(),
                        ]
                    }
                    _ => vec![wordless("lyrics-missing", "icons/mic-off.svg")],
                }
            }
            (None, LyricsState::Idle) => vec![empty("lyrics-idle", cx)],
            (None, LyricsState::Loading) => vec![empty("lyrics-loading", cx)],
            (None, LyricsState::Instrumental) => {
                vec![wordless("lyrics-instrumental", "icons/guitar.svg")]
            }
            (None, LyricsState::Missing) => {
                vec![wordless("lyrics-missing", "icons/mic-off.svg")]
            }
            (None, LyricsState::Failed(reason)) => {
                match Network::lost(cx) || music::trouble::offline(reason) {
                    true => vec![wordless("trouble-offline", "icons/wifi-off.svg")],
                    false => vec![empty("lyrics-failed", cx)],
                }
            }
        };

        if state == LyricsState::Ready
            && shown
            && let Some((from, by)) = self.credit.as_ref().map(|credit| credit.lines.clone())
        {
            let credit = body.len();
            let along = viewport_along(&scroll, credit, scroll.bounds(), drag.downward);
            let drift = self.dragged(credit, along, drag);
            drifting |= drift.is_some();
            let translation = match lines.is_some() {
                true => presentation + drift.unwrap_or_default(),
                false => px(0.),
            };
            let note = div()
                .w_full()
                .max_w(reach)
                .px_2()
                .pt_2()
                .flex()
                .flex_col()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(ui::spoken("lyrics-credit-source", from))
                .children(by.map(|by| ui::spoken("lyrics-credit-writers", by)));
            body.push(adrift(note, translation, false, window));
        }
        if drifting {
            window.request_animation_frame();
        }

        let (over, under) = match &lines {
            Some(lines) => self.verse_slack(lyric_row_count(lines), window, cx),
            None => (px(REST), px(REST)),
        };

        let sheet = Scroller::new("lyrics", &self.verse_bar)
            .bottom_inset(self.bottom(cx))
            .when(lines.is_some(), Scroller::manual_presentation)
            .when(lines.is_some(), |this| {
                this.capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    if event.button == gpui::MouseButton::Left {
                        this.release_verse(cx);
                    }
                }))
                .on_mouse_up_out(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.release_verse(cx)),
                )
            })
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .flex_1()
            .min_h_0()
            .px_1()
            .pt(over)
            .pb(under)
            .when(effects(), |this| {
                let fade = verse * VERSE_FADE;
                this.fade_edges(fade, fade)
            })
            .children(body);

        // A sheet only ever replaces another once, when every source has
        // answered, and that is the one change worth showing.
        match self.resolving && ui::motion::animates(cx) {
            true => sheet
                .with_animation(
                    ("verse-sheet", self.verse_take as usize),
                    Animation::new(Motion::Base.span()).with_easing(ui::ease_in_out_cubic),
                    move |this, t| {
                        this.blur(verse * RESOLVE_BLUR * (1. - t))
                            .opacity(1. - RESOLVE_FADE * (1. - t))
                    },
                )
                .into_any_element(),
            false => sheet.into_any_element(),
        }
    }

    /// An empty element that marks the sheet as laid out once the frame it renders in has been.
    fn witness(&self) -> impl IntoElement {
        let laid = self.laid.clone();
        canvas(move |_, _, _| laid.set(true), |_, _, _, _| {})
            .absolute()
            .size_0()
    }

    fn verse_slack(&self, count: usize, window: &Window, cx: &App) -> (Pixels, Pixels) {
        let scroll = self.verse_bar.read(cx).scroll().clone();
        let view = scroll.bounds().size.height;
        if view <= px(0.) {
            window.request_animation_frame();
            return (px(REST), px(REST));
        }
        let tail = count
            .checked_sub(1)
            .and_then(|last| scroll.bounds_for_item(last))
            .map_or(px(0.), |item| item.size.height);

        (
            snapped((view * PIN).max(px(REST)), window),
            snapped((view * (1. - PIN) - tail).max(px(REST)), window),
        )
    }

    fn anchor_verse(&mut self) {
        self.pinned = true;
        self.aiming = false;
        self.rested = None;
        self.followed = None;
        self.nudged = None;
    }

    /// Seeks to a verse and holds the panel on the row it was asked for. The
    /// clock takes a moment to report the new position, and until it does the
    /// verse being sung is still the old one, which is where the panel would
    /// otherwise fly off to.
    fn seek_verse(&mut self, row: usize, position: std::time::Duration, cx: &mut Context<Self>) {
        self.pinning = Some(row);
        self.seek_lyrics(position, cx);
    }

    fn seek_lyrics(&mut self, position: std::time::Duration, cx: &mut Context<Self>) {
        self.anchor_verse();
        self.playback
            .update(cx, |playback, cx| playback.seek(position, cx));
        cx.notify();
    }

    fn pin_verse(&mut self, sung: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let scroll = self.verse_bar.read(cx).scroll().clone();
        let resting = scroll.offset().y;
        let nudges = self.verse_bar.read(cx).nudges();
        if self.nudges != nudges {
            self.nudges = nudges;
            self.pinned = false;
            self.flying = false;
            self.drifts.clear();
            self.nudged = Some(std::time::Instant::now());
        }
        if !self.pinned {
            self.followed = sung;
            // Keep the reader in charge for as long as they keep moving: the timer counts from the
            // last scroll, not from the first one.
            if self.rested != Some(resting) {
                self.rested = Some(resting);
                self.nudged = Some(std::time::Instant::now());
            }
            if self.nudged.is_some_and(|at| at.elapsed() >= SETTLE) {
                self.anchor_verse();
            } else {
                return;
            }
        }
        if sung.is_none() {
            return;
        }
        // The rows a verse sits among change on the very frame it starts being sung, and their
        // bounds only settle once that frame has been laid out. Aim on the next one.
        if self.followed != sung {
            self.followed = sung;
            self.aiming = true;
            window.request_animation_frame();
            return;
        }
        if !self.aiming {
            return;
        }
        let Some(item) = sung.and_then(|index| scroll.bounds_for_item(index)) else {
            return;
        };
        self.aiming = false;
        let view = scroll.bounds();
        // Preserve the fractional target. Spring scrolls are presented by the compositor, so the
        // text never has to walk the raster grid while the layer is settling.
        let goal = anchored_lyrics_offset(
            view.origin.y,
            item.origin.y,
            view.size.height,
            scroll.max_offset().y,
        );
        self.flown(goal, scroll.offset().y);
        match std::mem::take(&mut self.placing) || cx.reduce_motion() {
            true => self.verse_bar.update(cx, |bar, _| bar.place(goal)),
            false => self.verse_bar.update(cx, |bar, _| bar.aim(goal, window)),
        }
    }

    fn pin(&mut self, sections: &Sections, window: &Window, cx: &Context<Self>) {
        let Some(index) = sections.current_index() else {
            self.anchor = false;
            return;
        };

        let viewport = self.scroll.0.borrow().base_handle.bounds().size.height;
        if viewport <= px(0.) {
            window.request_animation_frame();
            return;
        }

        let above = Self::rows_above(viewport, window, cx);
        self.scroll
            .scroll_to_item_strict_with_offset(index, ScrollStrategy::Top, above);
        self.anchor = false;
    }

    /// How many rows `pin` keeps above the now-playing one, so it sits a quarter of the way down.
    fn rows_above(viewport: Pixels, window: &Window, cx: &App) -> usize {
        let row = snapped(cx.theme().metrics.list_row, window);
        (viewport * PINNED_SHARE / row).round() as usize
    }

    /// Where the queue rests once `pin` has placed the now-playing row, in scrolled pixels,
    /// clamped the way gpui clamps the deferred scroll. Zero while nothing is playing, so a
    /// recall falls back to the top, and None before the list has been laid out.
    fn resting_offset(&self, sections: &Sections, window: &Window, cx: &App) -> Option<Pixels> {
        let handle = self.scroll.0.borrow().base_handle.clone();
        let viewport = handle.bounds().size.height;
        if viewport <= px(0.) {
            return None;
        }
        let Some(index) = sections.current_index() else {
            return Some(Pixels::ZERO);
        };
        let row = snapped(cx.theme().metrics.list_row, window);
        let above = Self::rows_above(viewport, window, cx);
        let goal = row * index.saturating_sub(above) as f32;
        Some(goal.clamp(Pixels::ZERO, handle.max_offset().y))
    }

    /// The name and page of the collection the current track was queued from. `None` for a track
    /// added on its own or an origin without a name, so the header shows no source.
    fn playing_from(
        &self,
        origin: &state::Origin,
        cx: &App,
    ) -> Option<(SharedString, Destination)> {
        let id = SharedString::from(origin.id.clone());
        let place = match origin.whence {
            Whence::Album => Destination::Album(id),
            Whence::Playlist => Destination::Playlist(id),
            Whence::Artist => Destination::Artist(id),
            Whence::Radio => Destination::Song(id),
            Whence::Saved => Destination::Library(LibraryTab::Songs),
            Whence::Local => Destination::Local(LibraryTab::Songs),
        };
        let name = match origin.whence {
            Whence::Saved => match Sonora::global(cx).library.read(cx).shape(Shelf::Streaming) {
                Shape::Saved => t!("library-liked-songs"),
                Shape::Catalog(_) => t!("nav-songs"),
            },
            Whence::Local => t!("nav-local"),
            Whence::Radio => t!("queue-from-radio", name = origin.name.as_deref()?),
            Whence::Album | Whence::Playlist | Whence::Artist => origin.name.clone()?,
        };

        Some((name, place))
    }

    fn rows(
        &self,
        listing: QueueView,
        sections: Sections,
        cx: &mut Context<Self>,
    ) -> gpui::UniformList {
        let remote = !listing.editable();
        let from = listing
            .origin(cx)
            .and_then(|origin| self.playing_from(origin, cx));
        let drop_gap = self.drop_gap;
        let upcoming = sections.upcoming;
        let audible = listing.playing;
        // A uniform list only draws rows inside its padding, so the space under a see-through
        // player bar is made of blank rows, which keeps the songs scrolling under it drawn.
        let under = (self.bottom(cx) / cx.theme().metrics.list_row).ceil() as usize;

        uniform_list(
            "queue-rows",
            sections.len() + under + TAIL_ROWS,
            cx.processor(move |_, range: Range<usize>, window, cx| {
                let reach = (range.len() as f32 * PREFETCH_REACH).ceil() as usize;
                let near = (range.start.saturating_sub(reach)..range.start)
                    .chain(range.end..(range.end + reach).min(sections.len()));
                let revision = listing.revision(cx);
                let row = |index, cx: &mut Context<Self>| match sections.slot(index) {
                    None => div().into_any_element(),
                    Some(Slot::Header(key)) => {
                        let label = section_label(key, cx);
                        match (key, from.clone()) {
                            ("queue-now-playing", Some((name, place))) => label
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .gap_1()
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(cx.theme().text(Text::Small))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(BULLET),
                                )
                                .child(faint(cx).child(ui::spoken("queue-from", t!("queue-from"))))
                                .child(source_link(name, place, cx))
                                .into_any_element(),
                            _ => label.into_any_element(),
                        }
                    }
                    Some(Slot::Track(position)) => {
                        let Some(track) = listing.track(position, cx) else {
                            return div().into_any_element();
                        };
                        let drop_line = match (position.upcoming(), drop_gap) {
                            (Some(queued), Some(gap)) if gap == queued => Some(Edge::Above),
                            (Some(queued), Some(gap))
                                if gap == upcoming && queued + 1 == upcoming =>
                            {
                                Some(Edge::Below)
                            }
                            _ => None,
                        };
                        let look = RowLook {
                            playing: audible && position == QueuePosition::Current,
                            drop_line,
                            queued: sections.kind(position) == Some(state::QueueKind::Queued),
                            remote,
                        };
                        let place = track_place(&sections, position);
                        Self::row(track, index, position, revision, look, place, cx)
                            .into_any_element()
                    }
                };
                let rows = range.map(|index| row(index, cx)).collect();
                for index in near {
                    prefetch(row(index, cx), window, cx);
                }
                rows
            }),
        )
    }
}

impl Render for Aside {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scrollbar.read(cx).sync();
        let listing = self.playback.read(cx).shown_queue(cx);
        let sections = listing.sections.clone();
        let remote = !listing.editable();
        let empty = sections.is_empty();
        if !cx.has_active_drag() {
            self.drop_gap = None;
        }

        if self.past_len != sections.past {
            self.past_len = sections.past;
            self.anchor = true;
        }
        let queue_track = listing
            .track(QueuePosition::Current, cx)
            .and_then(|track| track.id.clone());
        if self.queue_track != queue_track {
            self.queue_track = queue_track;
            self.anchor = true;
        }
        if self.anchor && self.tab == SideTab::Queue {
            self.pin(&sections, window, cx);
        }

        div()
            .id("aside")
            .role(Role::Complementary)
            .aria_label(match self.tab {
                SideTab::Queue => t!("queue-title"),
                SideTab::Lyrics => t!("lyrics-title"),
            })
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .on_drag_move(cx.listener(|this, _: &DragMoveEvent<DraggedPin>, _, cx| {
                if this.drop_gap.take().is_some() {
                    cx.notify();
                }
            }))
            .when(self.titled || self.tab == SideTab::Queue, |this| {
                this.child(self.header(&sections, remote, window, cx))
            })
            .child(
                div()
                    .id("queue-drop")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .when(self.tab == SideTab::Queue && !remote, |this| {
                        this.on_drop(
                            cx.listener(|this, dragged: &DraggedPin, _, cx| this.land(dragged, cx)),
                        )
                    })
                    .when(self.tab == SideTab::Lyrics, |this| {
                        this.child(self.verses(window, cx)).child(self.witness())
                    })
                    .when(self.tab == SideTab::Queue && empty, |this| {
                        this.child(
                            vacant(t!("queue-empty"), cx)
                                .id("queue-empty")
                                .role(Role::Label)
                                .aria_value(t!("queue-empty"))
                                .flex_1(),
                        )
                    })
                    .when(self.tab == SideTab::Queue && !empty, |this| {
                        this.child(
                            div().flex_1().min_h_0().child(
                                Scroller::listing("queue-rows", &self.scrollbar)
                                    .bottom_inset(self.bottom(cx))
                                    .when(effects(), |this| {
                                        this.fade_edges(px(FADE * 0.5), Pixels::ZERO)
                                    })
                                    .child(
                                        self.rows(listing, sections.clone(), cx)
                                            .px_2()
                                            .pt(px(FADE * 0.5))
                                            .track_scroll(&self.scroll)
                                            .size_full(),
                                    ),
                            ),
                        )
                    })
                    .children(self.follow(cx))
                    .children(self.recall(&sections, window, cx)),
            )
            .children(self.menu(cx))
    }
}

/// The part of a queue row that names who queued the song: a bullet, their round picture or
/// initials, and "Added by NAME".
fn adder(added: &Contributor, theme: ui::Theme) -> Div {
    let face = theme.text(Text::Small) * ADDER_FACE;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(BULLET)
        .child(match &added.avatar {
            Some(image) => Avatar::new(Some(image.clone()))
                .size(face)
                .into_any_element(),
            None => Initials::new(added.name.clone(), face).into_any_element(),
        })
        .child(t!("queue-added-by", name = added.name.as_str()))
}

/// Where the song at `position` stands among every song the queue lists, counted from one and
/// leaving out the section headings, and how many songs that is.
fn track_place(sections: &Sections, position: QueuePosition) -> (usize, usize) {
    let current = usize::from(sections.current);
    let before = match position {
        QueuePosition::Past(at) => at,
        QueuePosition::Current => sections.past,
        QueuePosition::Upcoming(at) => sections.past + current + at,
    };
    (before + 1, sections.past + current + sections.upcoming)
}

fn source_link(name: SharedString, to: Destination, cx: &App) -> impl IntoElement {
    let theme = *cx.theme();

    div()
        .id("queue-source")
        .role(Role::Link)
        .aria_label(name.clone())
        .min_w_0()
        .flex_shrink(1.)
        .truncate()
        .tails()
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .hover(|style| style.text_color(theme.foreground).underline())
        .link(to)
        .child(name)
}

fn fixed_lyrics_lane(rows: &[SharedString], voice: Voice, sung: Sung) -> Div {
    div()
        .flex()
        .flex_col()
        .children(rows.iter().map(move |row| {
            lifted(
                div()
                    .w_full()
                    .when(hugs_end(voice, sung.mirrored), |this| this.text_right())
                    .child(row.clone()),
                sung,
            )
        }))
}

/// One verse drawn the way the sheet draws it at rest, with its karaoke sweep at `position` and
/// its romanized lane, but without the sheet's pointer handling, drift or growth animations.
pub(crate) fn verse(
    line: &music::LyricsLine,
    look: VerseLook,
    position: std::time::Duration,
    theme: ui::Theme,
) -> Div {
    let karaoke = look.karaoke && effects();
    let ended = line_has_passed(line, position);
    let worded = karaoke && line.worded() && line.words.is_some();
    let tint = match (look.active, ended) {
        (true, _) if worded => theme.muted_foreground,
        (true, _) => theme.foreground,
        (false, true) => theme.muted_foreground.opacity(PAST),
        (false, false) => theme.muted_foreground.opacity(AHEAD),
    };
    let mirrored = right_to_left(&line.text);
    let sung = Sung {
        karaoke,
        lane: look.lane,
        scripts: look.scripts,
        theme,
        karaoke_tint: mix(
            theme.foreground,
            tint,
            primary_karaoke_fade(line, look.active, position),
        ),
        lift: 1.,
        from: match hugs_end(line.voice, mirrored) {
            true => gpui::point(1., 0.5),
            false => gpui::point(0., 0.5),
        },
        mirrored,
    };
    let primary = match (
        worded && primary_karaoke_visible(line, look.active, position),
        line.words.as_deref(),
    ) {
        (true, Some(words)) => karaoke_lane(
            &loose_plan(&line.text, words),
            line.start,
            words,
            position,
            look.size,
            line.voice,
            sung,
        ),
        _ => fixed_lyrics_lane(&[SharedString::from(line.text.clone())], line.voice, sung),
    };
    let active_size = active_verse_size(look.size);
    let haze = match effects() {
        true => look.haze,
        false => 0.,
    };

    div()
        .w_full()
        .px_2()
        .py_1()
        .text_size(match look.active {
            true => active_size,
            false => look.size,
        })
        .line_height(active_size * ui::LEADING)
        .text_color(tint)
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .when(hugs_end(line.voice, mirrored), |this| {
                    this.items_end().text_right()
                })
                .child(primary)
                .when_some(
                    selected_romanization(&line.romanized, look.scripts),
                    |this, text| this.child(romanized_lyrics_lane(text, look.lane, &theme)),
                ),
        )
        .when(haze > 0., |this| this.opacity(1. - VEIL * haze))
        .map(|this| match look.size * BLUR * haze {
            soft if soft > HAZE_LEAST => this.blur(soft),
            _ => this,
        })
}

/// The verse and lane sizes the sheet sets at `scale`, in the side panel or in Now playing,
/// leaving out how much a large window grows Now playing's.
pub(crate) fn verse_sizes(panel: bool, scale: f32, theme: &ui::Theme) -> (Pixels, Pixels) {
    let verse = match panel {
        true => theme.text(Text::Large),
        false => theme.text(Text::Title) + FULLSCREEN_VERSE_GROWTH,
    };
    (verse * scale, theme.text(Text::Body) * scale)
}

// a lane with no measured plan of its own, split on the spot
fn loose_plan(line: &str, words: &[music::LyricsWord]) -> Wrapped {
    let parts = karaoke_parts(line, words);
    let fragments = parts
        .iter()
        .map(|(text, _)| SharedString::from(text.clone()))
        .collect::<Vec<_>>();
    let spoken = parts.iter().map(|(_, word)| *word).collect::<Vec<_>>();
    Wrapped {
        rtl: right_to_left(line),
        spans: Vec::new(),
        evenly: evenly_filled(&fragments, &spoken),
        fragments,
        spoken,
        rows: Vec::new(),
        widths: Vec::new(),
        text: Vec::new(),
    }
}

fn karaoke_lane(
    plan: &Wrapped,
    line_start: std::time::Duration,
    words: &[music::LyricsWord],
    position: std::time::Duration,
    verse: Pixels,
    voice: Voice,
    sung: Sung,
) -> Div {
    let edge_fade = verse * REVEAL;
    let Wrapped {
        fragments,
        spoken,
        evenly,
        ..
    } = plan;
    let windows = (0..words.len())
        .map(|word| {
            let (start, end) = karaoke_window(line_start, words, word);
            (start, end, word + 1 >= words.len())
        })
        .collect::<Vec<_>>();
    let sweep = |word: usize| match (windows.get(word), evenly.get(word)) {
        (Some(&(start, end, _)), Some(true)) => progress_between(start, end, position),
        (Some(&(start, end, tail)), _) => swept(start, end, position, tail),
        (None, _) => 0.,
    };
    // a right-to-left line is read from its right edge, so its fill grows from there, and the
    // text inside it hangs from that edge to line up with the text beneath
    let rtl = plan.rtl;
    let overlay = |text: SharedString, reveal: Reveal, tint: gpui::Hsla| {
        div()
            .absolute()
            .map(|this| match rtl {
                true => this.right_0(),
                false => this.left_0(),
            })
            .top_0()
            .bottom_0()
            .map(|this| match reveal.width {
                Some(width) => this.w(width),
                None => this.w(relative(reveal.share)),
            })
            .overflow_hidden()
            .text_color(tint)
            .when(reveal.landing > 0., |this| {
                let fade = edge_fade * reveal.landing;
                match rtl {
                    true => this.fade_sides(fade, px(0.)),
                    false => this.fade_sides(px(0.), fade),
                }
            })
            .child(
                div()
                    .whitespace_nowrap()
                    .when(rtl, |this| this.absolute().top_0().right_0())
                    .child(text),
            )
    };
    let lit = |text: SharedString, reveal: Reveal| {
        div()
            .relative()
            .flex_none()
            .whitespace_nowrap()
            .child(text.clone())
            .when(reveal.shown, |this| {
                this.child(overlay(text, reveal, sung.karaoke_tint))
            })
    };

    match plan.rows.is_empty() {
        false => div()
            .flex()
            .flex_col()
            .text_left()
            .children((0..plan.rows.len()).map(|row| {
                let reveal = revealed(plan, row, &windows, position, edge_fade);
                lifted(
                    div()
                        .flex()
                        .when(hugs_end(voice, sung.mirrored), |this| this.justify_end())
                        .child(lit(plan.text[row].clone(), reveal)),
                    sung,
                )
            })),
        true => div()
            .flex()
            .flex_wrap()
            // right-to-left words flow from the right, where row-reverse starts, so for them
            // the far end of a row is its left
            .when(rtl, |this| this.flex_row_reverse())
            .text_left()
            .when(hugs_end(voice, sung.mirrored) != rtl, |this| {
                this.justify_end()
            })
            .children((0..fragments.len()).map(|index| {
                let share = sweep(spoken.get(index).copied().unwrap_or(index));
                let reveal = Reveal {
                    shown: share > 0.,
                    width: None,
                    share,
                    landing: match share < 1. {
                        true => ((1. - share) / LANDING).min(1.),
                        false => 0.,
                    },
                };
                lit(fragments[index].clone(), reveal)
            })),
    }
}

#[derive(Clone, Copy)]
struct Reveal {
    shown: bool,
    width: Option<Pixels>,
    share: f32,
    landing: f32,
}

fn revealed(
    plan: &Wrapped,
    row: usize,
    windows: &[(std::time::Duration, std::time::Duration, bool)],
    position: std::time::Duration,
    fade: Pixels,
) -> Reveal {
    let mut front = px(0.);
    let mut offset = px(0.);
    for index in plan.rows[row].clone() {
        let mine = plan.widths.get(index).copied().unwrap_or(px(0.));
        let word = plan.spoken.get(index).copied().unwrap_or(index);
        let Some(&(start, end, last)) = windows.get(word) else {
            offset += mine;
            continue;
        };

        // a wide character or a phrase timed as one word fills at an even pace;
        // the eased curve only reads as a flourish across Latin letters
        let even = plan.evenly.get(word).copied().unwrap_or(false);
        let share = match even {
            true => progress_between(start, end, position),
            false => swept(start, end, position, last),
        };
        if share > 0. {
            // a word covering several fragments hands each its own slice of the
            // sweep, and the edge follows whichever reaches furthest
            let (before, whole) = plan.spans.get(index).copied().unwrap_or((px(0.), mine));
            let part = match mine > px(0.) {
                true => ((whole * share - before) / mine).clamp(0., 1.),
                false => 0.,
            };
            let reach = offset + mine * part;
            if part > 0. && reach > front {
                front = reach;
            }
        }
        offset += mine;
    }

    // The edge keeps one soft trail the whole way across a row, no wider than
    // the text left to reveal. Letting it harden at every word would drag the
    // visible edge back each time, and a word can end mid-word: providers split
    // "nothing" into "no" and "thing".
    let landing = match fade > px(0.) {
        true => ((offset - front) / fade).min(1.),
        false => 0.,
    };

    Reveal {
        shown: front > px(0.),
        width: Some(front),
        share: 1.,
        landing,
    }
}

fn secondary_lyrics_lane(
    lane: &music::LyricsLane,
    plan: Option<&Wrapped>,
    line_passed: bool,
    position: std::time::Duration,
    dimming: Option<u64>,
    voice: Voice,
    sung: Sung,
) -> gpui::AnyElement {
    let theme = &sung.theme;
    let passed = line_passed || lane.sung_end().is_some_and(|end| position >= end);
    let shade = |singing: bool| {
        let active =
            singing && position >= lane.start && lane.sung_end().is_none_or(|end| position < end);
        let karaoke =
            secondary_karaoke_visible(lane, singing, position) && sung.karaoke && lane.worded();

        match (active, passed, karaoke) {
            (_, _, true) => theme.muted_foreground,
            (true, _, false) => theme.foreground,
            (false, true, false) => theme.muted_foreground.opacity(PAST),
            (false, false, false) => theme.muted_foreground.opacity(AHEAD),
        }
    };
    // a lane only shows under the line being sung
    let tint = shade(true);
    let size = sung.lane;
    let karaoke_capable = sung.karaoke && lane.worded();
    // a lane holds still while the line above it grows
    let still = Sung { lift: 1., ..sung };
    let lyrics =
        div()
            .text_size(size)
            .map(|this| match (karaoke_capable, lane.words.as_ref(), plan) {
                (true, Some(words), Some(plan)) => this.child(karaoke_lane(
                    plan, lane.start, words, position, size, voice, still,
                )),
                (true, Some(words), None) => this.child(karaoke_lane(
                    &loose_plan(&lane.text, words),
                    lane.start,
                    words,
                    position,
                    size,
                    voice,
                    still,
                )),
                (_, _, Some(plan)) => this.child(fixed_lyrics_lane(&plan.text, voice, still)),
                _ => this.child(SharedString::from(lane.text.clone())),
            });
    let held = shade(true);
    let lyrics = match dimming {
        Some(departure) => lyrics
            .motion(("lane-dim", departure as usize), Motion::Quick, {
                move |this, t| this.text_color(mix(held, tint, t))
            })
            .into_any_element(),
        None => lyrics.text_color(tint).into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .when(hugs_end(voice, sung.mirrored), |this| {
            this.items_end().text_right()
        })
        .child(lyrics)
        .when_some(
            selected_romanization(&lane.romanized, sung.scripts),
            |this, text| this.child(romanized_lyrics_lane(text, size, theme)),
        )
        .into_any_element()
}

fn secondary_lane_started(lane: &music::LyricsLane, position: std::time::Duration) -> bool {
    position >= lane.start
}

fn secondary_karaoke_visible(
    lane: &music::LyricsLane,
    line_active: bool,
    position: std::time::Duration,
) -> bool {
    line_active && secondary_lane_started(lane, position)
}

fn selected_romanization(
    romanized: &Option<music::RomanizedText>,
    scripts: Option<RomanizationScripts>,
) -> Option<String> {
    let romanized = romanized.as_ref()?;
    scripts?
        .contains(romanized.writing_system)
        .then(|| romanized.text.clone())
}

/// Which source a sheet came from and, when it names them, who wrote the song.
fn credit_lines(source: &str, writers: &[String]) -> (SharedString, Option<SharedString>) {
    let from = match source == music::lyrics::LOCAL {
        true => t!("lyrics-source-local"),
        false => t!("lyrics-source", source = source),
    };
    let by = (!writers.is_empty()).then(|| {
        let writers = writers.join(", ");
        t!("lyrics-writers", writers = writers.as_str())
    });
    (from, by)
}

/// A fingerprint of a font name, so the sheet notices a change of font without keeping a copy.
fn font_key(font: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    font.hash(&mut hasher);
    hasher.finish()
}

fn romanized_lyrics_lane(text: String, size: Pixels, theme: &ui::Theme) -> Div {
    div()
        .text_size(size)
        .text_color(theme.muted_foreground)
        .child(SharedString::from(text))
}

fn karaoke_window(
    line_start: std::time::Duration,
    words: &[music::LyricsWord],
    index: usize,
) -> (std::time::Duration, std::time::Duration) {
    let word = &words[index];
    let start = match index {
        0 => line_start.min(word.start),
        _ => word.start,
    };
    let sung = word.end.max(start);
    let end = match words.get(index + 1) {
        // a rest after a word ends its sweep there rather than dragging it along
        Some(next) => match sung > start {
            true => next.start.max(start).min(sung),
            false => next.start.max(start),
        },
        None => sung,
    };
    (start, end)
}

#[cfg(test)]
fn karaoke_fragments(line: &str, words: &[music::LyricsWord]) -> Vec<String> {
    karaoke_parts(line, words)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

fn karaoke_parts(line: &str, words: &[music::LyricsWord]) -> Vec<(String, usize)> {
    let mut starts = Vec::with_capacity(words.len());
    let mut cursor = 0;
    for word in words {
        if word.text.is_empty() {
            return spaced_words(words);
        }
        let Some(remainder) = line.get(cursor..) else {
            return spaced_words(words);
        };
        let Some(relative) = remainder.find(&word.text) else {
            return spaced_words(words);
        };
        let start = cursor + relative;
        starts.push(start);
        cursor = start + word.text.len();
    }

    starts
        .iter()
        .enumerate()
        .flat_map(|(index, start)| {
            let start = match index {
                0 => 0,
                _ => *start,
            };
            let end = starts.get(index + 1).copied().unwrap_or(line.len());
            plain_lyrics_fragments(&line[start..end])
                .into_iter()
                .map(move |piece| (piece, index))
        })
        .collect()
}

fn spaced_words(words: &[music::LyricsWord]) -> Vec<(String, usize)> {
    words
        .iter()
        .enumerate()
        .flat_map(|(index, word)| {
            let mut text = word.text.clone();
            if words
                .get(index + 1)
                .is_some_and(|next| needs_space(&word.text, &next.text))
            {
                text.push(' ');
            }
            plain_lyrics_fragments(&text)
                .into_iter()
                .map(move |piece| (piece, index))
        })
        .collect()
}

fn needs_space(left: &str, right: &str) -> bool {
    let Some(last) = left.chars().next_back() else {
        return false;
    };
    let Some(first) = right.chars().next() else {
        return false;
    };
    if last.is_whitespace() || first.is_whitespace() {
        return false;
    }
    if wide(last) || wide(first) {
        return false;
    }
    !matches!(last, '(' | '[' | '{' | '\'' | '’' | '-' | '—')
        && !matches!(
            first,
            ')' | ']' | '}' | ',' | '.' | '!' | '?' | ';' | ':' | '%' | '\'' | '’' | '-' | '—'
        )
}

/// How far along a transition started at this moment is, asking for frames while
/// it runs.
fn ramp(at: std::time::Instant, window: &mut Window) -> f32 {
    let span = Motion::Base.span().as_secs_f32().max(f32::EPSILON);
    let progress = (at.elapsed().as_secs_f32() / span).clamp(0., 1.);
    if progress < 1. {
        window.request_animation_frame();
    }
    ease_out_expo(progress)
}

/// Scales one line of text without touching the space between it and the next.
fn lifted(row: Div, sung: Sung) -> Div {
    match sung.lift == 1. {
        true => row,
        false => row.layer_scale(sung.lift).layer_scale_origin(sung.from),
    }
}

/// How much a window grows fullscreen lyrics past `LYRICS_FRAME`. The tighter of its width and
/// height decides, and a window no larger than the frame, stacked layouts included, keeps 1.
fn lyrics_frame_scale(viewport: Size<Pixels>) -> f32 {
    (viewport.width / LYRICS_FRAME.width)
        .min(viewport.height / LYRICS_FRAME.height)
        .clamp(1., LYRICS_FRAME_MOST)
}

fn active_verse_size(verse: Pixels) -> Pixels {
    verse + ACTIVE_VERSE_GROWTH
}

fn lyrics_parts(line: &str, words: Option<&[music::LyricsWord]>) -> Vec<(String, usize)> {
    match words {
        Some(words) if !words.is_empty() => karaoke_parts(line, words),
        _ => plain_lyrics_fragments(line)
            .into_iter()
            .enumerate()
            .map(|(index, piece)| (piece, index))
            .collect(),
    }
}

fn plain_lyrics_fragments(line: &str) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut start = 0;
    let mut spacing = false;
    let mut previous = None;
    for (index, letter) in line.char_indices() {
        if letter.is_whitespace() {
            spacing = true;
        } else if spacing || previous.is_some_and(|previous| parts(previous, letter)) {
            fragments.push(line[start..index].to_owned());
            start = index;
            spacing = false;
        }
        previous = Some(letter);
    }
    if start < line.len() {
        fragments.push(line[start..].to_owned());
    }
    fragments
}

// everything a line needs to lay out and light up, measured once per width
#[derive(Clone)]
struct Wrapped {
    fragments: Vec<SharedString>,
    spoken: Vec<usize>,
    rows: Vec<Range<usize>>,
    widths: Vec<Pixels>,
    spans: Vec<(Pixels, Pixels)>,
    evenly: Vec<bool>,
    text: Vec<SharedString>,
    /// Whether the line reads right to left, which sets the side its karaoke fill starts from.
    rtl: bool,
}

fn lyrics_wrap_rows(
    parts: &[(String, usize)],
    font_size: Pixels,
    width: Pixels,
    window: &mut Window,
) -> Option<Wrapped> {
    if width <= px(0.) {
        return None;
    }

    let mut style = window.text_style();
    style.font_weight = FontWeight::SEMIBOLD;
    let fragments = parts
        .iter()
        .map(|(text, _)| SharedString::from(text.clone()))
        .collect::<Vec<_>>();
    let spoken = parts.iter().map(|(_, word)| *word).collect::<Vec<_>>();
    // one shaped line per verse rather than one per fragment: a line of wide
    // characters is a shaping call each otherwise, and the widths that come back
    // this way also carry the kerning across a boundary
    let whole = SharedString::from(
        parts
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<String>(),
    );
    let rtl = right_to_left(&whole);
    let widths = match has_right_to_left(&whole) {
        // right-to-left text sits left of what follows it, so positions along the shaped line no
        // longer measure a fragment: each is shaped on its own instead
        true => parts
            .iter()
            .map(|(text, _)| {
                let run = style.to_run(text.len());
                window
                    .text_system()
                    .shape_line(SharedString::from(text.clone()), font_size, &[run], None)
                    .width
            })
            .collect::<Vec<_>>(),
        false => {
            let run = style.to_run(whole.len());
            let shaped = window
                .text_system()
                .shape_line(whole, font_size, &[run], None);
            let mut widths = Vec::with_capacity(parts.len());
            let mut at = 0;
            let mut left = shaped.x_for_index(0);
            for (text, _) in parts {
                at += text.len();
                let right = shaped.x_for_index(at);
                widths.push(right - left);
                left = right;
            }
            widths
        }
    };
    let breaks = fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| match index.checked_sub(1) {
            Some(previous) => separable(&fragments[previous], fragment),
            None => true,
        })
        .collect::<Vec<_>>();
    let rows = wrap_fragment_widths(&widths, &breaks, width);
    let text = rows
        .iter()
        .map(|row| {
            SharedString::from(parts[row.clone()].iter().fold(
                String::new(),
                |mut whole, (piece, _)| {
                    whole.push_str(piece);
                    whole
                },
            ))
        })
        .collect::<Vec<_>>();

    Some(Wrapped {
        rtl,
        spans: word_spans(&spoken, &widths),
        evenly: evenly_filled(&fragments, &spoken),
        fragments,
        spoken,
        rows,
        widths,
        text,
    })
}

// a fragment's own slice of the word it belongs to
fn word_spans(spoken: &[usize], widths: &[Pixels]) -> Vec<(Pixels, Pixels)> {
    let words = spoken.iter().copied().max().map_or(0, |last| last + 1);
    let mut wholes = vec![px(0.); words];
    for (index, word) in spoken.iter().enumerate() {
        wholes[*word] += widths.get(index).copied().unwrap_or(px(0.));
    }
    let mut befores = vec![px(0.); words];
    spoken
        .iter()
        .enumerate()
        .map(|(index, word)| {
            let before = befores[*word];
            befores[*word] += widths.get(index).copied().unwrap_or(px(0.));
            (before, wholes[*word])
        })
        .collect()
}

fn evenly_filled(fragments: &[SharedString], spoken: &[usize]) -> Vec<bool> {
    let words = spoken.iter().copied().max().map_or(0, |last| last + 1);
    let mut pieces = vec![0usize; words];
    let mut broad = vec![false; words];
    for (index, word) in spoken.iter().enumerate() {
        pieces[*word] += 1;
        broad[*word] |= fragments[index].chars().any(wide);
    }
    pieces
        .into_iter()
        .zip(broad)
        .map(|(pieces, broad)| pieces > 1 || broad)
        .collect()
}

fn separable(left: &str, right: &str) -> bool {
    // spacing ends a row
    if right.starts_with(char::is_whitespace) {
        return false;
    }
    if left.ends_with(char::is_whitespace) {
        return true;
    }

    match (left.chars().next_back(), right.chars().next()) {
        (Some(left), Some(right)) => parts(left, right),
        _ => false,
    }
}

fn wide(letter: char) -> bool {
    matches!(letter,
        '\u{2E80}'..='\u{303E}'
        | '\u{3041}'..='\u{33FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{A000}'..='\u{A4CF}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FF60}'
    )
}

fn parts(left: char, right: char) -> bool {
    if !wide(left) || !wide(right) {
        return false;
    }

    !matches!(
        right,
        '、' | '。'
            | '，'
            | '．'
            | '！'
            | '？'
            | '：'
            | '；'
            | '」'
            | '』'
            | '）'
            | '】'
            | '〉'
            | '》'
            | '〕'
            | '・'
            | 'ー'
            | '…'
            | '々'
            | 'ゝ'
            | 'ゞ'
            | 'っ'
            | 'ッ'
    ) && !matches!(left, '「' | '『' | '（' | '【' | '〈' | '《' | '〔')
}

fn wrap_fragment_widths(widths: &[Pixels], breaks: &[bool], width: Pixels) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = px(0.);

    for (index, fragment) in widths.iter().copied().enumerate() {
        if index > start
            && used + fragment > width
            && let Some(split) = (start + 1..=index)
                .rev()
                .find(|at| breaks.get(*at).copied().unwrap_or(true))
        {
            rows.push(start..split);
            used = widths[split..index].iter().copied().sum();
            start = split;
        }
        used += fragment;
    }
    if start < widths.len() {
        rows.push(start..widths.len());
    }
    rows
}

fn anchored_lyrics_offset(view: Pixels, item: Pixels, height: Pixels, reach: Pixels) -> Pixels {
    let delta = view - item + height * PIN;
    delta.clamp(-reach, px(0.))
}

fn swept(
    start: std::time::Duration,
    end: std::time::Duration,
    position: std::time::Duration,
    tail: bool,
) -> f32 {
    let span = end.saturating_sub(start);
    let travel = match tail {
        true => span.max(SWEEP_LEAST),
        false => span.mul_f32(SWEEP_STRETCH).max(SWEEP_LEAST),
    };
    let eased = ease_out_cubic(progress_between(start, start + travel, position));
    match eased >= SWEPT {
        true => 1.,
        false => eased,
    }
}

fn progress_between(
    start: std::time::Duration,
    end: std::time::Duration,
    position: std::time::Duration,
) -> f32 {
    if position < start {
        return 0.;
    }
    if position >= end {
        return 1.;
    }
    let span = (end - start).as_secs_f32();
    ((position - start).as_secs_f32() / span).clamp(0., 1.)
}

fn instrumental_gap_before(lines: &[music::LyricsLine], index: usize) -> std::time::Duration {
    let start = lines[index].start;
    match index {
        0 => start,
        _ => {
            let previous = &lines[index - 1];
            start.saturating_sub(previous.sung_end().unwrap_or(previous.start))
        }
    }
}

fn active_instrumental(
    lines: &[music::LyricsLine],
    position: std::time::Duration,
) -> Option<usize> {
    let next_line = lines.iter().position(|line| line.start > position)?;
    let gap = instrumental_gap_before(lines, next_line);
    let start = lines[next_line].start.saturating_sub(gap);
    (gap >= INSTRUMENTAL_BREAK && position >= start).then_some(next_line)
}

fn lyric_row_count(lines: &[music::LyricsLine]) -> usize {
    lines.len()
        + (0..lines.len())
            .filter(|index| instrumental_gap_before(lines, *index) >= INSTRUMENTAL_BREAK)
            .count()
}

fn line_row(lines: &[music::LyricsLine], index: usize) -> usize {
    index
        + (0..=index)
            .filter(|line| instrumental_gap_before(lines, *line) >= INSTRUMENTAL_BREAK)
            .count()
}

fn active_lyrics_row(lines: &[music::LyricsLine], position: std::time::Duration) -> Option<usize> {
    if let Some(index) = sung_line(lines, position) {
        return Some(line_row(lines, index));
    }
    let index = active_instrumental(lines, position)?;
    line_row(lines, index).checked_sub(1)
}

fn sung_line(lines: &[music::LyricsLine], position: std::time::Duration) -> Option<usize> {
    match active_instrumental(lines, position) {
        Some(_) => None,
        None => music::lyrics::active(lines, position),
    }
}

fn lanes_room(
    lanes: &[music::LyricsLane],
    scripts: Option<RomanizationScripts>,
    size: Pixels,
    leading: Pixels,
    width: Pixels,
    window: &mut Window,
) -> Pixels {
    let rows = lanes
        .iter()
        .map(|lane| {
            let spoken = wrapped_rows(&lane.text, size, width, window);
            let romanized = selected_romanization(&lane.romanized, scripts)
                .map_or(0, |text| wrapped_rows(&text, size, width, window));
            spoken + romanized
        })
        .sum::<usize>();

    // a lane inherits the line height of the verse, not its own text size
    let gaps = window.rem_size() * LANE_GAP_REM * lanes.len().saturating_sub(1) as f32;

    leading * (rows as f32 + LANE_SLACK) + gaps
}

fn wrapped_rows(text: &str, size: Pixels, width: Pixels, window: &mut Window) -> usize {
    let parts = lyrics_parts(text, None);
    lyrics_wrap_rows(&parts, size, width, window).map_or(1, |wrapped| wrapped.rows.len().max(1))
}

/// Draws a sheet row `shift` away from where layout put it. A moved row goes in a wrapper, so the
/// sheet's scroll handle keeps measuring where the row rests, which the haze and the follow read.
/// A row at rest goes in bare, since it lays out the same, unless `margined` says it carries an
/// outer margin the wrapper's bounds have to keep.
fn adrift(
    row: impl Styled + IntoElement,
    shift: Pixels,
    margined: bool,
    window: &Window,
) -> gpui::AnyElement {
    if shift == px(0.) && !margined {
        return row.into_any_element();
    }
    let grid = snapped(shift, window);

    div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .child(
            row.top(grid)
                .layer_translate(gpui::point(px(0.), shift - grid)),
        )
        .into_any_element()
}

struct Place {
    top: Pixels,
    height: Pixels,
    travel: f32,
    along: f32,
}

// shift is drawn, not laid out
fn viewport_place(
    scroll: &ScrollHandle,
    row: usize,
    view: Bounds<Pixels>,
    shift: Pixels,
) -> Option<Place> {
    let height = view.size.height;
    let item = scroll.bounds_for_item(row)?;
    if height <= px(0.) {
        return None;
    }
    let top = item.origin.y - view.origin.y + scroll.offset().y + shift;
    let travel = top - height * PIN;
    let reach = height
        * match travel >= px(0.) {
            true => 1. - PIN,
            false => PIN,
        };
    Some(Place {
        top,
        height: item.size.height,
        travel: (travel / reach.max(px(1.))).clamp(-1., 1.),
        along: (top / height).clamp(0., 1.),
    })
}

fn viewport_haze(
    scroll: &ScrollHandle,
    row: usize,
    view: Bounds<Pixels>,
    margin: Pixels,
    drift: Pixels,
) -> f32 {
    let Some(place) = viewport_place(scroll, row, view, drift) else {
        return 0.;
    };
    if place.top + place.height + margin < px(0.) || place.top - margin > view.size.height {
        return 0.;
    }
    place.travel.abs().powf(HAZE)
}

#[derive(Clone, Copy, Default)]
struct Drag {
    step: Pixels,
    beat: f32,
    downward: bool,
    most: Pixels,
}

fn lag_spring(along: f32) -> SpringConfig {
    let frequency = 1. - LAG_STAGGER * along.clamp(0., 1.);
    let spring = Springs::LYRICS_ROW;
    SpringConfig::new(
        spring.stiffness * frequency * frequency,
        spring.damping * frequency,
        spring.mass,
    )
}

// incoming rows last
fn viewport_along(scroll: &ScrollHandle, row: usize, view: Bounds<Pixels>, downward: bool) -> f32 {
    let Some(place) = viewport_place(scroll, row, view, px(0.)) else {
        return 0.;
    };
    match downward {
        true => place.along,
        false => 1. - place.along,
    }
}

/// A row's blur rounded to the nearest `HAZE_STEP`.
fn stepped(blur: Pixels) -> Pixels {
    HAZE_STEP * (blur / HAZE_STEP).round()
}

fn line_has_passed(line: &music::LyricsLine, position: std::time::Duration) -> bool {
    line.sung_end().is_some_and(|end| position >= end)
}

fn primary_karaoke_visible(
    line: &music::LyricsLine,
    line_active: bool,
    position: std::time::Duration,
) -> bool {
    line_active
        || (position >= line.start
            && line
                .sung_end()
                .is_some_and(|end| position < end + Motion::Control.span()))
}

fn primary_karaoke_fade(
    line: &music::LyricsLine,
    line_active: bool,
    position: std::time::Duration,
) -> f32 {
    if line_active {
        return 0.;
    }
    line.sung_end().map_or(0., |end| {
        progress_between(end, end + Motion::Control.span(), position)
    })
}

fn background_line_singing(
    line: &music::LyricsLine,
    line_active: bool,
    position: std::time::Duration,
) -> bool {
    !line_active && position >= line.start && line.sung_end().is_some_and(|end| position < end)
}

fn instrumental_row(progress: f32, past: bool, verse: Pixels, theme: &ui::Theme) -> Div {
    let note_size = verse * 1.;
    div()
        .flex()
        .items_center()
        .gap_2()
        .py(verse * 0.45)
        .children((0..3).map(|index| {
            let note_progress = (progress * 3. - index as f32).clamp(0., 1.);
            let tint = match past {
                true => theme.muted_foreground.opacity(PAST),
                false => mix(
                    theme.muted_foreground.opacity(AHEAD),
                    theme.primary,
                    note_progress,
                ),
            };
            div()
                .size(note_size)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(icons::path("icons/music-2.svg"))
                        .size(note_size)
                        .text_color(tint),
                )
        }))
}

fn wordless(key: &'static str, icon: &'static str) -> gpui::AnyElement {
    Vacancy::new(i18n::lookup(key, None))
        .spoken(key)
        .icon(icon)
        .flex_1()
        .into_any_element()
}

/// Whether a line by `voice` sits against the right edge. The lead voice starts on the side its
/// script starts from, and a counter voice answers from the other.
fn hugs_end(voice: Voice, mirrored: bool) -> bool {
    voice.lead() == mirrored
}

/// Whether most lines of `lyrics` read right to left, as Arabic or Hebrew do, so the lyrics sit
/// against the right edge. Lines without a letter that has a direction do not count.
fn reads_right_to_left(lyrics: &music::Lyrics) -> bool {
    let lean = |text: &str| match unicode_bidi::get_base_direction(text) {
        Direction::Rtl => 1,
        Direction::Ltr => -1,
        Direction::Mixed => 0,
    };
    let leaning: i32 = match lyrics {
        music::Lyrics::Plain { text, .. } => text.lines().map(lean).sum(),
        music::Lyrics::Synced { lines } => lines.iter().map(|line| lean(&line.text)).sum(),
    };
    leaning > 0
}

/// Whether `text` starts right to left, going by its first letter that has a direction.
fn right_to_left(text: &str) -> bool {
    unicode_bidi::get_base_direction(text) == Direction::Rtl
}

/// Whether any letter of `text` reads right to left.
fn has_right_to_left(text: &str) -> bool {
    text.chars().any(|letter| {
        matches!(
            unicode_bidi::bidi_class(letter),
            BidiClass::R | BidiClass::AL
        )
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use music::{LyricsLane, LyricsLine, LyricsWord, Voice};
    use ui::Springs;

    use super::{
        QueuePosition, Sections, Slot, active_lyrics_row, anchored_lyrics_offset,
        background_line_singing, karaoke_fragments, karaoke_window, lag_spring, line_has_passed,
        line_row, lyric_row_count, plain_lyrics_fragments, primary_karaoke_fade,
        primary_karaoke_visible, secondary_karaoke_visible, wrap_fragment_widths,
    };
    use gpui::px;
    use ui::Motion;

    fn slots(sections: Sections) -> Vec<Slot> {
        (0..sections.len())
            .filter_map(|i| sections.slot(i))
            .collect()
    }

    #[test]
    fn lays_out_every_section() {
        let sections = Sections::new(
            2,
            true,
            [
                state::QueueKind::Context,
                state::QueueKind::Queued,
                state::QueueKind::Context,
                state::QueueKind::Suggested,
            ],
        );

        assert_eq!(sections.current_index(), Some(4));
        assert_eq!(
            slots(sections),
            [
                Slot::Header("queue-history"),
                Slot::Track(QueuePosition::Past(0)),
                Slot::Track(QueuePosition::Past(1)),
                Slot::Header("queue-now-playing"),
                Slot::Track(QueuePosition::Current),
                Slot::Header("queue-up-next"),
                Slot::Track(QueuePosition::Upcoming(0)),
                Slot::Header("queue-next-in-queue"),
                Slot::Track(QueuePosition::Upcoming(1)),
                Slot::Header("queue-up-next"),
                Slot::Track(QueuePosition::Upcoming(2)),
                Slot::Header("queue-similar"),
                Slot::Track(QueuePosition::Upcoming(3)),
            ]
        );
    }

    #[test]
    fn suggests_similar_tracks_without_anything_up_next() {
        let sections = Sections::new(0, true, [state::QueueKind::Suggested]);

        assert_eq!(
            slots(sections),
            [
                Slot::Header("queue-now-playing"),
                Slot::Track(QueuePosition::Current),
                Slot::Header("queue-similar"),
                Slot::Track(QueuePosition::Upcoming(0)),
            ]
        );
    }

    #[test]
    fn drops_headers_for_empty_sections() {
        let sections = Sections::new(0, true, [state::QueueKind::Context]);

        assert_eq!(sections.current_index(), Some(1));
        assert_eq!(
            slots(sections),
            [
                Slot::Header("queue-now-playing"),
                Slot::Track(QueuePosition::Current),
                Slot::Header("queue-up-next"),
                Slot::Track(QueuePosition::Upcoming(0)),
            ]
        );
    }

    #[test]
    fn lays_out_history_without_a_current_track() {
        let sections = Sections::new(1, false, []);

        assert_eq!(sections.current_index(), None);
        assert_eq!(
            slots(sections),
            [
                Slot::Header("queue-history"),
                Slot::Track(QueuePosition::Past(0))
            ]
        );
    }

    #[test]
    fn an_empty_queue_has_no_rows() {
        let sections = Sections::new(0, false, []);

        assert_eq!(sections.len(), 0);
        assert_eq!(sections.current_index(), None);
    }

    #[test]
    fn a_long_instrumental_pause_gets_its_own_lyrics_row() {
        let lines = [
            LyricsLine {
                start: Duration::from_secs(2),
                end: Some(Duration::from_secs(5)),
                text: "first".to_owned(),
                romanized: None,
                words: None,
                secondary: Vec::new(),
                voice: Voice::Lead,
            },
            LyricsLine {
                start: Duration::from_secs(12),
                end: Some(Duration::from_secs(15)),
                text: "second".to_owned(),
                romanized: None,
                words: None,
                secondary: Vec::new(),
                voice: Voice::Lead,
            },
        ];

        assert_eq!(lyric_row_count(&lines), 3);
        assert_eq!(line_row(&lines, 0), 0);
        assert_eq!(line_row(&lines, 1), 2);
        assert_eq!(active_lyrics_row(&lines, Duration::from_secs(8)), Some(1));
        assert_eq!(active_lyrics_row(&lines, Duration::from_secs(13)), Some(2));
    }

    #[test]
    fn word_timing_exposes_a_pause_hidden_by_the_line_end() {
        let lines = [
            LyricsLine {
                start: Duration::from_secs(2),
                end: Some(Duration::from_secs(12)),
                text: "first".to_owned(),
                romanized: None,
                words: Some(vec![LyricsWord {
                    start: Duration::from_secs(2),
                    end: Duration::from_secs(5),
                    text: "first".to_owned(),
                }]),
                secondary: Vec::new(),
                voice: Voice::Lead,
            },
            LyricsLine {
                start: Duration::from_secs(12),
                end: Some(Duration::from_secs(15)),
                text: "second".to_owned(),
                romanized: None,
                words: None,
                secondary: Vec::new(),
                voice: Voice::Lead,
            },
        ];

        assert_eq!(lyric_row_count(&lines), 3);
        assert_eq!(active_lyrics_row(&lines, Duration::from_secs(8)), Some(1));
        assert!(line_has_passed(&lines[0], Duration::from_secs(8)));
    }

    #[test]
    fn lyrics_follow_uses_unscrolled_item_bounds() {
        let offset = anchored_lyrics_offset(px(0.), px(200.), px(100.), px(500.));

        assert_eq!(offset, px(-170.));
    }

    #[test]
    fn lyrics_follow_preserves_a_subpixel_target() {
        let offset = anchored_lyrics_offset(px(0.25), px(200.125), px(100.5), px(500.));

        assert!((offset.as_f32() - -169.725).abs() < 0.001);
    }

    #[test]
    fn karaoke_uses_spacing_from_the_complete_line() {
        let text = "I said oooh I'm drowning in the night";
        let words = ["I", "said", "oooh", "I'm", "drowning", "in", "the", "night"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| LyricsWord {
                start: Duration::from_millis(index as u64 * 100),
                end: Duration::from_millis(index as u64 * 100 + 100),
                text: text.to_owned(),
            })
            .collect::<Vec<_>>();

        let fragments = karaoke_fragments(text, &words);

        assert_eq!(fragments.concat(), text);
        assert_eq!(
            fragments,
            [
                "I ",
                "said ",
                "oooh ",
                "I'm ",
                "drowning ",
                "in ",
                "the ",
                "night"
            ]
        );
    }

    #[test]
    fn plain_lyrics_keep_spacing_in_breakable_fragments() {
        let fragments = plain_lyrics_fragments("Ладони полны слёзок, но время");

        assert_eq!(fragments.concat(), "Ладони полны слёзок, но время");
        assert_eq!(fragments, ["Ладони ", "полны ", "слёзок, ", "но ", "время"]);
    }

    #[test]
    fn lyrics_wrap_at_the_active_size_plan() {
        let rows = wrap_fragment_widths(&[px(40.), px(35.), px(30.), px(20.)], &[true; 4], px(80.));

        assert_eq!(rows, [0..2, 2..4]);
    }

    #[test]
    fn lyrics_keep_an_oversized_fragment_on_its_own_row() {
        let rows = wrap_fragment_widths(&[px(120.), px(30.), px(30.)], &[true; 3], px(80.));

        assert_eq!(rows, [0..1, 1..3]);
    }

    #[test]
    fn lyrics_row_springs_stagger_without_changing_their_damping_ratio() {
        let (first_frequency, first_ratio) = lag_spring(0.).canonical();
        let (last_frequency, last_ratio) = lag_spring(1.).canonical();

        assert!(first_frequency > last_frequency);
        assert!((first_ratio - last_ratio).abs() < f32::EPSILON);
        assert!(
            first_ratio < 1.,
            "the lyrics settle should have a subtle overshoot"
        );
    }

    #[test]
    fn lyrics_keep_their_tuned_spring_presets() {
        assert_eq!(Springs::LYRICS_SCROLL.stiffness, 170.);
        assert_eq!(Springs::LYRICS_SCROLL.damping, 23.);
        assert_eq!(Springs::LYRICS_SCROLL.mass, 1.);
        assert_eq!(Springs::LYRICS_ROW.stiffness, 210.);
        assert_eq!(Springs::LYRICS_ROW.damping, 22.);
        assert_eq!(Springs::LYRICS_ROW.mass, 1.);
    }

    #[test]
    fn a_late_first_word_uses_the_whole_lead_in() {
        let words = vec![
            LyricsWord {
                start: Duration::from_millis(1500),
                end: Duration::from_millis(1900),
                text: "first".to_owned(),
            },
            LyricsWord {
                start: Duration::from_millis(2000),
                end: Duration::from_millis(2400),
                text: "second".to_owned(),
            },
        ];

        assert_eq!(
            karaoke_window(Duration::from_millis(1000), &words, 0),
            (Duration::from_millis(1000), Duration::from_millis(1900))
        );
    }

    #[test]
    fn a_finished_background_lane_stays_sung_until_its_line_departs() {
        let lane = LyricsLane {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(3)),
            text: "(E)".to_owned(),
            romanized: None,
            words: Some(vec![LyricsWord {
                start: Duration::from_secs(2),
                end: Duration::from_secs(3),
                text: "(E)".to_owned(),
            }]),
        };

        assert!(!secondary_karaoke_visible(
            &lane,
            true,
            Duration::from_millis(1999)
        ));
        assert!(secondary_karaoke_visible(
            &lane,
            true,
            Duration::from_secs(2)
        ));
        assert!(secondary_karaoke_visible(
            &lane,
            true,
            Duration::from_secs(4)
        ));
        assert!(!secondary_karaoke_visible(
            &lane,
            false,
            Duration::from_secs(4)
        ));
    }

    #[test]
    fn an_overlapped_primary_line_keeps_singing_in_the_background() {
        let line = LyricsLine {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(8)),
            text: "Wake me up inside".to_owned(),
            romanized: None,
            words: Some(vec![LyricsWord {
                start: Duration::from_secs(2),
                end: Duration::from_secs(8),
                text: "Wake me up inside".to_owned(),
            }]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        };

        assert!(!primary_karaoke_visible(
            &line,
            false,
            Duration::from_millis(1999)
        ));
        assert!(primary_karaoke_visible(
            &line,
            false,
            Duration::from_secs(5)
        ));
        assert!(primary_karaoke_visible(
            &line,
            false,
            Duration::from_secs(8)
        ));
        assert!(!primary_karaoke_visible(
            &line,
            false,
            Duration::from_secs(8) + Motion::Base.span()
        ));
    }

    #[test]
    fn the_active_primary_line_keeps_its_completed_sweep_until_departure() {
        let line = LyricsLine {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(5)),
            text: "line".to_owned(),
            romanized: None,
            words: Some(vec![LyricsWord {
                start: Duration::from_secs(2),
                end: Duration::from_secs(5),
                text: "line".to_owned(),
            }]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        };

        assert!(primary_karaoke_visible(&line, true, Duration::from_secs(8)));
    }

    #[test]
    fn a_finished_background_line_fades_from_white_to_gray() {
        let line = LyricsLine {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(8)),
            text: "Wake me up inside".to_owned(),
            romanized: None,
            words: Some(vec![LyricsWord {
                start: Duration::from_secs(2),
                end: Duration::from_secs(8),
                text: "Wake me up inside".to_owned(),
            }]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        };
        let fade = Motion::Control.span();

        assert_eq!(
            primary_karaoke_fade(&line, false, Duration::from_millis(7999)),
            0.
        );
        assert_eq!(
            primary_karaoke_fade(&line, false, Duration::from_secs(8) + fade / 2),
            0.5
        );
        assert_eq!(
            primary_karaoke_fade(&line, false, Duration::from_secs(8) + fade),
            1.
        );
        assert_eq!(
            primary_karaoke_fade(&line, true, Duration::from_secs(8) + fade),
            0.
        );
    }

    #[test]
    fn only_a_currently_singing_background_line_gets_the_reduced_blur() {
        let line = LyricsLine {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(8)),
            text: "Wake me up inside".to_owned(),
            romanized: None,
            words: Some(vec![LyricsWord {
                start: Duration::from_secs(2),
                end: Duration::from_secs(8),
                text: "Wake me up inside".to_owned(),
            }]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        };

        assert!(!background_line_singing(
            &line,
            false,
            Duration::from_millis(1999)
        ));
        assert!(background_line_singing(
            &line,
            false,
            Duration::from_secs(5)
        ));
        assert!(!background_line_singing(
            &line,
            false,
            Duration::from_secs(8)
        ));
        assert!(!background_line_singing(
            &line,
            true,
            Duration::from_secs(5)
        ));
    }

    #[test]
    fn a_finished_line_stays_past_during_a_gap() {
        let line = LyricsLine {
            start: Duration::from_secs(2),
            end: Some(Duration::from_secs(5)),
            text: "line".to_owned(),
            romanized: None,
            words: None,
            secondary: Vec::new(),
            voice: Voice::Lead,
        };

        assert!(line_has_passed(&line, Duration::from_secs(8)));
    }
}
