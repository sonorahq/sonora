//! The strip under the graph: the output's level meter, drawn live from the window's poll, then
//! the clip warning and its fix while the output overloads, and at the far end whether what
//! leaves Sonora is bit-perfect.

use std::cell::Cell;

use i18n::{SharedString, t};
use rune::{Color, El, Live, Spring, el, icon, row, text};
use state::{FLOAT_BITS, SignalChange, SignalPath, Store};
use ui::{Button, Perch, Text, Theme, Tipped as _, Weight, font};

use super::rows::figure;
use crate::shared::page::rems;
use crate::{Ui, act};

/// Below this an overload rounds to no tenth of a decibel at all.
const SLIGHT_OVERLOAD: f64 = 0.05;
/// How wide the level meter is, in cover sizes.
const METER_WIDTH: f32 = 1.2;
/// How many changes the status names inline before it counts the rest.
const INLINE: usize = 2;
/// The levels the meter spans, in dBFS, its left edge first, as `ui::LevelMeter` draws them.
/// Full scale sits near the right, so the clip zone past it is a narrow band.
const RANGE: (f32, f32) = (-48., 6.);
/// How tall the bar is, and how wide the peak tick.
const THICKNESS: f32 = 4.;
const TICK: f32 = 2.;
/// How strongly the clip zone tints the track where the level does not reach.
const CLIP_ZONE: f32 = 0.25;
const ROUND: f32 = 9999.;
/// How quickly the fill and the tick follow each read, quick enough to keep up with the music.
const FOLLOW: Spring = Spring::new(0.08, 0.);

/// The live parts of the level meter: the fill up to the level, the part of it past full scale,
/// and the held peak's tick with its colour. A read moves them without running the view.
pub(super) struct Meter {
    fill: Live<[f32; 4]>,
    over: Live<[f32; 4]>,
    tick: Live<[f32; 4]>,
    tick_color: Live<Color>,
    /// The meter's width and the tick's two colours as the last view drew them.
    drawn: Cell<(f32, Color, Color)>,
    /// The level and the peak last drawn, so a still meter costs nothing.
    last: Option<(f32, f32)>,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            fill: Live::new([0., 0., 0., THICKNESS]),
            over: Live::new([0., 0., 0., THICKNESS]),
            tick: Live::new([0., 0., 0., THICKNESS]),
            tick_color: Live::new(Color::TRANSPARENT),
            drawn: Cell::new((0., Color::TRANSPARENT, Color::TRANSPARENT)),
            last: None,
        }
    }
}

impl Meter {
    /// Moves the meter to `level` with its held `peak`, in dBFS.
    pub(super) fn feed(&mut self, (level, peak): (f32, f32)) {
        if self.last == Some((level, peak)) {
            return;
        }
        self.last = Some((level, peak));
        self.draw();
    }

    /// Starts the meter afresh from silence, for a watch that starts afresh.
    pub(super) fn reset(&mut self) {
        self.last = None;
        self.draw();
    }

    fn draw(&self) {
        let (width, foreground, danger) = self.drawn.get();
        let (level, peak) = self.last.unwrap_or((RANGE.0, RANGE.0));
        let full = share(0.);
        let level = share(level);
        let shown = level.min(full);
        let over = (level - full).max(0.);
        self.fill.set([0., 0., width * shown, THICKNESS]);
        self.over.set([width * full, 0., width * over, THICKNESS]);
        let peak = share(peak);
        let tick = match peak > 0. {
            true => [width * peak - TICK / 2., 0., TICK, THICKNESS],
            false => [0., 0., 0., THICKNESS],
        };
        self.tick.set(tick);
        self.tick_color.set(match peak > full {
            true => danger,
            false => foreground,
        });
    }

