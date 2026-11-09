//! How equalizer profiles and device rules are stored in the settings file and edited through
//! `AppSettings`.
//! Every reader here is lenient, so a hand-edited mistake drops one value instead of making the
//! whole file unreadable.

use std::collections::HashSet;

use anyhow::{Context as _, Result};
use i18n::SharedString;
use music::equalizer::{
    Curve, Filter, Kind, MAX_FILTERS, PREAMP_RANGE, Preamp, Preset, Tone, Width,
};
use music::output::OutputId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{AppSettings, Values};
use crate::Cx;
use crate::equalizer::{DeviceRule, RuleDevice, Sound};

/// How a rule for every output no other rule names stores its output.
const OTHER_OUTPUTS: &str = "other";

/// One stored equalizer curve, in plain numbers and strings, so a hand-edited mistake drops one
/// filter instead of making the file unreadable. The name is empty for the profile Sonora made.
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Profile {
    name: String,
    preamp: StoredPreamp,
    filters: Vec<StoredFilter>,
    /// The outputs an older Sonora bound this profile to. Read once to make device rules from,
    /// and never written back.
    #[serde(skip_serializing)]
    outputs: Vec<OutputId>,
    /// Whether an older Sonora played this profile on every output no profile named. Read once
    /// like `outputs`.
    #[serde(skip_serializing)]
    other_outputs: bool,
}

