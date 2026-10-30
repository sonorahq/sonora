//! Exclusive access to an output device, which CoreAudio calls hog mode. It is what "Direct to
//! hardware" means on macOS: while Sonora hogs a device no other app can play through it, and
//! Sonora can set the device's own sample rate, so nothing between Sonora and the hardware
//! converts what it plays.
//!
//! Only macOS has hog mode. Elsewhere a `Hog` can never be made, so the code that holds one
//! compiles everywhere and nothing needs a `cfg` at its call sites.

#[cfg(not(target_os = "macos"))]
pub(crate) use elsewhere::Hog;
#[cfg(target_os = "macos")]
pub(crate) use macos::Hog;

/// The rate to run a device at for a track made at `track` Hz, out of the `available` rates it
/// offers. The track's own when the device has it, otherwise the nearest rate above that is a
/// whole multiple of it, which resamples cleanly, otherwise the highest the device offers. `None`
/// when the device offers nothing.
pub(crate) fn pick_rate(track: u32, available: &[u32]) -> Option<u32> {
    if available.contains(&track) {
        return Some(track);
    }
    available
        .iter()
        .copied()
        .filter(|rate| track > 0 && *rate > track && rate % track == 0)
        .min()
        .or_else(|| available.iter().copied().max())
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use anyhow::{Result, bail};

    /// Exclusive access to a device. There is none to hold on this system, so this has no values.
    pub(crate) enum Hog {}

    impl Hog {
        /// Always an error: hog mode is CoreAudio's.
        pub(crate) fn acquire(_id: &str) -> Result<Option<Self>> {
            bail!("hog mode is only available on macOS")
        }

        pub(crate) fn id(&self) -> &str {
            match *self {}
        }

        pub(crate) fn rates(&self) -> &[u32] {
            match *self {}
        }

        pub(crate) fn rate(&self) -> Option<u32> {
            match *self {}
        }

        pub(crate) fn set_rate(&mut self, _rate: u32) -> Result<()> {
            match *self {}
        }

        pub(crate) fn restore(&mut self) -> Result<()> {
            match *self {}
        }

        pub(crate) fn leave_at(&mut self, _rate: Option<u32>) {
            match *self {}
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::{NonNull, null};

    use anyhow::{Context as _, Result, bail};
    use objc2_core_audio::{
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData,
        kAudioDevicePropertyAvailableNominalSampleRates, kAudioDevicePropertyHogMode,
        kAudioDevicePropertyNominalSampleRate, kAudioHardwarePropertyTranslateUIDToDevice,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
        kAudioObjectUnknown,
    };
    use objc2_core_foundation::CFString;

    /// The rates a device that offers any rate between two bounds is taken to offer, which are the
    /// ones music is published and mastered at.
    const STANDARD: [f64; 9] = [
        32_000., 44_100., 48_000., 88_200., 96_000., 176_400., 192_000., 352_800., 384_000.,
    ];

    /// What `kAudioHardwarePropertyHogMode` reads when no process has the device.
    const FREE: i32 = -1;

    /// Exclusive access to one output device, held until this is dropped. Dropping it puts the
    /// device back at the rate it ran at when it was taken, and lets it go.
    pub(crate) struct Hog {
        object: AudioObjectID,
        /// cpal's id for the device, which settings store.
        id: String,
        /// The rate the device ran at when it was taken.
        original: f64,
        /// Whether `set_rate` has moved the device off `original`.
        changed: bool,
        /// The rates the device offers, lowest first.
        rates: Vec<u32>,
    }

    /// A raw range of rates, as CoreAudio lays one out.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Range {
        min: f64,
        max: f64,
    }

    impl Hog {
        /// Takes the device with cpal id `id`, or returns `None` when no such device is connected.
        /// It is an error when another app already has the device or CoreAudio refuses.
        pub(crate) fn acquire(id: &str) -> Result<Option<Self>> {
            let parsed: cpal::DeviceId = id.parse().context("cannot read the device id")?;
            let Some(object) = translate(parsed.id())? else {
                return Ok(None);
            };
            let pid = std::process::id() as i32;
            let owner: i32 =
                get(object, kAudioDevicePropertyHogMode).context("cannot read the owner")?;
            if owner != FREE && owner != pid {
                bail!("another app (process {owner}) has exclusive access to the device");
            }
            let original: f64 = get(object, kAudioDevicePropertyNominalSampleRate)
                .context("cannot read the sample rate")?;
            let rates = available(object).unwrap_or_else(|error| {
                log::warn!("output: cannot read the rates of {id}: {error:#}");
                Vec::new()
            });
            if owner != pid {
                set(object, kAudioDevicePropertyHogMode, pid).context("cannot take the device")?;
                let now: i32 = get(object, kAudioDevicePropertyHogMode)
                    .context("cannot confirm the device was taken")?;
                if now != pid {
                    bail!("the device did not give exclusive access");
                }
            }
            log::info!("output: exclusive access to {id}");
            Ok(Some(Self {
                object,
                id: id.to_owned(),
                original,
                changed: false,
                rates,
            }))
        }

        /// The cpal id of the device held.
        pub(crate) fn id(&self) -> &str {
            &self.id
        }

        /// The rates the device offers, lowest first. Empty when CoreAudio would not say.
        pub(crate) fn rates(&self) -> &[u32] {
            &self.rates
        }

        /// The rate the device runs at now.
        pub(crate) fn rate(&self) -> Option<u32> {
            let rate: f64 = get(self.object, kAudioDevicePropertyNominalSampleRate).ok()?;
            Some(rate.round() as u32)
        }

        /// Sets the rate the device runs at. The device may take a moment to follow.
        pub(crate) fn set_rate(&mut self, rate: u32) -> Result<()> {
            set(
                self.object,
                kAudioDevicePropertyNominalSampleRate,
                f64::from(rate),
            )
            .with_context(|| format!("cannot set the device to {rate} Hz"))?;
            self.changed = true;
            Ok(())
        }

        /// Puts the device back at the rate it ran at before it was taken, if it was moved.
        pub(crate) fn restore(&mut self) -> Result<()> {
            if !self.changed {
                return Ok(());
            }
            set(
                self.object,
                kAudioDevicePropertyNominalSampleRate,
                self.original,
            )
            .context("cannot restore the device's rate")?;
            self.changed = false;
            Ok(())
        }

        /// Has the device be left at `rate`, when it offers it, rather than the rate it ran at
        /// before it was taken, once this is dropped.
        pub(crate) fn leave_at(&mut self, rate: Option<u32>) {
            if let Some(rate) = rate.filter(|rate| self.rates.contains(rate)) {
                self.original = f64::from(rate);
                self.changed = true;
            }
        }
    }

    impl Drop for Hog {
        fn drop(&mut self) {
            if self.changed
                && let Err(error) = set(
                    self.object,
                    kAudioDevicePropertyNominalSampleRate,
                    self.original,
                )
            {
                log::warn!("output: cannot restore the rate of {}: {error:#}", self.id);
            }
            if let Err(error) = set(self.object, kAudioDevicePropertyHogMode, FREE) {
                log::warn!("output: cannot release {}: {error:#}", self.id);
            }
            log::info!("output: released exclusive access to {}", self.id);
        }
    }

    /// The property `selector` of a whole device.
    fn address(selector: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    /// Turns a CoreAudio status into an error, naming it by number as CoreAudio documents them.
    fn check(status: i32) -> Result<()> {
        match status {
            0 => Ok(()),
            status => bail!("CoreAudio error {status}"),
        }
    }

    /// Reads the property `selector` of `object`, which holds one `T`.
    fn get<T: Copy + Default>(object: AudioObjectID, selector: u32) -> Result<T> {
        let address = address(selector);
        let mut value = T::default();
        let mut size = size_of::<T>() as u32;
        // SAFETY: the address and size live through the call, and `value` is a `T`, the size
        // given, which is what these properties hold.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast(),
            )
        };
        check(status)?;
        Ok(value)
    }

    /// Writes `value` to the property `selector` of `object`.
    fn set<T: Copy>(object: AudioObjectID, selector: u32, mut value: T) -> Result<()> {
        let address = address(selector);
        // SAFETY: the address and value live through the call, and the size given is the value's.
        let status = unsafe {
            AudioObjectSetPropertyData(
                object,
                NonNull::from(&address),
                0,
                null(),
                size_of::<T>() as u32,
                NonNull::from(&mut value).cast(),
            )
        };
        check(status)
    }

    /// The device whose UID is `uid`, or `None` when none is connected.
    fn translate(uid: &str) -> Result<Option<AudioObjectID>> {
        let uid = CFString::from_str(uid);
        let qualifier: *const CFString = &*uid;
        let address = address(kAudioHardwarePropertyTranslateUIDToDevice);
        let mut object = kAudioObjectUnknown;
        let mut size = size_of::<AudioObjectID>() as u32;
        // SAFETY: the qualifier is the pointer to the UID's string, as this property takes, and
        // the address, size and output live through the call.
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                size_of::<*const CFString>() as u32,
                (&raw const qualifier).cast::<c_void>(),
                NonNull::from(&mut size),
                NonNull::from(&mut object).cast(),
            )
        };
        check(status).context("cannot look the device up")?;
        Ok((object != kAudioObjectUnknown).then_some(object))
    }

    /// The rates `object` offers, lowest first. A range of rates counts for the standard rates
    /// inside it.
    fn available(object: AudioObjectID) -> Result<Vec<u32>> {
        let address = address(kAudioDevicePropertyAvailableNominalSampleRates);
        let mut bytes = 0u32;
        // SAFETY: the address and size live through the call.
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                object,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut bytes),
            )
        };
        check(status)?;
        let mut ranges = vec![Range { min: 0., max: 0. }; bytes as usize / size_of::<Range>()];
        if ranges.is_empty() {
            return Ok(Vec::new());
        }
        let mut size = (ranges.len() * size_of::<Range>()) as u32;
        // SAFETY: the buffer holds `size` bytes of ranges, which is what this property fills.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut size),
                NonNull::from(ranges.as_mut_slice()).cast(),
            )
        };
        check(status)?;
        let mut rates: Vec<u32> = ranges
            .iter()
            .flat_map(|range| match range.min == range.max {
                true => vec![range.min],
                false => STANDARD
                    .iter()
                    .copied()
                    .filter(|rate| (range.min..=range.max).contains(rate))
                    .collect(),
            })
            .map(|rate| rate.round() as u32)
            .collect();
        rates.sort_unstable();
        rates.dedup();
        Ok(rates)
    }
}
