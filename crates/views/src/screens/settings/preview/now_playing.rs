//! A miniature of the Now playing view: the ambient field, the visualizer, artwork, title and
//! artist. The visualizer draws random levels rather than the spectrum, so it moves whether or
//! not anything plays.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{AnyElement, Bounds, Context, EntityId, FontWeight, Pixels, Window, canvas, div};
use ui::{ActiveTheme as _, Artwork, Levels, Text, Visualizer};

use crate::screens::settings::{Layout, SettingsView};

/// How tall the miniature stands for its width, the shape of a window.
const ASPECT: f32 = 0.5;
/// How much of the miniature's height the cover takes.
const COVER: f32 = 0.46;
/// How much of the miniature's width the title and artist may take.
const META: f32 = 0.6;
/// How many bands the random levels carry, as many as the analyzer reports.
const BANDS: usize = 32;
/// How often the levels pick new heights to move towards.
const PICK: Duration = Duration::from_millis(140);
/// How long the drawn levels take to close 63% of the gap to the picked ones.
const SETTLE: Duration = Duration::from_millis(90);
/// The longest gap between two frames that still counts as one step. Past it the levels adopt
/// the picked ones outright.
const STALL: Duration = Duration::from_millis(250);
/// The loudest and quietest a band leans to, from the lowest band to the highest, so the random
/// levels slope down the way music does.
const TILT: [f32; 2] = [1., 0.6];
/// The least of its lean a band picks, and the least the whole set swells to on one pick.
const LOW: f32 = 0.08;
const SWELL: f32 = 0.8;
/// How much of its randomness a band shares between the two channels.
const SHARED: f32 = 0.75;

/// What the Now playing page's preview keeps between frames.
pub(in crate::screens::settings) struct NowPlayingPreview {
    levels: RefCell<Wander>,
    /// Whether a redraw for the next frame is already on its way.
    armed: Cell<bool>,
    /// Where the miniature and its title and artist landed last frame, which bounds how tall
    /// the visualizer may reach. A move redraws the preview, so a still one catches up too.
    frame: Rc<Cell<Bounds<Pixels>>>,
    meta: Rc<Cell<Bounds<Pixels>>>,
}

/// Random levels that ease towards a fresh pick every `PICK`, standing in for the spectrum.
struct Wander {
    shown: Levels,
    target: Levels,
    seed: u64,
    picked: Option<Instant>,
    beat: Option<Instant>,
}

impl NowPlayingPreview {
    pub(in crate::screens::settings) fn new(_cx: &mut Context<SettingsView>) -> Self {
        Self {
            levels: RefCell::new(Wander::new(super::random())),
            armed: Cell::new(false),
            frame: Rc::default(),
            meta: Rc::default(),
        }
    }
}

impl Wander {
    fn new(seed: u64) -> Self {
        Self {
            shown: Levels::default(),
            target: Levels::default(),
            // xorshift never leaves zero
            seed: seed.max(1),
            picked: None,
            beat: None,
        }
    }

    /// Moves the levels on to `now`, picking new targets when the last pick is `PICK` old. With
    /// `still` set the levels jump to their targets and pick only once.
    fn step(&mut self, now: Instant, still: bool) -> Levels {
        let due = match self.picked {
            None => true,
            Some(picked) => !still && now.duration_since(picked) >= PICK,
        };
        if due {
            self.pick();
            self.picked = Some(now);
        }
        let rate = match self.beat.replace(now) {
            Some(beat) if !still && now.duration_since(beat) < STALL => {
                1. - (-now.duration_since(beat).as_secs_f32() / SETTLE.as_secs_f32()).exp()
            }
            _ => 1.,
        };
        ease(&mut self.shown.left, &self.target.left, rate);
        ease(&mut self.shown.right, &self.target.right, rate);
        self.shown.clone()
    }

    /// Picks new targets: a random height per band under its lean, the channels sharing most of
    /// their randomness, each smoothed across its neighbours so the wave rolls rather than
    /// spikes.
    fn pick(&mut self) {
        let swell = SWELL + (1. - SWELL) * self.next();
        let mut left = Vec::with_capacity(BANDS);
        let mut right = Vec::with_capacity(BANDS);
        for band in 0..BANDS {
            let lean = TILT[0] + (TILT[1] - TILT[0]) * band as f32 / (BANDS - 1) as f32;
            let shared = self.next();
            let (own_left, own_right) = (self.next(), self.next());
            let level = |own: f32| {
                lean * swell * (LOW + (1. - LOW) * (shared * SHARED + own * (1. - SHARED)))
            };
            left.push(level(own_left));
            right.push(level(own_right));
        }
        self.target = Levels {
            left: smooth(&left),
            right: smooth(&right),
        };
    }

