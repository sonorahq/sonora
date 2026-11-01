//! Which output device Sonora plays on, and what the stream on it runs at. Every engine mixes into
//! one stream on the chosen device, or on the system's default when none is chosen or the chosen
//! one is not connected. A device picked in settings is opened first, with `open`; only once it
//! opens does `choose_device` make it the one to play on, and the engines are restarted onto it.
//!
//! The stream lives here too: one shared cpal stream on that device, every engine's `Output` on
//! it, and the watch that has them reopen when the device changes.
//!
//! A device that would not open is remembered, so it is not tried again until it is chosen again
//! or leaves the system and comes back. Opening one can take CoreAudio ten seconds to give up on.
//! A device the system no longer lists is gone, which is not a failure: the default plays in its
//! place without trying it, and it is played on again once it is back.
//!
//! On Linux cpal plays through PulseAudio when a server answers, PipeWire's `pipewire-pulse`
//! included, and ALSA otherwise. A sound server offers its own outputs, and a sound card's own
//! ALSA device can still be picked to play past it.

#[cfg(target_os = "linux")]
mod output_alsa;
#[cfg(target_os = "linux")]
mod output_host;
mod output_rate;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use cpal::traits::{DeviceTrait as _, HostTrait as _};
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

use crate::audio::{self, Chain};
#[cfg(target_os = "linux")]
use output_host::host;

/// The buffer asked of the device, in frames, as far as the device allows it, when none is
/// chosen in settings.
const BUFFER_FRAMES: u32 = 1024;

/// The buffers, in frames, that can be chosen in settings. Below the sound server's quantum the
/// device underruns, and above it every pause and seek is heard late.
const BUFFERS: [u32; 6] = [128, 256, 512, 1024, 2048, 4096];

/// How often an open stream checks that its device is still the one `resolve_device` names and
/// still runs at the stream's rate, reads its sink again, and pauses once nothing plays on it.
const DEVICE_POLL: Duration = Duration::from_millis(500);

/// How many of the stream's buffers a resting stream plays out before it pauses. PulseAudio
/// double-buffers and the sound server adds a quantum on top, so this covers both.
const DRAIN_BUFFERS: u32 = 4;

/// The device stream every output in the process mixes into.
/// It runs at the rate the device runs at and closes when the last output on it drops.
static ACTIVE_DEVICE: Mutex<Weak<Device>> = Mutex::new(Weak::new());

/// Mutex that is held while a device is being opened.
static DEVICE_OPEN: Mutex<()> = Mutex::new(());

/// Which device was chosen in settings, and which devices would not open.
static DEVICE_SELECTION: Mutex<DeviceSelection> = Mutex::new(DeviceSelection::new());

/// Triggers watchers whenever a stream opens or closes, or the sound server says something new of
/// the sink it plays on, so a settings page can read `active_device_info` again.
static DEVICE_CHANGES: LazyLock<watch::Sender<()>> = LazyLock::new(|| watch::Sender::new(()));

/// An output device the system offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceOption {
    /// cpal's id for the device, which is what `choose_device` takes and what settings store.
    pub id: String,

    /// The name the system shows for the device in full.
    pub name: String,

    /// The sound card the device belongs to, when the system names devices "card, device" as
    /// ALSA does, so a card's many devices can be shown together.
    pub card: Option<String>,

    /// The name without its card, which tells the device apart among its card's devices. It is
    /// the whole name when there is no card.
    pub label: String,

    /// What else the system says about the device beyond its name, such as which outputs an
    /// ALSA device drives. It comes from the system untranslated.
    pub detail: Option<String>,

    /// Whether it is the system's default output right now. No device in the list is when the
    /// default is not a device of its own, as on ALSA, where it is left out of the list.
    pub system: bool,

    /// Whether it is a sound card's own device, opened past the sound server for bit-perfect
    /// output. Nothing else can play on the card while Sonora holds it, and it does not open
    /// while the server does.
    pub direct: bool,

    /// What the sound server says of the sink, for one of its own outputs.
    pub sink: Option<Sink>,
}

/// An output as device rules know it: its key, from `output_key`, and its name when it was seen,
/// so a rule for an output that is not connected can still be shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputId {
    pub id: String,
    pub name: String,
}

/// One port of a sound server's sink, such as headphones or speakers on one card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// The server's name for it, which `output_key` keys it by.
    pub id: String,
    /// What the user knows it as.
    pub name: String,
    /// Whether something is plugged into it. Only false when the server knows it is not.
    pub available: bool,
}

/// What a sound server says of one of its sinks: its ports, the one that plays, and the format
/// it runs at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sink {
    pub ports: Vec<Port>,
    /// Where the port that plays is in `ports`, or `None` when the sink has no ports.
    pub active: Option<usize>,
    /// The format the server runs the sink at, when it says. The server converts a stream at
    /// another rate to it.
    pub format: Option<Format>,
}

/// A rate, a channel count and a sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
    pub sample: Sample,
}

/// How each sample is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    Integer(u8),
    Float(u8),
    /// A format without a plain bit depth, such as a-law.
    Other,
}

