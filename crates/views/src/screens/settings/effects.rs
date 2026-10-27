use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, div};
use i18n::t;
use music::effects::{self, MAX_CUTOFF, MAX_SPEED, MIN_CUTOFF, MIN_SPEED, Preset};
use ui::{ActiveTheme as _, MenuItem, Picker, Scrubber, ScrubberState, Switch};

use super::{Group, SettingsView, Slot};

const PRESETS: &str = "effect-presets";
/// The step a dragged speed snaps to.
const SPEED_STEP: f32 = 0.05;
/// How far along its slider the cutoff opens the filter.
const CUTOFF_OPEN: f32 = 0.98;

impl SettingsView {
    /// The effects switch, and every effect under it while they are on.
    pub(super) fn effects_group(&self, cx: &App) -> Group {
        let mut rows = vec![Slot::Effects];
        if self.playback.read(cx).effects() {
            rows.extend([
                Slot::EffectsPreset,
                Slot::Speed,
                Slot::KeepPitch,
                Slot::LowPass,
                Slot::Reverb,
                Slot::Room,
            ]);
        }
        Group::new("settings-group-effects", "icons/gauge.svg").rows(rows)
    }

    pub(super) fn effects_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.playback.read(cx).effects();

        self.row(
            t!("settings-effects"),
            Switch::new("effects", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback
                        .update(cx, |playback, cx| playback.set_effects(!on, cx));
                }))
                .into_any_element(),
        )
    }

    /// The preset picker. It reads the values back, so a slider moved by hand shows as Custom.
    pub(super) fn effects_preset_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.playback.read(cx).effect_settings();
        let chosen = Preset::matching(&current);
        let label = match chosen {
            Some(preset) => i18n::lookup(preset_key(preset), None),
            None => t!("settings-effects-custom"),
        };

        let picker = Picker::new(PRESETS, &self.popovers, label)
            .width(Picker::NARROW)
            .items(Preset::ALL.map(|preset| {
                MenuItem::new(preset.id(), i18n::lookup(preset_key(preset), None))
                    .selected(chosen == Some(preset))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.playback.update(cx, |playback, cx| {
                            let settings = preset.settings(playback.effect_settings());
                            playback.set_effect_settings(settings, cx)
                        });
                        cx.notify();
                    }))
            }));

        self.row(t!("settings-effects-preset"), picker.into_any_element())
    }

    pub(super) fn speed_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let speed = self.playback.read(cx).effect_settings().speed;
        let slider = self.effect_slider(
            "settings-effects-speed",
            &self.speed,
            speed_fraction(speed),
            t!(
                "settings-effects-speed-value",
                speed = format!("{speed:.2}")
            ),
            |settings, fraction| effects::Settings {
                speed: speed_at(fraction),
                ..settings
            },
            cx,
        );
        self.row(t!("settings-effects-speed"), slider)
    }

    pub(super) fn keep_pitch_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.playback.read(cx).effect_settings().keep_pitch;

        self.row(
            t!("settings-effects-keep-pitch"),
            Switch::new("effects-keep-pitch", on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.playback.update(cx, |playback, cx| {
                        let settings = effects::Settings {
                            keep_pitch: !on,
                            ..playback.effect_settings()
                        };
                        playback.set_effect_settings(settings, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    pub(super) fn low_pass_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let cutoff = self.playback.read(cx).effect_settings().cutoff;
        let slider = self.effect_slider(
            "settings-effects-low-pass",
            &self.cutoff,
            cutoff_fraction(cutoff),
            cutoff_label(cutoff),
            |settings, fraction| effects::Settings {
                cutoff: cutoff_at(fraction),
                ..settings
            },
            cx,
        );
        self.row(t!("settings-effects-low-pass"), slider)
    }

    pub(super) fn reverb_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let reverb = self.playback.read(cx).effect_settings().reverb;
        let slider = self.effect_slider(
            "settings-effects-reverb",
            &self.reverb,
            reverb,
            t!("settings-effects-percent", percent = percent(reverb)),
            |settings, fraction| effects::Settings {
                reverb: hundredths(fraction),
                ..settings
            },
            cx,
        );
        self.row(t!("settings-effects-reverb"), slider)
    }

    pub(super) fn room_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let room = self.playback.read(cx).effect_settings().room;
        let slider = self.effect_slider(
            "settings-effects-room",
            &self.room,
            room,
            t!("settings-effects-percent", percent = percent(room)),
            |settings, fraction| effects::Settings {
                room: hundredths(fraction),
                ..settings
            },
            cx,
        );
        self.row(t!("settings-effects-room"), slider)
    }

    /// A slider with its value read out beside it, the way the opacity row draws one, announced
    /// by the setting `key` names. `apply` turns the dragged fraction into new effect values,
    /// which reach the engines as the slider moves.
    fn effect_slider(
        &self,
        key: &'static str,
        state: &ScrubberState,
        fraction: f32,
        readout: SharedString,
        apply: fn(effects::Settings, f32) -> effects::Settings,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id(key)
                    .role(Role::Group)
                    .aria_label(i18n::lookup(key, None))
                    .w(theme.metrics.cover)
                    .child(
                        Scrubber::new(state, fraction)
                            .colors(theme.progress_bar, theme.muted, theme.foreground)
                            .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                                this.playback.update(cx, |playback, cx| {
                                    let settings = apply(playback.effect_settings(), *fraction);
                                    playback.set_effect_settings(settings, cx)
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w(theme.metrics.control * 1.5)
                    .whitespace_nowrap()
                    .text_right()
                    .child(readout),
            )
            .into_any_element()
    }
}

fn preset_key(preset: Preset) -> &'static str {
    match preset {
        Preset::Off => "settings-effects-off",
        Preset::SlowedReverb => "settings-effects-slowed-reverb",
        Preset::Nightcore => "settings-effects-nightcore",
        Preset::LoFi => "settings-effects-lo-fi",
        Preset::Muffled => "settings-effects-muffled",
        Preset::Hall => "settings-effects-hall",
    }
}

/// Where a speed sits on its slider: on a log scale, so normal speed is in the middle and
/// half and double are the ends.
fn speed_fraction(speed: f32) -> f32 {
    let (low, high) = (MIN_SPEED.log2(), MAX_SPEED.log2());
    (speed.log2() - low) / (high - low)
}

/// The speed at a point on its slider, snapped to `SPEED_STEP`.
fn speed_at(fraction: f32) -> f32 {
    let (low, high) = (MIN_SPEED.log2(), MAX_SPEED.log2());
    let raw = (low + fraction * (high - low)).exp2();
    ((raw / SPEED_STEP).round() * SPEED_STEP).clamp(MIN_SPEED, MAX_SPEED)
}

/// Where a cutoff sits on its slider: on a log scale, so every octave gets the same room.
fn cutoff_fraction(cutoff: f32) -> f32 {
    (cutoff / MIN_CUTOFF).ln() / (MAX_CUTOFF / MIN_CUTOFF).ln()
}

/// The cutoff at a point on its slider, rounded to two significant figures. The far end
/// opens the filter.
fn cutoff_at(fraction: f32) -> f32 {
    if fraction >= CUTOFF_OPEN {
        return MAX_CUTOFF;
    }
    let raw = MIN_CUTOFF * (MAX_CUTOFF / MIN_CUTOFF).powf(fraction.max(0.));
    let unit = 10f32.powf(raw.log10().floor() - 1.);
    ((raw / unit).round() * unit).clamp(MIN_CUTOFF, MAX_CUTOFF)
}

/// A cutoff as the readout shows it: Off when the filter is open, then hertz, then kilohertz
/// with one decimal when it has one.
fn cutoff_label(cutoff: f32) -> SharedString {
    if cutoff >= MAX_CUTOFF {
        return t!("settings-effects-low-pass-off");
    }
    match cutoff >= 1_000. {
        true => {
            let khz = format!("{:.1}", cutoff / 1_000.);
            let khz = khz.strip_suffix(".0").unwrap_or(&khz).to_owned();
            t!("settings-effects-kilohertz", khz = khz)
        }
        false => t!("settings-effects-hertz", hz = cutoff.round() as i64),
    }
}

/// A share from zero to one as a whole percentage.
fn percent(share: f32) -> i64 {
    (share * 100.).round() as i64
}

/// A slider's fraction snapped to the nearest hundredth.
fn hundredths(fraction: f32) -> f32 {
    (fraction.clamp(0., 1.) * 100.).round() / 100.
}
