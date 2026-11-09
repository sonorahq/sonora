//! A miniature of the Now playing view: the ambient field, the visualizer, artwork, title and
//! artist. The visualizer draws random levels rather than the spectrum, so it moves whether or
//! not anything plays. The levels come from a ticker thread while the miniature is on screen
//! and reach the bars through the visualizer's live values, so the view never runs for them.

use std::time::{Duration, Instant};

use rune::{Cx, El, Transition, Tx, col, el, text};
use ui::{Cover, LEADING, Levels, Text, Visualizer, Weight, font};

use super::{Ticker, framed, playing_or_picked};
use crate::Ui;
use crate::screens::settings::Layout;

/// How tall the miniature stands for its width, the shape of a window.
const ASPECT: f32 = 0.5;
/// How much of the miniature's height the cover takes.
const COVER: f32 = 0.46;
/// How much of the miniature's width the title and artist may take.
const META: f32 = 0.6;
/// The space between the cover and the words, and between the two lines.
const META_TOP: f32 = 12.;
const LINE_GAP: f32 = 4.;
/// How many bands the random levels carry, as many as the analyzer reports.
const BANDS: usize = 32;
/// How often the levels move on, and how often they pick new heights to move towards.
const FRAME: Duration = Duration::from_millis(16);
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

/// What the Now playing page's preview keeps: the visualizer the levels are pushed into and the
/// thread that pushes them while the miniature shows.
pub(in crate::screens::settings) struct State {
    visualizer: Visualizer,
    ticker: Ticker,
}

impl Default for State {
    fn default() -> Self {
        Self {
            visualizer: Visualizer::new(BANDS),
            ticker: Ticker::default(),
        }
    }
}

impl State {
    /// Stops the levels, for a miniature that left the screen.
    pub(in crate::screens::settings) fn hide(&mut self) {
        self.ticker.stop();
    }

    /// Starts the levels moving, or with `still` set draws one pick and holds it.
    fn show(&mut self, still: bool) {
        let visualizer = self.visualizer.clone();
        let mut wander = Wander::new(super::random());
        match still {
            true => visualizer.push(&wander.step(Instant::now(), true)),
            false => self.ticker.start(FRAME, move || {
                visualizer.push(&wander.step(Instant::now(), false))
            }),
        }
    }
}

/// Random levels that ease towards a fresh pick every `PICK`, standing in for the spectrum.
struct Wander {
    shown: Levels,
    target: Levels,
    seed: u64,
    picked: Option<Instant>,
    beat: Option<Instant>,
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

/// The Now playing page's preview, a miniature of the Now playing view for the track playing, or
/// for one from history while nothing is loaded. The cover cross-fades as the track changes.
pub(in crate::screens::settings) fn view(ui: &Ui, cx: &mut Cx<Ui>, layout: &Layout) -> El {
    let width = layout.width;
    let height = (width * ASPECT).round();
    let sample = playing_or_picked(ui);
    let cover = sample.large_cover(&ui.store);
    let settings = &ui.store.settings;
    let style = settings.visualizer_style();
    let still = settings.stillness().still().unwrap_or(false) || cx.system_reduced_motion();

    // over the field the miniature wears what fullscreen would, not the theme around it
    let (theme, field) = match settings.ambient() {
        true => super::field(ui, cover.as_deref(), width),
        false => (
            ui.theme,
            el().key("field")
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .fill(ui.theme.background.color()),
        ),
    };
    let side = (height * COVER).round();
    // the peaks stop short of the artist line as in fullscreen, by a pad to suit the miniature
    let words = theme.text(Text::Body) * LEADING + LINE_GAP + theme.text(Text::Small) * LEADING;
    let content = side + META_TOP + words;
    let reach = (height - (height + content) / 2. - theme.metrics.pad).max(0.);
    let tint = match theme.tint {
        Some(tint) if settings.ambient() => theme.primary_of(tint),
        _ => theme.primary,
    };

    let mut frame = el()
        .key("now-playing-miniature")
        .w_full()
        .h(height)
        .color(theme.foreground.color())
        .on_visible(move |ui: &mut Ui, shown: bool, _: &mut Tx| {
            let state = &mut ui.screens.settings.previews.now_playing;
            match shown {
                true => state.show(still),
                false => state.hide(),
            }
        })
        .child(field);
    if style.shown() {
        frame = frame.child(
            ui.screens
                .settings
                .previews
                .now_playing
                .visualizer
                .view(width, reach)
                .style(style)
                .tint(tint)
                .behind(theme.background)
                .build(&theme)
                .absolute(0., height - reach)
                .transition(Transition::opacity()),
        );
    }
    let meta_width = width * META;
    frame = frame.child(
        col()
            .key("now-playing-content")
            .absolute(0., 0.)
            .w_full()
            .h_full()
            .center()
            .child(
                Cover::new(ui.artwork(), cover.as_deref())
                    .size(side)
                    .radius(theme.radius)
                    .fallback("icons/disc-3.svg")
                    .build(&theme),
            )
            .child(
                col()
                    .key("now-playing-meta")
                    .mt(META_TOP)
                    .gap(LINE_GAP)
                    .items_center()
                    .child(
                        text(sample.title.clone())
                            .key(format!("title-{}", sample.title))
                            .font(font(&theme, Text::Body, Weight::SEMIBOLD))
                            .color(theme.foreground.color())
                            .max_w(meta_width)
                            .truncate()
                            .transition(Transition::blur_replace()),
                    )
                    .child(
                        text(sample.artist.clone())
                            .key(format!("artist-{}", sample.artist))
                            .font(font(&theme, Text::Small, Weight::NORMAL))
                            .color(theme.muted_foreground.color())
                            .max_w(meta_width)
                            .truncate()
                            .transition(Transition::blur_replace()),
                    ),
            ),
    );
    framed("now-playing-preview", frame.a11y_hidden(), &ui.theme)
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
