use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div, svg};
use i18n::t;
use music::output::{Format, Sample};
use music::{Codec, Encoding, Normalisation};
use state::{Change, SignalPath};
use ui::{ActiveTheme as _, Button, Fact, InfoCard, Text};

use super::EqualizerView;
use super::outputs::device_name;
use super::rows::figure;

/// What joins the parts of one value, as the cards and the aside do.
const BULLET: &str = " · ";

impl EqualizerView {
    /// The button that opens and closes the signal path.
    pub(super) fn path_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.path.is_some();
        Button::new("equalizer-signal-path")
            .small()
            .outline()
            .selected(open)
            .label(t!("equalizer-signal-path"))
            .trailing(match open {
                true => "icons/chevron-up.svg",
                false => "icons/chevron-down.svg",
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_path(cx)))
            .into_any_element()
    }

    /// Reads the signal path while it is open, and stops when it closes.
    fn toggle_path(&mut self, cx: &mut Context<Self>) {
        self.path = match self.path.take() {
            Some(_) => None,
            None => Some(self.signal.update(cx, |signal, cx| signal.watch_path(cx))),
        };
        cx.notify();
    }

    /// What happens to the sound from the track to the device, stage by stage, and whether
    /// what leaves Sonora is the decoded track bit for bit.
    pub(super) fn signal_path(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.path.as_ref()?;
        let theme = *cx.theme();
        let card = InfoCard::new(t!("equalizer-signal-path"));
        let Some(path) = self.signal.read(cx).path().cloned() else {
            return Some(
                card.child(
                    div()
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground)
                        .child(t!("signal-path-waiting")),
                )
                .into_any_element(),
            );
        };
        let source = path.source.as_ref().map_or_else(
            || t!("signal-path-waiting"),
            |source| {
                let mut parts: Vec<SharedString> =
                    source.encoding.map(encoding).unwrap_or_default();
                parts.push(rate(source.rate));
                parts.push(t!("signal-path-channels", channels = source.channels));
                join(parts)
            },
        );
        let normalisation = match path.normalisation {
            Normalisation::Off => t!("signal-path-off"),
            Normalisation::Gain(db) => t!("signal-path-gain", db = signed(f64::from(db))),
            Normalisation::Limited => t!("signal-path-limited"),
        };
        let equalizer = match path.equalizer {
            None => t!("signal-path-off"),
            Some(eq) if eq.preamp_db == 0. && eq.passes_through => {
                t!("signal-path-equalizer-untouched")
            }
            Some(eq) => t!("signal-path-equalizer-preamp", db = signed(eq.preamp_db)),
        };
        let volume = match path.volume == 1. {
            true => t!("signal-path-full-volume"),
            false => t!(
                "signal-path-volume-level",
                percent = (path.level * 100.).round(),
                db = figure(f64::from(20. * path.volume.log10()), 1)
            ),
        };
        let sent = path.sent.map_or_else(|| t!("signal-path-waiting"), format);
        let server = match &path.device {
            Some(device) => match device.format {
                Some(at) => join(vec![device_name(device), format(at)]),
                None => device_name(device),
            },
            None => t!("signal-path-server-unknown"),
        };
        let rows = [
            (t!("signal-path-source"), source),
            (t!("signal-path-normalisation"), normalisation),
            (t!("signal-path-equalizer"), equalizer),
            (t!("signal-path-volume"), volume),
            (t!("signal-path-sent"), sent),
            (t!("signal-path-server"), server),
        ];
        Some(
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows.into_iter().enumerate().map(|(index, (label, value))| {
                        Fact::new(label, value).striped(index % 2 == 0)
                    })),
            )
            .child(verdict(&path, cx))
            .children(path.server_rate().map(|rate| {
                div()
                    .px(theme.metrics.pad)
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground)
                    .child(t!("signal-path-server-converts", khz = khz(rate)))
            }))
            .into_any_element(),
        )
    }
}