/// A device that would not open, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub id: String,
    pub error: String,
    /// Whether it is the chosen device, which gave way to the system's default, rather than the
    /// default itself, which leaves nothing playing.
    pub saved: bool,
}

/// A desktop sound server, which ALSA reaches only through the plugin its package installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Server {
    PipeWire,
    PulseAudio,
}

/// Which device and rate were chosen in settings, which devices would not open, and whether
/// anything plays at all. One lock covers them, so a new choice and the failures it clears change
/// together.
struct DeviceSelection {
    /// The chosen device's cpal id, or `None` to follow the system's default.
    chosen: Option<String>,
    /// The rate to open a device at when it offers it, or `None` for the rate it runs at.
    rate: Option<u32>,
    /// The buffer to ask a device for, in frames, or `None` for `BUFFER_FRAMES`.
    buffer: Option<u32>,
    /// The devices that would not open, most recent last.
    failed: Vec<Failure>,
    /// Whether the last attempt to get a stream to play on failed, so nothing is heard: every
    /// device to try would not open, or there was none.
    failing: bool,
}

impl DeviceSelection {
    const fn new() -> Self {
        Self {
            chosen: None,
            rate: None,
            buffer: None,
            failed: Vec::new(),
            failing: false,
        }
    }

    /// The process's one selection, locked.
    fn lock() -> MutexGuard<'static, Self> {
        DEVICE_SELECTION
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Chooses `id`, or the system's default when `None`, and forgets every failure, so every
    /// device is tried afresh.
    fn choose(&mut self, id: Option<String>) {
        self.chosen = id;
        self.failed.clear();
        self.failing = false;
    }

    /// Check if the device with `id` has failed to open.
    fn has_failed(&self, id: &str) -> bool {
        self.failed.iter().any(|failure| failure.id == id)
    }

    /// Records that the device with `id` would not open, replacing any earlier failure of it.
    /// Returns whether that changed anything, which the same failure again does not.
    fn record_failure(&mut self, id: &str, error: &anyhow::Error) -> bool {
        let failure = Failure {
            id: id.to_owned(),
            error: format!("{error:#}"),
            saved: self.chosen.as_deref() == Some(id),
        };
        let changed = self.failed.last() != Some(&failure);
        self.clear_failure(id);
        self.failed.push(failure);
        changed
    }

    /// Records whether the last attempt to get a stream to play on failed, returning whether
    /// that changed.
    fn set_failing(&mut self, failing: bool) -> bool {
        std::mem::replace(&mut self.failing, failing) != failing
    }

    /// Forgets any failure recorded for the device with `id`, as once it has opened.
    fn clear_failure(&mut self, id: &str) {
        self.failed.retain(|failure| failure.id != id);
    }

    /// The last failure recorded, if any.
    fn latest_failure(&self) -> Option<&Failure> {
        self.failed.last()
    }

    /// The ids of the devices that have failed to open.
    fn failed_ids(&self) -> Vec<String> {
        self.failed
            .iter()
            .map(|failure| failure.id.clone())
            .collect()
    }

    /// Forgets the failures of the devices in `gone`.
    fn forget(&mut self, gone: &[String]) {
        self.failed.retain(|failure| !gone.contains(&failure.id));
    }
}

/// A stream opened on a device picked in settings, before the engines move onto it. It stays
/// open while this is held, so the engines join it rather than opening the device again.
pub struct Opened {
    /// Only held, never read: it keeps a strong reference to the stream so it stays open.
    _device: Arc<Device>,
}

/// An output device with a stream open on it: which device, and the rate and buffer that stream
/// runs at, as a settings page shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Stable device id.
    pub id: String,

    /// Human-readable device name.
    pub name: String,

    /// What the stream runs at. Every track is resampled to its rate and remixed to its
    /// channels, and integer samples wrap past full scale where floats clip.
    pub format: Format,

    /// The buffer Sonora asks the device for, in frames.
    pub buffer: u32,

    /// The rates `choose_rate` can open the device at, lowest first, the format's among them.
    /// Only that one where the system sets the rate itself.
    pub rates: Vec<u32>,

    /// The buffers `choose_buffer` can open the device with, in frames, lowest first, `buffer`
    /// among them.
    pub buffers: Vec<u32>,

    /// What the sound server says of the sink the stream plays on, read again while it is open,
    /// for one of the server's own outputs.
    pub sink: Option<Sink>,
}

/// One open stream on an output device, with the mixer the engines' chains play into.
struct Device {
    /// Which device it is, and the rate and buffer the stream runs at. Its `sink` stays `None`,
    /// since `sink` holds the one kept up to date.
    info: DeviceInfo,

    /// Set by cpal's error callback, on the audio thread, when the stream reports any error but
    /// an underrun, such as the device going away. It is shared with that callback, hence the
    /// `Arc`, and never cleared: a failed stream is replaced, not recovered.
    failed: Arc<AtomicBool>,

    /// Set once the device is no longer the one `resolve_device` names, or runs at another
    /// rate, so no output joins the stream again.
    stale: AtomicBool,

