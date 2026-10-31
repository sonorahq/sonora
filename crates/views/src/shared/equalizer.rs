use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Entity, SharedString};
use i18n::t;
use music::equalizer::{Curve, Preset};
use state::{AppSettings, Equalizer, Sound};
use ui::{
    ActiveTheme as _, MenuItem, Picker, Popovers, ResponseAxes, ResponsePoint, ResponseSketch,
};

/// How tall a sound menu grows before it scrolls, in cover sizes.
pub(crate) const SOUND_MENU_HEIGHT: f32 = 2.4;
/// The band a curve sketch spans in hertz, from the lowest graphic band to the highest. The
/// roll-off past them takes a sketch's few end pixels and reads as noise at this size.
const SKETCH_BAND: (f64, f64) = (31.25, 16_000.);
/// How far either side of 0 dB every sketch reaches, the same for all so their shapes compare.
const SKETCH_SPAN: f32 = 8.;
/// How many points a sketch samples across the band, one for every two of its pixels, so the
/// stroke has few joins to break up.
const SKETCH_POINTS: usize = 15;
/// The rate a sketch is drawn for, whatever the output plays at.
const SKETCH_RATE: u32 = 48_000;

/// One picker for every sound the equalizer can play: the built-in presets, the curve someone
/// shaped from one, and the named profiles. Choosing any of them turns the equalizer on.
///
/// A preset loads into the profile with no name, so choosing one never overwrites a named
/// profile. That profile shows as the preset it matches, or as Custom once it was edited.
pub(crate) fn sound_picker(
    key: &'static str,
    popovers: &Popovers,
    equalizer: &Entity<Equalizer>,
    settings: &Entity<AppSettings>,
    cx: &App,
) -> Picker {
    let height = cx.theme().metrics.cover * SOUND_MENU_HEIGHT;
    let settings = settings.read(cx);
    let playing = settings.equalizer_profile();
    let scratch = settings.equalizer_scratch();
    let shaped = scratch
        .and_then(|index| settings.equalizer_profile_curve(index))
        .map(|curve| Preset::matching(&curve));
    let on_scratch = scratch == Some(playing);
    let custom = matches!(shaped, Some(None));
    let current = sound_name(settings);
    let face = sketch(equalizer.read(cx).curve());

    let custom_item = custom.then(|| {
        let equalizer = equalizer.clone();
        let index = scratch.unwrap_or_default();
        MenuItem::new("equalizer-sound-custom", t!("settings-equalizer-custom"))
            .selected(on_scratch)
            .when_some(settings.equalizer_profile_curve(index), |item, curve| {
                item.leading(sketch(&curve))
            })
            .on_click(move |_, _, cx| {
                equalizer.update(cx, |equalizer, cx| equalizer.play_profile(index, cx));
            })
    });
    let equalizer = equalizer.clone();
    let sounds = sound_items(
        settings,
        |sound| match sound {
            Sound::Preset(preset) => on_scratch && shaped == Some(Some(*preset)),
            Sound::Profile(name) => !on_scratch && settings.equalizer_profile_name() == name,
        },
        move |sound, cx| {
            equalizer.update(cx, |equalizer, cx| equalizer.play_sound(sound, cx));
        },
    );

    Picker::new(key, popovers, current)
        .name(t!("equalizer-sound"))
        .leading(face)
        .scroll_after(height)
        .items(custom_item)
        .items(sounds)
}

/// A menu item for each built-in preset, then one for each named profile after a divider, each
/// led by a sketch of its curve. The unnamed profile is left out. `selected` marks an item and `pick` runs when one is clicked.
pub(crate) fn sound_items(
    settings: &AppSettings,
    selected: impl Fn(&Sound) -> bool,
    pick: impl Fn(&Sound, &mut App) + 'static,
) -> Vec<MenuItem> {
    let pick = Rc::new(pick);
    let item = |id: gpui::ElementId, sound: Sound| {
        let pick = pick.clone();
        MenuItem::new(id, sound_label(&sound))
            .selected(selected(&sound))
            .when_some(sound_curve(settings, &sound), |item, curve| {
                item.leading(sketch(&curve))
            })
            .on_click(move |_, _, cx| pick(&sound, cx))
    };
    let presets = Preset::ALL.map(|preset| item(preset.id().into(), Sound::Preset(preset)));
    let named: Vec<_> = settings
        .equalizer_profiles()
        .enumerate()
        .filter(|(_, name)| !name.is_empty())
        .map(|(index, name)| {
            item(
                ("equalizer-profile", index).into(),
                Sound::Profile(name.to_owned()),
            )
        })
        .collect();
    let divider = (!named.is_empty()).then(|| MenuItem::separator("equalizer-sound-profiles"));
    presets.into_iter().chain(divider).chain(named).collect()
}

