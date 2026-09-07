//! Various visualizations for the Sonora UI.
//!
//! The idea here is that we may have multiple different visualizations that can be used in different contexts.
//! For now, we just have the frame glow used around artwork on the player bar and in fullscreen.

mod frame_glow;

pub(crate) use frame_glow::{FrameGlow, Glow, GlowBlur, GlowColor};
