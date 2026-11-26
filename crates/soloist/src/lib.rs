//! Installs and pairs Spotify's standalone player, controlled through Connect.

mod archive;
mod binary;
mod daemon;
mod pair;
mod player;
mod prefs;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Context as _, Result};

use music::{Encoding, Offer, Quality, credentials};

pub use binary::{Build, fetch, installed};
pub use daemon::stopped;
pub use pair::{Trouble, pair};
pub use player::start;

/// API key page for Spotify Premium accounts.
pub const DASHBOARD: &str = "https://developer.spotify.com/dashboard/soloist";

/// Terms for using Soloist.
pub const TERMS: &str = "https://www.spotify.com/legal/end-user-agreement/";

/// Connect device name shown during pairing and playback.
pub const DEVICE: &str = "Sonora Soloist";

/// The levels Soloist serves, best first. Lossless is a ceiling: Spotify serves the best the
/// account and the track have below it.
pub const QUALITIES: &[Offer] = &[
    Offer {
        quality: Quality::Lossless,
        encoding: Encoding::Flac {
            depth: 24,
            rate: 44_100,
        },
        up_to: true,
    },
    Offer {
        quality: Quality::High,
        encoding: Encoding::Vorbis { kbps: 320 },
        up_to: false,
    },
    Offer {
        quality: Quality::Normal,
        encoding: Encoding::Vorbis { kbps: 160 },
        up_to: false,
    },
    Offer {
        quality: Quality::Low,
        encoding: Encoding::Vorbis { kbps: 96 },
        up_to: false,
    },
];

const UNREADY: u8 = 0;
const READY: u8 = 1;
const UNKNOWN: u8 = 2;

/// Cached setup status.
static STANDING: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Cached enablement state.
static SWITCH: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Cached audio-key denial state.
static DENIED: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Checks whether Soloist supports this platform.
pub fn supported() -> bool {
    binary::archive().is_some()
}

/// Checks for a binary, API key, and pairing, then caches the result.
pub fn set_up() -> bool {
    cached_flag(&STANDING, setup_complete)
}

/// Whether Soloist is switched on. Turning it off keeps the existing pairing.
pub fn enabled() -> bool {
    cached_flag(&SWITCH, || !switched_off().is_file())
}

/// Switches Soloist on or off without touching its setup. Blocking.
pub fn enable(on: bool) -> Result<()> {
    let marker = switched_off();
    match on {
        true => match std::fs::remove_file(&marker) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).with_context(|| format!("cannot remove {}", marker.display()));
            }
            _ => {}
        },
        false => std::fs::create_dir_all(home())
            .and_then(|()| std::fs::write(&marker, []))
            .with_context(|| format!("cannot write {}", marker.display()))?,
    }
    SWITCH.store(u8::from(on), Ordering::Relaxed);
    Ok(())
}

/// Whether the saved Soloist pairing belongs to this Spotify account.
pub fn set_up_for(username: &str) -> bool {
    set_up() && paired_account().is_some_and(|paired| same_account(&paired, username))
}

/// Whether the paired daemon can play for this Spotify account.
pub fn ready_for(username: &str) -> bool {
    enabled() && set_up_for(username)
}

/// Reports whether Spotify denied the last native audio-key request.
pub fn wanted() -> bool {
    cached_flag(&DENIED, || denial().is_file())
}

/// Remembers that Spotify denied the account an audio key. Blocking the first time.
pub fn denied() {
    if DENIED.swap(READY, Ordering::Relaxed) == READY {
        return;
    }
    let marker = denial();
    let written = std::fs::create_dir_all(home()).and_then(|()| std::fs::write(&marker, []));
    if let Err(error) = written {
        log::warn!("soloist: cannot remember the denied key: {error}");
    }
}

/// Forgets a denied key after native playback succeeds. Blocking the first time.
pub fn granted() {
    if DENIED.swap(UNREADY, Ordering::Relaxed) == UNREADY {
        return;
    }
    std::fs::remove_file(denial()).ok();
}

/// Invalidates cached setup flags after a partial removal.
pub fn reload() {
    for standing in [&STANDING, &SWITCH, &DENIED] {
        standing.store(UNKNOWN, Ordering::Relaxed);
    }
}

/// Removes the Soloist program, pairing, key, and playback cache. Blocking.
pub fn remove() -> Result<()> {
    STANDING.store(UNREADY, Ordering::Relaxed);
    DENIED.store(UNREADY, Ordering::Relaxed);
    SWITCH.store(READY, Ordering::Relaxed);
    [home(), cache()]
        .iter()
        .try_for_each(|folder| clear(folder))?;
    binary::uninstall()
}

fn clear(folder: &Path) -> Result<()> {
    match std::fs::remove_dir_all(folder) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", folder.display())),
    }
}

/// Reads a persisted setting on first use and caches it until invalidated.
fn cached_flag(flag: &AtomicU8, read: impl FnOnce() -> bool) -> bool {
    match flag.load(Ordering::Relaxed) {
        UNKNOWN => {
            let value = read();
            flag.store(u8::from(value), Ordering::Relaxed);
            value
        }
        value => value == READY,
    }
}

fn setup_complete() -> bool {
    supported() && binary::path().is_file() && key().is_some() && paired_account().is_some()
}

/// Refreshes setup status after a change to the pairing or installed program.
fn recheck() {
    STANDING.store(u8::from(setup_complete()), Ordering::Relaxed);
}

/// Soloist's account data directory.
fn home() -> PathBuf {
    credentials::dir("spotify").join("soloist")
}

/// Directory for the device ID and paired credentials.
fn data() -> PathBuf {
    home().join("data")
}

/// Reads the local device identity saved by the paired Soloist daemon.
pub fn device_id() -> Option<String> {
    let id = std::fs::read_to_string(data().join(".device_id")).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// Staging folder that replaces the active pairing only after success.
fn pairing() -> PathBuf {
    home().join("pairing")
}

/// Directory for replaceable audio cache files.
fn cache() -> PathBuf {
    credentials::root().join("soloist")
}

fn key_file() -> PathBuf {
    home().join("key")
}

/// Marker file for disabled Soloist.
fn switched_off() -> PathBuf {
    home().join("off")
}

/// Marker file for a denied native audio key.
fn denial() -> PathBuf {
    home().join("denied")
}

/// The stored API key, when there is one.
fn key() -> Option<String> {
    let key = std::fs::read_to_string(key_file()).ok()?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_owned())
}

/// Stores the API key readable by the owner alone.
fn keep_key(key: &str) -> Result<()> {
    credentials::write(&key_file(), key.as_bytes())
}

/// Reads the paired username from `settings/Users/<username>-user`.
fn paired_account() -> Option<String> {
    paired_in(&data())
}

/// Compares the stored percent-encoded username with the signed-in account.
fn same_account(paired: &str, account: &str) -> bool {
    let paired = percent_encoding::percent_decode_str(paired).decode_utf8_lossy();
    paired.to_lowercase() == account.to_lowercase()
}

/// Reads the account paired in the given data directory.
fn paired_in(data: &Path) -> Option<String> {
    const SUFFIX: &str = "-user";
    std::fs::read_dir(data.join("settings").join("Users"))
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .find_map(|entry| {
            let name = entry.file_name();
            name.to_str()?.strip_suffix(SUFFIX).map(str::to_owned)
        })
}
