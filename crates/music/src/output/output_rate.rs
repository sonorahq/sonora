//! The config an output device is opened in, and whether it still runs at the rate a stream was
//! opened at. Sonora opens the device at the rate it really runs at and resamples every track to it
//! with ardftsrc, so nothing between Sonora and the hardware converts it again. On Linux a rate
//! picked in settings is asked for instead, where the stream's rate reaches the hardware.
//!
//! On Windows the default config is the shared-mode engine's mix format, and the engine
//! invalidates a stream whose format changes, which fails it, so it is not watched. On macOS the
//! default config's rate is the hardware's nominal rate. cpal reports a change of it as an error
//! on a stream opened on a named device, but not on one that follows the system default, so the
//! rate is watched there.
//!
//! On Linux a sound server's default config is the rate it runs the device at. A picked rate
//! opens the stream at that rate, on PulseAudio, `pipewire-pulse` included, and on ALSA alike, and
//! the server moves the device to it where it allows that rate.
//!
//! Every output opens in stereo at most, whatever channels the device has. rodio shifts every
//! channel over by one when a player starts on an output wider than stereo, so surround setups
//! heard the left channel from the centre speaker. The system maps stereo onto the speakers
//! itself: WASAPI and CoreAudio convert the stream, and a sound server mixes it.

use cpal::SupportedStreamConfig;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use cpal::traits::DeviceTrait as _;

#[cfg(target_os = "linux")]
use super::output_host;

/// The rates a device can be asked for in settings, from the lowest any music is published at to
/// the highest any is mastered at.
#[cfg(target_os = "linux")]
const RATES: [u32; 7] = [32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// The config to open `device` in, given `default`, its default one, in stereo at most. That is
/// the device's real rate on macOS and Windows, so `wanted` is not asked for there.
#[cfg(not(target_os = "linux"))]
pub(crate) fn config(
    device: &cpal::Device,
    default: SupportedStreamConfig,
    _wanted: Option<u32>,
) -> SupportedStreamConfig {
    stereo(device, default)
}

/// The config to open `device` in, given `default`, its default one: the same at `wanted` when
/// the device offers it in the default's channels and sample format. A sound server is asked for
/// stereo in float, which it mixes in anyway, and a sound card for stereo when it offers it.
#[cfg(target_os = "linux")]
pub(crate) fn config(
    device: &cpal::Device,
    default: SupportedStreamConfig,
    wanted: Option<u32>,
) -> SupportedStreamConfig {
    let default = match output_host::served(device) {
        true => for_server(default),
        false => stereo(device, default),
    };
    wanted
        .and_then(|rate| at_rate(device, &default, rate))
        .unwrap_or(default)
}

/// The rates `device` can be opened at in `config`, lowest first. That is only the rate it runs
/// at on macOS and Windows, where the system sets it.
#[cfg(not(target_os = "linux"))]
pub(crate) fn rates(_device: &cpal::Device, config: &SupportedStreamConfig) -> Vec<u32> {
    vec![config.sample_rate()]
}

/// The rates in `RATES` that `device` can be opened at in `config`'s channels and sample format,
/// with the rate `config` runs at, lowest first.
#[cfg(target_os = "linux")]
pub(crate) fn rates(device: &cpal::Device, config: &SupportedStreamConfig) -> Vec<u32> {
    let Ok(ranges) = device.supported_output_configs() else {
        return vec![config.sample_rate()];
    };
    let ranges: Vec<_> = ranges
        .filter(|range| {
            range.channels() == config.channels() && range.sample_format() == config.sample_format()
        })
        .collect();
    let mut rates: Vec<u32> = RATES
        .into_iter()
        .filter(|&rate| ranges.iter().any(|range| range.contains_rate(rate)))
        .collect();
    // a device running at a rate off the list still shows it, between its neighbours
    if !rates.contains(&config.sample_rate()) {
        rates.push(config.sample_rate());
        rates.sort_unstable();
    }
    rates
}

/// Whether `device` would now open at another rate than `rate`, the one its stream runs at, where
/// the platform would convert the stream rather than fail it. Only a sound server on Linux does.
#[cfg(target_os = "linux")]
pub(crate) fn moved(device: &cpal::Device, rate: u32, wanted: Option<u32>) -> bool {
    // ALSA's default config opens the device, which a stream on it already holds
    if !output_host::served(device) {
        return false;
    }
    device
        .default_output_config()
        .is_ok_and(|default| config(device, default, wanted).sample_rate() != rate)
}

/// Whether `device` now runs at another rate than `rate`, the one its stream runs at. CoreAudio
/// converts a stream that follows the system default rather than fail it.
#[cfg(target_os = "macos")]
pub(crate) fn moved(device: &cpal::Device, rate: u32, _wanted: Option<u32>) -> bool {
    device
        .default_output_config()
        .is_ok_and(|default| default.sample_rate() != rate)
}

/// Whether `device` would now open at another rate than `rate`, where the platform would convert
/// the stream rather than fail it. WASAPI fails it instead.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn moved(_device: &cpal::Device, _rate: u32, _wanted: Option<u32>) -> bool {
    false
}

/// `config` in stereo, or mono for a mono sink, and in 32-bit float.
#[cfg(target_os = "linux")]
fn for_server(config: SupportedStreamConfig) -> SupportedStreamConfig {
    SupportedStreamConfig::new(
        config.channels().min(2),
        config.sample_rate(),
        *config.buffer_size(),
        cpal::SampleFormat::F32,
    )
}

/// `config` in stereo when it has more channels. WASAPI and CoreAudio take a stereo stream on any
/// device and map it onto the speakers, so the device is not asked.
#[cfg(not(target_os = "linux"))]
fn stereo(_device: &cpal::Device, config: SupportedStreamConfig) -> SupportedStreamConfig {
    match config.channels() > 2 {
        true => SupportedStreamConfig::new(
            2,
            config.sample_rate(),
            *config.buffer_size(),
            config.sample_format(),
        ),
        false => config,
    }
}

/// `config` in stereo when it has more channels and the sound card offers stereo at its rate and
/// sample format. A card that only takes all its channels keeps them.
#[cfg(target_os = "linux")]
fn stereo(device: &cpal::Device, config: SupportedStreamConfig) -> SupportedStreamConfig {
    if config.channels() <= 2 {
        return config;
    }
    let offered = device
        .supported_output_configs()
        .ok()
        .and_then(|mut ranges| {
            ranges.find_map(|range| {
                (range.channels() == 2 && range.sample_format() == config.sample_format())
                    .then(|| range.try_with_sample_rate(config.sample_rate()))
                    .flatten()
            })
        });
    match offered {
        Some(stereo) => stereo,
        None => {
            log::warn!(
                "output: the card takes only {} channels, so it opens with all of them",
                config.channels()
            );
            config
        }
    }
}

/// `like` at `rate`, if the device offers that rate in its channels and sample format.
#[cfg(target_os = "linux")]
fn at_rate(
    device: &cpal::Device,
    like: &SupportedStreamConfig,
    rate: u32,
) -> Option<SupportedStreamConfig> {
    if like.sample_rate() == rate {
        return Some(*like);
    }
    device
        .supported_output_configs()
        .ok()?
        .filter(|range| {
            range.channels() == like.channels() && range.sample_format() == like.sample_format()
        })
        .find_map(|range| range.try_with_sample_rate(rate))
}
