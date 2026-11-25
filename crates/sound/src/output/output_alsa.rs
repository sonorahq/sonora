//! What Sonora needs of ALSA: tidying its device list when it is the host, finding the sound
//! server it should reach through a plugin, and each sound card's own device, which plays past the
//! server.

use std::ffi::CString;

use cpal::traits::{DeviceTrait as _, HostTrait as _};

use super::{DeviceOption, Server, cpal_describe};

/// ALSA devices the device list leaves out. `default` is the device "System default" opens, and
/// `null` plays nothing.
const HIDDEN_PCMS: [&str; 2] = ["default", "null"];

/// The prefix of a sound card's own device, which ALSA opens with no conversion of any kind.
const DIRECT: &str = "hw";

/// The prefixes of the devices cpal names by card number, which `named` names by card id.
const NUMBERED: [&str; 2] = ["hw", "plughw"];

/// Whether `host` is ALSA, whose devices need what this module does.
pub(super) fn is_alsa(host: &cpal::Host) -> bool {
    host.id() == cpal::HostId::Alsa
}

/// The ALSA device named `pcm` as the device list shows it, split into its card and label, or
/// `None` when it is one of `HIDDEN_PCMS`.
pub(super) fn tidy(pcm: &str, option: DeviceOption) -> Option<DeviceOption> {
    if HIDDEN_PCMS.contains(&pcm) {
        return None;
    }
    Some(split_card(option))
}

/// Whether `pcm` is a sound card's own device, the one ALSA device offered past a sound server.
pub(super) fn is_direct(pcm: &str) -> bool {
    split(pcm).is_some_and(|(prefix, _, _)| prefix == DIRECT)
}

/// Every sound card's own playback devices, for playing past the sound server. Each card has one
/// for each of its outputs, named "card, output" as the system names them.
pub(super) fn direct_devices() -> Vec<DeviceOption> {
    let Ok(host) = cpal::host_from_id(cpal::HostId::Alsa) else {
        return Vec::new();
    };
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let pcm = named(device.id().ok()?.id());
            if !is_direct(&pcm) {
                return None;
            }
            // the description only says that the device converts nothing, which the group says
            let (name, _) = cpal_describe(&device);
            Some(DeviceOption {
                id: cpal::DeviceId::new(cpal::HostId::Alsa, &pcm).to_string(),
                label: name.clone(),
                name,
                card: None,
                detail: None,
                system: false,
                direct: true,
                sink: None,
            })
        })
        .collect()
}

/// The ALSA device saved as `pcm`, found on `host` when ALSA is the host. While a sound server
/// plays, only a sound card's own device is looked for, on ALSA's host beside it.
pub(super) fn device(host: &cpal::Host, pcm: &str) -> Option<cpal::Device> {
    let id = cpal::DeviceId::new(cpal::HostId::Alsa, numbered(pcm));
    if is_alsa(host) {
        return host.device_by_id(&id);
    }
    if !is_direct(pcm) {
        return None;
    }
    cpal::host_from_id(cpal::HostId::Alsa)
        .ok()?
        .device_by_id(&id)
}

/// `pcm` with its card named by the card's id, as in `hw:CARD=M1,DEV=0`, where cpal numbers it.
/// Numbers change as cards come and go while ids stay, so this is the form Sonora saves. Any other
/// device comes back as it is.
pub(super) fn named(pcm: &str) -> String {
    let Some((prefix, card, device)) =
        split(pcm).filter(|(prefix, _, _)| NUMBERED.contains(prefix))
    else {
        return pcm.to_owned();
    };
    let Some(name) = card.parse().ok().and_then(card_id) else {
        return pcm.to_owned();
    };
    format!("{prefix}:CARD={name},DEV={device}")
}

/// The sound server this session runs when ALSA has no plugin routing to it. Sonora then opens
/// the card the server already holds and plays nothing, which `pipewire-alsa` or
/// `pulseaudio-alsa` fixes. Only ALSA's configuration is read, no device is opened, and nothing
/// is missing when Sonora plays through another host.
pub fn missing_bridge() -> Option<Server> {
    if !is_alsa(&super::host()) {
        return None;
    }
    let server = server()?;
    let hints = alsa::device_name::HintIter::new_str(None, "pcm").ok()?;
    let bridged = hints
        .filter_map(|hint| hint.name)
        .any(|name| name == "pipewire" || name == "pulse");
    (!bridged).then_some(server)
}

/// The sound server whose socket this session has, PipeWire first since it also answers on the
/// PulseAudio socket.
fn server() -> Option<Server> {
    let runtime = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if runtime.join("pipewire-0").exists() {
        return Some(Server::PipeWire);
    }
    let pulse = std::env::var_os("PULSE_SERVER").is_some() || runtime.join("pulse/native").exists();
    pulse.then_some(Server::PulseAudio)
}

/// Splits the device's name, "card, device", into its card and its label. A name without both
/// halves is left whole, with no card.
fn split_card(option: DeviceOption) -> DeviceOption {
    let Some((card, device)) = option.name.split_once(',') else {
        return option;
    };
    let (card, label) = (card.trim().to_owned(), device.trim().to_owned());
    if card.is_empty() || label.is_empty() {
        return option;
    }
    DeviceOption {
        card: Some(card),
        label,
        ..option
    }
}

/// `pcm` with its card numbered again, which is how cpal finds it. A card that is not there is
/// left named.
fn numbered(pcm: &str) -> String {
    let Some((prefix, card, device)) = split(pcm)
        .filter(|(prefix, card, _)| NUMBERED.contains(prefix) && card.parse::<u32>().is_err())
    else {
        return pcm.to_owned();
    };
    let Some(index) = CString::new(card)
        .ok()
        .and_then(|card| alsa::card::Card::from_str(&card).ok())
    else {
        return pcm.to_owned();
    };
    format!("{prefix}:CARD={},DEV={device}", index.get_index())
}

/// The id of the sound card numbered `index`, such as `M1`.
fn card_id(index: u32) -> Option<String> {
    let ctl = alsa::Ctl::new(&format!("hw:{index}"), false).ok()?;
    let info = ctl.card_info().ok()?;
    info.get_id().ok().map(str::to_owned)
}

/// Splits a device of the form `prefix:CARD=card,DEV=device` into its prefix, card and device.
fn split(pcm: &str) -> Option<(&str, &str, &str)> {
    let (prefix, rest) = pcm.split_once(':')?;
    let (card, device) = rest.strip_prefix("CARD=")?.split_once(",DEV=")?;
    Some((prefix, card, device))
}