/// What a sound is called: a preset's name in the interface's language, or a profile's own.
pub(crate) fn sound_label(sound: &Sound) -> SharedString {
    match sound {
        Sound::Preset(preset) => i18n::lookup(preset_key(*preset), None),
        Sound::Profile(name) => SharedString::from(name.clone()),
    }
}

/// The Fluent key naming a built-in preset.
pub(crate) fn preset_key(preset: Preset) -> &'static str {
    match preset {
        Preset::Flat => "settings-equalizer-flat",
        Preset::BassBoost => "settings-equalizer-bass-boost",
        Preset::BassReducer => "settings-equalizer-bass-reducer",
        Preset::TrebleBoost => "settings-equalizer-treble-boost",
        Preset::Vocal => "settings-equalizer-vocal",
        Preset::Rock => "settings-equalizer-rock",
        Preset::Pop => "settings-equalizer-pop",
        Preset::Jazz => "settings-equalizer-jazz",
        Preset::Classical => "settings-equalizer-classical",
        Preset::Electronic => "settings-equalizer-electronic",
        Preset::Acoustic => "settings-equalizer-acoustic",
        Preset::Loudness => "settings-equalizer-loudness",
    }
}

/// The name of the sound the equalizer plays: the preset the unnamed profile matches, Custom
/// once that profile was edited away from every preset, or the playing profile's own name.
pub(crate) fn sound_name(settings: &AppSettings) -> SharedString {
    let playing = settings.equalizer_profile();
    let scratch = settings.equalizer_scratch();
    let shaped = scratch
        .and_then(|index| settings.equalizer_profile_curve(index))
        .map(|curve| Preset::matching(&curve));
    match (scratch == Some(playing), shaped.flatten()) {
        (true, Some(preset)) => i18n::lookup(preset_key(preset), None),
        (true, None) => t!("settings-equalizer-custom"),
        (false, _) => settings
            .equalizer_profiles()
            .nth(playing)
            .map(|name| SharedString::from(name.to_owned()))
            .unwrap_or_default(),
    }
}

/// A tiny drawing of `curve`'s response for a sound picker, sampled from the same filter math
/// the equalizer's graph draws and plays. Every sketch shares one gain span, so a gentle sound
/// looks gentle beside a strong one.
pub(crate) fn sketch(curve: &Curve) -> ResponseSketch {
    let (low, high) = SKETCH_BAND;
    let response = curve.response(SKETCH_RATE);
    let ratio = high / low;
    let points = (0..SKETCH_POINTS).map(|point| {
        let frequency = low * ratio.powf(point as f64 / (SKETCH_POINTS - 1) as f64);
        ResponsePoint {
            frequency_hz: frequency as f32,
            gain_db: response.db(frequency) as f32,
        }
    });
    let axes = ResponseAxes::new(low as f32..=high as f32, -SKETCH_SPAN..=SKETCH_SPAN)
        .expect("the sketch's axes are valid");
    ResponseSketch::new(axes, points)
}

/// The curve `sound` plays: a preset's own, or a named profile's as the settings hold it, or
/// `None` for a profile that is gone.
pub(crate) fn sound_curve(settings: &AppSettings, sound: &Sound) -> Option<Curve> {
    match sound {
        Sound::Preset(preset) => Some(preset.curve()),
        Sound::Profile(name) => settings
            .equalizer_profiles()
            .position(|profile| profile == name)
            .and_then(|index| settings.equalizer_profile_curve(index)),
    }
}
