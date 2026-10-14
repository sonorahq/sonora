//! The config an output device is opened in, and whether it still runs at the rate a stream was
//! opened at. Sonora opens the device at the rate it really runs at and resamples every track to it
//! with ardftsrc, so nothing between Sonora and the hardware converts it again.
//!
//! On Windows the default config is the shared-mode engine's mix format, and the engine
//! invalidates a stream whose format changes, which fails it. On macOS the default config's rate
//! is the hardware's nominal rate, but a stream whose rate changes is converted rather than told,
//! so the watch has to look for the change itself.
//!
//! TODO: On Linux, cpal 0.17 opens ALSA's `default` device, which on most desktops is PipeWire's
//! ALSA plugin. Its default config is 44.1 kHz whatever the graph runs at, and PipeWire converts to
//! the graph's rate unseen, so `config` guesses 48 kHz, PipeWire's and PulseAudio's default, which
//! is right on most desktops and leaves one hidden conversion on the rest. When Sonora moves to
//! rodio 0.23 and cpal 0.18, open cpal's PipeWire host when its socket is reachable (checking the
//! Flatpak's read-only `xdg-run/pipewire-0` is enough), whose default config is the graph's
//! `clock.rate`, drop the guess, and have `moved` watch that rate.

use cpal::SupportedStreamConfig;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use cpal::traits::DeviceTrait as _;

#[cfg(target_os = "macos")]
use super::output_rate_macos as macos;

/// The rate a sound server on Linux most likely runs its graph at.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SERVER_RATE: u32 = 48_000;

/// The config to open `device` in, given `default`, its default one. That is the device's real
/// rate on macOS and Windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn config(
    _device: &cpal::Device,
    default: SupportedStreamConfig,
) -> SupportedStreamConfig {
    default
}

/// The config to open `device` in, given `default`, its default one: the same at 48 kHz when the
/// device offers it in the default's channels and sample format, since cpal's default is 44.1 kHz
/// whatever the sound server runs at.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn config(
    device: &cpal::Device,
    default: SupportedStreamConfig,
) -> SupportedStreamConfig {
    let Ok(ranges) = device.supported_output_configs() else {
        return default;
    };
    ranges
        .filter(|range| {
            range.channels() == default.channels()
                && range.sample_format() == default.sample_format()
        })
        .find_map(|range| range.try_with_sample_rate(SERVER_RATE))
        .unwrap_or(default)
}

/// Whether `device` has moved off `rate` since a stream opened at it, where the platform would
/// convert the stream rather than fail it.
#[cfg(target_os = "macos")]
pub(crate) fn moved(device: &cpal::Device, rate: u32) -> bool {
    macos::rate(device).is_some_and(|now| now != rate)
}

/// Whether `device` has moved off `rate` since a stream opened at it, where the platform would
/// convert the stream rather than fail it.
#[cfg(not(target_os = "macos"))]
pub(crate) fn moved(_device: &cpal::Device, _rate: u32) -> bool {
    false
}
