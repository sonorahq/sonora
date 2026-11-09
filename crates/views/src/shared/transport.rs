//! The playback controls the player bar and the fullscreen player share: the transport row, the
//! heart, and the helpers behind the volume and seek sliders. The player bar draws them first,
//! so they live there and this is where the rest of the app reaches them.

pub(crate) use crate::chrome::player_bar::{like, moved, open_menu, percent, transport, volume_icon};

/// How far one notch of the wheel over a volume control moves it.
pub(crate) const NOTCH: f32 = 0.05;
/// The level unmuting returns to when nothing was muted from.
pub(crate) const RESTORE: f32 = 0.7;
/// The match the playing cover carries in the player bar and in fullscreen, so opening and
/// leaving fullscreen flies the cover between the two.
pub(crate) const COVER_FLIGHT: &str = "playing-cover";

/// The volume one wheel step over a volume control asks for: a notch up or down from `level`.
pub(crate) fn turned(level: f32, dy: f32) -> f32 {
    let notch = match dy > 0. {
        true => NOTCH,
        false => -NOTCH,
    };
    (level + notch).clamp(0., 1.)
}
