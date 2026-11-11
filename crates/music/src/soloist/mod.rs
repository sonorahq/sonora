//! Spotify Soloist, the headless player Spotify publishes, as a second way to play Spotify.
//!
//! librespot asks for audio keys the way Spotify's desktop app does, and Spotify denies them to
//! some accounts. Soloist is Spotify's own client for third parties, so it plays for every
//! Premium account, lossless included. It is a separate program. Sonora downloads it from
//! Spotify on the listener's word, starts it with the listener's own API key and drives it over
//! the WebSocket it opens on loopback. It plays to the system's output by itself, so the
//! equalizer, the visualizer and the output setting do not reach its sound.
//!
//! It is off until the listener sets it up, in three steps `state` asks for in order: [`fetch`]
//! the program, take an API key, and [`pair`] the program with the account from one of Spotify's
//! own apps. [`set_up`] says whether all three are done. The listener can still switch Soloist
//! off, and the Spotify provider starts a Soloist engine instead of librespot's while [`ready`]
//! says it is both set up and on.
//!
//! Like `drm`, this sits in the crate root so `state` and `views` reach it without naming a
//! provider.

mod archive;
mod binary;
mod daemon;
mod pair;
mod player;
mod prefs;
mod wire;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Context as _, Result};

use crate::{Encoding, Offer, Quality, credentials};

pub use binary::{Build, fetch, installed};
pub use daemon::stopped;
pub use pair::{Trouble, pair};
pub(crate) use player::{Engine, start};

/// Where the listener makes an API key, which only a Premium account can.
pub const DASHBOARD: &str = "https://developer.spotify.com/dashboard/soloist";

/// Spotify's terms of use, which running Soloist accepts.
pub const TERMS: &str = "https://www.spotify.com/legal/end-user-agreement/";

/// What the program is called on the account's device list. It differs from the name Sonora
/// lists itself under, so the listener can tell the two apart while pairing.
pub const DEVICE: &str = "Sonora Soloist";