/// One device rule as the settings file holds it: the output, or `"other"`, and either a
/// preset's id or a profile's name.
#[derive(Serialize)]
struct StoredRule<'a> {
    output: StoredDevice<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preset: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum StoredDevice<'a> {
    Output(&'a OutputId),
    Other(&'static str),
}

/// `"auto"` or a number of decibels.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum StoredPreamp {
    Decibels(f64),
    Word(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StoredFilter {
    kind: String,
    frequency: f64,
    gain: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    q: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    octaves: Option<f64>,
    enabled: bool,
}

impl Values {
    /// Settles the equalizer values for use: one profile when the file had none, built from the
    /// band gains an older Sonora saved when it had those, the outputs an older Sonora bound to
    /// profiles made into rules, every rule under its output's current key, one rule per device,
    /// and the chosen profile in range.
    pub(super) fn settle_equalizer(&mut self) {
        if self.equalizer_profiles.is_empty() {
            let gains: Vec<f64> = self
                .equalizer_bands
                .iter()
                .map(|gain| f64::from(*gain))
                .collect();
            self.equalizer_profiles
                .push(Profile::of(&Curve::bands(&gains)));
        }
        self.equalizer_bands = Vec::new();
        self.migrate_bindings();
        self.upgrade_rule_keys();
        self.settle_rules();
        self.equalizer_profile = self
            .equalizer_profile
            .min(self.equalizer_profiles.len() - 1);
    }

    /// Makes the outputs an older Sonora bound to each profile into rules after the stored ones,
    /// which win for a device both name. A binding to the unnamed profile becomes a rule for the
    /// preset its curve matches, or nothing.
    fn migrate_bindings(&mut self) {
        let mut migrated = Vec::new();
        for profile in &mut self.equalizer_profiles {
            let outputs = std::mem::take(&mut profile.outputs);
            let other = std::mem::take(&mut profile.other_outputs);
            if outputs.is_empty() && !other {
                continue;
            }
            let sound = match profile.name.is_empty() {
                true => Preset::matching(&profile.curve()).map(Sound::Preset),
                false => Some(Sound::Profile(profile.name.clone())),
            };
            let Some(sound) = sound else {
                log::warn!(
                    "settings: dropping the outputs bound to the unnamed equalizer profile, whose curve is no preset"
                );
                continue;
            };
            let devices = outputs
                .into_iter()
                .map(RuleDevice::Output)
                .chain(other.then_some(RuleDevice::Other));
            migrated.extend(devices.map(|device| DeviceRule {
                device,
                sound: sound.clone(),
            }));
        }
        if !migrated.is_empty() {
            log::info!(
                "settings: made {} equalizer output bindings into device rules; the bindings are dropped the next time settings are saved",
                migrated.len()
            );
            self.equalizer_rules.extend(migrated);
        }
    }

    /// Moves rules an older Sonora keyed by a sound server's bare sink name to the key the sink
    /// goes by now, so they keep matching it.
    fn upgrade_rule_keys(&mut self) {
        let mut upgraded = 0;
        for rule in &mut self.equalizer_rules {
            if let RuleDevice::Output(output) = &mut rule.device
                && let Some(id) = music::output::upgrade_output_key(&output.id)
            {
                output.id = id;
                upgraded += 1;
            }
        }
        if upgraded > 0 {
            log::info!(
                "settings: moved {upgraded} equalizer device rules to their output's current key; the old keys are dropped the next time settings are saved"
            );
        }
    }

    /// Keeps the first rule for each device, drops rules for a profile that is not stored, and
    /// puts the rule for every other device last.
    fn settle_rules(&mut self) {
        let mut seen = HashSet::new();
        let profiles = &self.equalizer_profiles;
        self.equalizer_rules.retain(|rule| {
            if !seen.insert(rule.device.id().map(str::to_owned)) {
                log::warn!(
                    "settings: a device has two equalizer rules, keeping the first; the second is dropped the next time settings are saved"
                );
                return false;
            }
            let Sound::Profile(name) = &rule.sound else {
                return true;
            };
            let stored = profiles.iter().any(|profile| profile.name == *name);
            if !stored {
                log::warn!(
                    "settings: dropping an equalizer rule for {name:?}, which is no stored profile; it is gone the next time settings are saved"
                );
            }
            stored
        });
        self.equalizer_rules
            .sort_by_key(|rule| rule.device == RuleDevice::Other);
    }
}

impl Profile {
    fn of(curve: &Curve) -> Self {
        let mut profile = Self::default();
        profile.store(curve);
        profile
    }

    /// Every profile in the file's list that is an object, a filter that does not read dropped
    /// and logged and a preamp that does not read taken as automatic.
    pub(super) fn read_all<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Self>, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let serde_json::Value::Array(items) = value else {
            log::warn!(
                "settings: equalizer_profiles is not a list, starting over with one; the stored value is replaced the next time settings are saved"
            );
            return Ok(Vec::new());
        };
        Ok(items.into_iter().filter_map(Self::read).collect())
    }

    fn read(value: serde_json::Value) -> Option<Self> {
        let serde_json::Value::Object(mut fields) = value else {
            log::warn!(
                "settings: dropping an equalizer profile that is not an object; it is gone the next time settings are saved"
            );
            return None;
        };
        let mut profile = Self::default();
        if let Some(name) = profile_field(&mut fields, "name") {
            profile.name = name;
        }
        match profile_field(&mut fields, "preamp") {
            Some(StoredPreamp::Decibels(db)) if !PREAMP_RANGE.contains(&db) => log::warn!(
                "settings: equalizer preamp {db} dB is outside {PREAMP_RANGE:?}, taking auto; the stored value is replaced the next time settings are saved"
            ),
            Some(StoredPreamp::Word(word)) if word != "auto" => log::warn!(
                "settings: equalizer preamp {word:?} is not a number nor auto, taking auto; the stored value is replaced the next time settings are saved"
            ),
            Some(preamp) => profile.preamp = preamp,
            None => {}
        }
        let filters: Vec<serde_json::Value> =
            profile_field(&mut fields, "filters").unwrap_or_default();
        profile.filters = filters
            .into_iter()
            .filter_map(|filter| {
                serde_json::from_value::<StoredFilter>(filter)
                    .map_err(anyhow::Error::from)
                    .and_then(|filter| filter.parse())
                    .map(|filter| StoredFilter::of(&filter))
                    .map_err(|error| {
                        log::warn!(
                            "settings: dropping an equalizer filter, gone the next time settings are saved: {error:#}"
                        );
                    })
                    .ok()
            })
            .collect();
        let outputs: Vec<serde_json::Value> =
            profile_field(&mut fields, "outputs").unwrap_or_default();
        profile.outputs = outputs
            .into_iter()
            .filter_map(|output| match serde_json::from_value::<OutputId>(output) {
                Ok(output) if !output.id.is_empty() => Some(output),
                _ => {
                    log::warn!(
                        "settings: dropping an equalizer profile's output that has no id and name, gone the next time settings are saved"
                    );
                    None
                }
            })
            .collect();
        profile.other_outputs = profile_field(&mut fields, "other_outputs").unwrap_or(false);
        if profile.filters.len() > MAX_FILTERS {
            log::warn!(
                "settings: dropping {} equalizer filters past the first {MAX_FILTERS}",
                profile.filters.len() - MAX_FILTERS
            );
            profile.filters.truncate(MAX_FILTERS);
        }
        Some(profile)
    }

    fn store(&mut self, curve: &Curve) {
        self.preamp = match curve.preamp() {
            Preamp::Auto => StoredPreamp::default(),
            Preamp::Manual(db) => StoredPreamp::Decibels(db),
        };
        self.filters = curve.filters().iter().map(StoredFilter::of).collect();
    }

    /// The curve as stored. `read` already dropped, and logged, what does not make a valid
    /// curve, so this only converts.
    fn curve(&self) -> Curve {
        let preamp = match self.preamp {
            StoredPreamp::Decibels(db) if PREAMP_RANGE.contains(&db) => Preamp::Manual(db),
            _ => Preamp::Auto,
        };
        let filters = self
            .filters
            .iter()
            .filter_map(|filter| filter.parse().ok())
            .take(MAX_FILTERS)
            .collect();
        Curve::new(preamp, filters).expect("the preamp and every filter were checked")
    }
}

impl Default for StoredPreamp {
    fn default() -> Self {
        Self::Word("auto".to_owned())
    }
}

impl StoredFilter {
    fn of(filter: &Filter) -> Self {
        let (q, octaves) = match filter.width() {
            Width::Q(q) => (Some(q), None),
            Width::Octaves(octaves) => (None, Some(octaves)),
        };
        Self {
            kind: filter.kind().id().to_owned(),
            frequency: filter.frequency(),
            gain: filter.gain(),
            q,
            octaves,
            enabled: filter.enabled(),
        }
    }

    fn parse(&self) -> Result<Filter> {
        let kind = Kind::from_id(&self.kind)
            .with_context(|| format!("unknown filter kind {:?}", self.kind))?;
        let width = match (self.q, self.octaves) {
            (Some(q), _) => Width::Q(q),
            (None, Some(octaves)) => Width::Octaves(octaves),
            (None, None) => anyhow::bail!("a filter needs a q or a width in octaves"),
        };
        Ok(Filter::new(
            kind,
            self.frequency,
            self.gain,
            width,
            self.enabled,
        )?)
    }
}

impl Default for StoredFilter {
    fn default() -> Self {
        Self {
            kind: Kind::Peak.id().to_owned(),
            frequency: 1_000.,
            gain: 0.,
            q: None,
            octaves: None,
            enabled: true,
        }
    }
}

impl AppSettings {
    pub fn equalizer(&self) -> bool {
        self.values.equalizer
    }

    /// The curve of the profile that plays, with any filter out of range dropped. Flat when
    /// there is no profile.
    pub fn equalizer_curve(&self) -> Curve {
        self.values
            .equalizer_profiles
            .get(self.values.equalizer_profile)
            .map_or_else(|| Preset::Flat.curve(), Profile::curve)
    }

    /// The curve profile `index` holds, with any filter out of range dropped.
    pub fn equalizer_profile_curve(&self, index: usize) -> Option<Curve> {
        self.values
            .equalizer_profiles
            .get(index)
            .map(Profile::curve)
    }

    pub fn set_equalizer(&mut self, on: bool, cx: &mut Cx) {
        self.values.equalizer = on;
        self.schedule_save(cx);
    }

    pub fn set_equalizer_advanced(&mut self, advanced: bool, cx: &mut Cx) {
        self.values.equalizer_advanced = advanced;
        self.schedule_save(cx);
    }

    /// The names of the equalizer profiles in order, empty for the one Sonora made.
    pub fn equalizer_profiles(&self) -> impl Iterator<Item = &str> {
        self.values
            .equalizer_profiles
            .iter()
            .map(|profile| profile.name.as_str())
    }

    /// Which of `equalizer_profiles` plays.
    pub fn equalizer_profile(&self) -> usize {
        self.values.equalizer_profile
    }

    pub fn equalizer_advanced(&self) -> bool {
        self.values.equalizer_advanced
    }

    /// The bass and treble laid over whichever sound plays.
    pub fn equalizer_tone(&self) -> Tone {
        Tone::new(self.values.equalizer_bass, self.values.equalizer_treble)
    }

    pub fn set_equalizer_tone(&mut self, tone: Tone, cx: &mut Cx) {
        self.values.equalizer_bass = tone.bass();
        self.values.equalizer_treble = tone.treble();
        self.schedule_save(cx);
    }

    /// The name of the profile that plays, empty for the one Sonora made.
    pub fn equalizer_profile_name(&self) -> &str {
        self.values
            .equalizer_profiles
            .get(self.values.equalizer_profile)
            .map_or("", |profile| profile.name.as_str())
    }

    /// Makes profile `index` the one that plays. An index past the end is ignored.
    pub fn select_equalizer_profile(&mut self, index: usize, cx: &mut Cx) {
        if index < self.values.equalizer_profiles.len() {
            self.values.equalizer_profile = index;
            self.schedule_save(cx);
        }
    }

    /// Where the profile with no name sits, the one presets load into, if there is one.
    pub fn equalizer_scratch(&self) -> Option<usize> {
        self.values
            .equalizer_profiles
            .iter()
            .position(|profile| profile.name.is_empty())
    }

    /// Makes the profile with no name the one that plays, adding it holding `curve` when there
    /// is none, so a preset never lands on a profile someone named.
    pub fn select_equalizer_scratch(&mut self, curve: &Curve, cx: &mut Cx) {
        match self.equalizer_scratch() {
            Some(index) => self.values.equalizer_profile = index,
            None => {
                let values = &mut self.values;
                values.equalizer_profile = values.equalizer_profiles.len();
                values.equalizer_profiles.push(Profile::of(curve));
            }
        }
        self.schedule_save(cx);
    }

    /// Adds a profile after the others and makes it the one that plays. Its name is `name`
    /// trimmed, numbered when another profile shows it already. An empty name stays empty so
    /// the label is read at render, unless a profile with no name exists, which keeps that one
    /// the only one presets load into.
    pub fn add_equalizer_profile(&mut self, name: &str, curve: &Curve, cx: &mut Cx) {
        let name = match (name.trim(), self.equalizer_scratch()) {
            ("", None) => String::new(),
            ("", Some(_)) => self.free_profile_name(&equalizer_profile_label(""), None),
            (name, _) => self.free_profile_name(name, None),
        };
        let values = &mut self.values;
        values.equalizer_profile = values.equalizer_profiles.len();
        values.equalizer_profiles.push(Profile {
            name,
            ..Profile::of(curve)
        });
        self.schedule_save(cx);
    }

    /// Renames profile `index` to `name` trimmed, numbered when another profile shows it
    /// already, while it is still the profile named `from`. Returns false for an empty name.
    pub fn rename_equalizer_profile(
        &mut self,
        index: usize,
        from: &str,
        name: &str,
        cx: &mut Cx,
    ) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let Some(profile) = self.values.equalizer_profiles.get(index) else {
            return true;
        };
        if profile.name != from || equalizer_profile_label(&profile.name) == name {
            return true;
        }
        let name = self.free_profile_name(name, Some(index));
        let values = &mut self.values;
        let old = std::mem::replace(&mut values.equalizer_profiles[index].name, name.clone());
        for rule in &mut values.equalizer_rules {
            if rule.sound == Sound::Profile(old.clone()) {
                rule.sound = Sound::Profile(name.clone());
            }
        }
        self.schedule_save(cx);
        true
    }

    /// Removes profile `index` and the rules that play it, unless it is the only one or no
    /// longer the profile named `name`. When it was the one playing, its neighbour plays.
    pub fn remove_equalizer_profile(&mut self, index: usize, name: &str, cx: &mut Cx) {
        let values = &mut self.values;
        let named = values
            .equalizer_profiles
            .get(index)
            .is_some_and(|profile| profile.name == name);
        if !named || values.equalizer_profiles.len() < 2 {
            return;
        }
        values.equalizer_profiles.remove(index);
        if !name.is_empty() {
            let gone = Sound::Profile(name.to_owned());
            values.equalizer_rules.retain(|rule| rule.sound != gone);
        }
        if index < values.equalizer_profile {
            values.equalizer_profile -= 1;
        }
        values.equalizer_profile = values
            .equalizer_profile
            .min(values.equalizer_profiles.len() - 1);
        self.schedule_save(cx);
    }

    /// `name`, or `name` with the lowest number after it that leaves it unlike every profile's
    /// shown name, profile `except` left out.
    fn free_profile_name(&self, name: &str, except: Option<usize>) -> String {
        let taken = |candidate: &str| {
            self.values
                .equalizer_profiles
                .iter()
                .enumerate()
                .any(|(index, profile)| {
                    Some(index) != except && equalizer_profile_label(&profile.name) == candidate
                })
        };
        (1..)
            .map(|number| match number {
                1 => name.to_owned(),
                number => format!("{name} {number}"),
            })
            .find(|candidate| !taken(candidate))
            .unwrap_or_else(|| name.to_owned())
    }

    /// Where the stored profile named `name` sits. An empty name finds nothing, since presets
    /// load into the unnamed profile rather than play it.
    pub fn equalizer_profile_named(&self, name: &str) -> Option<usize> {
        self.values
            .equalizer_profiles
            .iter()
            .position(|profile| !name.is_empty() && profile.name == name)
    }

    /// The device rules in the order they show, the rule for every other device last.
    pub fn equalizer_rules(&self) -> &[DeviceRule] {
        &self.values.equalizer_rules
    }

    /// The rule output `id` goes by: its own, else the one for every other device. `None` when
    /// there is neither, so what plays stays.
    pub fn equalizer_rule_for_output(&self, id: &str) -> Option<&DeviceRule> {
        let rules = &self.values.equalizer_rules;
        rules
            .iter()
            .find(|rule| rule.device.id() == Some(id))
            .or_else(|| rules.iter().find(|rule| rule.device == RuleDevice::Other))
    }

    /// Adds `rule` in place of the rule its device had, else before the rule for every other
    /// device, which stays last.
    pub fn set_equalizer_rule(&mut self, rule: DeviceRule, cx: &mut Cx) {
        let rules = &mut self.values.equalizer_rules;
        match rules
            .iter_mut()
            .find(|held| held.device.id() == rule.device.id())
        {
            Some(held) => *held = rule,
            None => {
                let at = rules
                    .iter()
                    .position(|held| held.device == RuleDevice::Other)
                    .unwrap_or(rules.len());
                rules.insert(at, rule);
            }
        }
        self.schedule_save(cx);
    }

    /// Removes the rule for `device`, if it has one.
    pub fn remove_equalizer_rule(&mut self, device: &RuleDevice, cx: &mut Cx) {
        let rules = &mut self.values.equalizer_rules;
        let before = rules.len();
        rules.retain(|rule| rule.device.id() != device.id());
        if rules.len() < before {
            self.schedule_save(cx);
        }
    }

    /// Stores `curve` as the profile that plays, keeping its name.
    pub fn set_equalizer_curve(&mut self, curve: &Curve, cx: &mut Cx) {
        let values = &mut self.values;
        match values.equalizer_profiles.get_mut(values.equalizer_profile) {
            Some(profile) => profile.store(curve),
            None => {
                values.equalizer_profile = values.equalizer_profiles.len();
                values.equalizer_profiles.push(Profile::of(curve));
            }
        }
        self.schedule_save(cx);
    }
}

