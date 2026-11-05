use std::time::Duration;

use anyhow::Result;
use gpui::{App, Context, Entity, Subscription, Task};
use music::output::{self, DeviceInfo, DeviceOption, Failure, Opened, OutputId, Sink};

use crate::{AppSettings, Io, Outcome, Playback, Toasts, join};

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
/// plays on, which the equalizer follows to play the sound its device rule asks for.
pub struct Outputs {
    settings: Entity<AppSettings>,
    playback: Entity<Playback>,
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
    opening: Option<Task<()>>,
    /// Runs out `SWITCH_TIMEOUT` after a pick. While it is set the picker is held.
    holding: Option<Task<()>>,
    /// The picked device's stream, kept open until the engines have joined it.
    handover: Option<(Opened, Task<()>)>,
    listing: Option<Task<()>>,
    _watch: Task<()>,
    /// Why the chosen device could not be taken for direct output, the last time it tried. It is
    /// toasted when it first appears.
    hold_failure: Option<String>,
    /// Why the device could not be switched to a rate, the last time it tried. It is toasted when
    /// it first appears.
    rate_failure: Option<String>,
    /// Lets the device go when the app quits, so it is not left at a track's rate.
    _quit: Subscription,
}

impl Outputs {
    /// Hands the device, rate and buffer saved in settings to the engines, so call it before the
    /// first one starts.
    pub fn new(
        settings: Entity<AppSettings>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = settings.read(cx).output_device().map(str::to_owned);
        output::choose_device(selected.clone());
        output::choose_rate(settings.read(cx).output_rate());
        output::choose_buffer(settings.read(cx).output_buffer());
        output::choose_match_rate(settings.read(cx).output_match_rate());

        let mut changes = output::watch_device_changes();
        let watch = cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
                let updated = this.update(cx, |this, cx| {
                    this.read_device();
                    this.failure = output::latest_device_failure();
                    this.read_direct(cx);
                    this.failing = output::failing();
                    this.forget_broken(cx);
                    this.refresh(cx);
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
        });
        let quit = cx.on_app_quit(|_, _| {
            output::release_hold();
            std::future::ready(())
        });

        let mut outputs = Self {
            settings,
            playback,
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
            _quit: quit,
            _watch: watch,
        };
        outputs.refresh(cx);
        outputs
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
    fn read_direct(&mut self, cx: &mut Context<Self>) {
        let taken = output::hold_failure();
        if taken != self.hold_failure {
            if let Some(error) = taken.clone() {
                log::warn!("playback: cannot play directly on the device: {error}");
                Toasts::with_detail(
                    Outcome::Failed,
                    "toast-output-direct-failed",
                    None,
                    error,
                    cx,
                );
            }
            self.hold_failure = taken;
        }
        let rate = output::rate_failure();
        if rate != self.rate_failure {
            if let Some(error) = rate.clone() {
                log::warn!("playback: cannot switch the sample rate: {error}");
                Toasts::with_detail(Outcome::Failed, "toast-output-rate-failed", None, error, cx);
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
    pub fn match_rate(&self, cx: &App) -> bool {
        self.match_rate_supported() && self.settings.read(cx).output_match_rate()
    }

    /// Whether the device chosen is played on directly, which is the only kind whose rate can be
    /// switched.
    pub fn direct_chosen(&self) -> bool {
        self.chosen().is_some_and(output::is_direct)
    }

    /// Whether the rate follows each track right now: it is turned on, and the stream open is on
    /// a direct device.
    pub fn following(&self, cx: &App) -> bool {
        self.match_rate(cx) && self.current().is_some_and(|device| device.direct)
    }

    /// Turns following each track's sample rate on or off, and saves it. The engines reopen,
    /// paused or not, so the track playing is heard at once at the rate it was made at.
    pub fn set_match_rate(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |settings, cx| settings.set_output_match_rate(on, cx));
        output::choose_match_rate(on);
        self.playback
            .update(cx, |playback, cx| playback.restart_outputs(cx));
        cx.notify();
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
    pub fn rate(&self, cx: &App) -> Option<u32> {
        let active = self.current()?;
        let wanted = self
            .settings
            .read(cx)
            .output_rate()
            .filter(|rate| active.rates.contains(rate));
        Some(wanted.unwrap_or(active.format.rate))
    }

    /// Opens the device at `rate` from now on, and saves it. The engines reopen at it, paused or
    /// not.
    pub fn set_rate(&mut self, rate: u32, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |settings, cx| settings.set_output_rate(Some(rate), cx));
        output::choose_rate(Some(rate));
        cx.notify();
    }

    /// The buffer the device plays with, in frames, or is reopening with once a new one is
    /// chosen: the one saved in settings when the device allows it, otherwise the one it has.
    pub fn buffer(&self, cx: &App) -> Option<u32> {
        let active = self.current()?;
        let wanted = self
            .settings
            .read(cx)
            .output_buffer()
            .filter(|frames| active.buffers.contains(frames));
        Some(wanted.unwrap_or(active.buffer))
    }

    /// Asks the device for a buffer of `frames` from now on, and saves it. The engines reopen
    /// with it, paused or not.
    pub fn set_buffer(&mut self, frames: u32, cx: &mut Context<Self>) {
        self.settings.update(cx, |settings, cx| {
            settings.set_output_buffer(Some(frames), cx)
        });
        output::choose_buffer(Some(frames));
        cx.notify();
    }

    /// Whether a device was just picked and has not answered yet, so no other can be picked.
    pub fn switching(&self) -> bool {
        self.holding.is_some()
    }

    /// Opens `id`, or the system's default when `None`, and once it opens moves playback onto
    /// it, paused or not, and saves it. A device that will not open leaves playback and the saved
    /// device as they were, with a toast saying so.
    pub fn choose(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.switching() {
            return;
        }
        // the same device again, with nothing wrong, would only cut the sound for a moment
        if id == self.selected && self.failure.is_none() {
            return;
        }
        self.trying = Some(id.clone());
        let io = Io::global(cx);
        let wanted = id.clone();
        self.opening = Some(cx.spawn(async move |this, cx| {
            let opened = join(io.spawn_blocking(move || output::open(wanted.as_deref()))).await;
            this.update(cx, |this, cx| this.picked(id, opened, cx)).ok();
        }));
        self.holding = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SWITCH_TIMEOUT).await;
            this.update(cx, |this, cx| {
                this.holding = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Takes the answer to a pick: moves playback onto a device that opened and saves it, or
    /// toasts that it would not.
    fn picked(&mut self, id: Option<String>, opened: Result<Opened>, cx: &mut Context<Self>) {
        self.opening = None;
        self.holding = None;
        self.trying = None;
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) => {
                log::warn!("playback: cannot select the output device: {error:#}");
                self.toast_rejected(id.as_deref(), &error, cx);
                cx.notify();
                return;
            }
        };
        self.selected = id.clone();
        output::choose_device(id.clone());
        self.save(id, cx);
        self.playback
            .update(cx, |playback, cx| playback.restart_outputs(cx));
        let release = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HANDOVER).await;
            this.update(cx, |this, _| this.handover = None).ok();
        });
        self.handover = Some((opened, release));
        cx.notify();
    }