    /// Where each output on the stream hears that it has to reopen.
    listeners: Mutex<Vec<UnboundedSender<()>>>,

    /// What the sound server last said of the sink, which `info` hands out in place of its own.
    sink: Mutex<Option<Sink>>,

    /// rodio's handle on the open cpal stream, and the mixer every engine's chain is added to.
    /// Dropping it closes the stream, which happens when the last output lets go of this device.
    stream: MixerDeviceSink,

    /// Which outputs play on the stream, and whether it still runs for none of them.
    activity: Mutex<Activity>,
}

/// How many outputs play on a stream, and since when it has run for none. A paused stream keeps
/// what it holds and plays that first when it starts again, which after a seek or a reload is the
/// old position, so a stream with nothing playing pauses only once its buffer has played out the
/// silence behind the last sound. Paused, an idle mixer pulls nothing through the chains.
#[derive(Default)]
struct Activity {
    playing: usize,
    /// When the last output stopped, while the stream still runs to play its buffer out.
    resting: Option<Instant>,
}

impl Device {
    /// Returns the stream every engine should play into, opening one if needed.
    ///
    /// The device comes from `resolve_device`: the one chosen in settings, or the system default.
    /// If a healthy stream is already open on that device, it is reused; otherwise a new stream
    /// is opened at the device's own sample rate.
    ///
    /// If the device fails to open, it is recorded as failed, so `resolve_device` skips it from
    /// then on. `resolve_device` is then asked once more, and whatever it names instead, usually
    /// the system default, is tried. If there is nothing else to try, the first error is returned.
    ///
    /// An existing stream on another device is not closed here. The engines still using it keep
    /// it until `spawn_watcher` tells them to reopen, and it closes when the last one lets go.
    fn acquire() -> Result<Arc<Self>> {
        // Opens take turns, so two engines starting together share one stream. Only engines wait
        // on this lock: `active_device_info`, which the settings page calls, reads
        // `ACTIVE_DEVICE`, and that lock is never held across an open, since CoreAudio can take
        // ten seconds to give up on a device.
        let _opening = DEVICE_OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        let acquired = resolve_device().and_then(|device| {
            Self::join_or_record(device).or_else(|error| match resolve_device() {
                Ok(device) => Self::join_or_record(device),
                Err(_) => Err(error),
            })
        });
        Self::track_failing(acquired)
    }

    /// Passes `acquired` through, recording whether it failed, so `failing` reports that nothing
    /// plays, and announcing when that changes.
    fn track_failing(acquired: Result<Arc<Self>>) -> Result<Arc<Self>> {
        if DeviceSelection::lock().set_failing(acquired.is_err()) {
            announce_device_changes();
        }
        acquired
    }

    /// The shared stream on `device`, opened even if it is not the device `resolve_device` would
    /// pick. This is how a device picked in settings is tried before the engines move onto it. A
    /// device that will not open here is not recorded as failed.
    fn open_now(device: cpal::Device) -> Result<Arc<Self>> {
        let _opening = DEVICE_OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        Self::join(device)
    }

    /// `join`, recording a device that would not open so `resolve_device` skips it.
    fn join_or_record(device: cpal::Device) -> Result<Arc<Self>> {
        let id = cpal_ident(&device);
        Self::join(device).inspect_err(|error| {
            if let Some(id) = &id
                && DeviceSelection::lock().record_failure(id, error)
            {
                announce_device_changes();
            }
        })
    }

    /// The open stream on `device`, or a new one.
    fn join(device: cpal::Device) -> Result<Arc<Self>> {
        let id = cpal_ident(&device).context("the output device has no id")?;
        let wanted = DeviceSelection::lock().rate;
        let current = ACTIVE_DEVICE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .upgrade();
        if let Some(open) = current.filter(|open| {
            open.info.id == id
                && open.live()
                && !output_rate::moved(&device, open.info.format.rate, wanted)
        }) {
            return Ok(open);
        }

        let open = Arc::new(Self::open(device, id.clone())?);
        *ACTIVE_DEVICE.lock().unwrap_or_else(PoisonError::into_inner) = Arc::downgrade(&open);
        spawn_watcher(Arc::downgrade(&open));
        DeviceSelection::lock().clear_failure(&id);
        announce_device_changes();
        Ok(open)
    }

    /// Opens the device at the rate chosen in settings when it offers it, otherwise at the rate
    /// it runs at, so nothing between here and the hardware converts what Sonora plays.
    fn open(device: cpal::Device, id: String) -> Result<Self> {
        let default = device
            .default_output_config()
            .map_err(|error| anyhow::anyhow!("cannot read the output config: {error}"))?;
        let (wanted, frames) = {
            let selection = DeviceSelection::lock();
            (selection.rate, selection.buffer.unwrap_or(BUFFER_FRAMES))
        };
        let config = output_rate::config(&device, default, wanted);
        let rates = output_rate::rates(&device, &config);
        let (name, _) = cpal_describe(&device);
        let failed = Arc::new(AtomicBool::new(false));
        let (stream, buffer) = open_stream(device, &id, &config, frames, &failed)?;
        stream.pause();
        let buffers = buffers(&config, buffer);
        let sink = read_sink(&id);
        Ok(Self {
            info: DeviceInfo {
                id,
                name,
                format: Format {
                    rate: config.sample_rate(),
                    channels: config.channels(),
                    sample: Sample::of(config.sample_format()),
                },
                buffer,
                rates,
                buffers,
                sink: None,
            },
            failed,
            stale: AtomicBool::new(false),
            listeners: Mutex::new(Vec::new()),
            sink: Mutex::new(sink),
            stream,
            activity: Mutex::default(),
        })
    }

