//! Pairing the program with the listener's account.
//!
//! Soloist never takes a password or opens a browser. Started with `--pair` it lists itself as
//! a Spotify Connect device on the local network, waits for one of Spotify's own apps to pick
//! it, stores the sign-in that app hands over and exits. The API key is only checked at that
//! sign-in, so a wrong key shows up after the listener has picked the device, not before.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;

use super::daemon::{RUNNING, follow, launch, stop};
use super::{binary, data, enable, home, keep_key, paired_in, pairing, recheck};

/// How long the listener gets to pick the device before pairing gives up.
const PATIENCE: Duration = Duration::from_secs(10 * 60);

/// How long a program that is still playing gets to let go of the data folder.
const BUSY: Duration = Duration::from_secs(5);

/// Held while the pairing folder is emptied or takes the place of the data folder. Both run as
/// blocking work that outlives a pairing the listener gives up, so the lock that covers the
/// pairing itself cannot keep them apart.
static ADOPTING: Mutex<()> = Mutex::new(());

/// What the program prints when Spotify turns its sign-in down, which the key decides.
const REFUSED: &str = "login failed";

/// What the program prints, with all its logs on, once it listens for the app that picks it.
/// The port follows.
const LISTENING: &str = "Started ZeroConf service on port ";

/// Why pairing did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// Spotify turned the sign-in down, which a wrong or revoked API key does.
    Key,
    /// The device was picked from an app signed into another account, the one named here.
    Account(String),
    /// Nobody picked the device in time.
    Waited,
    /// Anything else, in words for the log and the dialog's detail line.
    Failed(String),
}

/// Pairs the program with whichever account picks it, then stores `key`. The pairing is made
/// in a folder of its own and only replaces the one before it once it has gone through, so a
/// pairing that fails or is given up costs the listener nothing they had. With `account` given,
/// a pairing from any other account is thrown away and reported, since the library and the
/// sound would otherwise belong to two different people. Dropping the future stops the program.
///
/// `listening` is told the TCP port the program waits on, a new one every run. The app that
/// picks the device connects to it from the network, so a firewall has to let it in, along with
/// mDNS on UDP 5353. The program only names the port with all its logs on, which is why it runs
/// that way here.
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
    let mut lines = follow(&mut child, &key);
    let watched = tokio::time::timeout(PATIENCE, watch(&mut child, &mut lines, listening)).await;
    let code = match watched {
        Ok(Ok(code)) => code,
        Ok(Err(trouble)) => {
            stop(&mut child).await;
            return Err(trouble);
        }
        Err(_) => {
            stop(&mut child).await;
            return Err(Trouble::Waited);
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

/// Follows the program until it exits, which answers with its exit code, or until it says the
/// sign-in was refused. It keeps running after a refusal, so that line is the only sign. The
/// port it listens on goes to `listening` as soon as the program names it.
async fn watch(
    child: &mut Child,
    lines: &mut UnboundedReceiver<String>,
    listening: oneshot::Sender<u16>,
) -> Result<Option<i32>, Trouble> {
    let mut open = true;
    let mut listening = Some(listening);
    loop {
        tokio::select! {
            line = lines.recv(), if open => match line {
                Some(line) if line.contains(REFUSED) => return Err(Trouble::Key),
                Some(line) => {
                    if let Some(port) = port_in(&line)
                        && let Some(listening) = listening.take()
                    {
                        listening.send(port).ok();
                    }
                }
                None => open = false,
            },
            status = child.wait() => {
                let status = status.map_err(failed)?;
                while let Ok(line) = lines.try_recv() {
                    if line.contains(REFUSED) {
                        return Err(Trouble::Key);
                    }
                }
                return Ok(status.code());
            }
        }
    }
}

/// The port in the line that says where the program listens, or `None` for any other line.
fn port_in(line: &str) -> Option<u16> {
    let (_, rest) = line.split_once(LISTENING)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// Empties the pairing folder, so the program starts in it with no sign-in and a new device
/// identity, and leaves it readable by the owner alone. It waits for an adoption still under
/// way, which a pairing given up at its last moment leaves running.
fn fresh() -> Result<(), Trouble> {
    let _held = ADOPTING.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = pairing();
    clear(&folder)?;
    std::fs::create_dir_all(&folder).map_err(failed)?;
    private(&folder).map_err(failed)
}

/// Puts the pairing just made in the place of the one before it and stores `key`. The old
/// pairing is moved aside first and put back if the new one cannot take its place, so the
/// listener is never left with neither.
fn adopt(key: &str) -> Result<(), Trouble> {
    let _held = ADOPTING.lock().unwrap_or_else(PoisonError::into_inner);
    let (data, aside) = (data(), home().join("replaced"));
    clear(&aside)?;
    let had = match std::fs::rename(&data, &aside) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(failed(error)),
    };
    if let Err(error) = std::fs::rename(pairing(), &data) {
        if had {
            std::fs::rename(&aside, &data).ok();
        }
        return Err(failed(error));
    }
    std::fs::remove_dir_all(&aside).ok();
    // a setup the listener just went through is one they want on
    keep_key(key).and_then(|()| enable(true)).map_err(failed)
}

/// Removes a folder, which may not be there.
fn clear(folder: &Path) -> Result<(), Trouble> {
    match std::fs::remove_dir_all(folder) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed(error)),
    }
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

/// Whether the name the program stored is the signed-in account. The program writes the
/// username percent-encoded and in lower case.
fn same_account(paired: &str, account: &str) -> bool {
    let paired = percent_encoding::percent_decode_str(paired).decode_utf8_lossy();
    paired.to_lowercase() == account.to_lowercase()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Trouble> {
    tokio::task::spawn_blocking(work).await.map_err(failed)
}

fn failed(error: impl std::fmt::Display) -> Trouble {
    Trouble::Failed(format!("{error:#}"))
}
