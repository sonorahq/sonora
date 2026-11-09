//! The Sound page: the path the sound takes out of Sonora, the device it plays on with its rate
//! and buffer, and the way into the equalizer.

use i18n::t;
use router::Destination;
use rune::{Cursor, Cx, El, Transition, Tx, icon, row, text};
use state::{Outputs, Store};
use ui::{Button, Picker, Text, Theme, Weight, font};

use super::{Layout, Row, group, toggle};
use crate::shared::equalizer::sound_picker;
use crate::{Ui, act};

/// The chevron at the end of the equalizer row, as large as a button's icon.
const CHEVRON: f32 = 16.;
/// The popover key of the equalizer row's sound picker.
const SOUND: &str = "settings-equalizer-sound";
/// How wide the failure's reason may run, in covers.
const FAILURE_WIDTH: f32 = 2.;
/// How far the equalizer row reaches past the column, so its label lines up with the rows.
const REACH: f32 = 12.;

/// What changes one of the device's settings: its rate or its buffer.
type Apply = fn(&mut Outputs, u32, &mut state::AppSettings, &mut state::Cx);

/// The Sound page: the signal path preview, the output group and the equalizer group. With
/// nothing open there is no rate or buffer to show, so the failure stands in for them.
pub(super) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> Vec<El> {
    let theme = &ui.theme;
    let outputs = &ui.store.outputs;
    let mut output = vec![
        Row::new(
            "output-device",
            t!("settings-output-device"),
            crate::chrome::output_picker::view(ui, cx, layer),
        )
        .build(theme),
    ];
    match outputs.failure().is_some() {
        true => output.push(failure_row(ui)),
        false => {
            output.push(sample_rate_row(ui));
            if outputs.match_rate_supported() {
                output.push(match_rate_row(ui));
            }
            output.push(buffer_row(ui));
        }
    }
    vec![
        super::preview::sound::view(ui, cx, layout),
        group(
            theme,
            Some(("settings-group-output", "icons/volume-2.svg")),
            None,
            output,
        ),
        group(
            theme,
            Some(("settings-equalizer", "icons/sliders-horizontal.svg")),
            None,
            [equalizer_row(ui, layer)],
        ),
    ]
}

/// The failure row's title and detail, which a search reads too.
pub(super) fn output_failure(ui: &Ui) -> (String, String) {
    let error = ui
        .store
        .outputs
        .failure()
        .map(|failure| failure.error.clone())
        .unwrap_or_default();
    (
        t!("settings-output-failed").into_owned(),
        t!("settings-output-failed-detail", error = error).into_owned(),
    )
}

/// Why nothing plays: the device to play on would not open.
fn failure_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let (title, detail) = output_failure(ui);
    Row::new(
        "output-failure",
        title,
        ui::spoken("settings-output-failure", detail, theme)
            .max_w(theme.metrics.cover * FAILURE_WIDTH)
            .truncate()
            .font(font(theme, Text::Label, Weight::NORMAL))
            .color(theme.muted_foreground.color())
            .transition(Transition::opacity()),
    )
    .build(theme)
}

/// The rate the device runs at. Where Sonora can open the device at other rates it steps
/// through them, otherwise it offers a way into the system's own settings where there is one.
/// The row keeps its shape while the stream reopens, showing the rate it reopens at.
fn sample_rate_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let outputs = &ui.store.outputs;
    let device = outputs.current();
    // while the rate follows each track there is none to pick
    let following = outputs.following(settings);
    let current = match following {
        true => device.map(|device| device.format.rate),
        false => outputs.rate(settings),
    };
    let rates = device
        .map(|device| device.rates.clone())
        .unwrap_or_default();
    // while another device is being tried, the open stream is not the one that will play
    let locked = outputs.switching();
    let settings_uri = match rates.len() > 1 || following {
        true => None,
        false => music::output::system_audio_settings_uri(device.map(|device| device.id.as_str())),
    };
    let rate = current
        .map(|rate| t!("settings-sample-rate-value", khz = f64::from(rate) / 1_000.))
        .unwrap_or_else(|| t!("settings-output-idle"));
    let rate = value(theme, "sample-rate-value", rate.into_owned());
    let mut control = row().center_y().gap(theme.font_size * 0.5);
    match current.filter(|_| rates.len() > 1 && !following) {
        Some(current) => {
            let (lower, higher) = stepper(
                theme,
                "sample-rate",
                ("settings-sample-rate-lower", "settings-sample-rate-higher"),
                current,
                &rates,
                locked,
                Outputs::set_rate,
            );
            control = control.child(lower).child(rate).child(higher);
        }
        None => control = control.child(rate),
    }
    if let Some(uri) = settings_uri {
        control = control.child(
            Button::new("sample-rate-change")
                .label(t!("settings-sample-rate-change"))
                .small()
                .outline()
                .icon("icons/link.svg")
                .on_press_tx(move |_: &mut Ui, tx: &mut Tx| {
                    // the system's own settings are an app or a settings URI, which the
                    // browser handoff refuses, so they open through the system
                    if tx.open_url(&uri).is_err()
                        && let Err(error) = open::that_detached(&uri)
                    {
                        log::warn!("settings: cannot open {uri}: {error}");
                    }
                })
                .build(theme),
        );
    }
    Row::new("sample-rate", t!("settings-sample-rate"), control).build(theme)
}

