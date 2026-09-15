//! The system Widevine module, as the rest of the app has to talk about it.
//!
//! Apple Music is the only provider that needs one today, and nothing about it is Apple's: the
//! module belongs to the machine, which is why this sits in the crate root rather than in a
//! provider and why `state` can ask about it without naming a provider at all. The work is in
//! the `widevine` crate.

pub use widevine::{CDM_PATH, Found, Origin, available, find, installed, supported};
