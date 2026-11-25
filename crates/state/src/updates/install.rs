use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context as _, Result};

#[cfg(target_os = "windows")]
const UNINSTALLER: &str = "unins000.exe";
/// The uninstall entry Inno Setup writes for Sonora's `AppId`, under the user or the machine.
#[cfg(target_os = "windows")]
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{8D65C17E-79E8-46D7-9A37-42E85E73F738}_is1";
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
    Bundle(PathBuf),
    /// An AppImage, replaced file for file.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    AppImage(PathBuf),
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

    /// Readies a downloaded asset beside what it replaces, so that [`Self::swap`] only has to
    /// move it into place. Returns the path `swap` takes. Blocks on disk work and, on macOS, on
    /// `hdiutil`.
    pub(super) fn prepare(&self, download: &Path) -> Result<PathBuf> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Setup => Ok(download.to_path_buf()),
            #[cfg(target_os = "macos")]
            Self::Bundle(_) => unpack_bundle(download, &self.beside().context("no bundle")?),
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            Self::AppImage(_) => stage_image(download, &self.beside().context("no image")?),
            Self::Managed => anyhow::bail!("a package manager updates this copy of sonora"),
        }
    }

    /// Puts a prepared update in place of the running copy and arranges for the new one to
    /// start once this process has quit, which the caller does right after. On Windows the
    /// installer does both, and asks for elevation when Sonora was installed for all users.
    pub(super) fn swap(&self, prepared: &Path) -> Result<()> {
        match self {
            #[cfg(target_os = "windows")]
            Self::Setup => setup(prepared),
            #[cfg(target_os = "macos")]
            Self::Bundle(bundle) => swap_bundle(prepared, bundle),
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            Self::AppImage(image) => swap_image(prepared, image),
            Self::Managed => anyhow::bail!("a package manager updates this copy of sonora"),
        }
    }

    /// Where [`Self::prepare`] puts an update beside the running copy, which a preparation that
    /// was abandoned part way can leave behind. `None` when preparing keeps the download where it
    /// is.
    pub(super) fn beside(&self) -> Option<PathBuf> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Bundle(bundle) => Some(bundle.with_file_name(".Sonora.app.update")),
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            Self::AppImage(image) => Some(image.with_file_name(".sonora-update.AppImage")),
            _ => None,
        }
    }

    /// Prepares the downloaded asset and puts it in place at once.
    pub(super) fn apply(&self, download: &Path) -> Result<()> {
        let prepared = self.prepare(download)?;
        self.swap(&prepared)
    }
}

/// Deletes a file or a folder with everything in it. One that is already gone, or that another
/// process still holds open, is left for a later launch to retry.
pub(super) fn discard(path: &Path) {
    let removed = match path.is_dir() {
        true => std::fs::remove_dir_all(path),
        false => std::fs::remove_file(path),
    };
    if let Err(error) = removed
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("updates: cannot delete {}: {error}", path.display());
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
        .map(PathBuf::from)
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
/// it again. It is told which install to upgrade, since left to itself it picks whichever
/// earlier install it finds first. An install for all users makes the installer ask for
/// elevation.
#[cfg(target_os = "windows")]
fn setup(installer: &Path) -> Result<()> {
    let scope = match for_everyone() {
        true => "/ALLUSERS",
        false => "/CURRENTUSER",
    };
    Command::new(installer)
        .args([
            "/SILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            scope,
            "/relaunch=1",
        ])
        .spawn()
        .context("cannot start the installer")?;
    Ok(())
}

/// Whether the running copy is the one installed for all users. The installer records each
/// install's folder under the current user or under the machine, so the folder decides. A folder
/// neither record names counts as shared when it sits under Program Files or this user cannot
/// write to it.
#[cfg(target_os = "windows")]
fn for_everyone() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let Some(folder) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.to_path_buf()))
    else {
        return false;
    };
    let owns =
        |root| install_location(root).is_some_and(|location| same_folder(&location, &folder));
    if owns(HKEY_CURRENT_USER) {
        return false;
    }
    if owns(HKEY_LOCAL_MACHINE) {
        return true;
    }
    let shared = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .any(|programs| folder.starts_with(programs));
    shared || !writable(&folder)
}

/// The folder the installer recorded for Sonora under `root`, read from its uninstall entry.
#[cfg(target_os = "windows")]
fn install_location(root: windows_sys::Win32::System::Registry::HKEY) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    use windows_sys::Win32::System::Registry::{
        RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    };

    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect()
    };
    let key = wide(UNINSTALL_KEY);
    let value = wide("InstallLocation");
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: both names are NUL-terminated and `size` is the buffer's length in bytes.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let length = buffer.iter().position(|&unit| unit == 0)?;
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}

/// Compares two folders the way Windows does, ignoring case and a trailing separator.
#[cfg(target_os = "windows")]
fn same_folder(left: &Path, right: &Path) -> bool {
    let plain = |path: &Path| {
        path.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    plain(left) == plain(right)
}

/// Whether this user can create files in `folder`, found by making and deleting a probe file.
#[cfg(target_os = "windows")]
fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".sonora-write-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            std::fs::remove_file(&probe).ok();
            true
        }
        Err(_) => false,
    }
}

/// Copies Sonora.app off the disk image into `staged`, a hidden bundle beside the running one,
/// and deletes the image. Returns the hidden bundle.
#[cfg(target_os = "macos")]
fn unpack_bundle(image: &Path, staged: &Path) -> Result<PathBuf> {
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

    let copied = remove(staged).and_then(|()| {
        run(Command::new("/usr/bin/ditto")
            .arg(mount.path().join("Sonora.app"))
            .arg(staged))
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
    Ok(staged.to_path_buf())
}

/// Trades the running bundle for the unpacked one by renaming, so a failure part way leaves the
/// old one where it was, then schedules the new one to open.
#[cfg(target_os = "macos")]
fn swap_bundle(staged: &Path, bundle: &Path) -> Result<()> {
    if !staged.is_dir() {
        anyhow::bail!("the unpacked update is gone");
    }
    let retired = bundle.with_file_name(".Sonora.app.old");
    remove(&retired)?;
    std::fs::rename(bundle, &retired).context("cannot move the old version aside")?;
    if let Err(error) = std::fs::rename(staged, bundle) {
        std::fs::rename(&retired, bundle).ok();
        return Err(error).context("cannot put the new version in place");
    }
    if let Err(error) = remove(&retired) {
        log::warn!("updates: cannot delete the old version: {error:#}");
    }
    relaunch(Command::new("/usr/bin/open").arg(bundle))
}

/// Copies the new image into `staged`, a hidden, executable file beside the old one, so the
/// later rename stays on one filesystem. Returns the hidden file.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn stage_image(download: &Path, staged: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::copy(download, staged).context("cannot copy the new AppImage beside the old")?;
    std::fs::remove_file(download).ok();
    std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
        .context("cannot make the new AppImage executable")?;
    Ok(staged.to_path_buf())
}

/// Renames the staged image over the old one and schedules it to start. The running image
/// survives, since its file stays open under the old inode.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn swap_image(staged: &Path, image: &Path) -> Result<()> {
    std::fs::rename(staged, image).context("cannot put the new AppImage in place")?;
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
