//! Soloist process lifecycle and log handling.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use tokio::io::{AsyncBufReadExt as _, BufReader, Split};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::Mutex;

use super::{DEVICE, binary, cache};

/// Grace period before forcing the child to exit.
const GRACE: Duration = Duration::from_secs(2);

/// Marker for verbose Soloist logs.
const INTERNAL: &str = " [f:";

/// PID file for the last Soloist process Sonora launched.
const PID: &str = "sonora.pid";

/// Deadline for a stale process to release the data folder.
const RELEASE: Duration = Duration::from_secs(2);

/// Polling interval after signalling a stale process.
const RELEASE_POLL: Duration = Duration::from_millis(100);

/// Allows only one Soloist launch or pairing to use the data folder at a time.
pub(super) static RUNNING: Mutex<()> = Mutex::const_new(());

/// Starts Soloist with a stored key and pipes its logs for redaction.
pub(super) fn launch(key: &str, data: &Path, flags: &[&OsStr]) -> Result<Daemon> {
    let path = binary::path();
    reap(data, &path);
    let mut command = Command::from(binary::command(&path));
    command.arg("--device-name").arg(DEVICE);
    if let Err(error) = entrust(&path, key, data) {
        log::warn!("soloist: {error:#}, so the key goes on the command line");
        command.arg("--api-key").arg(key);
    }
    command
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
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot start {}", path.display()))?;
    if let Some(pid) = child.id()
        && let Err(error) = std::fs::write(data.join(PID), pid.to_string())
    {
        log::warn!("soloist: cannot record the program's pid: {error}");
    }
    Ok(Daemon {
        stdout: child
            .stdout
            .take()
            .map(|pipe| BufReader::new(pipe).split(b'\n')),
        stderr: child
            .stderr
            .take()
            .map(|pipe| BufReader::new(pipe).split(b'\n')),
        child,
        key: key.to_owned(),
    })
}

/// Stores the key over stdin to keep it off the command line. Blocks on the process.
fn entrust(program: &Path, key: &str, data: &Path) -> Result<()> {
    let mut child = binary::command(program)
        .arg("setup-api-key")
        .arg("--data-dir")
        .arg(data)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start {}", program.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        // Read stderr for early startup failures.
        writeln!(stdin, "{key}").ok();
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "soloist did not store the api key: {}",
        String::from_utf8_lossy(&output.stderr)
            .replace(key, "<key>")
            .lines()
            .next()
            .unwrap_or_default()
    );
    Ok(())
}

/// Reaps a verified stale process before reusing its data folder.
fn reap(data: &Path, program: &Path) {
    let pid = std::fs::read_to_string(data.join(PID))
        .ok()
        .and_then(|pid| pid.trim().parse::<u32>().ok());
    if let Some(pid) = pid
        && ended(pid, program, data)
    {
        log::info!("soloist: ended a program an earlier run left behind (pid {pid})");
        // Wait for the signalled process to release the data folder.
        let deadline = Instant::now() + RELEASE;
        while alive(pid, program) && Instant::now() < deadline {
            std::thread::sleep(RELEASE_POLL);
        }
    }
    std::fs::remove_file(data.join(PID)).ok();
}

/// Whether process `pid` still runs the program.
#[cfg(windows)]
fn alive(pid: u32, program: &Path) -> bool {
    let Some(name) = program.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    binary::command(Path::new("tasklist"))
        .args([
            "/NH",
            "/FI",
            &format!("PID eq {pid}"),
            "/FI",
            &format!("IMAGENAME eq {name}"),
        ])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .to_lowercase()
                .contains(&name.to_lowercase())
        })
}

#[cfg(unix)]
fn alive(pid: u32, _program: &Path) -> bool {
    // SAFETY: signal 0 sends nothing, it only asks whether the process exists.
    libc::pid_t::try_from(pid).is_ok_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}

