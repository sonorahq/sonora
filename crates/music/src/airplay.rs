//! Whether the sound is leaving the machine over AirPlay.
//!
//! AirPlay buffers about two seconds ahead of the speaker, so the decoder is that far in
//! front of what the listener hears. Only macOS can be asked, through CoreAudio's transport
//! type for the default output device; every other platform answers `false` and leaves the
//! offset to the user.

/// How far behind the engine an AirPlay output plays. Apple's receivers buffer to a fixed
/// two-second horizon, which is why the delay can be a constant rather than a measurement.
pub const LATENCY: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;

    /// `kAudioObjectSystemObject`.
    const SYSTEM: u32 = 1;
    /// `kAudioHardwarePropertyDefaultOutputDevice`, `'dOut'`.
    const DEFAULT_OUTPUT: u32 = u32::from_be_bytes(*b"dOut");
    /// `kAudioDevicePropertyTransportType`, `'tran'`.
    const TRANSPORT: u32 = u32::from_be_bytes(*b"tran");
    /// `kAudioObjectPropertyScopeGlobal`, `'glob'`.
    const GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    /// `kAudioDeviceTransportTypeAirPlay`, `'airp'`.
    const AIRPLAY: u32 = u32::from_be_bytes(*b"airp");
    /// `kAudioObjectPropertyElementMain`.
    const MAIN: u32 = 0;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    /// Reads one fixed-size property off an audio object, or `None` when CoreAudio refuses.
    fn property(object: u32, selector: u32) -> Option<u32> {
        let address = Address {
            selector,
            scope: GLOBAL,
            element: MAIN,
        };
        let mut value: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        // Every pointer is to a local live for the call, and the size matches the property.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                (&raw mut value).cast::<c_void>(),
            )
        };
        (status == 0).then_some(value)
    }

    pub fn engaged() -> bool {
        let Some(device) = property(SYSTEM, DEFAULT_OUTPUT).filter(|device| *device != 0) else {
            return false;
        };
        property(device, TRANSPORT) == Some(AIRPLAY)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn engaged() -> bool {
        false
    }
}

/// Whether the system's default output is an AirPlay device right now. Asking costs a
/// CoreAudio round trip, so the answer is worth holding until the output changes.
pub fn engaged() -> bool {
    platform::engaged()
}
