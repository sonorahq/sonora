//! The path the sound takes out of Sonora, drawn as a row of small steps from the source to the
//! device. Only what Sonora really does and can report has a step: the track's source and rate,
//! loudness normalization, the equalizer, resampling, and the device it plays on.

use std::cell::RefCell;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, Pixels, SharedString, Stateful, Window, div, px,
    relative, svg,
};
use i18n::t;
use music::{Encoding, Normalisation};
use state::{Following, SignalPath};
use ui::{ActiveTheme as _, Text, tabular};

use crate::screens::settings::SettingsView;
use crate::shared::equalizer::sound_name;
use crate::shared::{provider_brand, provider_logo};

/// How large a step's icon draws, and the chevron between two steps.
const ICON: Pixels = px(16.);
const CHEVRON: Pixels = px(14.);
/// How strong the chevrons between steps are against the muted text.
const CHEVRON_STRENGTH: f32 = 0.6;

/// What the Sound page's preview keeps between frames.
pub(in crate::screens::settings) struct SoundPreview {
    /// Keeps the signal path read while the preview is on show.
    following: RefCell<Option<Following>>,
    /// The path as last drawn, so the signal's other news redraws nothing.
    path: RefCell<Option<SignalPath>>,
}

/// One step of the path: its icon and its colour, what it is, and a figure under that.
struct Step {
    icon: &'static str,
    tint: Option<Hsla>,
    label: SharedString,
    detail: Option<SharedString>,
}

impl SoundPreview {
    pub(in crate::screens::settings) fn new(cx: &mut Context<SettingsView>) -> Self {
        let signal = state::Sonora::global(cx).signal.clone();
        cx.observe(&signal, |this, signal, cx| {
            let path = signal.read(cx).path().cloned();
            let preview = &this.previews.sound;
            if preview.following.borrow().is_none() {
                return;
            }
            if *preview.path.borrow() != path {
                preview.path.replace(path);
                cx.notify();
            }
        })
        .detach();

        Self {
            following: RefCell::new(None),
            path: RefCell::new(None),
        }
    }

    /// Stops reading the signal path for the preview.
    pub(in crate::screens::settings) fn hide(&mut self) {
        self.following.get_mut().take();
    }
}

