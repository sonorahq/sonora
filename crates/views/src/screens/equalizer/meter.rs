use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, FontWeight, Render, SharedString, Window, div, svg};
use i18n::t;
use state::{Change, Equalizer, FLOAT_BITS, OutputLevel, Signal};
use ui::{ActiveTheme as _, Button, LevelMeter, Perch, Text, Tipped as _};

use super::rows::figure;

/// Below this an overload rounds to no tenth of a decibel at all.
const SLIGHT_OVERLOAD: f64 = 0.05;
/// How wide the level meter is, in cover sizes.
const METER_WIDTH: f32 = 1.2;
/// How many changes the status names inline before it counts the rest.
const INLINE: usize = 2;
/// How much more readily the status gives up its width than the clip warning does.
const STATUS_SHRINK: f32 = 1000.;

/// The strip under the graph: the output's level meter, then the clip warning and its fix while
/// the output overloads, and at the far end whether what leaves Sonora is bit-perfect. The screen
/// places it as a cached view at a fixed height, so the meter's reads and the warning and status
/// coming and going redraw only the strip and never move the page.
pub(super) struct OutputMeter {
    equalizer: Entity<Equalizer>,
    level: Entity<OutputLevel>,
    /// The warning as last drawn, so an equalizer notify that left it as it was draws nothing.
    warning: Option<Warning>,
    /// The status as last drawn, so the signal's spectrum ticks that leave it as it was draw
    /// nothing.
    status: Status,
}

/// The clip warning as drawn: the over in tenths of a decibel, and whether lowering the preamp
/// is offered.
#[derive(Clone, Copy, PartialEq)]
struct Warning {
    db: f64,
    fixable: bool,
}

/// Whether the sound leaves Sonora as the decoded track bit for bit, as the strip says it.
#[derive(Clone, PartialEq)]
enum Status {
    /// Nothing is loaded, or the engine has not opened the track yet.
    Idle,
    /// Bit-perfect from Sonora, with the rate the sound server converts it to afterwards.
    Perfect { server: Option<u32> },
    /// What changes the sound, in the order it happens.
    Altered(Vec<Alter>),
}

/// One thing that changes the sound, holding only what its short label shows.
#[derive(Clone, Copy, PartialEq)]
enum Alter {
    Rounded(u8),
    Normalized,
    Resampled(u32, u32),
    Equalized,
    Volume,
    Remixed(u16, u16),
    /// The bits Sonora carries, `None` for a track without a bit depth, and the device's.
    Requantized(Option<u8>, u8),
}

impl OutputMeter {
    pub(super) fn new(
        equalizer: Entity<Equalizer>,
        signal: Entity<Signal>,
        cx: &mut Context<Self>,
    ) -> Self {
        let level = equalizer.read(cx).output_level();
        cx.observe(&level, |_, _, cx| cx.notify()).detach();
        cx.observe(&equalizer, |this, _, cx| {
            let warning = this.warning(cx);
            if warning != this.warning {
                this.warning = warning;
                cx.notify();
            }
        })
        .detach();
        cx.observe(&signal, |this, signal, cx| {
            let status = status(signal.read(cx));
            if status != this.status {
                this.status = status;
                cx.notify();
            }
        })
        .detach();
        let mut meter = Self {
            equalizer,
            level,
            status: status(signal.read(cx)),
            warning: None,
        };
        meter.warning = meter.warning(cx);
        meter
    }

    /// The warning the equalizer asks for right now, with the fix left out when it is off or
    /// the preamp is as low as it goes.
    fn warning(&self, cx: &Context<Self>) -> Option<Warning> {
        let equalizer = self.equalizer.read(cx);
        Some(Warning {
            db: equalizer.overload()?,
            fixable: equalizer.enabled() && equalizer.lowered_preamp().is_some(),
        })
    }

