//! The equalizer the app plays through: the curve every engine reads, the profiles the settings
//! store it in, undo and redo, the sound each device rule asks for, and the clip warning the
//! output meter raises against it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use music::Meter;
use music::equalizer::{Curve, Edit, EqualizerWriter, PREAMP_RANGE, Preamp, Preset, Tone, apo};
use music::output::OutputId;

use crate::overload::{OutputLevel, Overload};
use crate::{AppSettings, Change, Cx, Event, Job, Store, Topic};

/// How long after the last glided edit the curve is stored, so a drag or a run of wheel steps
/// reaches the settings once, when it pauses.
const GLIDE_STORE: Duration = Duration::from_millis(300);
/// How many equalizer edits can be undone.
const UNDO_STEPS: usize = 50;

/// The equalizer as it plays. Created once in `Sonora`, it hands every engine the same reader
/// and meter, so a change reaches the output without a restart. It follows the output the sound
/// plays on and plays the sound that output's device rule names.
pub struct Equalizer {
    writer: EqualizerWriter,
    /// The meter every engine reports to, with the level and the clip warning made of it, which
    /// the signal's tick feeds off the UI thread while watched.
    metering: Metering,
    /// Stores a glided curve in the settings once the edits pause.
    glide_store: Option<Job>,
    /// Stores a glided tone in the settings once the edits pause.
    tone_store: Option<Job>,
    history: History,
    /// The id of the output played on as `Outputs` last settled it, so a rule added for it can
    /// play at once and the same output again is not a move.
    output: Option<String>,
    /// The volume as playback last had it, so a change to it takes the clip warning down. `None`
    /// until playback first changed.
    volume: Option<f32>,
}

/// The output meter and what is made of it, shared between the equalizer and the signal's tick,
/// which reads it about sixty times a second without waking the UI thread.
#[derive(Clone, Default)]
pub(crate) struct Metering {
    /// Shared with every engine like the equalizer's reader, and read only while watched.
    meter: Meter,
    gauges: Arc<Mutex<Gauges>>,
}

/// What the meter's reads make: the clip warning and the level a meter draws.
#[derive(Default)]
struct Gauges {
    overload: Overload,
    /// The output's level as the meter last read it.
    level: OutputLevel,
}

/// What one meter read moved.
pub(crate) struct Read {
    /// Whether the level meter moved by a visible step.
    pub(crate) level: bool,
    /// Whether the clip warning changed.
    pub(crate) overload: bool,
}

/// A sound a device rule plays: a built-in preset, loaded into the unnamed profile the way
/// picking it does, or a stored profile by its name. The unnamed profile is never one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sound {
    Preset(Preset),
    Profile(String),
}

/// Which output a device rule is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleDevice {
    /// One output, by the key `Outputs::binding_key` gives it. Its name is only shown.
    Output(OutputId),
    /// Every output no other rule names.
    Other,
}

/// The sound to play once the sound moves to a device. A device has one rule at most.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRule {
    pub device: RuleDevice,
    pub sound: Sound,
}

/// The playing profile's curves from before each edit and from before each undo, newest last.
/// It is kept in memory only and emptied when another profile plays.
#[derive(Default)]
struct History {
    undo: Vec<Curve>,
    redo: Vec<Curve>,
}

impl RuleDevice {
    /// The output's key, or `None` for every other output. Two rules are for the same device
    /// when their keys are equal, whatever names they were seen with.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Output(output) => Some(&output.id),
            Self::Other => None,
        }
    }
}

impl History {
    /// Remembers `curve`, the one an edit is about to replace, and forgets what was undone.
    fn record(&mut self, curve: &Curve) {
        if self.undo.len() == UNDO_STEPS {
            self.undo.remove(0);
        }
        self.undo.push(curve.clone());
        self.redo.clear();
    }

    /// Steps back from `current` and returns the curve to play instead.
    fn undo(&mut self, current: &Curve) -> Option<Curve> {
        let curve = self.undo.pop()?;
        self.redo.push(current.clone());
        Some(curve)
    }

    /// Steps forward from `current` again and returns the curve to play instead.
    fn redo(&mut self, current: &Curve) -> Option<Curve> {
        let curve = self.redo.pop()?;
        self.undo.push(current.clone());
        Some(curve)
    }
}

