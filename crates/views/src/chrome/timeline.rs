use ui::ActiveTheme as _;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, Global, Pixels, Point, Render, SharedString, Window, canvas, div,
};
use i18n::t;
use state::{AppSettings, Playback, Sonora};
use ui::{clock, tabular};

use crate::shared::transport::{SeekControl, clock_ems, end_clock};

/// The window y of the middle of the seek line as the player bar last painted it.
#[derive(Default)]
struct SeekLine(Option<Pixels>);

impl Global for SeekLine {}

/// The player bar's elapsed clock, seek bar and remaining or total clock. It follows the
/// position on its own and redraws only when a clock reads differently or the bar would move by
/// a device pixel, so the rest of the player bar does not draw on every position report.
pub(crate) struct Timeline {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    seek: SeekControl,
    /// The window's scale factor at the last render, for telling whether a report moves the bar
    /// by a device pixel.
    scale: f32,
    /// What the last render drew, so a report that changes none of it draws nothing.
    drawn: Option<Drawn>,
}

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
            seek: SeekControl::new("seek"),
            scale: 1.,
            drawn: None,
        }
    }

    /// Follows the pointer over the player bar, which shows the bubble while it is over the
    /// seek bar. The bar hands every move on, so leaving the seek bar for the rest of the bar
    /// hides it.
    pub fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.seek.hover(position, cx.theme().metrics.pad) {
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
        let (elapsed, total) = playback.elapsed_and_total();
        let travel = f32::from(self.seek.travel(cx.theme().metrics.pad));
        let at = self.seek.progress(elapsed, total);
        Drawn {
            elapsed: elapsed.as_secs(),
            left: total.saturating_sub(elapsed).as_secs(),
            total: total.as_secs(),
            fill: (at * travel * self.scale).round() as i64,
            seekable: playback.track().is_some(),
            time_left: self.settings.read(cx).player_bar_time_left(),
        }
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
        let clock_text = theme.text(ui::Text::Tiny);

        let playback = self.playback.read(cx);
        let (elapsed, total) = playback.elapsed_and_total();
        let time_left = self.settings.read(cx).player_bar_time_left();
        let clock_width = clock_text * clock_ems(total, time_left);

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

        div()
            .id("player-seek")
            .role(Role::Group)
            .aria_label(t!("song-position"))
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(clock_label("player-elapsed", clock(elapsed), true))
            .child(
                div().flex_1().min_w_0().child(self.seek.slider(
                    &self.playback,
                    (elapsed, total),
                    cx,
                    |view| &mut view.seek,
                )),
            )
            .child(match time_left {
                true => {
                    let end = end_clock(elapsed, total, time_left);
                    clock_label("player-time-left", end.text, false)
                        .when_some(end.spoken, |this, spoken| this.aria_value(spoken))
                }
                false => clock_label("player-duration", clock(total), false),
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