/// Bit-perfect from Sonora, or what keeps it from being.
fn verdict(path: &SignalPath, cx: &mut Context<EqualizerView>) -> AnyElement {
    let theme = *cx.theme();
    let changes = path.changes();
    let size = theme.text(Text::Small);
    let (icon, color, text) = match changes.is_empty() {
        true => (
            "icons/circle-check.svg",
            theme.primary,
            t!("signal-path-bit-perfect"),
        ),
        false => (
            "icons/circle-alert.svg",
            theme.muted_foreground,
            t!(
                "signal-path-altered",
                changes = changes
                    .into_iter()
                    .map(change)
                    .collect::<Vec<_>>()
                    .join(&t!("signal-path-list-separator"))
            ),
        ),
    };
    div()
        .flex()
        .items_start()
        .gap_2()
        .px(theme.metrics.pad)
        .text_size(size)
        .child(
            svg()
                .path(icons::path(icon))
                .size(size)
                .flex_none()
                .text_color(color),
        )
        .child(div().flex_1().min_w_0().child(text))
        .into_any_element()
}

fn change(change: Change) -> SharedString {
    match change {
        Change::Rounded { from } => t!("signal-path-change-rounded", bits = from),
        Change::Normalised(db) => {
            t!("signal-path-change-normalised", db = signed(f64::from(db)))
        }
        Change::Limited => t!("signal-path-change-limited"),
        Change::Equalized => t!("signal-path-change-equalized"),
        Change::Volume(_) => t!("signal-path-change-volume"),
        Change::Resampled { from, to } => t!(
            "signal-path-change-resampled",
            from = khz(from),
            to = khz(to)
        ),
        Change::Remixed { from, to } => t!("signal-path-change-remixed", from = from, to = to),
        Change::Requantized { to } => t!("signal-path-change-requantized", bits = to),
    }
}

/// The codec and, for a lossless one, its bit depth or, for a lossy one, its bit rate.
fn encoding(encoding: Encoding) -> Vec<SharedString> {
    let codec = match encoding.codec {
        Codec::Flac => t!("signal-path-codec-flac"),
        Codec::Alac => t!("signal-path-codec-alac"),
        Codec::Pcm => t!("signal-path-codec-pcm"),
        Codec::Mp3 => t!("signal-path-codec-mp3"),
        Codec::Aac => t!("signal-path-codec-aac"),
        Codec::Vorbis => t!("signal-path-codec-vorbis"),
        Codec::Opus => t!("signal-path-codec-opus"),
        Codec::Other => t!("signal-path-codec-other"),
    };
    let detail = match (encoding.bits, encoding.kbps) {
        (Some(bits), _) => Some(t!("signal-path-bits", bits = bits)),
        (None, Some(kbps)) => Some(t!("signal-path-kbps", kbps = kbps)),
        (None, None) => None,
    };
    std::iter::once(codec).chain(detail).collect()
}

/// A rate, a channel count and a sample format.
fn format(format: Format) -> SharedString {
    let sample = match format.sample {
        Sample::Integer(bits) => Some(t!("signal-path-bits", bits = bits)),
        Sample::Float(bits) => Some(t!("signal-path-float", bits = bits)),
        Sample::Other => None,
    };
    let parts = [
        Some(rate(format.rate)),
        Some(t!("signal-path-channels", channels = format.channels)),
        sample,
    ];
    join(parts.into_iter().flatten().collect())
}

/// Decibels to a tenth, a boost marked with a plus.
fn signed(db: f64) -> String {
    match figure(db, 1) {
        text if db > 0. && text != "0" => format!("+{text}"),
        text => text,
    }
}

fn rate(rate: u32) -> SharedString {
    t!("signal-path-rate", khz = khz(rate))
}

/// A rate in kilohertz, 44.1 rather than 44.100.
fn khz(rate: u32) -> f64 {
    f64::from(rate) / 1000.
}

fn join(parts: Vec<SharedString>) -> SharedString {
    parts
        .iter()
        .map(SharedString::as_ref)
        .collect::<Vec<_>>()
        .join(BULLET)
        .into()
}