/// The name a profile shows: its stored name, or the interface's label for the empty one
/// Sonora made.
pub fn equalizer_profile_label(stored: &str) -> SharedString {
    match stored.is_empty() {
        true => i18n::t!("equalizer-profile-default"),
        false => SharedString::from(stored.to_owned()),
    }
}

/// The device rules as stored: every entry that reads as a rule, the rest dropped and logged,
/// and anything but a list read as none.
pub(super) fn read_rules<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<DeviceRule>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else {
        log::warn!(
            "settings: equalizer_rules is not a list, starting with none; the stored value is replaced the next time settings are saved"
        );
        return Ok(Vec::new());
    };
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let rule = read_rule(&item);
            if rule.is_none() {
                log::warn!(
                    "settings: dropping the equalizer rule {item}, which needs an output and a known preset or a profile name; it is gone the next time settings are saved"
                );
            }
            rule
        })
        .collect())
}

/// Writes the device rules as `read_rules` reads them.
pub(super) fn write_rules<S: Serializer>(
    rules: &[DeviceRule],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(rules.iter().map(|rule| {
        let output = match &rule.device {
            RuleDevice::Output(output) => StoredDevice::Output(output),
            RuleDevice::Other => StoredDevice::Other(OTHER_OUTPUTS),
        };
        let (preset, profile) = match &rule.sound {
            Sound::Preset(preset) => (Some(preset.id()), None),
            Sound::Profile(name) => (None, Some(name.as_str())),
        };
        StoredRule {
            output,
            preset,
            profile,
        }
    }))
}

