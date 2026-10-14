//! CoreAudio's own view of an output device: the nominal rate its hardware runs at. cpal's default
//! config reads the stream format = takes 18 to 37 ms a call against this module's 50 to
//! 170 µs. The watcher asks every 500 ms for as long as a stream is open, so we have this optimized path.
//!
//! TODO: Re-assess once sonora moves to cpal 0.18. If its default config is as cheap to read, or
//! it reports rate changes on an open stream, this module and its unsafe code can go, along with
//! the `objc2-core-audio` dependency.

use std::ptr::{NonNull, null};

use cpal::traits::DeviceTrait as _;
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertySelector, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
};
use objc2_core_foundation::{CFRetained, CFString};

/// The hardware's nominal rate, or `None` when CoreAudio cannot find or answer for the device.
pub(super) fn rate(device: &cpal::Device) -> Option<u32> {
    let rate = nominal(object(device)?)?;
    (1.0..=f64::from(u32::MAX))
        .contains(&rate)
        .then(|| rate.round() as u32)
}

/// The CoreAudio object behind a cpal device, matched on the UID cpal reports as its id. Walking
/// every device rather than translating the UID keeps this working on macOS 11.
fn object(device: &cpal::Device) -> Option<AudioObjectID> {
    let uid = device.id().ok()?.1;
    list::<AudioObjectID>(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyDevices,
    )?
    .into_iter()
    .find(|&object| uid_of(object).as_deref() == Some(uid.as_str()))
}

fn uid_of(object: AudioObjectID) -> Option<String> {
    let address = address(kAudioDevicePropertyDeviceUID);
    let mut uid: *const CFString = null();
    let mut size = size_of::<*const CFString>() as u32;
    // SAFETY: the property is one CFString pointer, and `size` is the size of `uid`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::from(&mut uid).cast(),
        )
    };
    if status != 0 {
        return None;
    }
    // SAFETY: CoreAudio hands the UID over retained, so `CFRetained` takes that reference and
    // releases it.
    let uid = unsafe { CFRetained::from_raw(NonNull::new(uid.cast_mut())?) };
    Some(uid.to_string())
}

fn nominal(object: AudioObjectID) -> Option<f64> {
    list::<f64>(object, kAudioDevicePropertyNominalSampleRate)?
        .first()
        .copied()
}

/// Reads a property made of `T`s, which covers one-value properties too.
fn list<T: Copy>(object: AudioObjectID, selector: AudioObjectPropertySelector) -> Option<Vec<T>> {
    let address = address(selector);
    let mut size = 0u32;
    // SAFETY: this only writes the property's size in bytes into `size`.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 {
        return None;
    }

    let mut items = Vec::<T>::with_capacity(size as usize / size_of::<T>());
    // SAFETY: `items` has room for `size` bytes, and CoreAudio writes at most that many and
    // reports how many it wrote back into `size`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::new(items.as_mut_ptr())?.cast(),
        )
    };
    if status != 0 {
        return None;
    }
    // SAFETY: the call initialised `size` bytes from the start of `items`, which is this many
    // whole `T`s.
    unsafe { items.set_len(size as usize / size_of::<T>()) };
    Some(items)
}

fn address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}