    /// Counts one more output playing, starting the stream for the first unless it still runs.
    fn wake(&self) {
        let mut activity = self.activity();
        if activity.playing == 0 && activity.resting.take().is_none() {
            self.stream.play();
        }
        activity.playing += 1;
    }

    /// Counts one output fewer playing. Once none is, the stream runs on until `settle` pauses it.
    fn rest(&self) {
        let mut activity = self.activity();
        let Some(left) = activity.playing.checked_sub(1) else {
            return;
        };
        activity.playing = left;
        if left == 0 {
            activity.resting = Some(Instant::now());
        }
    }

    /// Pauses a stream nothing plays on once it has played its buffer out.
    fn settle(&self) {
        let mut activity = self.activity();
        let drained = activity
            .resting
            .is_some_and(|since| since.elapsed() >= self.drain());
        if activity.playing == 0 && drained {
            self.stream.pause();
            activity.resting = None;
        }
    }

    /// How long the stream takes to play out everything it may hold.
    fn drain(&self) -> Duration {
        let frames = f64::from(self.info.buffer * DRAIN_BUFFERS);
        Duration::from_secs_f64(frames / f64::from(self.info.format.rate.max(1)))
    }

    fn activity(&self) -> MutexGuard<'_, Activity> {
        self.activity.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether outputs can still join this stream: it has reported no error, and the watcher has
    /// not found it outdated.
    fn live(&self) -> bool {
        !self.failed.load(Ordering::Acquire) && !self.stale.load(Ordering::Acquire)
    }

    /// Whether the stream has to reopen. That is when it reported an error, when `resolve_device`
    /// now picks another device or none at all, or when the device has switched to another
    /// sample rate.
    fn outdated(&self) -> bool {
        if self.failed.load(Ordering::Acquire) {
            return true;
        }
        let Ok(device) = resolve_device() else {
            return true;
        };
        let wanted = DeviceSelection::lock().rate;
        cpal_ident(&device).as_deref() != Some(self.info.id.as_str())
            || output_rate::moved(&device, self.info.format.rate, wanted)
    }

    /// Has `changed` hear when the stream has to reopen.
    fn listen(&self, changed: UnboundedSender<()>) {
        let mut listeners = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        listeners.retain(|listener| !listener.is_closed());
        listeners.push(changed);
    }

    /// Marks the stream stale and tells every output on it to reopen.
    fn retire(&self) {
        self.stale.store(true, Ordering::Release);
        let listeners = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for listener in listeners.iter() {
            listener.send(()).ok();
        }
        drop(listeners);
        announce_device_changes();
    }

    /// Puts one engine's chain into the mixer.
    fn add(&self, source: impl Source + Send + 'static) {
        self.stream.mixer().add(source);
    }

    /// Which device it is and what its stream runs at, with what the sound server last said of
    /// the sink.
    fn info(&self) -> DeviceInfo {
        DeviceInfo {
            sink: self
                .sink
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            ..self.info.clone()
        }
    }

    /// Asks the sound server about the sink again, returning whether it says something new, such
    /// as another port playing or another rate.
    fn follow_sink(&self) -> bool {
        let now = read_sink(&self.info.id);
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        if *sink == now {
            return false;
        }
        *sink = now;
        true
    }
}

/// Holds the calling thread in COM's multithreaded apartment until it exits. Every cpal call goes
/// through `host` first, so a thread joins it before cpal would put it in a single-threaded one.
///
/// From a single-threaded apartment WASAPI now and then fails to open the default device just
/// after the system switches it, with `RPC_E_CHANGED_MODE`. A bare cpal program does not hit that,
/// so something else in Sonora that uses COM plays a part, but from the multithreaded apartment
/// the new default opens every time. A thread already in an apartment, such as the UI thread,
/// stays in it.
#[cfg(windows)]
struct Apartment {
    /// Whether this thread's call initialized COM, so it has to uninitialize it on exit.
    entered: bool,
}

#[cfg(windows)]
impl Apartment {
    fn enter() {
        thread_local! {
            static APARTMENT: Apartment = Apartment {
                entered: unsafe {
                    windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_MULTITHREADED,
                    )
                }
                .is_ok(),
            };
        }
        APARTMENT.with(|_| {});
    }
}

#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.entered {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

