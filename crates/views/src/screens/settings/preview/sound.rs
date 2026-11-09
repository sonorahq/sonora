//! The path the sound takes out of Sonora, drawn as a row of small steps from the source to the
//! device. Only what Sonora really does and can report has a step: the track's source and rate,
//! loudness normalization, the equalizer, resampling, and the device it plays on. A step that
//! comes or goes as a setting changes grows in or out while the others slide to make room.

use i18n::t;
use music::{Encoding, Normalisation};
use rune::{Cx, El, Transition, Tx, col, icon, row, text};
use state::{Following, SignalPath, Store};
use ui::{Hsla, LEADING, TABULAR, Text, Theme, Weight, font};

use crate::Ui;
use crate::screens::settings::Layout;
use crate::screens::settings::accounts::{provider_brand, provider_logo};
use crate::shared::equalizer::sound_name;

/// How large a step's icon draws, and the chevron between two steps.
const ICON: f32 = 16.;
const CHEVRON: f32 = 14.;
/// How strong the chevrons between steps are against the muted text.
const CHEVRON_STRENGTH: f32 = 0.6;
const BORDER: f32 = 1.;
/// The space between the steps and their chevrons, and inside a step.
const GAP: f32 = 8.;
const STEP_GAP: f32 = 4.;
const STEP_PAD: [f32; 2] = [8., 12.];

/// What the Sound page's preview keeps: the hold that keeps the signal path read while the
/// preview is on screen.
#[derive(Default)]
pub(in crate::screens::settings) struct State {
    following: Option<Following>,
}

impl State {
    /// Stops reading the signal path for the preview.
    pub(in crate::screens::settings) fn hide(&mut self, _store: &mut Store) {
        self.following = None;
    }
}

/// One step of the path: its icon and its colour, what it is, and a figure under that.
struct Step {
    icon: &'static str,
    tint: Option<Hsla>,
    label: String,
    detail: Option<String>,
}

/// The Sound page's preview: the source, then each thing Sonora does to the sound on its way,
/// then the device, with a mark when what leaves Sonora is the decoded track bit for bit.
pub(in crate::screens::settings) fn view(ui: &Ui, _cx: &mut Cx<Ui>, _layout: &Layout) -> El {
    let theme = &ui.theme;
    let store = &ui.store;
    let path = store.signal.path();
    let track = path.and_then(|path| path.track.clone());
    let session = &store.session;
    let playing = store.playback.track().and_then(|track| track.id.as_deref());
    let slug = playing
        .and_then(|id| session.slug_for(id))
        .or_else(|| session.provider_slug());
    let name = playing
        .and_then(|id| session.provider_for(id))
        .map(|provider| provider.name())
        .or_else(|| session.provider_name());

    let mut steps = vec![Step {
        icon: slug.map_or("icons/music.svg", provider_logo),
        tint: slug.map(|slug| provider_brand(slug, theme)),
        label: name.map_or_else(|| t!("settings-signal-source").into_owned(), str::to_owned),
        detail: Some(track.as_ref().map_or_else(
            || t!("settings-output-idle").into_owned(),
            |track| source(track.rate, track.encoding),
        )),
    }];
    let normalised = match track.as_ref().map(|track| track.normalisation) {
        Some(Normalisation::Off) => None,
        Some(Normalisation::Gain(db)) => Some(Some(db)),
        Some(Normalisation::Limited) => Some(None),
        None => store.settings.normalisation().then_some(None),
    };
    if let Some(gain) = normalised {
        steps.push(Step {
            icon: "icons/gauge.svg",
            tint: None,
            label: t!("settings-signal-normalized").into_owned(),
            detail: gain.map(|db| t!("settings-signal-gain", db = signed(db)).into_owned()),
        });
    }
    if store.equalizer.enabled() {
        steps.push(Step {
            icon: "icons/sliders-horizontal.svg",
            tint: None,
            label: t!("settings-equalizer").into_owned(),
            detail: Some(sound_name(&store.settings)),
        });
    }
    if let Some((from, to)) = path.and_then(SignalPath::resampled) {
        steps.push(Step {
            icon: "icons/arrow-right-left.svg",
            tint: None,
            label: t!("settings-signal-resampled").into_owned(),
            detail: Some(t!("equalizer-status-rate", from = khz(from), to = khz(to)).into_owned()),
        });
    }
    let outputs = &store.outputs;
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
            .unwrap_or_else(|| t!("settings-output-device-system").into_owned()),
        detail: device.map(|device| rate(device.format.rate)),
    });
    let perfect = path.is_some_and(|path| path.track.is_some() && path.changes().is_empty());

    let count = steps.len();
    let mut path_row = row()
        .key("signal-path")
        .role("list")
        .label(t!("settings-signal-path"))
        .w_full()
        .center_y()
        .gap(GAP);
    for (index, step) in steps.into_iter().enumerate() {
        let key = step.icon;
        path_row = path_row.child(node(theme, step, (index, count)));
        if index + 1 < count {
            path_row = path_row.child(
                icon(icons::get("icons/chevron-right.svg"), CHEVRON)
                    .key(format!("signal-chevron-{key}"))
                    .color(theme.muted_foreground.opacity(CHEVRON_STRENGTH).color())
                    .motion(ui::Springs::BASE)
                    .transition(Transition::opacity())
                    .a11y_hidden(),
            );
        }
    }

    let mut preview = col()
        .key("sound-preview")
        .w_full()
        .on_visible(|ui: &mut Ui, shown: bool, _: &mut Tx| {
            let following = match shown {
                true => Some(ui.store.follow_signal()),
                false => None,
            };
            ui.screens.settings.previews.sound.following = following;
            ui.settle();
        })
        .child(path_row);
    if perfect {
        let tiny = theme.text(Text::Tiny);
        // under the device step, in the space before the first group
        preview = preview.child(
            row()
                .key("signal-bit-perfect")
                .pin(
                    None,
                    None,
                    Some(0.),
                    Some(-(tiny * LEADING + theme.metrics.pad)),
                )
                .center_y()
                .gap(theme.font_size * 0.25)
                .transition(Transition::offset(0., -4.).combined(Transition::opacity()))
                .child(
                    icon(icons::get("icons/circle-check.svg"), tiny)
                        .color(theme.primary.color())
                        .a11y_hidden(),
                )
                .child(
                    ui::spoken(
                        "signal-bit-perfect-label",
                        t!("equalizer-status-bit-perfect"),
                        theme,
                    )
                    .font(font(theme, Text::Tiny, Weight::NORMAL))
                    .color(theme.primary.color()),
                ),
        );
    }
    preview
}