    /// The meter itself: the track, the clip zone past full scale, and the live fill and tick.
    fn view(&self, theme: &Theme) -> El {
        let width = theme.metrics.cover * METER_WIDTH;
        let drawn = (width, theme.foreground.color(), theme.danger.color());
        if self.drawn.replace(drawn) != drawn {
            self.draw();
        }
        let full = share(0.);
        let bar = |key: &str, live: &Live<[f32; 4]>, color: Color| {
            el().key(key)
                .absolute(0., 0.)
                .live_rect(live.smooth(FOLLOW))
                .pixel_snap(false)
                .fill(color)
        };
        el().key("equalizer-level-meter")
            .w(width)
            .h(THICKNESS)
            .radius(ROUND)
            .clip_rounded(ROUND)
            .fill(theme.muted.color())
            .child(
                el().key("clip-zone")
                    .absolute(width * full, 0.)
                    .size(width * (1. - full), THICKNESS)
                    .fill(theme.danger.opacity(CLIP_ZONE).color()),
            )
            .child(bar("fill", &self.fill, theme.primary.color()))
            .child(bar("over", &self.over, theme.danger.color()))
            .child(
                el().key("peak")
                    .absolute(0., 0.)
                    .live_rect(self.tick.smooth(FOLLOW))
                    .pixel_snap(false)
                    .live_fill(&self.tick_color),
            )
            .a11y_hidden()
    }
}

/// Whether the sound leaves Sonora as the decoded track bit for bit, as the strip says it.
enum Status {
    /// Nothing is loaded, or the engine has not opened the track yet.
    Idle,
    /// Bit-perfect from Sonora, with the rate the sound server converts it to afterwards.
    Perfect { server: Option<u32> },
    /// What changes the sound, in the order it happens.
    Altered(Vec<SignalChange>),
}

/// The strip under the graph, as tall as a small control so the warning and the status coming
/// and going never move the page.
pub(super) fn view(ui: &Ui) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.equalizer;
    let equalizer = &ui.store.equalizer;
    let small = font(theme, Text::Small, Weight::NORMAL);
    let mut strip = row()
        .key("equalizer-meter")
        .w_full()
        .h(theme.metrics.control_small)
        .mt(rems(theme, 0.75))
        .center_y()
        .gap(rems(theme, 0.75))
        .clip_rounded(0.)
        .child(
            text(t!("equalizer-level"))
                .key("equalizer-level-label")
                .font(small)
                .color(theme.muted_foreground.color()),
        )
        .child(state.meter.view(theme));
    if let Some(db) = equalizer.overload() {
        let fixable = equalizer.enabled() && equalizer.lowered_preamp().is_some();
        let warning = match db < SLIGHT_OVERLOAD {
            true => t!("equalizer-overload-slight"),
            false => t!("equalizer-overload", db = figure(db, 1)),
        };
        strip = strip
            .child(
                icon(icons::get("icons/triangle-alert.svg"), small.size)
                    .key("equalizer-overload-icon")
                    .color(theme.danger.color())
                    .transition(rune::Transition::opacity())
                    .a11y_hidden(),
            )
            .child(
                ui::spoken("equalizer-overload", warning, theme)
                    .font(font(theme, Text::Small, Weight::MEDIUM))
                    .color(theme.danger.color())
                    .shrink()
                    .min_w(0.)
                    .truncate()
                    .transition(rune::Transition::opacity()),
            );
        if fixable {
            strip = strip.child(
                Button::new("equalizer-lower-preamp")
                    .small()
                    .outline()
                    .label(t!("equalizer-lower-preamp"))
                    .on_press(act(|ui: &mut Ui| {
                        let Store {
                            equalizer,
                            settings,
                            cx,
                            ..
                        } = &mut ui.store;
                        equalizer.lower_preamp(settings, cx);
                    }))
                    .build(theme)
                    .transition(rune::Transition::opacity()),
            );
        }
    }
    let status = status(ui.store.signal.path());
    // the status keeps to the strip's far end, and gives up its width before the warning does
    strip
        .child(el().key("equalizer-meter-gap").grow(1.).min_w(0.))
        .children(status_element(theme, &status))
}

