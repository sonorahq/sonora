use std::time::Duration;

use anyhow::Result;
use music::output::{self, DeviceInfo, DeviceOption, Failure, Opened, OutputId, Sink};

use crate::{AppSettings, Cx, Event, Job, Outcome, Store, Toast, Topic, join};

/// How long a newly picked device holds the picker before it is let go, whether or not it has
/// answered. CoreAudio takes about ten seconds to give up on a device that will not start.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a picked device's stream is kept open for the restarted engines to join it.
/// TODO: Investigate if we should use a signal instead of a duration.
/// Perhaps rearchitect SHARED
const HANDOVER: Duration = Duration::from_secs(2);

/// The output devices the system offers, the one chosen, and what the stream Sonora plays into
/// runs at. The list is read off the main thread, since that opens devices on some platforms, and
/// both are read again whenever a stream opens or closes or its sink changes. A device picked
/// here is opened first and only moved onto, and saved, once it has. One that will not open
/// leaves everything as it was and says so in a toast. `binding_key` names the output the sound
/// plays on, which the equalizer follows to play the sound its device rule asks for. A change
/// raises [`Topic::Outputs`]. The host calls [`Outputs::on_quit`] as the app quits.
pub struct Outputs {
    devices: Vec<DeviceOption>,
    /// Whether `devices` has been read at least once, so an empty list means none.
    listed: bool,
    active: Option<DeviceInfo>,
    /// The device a stream was last open on, kept while the stream reopens so what it offers
    /// stays on screen.
    known: Option<DeviceInfo>,
    failure: Option<Failure>,
    /// Whether nothing is heard because no stream would open.
    failing: bool,
    /// The device chosen, by cpal id, or `None` to follow the system's default.
    selected: Option<String>,
    /// A device picked and being opened, shown as chosen until it answers.
    trying: Option<Option<String>>,
    /// Opens the picked device off the main thread. Dropping it drops the pick.
    opening: Option<Job>,
    /// Runs out `SWITCH_TIMEOUT` after a pick. While it is set the picker is held.
    holding: Option<Job>,
    /// The picked device's stream, kept open until the engines have joined it.
    handover: Option<(Opened, Job)>,
    listing: Option<Job>,
    _watch: Job,
    /// Why the chosen device could not be taken for direct output, the last time it tried. It is
    /// toasted when it first appears.
    hold_failure: Option<String>,
    /// Why the device could not be switched to a rate, the last time it tried. It is toasted when
    /// it first appears.
    rate_failure: Option<String>,
}

impl Outputs {
    /// Hands the device, rate and buffer saved in settings to the engines, so call it before the
    /// first one starts.
    pub fn new(settings: &AppSettings, cx: &mut Cx) -> Self {
        let selected = settings.output_device().map(str::to_owned);
        output::choose_device(selected.clone());
        output::choose_rate(settings.output_rate());
        output::choose_buffer(settings.output_buffer());
        output::choose_match_rate(settings.output_match_rate());

        let mut changes = output::watch_device_changes();
        let watch = cx.spawn(|link| async move {
            while changes.changed().await.is_ok() {
                link.post(|store| {
                    let Store {
                        outputs: this,
                        settings,
                        cx,
                        ..
                    } = store;
                    this.read_device();
                    this.failure = output::latest_device_failure();
                    this.read_direct(cx);
                    this.failing = output::failing();
                    this.forget_broken(settings, cx);
                    this.refresh(cx);
                    cx.changed(Topic::Outputs);
                });
            }
        });

        let mut outputs = Self {
            devices: Vec::new(),
            listed: false,
            active: output::active_device_info(),
            known: output::active_device_info(),
            failure: output::latest_device_failure(),
            failing: output::failing(),
            selected,
            trying: None,
            opening: None,
            holding: None,
            handover: None,
            listing: None,
            hold_failure: output::hold_failure(),
            rate_failure: output::rate_failure(),
            _watch: watch,
        };
        outputs.refresh(cx);
        outputs
    }

