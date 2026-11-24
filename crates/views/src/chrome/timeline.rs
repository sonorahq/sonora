use ui::ActiveTheme as _;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, Global, MouseUpEvent, Pixels, Point, Render, SharedString,
    Window, canvas, div,
};
use i18n::t;
use state::{AppSettings, Playback, Sonora};
use ui::{Scrubber, ScrubberState, clock, tabular};

use crate::shared::transport::{clock_ems, end_clock, fraction, moved};

/// How thick the island's seek line draws at rest and wide open, as multiples of the usual track.
const ISLAND_THIN: f32 = 0.6;
const ISLAND_THICK: f32 = 1.5;
/// How far above the bottom of the seek row the island's clocks stand, in pads: just over the
/// open line, whose middle is the row's.
const CLOCK_LIFT: f32 = 2.25;
/// The seek bar's track and thumb in pads and its bubble's padding either side of the time in
/// rems, as `ui::Scrubber` draws them, and the air in pads the island keeps between the bubble
/// and a clock before that clock gives way to it.
const TRACK: f32 = 0.5;
const THUMB: f32 = 1.5;
const BUBBLE_PAD: f32 = 0.375;
const BUBBLE_AIR: f32 = 0.5;

/// The window y of the middle of the seek line as the player bar last painted it.
#[derive(Default)]
struct SeekLine(Option<Pixels>);

impl Global for SeekLine {}

/// The player bar's elapsed clock, seek bar and remaining or total clock. In the island the
/// clocks stand above the line instead and show only while it is open. It follows the
/// position on its own and redraws only when a clock reads differently or the bar would move by
/// a device pixel, so the rest of the player bar does not draw on every position report.
pub(crate) struct Timeline {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    seek: ScrubberState,
    /// Where a drag on the seek bar has the thumb, until it lets go and seeks there.
    pending: Option<f32>,
    /// Where the pointer rests over the seek bar, for the bubble.
    over: Option<f32>,
    /// The window's scale factor at the last render, for telling whether a report moves the bar
    /// by a device pixel.
    scale: f32,
    /// What the last render drew, so a report that changes none of it draws nothing.
    drawn: Option<Drawn>,
    /// How far the island's seek line has opened, from 0 for a thin bare line to 1 for a thick one
    /// with both clocks above it. `None` outside the island, where the clocks stand at either end
    /// of the bar.
    spread: Option<f32>,
}

/// Tells that a drag on the seek bar has let go and sought, with where the pointer was.
pub(crate) struct Released(pub Point<Pixels>);

impl EventEmitter<Released> for Timeline {}

/// What a position report can change on the timeline: the seconds both clocks read, and how
/// far along the bar is in device pixels.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    elapsed: u64,
    left: u64,
    total: u64,
    fill: i64,
    seekable: bool,
    time_left: bool,
}