/// The icon and short verdict at the strip's end, with every change in its tooltip. Nothing
/// while idle.
fn status_element(theme: &Theme, status: &Status) -> Option<El> {
    let size = theme.text(Text::Small);
    let (glyph, color, words, tip) = match status {
        Status::Idle => return None,
        Status::Perfect { server } => (
            "icons/circle-check.svg",
            theme.primary,
            t!("equalizer-status-bit-perfect").into_owned(),
            lines(
                std::iter::once(t!("equalizer-status-bit-perfect-detail"))
                    .chain(server.map(|rate| t!("equalizer-status-server", khz = khz(rate)))),
            ),
        ),
        Status::Altered(changes) => {
            let labels: Vec<SharedString> = changes.iter().copied().map(label).collect();
            let mut words = labels
                .iter()
                .take(INLINE)
                .map(AsRef::as_ref)
                .collect::<Vec<&str>>()
                .join(&t!("equalizer-status-separator"));
            if labels.len() > INLINE {
                words.push(' ');
                words.push_str(&t!("equalizer-status-more", count = labels.len() - INLINE));
            }
            (
                "icons/info.svg",
                theme.muted_foreground,
                words,
                lines(std::iter::once(t!("equalizer-status-altered")).chain(labels)),
            )
        }
    };
    Some(
        row()
            .key("equalizer-signal-status")
            .role("text")
            .label(words.clone())
            .description(tip.clone())
            .min_w(0.)
            .shrink()
            .center_y()
            .gap(rems(theme, 0.375))
            .transition(rune::Transition::opacity())
            .child(
                icon(icons::get(glyph), size)
                    .color(color.color())
                    .a11y_hidden(),
            )
            .child(
                text(words)
                    .font(font(theme, Text::Small, Weight::NORMAL))
                    .color(color.color())
                    .shrink()
                    .min_w(0.)
                    .truncate()
                    .a11y_hidden(),
            )
            .tip_label(tip, Perch::Below),
    )
}

/// What the strip says of the signal path right now.
fn status(path: Option<&SignalPath>) -> Status {
    let Some(path) = path.filter(|path| path.track.is_some()) else {
        return Status::Idle;
    };
    let changes = path.changes();
    match changes.is_empty() {
        true => Status::Perfect {
            server: path.server_rate(),
        },
        false => Status::Altered(changes),
    }
}

fn label(change: SignalChange) -> SharedString {
    match change {
        SignalChange::Rounded { from } => {
            t!("equalizer-status-bits", from = from, to = FLOAT_BITS)
        }
        SignalChange::Normalised(_) | SignalChange::Limited => t!("equalizer-status-normalized"),
        SignalChange::Resampled { from, to } => {
            t!("equalizer-status-rate", from = khz(from), to = khz(to))
        }
        SignalChange::Equalized => t!("equalizer-status-equalized"),
        SignalChange::Volume(_) => t!("equalizer-status-volume"),
        SignalChange::Remixed { from, to } => {
            t!("equalizer-status-channels", from = from, to = to)
        }
        SignalChange::Requantized {
            from: Some(from),
            to,
        } => t!("equalizer-status-bits", from = from, to = to),
        SignalChange::Requantized { from: None, to } => t!("equalizer-status-float", to = to),
    }
}

/// A rate in kilohertz, 44.1 rather than 44.100.
fn khz(rate: u32) -> f64 {
    f64::from(rate) / 1000.
}

/// One tooltip line per item.
fn lines(items: impl IntoIterator<Item = SharedString>) -> String {
    items
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>()
        .join("\n")
}

/// How far along the meter `db` sits, from 0 at its left edge to 1 at its right.
fn share(db: f32) -> f32 {
    ((db - RANGE.0) / (RANGE.1 - RANGE.0)).clamp(0., 1.)
}