    /// Lets the device go, so it is not left at a track's rate. The host calls it once as the
    /// app quits.
    pub fn on_quit(&mut self) {
        output::release_hold();
    }

    pub fn devices(&self) -> &[DeviceOption] {
        &self.devices
    }

    /// Whether the system's devices have been read yet.
    pub fn listed(&self) -> bool {
        self.listed
    }

    /// The device Sonora plays on, or `None` while no stream is open.
    pub fn active(&self) -> Option<&DeviceInfo> {
        self.active.as_ref()
    }

    /// The device playing, or while its stream reopens, the one that last played.
    pub fn current(&self) -> Option<&DeviceInfo> {
        self.active.as_ref().or(self.known.as_ref())
    }

    /// The key device rules know the output the sound plays on by, from `output::output_key`,
    /// with the device's name. That is the device Sonora's stream is open on, or last was, and
    /// before any stream opens the device one would open on, so a rule applies from the start.
    pub fn binding_key(&self) -> Option<OutputId> {
        let (id, name, sink) = self.playing()?;
        Some(OutputId {
            id: output::output_key(id, sink.and_then(Sink::port)),
            name: name.to_owned(),
        })
    }

    /// What the sound server says of the sink the sound plays on, for one of its own outputs.
    pub fn playing_sink(&self) -> Option<&Sink> {
        self.playing()?.2
    }

    /// The id, name and sink of the device the sound plays on: the open or last open one, else
    /// the chosen device while it is listed, else the system's default.
    fn playing(&self) -> Option<(&str, &str, Option<&Sink>)> {
        if let Some(device) = self.current() {
            return Some((&device.id, &device.name, device.sink.as_ref()));
        }
        let device = self
            .devices
            .iter()
            .find(|device| self.selected.as_deref() == Some(device.id.as_str()))
            .or_else(|| self.devices.iter().find(|device| device.system))?;
        Some((&device.id, &device.name, device.sink.as_ref()))
    }

    /// Reads the device Sonora plays on again.
    fn read_device(&mut self) {
        self.active = output::active_device_info();
        if self.active.is_some() {
            self.known.clone_from(&self.active);
        }
    }

    /// Reads why the chosen device could not be taken, or switched to a rate, again. A failure
    /// that is new is toasted once, since playback goes on without it.
    fn read_direct(&mut self, cx: &mut Cx) {
        let taken = output::hold_failure();
        if taken != self.hold_failure {
            if let Some(error) = taken.clone() {
                log::warn!("playback: cannot play directly on the device: {error}");
                cx.emit(Event::Toast(
                    Toast::new(Outcome::Failed, "toast-output-direct-failed").detail(error),
                ));
            }
            self.hold_failure = taken;
        }
        let rate = output::rate_failure();
        if rate != self.rate_failure {
            if let Some(error) = rate.clone() {
                log::warn!("playback: cannot switch the sample rate: {error}");
                cx.emit(Event::Toast(
                    Toast::new(Outcome::Failed, "toast-output-rate-failed").detail(error),
                ));
            }
            self.rate_failure = rate;
        }
    }

    /// Whether this system can follow each track's sample rate, which is for the settings page to
    /// show.
    pub fn match_rate_supported(&self) -> bool {
        output::match_rate_supported()
    }

    /// Whether following each track's sample rate is turned on in settings.
    pub fn match_rate(&self, settings: &AppSettings) -> bool {
        self.match_rate_supported() && settings.output_match_rate()
    }

    /// Whether the device chosen is played on directly, which is the only kind whose rate can be
    /// switched.
    pub fn direct_chosen(&self) -> bool {
        self.chosen().is_some_and(output::is_direct)
    }

    /// Whether the rate follows each track right now: it is turned on, and the stream open is on
    /// a direct device.
    pub fn following(&self, settings: &AppSettings) -> bool {
        self.match_rate(settings) && self.current().is_some_and(|device| device.direct)
    }

