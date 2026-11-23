//! Running the Soloist program: starting it, reading what it prints and stopping it.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use tokio::io::{AsyncBufReadExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::Mutex;

use super::{DEVICE, binary, cache};

/// How long the program gets to shut down by itself before it is killed.
const GRACE: Duration = Duration::from_secs(2);

/// What marks a line the program only prints with all its logs on.
const INTERNAL: &str = " [f:";

/// The file in the data folder that names the program this app started last.
const PID: &str = "sonora.pid";

/// How long a program that was just killed gets to let go of the data folder.
const RELEASE: Duration = Duration::from_secs(2);

/// How often to look whether a killed program has let go of the data folder.
const RELEASE_POLL: Duration = Duration::from_millis(100);

/// Held for as long as a program runs on the data folder. The program refuses to start while
/// another one has the folder, and an engine that is being replaced lets go of it a moment after
/// its successor asks.
pub(super) static RUNNING: Mutex<()> = Mutex::const_new(());

/// Starts the program on the data folder `data` with `flags` after the ones every run takes. The
/// key is stored in the folder first, which keeps it off the command line, where every process
/// on the computer could read it. A build before 1.4.0 cannot store one and takes it no other
/// way. The daemon keeps its output piped for redaction.
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
        stdout: child.stdout.take().map(|pipe| BufReader::new(pipe).lines()),
        stderr: child.stderr.take().map(|pipe| BufReader::new(pipe).lines()),
        child,
        key: key.to_owned(),
    })
}

/// Has the program store `key` in the data folder `data`, where its runs on that folder read it.
/// The key goes over its standard input, which no other process sees. This blocks for the few
/// milliseconds the program takes.
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
        // a program that left early says why below
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

/// Ends a program a Sonora that crashed left running on `data`. It keeps the folder, and the
/// program refuses to start beside it, so nothing would play. The caller holds `RUNNING`, so a
/// program still alive under the recorded pid is nobody's, unless a second Sonora started it on
/// the same folder.
fn reap(data: &Path, program: &Path) {
    let pid = std::fs::read_to_string(data.join(PID))
        .ok()
        .and_then(|pid| pid.trim().parse::<u32>().ok());
    if let Some(pid) = pid
        && ended(pid, program, data)
    {
        log::info!("soloist: ended a program an earlier run left behind (pid {pid})");
        // The kill only sends the signal, and the program that starts next would meet the folder
        // still held. This blocks the engine's thread, and only after a crash.
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

/// Whether a command line is this program started on exactly this data folder. The folder is
/// matched as the whole `--data-dir` value, since a sibling such as `soloist-2` or the cache
/// folder would share its prefix.
fn started_with(line: &str, program: &Path, data: &Path) -> bool {
    // Windows paths ignore case, and those of the other systems tell `/srv/A` from `/srv/a`
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

/// What process `pid` was started with, whole, or `None` when it does not run. PowerShell is
/// slow to start, so callers ask only about a pid that `alive` found.
#[cfg(windows)]
fn command_line(pid: u32) -> Option<String> {
    let output = binary::command(Path::new("powershell"))
        .args([
            "-NoProfile",
            "-Command",
            // the output is read as UTF-8, and PowerShell writes the OEM code page by default
            &format!(
                "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
                 (Get-CimInstance Win32_Process -Filter 'ProcessId={pid}').CommandLine"
            ),
        ])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Kills process `pid` when it was started with this program on this data folder, and says
/// whether it did. The command line tells a program of ours from another Soloist that took the
/// pid, and the image name keeps anything else safe.
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

/// What process `pid` was started with, whole, or `None` when it does not run. The arguments of
/// `/proc` are separated by NULs.
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

/// Kills process `pid` when it was started with this program on this data folder, and says
/// whether it did. The command line shows both even where the Nix package's loader is the process
/// the system names, and tells a program of ours from another Soloist that took the pid.
#[cfg(unix)]
fn ended(pid: u32, program: &Path, data: &Path) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    let ours = command_line(pid).is_some_and(|line| started_with(&line, program, data));
    // SAFETY: `kill` only sends a signal, and the pid was just checked to run our program.
    ours && unsafe { libc::kill(pid, libc::SIGKILL) } == 0
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

/// Owns the child and both pipes. A final refusal is read before reporting the exit.
pub(super) struct Daemon {
    child: Child,
    stdout: Option<Lines<BufReader<ChildStdout>>>,
    stderr: Option<Lines<BufReader<ChildStderr>>>,
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
    pipe: &mut Option<Lines<R>>,
) -> Option<String> {
    let Some(lines) = pipe else {
        return std::future::pending().await;
    };
    let line = lines.next_line().await.ok().flatten();
    if line.is_none() {
        *pipe = None;
    }
    line
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
