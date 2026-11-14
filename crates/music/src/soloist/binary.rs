//! The Soloist program on disk: downloading it from Spotify, checking that it runs, and reading
//! which build it is.
//!
//! Spotify forbids passing the program on, so Sonora ships none and fetches it from Spotify's own
//! servers for each listener, the way their documentation tells people to. Every build stops
//! working 90 days after it was made, which is why a build is replaced well before that.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail, ensure};

use super::archive;

/// Where Spotify publishes the builds, one archive per platform under a fixed name.
const CDN: &str = "https://soloist-builds.spotifycdn.com";

/// The hosts a download may be redirected to.
const HOSTS: [&str; 2] = ["spotifycdn.com", "spotify.com"];

const REDIRECTS: usize = 5;

/// How long the download server gets to answer, and how long the whole archive may take on a
/// slow line before the download is given up.
const CONNECT_PATIENCE: Duration = Duration::from_secs(15);
const DOWNLOAD_PATIENCE: Duration = Duration::from_secs(10 * 60);

/// The most an archive may weigh. A real one is about 14 MB.
const ARCHIVE_LIMIT: usize = 64 * 1024 * 1024;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long Spotify's servers take a build for, counted from when it was made.
const LIFE: Duration = Duration::from_secs(90 * 24 * 60 * 60);

/// The age past which a build is replaced, two weeks short of its end.
const KEEP: Duration = Duration::from_secs(76 * 24 * 60 * 60);

/// Held while the program's folder is written or cleared, so an install and a removal never
/// run into each other.
static STORE: Mutex<()> = Mutex::new(());

/// How many times the program has been removed. A download that started before a removal must
/// not install after it, or removing Soloist would not be the end of it.
static REMOVALS: AtomicU64 = AtomicU64::new(0);

/// What the program exits with once its build is too old.
pub(crate) const EXPIRED: i32 = 10;

/// The build of the installed program, read from its `--version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    /// When Spotify made it, when the program says.
    built: Option<SystemTime>,
}

impl Build {
    /// How many whole days the build still works for, when it says how old it is.
    pub fn days_left(&self) -> Option<u64> {
        let end = self.built? + LIFE;
        let left = end.duration_since(SystemTime::now()).unwrap_or_default();
        Some(left.as_secs() / DAY.as_secs())
    }

    /// Whether the build is old enough to replace.
    pub fn stale(&self) -> bool {
        self.age().is_some_and(|age| age >= KEEP)
    }

    fn expired(&self) -> bool {
        self.age().is_some_and(|age| age >= LIFE)
    }

    fn age(&self) -> Option<Duration> {
        SystemTime::now().duration_since(self.built?).ok()
    }
}

/// The archive Spotify publishes for this computer, or `None` where it builds none.
pub(crate) fn archive() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("soloist_release_x86_64.tar.gz"),
        ("linux", "aarch64") => Some("soloist_release_arm64.tar.gz"),
        ("linux", "arm") => Some("soloist_release_arm32.tar.gz"),
        ("macos", "aarch64") => Some("soloist_release_osx_arm64.tar.gz"),
        ("windows", "x86_64") => Some("soloist_release_windows_x86_64.zip"),
        _ => None,
    }
}

/// Sonora's own folder for the program, under its data folder so a cache cleaner leaves it.
pub(crate) fn store() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join("soloist")
}

/// Where the installed program is, or would be.
pub(crate) fn path() -> PathBuf {
    store().join(program())
}

/// A command for `program` that opens no console window on Windows and, in the Nix package,
/// goes through the store's loader.
pub(crate) fn command(program: &Path) -> Command {
    windowless(loaded(program))
}

/// Starts `program` through the loader and library path the Nix package bakes in, since a NixOS
/// without nix-ld has nothing at the loader path the binary names. Every other build starts it
/// directly. The program then sees the loader as `/proc/self/exe`, which only costs it the crash
/// handler it never starts anyway.
#[cfg(target_os = "linux")]
fn loaded(program: &Path) -> Command {
    const LOADER: Option<&str> = option_env!("SONORA_SOLOIST_LOADER");
    const LIBRARIES: Option<&str> = option_env!("SONORA_SOLOIST_LIBRARIES");
    let Some(loader) = LOADER else {
        return Command::new(program);
    };
    let mut command = Command::new(loader);
    if let Some(libraries) = LIBRARIES {
        command.arg("--library-path").arg(libraries);
    }
    command.arg("--argv0").arg(program).arg(program);
    command
}

#[cfg(not(target_os = "linux"))]
fn loaded(program: &Path) -> Command {
    Command::new(program)
}

#[cfg(windows)]
fn windowless(mut command: Command) -> Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn windowless(command: Command) -> Command {
    command
}

/// The build that is installed, or `None` when nothing is or it does not run. Blocking: it
/// runs the program for its version.
pub fn installed() -> Option<Build> {
    let path = path();
    if !path.is_file() {
        return None;
    }
    probe(&path)
        .inspect_err(|error| log::warn!("soloist: the installed program is unusable: {error:#}"))
        .ok()
}