    /// Forgets the saved device once it is there but would not open, so the next run does not
    /// wait on it again. The system's default already plays in its place. A device that is gone
    /// is never tried, so it never fails and stays saved.
    fn forget_broken(&mut self, cx: &mut Context<Self>) {
        let Some(failure) = self.failure.as_ref().filter(|failure| failure.saved) else {
            return;
        };
        if self.settings.read(cx).output_device() != Some(failure.id.as_str()) {
            return;
        }
        log::warn!(
            "playback: forgetting the saved output device, which would not open: {}",
            failure.error
        );
        self.selected = None;
        self.save(None, cx);
        output::choose_device(None);
    }

    /// Says in a toast that the pick `id`, or the system's default when `None`, would not open,
    /// with the error below.
    fn toast_rejected(&self, id: Option<&str>, error: &anyhow::Error, cx: &mut Context<Self>) {
        let error = format!("{error:#}");
        let Some(id) = id else {
            Toasts::with_detail(
                Outcome::Failed,
                "toast-output-default-failed",
                None,
                error,
                cx,
            );
            return;
        };
        let name = self
            .devices
            .iter()
            .find(|device| device.id == id)
            .map_or(id, |device| device.name.as_str())
            .to_owned();
        let name = Some(name.into());
        Toasts::with_detail(
            Outcome::Failed,
            "toast-output-select-failed",
            name,
            error,
            cx,
        );
    }

    fn save(&self, id: Option<String>, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |settings, cx| settings.set_output_device(id, cx));
    }

    /// Reads the system's devices again, with what the sound server says of its sinks, as when a
    /// settings page comes into view. A saved device that can never be found again, such as an
    /// ALSA device saved before a sound server played, is forgotten here, and the system's
    /// default plays in its place.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected.clone();
        let listed = cx.background_executor().spawn(async move {
            let retired = selected.filter(|id| output::retired(id));
            (output::devices(), retired)
        });
        self.listing = Some(cx.spawn(async move |this, cx| {
            let (devices, retired) = listed.await;
            this.update(cx, |this, cx| {
                this.listing = None;
                this.listed = true;
                if let Some(id) = retired.filter(|id| this.selected.as_ref() == Some(id)) {
                    log::info!(
                        "playback: the saved output device {id} is not offered here anymore, \
                         playing on the system default"
                    );
                    this.selected = None;
                    this.save(None, cx);
                    output::choose_device(None);
                    cx.notify();
                }
                if this.devices != devices {
                    this.devices = devices;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}