impl Sample {
    fn of(format: cpal::SampleFormat) -> Self {
        use cpal::SampleFormat::*;
        match format {
            I8 | U8 => Self::Integer(8),
            I16 | U16 => Self::Integer(16),
            I24 | U24 => Self::Integer(24),
            I32 | U32 => Self::Integer(32),
            I64 | U64 => Self::Integer(64),
            F32 => Self::Float(32),
            F64 => Self::Float(64),
            _ => Self::Other,
        }
    }
}

impl Sink {
    /// The port that plays, or `None` when the sink has no ports.
    pub fn port(&self) -> Option<&Port> {
        self.ports.get(self.active?)
    }

    /// The port that plays, only when the sink has more than one, so it is worth naming.
    pub fn shown_port(&self) -> Option<&Port> {
        self.port().filter(|_| self.ports.len() > 1)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        announce_device_changes();
    }
}

/// One engine's player on the shared device stream. Dropping it ends the player's source in the
/// mixer, and the stream closes once no output is left on it.
pub(crate) struct Output {
    chain: Chain,
    sink: Arc<rodio::Player>,
    /// What every chunk appended to `sink` publishes its format into.
    format: audio::PublishedFormat,
    device: Arc<Device>,
    /// Whether this output counts as playing on the device.
    playing: AtomicBool,
}

impl Output {
    /// Joins the shared stream for packets resampled to its rate already, and runs every sample
    /// through the equalizer and then the volume ramp before it reaches the mixer. `changed`
    /// hears when the stream has to reopen, playing or not.
    pub fn open(chain: Chain, changed: UnboundedSender<()>) -> Result<Self> {
        let device = Device::acquire()?;
        device.listen(changed);
        let (sink, source) = rodio::Player::new();
        let format = audio::PublishedFormat::default();
        let integer = matches!(device.info.format.sample, Sample::Integer(_));
        device.add(chain.apply(source, format.clone(), integer));
        Ok(Self {
            chain,
            sink: Arc::new(sink),
            format,
            device,
            playing: AtomicBool::new(false),
        })
    }

    pub fn sink(&self) -> &Arc<rodio::Player> {
        &self.sink
    }

    /// Plays the sink, starting the shared stream if it was resting.
    pub fn play(&self) {
        if !self.playing.swap(true, Ordering::Relaxed) {
            self.device.wake();
        }
        self.sink.play();
    }

    /// Pauses the sink, and the shared stream once no other output plays and it has played out.
    pub fn pause(&self) {
        self.sink.pause();
        if self.playing.swap(false, Ordering::Relaxed) {
            self.device.rest();
        }
    }

    /// Where a chunk appended to `sink` publishes its format, so the chain knows what it plays.
    pub fn format(&self) -> &audio::PublishedFormat {
        &self.format
    }

    /// The rate the device runs at, which every packet is resampled to before it is heard.
    pub fn rate(&self) -> u32 {
        self.device.info.format.rate
    }

    pub fn set_volume(&self, gain: f32) {
        self.chain.volume.set(gain);
    }

    /// Whether the stream reported an error or has to reopen, so what this output holds can no
    /// longer be heard as it should. Every output on the stream sees it.
    pub fn failed(&self) -> bool {
        !self.device.live()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if *self.playing.get_mut() {
            self.device.rest();
        }
    }
}

/// Every output device the system offers. On Linux with a sound server that is the server's own
/// outputs, followed by each sound card's direct device. This opens devices on some platforms, so
/// call it off the UI thread.
pub fn devices() -> Vec<DeviceOption> {
    let host = host();
    let system = system_default(&host);
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    let mut options: Vec<DeviceOption> = devices
        .filter_map(|device| {
            let id = cpal_ident(&device)?;
            let (name, detail) = cpal_describe(&device);
            let option = DeviceOption {
                system: system.as_ref() == Some(&id),
                id,
                card: None,
                label: name.clone(),
                name,
                detail,
                direct: false,
                sink: None,
            };
            #[cfg(target_os = "linux")]
            let option = output_host::listed(&device, option)?;
            Some(option)
        })
        .collect();
    #[cfg(target_os = "linux")]
    if output_host::is_server(&host) {
        let mut sinks = output_host::sinks();
        for option in &mut options {
            option.sink = sinks.remove(&option.id);
        }
        options.extend(output_alsa::direct_devices());
    }
    // ALSA can list one card both by name and by number, which come out as one id
    let mut seen = HashSet::new();
    options.retain(|option| seen.insert(option.id.clone()));
    options
}

/// Whether a device id saved in settings can never name a device here again, so it should be
/// forgotten rather than waited for. That is an id that does not parse, or an ALSA device other
/// than a sound card's own once a sound server plays. Asks the system, so call it off the UI
/// thread.
pub fn retired(id: &str) -> bool {
    match id.parse::<cpal::DeviceId>() {
        Err(_) => true,
        #[cfg(target_os = "linux")]
        Ok(id) => {
            id.host() == cpal::HostId::Alsa
                && !output_alsa::is_direct(id.id())
                && output_host::is_server(&host())
        }
        #[cfg(not(target_os = "linux"))]
        Ok(_) => false,
    }
}

