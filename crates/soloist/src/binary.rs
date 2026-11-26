//! Downloads and installs Soloist builds before their 90-day expiration.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail, ensure};

use super::archive;

/// Spotify's base URL for platform archives.
const CDN: &str = "https://soloist-builds.spotifycdn.com";

/// The hosts a download may be redirected to.
const HOSTS: [&str; 2] = ["spotifycdn.com", "spotify.com"];

const REDIRECTS: usize = 5;

/// Connection and total deadlines for the archive download.
const CONNECT_PATIENCE: Duration = Duration::from_secs(15);
const DOWNLOAD_PATIENCE: Duration = Duration::from_secs(10 * 60);

/// Maximum compressed archive size.
const ARCHIVE_LIMIT: usize = 64 * 1024 * 1024;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Lifetime of a Soloist build from its build date.
const LIFE: Duration = Duration::from_secs(90 * 24 * 60 * 60);

/// Refresh builds two weeks before expiration.
const KEEP: Duration = Duration::from_secs(76 * 24 * 60 * 60);

/// Serializes installation and removal.
static STORE: Mutex<()> = Mutex::new(());

/// Removal counter used to discard downloads started before uninstall.
static REMOVALS: AtomicU64 = AtomicU64::new(0);

/// Exit code for an expired build.
pub(crate) const EXPIRED: i32 = 10;

/// The build of the installed program, read from its `--version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    /// Build timestamp when reported.
    built: Option<SystemTime>,
}

impl Build {
    /// Whole days until expiration, when the build date is known.
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

/// Keeps the installed binary outside the cache.
pub(crate) fn store() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join("soloist")
}

/// Path to the platform executable.
pub(crate) fn path() -> PathBuf {
    store().join(program())
}

/// Hides the Windows console and uses the Nix loader when configured.
pub(crate) fn command(program: &Path) -> Command {
    #[cfg(target_os = "linux")]
    if let Some(launcher) = option_env!("SONORA_SOLOIST_LAUNCHER") {
        let mut command = Command::new(launcher);
        command.arg(program);
        return command;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = Command::new(program);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    Command::new(program)
}

/// Runs the installed binary to read its version. Blocks on the process.
pub fn installed() -> Option<Build> {
    let path = path();
    if !path.is_file() {
        return None;
    }
    probe(&path)
        .inspect_err(|error| log::warn!("soloist: the installed program is unusable: {error:#}"))
        .ok()
}

/// Downloads a replacement and installs it only after it runs.
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

/// Uninstalls Soloist and invalidates downloads in flight. Blocks on disk I/O.
pub(super) fn uninstall() -> Result<()> {
    let _held = STORE.lock().unwrap_or_else(PoisonError::into_inner);
    REMOVALS.fetch_add(1, Ordering::Relaxed);
    super::clear(&store())
}

/// Tests the unpacked binary before installing it, unless removal came first.
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
    let staging = tempfile::tempdir_in(&folder).context("cannot stage the program")?;
    let part = staging.path().join(self::program());
    std::fs::write(&part, program).with_context(|| format!("cannot write {}", part.display()))?;
    runnable(&part)?;
    let build = probe(&part)?;
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

/// Parses the build line, leaving version and age unknown when it is unreadable.
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

/// Returns the first stderr line for launch diagnostics.
fn complaint(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| format!(": {line}"))
        .unwrap_or_default()
}

/// Platform executable name.
fn program() -> String {
    format!("soloist{}", std::env::consts::EXE_SUFFIX)
}
