use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use rodio::source::SeekError;
use rodio::{ChannelCount, DeviceSinkBuilder, MixerDeviceSink, SampleRate, Source};

use crate::equalizer::{Engine, Equalizer};
use crate::output;
use crate::route::Route;
use crate::spectrum::{Probe, Spectrum, Tap};

pub const RAMP: Duration = Duration::from_millis(25);
const BUFFER: Duration = Duration::from_millis(50);
/// The loudest sample an integer device takes: the largest `f32` below one, since one itself
/// overflows a 24-bit sample and wraps to the far end of the scale.
const FULL_SCALE: f32 = 1. - f32::EPSILON / 2.;

/// The device stream every output in the process mixes into, so the sound server sees Sonora as
/// one client however many engines are running. It runs at the rate of whatever played last and
/// closes when the last output on it drops.
static SHARED: Mutex<Weak<Device>> = Mutex::new(Weak::new());
/// Whether the last attempt to open a device stream failed, which is what `failing` reports.
static FAILING: AtomicBool = AtomicBool::new(false);

/// A desktop sound server, which ALSA reaches only through the plugin its package installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Server {
    PipeWire,
    PulseAudio,
}

#[derive(Clone)]
pub struct Volume(Arc<AtomicU32>);

impl Volume {
    pub fn new(gain: f32) -> Self {
        Self(Arc::new(AtomicU32::new(gain.to_bits())))
    }

    pub fn set(&self, gain: f32) {
        self.0.store(gain.to_bits(), Ordering::Relaxed);
    }

    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The loudest sample any output sent its device since the meter was last read, after the
/// volume and before the clamp an integer device gets, so an over shows however the device
/// takes it. Every output shares one.
#[derive(Clone, Debug, Default)]
pub struct Meter(Arc<AtomicU32>);

impl Meter {
    /// The loudest absolute sample since the last call, as a linear amplitude, and a fresh
    /// start for the next one.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }

    /// Keeps `sample` if it is the loudest yet. A finite non-negative `f32` orders like its
    /// bits, so the maximum is kept on the bits; NaN would sort above infinity and is left out.
    fn record(&self, sample: f32) {
        let bits = sample.abs().to_bits();
        // A plain load first, since a new loudest sample is rare once the meter has warmed up.
        if sample.is_finite() && bits > self.0.load(Ordering::Relaxed) {
            self.0.fetch_max(bits, Ordering::Relaxed);
        }
    }
}

/// The rate and channel count of what one player is playing, published by each chunk as it
/// yields its first sample. The queue's own answer depends on how the previous chunk ended, so
/// the chain follows this instead. Zero means no chunk has played yet.
#[derive(Clone, Default)]
pub(crate) struct Format(Arc<AtomicU64>);

impl Format {
    pub(crate) fn publish(&self, rate: SampleRate, channels: ChannelCount) {
        let word = u64::from(rate.get()) << 16 | u64::from(channels.get());
        self.0.store(word, Ordering::Relaxed);
    }
}

/// Which channel of its frame each sample a chain stage pulls belongs to, counted from the
/// format the playing chunk published. A change of format starts a new frame.
pub(crate) struct Frames {
    format: Format,
    /// The word last read, so a change is noticed on the sample that brings it.
    seen: u64,
    channel: u16,
}

/// Where the sample just pulled sits in its frame.
pub(crate) struct Place {
    pub channel: u16,
    /// Whether this sample brought a new format, which the stage takes up before using it.
    pub changed: bool,
}

impl Frames {
    pub(crate) fn new(format: Format) -> Self {
        Self {
            format,
            seen: 0,
            channel: 0,
        }
    }

    /// Pulls the next sample from `input` and places it in its frame. The place is `None` until a
    /// chunk has published a format, and the stage passes those samples through untouched.
    pub(crate) fn pull(
        &mut self,
        input: &mut impl Iterator<Item = f32>,
    ) -> Option<(f32, Option<Place>)> {
        let sample = input.next()?;
        // The chunk publishes during the pull, so the word is read after it.
        let word = self.format.0.load(Ordering::Relaxed);
        let changed = word != self.seen;
        if changed {
            self.seen = word;
            self.channel = 0;
        }
        let channels = self.channels();
        if channels == 0 {
            return Some((sample, None));
        }

        let channel = self.channel;
        self.channel += 1;
        if self.channel >= channels {
            self.channel = 0;
        }
        Some((sample, Some(Place { channel, changed })))
    }