/// Matches the executable and the full `--data-dir` argument.
fn started_with(line: &str, program: &Path, data: &Path) -> bool {
    // Compare paths case-insensitively on Windows only.
    let fold = |text: &str| match cfg!(windows) {
        true => text.to_lowercase(),
        false => text.to_owned(),
    };
    let line = fold(&line.replace('"', ""));
    let flag = fold(&format!("--data-dir {}", data.to_string_lossy()));
    line.contains(&fold(&program.to_string_lossy()))
        && line.match_indices(&flag).any(|(at, _)| {
            line[at + flag.len()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        })
}

/// Reads a verified process's command line via PowerShell.
#[cfg(windows)]
fn command_line(pid: u32) -> Option<String> {
    let output = binary::command(Path::new("powershell"))
        .args([
            "-NoProfile",
            "-Command",
            // Force UTF-8 because PowerShell defaults to the OEM code page.
            &format!(
                "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
                 (Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').CommandLine"
            ),
        ])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Kills the process only if its command line and image match this installation.
#[cfg(windows)]
fn ended(pid: u32, program: &Path, data: &Path) -> bool {
    let Some(name) = program.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let ours = alive(pid, program)
        && command_line(pid).is_some_and(|line| started_with(&line, program, data));
    ours && binary::command(Path::new("taskkill"))
        .args([
            "/F",
            "/PID",
            &pid.to_string(),
            "/FI",
            &format!("IMAGENAME eq {name}"),
        ])
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Reads the Linux command line, converting NUL separators to spaces.
#[cfg(target_os = "linux")]
fn command_line(pid: libc::pid_t) -> Option<String> {
    let line = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(String::from_utf8_lossy(&line).replace('\0', " "))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn command_line(pid: libc::pid_t) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Matches the executable and data folder before killing the process, including under Nix.
#[cfg(unix)]
fn ended(pid: u32, program: &Path, data: &Path) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    let ours = command_line(pid).is_some_and(|line| started_with(&line, program, data));
    // SAFETY: `kill` only sends a signal, and the pid was just checked to run our program.
    ours && unsafe { libc::kill(pid, libc::SIGKILL) } == 0
}

/// On Linux, stop Soloist when its launching thread exits.
#[cfg(target_os = "linux")]
fn tie(command: &mut Command) {
    // SAFETY: The pre-exec closure calls only async-signal-safe `prctl`.
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

/// Waits for the daemon to release the data folder.
pub async fn stopped() {
    drop(RUNNING.lock().await);
}

/// Owns the child and both pipes. A final refusal is read before reporting the exit.
pub(super) struct Daemon {
    child: Child,
    stdout: Option<Split<BufReader<ChildStdout>>>,
    stderr: Option<Split<BufReader<ChildStderr>>>,
    key: String,
}

impl Daemon {
    /// Watches until exit. A reported login refusal returns no exit status.
    pub(super) async fn wait(
        &mut self,
        mut on_line: impl FnMut(&str),
    ) -> std::io::Result<Option<ExitStatus>> {
        while self.stdout.is_some() || self.stderr.is_some() {
            let line = tokio::select! {
                line = next_line(&mut self.stdout) => line,
                line = next_line(&mut self.stderr) => line,
            };
            if let Some(line) = line {
                let line = line.replace(&self.key, "<key>");
                if line.contains(INTERNAL) {
                    log::trace!("soloist: {line}");
                } else {
                    log::debug!("soloist: {line}");
                }
                on_line(&line);
                if line.contains("login failed") {
                    return Ok(None);
                }
            }
        }
        self.child.wait().await.map(Some)
    }

    /// Gives the child time to close its sign-in store before killing it.
    pub(super) async fn stop(&mut self) {
        #[cfg(unix)]
        interrupt(&self.child);
        #[cfg(not(unix))]
        self.child.start_kill().ok();
        let drained = tokio::time::timeout(GRACE, async {
            while self.wait(|_| {}).await?.is_none() {}
            Ok::<_, std::io::Error>(())
        })
        .await;
        if !matches!(drained, Ok(Ok(()))) {
            self.child.kill().await.ok();
        }
    }
}

/// Reads a pipe until EOF and disables its branch once it is drained.
async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(
    pipe: &mut Option<Split<R>>,
) -> Option<String> {
    let Some(lines) = pipe else {
        return std::future::pending().await;
    };
    match lines.next_segment().await {
        Ok(Some(bytes)) => {
            let bytes = bytes.strip_suffix(b"\r").unwrap_or(&bytes);
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        result => {
            if let Err(error) = result {
                log::debug!("soloist: cannot read a daemon pipe: {error}");
            }
            *pipe = None;
            None
        }
    }
}

#[cfg(unix)]
fn interrupt(child: &Child) {
    let Some(pid) = child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) else {
        return;
    };
    // SAFETY: The PID belongs to a child this process still owns.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}
