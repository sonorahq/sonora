//! macOS trackpad page swipes.
//!
//! macOS's "Swipe between pages" gesture reaches the app as an `NSEventTypeSwipe` event.
//! GPUI typically drops that event because it accepts a swipe only when the swipe carries the
//! `Ended` phase, so here a local monitor reads the event before it's passed to the window.

use std::ptr::NonNull;

use gpui::{App, NavigationDirection};
use objc2_app_kit::{NSEvent, NSEventMask};
use tokio::sync::mpsc::{self, UnboundedSender};

pub fn install(cx: &mut App) {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    if !watch(sender) {
        return;
    }

    cx.spawn(async move |cx| {
        while let Some(direction) = receiver.recv().await {
            cx.update(|cx| match direction {
                NavigationDirection::Back => router::back(cx),
                NavigationDirection::Forward => router::forward(cx),
            });
        }
    })
    .detach();
}

/// Asks AppKit for this app's swipe events and reports the direction of each one. Returns false
/// when macOS refuses a monitor, which leaves `install` nothing to feed.
fn watch(sender: UnboundedSender<NavigationDirection>) -> bool {
    let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let raw = event;
        let event = unsafe { event.as_ref() };
        // Swallow the swipe rather than pass it on, because GPUI reads any that carry the
        // `Ended` phase as a click on its navigation buttons, and the app would navigate twice.
        let Some(direction) = direction(event) else {
            return raw.as_ptr();
        };
        sender.send(direction).ok();
        std::ptr::null_mut()
    });

    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Swipe, &block)
    };
    let Some(monitor) = monitor else {
        log::warn!("gestures: macOS would not hand out a monitor for swipe events");
        return false;
    };
    // AppKit calls the block only while the monitor and the block are alive, and the app wants
    // them watching for as long as it runs.
    std::mem::forget(block);
    std::mem::forget(monitor);
    true
}

/// The direction of swipe an event carries, or `None` for one that went nowhere. The sign of
/// `deltaX` is the direction. This should also work for mice with back/forward buttons, as
/// they're reported as swipes as well.
fn direction(swipe: &NSEvent) -> Option<NavigationDirection> {
    match swipe.deltaX() {
        x if x > 0. => Some(NavigationDirection::Back),
        x if x < 0. => Some(NavigationDirection::Forward),
        _ => None,
    }
}