/// Opens the device with `id`, or the system's default when `id` is `None`, as a device picked in
/// settings is tried before anything moves onto it. A device that will not open is not
/// remembered: the pick is simply not taken. This blocks for as long as the system takes, which
/// CoreAudio can stretch to ten seconds, so call it off the UI thread.
pub fn open(id: Option<&str>) -> Result<Opened> {
    let host = host();
    let device = match id {
        Some(id) => find_device(&host, id).context("the device is not connected")?,
        None => host
            .default_output_device()
            .context("no audio output device")?,
    };
    Ok(Opened {
        _device: Device::open_now(device)?,
    })
}

/// Plays on the device with `id` from now on, or on the system's default when `id` is `None`.
/// The chosen device gives way to the default while it is not connected or would not open.
/// Every device is tried afresh.
pub fn choose_device(id: Option<String>) {
    DeviceSelection::lock().choose(id);
    announce_device_changes();
}

/// Opens devices at `rate` from now on where they offer it, or at the rate they run at when
/// `None`. A stream open at another rate is retired, so every output on it reopens at the new one.
pub fn choose_rate(rate: Option<u32>) {
    DeviceSelection::lock().rate = rate;
    let open = ACTIVE_DEVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .upgrade();
    if let Some(rate) = rate
        && let Some(open) = open
        && rate != open.info.format.rate
        && open.info.rates.contains(&rate)
    {
        log::info!("output: reopening {} at {rate} Hz", open.info.id);
        open.retire();
    }
}

/// Asks devices for a buffer of `frames` from now on, as far as they allow it, or for
/// `BUFFER_FRAMES` when `None`. A stream open with another buffer is retired, so every output on
/// it reopens with the new one.
pub fn choose_buffer(frames: Option<u32>) {
    DeviceSelection::lock().buffer = frames;
    let open = ACTIVE_DEVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .upgrade();
    if let Some(frames) = frames
        && let Some(open) = open
        && frames != open.info.buffer
        && open.info.buffers.contains(&frames)
    {
        log::info!("output: reopening {} with {frames} frames", open.info.id);
        open.retire();
    }
}

/// The device every engine plays on, if a stream is open on one.
pub fn active_device_info() -> Option<DeviceInfo> {
    let device = ACTIVE_DEVICE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .upgrade()?;
    Some(device.info())
}

/// The device that last would not open, and why, if one has not opened since.
pub fn latest_device_failure() -> Option<Failure> {
    DeviceSelection::lock().latest_failure().cloned()
}

/// A watch that moves on whenever a stream opens or closes, a device would not open, or the sound
/// server says something new of the sink the stream plays on.
pub fn watch_device_changes() -> watch::Receiver<()> {
    DEVICE_CHANGES.subscribe()
}

/// Tells every `watch_device_changes` watcher that the stream or its sink changed.
fn announce_device_changes() {
    DEVICE_CHANGES.send_modify(|_| {});
}

/// Where the system lets someone change the device's sample rate, as a URI to open, or `None` on
/// a system that has no such place. `device` is the `DeviceInfo::id` of the device playing, if
/// any; Windows opens that device's own page, and its general sound page without one.
pub fn system_audio_settings_uri(device: Option<&str>) -> Option<String> {
    if cfg!(target_os = "macos") {
        Some("file:///System/Applications/Utilities/Audio%20MIDI%20Setup.app".to_owned())
    } else if cfg!(target_os = "windows") {
        // a WASAPI id is the endpoint id the device properties page takes
        let endpoint = device
            .and_then(|id| id.parse::<cpal::DeviceId>().ok())
            .map(|id| id.id().to_owned());
        Some(match endpoint {
            Some(endpoint) => format!("ms-settings:sound-properties?endpointId={endpoint}"),
            None => "ms-settings:sound".to_owned(),
        })
    } else {
        None
    }
}

/// Picks the device Sonora should play on right now.
///
/// 1. The device chosen in settings, if one is chosen, it is connected, and it has not failed to
///    open.
/// 2. Otherwise the system's default output, if it has not failed to open.
/// 3. Otherwise an error: there is no device, the default has failed too, or it has no id to
///    tell it apart by.
///
/// A device that failed to open is skipped until it is chosen again (`choose_device` clears the
/// failures) or it is unplugged and plugged back in, which `forget_gone` notices at the start of
/// each call.
fn resolve_device() -> Result<cpal::Device> {
    let host = host();
    forget_gone(&host);

    // First try the device chosen in settings, if it has not failed to open and is connected.
    // read in its own block, so the lock is let go before the system is asked for the device
    let chosen = {
        let selection = DeviceSelection::lock();
        selection
            .chosen
            .clone()
            .filter(|id| !selection.has_failed(id))
    };
    if let Some(id) = chosen
        && let Some(device) = find_device(&host, &id)
    {
        return Ok(device);
    }

    // Otherwise try the system's default output, if it has not failed to open.
    let device = host
        .default_output_device()
        .context("no audio output device")?;
    let Some(id) = cpal_ident(&device) else {
        bail!("the default output device has no id");
    };
    if DeviceSelection::lock().has_failed(&id) {
        bail!("the default output device would not open");
    }

    Ok(device)
}