impl Timeline {
    pub fn new(playback: Entity<Playback>, cx: &mut Context<Self>) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |this, _, cx| this.follow(cx)).detach();
        cx.observe(&playback, |this, _, cx| this.follow(cx))
            .detach();
        cx.observe(&settings, |this, _, cx| this.follow(cx))
            .detach();
        let sponsorblock = Sonora::global(cx).sponsorblock.clone();
        cx.observe(&sponsorblock, |_, _, cx| cx.notify()).detach();
        Self {
            playback,
            settings,
            seek: ScrubberState::new("seek"),
            pending: None,
            over: None,
            scale: 1.,
            drawn: None,
            spread: None,
        }
    }

    /// Draws the seek bar the island's way when `spread` is set, `None` for the clocks at either
    /// end. It redraws only when that changes.
    pub fn spread(&mut self, spread: Option<f32>, cx: &mut Context<Self>) {
        if self.spread != spread {
            self.spread = spread;
            cx.notify();
        }
    }

    /// Whether the pointer rests over the seek bar, as of the last move the player bar handed on.
    pub fn over_seek(&self) -> bool {
        self.over.is_some()
    }

    /// Whether `position` lies on the seek bar.
    pub fn on_seek(&self, position: Point<Pixels>, cx: &App) -> bool {
        self.seek
            .hovered(position, cx.theme().metrics.pad)
            .is_some()
    }

    /// Whether a drag on the seek bar is going on.
    pub fn dragging(&self) -> bool {
        self.pending.is_some()
    }

    /// Follows the pointer over the player bar, which shows the bubble while it is over the
    /// seek bar. The bar hands every move on, so leaving the seek bar for the rest of the bar
    /// hides it.
    pub fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let over = self.seek.hovered(position, cx.theme().metrics.pad);
        if moved(self.over, over) {
            self.over = over;
            cx.notify();
        }
    }

    /// Hides the bubble once the pointer is off the island, which can move out from under a
    /// still pointer as the sheet slides and so hands on no last move to hide it by.
    pub fn leave(&mut self, cx: &mut Context<Self>) {
        if self.over.take().is_some() {
            cx.notify();
        }
    }

    /// Redraws when what the timeline shows has changed since the last render.
    fn follow(&mut self, cx: &mut Context<Self>) {
        if self.drawn != Some(self.shown(cx)) {
            cx.notify();
        }
    }

    /// What the timeline would draw now.
    fn shown(&self, cx: &App) -> Drawn {
        let playback = self.playback.read(cx);
        let (elapsed, total) = playback.shown_time(cx);
        let travel = f32::from(self.seek.travel(cx.theme().metrics.pad));
        let at = self.pending.unwrap_or_else(|| fraction(elapsed, total));
        Drawn {
            elapsed: elapsed.as_secs(),
            left: total.saturating_sub(elapsed).as_secs(),
            total: total.as_secs(),
            fill: (at * travel * self.scale).round() as i64,
            seekable: playback.shown_track(cx).is_some(),
            time_left: self.settings.read(cx).player_bar_time_left(),
        }
    }

    fn commit_seek(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(fraction) = self.pending.take() else {
            return;
        };
        self.playback
            .update(cx, |playback, cx| playback.seek_fraction(fraction, cx));
        cx.emit(Released(at));
    }
}

impl Timeline {
    /// The window y of the middle of the seek line as it last painted, for an overlay that lines
    /// its own controls up with it. None until the player bar has painted once.
    pub(crate) fn line(cx: &App) -> Option<Pixels> {
        cx.try_global::<SeekLine>().and_then(|line| line.0)
    }
}

