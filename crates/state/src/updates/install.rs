use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context as _, Result};

#[cfg(target_os = "windows")]
const UNINSTALLER: &str = "unins000.exe";
#[cfg(target_os = "windows")]
const INSTALLER: &str = match cfg!(target_arch = "aarch64") {
    true => "Sonora-Setup-arm64.exe",
    false => "Sonora-Setup.exe",
};
/// Where Homebrew keeps the stable and the nightly cask, on Apple silicon and on Intel. Any of
/// them existing means Homebrew owns /Applications/Sonora.app, though not a copy installed
/// anywhere else.
#[cfg(target_os = "macos")]
const CASKROOMS: [&str; 4] = [
    "/opt/homebrew/Caskroom/sonora",
    "/opt/homebrew/Caskroom/sonora-nightly",
    "/usr/local/Caskroom/sonora",
    "/usr/local/Caskroom/sonora-nightly",
];

/// How this copy of Sonora was installed, which decides whether it may replace itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Install {
    /// The Windows installer, which upgrades the install in place.
    #[cfg(target_os = "windows")]
    Setup,
    /// An app bundle Sonora can write beside, outside Homebrew and the Nix store.
    #[cfg(target_os = "macos")]
    Bundle(std::path::PathBuf),
    /// An AppImage, replaced file for file.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    AppImage(std::path::PathBuf),
    /// A package manager or a portable copy, either of which Sonora leaves alone.
    Managed,
}

impl Install {
    /// Works out how Sonora was installed, once per run.
    pub(super) fn current() -> &'static Self {
        static CURRENT: OnceLock<Install> = OnceLock::new();
        CURRENT.get_or_init(detect)
    }

    pub(super) fn updates_itself(&self) -> bool {
        *self != Self::Managed
    }

    /// Whether a release asset is the one this install updates from.
    pub(super) fn wants(&self, asset: &str) -> bool {
        match self {
            #[cfg(target_os = "windows")]
            Self::Setup => asset == INSTALLER,
            #[cfg(target_os = "macos")]
            Self::Bundle(_) => asset.ends_with("-macos.dmg"),
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            Self::AppImage(_) => asset.ends_with(&format!("-{}.AppImage", std::env::consts::ARCH)),
            Self::Managed => false,
        }
    }

    /// Puts the downloaded asset in place of the running copy and arranges for the new one to
    /// start once this process has quit, which the caller does right after. Blocks on disk work
    /// and, on macOS, on `hdiutil`.
    pub(super) fn apply(&self, download: &Path) -> Result<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Setup => setup(download),
            #[cfg(target_os = "macos")]
            Self::Bundle(bundle) => swap_bundle(download, bundle),
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            Self::AppImage(image) => swap_image(download, image),
            Self::Managed => anyhow::bail!("a package manager updates this copy of sonora"),
        }
    }
}

#[cfg(target_os = "windows")]
fn detect() -> Install {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(UNINSTALLER)))
        .is_some_and(|uninstaller| uninstaller.exists());
    match installed {
        true => Install::Setup,
        false => Install::Managed,
    }
}

/// A bundle counts when Sonora runs from its own `.app` in a folder it can write to. Homebrew's,
/// one in the Nix store, one still on the disk image and one macOS translocated are left alone.
#[cfg(target_os = "macos")]
fn detect() -> Install {
    let bundle = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.parent()?.to_path_buf()))
        .filter(|bundle| {
            bundle
                .extension()
                .is_some_and(|extension| extension == "app")
        });
    let Some(bundle) = bundle else {
        return Install::Managed;
    };
    let brewed = bundle.parent() == Some(Path::new("/Applications"))
        && CASKROOMS
            .iter()
            .any(|caskroom| Path::new(caskroom).exists());
    let foreign = brewed
        || bundle.starts_with("/nix/store")
        || bundle.starts_with("/Volumes")
        || bundle.to_string_lossy().contains("/AppTranslocation/");
    match !foreign && bundle.parent().is_some_and(writable) {
        true => Install::Bundle(bundle),
        false => Install::Managed,
    }
}

