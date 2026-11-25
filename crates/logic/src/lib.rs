//! What Sonora decides about listening, with no UI toolkit underneath.
//!
//! The desktop app wraps these types in GPUI entities, and the Android app calls them through
//! its native library, so both follow the same rules. Nothing here draws, spawns or notifies.

pub mod lyrics;
pub mod queue;
pub mod station;