    pub(crate) fn rate(&self) -> u32 {
        (self.seen >> 16) as u32
    }

    pub(crate) fn channels(&self) -> u16 {
        self.seen as u16
    }
}

/// What sits between the queue and the device: the equalizer, then the volume ramp, with the
/// spectrum tap listening at the end. Every engine builds one and hands it to the output.
pub struct Chain {
    pub volume: Volume,
    pub equalizer: Equalizer,
    pub spectrum: Spectrum,
    pub meter: Meter,
    pub route: Route,
}

/// One open stream on an output device, with the mixer the engines' chains play into.
struct Device {
    id: String,
    /// The rate the stream was opened for, whether or not the device could honour it.
    wanted: Option<u32>,
    rate: u32,
    /// Whether the stream takes integer samples, which wrap past full scale instead of clipping.
    integer: bool,
    failed: Arc<AtomicBool>,
    /// Taken when an output reopens the device at another rate, which closes the stream under
    /// every output still on it.
    stream: Mutex<Option<MixerDeviceSink>>,
}

impl Device {
    /// The shared stream on the default device, at `rate` when one is given. It opens a new
    /// stream when no output holds a healthy one there at that rate, closing the old stream on
    /// the same device first. A stream left on a device that is no longer the default stays with
    /// the outputs still using it until they reopen.
    fn shared(rate: Option<u32>) -> Result<Arc<Self>> {
        let shared = Self::reach(rate);
        FAILING.store(shared.is_err(), Ordering::Release);
        shared
    }

    fn reach(rate: Option<u32>) -> Result<Arc<Self>> {
        let device = cpal::default_host()
            .default_output_device()
            .context("no audio output device")?;
        let id = ident(&device);

        let mut shared = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        let current = shared.upgrade().filter(|open| open.id == id);
        if let Some(open) = &current
            && open.live()
            && rate.is_none_or(|rate| open.fits(rate))
        {
            return Ok(open.clone());
        }
        if let Some(open) = current {
            open.close();
        }

        let open = Arc::new(Self::open(device, id, rate)?);
        *shared = Arc::downgrade(&open);
        Ok(open)
    }

    /// Opens the device at `rate` when it offers that rate and at its default otherwise.
    fn open(device: cpal::Device, id: String, rate: Option<u32>) -> Result<Self> {
        let default = device
            .default_output_config()
            .map_err(|error| anyhow::anyhow!("cannot read the output config: {error}"))?;
        let config = match rate {
            Some(rate) => at_rate(&device, &default, rate).unwrap_or_else(|| {
                log::info!("sink: the device does not offer {rate} Hz, resampling instead");
                default
            }),
            None => default,
        };

        log::info!(
            "sink: using {} at {} Hz, {} channels, {}",
            id,
            config.sample_rate(),
            config.channels(),
            config.sample_format()
        );

        let format = config.sample_format();
        output::opened(output::Stream {
            device: output::Device {
                id: id.clone(),
                name: device
                    .description()
                    .map_or_else(|_| id.clone(), |description| description.name().to_owned()),
                port: None,
                format: None,
            },
            format: output::Format {
                rate: config.sample_rate(),
                channels: config.channels(),
                sample: output::Sample::of(format),
            },
        });
        let frames = (BUFFER.as_secs_f64() * config.sample_rate() as f64).round() as u32;
        let failed = Arc::new(AtomicBool::new(false));
        let stream_failed = failed.clone();
        let builder = DeviceSinkBuilder::default()
            .with_device(device)
            .with_config(&config.config())
            .with_buffer_size(cpal::BufferSize::Fixed(frames))
            .with_sample_format(format)
            .with_error_callback(move |error| match error {
                cpal::StreamError::BufferUnderrun => log::debug!("sink: buffer underrun"),
                error => {
                    log::warn!("sink: audio output failed: {error}");
                    stream_failed.store(true, Ordering::Release);
                }
            });
        let mut stream = builder
            .open_stream()
            .map_err(|error| anyhow::anyhow!("cannot open the audio output: {error}"))?;
        stream.log_on_drop(false);

        Ok(Self {
            id,
            wanted: rate,
            rate: config.sample_rate(),
            integer: !format.is_float(),
            failed,
            stream: Mutex::new(Some(stream)),
        })
    }