    /// Turns following each track's sample rate on or off, and saves it. The engines have to
    /// reopen for it to be heard, which [`Store::set_output_match_rate`] does too.
    pub fn set_match_rate(&mut self, on: bool, settings: &mut AppSettings, cx: &mut Cx) {
        settings.set_output_match_rate(on, cx);
        output::choose_match_rate(on);
        cx.changed(Topic::Outputs);
    }

    /// Why nothing plays, when the device to play on would not open and nothing took its place.
    pub fn failure(&self) -> Option<&Failure> {
        self.failure.as_ref().filter(|_| self.active.is_none())
    }

    /// Whether nothing is heard because no stream would open: every device to try would not open,
    /// or there was none.
    pub fn failing(&self) -> bool {
        self.failing
    }

    /// The device playing or being tried, or `None` when Sonora follows the system's default.
    /// It may not be connected, in which case Sonora plays on the default until it is.
    pub fn chosen(&self) -> Option<&str> {
        match &self.trying {
            Some(trying) => trying.as_deref(),
            None => self.selected.as_deref(),
        }
    }

    /// The rate the device plays at, or is reopening at once a new one is chosen: the one saved in
    /// settings when the device offers it, otherwise the one it runs at.
    pub fn rate(&self, settings: &AppSettings) -> Option<u32> {
        let active = self.current()?;
        let wanted = settings
            .output_rate()
            .filter(|rate| active.rates.contains(rate));
        Some(wanted.unwrap_or(active.format.rate))
    }

    /// Opens the device at `rate` from now on, and saves it. The engines reopen at it, paused or
    /// not.
    pub fn set_rate(&mut self, rate: u32, settings: &mut AppSettings, cx: &mut Cx) {
        settings.set_output_rate(Some(rate), cx);
        output::choose_rate(Some(rate));
        cx.changed(Topic::Outputs);
    }

    /// The buffer the device plays with, in frames, or is reopening with once a new one is
    /// chosen: the one saved in settings when the device allows it, otherwise the one it has.
    pub fn buffer(&self, settings: &AppSettings) -> Option<u32> {
        let active = self.current()?;
        let wanted = settings
            .output_buffer()
            .filter(|frames| active.buffers.contains(frames));
        Some(wanted.unwrap_or(active.buffer))
    }

    /// Asks the device for a buffer of `frames` from now on, and saves it. The engines reopen
    /// with it, paused or not.
    pub fn set_buffer(&mut self, frames: u32, settings: &mut AppSettings, cx: &mut Cx) {
        settings.set_output_buffer(Some(frames), cx);
        output::choose_buffer(Some(frames));
        cx.changed(Topic::Outputs);
    }

    /// Whether a device was just picked and has not answered yet, so no other can be picked.
    pub fn switching(&self) -> bool {
        self.holding.is_some()
    }

    /// Opens `id`, or the system's default when `None`, and once it opens moves playback onto
    /// it, paused or not, and saves it. A device that will not open leaves playback and the saved
    /// device as they were, with a toast saying so.
    pub fn choose(&mut self, id: Option<String>, cx: &mut Cx) {
        if self.switching() {
            return;
        }
        // the same device again, with nothing wrong, would only cut the sound for a moment
        if id == self.selected && self.failure.is_none() {
            return;
        }
        self.trying = Some(id.clone());
        let wanted = id.clone();
        self.opening = Some(cx.spawn(|link| async move {
            let opened = join(tokio::task::spawn_blocking(move || {
                output::open(wanted.as_deref())
            }))
            .await;
            link.post(move |store| {
                let Store {
                    outputs,
                    settings,
                    cx,
                    ..
                } = store;
                if outputs.picked(id, opened, settings, cx) {
                    store.restart_outputs();
                }
            });
        }));
        self.holding = Some(cx.after(SWITCH_TIMEOUT, |store| {
            store.outputs.holding = None;
            store.cx.changed(Topic::Outputs);
        }));
        cx.changed(Topic::Outputs);
    }

