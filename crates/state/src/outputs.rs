use std::time::Duration;

use anyhow::Result;
use gpui::{Context, Entity, Task};
use music::output::{self, DeviceInfo, DeviceOption, Failure, Opened};

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
/// both are read again whenever a stream opens or closes. A device picked here is opened first
/// and only moved onto, and saved, once it has; one that will not open leaves everything as it
/// was and says so in a toast.
pub struct Outputs {
    settings: Entity<AppSettings>,
    playback: Entity<Playback>,
    devices: Vec<DeviceOption>,
    /// Whether `devices` has been read at least once, so an empty list means none.
    listed: bool,
    active: Option<DeviceInfo>,
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
}

impl Outputs {
    /// Hands the device saved in settings to the engines, so call it before the first one
    /// starts.
    pub fn new(
        settings: Entity<AppSettings>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = settings.read(cx).output_device().map(str::to_owned);
        output::choose_device(selected.clone());

        let mut changes = output::watch_device_changes();
        let watch = cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
                let updated = this.update(cx, |this, cx| {
                    this.active = output::active_device_info();
                    this.failure = output::latest_device_failure();
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

        let mut outputs = Self {
            settings,
            playback,
            devices: Vec::new(),
            listed: false,
            active: output::active_device_info(),
            failure: output::latest_device_failure(),
            failing: output::failing(),
            selected,
            trying: None,
            opening: None,
            holding: None,
            handover: None,
            listing: None,
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

    /// Reads the system's devices again, as when a settings page comes into view.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let listed = cx
            .background_executor()
            .spawn(async move { output::devices() });
        self.listing = Some(cx.spawn(async move |this, cx| {
            let devices = listed.await;
            this.update(cx, |this, cx| {
                this.listing = None;
                this.listed = true;
                if this.devices != devices {
                    this.devices = devices;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}
