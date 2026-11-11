//! Pairing the program with the listener's account.
//!
//! Soloist never takes a password or opens a browser. Started with `--pair` it lists itself as
//! a Spotify Connect device on the local network, waits for one of Spotify's own apps to pick
//! it, stores the sign-in that app hands over and exits. The API key is only checked at that
//! sign-in, so a wrong key shows up after the listener has picked the device, not before.

use std::ffi::OsStr;
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;

use super::daemon::{RUNNING, follow, launch, stop};
use super::{binary, data, enable, keep_key, paired_account, recheck};

/// How long the listener gets to pick the device before pairing gives up.
const PATIENCE: Duration = Duration::from_secs(10 * 60);

/// How long a program that is still playing gets to let go of the data folder.
const BUSY: Duration = Duration::from_secs(5);

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

/// Pairs the program with whichever account picks it, then stores `key`. With `account` given,
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

    blocking(start_over).await??;
    recheck();
    let flags = ["--pair", "--verbose"].map(OsStr::new);
    let mut child = launch(&key, &flags).map_err(failed)?;
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

    let paired = blocking(paired_account)
        .await?
        .ok_or_else(|| Trouble::Failed("soloist stored no sign-in".to_owned()))?;
    if let Some(account) = account
        && !same_account(&paired, &account)
    {
        log::warn!("soloist: paired with another account than the one signed in");
        blocking(start_over).await??;
        recheck();
        return Err(Trouble::Account(paired));
    }
    // a setup the listener just went through is one they want on
    blocking(move || keep_key(&key).and_then(|()| enable(true)).map_err(failed)).await??;
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

/// Empties the data folder, so the program starts with no sign-in and a new device identity,
/// and leaves it readable by the owner alone.
fn start_over() -> Result<(), Trouble> {
    let data = data();
    match std::fs::remove_dir_all(&data) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    std::fs::create_dir_all(&data).map_err(failed)?;
    private(&data).map_err(failed)
}

#[cfg(unix)]
fn private(folder: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn private(_folder: &std::path::Path) -> std::io::Result<()> {
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