    /// Takes the answer to a pick: saves a device that opened, or toasts that it would not.
    /// Returns whether it opened, after which the caller moves playback onto it.
    fn picked(
        &mut self,
        id: Option<String>,
        opened: Result<Opened>,
        settings: &mut AppSettings,
        cx: &mut Cx,
    ) -> bool {
        self.opening = None;
        self.holding = None;
        self.trying = None;
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) => {
                log::warn!("playback: cannot select the output device: {error:#}");
                self.toast_rejected(id.as_deref(), &error, cx);
                cx.changed(Topic::Outputs);
                return false;
            }
        };
        self.selected = id.clone();
        output::choose_device(id.clone());
        self.save(id, settings, cx);
        let release = cx.after(HANDOVER, |store| store.outputs.handover = None);
        self.handover = Some((opened, release));
        cx.changed(Topic::Outputs);
        true
    }

    /// Forgets the saved device once it is there but would not open, so the next run does not
    /// wait on it again. The system's default already plays in its place. A device that is gone
    /// is never tried, so it never fails and stays saved.
    fn forget_broken(&mut self, settings: &mut AppSettings, cx: &mut Cx) {
        let Some(failure) = self.failure.as_ref().filter(|failure| failure.saved) else {
            return;
        };
        if settings.output_device() != Some(failure.id.as_str()) {
            return;
        }
        log::warn!(
            "playback: forgetting the saved output device, which would not open: {}",
            failure.error
        );
        self.selected = None;
        self.save(None, settings, cx);
        output::choose_device(None);
    }

    /// Says in a toast that the pick `id`, or the system's default when `None`, would not open,
    /// with the error below.
    fn toast_rejected(&self, id: Option<&str>, error: &anyhow::Error, cx: &mut Cx) {
        let error = format!("{error:#}");
        let Some(id) = id else {
            cx.emit(Event::Toast(
                Toast::new(Outcome::Failed, "toast-output-default-failed").detail(error),
            ));
            return;
        };
        let name = self
            .devices
            .iter()
            .find(|device| device.id == id)
            .map_or(id, |device| device.name.as_str())
            .to_owned();
        cx.emit(Event::Toast(
            Toast::new(Outcome::Failed, "toast-output-select-failed")
                .name(name)
                .detail(error),
        ));
    }

    fn save(&self, id: Option<String>, settings: &mut AppSettings, cx: &mut Cx) {
        settings.set_output_device(id, cx);
    }

    /// Reads the system's devices again, with what the sound server says of its sinks, as when a
    /// settings page comes into view. A saved device that can never be found again, such as an
    /// ALSA device saved before a sound server played, is forgotten here, and the system's
    /// default plays in its place.
    pub fn refresh(&mut self, cx: &mut Cx) {
        let selected = self.selected.clone();
        self.listing = Some(cx.spawn(|link| async move {
            let listed = tokio::task::spawn_blocking(move || {
                let retired = selected.filter(|id| output::retired(id));
                (output::devices(), retired)
            })
            .await;
            let Ok((devices, retired)) = listed else {
                return;
            };
            link.post(move |store| {
                let Store {
                    outputs: this,
                    settings,
                    cx,
                    ..
                } = store;
                this.listing = None;
                this.listed = true;
                if let Some(id) = retired.filter(|id| this.selected.as_ref() == Some(id)) {
                    log::info!(
                        "playback: the saved output device {id} is not offered here anymore, \
                         playing on the system default"
                    );
                    this.selected = None;
                    this.save(None, settings, cx);
                    output::choose_device(None);
                    cx.changed(Topic::Outputs);
                }
                if this.devices != devices {
                    this.devices = devices;
                    cx.changed(Topic::Outputs);
                }
            });
        }));
    }
}

impl Store {
    /// Turns following each track's sample rate on or off, and saves it. The engines reopen,
    /// paused or not, so the track playing is heard at once at the rate it was made at.
    pub fn set_output_match_rate(&mut self, on: bool) {
        self.outputs
            .set_match_rate(on, &mut self.settings, &mut self.cx);
        self.restart_outputs();
    }
}
