use std::sync::Arc;
use std::time::Duration;

use gpui::{FontFeatures, SharedString};

pub fn clock(value: Duration) -> SharedString {
    let total = value.as_secs();
    let hours = total / 3600;
    let minutes = total % 3600 / 60;
    let seconds = total % 60;
    match hours {
        0 => SharedString::from(format!("{minutes}:{seconds:02}")),
        _ => SharedString::from(format!("{hours}:{minutes:02}:{seconds:02}")),
    }
}

/// Font features that give every digit the same width, so a column of clock
/// values lines up like monospace text while staying in the UI font.
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}