/// The levels Soloist serves, best first. Lossless is a ceiling: Spotify serves the best the
/// account and the track have below it.
pub(crate) const QUALITIES: &[Offer] = &[
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

const UNKNOWN: u8 = 0;
const READY: u8 = 1;
const UNREADY: u8 = 2;

/// What [`set_up`] last found, so asking again reads no files.
static STANDING: AtomicU8 = AtomicU8::new(UNKNOWN);

/// What [`enabled`] last found: `READY` while Soloist is switched on.
static SWITCH: AtomicU8 = AtomicU8::new(UNKNOWN);

/// What [`wanted`] last found: `READY` for a denied account, `UNREADY` for any other.
static DENIED: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Whether Spotify builds Soloist for this computer.
pub fn supported() -> bool {
    binary::archive().is_some()
}

/// Whether the setup is complete: the program is installed, an API key is stored and an
/// account is paired. The first call reads the disk and later ones answer from memory.
pub fn set_up() -> bool {
    match STANDING.load(Ordering::Relaxed) {
        READY => true,
        UNREADY => false,
        _ => recheck(),
    }
}

/// Whether the listener has Soloist switched on, which it is unless they switched it off. The
/// switch keeps the setup, so switching back on needs no pairing. The first call reads the disk.
pub fn enabled() -> bool {
    match SWITCH.load(Ordering::Relaxed) {
        READY => true,
        UNREADY => false,
        _ => {
            let on = !switched_off().is_file();
            let standing = match on {
                true => READY,
                false => UNREADY,
            };
            SWITCH.store(standing, Ordering::Relaxed);
            on
        }
    }
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
    let standing = match on {
        true => READY,
        false => UNREADY,
    };
    SWITCH.store(standing, Ordering::Relaxed);
    Ok(())
}

/// Whether Soloist plays: it is set up and switched on. The Spotify provider starts a Soloist
/// engine while this holds. Cheap enough for a render once the disk has been read.
pub fn ready() -> bool {
    set_up() && enabled()
}

/// Whether Spotify denied this account an audio key the last time librespot asked for one,
/// which is when Soloist is worth offering unasked. It is remembered across runs, since a
/// denied account stays denied, and forgotten the first time librespot plays a track. The
/// first call reads the disk and later ones answer from memory.
pub fn wanted() -> bool {
    match DENIED.load(Ordering::Relaxed) {
        READY => true,
        UNREADY => false,
        _ => {
            let denied = denial().is_file();
            let standing = match denied {
                true => READY,
                false => UNREADY,
            };
            DENIED.store(standing, Ordering::Relaxed);
            denied
        }
    }
}

/// Remembers that Spotify denied the account an audio key. Blocking the first time.
pub(crate) fn denied() {
    if DENIED.swap(READY, Ordering::Relaxed) == READY {
        return;
    }
    let marker = denial();
    let written = std::fs::create_dir_all(home()).and_then(|()| std::fs::write(&marker, []));
    if let Err(error) = written {
        log::warn!("soloist: cannot remember the denied key: {error}");
    }
}

/// Forgets a denied key, now that librespot played a track. Blocking the first time.
pub(crate) fn granted() {
    if DENIED.swap(UNREADY, Ordering::Relaxed) == UNREADY {
        return;
    }
    std::fs::remove_file(denial()).ok();
}

/// Forgets what was read from the disk, so the next question reads it again. For after a
/// removal that failed halfway, when what is left is not what memory says.
pub fn reload() {
    for standing in [&STANDING, &SWITCH, &DENIED] {
        standing.store(UNKNOWN, Ordering::Relaxed);
    }
}

/// Removes what belongs to the account: the API key, the paired sign-in, the playback cache
/// and the memory of a denied key. Soloist is off from here on, and the program stays for the
/// next setup. Blocking.
pub fn forget() -> Result<()> {
    STANDING.store(UNREADY, Ordering::Relaxed);
    DENIED.store(UNREADY, Ordering::Relaxed);
    SWITCH.store(READY, Ordering::Relaxed);
    [home(), cache()]
        .iter()
        .try_for_each(|folder| clear(folder))
}

/// Removes everything [`forget`] does and the program too. Blocking.
pub fn remove() -> Result<()> {
    forget()?;
    binary::uninstall()
}

fn clear(folder: &Path) -> Result<()> {
    match std::fs::remove_dir_all(folder) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", folder.display())),
    }
}

/// Reads the disk for [`set_up`] and remembers the answer.
fn recheck() -> bool {
    let ready =
        supported() && binary::path().is_file() && key().is_some() && paired_account().is_some();
    let standing = match ready {
        true => READY,
        false => UNREADY,
    };
    STANDING.store(standing, Ordering::Relaxed);
    ready
}

/// The folder for what belongs to the account: the API key and the program's own sign-in.
fn home() -> PathBuf {
    credentials::dir("spotify").join("soloist")
}

/// The folder the program keeps its device identity and paired sign-in in.
fn data() -> PathBuf {
    home().join("data")
}

/// The folder for the audio the program has fetched, which it can fetch again.
fn cache() -> PathBuf {
    credentials::root().join("soloist")
}

fn key_file() -> PathBuf {
    home().join("key")
}

/// The empty file whose presence says the listener switched Soloist off.
fn switched_off() -> PathBuf {
    home().join("off")
}

/// The empty file whose presence says Spotify denied this account an audio key.
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

/// The account the program is paired with. Its folder under `settings/Users` is the only place
/// the program writes that name down, as `<username>-user`.
fn paired_account() -> Option<String> {
    const SUFFIX: &str = "-user";
    std::fs::read_dir(data().join("settings").join("Users"))
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .find_map(|entry| {
            let name = entry.file_name();
            name.to_str()?.strip_suffix(SUFFIX).map(str::to_owned)
        })
}