impl Equalizer {
    /// Plays what the settings hold. It follows the output and the volume through
    /// [`Store::react_equalizer`] once `Outputs` and playback raise their changes.
    pub fn new(settings: &AppSettings) -> Self {
        let mut writer = EqualizerWriter::new(settings.equalizer(), settings.equalizer_curve());
        writer.set_tone(settings.equalizer_tone(), Edit::Replace);
        Self {
            writer,
            metering: Metering::default(),
            glide_store: None,
            tone_store: None,
            history: History::default(),
            output: None,
            volume: None,
        }
    }

    /// The reader every engine plays the curve through.
    pub(crate) fn reader(&self) -> music::Equalizer {
        self.writer.reader()
    }

    /// The meter every engine's output reports its loudest sample to.
    pub(crate) fn meter(&self) -> Meter {
        self.metering.meter.clone()
    }

    /// The meter with the level and the clip warning it feeds, for the signal's tick.
    pub(crate) fn metering(&self) -> Metering {
        self.metering.clone()
    }

    pub fn enabled(&self) -> bool {
        self.writer.enabled()
    }

    /// The playing sound's own curve, the one profiles store and the filter list edits.
    pub fn curve(&self) -> &Curve {
        self.writer.curve()
    }

    /// The bass and treble laid over every sound.
    pub fn tone(&self) -> Tone {
        self.writer.tone()
    }

    /// The curve heard: the sound's own with the tone's shelves after it, and the preamp that
    /// makes room for both.
    pub fn played(&self) -> &Curve {
        self.writer.played()
    }

    /// Puts `tone` in place and stores it, the engines cross-fading to it. It turns the equalizer
    /// on like picking a sound does, since a tone is only heard through it.
    pub fn set_tone(&mut self, tone: Tone, settings: &mut AppSettings, cx: &mut Cx) {
        self.tone_store = None;
        self.writer.set_tone(tone, Edit::Replace);
        self.toned(settings, cx);
        self.store_tone(settings, cx);
    }

    /// Puts `tone` in place the way a slider drag does: the engines glide to it, and the settings
    /// store it once the edits pause.
    pub fn glide_tone(&mut self, tone: Tone, settings: &mut AppSettings, cx: &mut Cx) {
        if tone == self.writer.tone() {
            return;
        }
        self.writer.set_tone(tone, Edit::Glide);
        self.toned(settings, cx);
        self.tone_store = Some(cx.after(GLIDE_STORE, |store| {
            let Store {
                equalizer: this,
                settings,
                cx,
                ..
            } = store;
            this.tone_store = None;
            this.store_tone(settings, cx);
        }));
    }

    /// Turns the equalizer on or off. The engines pick the change up on their next frame and
    /// fade to it, so no restart and no click.
    pub fn set_enabled(&mut self, on: bool, settings: &mut AppSettings, cx: &mut Cx) {
        self.writer.set_enabled(on);
        self.gauges().overload.settle(Instant::now());
        settings.set_equalizer(on, cx);
        cx.changed(Topic::Equalizer);
    }

    /// Puts a whole curve in place of the playing profile's, the way a preset pick or a typed
    /// value does: the engines cross-fade to it.
    pub fn set_curve(&mut self, curve: Curve, settings: &mut AppSettings, cx: &mut Cx) {
        if curve == *self.writer.curve() {
            return;
        }
        self.history.record(self.writer.curve());
        self.replace_curve(curve, settings, cx);
    }

    /// Whether there is an edit to undo, and whether there is an undo to redo.
    pub fn history(&self) -> (bool, bool) {
        (!self.history.undo.is_empty(), !self.history.redo.is_empty())
    }

    /// Plays the curve from before the last edit, cross-fading to it.
    pub fn undo(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        if let Some(curve) = self.history.undo(self.writer.curve()) {
            self.replace_curve(curve, settings, cx);
        }
    }

    /// Plays the curve the last undo took away again.
    pub fn redo(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        if let Some(curve) = self.history.redo(self.writer.curve()) {
            self.replace_curve(curve, settings, cx);
        }
    }