    /// Whether a track at `rate` can play on this stream without a reopen. A rate the device
    /// turned down counts as fitting, so the output does not retry it on every track.
    fn fits(&self, rate: u32) -> bool {
        rate == self.rate || self.wanted == Some(rate)
    }

    /// Whether the stream is still open and has reported no error.
    fn live(&self) -> bool {
        !self.failed.load(Ordering::Acquire) && self.stream().is_some()
    }

    fn close(&self) {
        self.stream().take();
    }

    /// Puts one engine's chain into the mixer. A stream that has closed drops it.
    fn add(&self, source: impl Source + Send + 'static) {
        if let Some(stream) = self.stream().as_ref() {
            stream.mixer().add(source);
        }
    }

    fn stream(&self) -> MutexGuard<'_, Option<MixerDeviceSink>> {
        self.stream.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One engine's player on the shared device stream. Dropping it ends the player's source in the
/// mixer, and the stream closes once no output is left on it.
pub struct Output {
    chain: Chain,
    sink: Arc<rodio::Player>,
    /// What every chunk appended to `sink` publishes its format into.
    format: Format,
    device: Arc<Device>,
    /// Set when `fit` could not reopen the device, so the engine reports the output gone.
    broken: bool,
}

impl Output {
    /// Joins the stream on the default output device at whatever rate it runs and runs every
    /// sample through the equalizer and then the volume ramp before it reaches the mixer.
    pub fn open(chain: Chain) -> Result<Self> {
        let device = Device::shared(None)?;
        let (sink, format) = attach(&chain, &device);
        Ok(Self {
            chain,
            sink: Arc::new(sink),
            format,
            device,
            broken: false,
        })
    }

    pub fn sink(&self) -> &Arc<rodio::Player> {
        &self.sink
    }

    /// The format word of the current `sink`. A chunk appended to `sink` must publish into this
    /// word, from the same `Output`, or the chain never learns its format.
    pub(crate) fn format(&self) -> &Format {
        &self.format
    }

    /// Whether a track at `rate` can go out as the output stands, without `fit` reopening it.
    pub fn fits(&self, rate: u32) -> bool {
        self.device.live() && self.device.fits(rate)
    }

    /// Moves this output onto a shared stream at `rate` unless it `fits` already, reopening the
    /// stream under the other engines when it runs at another rate. On `true` the queue was
    /// dropped and `sink` and `format` replaced, so take both again.
    pub fn fit(&mut self, rate: u32) -> Result<bool> {
        if self.fits(rate) {
            return Ok(false);
        }
        let paused = self.sink.is_paused();
        let device = match Device::shared(Some(rate)) {
            Ok(device) => device,
            Err(error) => {
                self.broken = true;
                return Err(error);
            }
        };
        let (sink, format) = attach(&self.chain, &device);
        if paused {
            sink.pause();
        }
        self.sink = Arc::new(sink);
        self.format = format;
        self.device = device;
        self.broken = false;
        Ok(true)
    }

    pub fn set_volume(&self, gain: f32) {
        self.chain.volume.set(gain);
    }

    /// Whether what this output holds can no longer be heard: the stream reported an error,
    /// another engine reopened it at another rate, or `fit` failed. Every output on the stream
    /// sees an error. A fresh track recovers through `fit`, a paused one has to be reloaded.
    pub fn failed(&self) -> bool {
        self.broken || !self.device.live()
    }

