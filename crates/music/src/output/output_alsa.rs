//! ALSA Host specific functionality.

use super::{DeviceOption, Server};

/// ALSA devices the device list leaves out. `default` is the device "System default" opens, and
/// `null` plays nothing.
const HIDDEN_PCMS: [&str; 2] = ["default", "null"];

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

/// The sound server this session runs when ALSA has no plugin routing to it. Sonora then opens
/// the card the server already holds and plays nothing, which `pipewire-alsa` or
/// `pulseaudio-alsa` fixes. Only ALSA's configuration is read, no device is opened, and nothing
/// is missing when Sonora plays through another host.
pub fn missing_bridge() -> Option<Server> {
    if !is_alsa(&cpal::default_host()) {
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