    /// Puts a curve in place that moves one filter a little from the playing one, the way a
    /// drag does: the engines glide it there. The settings store it once the edits pause.
    pub fn glide_curve(&mut self, curve: Curve, cx: &mut Cx) {
        if curve == *self.writer.curve() {
            return;
        }
        // A glide waiting to be stored belongs to the gesture that started it.
        if self.glide_store.is_none() {
            self.history.record(self.writer.curve());
        }
        self.writer.set_curve(curve, Edit::Glide);
        self.gauges().overload.settle(Instant::now());
        self.glide_store = Some(cx.after(GLIDE_STORE, |store| {
            let Store {
                equalizer: this,
                settings,
                cx,
                ..
            } = store;
            this.store_glided(settings, cx);
        }));
        cx.changed(Topic::Equalizer);
    }

    /// The preamp the curve heard resolves to, in decibels, with room for the tone's boost.
    pub fn preamp_db(&self) -> f64 {
        self.writer.preamp_db()
    }

    /// How many decibels over full scale the clip warning says the output went, to a tenth,
    /// while something watches. It changes at most about once a second, and the equalizer
    /// raises [`Topic::Equalizer`] only when it does.
    pub fn overload(&self) -> Option<f64> {
        self.gauges().overload.db()
    }

    /// The preamp "Lower preamp" would set, or `None` without an overload or when the preamp
    /// is as low as it goes.
    pub fn lowered_preamp(&self) -> Option<f64> {
        self.gauges().overload.lowered(self.writer.preamp_db())
    }

    /// The output's recent level and its held peak, in dBFS. The signal's watch moves it about
    /// sixty times a second without raising anything, so a level meter reads it on every frame
    /// it draws, and [`crate::Signal::take_fresh`] says when it moved.
    pub fn output_level(&self) -> (f32, f32) {
        let gauges = self.gauges();
        (gauges.level.level_db(), gauges.level.peak_db())
    }

    /// Starts the level meter and the clip warning afresh, from no overload, for a screen that
    /// starts watching. [`Store::watch_signal`] then reads the meter on its own tick.
    pub(crate) fn reset_meter(&mut self, cx: &mut Cx) {
        let mut gauges = self.gauges();
        gauges.overload = Overload::default();
        self.metering.meter.take();
        gauges.level.reset();
        drop(gauges);
        cx.changed(Topic::Equalizer);
    }

    /// Lowers the preamp by the overload and a margin, made manual, so what overloaded fits. The
    /// sound stores its own preamp, which the tone lowers further, so the stored one keeps the
    /// difference between the two.
    pub fn lower_preamp(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        let Some(preamp) = self.lowered_preamp() else {
            return;
        };
        let own = self.writer.curve().preamp_db() - self.writer.preamp_db();
        let preamp = (preamp + own).clamp(*PREAMP_RANGE.start(), *PREAMP_RANGE.end());
        let filters = self.writer.curve().filters().to_vec();
        if let Ok(curve) = Curve::new(Preamp::Manual(preamp), filters) {
            self.set_curve(curve, settings, cx);
        }
    }

    /// Adds `rule` in place of the one its device had. When it is the rule the output played on
    /// now goes by, its sound plays at once.
    pub fn set_rule(&mut self, rule: DeviceRule, settings: &mut AppSettings, cx: &mut Cx) {
        let device = rule.device.clone();
        settings.set_equalizer_rule(rule, cx);
        let governs = self.output.as_deref().is_some_and(|id| {
            settings
                .equalizer_rule_for_output(id)
                .is_some_and(|rule| rule.device.id() == device.id())
        });
        if governs {
            self.play_output_sound(settings, cx);
        }
    }

    /// Removes the rule for `device`. What plays stays.
    pub fn remove_rule(&mut self, device: &RuleDevice, settings: &mut AppSettings, cx: &mut Cx) {
        settings.remove_equalizer_rule(device, cx);
    }

    /// Plays `sound` and turns the equalizer on, the way picking it does. A profile that is no
    /// longer stored plays nothing.
    pub fn play_sound(&mut self, sound: &Sound, settings: &mut AppSettings, cx: &mut Cx) {
        if self.load_sound(sound, settings, cx) && !self.enabled() {
            self.set_enabled(true, settings, cx);
        }
    }

    /// Plays profile `index`, cross-fading to its curve.
    pub fn select_profile(&mut self, index: usize, settings: &mut AppSettings, cx: &mut Cx) {
        self.store_glided(settings, cx);
        settings.select_equalizer_profile(index, cx);
        self.follow_settings(settings, cx);
    }

    /// Plays profile `index` and turns the equalizer on, the way picking a sound does.
    pub fn play_profile(&mut self, index: usize, settings: &mut AppSettings, cx: &mut Cx) {
        self.select_profile(index, settings, cx);
        if !self.enabled() {
            self.set_enabled(true, settings, cx);
        }
    }