    /// Whether the system's default device is no longer the one this output plays on.
    pub fn changed(&self) -> bool {
        cpal::default_host()
            .default_output_device()
            .map(|device| ident(&device))
            .is_some_and(|device| device != "unknown" && device != self.device.id)
    }
}

/// A player's queue run through its `Chain`: the probe hears it, the equalizer filters it and
/// the volume ramps it, then the tap and the meter hear it, and an integer device gets it held
/// within full scale. One frame count places every sample for all of them.
pub(crate) struct Chained<I> {
    input: I,
    frames: Frames,
    probe: Probe,
    equalizer: Engine,
    /// Where the rate the equalizer runs at is published.
    rate: Arc<AtomicU32>,
    volume: Volume,
    tap: Tap,
    /// Whether the tap hears the samples from before the gain. It is read once per frame so
    /// every channel of a frame agrees.
    unscaled: bool,
    meter: Meter,
    /// Told the format of what plays, for the signal path.
    route: Route,
    /// Whether the output is held within full scale, for a device whose integer samples would
    /// wrap. The tap and the meter hear the sample before the clamp, so they still see the over.
    clamped: bool,

    current: f32,
    target: f32,
    step: f32,
    frames_left: u32,
    ramp_frames: u32,
}

impl<I: Source> Chained<I> {
    pub(crate) fn new(input: I, chain: &Chain, format: Format, clamped: bool) -> Self {
        let gain = chain.volume.get();
        Self {
            input,
            frames: Frames::new(format),
            probe: chain.equalizer.heard().probe(),
            equalizer: Engine::new(chain.equalizer.clone()),
            rate: chain.equalizer.rate_slot(),
            volume: chain.volume.clone(),
            tap: chain.spectrum.attach(),
            unscaled: false,
            meter: chain.meter.clone(),
            route: chain.route.clone(),
            clamped,
            current: gain,
            target: gain,
            step: 0.0,
            frames_left: 0,
            ramp_frames: 1,
        }
    }

    /// Takes up a new format. Every stage learns it, and the ramp keeps its length in time.
    fn resync(&mut self) {
        let rate = self.frames.rate();
        let channels = self.frames.channels();
        self.equalizer.set_format(rate, channels);
        self.rate.store(rate, Ordering::Relaxed);
        self.probe.format(rate, channels);
        self.tap.format(rate, channels);
        self.route.hear(rate, channels);
        self.ramp_frames = (RAMP.as_secs_f64() * rate as f64).round().max(1.0) as u32;
        self.frames_left = self.frames_left.min(self.ramp_frames);
    }
}

impl<I: Source> Iterator for Chained<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let (sample, place) = self.frames.pull(&mut self.input)?;
        let Some(place) = place else {
            return Some(sample);
        };

        if place.changed {
            self.resync();
        }
        self.probe.hear(place.channel, sample);
        if place.channel == 0 {
            self.equalizer.begin();
            self.unscaled = self.tap.absolute();
            let requested = self.volume.get().max(0.0);

            if requested.to_bits() != self.target.to_bits() {
                self.target = requested;
                self.frames_left = self.ramp_frames;
                self.step = (self.target - self.current) / self.ramp_frames as f32;
            }

            if self.frames_left > 0 {
                self.current += self.step;
                self.frames_left -= 1;

                if self.frames_left == 0 {
                    self.current = self.target;
                }
            }
        }

        let equalized = self.equalizer.process(usize::from(place.channel), sample);
        let output = equalized * self.current;
        let heard = match self.unscaled {
            true => equalized,
            false => output,
        };
        self.tap.push(place.channel, heard);
        self.meter.record(output);

        Some(match self.clamped {
            true => output.clamp(-1., FULL_SCALE),
            false => output,
        })
    }
}

impl<I: Source> Source for Chained<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position)?;
        self.equalizer.reset();
        Ok(())
    }
}

pub struct Trimmed<I> {
    input: I,
    head: u64,
    body: Option<u64>,
    emitted: u64,
    primed: bool,
    rate: u64,
    channels: u64,
}

impl<I: Source> Trimmed<I> {
    pub fn new(input: I, skip: Duration, take: Option<Duration>) -> Self {
        let rate = input.sample_rate().get() as u64;
        let channels = input.channels().get() as u64;

        Self {
            head: whole_frames(skip, rate, channels),
            body: take.map(|take| whole_frames(take, rate, channels)),
            emitted: 0,
            primed: false,
            rate,
            channels,
            input,
        }
    }

    fn offset(&self) -> Duration {
        self.playing_time(self.head)
    }