/// Whether there is an output device to play on. A device can be missing for as long as it
/// takes a headset to reconnect, so the engines ask again rather than give up.
pub fn output_available() -> bool {
    resolve_device().is_ok()
}

/// Whether a stream on the device to play on opens right now. The stream is the shared one, so
/// it closes again at once unless an output already plays on it.
///
/// A system default that would not open is tried again, in case whatever held it has let go,
/// which `resolve_device` alone never does. A device chosen in settings that would not open is
/// not tried again.
pub(crate) fn output_openable() -> bool {
    if output_available() {
        return Device::acquire().is_ok();
    }
    let Some(device) = host().default_output_device() else {
        return false;
    };
    let chosen_and_failed = cpal_ident(&device).is_some_and(|id| {
        let selection = DeviceSelection::lock();
        selection.chosen.as_deref() == Some(id.as_str()) && selection.has_failed(&id)
    });
    if chosen_and_failed {
        return false;
    }
    let _opening = DEVICE_OPEN.lock().unwrap_or_else(PoisonError::into_inner);
    Device::track_failing(Device::join_or_record(device)).is_ok()
}

/// Whether the last attempt to get a stream to play on failed, so whatever plays is not heard:
/// every device to try would not open, or there was none. On Linux ALSA always names a default
/// device, so this rather than `output_available` is what tells that the sound cannot get out.
/// `watch_device_changes` fires when it changes.
pub fn failing() -> bool {
    DeviceSelection::lock().failing
}

#[cfg(target_os = "linux")]
pub use output_alsa::missing_bridge;

#[cfg(not(target_os = "linux"))]
pub fn missing_bridge() -> Option<Server> {
    None
}

/// The key device rules know an output by: the device's id, and for a sound server's sink the
/// port that plays after a `#`, so headphones and speakers on one card are two outputs.
pub fn output_key(device: &str, port: Option<&Port>) -> String {
    match port {
        Some(port) => format!("{device}#{}", port.id),
        None => device.to_owned(),
    }
}

/// The key an output saved by an older Sonora goes by now, or `None` when `key` is current.
/// Device rules once keyed a sound server's sink by its bare name, which cpal now puts behind its
/// `pulseaudio` host.
#[cfg(target_os = "linux")]
pub fn upgrade_output_key(key: &str) -> Option<String> {
    let device = key.split_once('#').map_or(key, |(device, _)| device);
    if device.parse::<cpal::DeviceId>().is_ok() {
        return None;
    }
    Some(format!("{}:{key}", cpal::HostId::PulseAudio))
}

/// The key an output saved by an older Sonora goes by now. Every key is current here, since only
/// Linux ever keyed a sound server's sink.
#[cfg(not(target_os = "linux"))]
pub fn upgrade_output_key(_key: &str) -> Option<String> {
    None
}

/// What the sound server says of the sink behind the device `id`, or `None` for any other device.
/// It asks the server, so call it off the UI thread.
#[cfg(target_os = "linux")]
fn read_sink(id: &str) -> Option<Sink> {
    output_host::sink(id)
}

/// What the sound server says of the sink behind the device `id`. There is no sound server here.
#[cfg(not(target_os = "linux"))]
fn read_sink(_id: &str) -> Option<Sink> {
    None
}

/// Forgets the failures of devices that have left the system, so one that comes back is tried
/// again.
fn forget_gone(host: &cpal::Host) {
    // the system is asked without the lock held, since `latest_device_failure` reads it on the
    // UI thread
    let ids = DeviceSelection::lock().failed_ids();
    let gone: Vec<String> = ids
        .into_iter()
        .filter(|id| find_device(host, id).is_none())
        .collect();
    if !gone.is_empty() {
        DeviceSelection::lock().forget(&gone);
    }
}

/// The host Sonora plays through. On Windows the calling thread joins COM's multithreaded
/// apartment first, see `Apartment`.
#[cfg(not(target_os = "linux"))]
fn host() -> cpal::Host {
    #[cfg(windows)]
    Apartment::enter();
    cpal::default_host()
}

/// The id of the device the system plays its default output on, as `devices` lists it, or `None`
/// when the default is not one of the listed devices.
#[cfg(target_os = "linux")]
fn system_default(host: &cpal::Host) -> Option<String> {
    output_host::system_default(host)
}

/// The id of the device the system plays its default output on, as `devices` lists it.
#[cfg(not(target_os = "linux"))]
fn system_default(host: &cpal::Host) -> Option<String> {
    host.default_output_device()
        .and_then(|device| cpal_ident(&device))
}

/// The device saved as `id`, looked up on `host`, or on Linux on the ALSA host for a sound card's
/// own device. `None` when it is not connected or `id` names nothing here.
fn find_device(host: &cpal::Host, id: &str) -> Option<cpal::Device> {
    let id: cpal::DeviceId = id.parse().ok()?;
    #[cfg(target_os = "linux")]
    if id.host() == cpal::HostId::Alsa {
        return output_alsa::device(host, id.id());
    }
    if id.host() != host.id() {
        return None;
    }
    host.device_by_id(&id)
}