/// One step as a small tile: the icon over what it is, with its figure under that. Screen
/// readers hear it as one item of the path, named by what it is and described by its figure.
fn node(theme: &Theme, step: Step, (index, count): (usize, usize)) -> El {
    let mut tile = col()
        .key(format!("signal-step-{}", step.icon))
        .role("listitem")
        .label(step.label.clone())
        .position_in_set(index + 1, count)
        .grow(1.)
        .shrink()
        .min_w(0.)
        .items_center()
        .gap(STEP_GAP)
        .py(STEP_PAD[1])
        .px(STEP_PAD[0])
        .radius(theme.radius)
        .border(BORDER, theme.border.color())
        .fill(theme.background.color())
        .motion(ui::Springs::BASE)
        .transition(Transition::scale(0.9).combined(Transition::opacity()))
        .child(
            icon(icons::get(step.icon), ICON)
                .color(step.tint.unwrap_or(theme.muted_foreground).color())
                .a11y_hidden(),
        )
        .child(
            text(step.label)
                .font(font(theme, Text::Small, Weight::MEDIUM))
                .color(theme.foreground.color())
                .truncate()
                .self_stretch()
                .align(rune::Align::Center)
                .a11y_hidden(),
        )
        .child(
            text(step.detail.clone().unwrap_or_else(|| "\u{a0}".to_owned()))
                .font(font(theme, Text::Tiny, Weight::NORMAL).features(TABULAR))
                .color(theme.muted_foreground.color())
                .truncate()
                .self_stretch()
                .align(rune::Align::Center)
                .a11y_hidden(),
        );
    if let Some(detail) = step.detail {
        tile = tile.description(detail);
    }
    tile
}

/// What the source step says of the track playing: its rate, then the bit rate a lossy stream
/// comes at or the bit depth of a lossless one, as "44.1 kHz · 256 kbps". The rate stands alone
/// when the engine reports no encoding.
fn source(rate_hz: u32, encoding: Option<Encoding>) -> String {
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
            rate = rate(rate_hz),
            quality = quality.into_owned()
        )
        .into_owned(),
        None => rate(rate_hz),
    }
}

/// A rate as the steps show it, 44.1 kHz.
fn rate(rate: u32) -> String {
    t!("settings-signal-khz", khz = khz(rate)).into_owned()
}

/// A rate in kilohertz, 44.1 rather than 44.100.
fn khz(rate: u32) -> f64 {
    f64::from(rate) / 1000.
}

/// A gain in decibels with its sign, a true minus rather than a hyphen.
fn signed(db: f32) -> String {
    format!("{db:+.1}").replace('-', "\u{2212}")
}
