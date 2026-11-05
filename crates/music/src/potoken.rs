//! The hand-off between the client that needs a proof-of-origin token and whatever mints it.
//!
//! YouTube will not serve a stream to a flagged address without one, and minting one takes a real
//! browser engine, which this crate has none of. So the client records the identifier it needs a
//! token for and carries on with a cold start token, the app's browser window notices the ask and
//! mints, and the stream picks the real token up when it renews its url. Nothing here knows what a
//! browser is, and the app side never names a provider.
//!
//! The identifier can be the session or a single track, depending on what YouTube asks for, so a
//! failure is held against the minter as a whole rather than against one identifier. Otherwise a
//! browser Google will not attest would be tried again for every new track.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a minted token is handed out before the client asks for another. Google's integrity
/// token lasts twelve hours, so this never outlives the minter that produced it.
const KEEP: Duration = Duration::from_secs(6 * 60 * 60);
/// How long a failed mint is left alone before another is tried, and how far that grows.
///
/// A failure is worth retrying soon: the likeliest one is a page that did not finish loading, and
/// the next attempt fixes it. A browser Google will not attest fails every time instead, and the
/// doubling is what stops that from opening a window every few seconds for the life of the run.
/// Nothing waits on the hold, but a stream the token was meant for cannot finish on a cold start
/// token, so the wait is the user's music and is kept short at the start.
const SOONEST: Duration = Duration::from_secs(5);
const SLOWEST: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
struct Shared {
    /// What the client last asked for and has no token for.
    wanted: Option<String>,
    minted: HashMap<String, Minted>,
    /// How many attempts in a row have failed. Zero once one has landed.
    failures: u32,
    /// When the last attempt failed.
    failed_at: Option<Instant>,
    /// Whether the minter has an attested page open and answers an ask in about a second.
    warm: bool,
}

struct Minted {
    token: String,
    at: Instant,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Mutex::default)
}

/// The token for `binding`, if one has been minted and is still fresh. Records the ask otherwise,
/// which is what [`wanted`] hands the app, and answers `None` straight away: a track must never
/// wait on a browser window. An ask made while a failure is being held off is not recorded.
pub fn token(binding: &str) -> Option<String> {
    let mut shared = shared().lock().ok()?;
    if let Some(token) = shared.fresh(binding) {
        return Some(token);
    }
    if !shared.held() {
        shared.wanted = Some(binding.to_string());
    }
    None
}

/// The token for `binding` if one is minted and fresh. Unlike [`token`] it never records an ask,
/// so a client waiting on an ask it already made can look as often as it likes.
pub fn minted(binding: &str) -> Option<String> {
    shared().lock().ok()?.fresh(binding)
}

/// The identifier a client is waiting on a token for, taken rather than read: one ask leads to one
/// attempt, and a failed attempt is held off by [`failed`] rather than by asking again at once.
pub fn wanted() -> Option<String> {
    shared().lock().ok()?.wanted.take()
}

/// Whether the minter can answer an ask within about a second, because its page is already
/// attested. A client may wait that long for a token then, and never otherwise.
pub fn warm() -> bool {
    shared().lock().is_ok_and(|shared| shared.warm)
}

/// Records whether the minter has an attested page open. See [`warm`].
pub fn set_warm(warm: bool) {
    if let Ok(mut shared) = shared().lock() {
        shared.warm = warm;
    }
}

/// Files a freshly minted token. It is handed out for the next six hours, and the failures
/// behind it are forgotten, so a run that recovers starts its next hold from the beginning.
pub fn give(binding: &str, token: String) {
    let Ok(mut shared) = shared().lock() else {
        return;
    };
    shared.minted.retain(|_, minted| minted.at.elapsed() < KEEP);
    shared.minted.insert(
        binding.to_string(),
        Minted {
            token,
            at: Instant::now(),
        },
    );
    shared.failures = 0;
    shared.failed_at = None;
}

/// Records that a mint failed. The next attempt waits [`SOONEST`], and each failure after that
/// doubles the wait up to [`SLOWEST`].
pub fn failed() {
    let Ok(mut shared) = shared().lock() else {
        return;
    };
    shared.failures = shared.failures.saturating_add(1);
    shared.failed_at = Some(Instant::now());
}

impl Shared {
    fn fresh(&self, binding: &str) -> Option<String> {
        self.minted
            .get(binding)
            .filter(|minted| minted.at.elapsed() < KEEP)
            .map(|minted| minted.token.clone())
    }

    /// Whether the last failure is still being held off.
    fn held(&self) -> bool {
        self.failed_at
            .is_some_and(|at| at.elapsed() < hold(self.failures))
    }
}

/// How long to leave the minter alone after `failures` attempts in a row have failed.
fn hold(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    SOONEST.saturating_mul(1u32 << doublings).min(SLOWEST)
}