    /// How long `samples` play for.
    fn playing_time(&self, samples: u64) -> Duration {
        Duration::from_secs_f64(samples as f64 / (self.rate * self.channels) as f64)
    }
}

impl<I: Source> Iterator for Trimmed<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.primed {
            self.primed = true;
            for _ in 0..self.head {
                self.input.next()?;
            }
        }
        if self.body.is_some_and(|body| self.emitted >= body) {
            return None;
        }

        let sample = self.input.next()?;
        self.emitted += 1;
        Some(sample)
    }
}

impl<I: Source> Source for Trimmed<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        match self.body {
            Some(body) => Some(self.playing_time(body)),
            None => self
                .input
                .total_duration()
                .map(|whole| whole.saturating_sub(self.offset())),
        }
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position + self.offset())?;
        self.primed = true;
        self.emitted = whole_frames(position, self.rate, self.channels);
        Ok(())
    }
}

/// Whether the system has a default output device to play on. A device can be missing for as
/// long as it takes a headset to reconnect, so the engines ask again rather than give up.
pub fn available() -> bool {
    cpal::default_host().default_output_device().is_some()
}

/// Whether a stream on the default device opens right now. The stream is the shared one, so it
/// closes again at once unless an output already plays on it.
pub fn openable() -> bool {
    available() && Device::shared(None).is_ok()
}

/// Whether the last attempt to open the output failed, so whatever plays is not heard. On Linux
/// ALSA always names a default device, so this rather than `available` is what tells that the
/// sound cannot get out.
pub fn failing() -> bool {
    FAILING.load(Ordering::Acquire)
}

/// The sound server this session runs when ALSA has no plugin routing to it. Sonora then opens
/// the card the server already holds and plays nothing, which `pipewire-alsa` or
/// `pulseaudio-alsa` fixes. Only ALSA's configuration is read, no device is opened.
#[cfg(target_os = "linux")]
pub fn missing_bridge() -> Option<Server> {
    let server = server()?;
    let hints = alsa::device_name::HintIter::new_str(None, "pcm").ok()?;
    let bridged = hints
        .filter_map(|hint| hint.name)
        .any(|name| name == "pipewire" || name == "pulse");
    (!bridged).then_some(server)
}

#[cfg(not(target_os = "linux"))]
pub fn missing_bridge() -> Option<Server> {
    None
}

/// The samples `span` covers at `rate` frames a second, rounded to whole frames, so a cut never
/// lands between the channels of one frame and swaps them for the rest of the track.
fn whole_frames(span: Duration, rate: u64, channels: u64) -> u64 {
    (span.as_secs_f64() * rate as f64).round() as u64 * channels
}

fn ident(device: &cpal::Device) -> String {
    device
        .id()
        .map(|id| id.to_string())
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// A fresh player whose chain plays into `device`, and the format word its chunks publish into.
fn attach(chain: &Chain, device: &Device) -> (rodio::Player, Format) {
    let format = Format::default();
    let (sink, source) = rodio::Player::new();
    device.add(Chained::new(source, chain, format.clone(), device.integer));
    (sink, format)
}

/// The device's config at `rate` in the default's channels and sample format, if the device
/// offers that combination.
fn at_rate(
    device: &cpal::Device,
    default: &cpal::SupportedStreamConfig,
    rate: u32,
) -> Option<cpal::SupportedStreamConfig> {
    if default.sample_rate() == rate {
        return Some(default.clone());
    }
    device
        .supported_output_configs()
        .ok()?
        .filter(|range| {
            range.channels() == default.channels()
                && range.sample_format() == default.sample_format()
        })
        .find_map(|range| range.try_with_sample_rate(rate))
}

/// The sound server whose socket this session has, PipeWire first since it also answers on the
/// PulseAudio socket.
#[cfg(target_os = "linux")]
fn server() -> Option<Server> {
    let runtime = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if runtime.join("pipewire-0").exists() {
        return Some(Server::PipeWire);
    }
    let pulse = std::env::var_os("PULSE_SERVER").is_some() || runtime.join("pulse/native").exists();
    pulse.then_some(Server::PulseAudio)
}
