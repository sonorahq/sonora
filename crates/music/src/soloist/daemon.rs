//! Running the Soloist program: starting it, reading what it prints and stopping it.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::{DEVICE, binary, cache};

/// How long the program gets to shut down by itself before it is killed.
const GRACE: Duration = Duration::from_secs(2);

/// What marks a line the program only prints with all its logs on.
const INTERNAL: &str = " [f:";

/// Held for as long as a program runs on the data folder. The program refuses to start while
/// another one has the folder, and an engine that is being replaced lets go of it a moment after
/// its successor asks.
pub(super) static RUNNING: Mutex<()> = Mutex::const_new(());

/// Starts the program on the data folder `data` with `flags` after the ones every run takes. Its
/// output is piped, since anything it inherits would print the command line, key included, to
/// Sonora's own terminal. The key travels on the command line because the program takes it no
/// other way.
pub(super) fn launch(key: &str, data: &Path, flags: &[&OsStr]) -> Result<Child> {
    let path = binary::path();
    let mut command = Command::from(binary::command(&path));
    command
        .arg("--device-name")
        .arg(DEVICE)
        .arg("--api-key")
        .arg(key)
        .arg("--data-dir")
        .arg(data)
        .arg("--cache-dir")
        .arg(cache())
        .args(flags)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tie(&mut command);
    command
        .spawn()
        .with_context(|| format!("cannot start {}", path.display()))
}

/// Has the program told to stop when the thread that starts it ends, which is the engine's own
/// thread or one that lives as long as Sonora. Stopping it by hand needs Sonora to get that far,
/// and a Sonora that is killed, crashes or exits in a hurry would otherwise leave the program
/// playing on with nobody to steer it. Only Linux can ask for this.
#[cfg(target_os = "linux")]
fn tie(command: &mut Command) {
    // SAFETY: the closure runs in the forked child before exec and only calls `prctl`, which is
    // async-signal-safe and touches no memory the parent shares.
    unsafe {
        command.pre_exec(
            || match libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) {
                0 => Ok(()),
                _ => Err(std::io::Error::last_os_error()),
            },
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn tie(_command: &mut Command) {}

/// Resolves once no program runs on the data folder, for a caller that let go of an engine and
/// has to know its program is gone, as at quit.
pub async fn stopped() {
    drop(RUNNING.lock().await);
}

/// Everything the program prints from here on, a line at a time, with the key cut out. Each
/// line is logged as it comes. The channel closes once the program has closed both its streams.
pub(super) fn follow(child: &mut Child, key: &str) -> UnboundedReceiver<String> {
    let (lines, read) = unbounded_channel();
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(forward(stdout, key.to_owned(), lines.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(forward(stderr, key.to_owned(), lines));
    }
    read
}

async fn forward(stream: impl AsyncRead + Unpin, key: String, lines: UnboundedSender<String>) {
    let mut read = BufReader::new(stream).lines();
    while let Ok(Some(line)) = read.next_line().await {
        let line = line.replace(&key, "<key>");
        // with all its logs on the program prints its internals, the account among them
        match line.contains(INTERNAL) {
            true => log::trace!("soloist: {line}"),
            false => log::debug!("soloist: {line}"),
        }
        if lines.send(line).is_err() {
            // keep reading, or the program would block on a full pipe
            while let Ok(Some(_)) = read.next_line().await {}
            return;
        }
    }
}

/// Asks the program to shut down, which lets it close its sign-in store cleanly, and kills it
/// if it has not gone after `GRACE`.
pub(super) async fn stop(child: &mut Child) {
    interrupt(child);
    if tokio::time::timeout(GRACE, child.wait()).await.is_err() {
        child.kill().await.ok();
    }
}

#[cfg(unix)]
fn interrupt(child: &Child) {
    let Some(pid) = child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) else {
        return;
    };
    // SAFETY: `kill` only sends a signal, and the pid is a child this process still owns, so
    // it cannot have been handed to another process.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn interrupt(child: &mut Child) {
    child.start_kill().ok();
}