/// The AppImage runtime names the image it started from in `APPIMAGE`.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn detect() -> Install {
    let image = std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .filter(|image| image.is_file());
    match image {
        Some(image) if image.parent().is_some_and(writable) => Install::AppImage(image),
        _ => Install::Managed,
    }
}

#[cfg(unix)]
fn writable(folder: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    let Ok(path) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a valid NUL-terminated string that outlives the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Runs the Inno Setup installer silently. It waits for Sonora to quit, upgrades it and starts
/// it again.
#[cfg(target_os = "windows")]
fn setup(installer: &Path) -> Result<()> {
    Command::new(installer)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/relaunch=1"])
        .spawn()
        .context("cannot start the installer")?;
    Ok(())
}

/// Copies Sonora.app off the disk image beside the running bundle, deletes the image, then
/// trades the two bundles by renaming, so a failure part way leaves the old one where it was.
#[cfg(target_os = "macos")]
fn swap_bundle(image: &Path, bundle: &Path) -> Result<()> {
    let mount = tempfile::tempdir().context("cannot make a mount point")?;
    run(Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(mount.path())
        .arg(image))
    .context("cannot open the disk image")?;

    let staged = bundle.with_file_name(".Sonora.app.update");
    let copied = remove(&staged).and_then(|()| {
        run(Command::new("/usr/bin/ditto")
            .arg(mount.path().join("Sonora.app"))
            .arg(&staged))
        .context("cannot copy the new version off the disk image")
    });
    if let Err(error) = run(Command::new("/usr/bin/hdiutil")
        .args(["detach", "-quiet"])
        .arg(mount.path()))
    {
        log::warn!("updates: cannot eject the disk image: {error:#}");
    }
    copied?;
    std::fs::remove_file(image).ok();

    let retired = bundle.with_file_name(".Sonora.app.old");
    remove(&retired)?;
    std::fs::rename(bundle, &retired).context("cannot move the old version aside")?;
    if let Err(error) = std::fs::rename(&staged, bundle) {
        std::fs::rename(&retired, bundle).ok();
        return Err(error).context("cannot put the new version in place");
    }
    if let Err(error) = remove(&retired) {
        log::warn!("updates: cannot delete the old version: {error:#}");
    }
    relaunch(Command::new("/usr/bin/open").arg(bundle))
}

/// Writes the new image beside the old one and renames it over. The running image survives,
/// since its file stays open under the old inode.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn swap_image(download: &Path, image: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let staged = image.with_file_name(".sonora-update.AppImage");
    std::fs::copy(download, &staged).context("cannot copy the new AppImage beside the old")?;
    std::fs::remove_file(download).ok();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .context("cannot make the new AppImage executable")?;
    std::fs::rename(&staged, image).context("cannot put the new AppImage in place")?;
    let mut start = Command::new(image);
    for variable in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
        start.env_remove(variable);
    }
    relaunch(&mut start)
}

/// Starts `command` from a shell that first waits for this process to exit, so the new copy
/// never runs beside the old one.
#[cfg(unix)]
fn relaunch(command: &mut Command) -> Result<()> {
    let mut waiter = Command::new("/bin/sh");
    waiter
        .arg("-c")
        .arg(r#"pid=$1; shift; while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done; exec "$@""#)
        .arg("sh")
        .arg(std::process::id().to_string())
        .arg(command.get_program())
        .args(command.get_args());
    for (variable, value) in command.get_envs() {
        match value {
            Some(value) => waiter.env(variable, value),
            None => waiter.env_remove(variable),
        };
    }
    waiter.spawn().context("cannot schedule the restart")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn run(command: &mut Command) -> Result<()> {
    let output = command.output().context("cannot start the command")?;
    if !output.status.success() {
        anyhow::bail!(
            "{} exited with {}: {}",
            command.get_program().to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path).or_else(|_| std::fs::remove_file(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot delete {}", path.display())),
    }
}