    /// Plays `preset` in the profile with no name, switching to it first when another plays, and
    /// turns the equalizer on. A named profile is never overwritten.
    pub fn play_preset(&mut self, preset: Preset, settings: &mut AppSettings, cx: &mut Cx) {
        self.load_preset(preset, settings, cx);
        if !self.enabled() {
            self.set_enabled(true, settings, cx);
        }
    }

    /// Puts `preset` in the profile with no name and plays that profile, leaving the equalizer on
    /// or off.
    fn load_preset(&mut self, preset: Preset, settings: &mut AppSettings, cx: &mut Cx) {
        let curve = preset.curve();
        if settings.equalizer_scratch() != Some(settings.equalizer_profile()) {
            self.store_glided(settings, cx);
            settings.select_equalizer_scratch(&curve, cx);
            self.follow_settings(settings, cx);
        }
        self.set_curve(curve, settings, cx);
    }

    /// Switches to `sound` and leaves the equalizer on or off. A stored profile that plays
    /// already keeps its undo history. Returns false when `sound` names no stored profile.
    fn load_sound(&mut self, sound: &Sound, settings: &mut AppSettings, cx: &mut Cx) -> bool {
        match sound {
            Sound::Preset(preset) => self.load_preset(*preset, settings, cx),
            Sound::Profile(name) => {
                let Some(index) = settings.equalizer_profile_named(name) else {
                    return false;
                };
                if index != settings.equalizer_profile() {
                    self.select_profile(index, settings, cx);
                }
            }
        }
        true
    }

    /// Adds a profile named after `name` holding `curve` and plays it. `AppSettings` makes the
    /// name unique.
    pub fn add_profile(
        &mut self,
        name: &str,
        curve: Curve,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) {
        self.store_glided(settings, cx);
        settings.add_equalizer_profile(name, &curve, cx);
        self.follow_settings(settings, cx);
    }

    /// Saves the playing curve as a new profile named after `name` and plays it. When the curve
    /// was in the profile with no name, that profile goes back to flat. `AppSettings` makes the
    /// name unique.
    pub fn save_as(&mut self, name: &str, settings: &mut AppSettings, cx: &mut Cx) {
        self.store_glided(settings, cx);
        let curve = self.writer.curve().clone();
        if settings.equalizer_scratch() == Some(settings.equalizer_profile()) {
            settings.set_equalizer_curve(&Preset::Flat.curve(), cx);
        }
        settings.add_equalizer_profile(name, &curve, cx);
        self.follow_settings(settings, cx);
    }

    /// Removes profile `index` while it is still the one named `name`, unless it is the only
    /// one, and the rules that play it. When it was playing, the sound the output's rule names
    /// plays, else the profile `AppSettings` picks.
    pub fn remove_profile(
        &mut self,
        index: usize,
        name: &str,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) {
        self.store_glided(settings, cx);
        let playing = settings.equalizer_profile() == index;
        settings.remove_equalizer_profile(index, name, cx);
        self.follow_settings(settings, cx);
        if let Some(sound) = self.output_sound(settings).filter(|_| playing) {
            self.load_sound(&sound, settings, cx);
        }
    }

    /// Reads an Equalizer APO or AutoEQ text into a new profile named after `name` and plays
    /// it. Returns the lines it left out, or why nothing was imported.
    pub fn import(
        &mut self,
        name: &str,
        text: &str,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) -> Result<Vec<apo::Skipped>, apo::Refused> {
        let parsed = apo::parse(text)?;
        self.add_profile(name, parsed.curve, settings, cx);
        Ok(parsed.skipped)
    }

    /// The playing curve as an Equalizer APO text.
    pub fn export(&self) -> String {
        apo::export(self.writer.curve())
    }

