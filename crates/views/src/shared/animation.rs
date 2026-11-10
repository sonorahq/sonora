use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{App, Task};

const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 120);
static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Waits for a shared 120 Hz tick so slower animations can share its redraws.
/// `frames` must be positive and selects every nth tick.
pub(crate) fn next(frames: u32, cx: &App) -> Task<()> {
    let interval = FRAME * frames;
    let elapsed = EPOCH.get_or_init(Instant::now).elapsed().as_nanos();
    let remaining = interval.as_nanos() - elapsed % interval.as_nanos();
    cx.background_executor()
        .timer(Duration::from_nanos(remaining as u64))
}
