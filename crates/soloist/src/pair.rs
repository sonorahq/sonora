//! Pairs Soloist through Spotify Connect on the local network.

use std::ffi::OsStr;
use std::mem::DropGuard;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::oneshot;

use super::daemon::{RUNNING, launch};
use super::{
    binary, clear, data, enable, home, keep_key, paired_in, pairing, recheck, same_account,
};

/// Deadline for selecting the pairing device.
const PATIENCE: Duration = Duration::from_secs(10 * 60);

/// Deadline for the current daemon to release its data folder.
const BUSY: Duration = Duration::from_secs(5);

/// Serializes blocking folder changes that can outlive a cancelled pairing.
static ADOPTING: Mutex<()> = Mutex::new(());

/// Log prefix for the TCP pairing port.
const LISTENING: &str = "Started ZeroConf service on port ";

/// Why pairing did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// Spotify rejected the API key.
    Key,
    /// Username of the account that selected the device instead.
    Account(String),
    /// Nobody picked the device in time.
    Waited,
    /// Error detail for the log and dialog.
    Failed(String),
}

/// Pairs the expected account, reports the TCP port, and replaces saved credentials on success.
pub async fn pair(
    key: String,
    account: Option<String>,
    listening: oneshot::Sender<u16>,
) -> Result<(), Trouble> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err(Trouble::Key);
    }
    let _running = tokio::time::timeout(BUSY, RUNNING.lock())
        .await
        .map_err(|_| Trouble::Failed("soloist is still playing".to_owned()))?;

    blocking(fresh).await??;
    let flags = ["--pair", "--verbose"].map(OsStr::new);
    let mut child = launch(&key, &pairing(), &flags).map_err(failed)?;
    let mut listening = Some(listening);
    let watched = tokio::time::timeout(
        PATIENCE,
        child.wait(|line| {
            if let Some(port) = port_in(line)
                && let Some(listening) = listening.take()
            {
                listening.send(port).ok();
            }
        }),
    )
    .await;
    let code = match watched {
        Ok(Ok(Some(status))) => status.code(),
        other => {
            child.stop().await;
            return Err(match other {
                Ok(Ok(_)) => Trouble::Key,
                Ok(Err(error)) => failed(error),
                Err(_) => Trouble::Waited,
            });
        }
    };
    if code == Some(binary::EXPIRED) {
        return Err(Trouble::Failed("the soloist build is too old".to_owned()));
    }
    if code != Some(0) {
        return Err(Trouble::Failed(format!(
            "soloist stopped pairing with code {code:?}"
        )));
    }

    let paired = blocking(|| paired_in(&pairing()))
        .await?
        .ok_or_else(|| Trouble::Failed("soloist stored no sign-in".to_owned()))?;
    if let Some(account) = account
        && !same_account(&paired, &account)
    {
        log::warn!("soloist: paired with another account than the one signed in");
        return Err(Trouble::Account(paired));
    }
    blocking(move || adopt(&key)).await??;
    recheck();
    log::info!("soloist: paired");
    Ok(())
}

/// Parses the TCP port from a listening log line.
fn port_in(line: &str) -> Option<u16> {
    let (_, rest) = line.split_once(LISTENING)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// Waits for adoption to finish before creating a clean, private pairing folder.
fn fresh() -> Result<(), Trouble> {
    let _held = ADOPTING.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = pairing();
    clear(&folder).map_err(failed)?;
    std::fs::create_dir_all(&folder).map_err(failed)?;
    private(&folder).map_err(failed)
}

/// Installs the new pairing and restores the old one if the move fails.
fn adopt(key: &str) -> Result<(), Trouble> {
    let _held = ADOPTING.lock().unwrap_or_else(PoisonError::into_inner);
    let (data, aside) = (data(), home().join("replaced"));
    clear(&aside).map_err(failed)?;
    let had = match std::fs::rename(&data, &aside) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(failed(error)),
    };
    let aside = DropGuard::new(aside, |aside: std::path::PathBuf| {
        if had && let Err(error) = std::fs::rename(&aside, &data) {
            log::error!(
                "soloist: cannot restore the old pairing from {}: {error}",
                aside.display()
            );
        }
    });
    std::fs::rename(pairing(), &data).map_err(failed)?;
    let aside = DropGuard::dismiss(aside);
    std::fs::remove_dir_all(&aside).ok();
    // Enable Soloist after pairing.
    keep_key(key).and_then(|()| enable(true)).map_err(failed)
}

#[cfg(unix)]
fn private(folder: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn private(_folder: &Path) -> std::io::Result<()> {
    Ok(())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Trouble> {
    tokio::task::spawn_blocking(work).await.map_err(failed)
}

fn failed(error: impl std::fmt::Display) -> Trouble {
    Trouble::Failed(format!("{error:#}"))
}