    /// The icon and short verdict at the strip's end, with every change in its tooltip.
    /// Nothing while idle.
    fn status_element(&self, cx: &App) -> Option<AnyElement> {
        let theme = cx.theme();
        let size = theme.text(Text::Small);
        let (icon, color, text, tip) = match &self.status {
            Status::Idle => return None,
            Status::Perfect { server } => (
                "icons/circle-check.svg",
                theme.primary,
                t!("equalizer-status-bit-perfect"),
                lines(
                    std::iter::once(t!("equalizer-status-bit-perfect-detail"))
                        .chain(server.map(|rate| t!("equalizer-status-server", khz = khz(rate)))),
                ),
            ),
            Status::Altered(changes) => {
                let labels: Vec<SharedString> = changes.iter().copied().map(label).collect();
                let mut text = labels
                    .iter()
                    .take(INLINE)
                    .map(SharedString::as_ref)
                    .collect::<Vec<_>>()
                    .join(&t!("equalizer-status-separator"));
                if labels.len() > INLINE {
                    text.push(' ');
                    text.push_str(&t!("equalizer-status-more", count = labels.len() - INLINE));
                }
                (
                    "icons/info.svg",
                    theme.muted_foreground,
                    text.into(),
                    lines(std::iter::once(t!("equalizer-status-altered")).chain(labels)),
                )
            }
        };
        Some(
            div()
                .id("equalizer-signal-status")
                .role(Role::Label)
                .aria_value(text.clone())
                .aria_description(tip.clone())
                .ml_auto()
                .min_w_0()
                .flex_shrink(STATUS_SHRINK)
                .flex()
                .items_center()
                .gap_1p5()
                .text_color(color)
                .child(
                    svg()
                        .path(icons::path(icon))
                        .size(size)
                        .flex_none()
                        .text_color(color),
                )
                .child(div().min_w_0().truncate().child(text))
                .tip_label(tip, Perch::Pointer)
                .into_any_element(),
        )
    }
}

impl Render for OutputMeter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let level = self.level.read(cx);
        let size = theme.text(Text::Small);
        let status = self.status_element(cx);

        div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .overflow_hidden()
            .text_size(size)
            .child(
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(t!("equalizer-level")),
            )
            .child(
                LevelMeter::new(level.level_db())
                    .peak(level.peak_db())
                    .w(theme.metrics.cover * METER_WIDTH),
            )
            .when_some(self.warning, |this, warning| {
                this.child(
                    svg()
                        .path(icons::path("icons/triangle-alert.svg"))
                        .size(size)
                        .flex_none()
                        .text_color(theme.danger),
                )
                .child(
                    ui::spoken(
                        "equalizer-overload",
                        match warning.db < SLIGHT_OVERLOAD {
                            true => t!("equalizer-overload-slight"),
                            false => t!("equalizer-overload", db = figure(warning.db, 1)),
                        },
                    )
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.danger),
                )
                .when(warning.fixable, |this| {
                    this.child(
                        Button::new("equalizer-lower-preamp")
                            .small()
                            .outline()
                            .flex_none()
                            .label(t!("equalizer-lower-preamp"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.equalizer
                                    .update(cx, |equalizer, cx| equalizer.lower_preamp(cx))
                            })),
                    )
                })
            })
            .children(status)
    }
}

impl From<Change> for Alter {
    fn from(change: Change) -> Self {
        match change {
            Change::Rounded { from } => Self::Rounded(from),
            Change::Normalised(_) | Change::Limited => Self::Normalized,
            Change::Resampled { from, to } => Self::Resampled(from, to),
            Change::Equalized => Self::Equalized,
            Change::Volume(_) => Self::Volume,
            Change::Remixed { from, to } => Self::Remixed(from, to),
            Change::Requantized { from, to } => Self::Requantized(from, to),
        }
    }
}

/// What the strip says of the signal path right now.
fn status(signal: &Signal) -> Status {
    let Some(path) = signal.path().filter(|path| path.track.is_some()) else {
        return Status::Idle;
    };
    let changes = path.changes();
    match changes.is_empty() {
        true => Status::Perfect {
            server: path.server_rate(),
        },
        false => Status::Altered(changes.into_iter().map(Alter::from).collect()),
    }
}

fn label(alter: Alter) -> SharedString {
    match alter {
        Alter::Rounded(from) => t!("equalizer-status-bits", from = from, to = FLOAT_BITS),
        Alter::Normalized => t!("equalizer-status-normalized"),
        Alter::Resampled(from, to) => {
            t!("equalizer-status-rate", from = khz(from), to = khz(to))
        }
        Alter::Equalized => t!("equalizer-status-equalized"),
        Alter::Volume => t!("equalizer-status-volume"),
        Alter::Remixed(from, to) => t!("equalizer-status-channels", from = from, to = to),
        Alter::Requantized(Some(from), to) => t!("equalizer-status-bits", from = from, to = to),
        Alter::Requantized(None, to) => t!("equalizer-status-float", to = to),
    }
}

/// A rate in kilohertz, 44.1 rather than 44.100.
fn khz(rate: u32) -> f64 {
    f64::from(rate) / 1000.
}

/// One tooltip line per item.
fn lines(items: impl IntoIterator<Item = SharedString>) -> SharedString {
    items
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>()
        .join("\n")
        .into()
}
