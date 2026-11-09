use std::time::Duration;

use i18n::{SharedString, t};

pub fn clock(value: Duration) -> SharedString {
    let total = value.as_secs();
    let hours = total / 3600;
    let minutes = total % 3600 / 60;
    let seconds = total % 60;
    match hours {
        0 => std::borrow::Cow::Owned(format!("{minutes}:{seconds:02}")),
        _ => std::borrow::Cow::Owned(format!("{hours}:{minutes:02}:{seconds:02}")),
    }
}

/// A total running time in words, such as "46m 15s", or "1h 2m" once it passes an hour. It
/// is for the length of a whole collection, where a clock reading would look like a track.
pub fn runtime(value: Duration) -> SharedString {
    let total = value.as_secs();
    let hours = total / 3600;
    let minutes = total % 3600 / 60;
    let seconds = total % 60;
    match (hours, minutes) {
        (0, 0) => t!("runtime-seconds", seconds = seconds),
        (0, _) => t!("runtime-minutes", minutes = minutes, seconds = seconds),
        _ => t!("runtime-hours", hours = hours, minutes = minutes),
    }
}

/// Font features that give every digit the same width, so a column of clock
/// values lines up like monospace text while staying in the UI font.
pub const TABULAR: &str = "'tnum' 1";