/// The id Sonora saves for `device`, or `None` when cpal cannot read one. It is cpal's id, with an
/// ALSA sound card named rather than numbered, since the numbers change as cards come and go.
fn cpal_ident(device: &cpal::Device) -> Option<String> {
    let id = device.id().ok()?;
    #[cfg(target_os = "linux")]
    if id.host() == cpal::HostId::Alsa {
        let pcm = output_alsa::named(id.id());
        return Some(cpal::DeviceId::new(cpal::HostId::Alsa, &pcm).to_string());
    }
    Some(id.to_string())
}

/// The name the system shows for `device`, or its id when it has none, and the lines of its
/// description that do not repeat the name, joined.
fn cpal_describe(device: &cpal::Device) -> (String, Option<String>) {
    let Ok(description) = device.description() else {
        let name = device
            .id()
            .map(|id| id.to_string())
            .unwrap_or_else(|_| "unknown".to_owned());
        return (name, None);
    };
    let name = description.name();
    let lines: Vec<&str> = description
        .extended()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != name)
        .collect();
    let detail = match lines.is_empty() {
        true => None,
        false => Some(lines.join(" · ")),
    };
    (name.to_owned(), detail)
}

/// The buffers in `BUFFERS` that `config` allows, with `buffer`, the one the stream asked for,
/// lowest first.
fn buffers(config: &cpal::SupportedStreamConfig, buffer: u32) -> Vec<u32> {
    let mut buffers: Vec<u32> = BUFFERS
        .into_iter()
        .filter(|frames| buffer_range(config).is_none_or(|(min, max)| (min..=max).contains(frames)))
        .collect();
    if !buffers.contains(&buffer) {
        buffers.push(buffer);
        buffers.sort_unstable();
    }
    buffers
}

/// The buffers `config` allows, in frames, as `(min, max)`, or `None` when the device does not
/// say. An empty range says nothing either.
fn buffer_range(config: &cpal::SupportedStreamConfig) -> Option<(u32, u32)> {
    match *config.buffer_size() {
        cpal::SupportedBufferSize::Range { max: 0, .. } => None,
        cpal::SupportedBufferSize::Range { min, max } => Some((min, max.max(min))),
        cpal::SupportedBufferSize::Unknown => None,
    }
}

/// Opens the stream on `device` in `config` with a buffer of `frames`, as far as the device
/// allows it, flagging `failed` when the device reports an error. Returns the stream and the
/// buffer it asked for, in frames.
fn open_stream(
    device: cpal::Device,
    id: &str,
    config: &cpal::SupportedStreamConfig,
    frames: u32,
    failed: &Arc<AtomicBool>,
) -> Result<(MixerDeviceSink, u32)> {
    let frames = match buffer_range(config) {
        Some((min, max)) => frames.clamp(min, max),
        None => frames,
    };
    let latency = f64::from(frames) * 1_000. / f64::from(config.sample_rate().max(1));
    log::info!(
        "output: using {} at {} Hz, {} channels, {}, {frames} frames ({latency:.1} ms)",
        id,
        config.sample_rate(),
        config.channels(),
        config.sample_format()
    );

    let stream_failed = failed.clone();
    let builder = DeviceSinkBuilder::default()
        .with_device(device)
        .with_config(&config.config())
        .with_buffer_size(cpal::BufferSize::Fixed(frames))
        .with_sample_format(config.sample_format())
        .with_error_callback(move |error| match error.kind() {
            cpal::ErrorKind::Xrun => log::debug!("output: buffer underrun"),
            // the server moves a stream that follows the default on its own
            cpal::ErrorKind::DeviceChanged => log::info!("output: the system default moved"),
            cpal::ErrorKind::RealtimeDenied => log::debug!("output: {error}"),
            _ => {
                log::warn!("output: audio output failed: {error}");
                stream_failed.store(true, Ordering::Release);
            }
        });
    let mut stream = builder
        .open_stream()
        .map_err(|error| anyhow::anyhow!("cannot open the audio output: {error}"))?;
    stream.log_on_drop(false);
    Ok((stream, frames))
}

/// Looks at `device` every `DEVICE_POLL` for as long as it is open, playing or not, and retires
/// it once it is outdated, so its outputs reopen and a settings page reads the new stream. A sink
/// whose port or rate moved is announced the same way, and a stream nothing plays on is paused
/// once it has played out.
fn spawn_watcher(device: Weak<Device>) {
    let spawned = thread::Builder::new()
        .name("output-watch".to_owned())
        .spawn(move || {
            loop {
                thread::sleep(DEVICE_POLL);
                let Some(device) = device.upgrade() else {
                    return;
                };
                if device.outdated() {
                    log::info!("output: {} changed, reopening", device.info.id);
                    device.retire();
                    return;
                }
                if device.follow_sink() {
                    announce_device_changes();
                }
                device.settle();
            }
        });
    if let Err(error) = spawned {
        log::warn!("output: cannot watch the output device: {error}");
    }
}
