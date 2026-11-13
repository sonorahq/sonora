//! Which of cpal's Linux hosts Sonora plays through, and what each one's device list holds. cpal
//! picks PulseAudio when its socket answers, which PipeWire serves through `pipewire-pulse`, and
//! ALSA otherwise. A sound server lists its outputs by the names the desktop shows, while ALSA
//! lists every PCM its configuration defines.
//!
//! cpal says nothing of a sink's ports or of the format the server runs it at, and keeps its
//! connection to itself, so those are read over a second connection of the same `pulseaudio`
//! crate.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::{Arc, Mutex, PoisonError};

use cpal::traits::{DeviceTrait as _, HostTrait as _};
use pulseaudio::protocol::port_info::PortAvailable;
use pulseaudio::protocol::{SampleFormat, SinkInfo};

use super::{DeviceOption, Format, Port, Sample, Sink, output_alsa};

/// The PulseAudio host every caller shares while its connection to the server holds.
static SERVER_HOST: Mutex<Option<ServerHost>> = Mutex::new(None);

/// The connection the server's sinks are read over, shared and made again once lost like the host.
static SERVER_CLIENT: Mutex<Option<pulseaudio::Client>> = Mutex::new(None);

/// A shared host and the last device-list error, so a persistent failure is logged only once.
struct ServerHost {
    host: Arc<cpal::Host>,
    listing_error: Option<cpal::Error>,
}

/// The host Sonora plays through. A PulseAudio host is made once and shared, and made again once
/// its connection to the server is lost, while an ALSA host is made afresh, so a server that
/// starts later is still found.
pub(super) fn host() -> Arc<cpal::Host> {
    let mut shared = SERVER_HOST.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(cached) = shared.as_mut() {
        match cached.host.output_devices() {
            Ok(_) => {
                cached.listing_error = None;
                return cached.host.clone();
            }
            Err(error) => {
                if cached.listing_error.as_ref() != Some(&error) {
                    log::warn!("output: cannot list audio devices: {error}");
                    cached.listing_error = Some(error.clone());
                }
                if !matches!(
                    error.kind(),
                    cpal::ErrorKind::HostUnavailable | cpal::ErrorKind::StreamInvalidated
                ) {
                    return cached.host.clone();
                }
            }
        }
    }
    let host = Arc::new(cpal::default_host());
    *shared = is_server(&host).then(|| ServerHost {
        host: host.clone(),
        listing_error: None,
    });
    host
}

/// Whether `host` is a sound server, which mixes every app's streams and routes them to its
/// sinks, rather than ALSA, which opens the device itself.
pub(super) fn is_server(host: &cpal::Host) -> bool {
    host.id() == cpal::HostId::PulseAudio
}

/// Whether `device` belongs to a sound server's host.
pub(super) fn served(device: &cpal::Device) -> bool {
    device
        .id()
        .is_ok_and(|id| id.host() == cpal::HostId::PulseAudio)
}

/// `option`, made from `device`, as the device list shows it, or `None` when the list leaves it
/// out. ALSA's devices are tidied by `output_alsa::tidy`.
pub(super) fn listed(device: &cpal::Device, option: DeviceOption) -> Option<DeviceOption> {
    let id = device.id().ok()?;
    match id.host() {
        cpal::HostId::Alsa => output_alsa::tidy(id.id(), option),
        _ => Some(option),
    }
}

/// The id of the sink the server plays its default output on, as `devices` lists it, or `None`
/// on ALSA, whose default is left out of the list. The server names the sink behind its default
/// when asked for it, `pipewire-pulse` from PipeWire's own default.
pub(super) fn system_default(host: &cpal::Host) -> Option<String> {
    match host.id() {
        cpal::HostId::PulseAudio => Some(host.default_output_device()?.id().ok()?.to_string()),
        _ => None,
    }
}

/// The ports and formats of every sink the server offers, keyed by the id `devices` lists each
/// sink under. Empty when the server does not answer.
pub(super) fn sinks() -> HashMap<String, Sink> {
    let Some(sinks) = ask(async |client| client.list_sinks().await) else {
        return HashMap::new();
    };
    sinks
        .iter()
        .map(|sink| (sink_id(&sink.name), of(sink)))
        .collect()
}

/// The ports and format of the sink behind `device`, a sound server's device id. `None` for any
/// other device, and when the server does not answer or no longer has the sink.
pub(super) fn sink(device: &str) -> Option<Sink> {
    let id: cpal::DeviceId = device.parse().ok()?;
    if id.host() != cpal::HostId::PulseAudio {
        return None;
    }
    let name = CString::new(id.id()).ok()?;
    ask(async move |client| client.sink_info_by_name(name).await).map(|sink| of(&sink))
}

/// Runs `call` on the shared client and waits for it. A failure is logged and gives `None`, and
/// one that lost the connection drops the client, so the next call connects again.
fn ask<T>(call: impl AsyncFnOnce(pulseaudio::Client) -> pulseaudio::Result<T>) -> Option<T> {
    let client = client()?;
    match futures::executor::block_on(call(client)) {
        Ok(answer) => Some(answer),
        Err(error) => {
            log::debug!("output: the sound server did not answer: {error}");
            if matches!(
                error,
                pulseaudio::ClientError::Disconnected | pulseaudio::ClientError::Io(_)
            ) {
                *SERVER_CLIENT.lock().unwrap_or_else(PoisonError::into_inner) = None;
            }
            None
        }
    }
}

/// The shared client, connected now if it is not yet. `None` when no server answers, which is
/// only tried where cpal plays through one.
fn client() -> Option<pulseaudio::Client> {
    let mut shared = SERVER_CLIENT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(client) = shared.as_ref() {
        return Some(client.clone());
    }
    if !is_server(&host()) {
        return None;
    }
    let client = pulseaudio::Client::from_env(c"Sonora")
        .inspect_err(|error| log::debug!("output: cannot reach the sound server: {error}"))
        .ok()?;
    *shared = Some(client.clone());
    Some(client)
}

/// The id `devices` lists the sink named `name` under, the one cpal gives it.
fn sink_id(name: &CStr) -> String {
    cpal::DeviceId::new(cpal::HostId::PulseAudio, name.to_string_lossy()).to_string()
}

/// What the server says of `sink`, in Sonora's terms.
fn of(sink: &SinkInfo) -> Sink {
    let ports: Vec<Port> = sink
        .ports
        .iter()
        .map(|port| Port {
            id: port.name.to_string_lossy().into_owned(),
            name: port
                .description
                .as_ref()
                .unwrap_or(&port.name)
                .to_string_lossy()
                .into_owned(),
            available: port.available != PortAvailable::No,
        })
        .collect();
    let spec = sink.sample_spec;
    let sample = match spec.format {
        SampleFormat::U8 => Sample::Integer(8),
        SampleFormat::S16Le | SampleFormat::S16Be => Sample::Integer(16),
        SampleFormat::S24Le
        | SampleFormat::S24Be
        | SampleFormat::S24In32Le
        | SampleFormat::S24In32Be => Sample::Integer(24),
        SampleFormat::S32Le | SampleFormat::S32Be => Sample::Integer(32),
        SampleFormat::Float32Le | SampleFormat::Float32Be => Sample::Float(32),
        _ => Sample::Other,
    };
    let format = (spec.sample_rate > 0 && spec.channels > 0).then_some(Format {
        rate: spec.sample_rate,
        channels: u16::from(spec.channels),
        sample,
    });
    Sink {
        active: (sink.active_port < ports.len()).then_some(sink.active_port),
        ports,
        format,
    }
}
