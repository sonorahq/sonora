//! A surface that keeps presses from reaching what is under it.

use rune::{El, el};

/// An empty element that takes every press over it and does nothing with them, laid over
/// content that must not be reached, such as the page behind an open panel. The caller sizes
/// and places it. `S` is the window's state type, which the empty handler is typed by.
pub fn shield<S: 'static>(key: impl Into<String>) -> El {
    el().key(key).role("none").on_press(|_: &mut S| {})
}