    /// The next number in `0..1` from an xorshift over `seed`.
    fn next(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// The Now playing page's preview, a miniature of the Now playing view for the track playing,
/// or for one from history while nothing is loaded.
pub(in crate::screens::settings) fn preview(
    state: &NowPlayingPreview,
    view: &SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Option<AnyElement> {
    let width = Layout::of(window, cx).width;
    let height = (width * ASPECT).round();
    let sample = view.previews.playing_or_picked(cx);
    let cover = sample.large_cover(cx);
    let settings = view.settings.read(cx);
    let ambient = settings.ambient();
    let style = settings.visualizer_style();

    let field = ambient.then(|| view.previews.ambient(cover.clone(), cx));
    // over the field the miniature wears what fullscreen would, not the theme around it
    let theme = match &field {
        Some(field) => field.read(cx).theme(cx),
        None => *cx.theme(),
    };
    let side = (height * COVER).round();

    let still = cx.reduce_motion();
    let levels = style
        .shown()
        .then(|| state.levels.borrow_mut().step(Instant::now(), still));
    if levels.is_some() && !still && !state.armed.replace(true) {
        cx.on_next_frame(window, |this, _, cx| {
            this.previews.now_playing.armed.set(false);
            cx.notify();
        });
    }
    // the peaks stop short of the artist line as in fullscreen, by a pad to suit the miniature
    let bottom = |bounds: Bounds<Pixels>| bounds.origin.y + bounds.size.height;
    let reach = (bottom(state.frame.get()) - bottom(state.meta.get()) - theme.metrics.pad)
        .max(Pixels::ZERO);
    let tint = match &field {
        Some(_) => theme
            .tint
            .map_or(theme.primary, |tint| theme.primary_of(tint)),
        None => theme.primary,
    };
    let owner = cx.entity_id();

    Some(
        super::framed(
            div()
                .relative()
                .w_full()
                .h(height)
                .overflow_hidden()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(measure(state.frame.clone(), owner))
                .children(field)
                .when_some(levels, |this, levels| {
                    this.child(
                        Visualizer::new(levels, reach)
                            .style_kind(style)
                            .tint(tint)
                            .behind(theme.background)
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0(),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .child(Artwork::new(cover).size(side).corner_radius(theme.radius))
                        .child(
                            div()
                                .relative()
                                .mt_2()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(measure(state.meta.clone(), owner))
                                .child(
                                    div()
                                        .max_w(width * META)
                                        .truncate()
                                        .text_size(theme.text(Text::Body))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(sample.title.clone()),
                                )
                                .child(
                                    div()
                                        .max_w(width * META)
                                        .truncate()
                                        .text_size(theme.text(Text::Small))
                                        .text_color(theme.muted_foreground)
                                        .child(sample.artist.clone()),
                                ),
                        ),
                ),
            cx,
        )
        .into_any_element(),
    )
}

/// Fills its parent and keeps the parent's bounds in `cell`, redrawing `owner` next frame when
/// they move.
fn measure(cell: Rc<Cell<Bounds<Pixels>>>, owner: EntityId) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            if cell.replace(bounds) != bounds {
                window.on_next_frame(move |_, cx| cx.notify(owner));
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// Walks `shown` a share of the way towards `target`, adopting it outright when the band count
/// changes.
fn ease(shown: &mut Vec<f32>, target: &[f32], rate: f32) {
    if shown.len() != target.len() {
        *shown = target.to_vec();
        return;
    }
    for (shown, target) in shown.iter_mut().zip(target) {
        *shown += (target - *shown) * rate;
    }
}

/// Each band averaged with its neighbours, weighted towards itself.
fn smooth(bands: &[f32]) -> Vec<f32> {
    (0..bands.len())
        .map(|index| {
            let before = bands[index.saturating_sub(1)];
            let after = bands[(index + 1).min(bands.len() - 1)];
            (before + bands[index] * 4. + after) / 6.
        })
        .collect()
}
