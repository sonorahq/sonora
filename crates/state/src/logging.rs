use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const FILE: &str = "sonora.log";
const MIB: u64 = 1024 * 1024;
/// How large `sonora.log` may grow before it is rotated, in MiB, as the Log size setting offers
/// them: a doubling ladder, since a size is picked by magnitude rather than tuned.
pub const LOG_SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];
pub(crate) const DEFAULT_LOG_SIZE: u32 = 16;

/// The current limit in bytes. The logger reads it on every write, so a change from Settings
/// takes effect at once, and it starts at the default because the logger opens before
/// `settings.json` has been read.
static LIMIT: AtomicU64 = AtomicU64::new(DEFAULT_LOG_SIZE as u64 * MIB);

/// The file the running Sonora appends its log to, `sonora.log` under the platform's
/// state folder, or the cache folder where the platform has none. `None` when neither is known.
pub fn log_file() -> Option<PathBuf> {
    let root = dirs::state_dir().or_else(dirs::cache_dir)?;

    Some(root.join("sonora").join(FILE))
}

/// How many bytes `sonora.log` may hold before the logger rotates it.
pub fn log_limit() -> u64 {
    LIMIT.load(Ordering::Relaxed)
}

/// Clamps a stored size onto the ladder's range, so a hand-edited value can neither disable
/// rotation nor rotate on every line.
pub(crate) fn clamp_log_size(mib: u32) -> u32 {
    mib.clamp(LOG_SIZES[0], LOG_SIZES[LOG_SIZES.len() - 1])
}

pub(crate) fn set_log_limit(mib: u32) {
    LIMIT.store(clamp_log_size(mib) as u64 * MIB, Ordering::Relaxed);
}
