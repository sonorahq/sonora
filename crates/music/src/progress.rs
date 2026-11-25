//! How far a library scan has got, and whether it should still be running.
//!
//! A scan runs on worker threads with no channel back to the app, so the count lives in atomics
//! a reader can sample whenever it repaints. The same atomics carry the other direction: a scan
//! holds a generation, and anything that bumps the generation is telling the scan to stop, which
//! is what removing a folder mid-scan does. A second scan starting cancels the first for free.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// The scan allowed to run. A guard whose id no longer matches has been cancelled.
static CURRENT: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static WALKING: AtomicBool = AtomicBool::new(true);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static FOUND: AtomicUsize = AtomicUsize::new(0);
static READ: AtomicUsize = AtomicUsize::new(0);

/// A scan in flight. While `walking`, `found` is what the walk has turned up so far and there is
/// no total to measure against; afterwards it is the total and `read` climbs towards it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub read: usize,
    pub found: usize,
    pub walking: bool,
}

impl Progress {
    /// How far along, once the walk has settled on a total.
    pub fn percent(&self) -> Option<u8> {
        match (self.walking, self.found) {
            (false, found) if found > 0 => Some(((self.read.min(found) * 100) / found) as u8),
            _ => None,
        }
    }
}

/// The scan running right now, if one is.
pub fn scanning() -> Option<Progress> {
    RUNNING.load(Ordering::Relaxed).then(|| Progress {
        read: READ.load(Ordering::Relaxed),
        found: FOUND.load(Ordering::Relaxed),
        walking: WALKING.load(Ordering::Relaxed),
    })
}

/// Stops whatever is scanning, as soon as it next looks. A scan that never started is left
/// alone, so nothing later reads as interrupted when it was not.
pub fn cancel() {
    CURRENT.fetch_add(1, Ordering::Relaxed);
    if RUNNING.swap(false, Ordering::Relaxed) {
        INTERRUPTED.store(true, Ordering::Relaxed);
    }
}

/// Whether the last scan was cut short rather than finished, which is how a caller knows not to
/// report how long it took.
pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

/// Marks a scan as running until the returned guard is dropped, which happens however the scan
/// ends, panic included.
pub fn start() -> Scan {
    let id = CURRENT.fetch_add(1, Ordering::Relaxed) + 1;
    FOUND.store(0, Ordering::Relaxed);
    READ.store(0, Ordering::Relaxed);
    WALKING.store(true, Ordering::Relaxed);
    INTERRUPTED.store(false, Ordering::Relaxed);
    RUNNING.store(true, Ordering::Relaxed);
    Scan { id }
}

pub struct Scan {
    id: u64,
}

impl Scan {
    /// Whether this scan is still the one that should be running. Every loop long enough to be
    /// worth stopping asks between items.
    pub fn live(&self) -> bool {
        CURRENT.load(Ordering::Relaxed) == self.id
    }

    /// How much the walk has turned up so far. A network share can spend a minute here, so the
    /// count moves rather than leaving the screen on a bare caption.
    pub fn walking(&self, entries: usize) {
        FOUND.store(entries, Ordering::Relaxed);
    }

    /// The walk is over and this is the total to count against.
    pub fn found(&self, entries: usize) {
        FOUND.store(entries, Ordering::Relaxed);
        WALKING.store(false, Ordering::Relaxed);
    }

    /// One more entry read, from whichever worker thread read it.
    pub fn read(&self) {
        READ.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        if self.live() {
            RUNNING.store(false, Ordering::Relaxed);
        }
    }
}