/// The Sound page's preview: the source, then each thing Sonora does to the sound on its way,
/// then the device, with a mark when what leaves Sonora is the decoded track bit for bit.
pub(in crate::screens::settings) fn preview(
    state: &SoundPreview,
    view: &SettingsView,
    _window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Option<AnyElement> {
    if state.following.borrow().is_none() {
        let signal = state::Sonora::global(cx).signal.clone();
        let following = signal.update(cx, |signal, cx| signal.follow(cx));
        state.path.replace(signal.read(cx).path().cloned());
        state.following.replace(Some(following));
    }
    let theme = *cx.theme();
    let path = state.path.borrow().clone();
    let track = path.as_ref().and_then(|path| path.track.clone());

    let session = view.session.read(cx);
    let slug = view
        .playback
        .read(cx)
        .track()
        .and_then(|track| track.id.as_deref())
        .and_then(|id| session.slug_for(id))
        .or_else(|| session.provider_slug());
    let name = view
        .playback
        .read(cx)
        .track()
        .and_then(|track| track.id.as_deref())
        .and_then(|id| session.provider_for(id))
        .map(|provider| provider.name())
        .or_else(|| session.provider_name());

    let mut steps = Vec::new();
    steps.push(Step {
        icon: slug.map_or("icons/music.svg", provider_logo),
        tint: slug.map(|slug| provider_brand(slug, cx)),
        label: name.map_or_else(|| t!("settings-signal-source"), SharedString::from),
        detail: Some(track.as_ref().map_or_else(
            || t!("settings-output-idle"),
            |track| source(track.rate, track.encoding),
        )),
    });
    let normalised = match track.as_ref().map(|track| track.normalisation) {
        Some(Normalisation::Off) => None,
        Some(Normalisation::Gain(db)) => Some(Some(db)),
        Some(Normalisation::Limited) => Some(None),
        None => view.settings.read(cx).normalisation().then_some(None),
    };
    if let Some(gain) = normalised {
        steps.push(Step {
            icon: "icons/gauge.svg",
            tint: None,
            label: t!("settings-signal-normalized"),
            detail: gain.map(|db| t!("settings-signal-gain", db = signed(db))),
        });
    }
    if view.equalizer.read(cx).enabled() {
        steps.push(Step {
            icon: "icons/sliders-horizontal.svg",
            tint: None,
            label: t!("settings-equalizer"),
            detail: Some(sound_name(view.settings.read(cx))),
        });
    }
    if let Some((from, to)) = path.as_ref().and_then(SignalPath::resampled) {
        steps.push(Step {
            icon: "icons/arrow-right-left.svg",
            tint: None,
            label: t!("settings-signal-resampled"),
            detail: Some(t!("equalizer-status-rate", from = khz(from), to = khz(to))),
        });
    }
    let outputs = view.outputs.read(cx);
    let device = outputs.current();
    steps.push(Step {
        icon: "icons/speaker.svg",
        tint: None,
        label: device
            .map(|device| {
                outputs
                    .devices()
                    .iter()
                    .find(|option| option.id == device.id)
                    .map_or_else(|| device.name.clone(), |option| option.label.clone())
            })
            .map_or_else(|| t!("settings-output-device-system"), SharedString::from),
        detail: outputs.rate(cx).map(rate),
    });
    let perfect = path
        .as_ref()
        .is_some_and(|path| path.track.is_some() && path.changes().is_empty());

    let count = steps.len();
    let mut row = div()
        .id("signal-path")
        .role(Role::List)
        .aria_label(t!("settings-signal-path"))
        .flex()
        .items_center()
        .gap_2()
        .w_full();
    for (index, step) in steps.into_iter().enumerate() {
        row = row.child(node(step, (index, count), cx));
        if index + 1 < count {
            row = row.child(
                svg()
                    .path(icons::path("icons/chevron-right.svg"))
                    .size(CHEVRON)
                    .flex_none()
                    .text_color(theme.muted_foreground.opacity(CHEVRON_STRENGTH)),
            );
        }
    }

    Some(
        div()
            .relative()
            .w_full()
            .child(row)
            .when(perfect, |this| {
                this.child(
                    div()
                        .absolute()
                        // under the device step, in the space before the first group
                        .top(relative(1.))
                        .mt(theme.metrics.pad)
                        .right_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(theme.text(Text::Tiny))
                        .text_color(theme.primary)
                        .child(
                            svg()
                                .path(icons::path("icons/circle-check.svg"))
                                .size(theme.text(Text::Tiny))
                                .text_color(theme.primary),
                        )
                        .child(ui::spoken(
                            "signal-bit-perfect",
                            t!("equalizer-status-bit-perfect"),
                        )),
                )
            })
            .into_any_element(),
    )
}

/// One step as a small tile: the icon over what it is, with its figure under that. Screen readers
/// hear it as one item of the path, named by what it is and described by its figure.
fn node(step: Step, (index, count): (usize, usize), cx: &gpui::App) -> Stateful<Div> {
    let theme = cx.theme();

    div()
        .id(SharedString::from(format!("signal-step-{}", step.icon)))
        .role(Role::ListItem)
        .aria_label(step.label.clone())
        .when_some(step.detail.clone(), |this, detail| {
            this.aria_description(detail)
        })
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
        .flex_auto()
        .min_w_0()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .py_3()
        .px_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .child(
            svg()
                .path(icons::path(step.icon))
                .size(ICON)
                .flex_none()
                .text_color(step.tint.unwrap_or(theme.muted_foreground)),
        )
        .child(
            div()
                .max_w_full()
                .truncate()
                .text_size(theme.text(Text::Small))
                .font_weight(FontWeight::MEDIUM)
                .child(step.label),
        )
        .child(
            div()
                .max_w_full()
                .truncate()
                .text_size(theme.text(Text::Tiny))
                .text_color(theme.muted_foreground)
                .font_features(tabular())
                .child(step.detail.unwrap_or_else(|| "\u{a0}".into())),
        )
}

/// What the source step says of the track playing: its rate, then the bit rate a lossy stream
/// comes at or the bit depth of a lossless one, as "44.1 kHz · 256 kbps". The rate stands alone
/// when the engine reports no encoding.
fn source(rate_hz: u32, encoding: Option<Encoding>) -> SharedString {
    let quality = match encoding {
        Some(
            Encoding::Aac { kbps }
            | Encoding::Vorbis { kbps }
            | Encoding::Mp3 { kbps }
            | Encoding::Opus { kbps },
        ) => Some(t!("settings-signal-kbps", kbps = i64::from(kbps))),
        Some(Encoding::Flac { depth, .. } | Encoding::Alac { depth } | Encoding::Pcm { depth }) => {
            Some(t!("settings-signal-bits", depth = i64::from(depth)))
        }
        Some(Encoding::Other) | None => None,
    };
    match quality {
        Some(quality) => t!(
            "settings-signal-source-detail",
            rate = rate(rate_hz).to_string(),
            quality = quality.to_string()
        ),
        None => rate(rate_hz),
    }
}

/// A rate as the steps show it, 44.1 kHz.
fn rate(rate: u32) -> SharedString {
    t!("settings-signal-khz", khz = khz(rate))
}

/// A rate in kilohertz, 44.1 rather than 44.100.
fn khz(rate: u32) -> f64 {
    f64::from(rate) / 1000.
}

/// A gain in decibels with its sign, a true minus rather than a hyphen.
fn signed(db: f32) -> String {
    format!("{db:+.1}").replace('-', "\u{2212}")
}