/// The switch that has a direct output run at the sample rate of each track. It means nothing
/// on a shared output, whose rate is every other app's too, and says so until a direct one is
/// chosen.
fn match_rate_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let outputs = &ui.store.outputs;
    let on = outputs.match_rate(&ui.store.settings);
    let available = outputs.direct_chosen();
    let locked = outputs.switching();
    let detail = match available {
        true => t!("settings-match-rate-detail"),
        false => t!("settings-match-rate-direct-only"),
    };
    Row::new(
        "match-rate",
        t!("settings-match-rate"),
        toggle(
            theme,
            "match-rate",
            t!("settings-match-rate"),
            on,
            !available || locked,
            |ui, on| ui.store.set_output_match_rate(on),
        ),
    )
    .detail(detail)
    .disabled(!available)
    .build(theme)
}

/// The buffer Sonora asks the device for, in frames, with the buffers the device allows to
/// step through. The row keeps its shape while the stream reopens, showing the buffer it
/// reopens with.
fn buffer_row(ui: &Ui) -> El {
    let theme = &ui.theme;
    let outputs = &ui.store.outputs;
    let current = outputs.buffer(&ui.store.settings);
    let buffers = outputs
        .current()
        .map(|device| device.buffers.clone())
        .unwrap_or_default();
    let locked = outputs.switching();
    let buffer = current
        .map(|frames| frames.to_string())
        .unwrap_or_else(|| t!("settings-output-idle").into_owned());
    let buffer = value(theme, "buffer-value", buffer);
    let control = match current.filter(|_| buffers.len() > 1) {
        Some(current) => {
            let (lower, higher) = stepper(
                theme,
                "buffer",
                ("settings-buffer-smaller", "settings-buffer-larger"),
                current,
                &buffers,
                locked,
                Outputs::set_buffer,
            );
            row()
                .center_y()
                .gap(theme.font_size * 0.5)
                .child(lower)
                .child(buffer)
                .child(higher)
        }
        None => row().center_y().child(buffer),
    };
    Row::new("buffer", t!("settings-buffer"), control).build(theme)
}

/// A figure between its steppers, in tabular digits so a step does not shift the buttons, and
/// fading as it changes.
fn value(theme: &Theme, key: &str, figure: String) -> El {
    ui::spoken(key, figure, theme)
        .font(font(theme, Text::Body, Weight::NORMAL).features(ui::TABULAR))
}

/// The way into the equalizer: its name, a picker of every sound it can play, its switch and a
/// chevron. The row opens the equalizer page, while the picker and the switch work in place.
fn equalizer_row(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let on = ui.store.equalizer.enabled();
    let sound = sound_picker(ui, SOUND, &ui.screens.settings.popovers, |ui: &mut Ui| {
        &mut ui.screens.settings.popovers
    })
    .width(Picker::<Ui>::REGULAR)
    .name(t!("equalizer-sound"))
    .build(theme, layer);

    row()
        .key("settings-row-equalizer")
        .role("button")
        .label(t!("settings-equalizer"))
        .focusable(true)
        .w_full()
        .center_y()
        .gap(theme.font_size * 0.75)
        .h(theme.metrics.list_row)
        .mx(-REACH)
        .px(REACH)
        .radius(theme.radius)
        .cursor(Cursor::Pointer)
        .hover(theme.secondary_hover.color())
        .press_scale(ui::PRESS_SCALE_CARD)
        .on_press(act(|ui: &mut Ui| ui.go(Destination::Equalizer)))
        .child(
            text(t!("settings-equalizer"))
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(theme.foreground.color())
                .truncate()
                .grow(1.)
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        )
        .child(sound)
        .child(toggle(
            theme,
            "equalizer",
            t!("settings-equalizer"),
            on,
            false,
            |ui, on| {
                let Store {
                    equalizer,
                    settings,
                    cx,
                    ..
                } = &mut ui.store;
                equalizer.set_enabled(on, settings, cx);
            },
        ))
        .child(
            icon(icons::get("icons/chevron-right.svg"), CHEVRON)
                .color(theme.muted_foreground.color())
                .a11y_hidden(),
        )
}

/// The minus and plus buttons beside an output setting's value, which `apply` the next lower or
/// higher of `options` than `current`. A button with nothing further that way is disabled, and
/// both are while `locked`. `names` holds the Fluent keys screen readers name the two buttons by.
fn stepper(
    theme: &Theme,
    key: &str,
    names: (&'static str, &'static str),
    current: u32,
    options: &[u32],
    locked: bool,
    apply: Apply,
) -> (El, El) {
    let lower = options
        .iter()
        .rev()
        .find(|&&option| option < current)
        .copied();
    let higher = options.iter().find(|&&option| option > current).copied();
    let step = |suffix: &str, glyph: &str, name: &str, next: Option<u32>| {
        Button::new(format!("{key}-{suffix}"))
            .label(glyph)
            .name(i18n::lookup(name, None))
            .small()
            .outline()
            .disabled(locked || next.is_none())
            .on_press(act(move |ui: &mut Ui| {
                let Some(next) = next else { return };
                let Store {
                    outputs,
                    settings,
                    cx,
                    ..
                } = &mut ui.store;
                apply(outputs, next, settings, cx);
            }))
            .build(theme)
    };
    (
        step("lower", "\u{2212}", names.0, lower),
        step("higher", "+", names.1, higher),
    )
}
