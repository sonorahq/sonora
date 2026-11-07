//! The Widevine module, as the rest of the app has to talk about it.
//!
//! Apple Music and Spotify cookie playback need one, and nothing about it is provider-specific: the
//! module belongs to the machine, which is why this sits in the crate root rather than in a
//! provider and why `state` can ask about it without naming a provider at all. The work is in
//! the `widevine` crate.

pub use widevine::{
    CDM_PATH, Found, Offer, Origin, available, fetch, find, host, hosted, offer, supported,
    uninstall,
};
