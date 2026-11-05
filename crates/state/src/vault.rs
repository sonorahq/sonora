//! The master key of the vault played tracks are kept in.
//!
//! The key lives in the OS keychain, so a kept track opens only for this user on this machine.
//! A desktop with no keychain service, such as a bare Wayland compositor with no Secret Service
//! running, gets a key file readable by the owner alone instead. That file sits in Sonora's data
//! folder beside the downloads, so anyone who can read that folder as the owner can open them.
//!
//! Every entry's name and contents depend on the key, so it must never change once a download is
//! sealed under it. A fresh key is made only when neither the keychain nor the key file holds
//! one and no download is sealed under an older one. A keychain that fails to answer leaves the
//! vault locked for the session rather than replacing the key.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext as _, AsyncApp, Task};
use music::vault::{MASTER, Vault};

/// The keychain entry the master key is stored under.
const KEYCHAIN_URL: &str = "sonora://kept-tracks";
const KEYCHAIN_USER: &str = "sonora";
/// How many gigabytes the cache of played tracks may take until the listener says otherwise.
pub(crate) const CACHE_GB: u32 = 2;
/// The bounds of that setting.
pub const CACHE_GB_RANGE: (u32, u32) = (1, 64);
/// What a keychain error says when the desktop has no Secret Service at all, rather than one
/// that is locked, busy or declined. Only these count as having no keychain to ask.
const NO_SERVICE: [&str; 3] = [
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.NameHasNoOwner",
    "address error",
];

/// A size in gigabytes as the bytes the vault counts in.
pub(crate) fn bytes(gb: u32) -> u64 {
    u64::from(gb) << 30
}

/// Finds the master key, or makes and stores one on a first run, and unlocks `vault` with it.
/// Until the task ends the vault keeps nothing, and a keychain that refuses leaves it locked for
/// the session, with every kept track and download out of reach.
pub(crate) fn unlock(vault: Arc<Vault>, cx: &mut App) -> Task<()> {
    cx.spawn(async move |cx| match master(&vault, cx).await {
        Ok(master) => {
            vault.unlock(&master);
            log::info!("vault: unlocked");
        }
        Err(error) => {
            vault.stay_locked();
            log::warn!("vault: stays locked, no track is kept or played from disk: {error:#}");
        }
    })
}

/// The master key from the keychain, or from the key file when the keychain has none or there
/// is no keychain to ask.
///
/// The keychain answering that it holds no key is not proof that none exists, since macOS
/// answers the same when the listener declines its prompt. An existing key file is used then and
/// moved into the keychain. Any other keychain error is taken as passing, so with no key file
/// the vault stays locked rather than sealing tracks under a key the next launch will not find.
async fn master(vault: &Arc<Vault>, cx: &mut AsyncApp) -> Result<Vec<u8>> {
    let read = cx.update(|cx| cx.read_credentials(KEYCHAIN_URL)).await;
    match read {
        Ok(Some((_, master))) if master.len() == MASTER => Ok(master),
        Ok(Some(_)) => bail!("the keychain holds a key of the wrong length"),
        Ok(None) => {
            if let Some(master) = cx.background_spawn(async { from_file() }).await? {
                into_keychain(&master, cx).await;
                return Ok(master);
            }
            fresh(vault, true, cx).await
        }
        Err(error) if no_service(&error) => {
            log::warn!("vault: there is no keychain, using a key file: {error:#}");
            match cx.background_spawn(async { from_file() }).await? {
                Some(master) => Ok(master),
                None => fresh(vault, false, cx).await,
            }
        }
        Err(error) => match cx.background_spawn(async { from_file() }).await? {
            Some(master) => {
                log::warn!("vault: the keychain did not answer, using the key file: {error:#}");
                Ok(master)
            }
            None => Err(error.context(
                "the keychain did not answer and there is no key file, so no key is made \
                 until a launch where it does",
            )),
        },
    }
}

/// Makes a new master key and stores it in the keychain when `keychain` allows, or in the key
/// file. It refuses while any download is sealed, since those were sealed under a key that
/// still exists somewhere and a new one would orphan them.
async fn fresh(vault: &Arc<Vault>, keychain: bool, cx: &mut AsyncApp) -> Result<Vec<u8>> {
    let sealed = {
        let vault = vault.clone();
        cx.background_spawn(async move { vault.holds_downloads() })
            .await
    };
    if sealed {
        bail!(
            "no key was found, but downloads are sealed under one, so no key is made until it \
             is found or the downloads are removed"
        );
    }
    let master = Vault::new_master()?;
    if keychain {
        let written = cx
            .update(|cx| cx.write_credentials(KEYCHAIN_URL, KEYCHAIN_USER, &master))
            .await;
        match written {
            Ok(()) => return Ok(master.to_vec()),
            Err(error) => {
                log::warn!("vault: the keychain will not store a key, using a file: {error:#}")
            }
        }
    }
    cx.background_spawn(async move { write_file(&master) })
        .await?;
    log::info!("vault: made a new key");
    Ok(master.to_vec())
}

/// Moves a key found in the key file into the keychain, and removes the file once the keychain
/// holds it. A keychain that will not store it leaves the file where it is.
async fn into_keychain(master: &[u8], cx: &mut AsyncApp) {
    let written = cx
        .update(|cx| cx.write_credentials(KEYCHAIN_URL, KEYCHAIN_USER, master))
        .await;
    if let Err(error) = written {
        log::warn!("vault: the keychain will not store the key, keeping the file: {error:#}");
        return;
    }
    let removed = cx
        .background_spawn(async {
            let path = key_file().context("cannot find the vault key file")?;
            std::fs::remove_file(path).context("cannot remove the vault key file")
        })
        .await;
    match removed {
        Ok(()) => log::info!("vault: moved the key from its file into the keychain"),
        Err(error) => log::warn!("vault: the key is in the keychain, but {error:#}"),
    }
}

/// Whether a keychain error means the desktop has no keychain at all, which a later launch will
/// not change, rather than one that failed this time.
fn no_service(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    NO_SERVICE.iter().any(|sign| text.contains(sign))
}

/// The master key from the key file, or `None` when there is no file.
fn from_file() -> Result<Option<Vec<u8>>> {
    let path = key_file().context("cannot find a folder for the vault key")?;
    match std::fs::read(&path) {
        Ok(master) if master.len() == MASTER => Ok(Some(master)),
        Ok(_) => bail!("the vault key file has the wrong length"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("cannot read the vault key file"),
    }
}

/// Writes a new key file readable by the owner alone. It never replaces an existing file.
fn write_file(master: &[u8]) -> Result<()> {
    let path = key_file().context("cannot find a folder for the vault key")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("cannot create the vault key folder")?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&path)
        .context("cannot create the vault key file")?;
    std::io::Write::write_all(&mut file, master).context("cannot write the vault key file")
}

/// The key file sits in the data folder rather than in the cache folder, so clearing or copying
/// the cache alone never carries the key with it. The downloads share the data folder.
fn key_file() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("sonora").join("vault.key"))
}