impl Render for Timeline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scale = window.scale_factor();
        self.drawn = Some(self.shown(cx));
        let theme = *cx.theme();
        let muted = theme.muted_foreground;
        let empty = muted.opacity(0.3);
        // The island's clocks take the room the cover and title leave, so they read at the title's
        // size and a little brighter than the small muted ones beside a classic bar's line.
        let (clock_text, ink) = match self.spread {
            Some(_) => (theme.text(ui::Text::Body), theme.foreground.opacity(0.7)),
            None => (theme.text(ui::Text::Tiny), muted),
        };

        let playback = self.playback.read(cx);
        let seekable = playback.shown_track(cx).is_some();
        let (elapsed, total) = playback.shown_time(cx);
        let progress = self.pending.unwrap_or_else(|| fraction(elapsed, total));
        let time_left = self.settings.read(cx).player_bar_time_left();
        let clock_width = clock_text * clock_ems(total, time_left);

        let bubble = self
            .over
            .or(self.pending)
            .map(|at| (at, clock(total.mul_f32(at))));
        // In the island a clock gives way to the bubble only once the two would touch, measured
        // by the text each one sets rather than the room a clock keeps for a longer time. The
        // bubble stands over the thumb, which travels a thumb short of the row.
        let pad = theme.metrics.pad;
        let travel = self.seek.travel(pad);
        let thumb = pad * THUMB;
        let row = travel + thumb;
        let closing_text = match time_left {
            true => end_clock(elapsed, total, time_left).text,
            false => clock(total),
        };
        let (start_clear, end_clear) = match &bubble {
            Some((at, text)) => {
                let chip = set_width(text, theme.text(ui::Text::Tiny), window)
                    + window.rem_size() * BUBBLE_PAD * 2.;
                let reach = chip / 2. + pad * BUBBLE_AIR;
                let x = thumb / 2. + travel * *at;
                let start = set_width(&clock(elapsed), clock_text, window);
                let end = set_width(&closing_text, clock_text, window);
                (x - reach >= start, x + reach <= row - end)
            }
            None => (true, true),
        };
        let gives_way = |clear: bool| match clear {
            true => 1.,
            false => 0.,
        };

        let clock_label = |id: &'static str, text: SharedString, align_end: bool| {
            ui::spoken(id, text)
                .w(clock_width)
                .flex_none()
                .whitespace_nowrap()
                .text_size(clock_text)
                .font_features(tabular())
                .text_color(ink)
                .when_else(align_end, |this| this.text_right(), |this| this.text_left())
        };

        // The remaining or total clock, whichever the settings ask for.
        let closing = |align_end: bool| match time_left {
            true => {
                let end = end_clock(elapsed, total, time_left);
                clock_label("player-time-left", end.text, align_end)
                    .when_some(end.spoken, |this, spoken| this.aria_value(spoken))
            }
            false => clock_label("player-duration", clock(total), align_end),
        };
        // The island's line rests thin and bare, and swells with its clocks as it opens. Its
        // thumb is part of the line rather than a dot of its own: at rest it is as thick as the
        // line and its colour, and it grows and lightens to a full thumb as the line opens. While
        // the line moves it is drawn off the pixel grid, so it swells smoothly rather than a
        // pixel at a time.
        let (thickness, pin, dot, moving) = match self.spread {
            Some(spread) => {
                let thickness = ISLAND_THIN + (ISLAND_THICK - ISLAND_THIN) * spread;
                let flush = thickness * TRACK / THUMB;
                (
                    thickness,
                    ui::mix(theme.progress_bar, theme.foreground, spread),
                    flush + (1. - flush) * spread,
                    spread > 0. && spread < 1.,
                )
            }
            None => (1., theme.foreground, 1., false),
        };

        div()
            .id("player-seek")
            .role(Role::Group)
            .aria_label(t!("song-position"))
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .when(self.spread.is_none(), |this| {
                this.child(clock_label("player-elapsed", clock(elapsed), true))
            })
            .child(
                div().flex_1().min_w_0().child(
                    Scrubber::new(&self.seek, progress)
                        .name(t!("song-position"))
                        .value(clock(elapsed))
                        .thickness(thickness)
                        .thumb_scale(dot)
                        .when(moving, |this| this.pixel_snap(false))
                        .colors(theme.progress_bar, empty, pin)
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
                        .on_release(cx.listener(|this, event: &MouseUpEvent, _, cx| {
                            this.commit_seek(event.position, cx)
                        })),
                ),
            )
            .when(self.spread.is_none(), |this| this.child(closing(false)))
            // The clocks stand above the line at its two ends and show as it opens. They stay in
            // the tree while it is shut, clear, so a screen reader still has the time left.
            .when_some(self.spread, |this, spread| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(theme.metrics.pad * CLOCK_LIFT)
                        .flex()
                        .justify_between()
                        .child(
                            clock_label("player-elapsed", clock(elapsed), false)
                                .opacity(spread * gives_way(start_clear)),
                        )
                        .child(closing(true).opacity(spread * gives_way(end_clear))),
                )
            })
            .child(
                canvas(
                    |bounds, _, cx| cx.default_global::<SeekLine>().0 = Some(bounds.center().y),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

/// How wide `text` sets at `size` in the window's font.
fn set_width(text: &SharedString, size: Pixels, window: &Window) -> Pixels {
    let base = window.text_style().font_size.to_pixels(window.rem_size());
    ui::text_width(text.clone(), window) * (size / base)
}