/// The band gains an older Sonora saved: a band that is not a number reads as 0 dB, so the
/// bands after it keep their places, and anything but a list reads as none.
pub(super) fn read_gains<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<f32>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(gains) = value else {
        log::warn!(
            "settings: equalizer_bands {value} is not a list, ignoring it; the key is dropped the next time settings are saved"
        );
        return Ok(Vec::new());
    };
    Ok(gains
        .iter()
        .map(|gain| match gain.as_f64() {
            Some(gain) => gain as f32,
            None => {
                log::warn!(
                    "settings: equalizer band {gain} is not a number, taking 0 dB; equalizer_bands is dropped the next time settings are saved"
                );
                0.
            }
        })
        .collect())
}

/// Field `key` of a stored equalizer profile, or `None` with a warning when it is there but does
/// not read as a `T`, so one mistyped field never drops the profile.
fn profile_field<T: DeserializeOwned>(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<T> {
    serde_json::from_value(fields.remove(key)?)
        .map_err(|error| {
            log::warn!(
                "settings: an equalizer profile's {key} does not read, taking the default; the stored value is replaced the next time settings are saved: {error}"
            );
        })
        .ok()
}

/// One stored device rule, or `None` when its output or its sound does not read.
fn read_rule(item: &serde_json::Value) -> Option<DeviceRule> {
    let device = match item.get("output")? {
        serde_json::Value::String(word) if word == OTHER_OUTPUTS => RuleDevice::Other,
        output => serde_json::from_value::<OutputId>(output.clone())
            .ok()
            .filter(|output| !output.id.is_empty())
            .map(RuleDevice::Output)?,
    };
    let text = |key| item.get(key).and_then(serde_json::Value::as_str);
    let sound = match (text("preset"), text("profile")) {
        (Some(id), _) => Sound::Preset(Preset::ALL.into_iter().find(|preset| preset.id() == id)?),
        (None, Some(name)) if !name.is_empty() => Sound::Profile(name.to_owned()),
        _ => return None,
    };
    Some(DeviceRule { device, sound })
}

/// The profile index as stored: any number clamped at zero, and anything else as zero, so a
/// hand-edited mistake picks the first profile instead of making the file unreadable.
/// A bass or treble gain in decibels, or 0 dB with a warning when the stored value is not a
/// number. `Tone::new` clamps it into range.
pub(super) fn read_tone<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let gain = value.as_f64();
    if gain.is_none() {
        log::warn!(
            "settings: equalizer tone gain {value} is not a number, taking 0 dB; the stored value is replaced the next time settings are saved"
        );
    }
    Ok(gain.unwrap_or(0.))
}

pub(super) fn read_index<'de, D: Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    if !value.is_number() {
        log::warn!(
            "settings: equalizer_profile {value} is not a number, taking the first; the stored value is replaced the next time settings are saved"
        );
    }
    Ok(value.as_f64().unwrap_or(0.).max(0.) as usize)
}