/// Downloads the current build from Spotify and installs it over whatever is there. The old
/// program stays in place unless the new one runs.
pub async fn fetch() -> Result<Build> {
    let name = archive().context("spotify builds no soloist for this computer")?;
    let url = format!("{CDN}/{name}");
    log::info!("soloist: fetching {name} from spotify");
    let removals = REMOVALS.load(Ordering::Relaxed);
    let bytes = download(&url).await?;
    let build = tokio::task::spawn_blocking(move || install(&bytes, name, removals))
        .await
        .context("the soloist install stopped")??;
    log::info!("soloist: {} is installed", build.version);
    Ok(build)
}

async fn download(url: &str) -> Result<Vec<u8>> {
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        let trusted = attempt.url().scheme() == "https"
            && attempt.url().host_str().is_some_and(|host| {
                HOSTS
                    .iter()
                    .any(|known| host == *known || host.ends_with(&format!(".{known}")))
            });
        match trusted && attempt.previous().len() <= REDIRECTS {
            true => attempt.follow(),
            false => attempt.error("the soloist download was sent somewhere unexpected"),
        }
    });
    let http = reqwest::Client::builder()
        .redirect(redirects)
        .connect_timeout(CONNECT_PATIENCE)
        .timeout(DOWNLOAD_PATIENCE)
        .build()
        .context("cannot build an http client")?;
    let mut response = http
        .get(url)
        .send()
        .await
        .context("cannot reach spotify's download server")?
        .error_for_status()
        .context("spotify's download server refused the request")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("cannot read the soloist archive")?
    {
        ensure!(
            bytes.len() + chunk.len() <= ARCHIVE_LIMIT,
            "the soloist archive is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Removes the program and its folder, and has any download still under way come to nothing.
/// Blocking.
pub(super) fn uninstall() -> Result<()> {
    let _held = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    REMOVALS.fetch_add(1, Ordering::Relaxed);
    super::clear(&store())
}

/// Unpacks the program beside the installed one, runs it once, and only then moves it into
/// place. `removals` is the count of removals when the download began, and an install that a
/// removal has overtaken is given up. Blocking.
fn install(bytes: &[u8], name: &str, removals: u64) -> Result<Build> {
    let _held = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    ensure!(
        REMOVALS.load(Ordering::Relaxed) == removals,
        "soloist was removed while it downloaded"
    );
    let program = archive::extract(bytes, name, &program())?;
    let folder = store();
    std::fs::create_dir_all(&folder)
        .with_context(|| format!("cannot create {}", folder.display()))?;
    let part = folder.join(format!("new-{}", self::program()));
    std::fs::write(&part, program).with_context(|| format!("cannot write {}", part.display()))?;
    let checked = runnable(&part).and_then(|()| probe(&part));
    let build = match checked {
        Ok(build) => build,
        Err(error) => {
            std::fs::remove_file(&part).ok();
            return Err(error);
        }
    };
    let path = path();
    std::fs::rename(&part, &path)
        .with_context(|| format!("cannot move the program into {}", path.display()))?;
    Ok(build)
}

#[cfg(unix)]
fn runnable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("cannot make {} runnable", path.display()))
}

#[cfg(not(unix))]
fn runnable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Runs the program for its version and fails when it does not run or its build is too old.
fn probe(path: &Path) -> Result<Build> {
    let output = command(path)
        .arg("--version")
        .output()
        .with_context(|| format!("cannot run {}", path.display()))?;
    if output.status.code() == Some(EXPIRED) {
        bail!("the soloist build is too old to run");
    }
    ensure!(
        output.status.success(),
        "soloist exited with {} when asked for its version{}",
        output.status,
        complaint(&output.stderr)
    );
    let build = read(&String::from_utf8_lossy(&output.stdout));
    ensure!(!build.expired(), "the soloist build is too old to run");
    Ok(build)
}

/// Reads a line such as `soloist 1.3.10.59 build 1794333744 (20261110) (g19412df6ea)
/// (linux/x86_64)`. Spotify documents no format, so a line it cannot read gives an empty version
/// and no age rather than an error.
fn read(output: &str) -> Build {
    let version = output
        .split_whitespace()
        .find(|word| word.contains('.') && word.starts_with(|first: char| first.is_ascii_digit()))
        .unwrap_or_default()
        .to_owned();
    let built = output
        .split_whitespace()
        .skip_while(|word| *word != "build")
        .nth(1)
        .and_then(|word| word.parse::<u64>().ok())
        .filter(|epoch| (1_500_000_000..4_100_000_000).contains(epoch))
        .map(|epoch| UNIX_EPOCH + Duration::from_secs(epoch));
    Build { version, built }
}

/// The first line the program printed to stderr, ready to follow an error, or nothing when it
/// printed none. When the loader cannot start the program, that line names the missing library.
fn complaint(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!(": {line}"))
        .unwrap_or_default()
}

/// The program's file name, `soloist` or `soloist.exe`.
fn program() -> String {
    format!("soloist{}", std::env::consts::EXE_SUFFIX)
}
