//! macOS trackpad page swipes.
//!
//! macOS's "Swipe between pages" gesture, and the back and forward buttons of many mice, reach
//! the app as an `NSEventTypeSwipe` event. Rune hears wheel, pinch and rotate gestures but not
//! swipes, so a local monitor reads the event before it is passed to the window and moves
//! through the navigation history.

use std::ptr::NonNull;

use objc2_app_kit::{NSEvent, NSEventMask};
use views::Ui;

#[derive(Clone, Copy)]
enum Direction {
    Back,
    Forward,
}

/// Asks AppKit for this app's swipe events and goes back or forward on each one. Call it on the
/// main thread once the app runs.
pub fn install() {
    let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let raw = event;
        // SAFETY: AppKit hands the monitor a live event for the length of the call.
        let event = unsafe { event.as_ref() };
        let Some(direction) = direction(event) else {
            return raw.as_ptr();
        };
        crate::handle().update(move |ui: &mut Ui, _| match direction {
            Direction::Back => ui.back(),
            Direction::Forward => ui.forward(),
        });
        // swallowed, so nothing else reads the same swipe as a second navigation
        std::ptr::null_mut()
    });

    // SAFETY: the block matches the handler's signature and is kept alive below.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Swipe, &block)
    };
    let Some(monitor) = monitor else {
        return log::warn!("gestures: macOS would not hand out a monitor for swipe events");
    };
    // AppKit calls the block only while the monitor and the block are alive, and the app wants
    // them watching for as long as it runs.
    std::mem::forget(block);
    std::mem::forget(monitor);
}

/// The direction of swipe an event carries, or `None` for one that went nowhere. The sign of
/// `deltaX` is the direction.
fn direction(swipe: &NSEvent) -> Option<Direction> {
    match swipe.deltaX() {
        x if x > 0. => Some(Direction::Back),
        x if x < 0. => Some(Direction::Forward),
        _ => None,
    }
}