    /// The clip warning and the level meter, behind the lock the signal's tick shares.
    fn gauges(&self) -> MutexGuard<'_, Gauges> {
        self.metering.gauges()
    }

    /// Follows a tone that was just put in place: the clip warning starts afresh, and the
    /// equalizer turns on when it was off.
    fn toned(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        self.gauges().overload.settle(Instant::now());
        match self.enabled() {
            true => cx.changed(Topic::Equalizer),
            false => self.set_enabled(true, settings, cx),
        }
    }

    fn store_tone(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        let tone = self.writer.tone();
        settings.set_equalizer_tone(tone, cx);
    }

    fn replace_curve(&mut self, curve: Curve, settings: &mut AppSettings, cx: &mut Cx) {
        self.writer.set_curve(curve, Edit::Replace);
        self.gauges().overload.settle(Instant::now());
        self.save_curve(settings, cx);
    }

    /// Stores a glided curve that waits for the edits to pause now, so a profile change that
    /// follows keeps it in the profile it was made in.
    fn store_glided(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        if self.glide_store.take().is_some() {
            self.save_curve(settings, cx);
        }
    }

    /// Takes in output `id` as the one played on, and plays the sound its rule names when it is
    /// another output than the last one.
    fn follow_output(&mut self, id: String, settings: &mut AppSettings, cx: &mut Cx) {
        if self.output.as_ref() == Some(&id) {
            return;
        }
        self.output = Some(id);
        self.play_output_sound(settings, cx);
    }

    /// The sound the output played on asks for: its own rule's, else the one for any other
    /// device.
    fn output_sound(&self, settings: &AppSettings) -> Option<Sound> {
        let id = self.output.as_deref()?;
        let rule = settings.equalizer_rule_for_output(id)?;
        Some(rule.sound.clone())
    }

    /// Plays the sound the output played on asks for and turns the equalizer on. Nothing changes
    /// when no rule applies.
    fn play_output_sound(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        let Some(sound) = self.output_sound(settings) else {
            return;
        };
        log::info!("equalizer: playing {sound:?}, the sound for the output played on");
        self.play_sound(&sound, settings, cx);
    }

    /// Plays what the settings say, the equalizer on or off and the selected profile's curve,
    /// as one replace, so the sound, the picker and the screen agree after a profile change. A
    /// tone still waiting to be stored is newer than the settings' and stays.
    fn follow_settings(&mut self, settings: &AppSettings, cx: &mut Cx) {
        let (on, curve) = (settings.equalizer(), settings.equalizer_curve());
        if self.tone_store.is_none() {
            self.writer
                .set_tone(settings.equalizer_tone(), Edit::Replace);
        }
        self.history = History::default();
        self.gauges().overload.settle(Instant::now());
        // Whichever change is heard goes last, so only one replace reaches the ear.
        match on {
            true => {
                self.writer.set_curve(curve, Edit::Replace);
                self.writer.set_enabled(true);
            }
            false => {
                self.writer.set_enabled(false);
                self.writer.set_curve(curve, Edit::Replace);
            }
        }
        cx.changed(Topic::Equalizer);
    }

    fn save_curve(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        self.glide_store = None;
        let curve = self.writer.curve().clone();
        settings.set_equalizer_curve(&curve, cx);
        cx.changed(Topic::Equalizer);
    }
}

impl Metering {
    /// Reads the meter once, the loudest sample since the read before, feeding the level meter
    /// and the clip warning. Called off the UI thread by the signal's tick.
    pub(crate) fn read(&self, now: Instant) -> Read {
        let peak = self.meter.take();
        let mut gauges = self.gauges();
        Read {
            level: gauges.level.observe(peak, now),
            overload: gauges.overload.observe(peak, now),
        }
    }

    fn gauges(&self) -> MutexGuard<'_, Gauges> {
        self.gauges.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store {
    /// Follows a settings file read again, the volume, which takes the clip warning down when it
    /// changes, and the output the sound plays on, whose device rule names the sound to play.
    pub(crate) fn react_equalizer(&mut self, change: &Change) {
        let Store {
            equalizer: this,
            settings,
            playback,
            outputs,
            cx,
            ..
        } = self;
        match change {
            Change::Event(Event::SettingsReloaded) => {
                // The file's curve and tone win over glides that were still to be stored.
                this.glide_store = None;
                this.tone_store = None;
                this.follow_settings(settings, cx)
            }
            Change::Topic(Topic::Playback) => {
                let volume = playback.volume();
                if this.volume.replace(volume).is_some_and(|was| was != volume) {
                    this.gauges().overload.settle(Instant::now());
                }
            }
            Change::Topic(Topic::Outputs) => {
                let playing = outputs.binding_key().map(|output| output.id);
                if let Some(id) = playing {
                    this.follow_output(id, settings, cx);
                }
            }
            _ => {}
        }
    }
}
