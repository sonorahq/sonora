//! Where the CDM comes from.
//!
//! A copy the user pointed at comes first, then the one Sonora fetched from Google into its own
//! store, then a copy a browser on the machine already has. The store only holds a module the
//! user accepted Google's terms for, so it outranks a browser's copy, which can be one the
//! host cannot open.
//!
//! Nothing here lists a directory. Every candidate is an exact path this module builds and then
//! stats, because a process that walks folders looking for libraries is what an antivirus
//! flags, and a browser records enough for the path to be built without looking. A
//! Chromium-family browser writes the folder it settled on into its own
//! `WidevineCdm/latest-component-updated-widevine-cdm`, which is also the only way to reach a
//! browser installed where no fixed path predicts: on NixOS the bundled module sits in the Nix
//! store rather than under `/opt`. A Firefox-family browser names its profiles in
//! `profiles.ini` and the version it unpacked in that profile's `prefs.js`. The one folder
//! Sonora reads is its own store, which it wrote itself.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context as _, Result};

use crate::{CDM_PATH, SKIP_BROWSERS};

/// The file name the CDM has on this platform.
#[cfg(target_os = "windows")]
pub const LIBRARY: &str = "widevinecdm.dll";
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "libwidevinecdm.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const LIBRARY: &str = "libwidevinecdm.so";

/// The operating system as the update service and the `_platform_specific` folders name it.
#[cfg(target_os = "windows")]
pub(crate) const OS: &str = "win";
#[cfg(target_os = "macos")]
pub(crate) const OS: &str = "mac";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) const OS: &str = "linux";

/// The processor as they name it, empty on a machine Google builds no CDM for.
#[cfg(target_arch = "x86_64")]
pub(crate) const ARCH: &str = "x64";
#[cfg(target_arch = "aarch64")]
pub(crate) const ARCH: &str = "arm64";
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(crate) const ARCH: &str = "";

/// How a CDM was come by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Named by `SONORA_WIDEVINE_CDM`.
    Configured,
    /// A copy a browser on this machine already had.
    Installed,
    /// Fetched from Google's update service into Sonora's own store.
    Fetched,
}

/// A CDM on disk and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub origin: Origin,
}

/// A place to look, in the one form each browser family answers to.
enum Place {
    /// A module at exactly this path, if it is there at all. Windows never builds one, since
    /// only the pointer file knows a browser's versioned folder there.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    Exact(PathBuf),
    /// A Chromium-family browser's own `WidevineCdm` folder, whose pointer file names the
    /// folder the module is in.
    Pointed(PathBuf),
    /// A Firefox-family browser's profile root, the folder holding `profiles.ini`.
    Profiles(PathBuf),
}

/// The file a Chromium-family browser writes into its own `WidevineCdm` folder naming the
/// folder it settled on, whether that is one it downloaded or the one bundled with the browser.
const POINTER: &str = "latest-component-updated-widevine-cdm";

/// The pref a Firefox-family browser writes for the Widevine version it unpacked. The folder
/// below `gmp-widevinecdm` carries that version as its name.
const GMP_VERSION: &str = r#"user_pref("media.gmp-widevinecdm.version","#;

/// The CDM this process settled on. Only a search that found something is remembered: with
/// nothing found the next call looks again, so a module fetched part way through a run is
/// picked up without a restart. [`uninstall`] clears it.
static FOUND: Mutex<Option<Found>> = Mutex::new(None);

fn settled() -> MutexGuard<'static, Option<Found>> {
    FOUND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The CDM this process would use, or nothing when the machine has none. Cheap after the first
/// call, which matters because every track asks.
pub fn find() -> Option<Found> {
    if let Some(found) = settled().as_ref() {
        return Some(found.clone());
    }
    let found = search()?;
    Some(settled().get_or_insert(found).clone())
}

/// Remembers a module the store just gained, unless the process has settled on one already.
pub(crate) fn remember(found: Found) -> Found {
    settled().get_or_insert(found).clone()
}

/// Settles on a module the user just installed, in place of a browser's copy the process
/// settled on before. A module named by `SONORA_WIDEVINE_CDM` or an earlier one from the store
/// stays settled, as [`remember`] keeps it.
pub(crate) fn prefer(found: Found) -> Found {
    let mut settled = settled();
    match settled.as_ref() {
        Some(kept) if kept.origin != Origin::Installed => kept.clone(),
        _ => settled.insert(found).clone(),
    }
}

/// Removes every module Sonora fetched from Google, store folder and all, and forgets the one
/// the process settled on so the next search starts over. A CDM already open stays open until
/// the process ends: the file is unlinked, not unloaded.
pub fn uninstall() -> Result<()> {
    let store = store();
    match std::fs::remove_dir_all(&store) {
        Ok(()) => log::info!("widevine: removed the store at {}", store.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot remove {}", store.display()));
        }
    }
    *settled() = None;
    Ok(())
}

/// The environment, then the store, then a browser's copy.
fn search() -> Option<Found> {
    if let Some(path) = configured() {
        return Some(Found {
            path,
            origin: Origin::Configured,
        });
    }
    if let Some(path) = stored() {
        return Some(Found {
            path,
            origin: Origin::Fetched,
        });
    }
    installed().map(|path| Found {
        path,
        origin: Origin::Installed,
    })
}

/// The path `SONORA_WIDEVINE_CDM` names, if it names a file that is there.
pub fn configured() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(CDM_PATH)?);
    path.is_file().then_some(path)
}

/// The CDM a browser on this machine has, looked for in the order of [`places`]: the first
/// place holding one answers. Nothing when `SONORA_WIDEVINE_SKIP_BROWSERS` is set.
pub fn installed() -> Option<PathBuf> {
    if std::env::var_os(SKIP_BROWSERS).is_some_and(|value| !value.is_empty()) {
        log::debug!("widevine: skipping the browser search as asked");
        return None;
    }
    places().into_iter().find_map(look)
}

/// The module one place holds, or nothing when it holds none.
fn look(place: Place) -> Option<PathBuf> {
    match place {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        Place::Exact(path) => path.is_file().then_some(path),
        Place::Pointed(folder) => pointed(&folder),
        Place::Profiles(root) => profiles(&root)
            .into_iter()
            .find_map(|profile| gmp(&profile)),
    }
}

/// The module the pointer file in a browser's `WidevineCdm` folder names. The folder it gives
/// is either a version the browser downloaded or the one bundled with it, and the component
/// layout sits below both.
fn pointed(folder: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(folder.join(POINTER)).ok()?;
    let pointer: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = PathBuf::from(pointer.get("Path")?.as_str()?).join(component());
    path.is_file().then_some(path)
}

/// Every profile folder `profiles.ini` names. A relative `Path` sits below the root the file
/// is in, an absolute one is wherever the user put the profile.
fn profiles(root: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("Path="))
        .map(|path| match Path::new(path).is_absolute() {
            true => PathBuf::from(path),
            false => root.join(path),
        })
        .collect()
}

/// The module one Firefox profile unpacked, at the version its `prefs.js` names. The version
/// has to parse as one, which is also what keeps a hand-edited pref from naming a folder
/// outside the profile.
fn gmp(profile: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(profile.join("prefs.js")).ok()?;
    let release = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix(GMP_VERSION))
        .find_map(|rest| rest.split('"').nth(1))
        .filter(|release| version(release).is_some())?;
    let path = profile.join("gmp-widevinecdm").join(release).join(LIBRARY);
    path.is_file().then_some(path)
}

/// Sonora's own folder for the module, `$XDG_CACHE_HOME/sonora/widevine`, holding one
/// subfolder per version fetched.
pub fn store() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join("widevine")
}

/// The module of the newest version in the store. Sonora wrote every folder read here.
pub fn stored() -> Option<PathBuf> {
    std::fs::read_dir(store())
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(LIBRARY))
        .filter(|path| path.is_file())
        .max_by_key(|path| version_of(path))
}

/// The component layout below a version folder.
fn component() -> String {
    format!("_platform_specific/{OS}_{ARCH}/{LIBRARY}")
}

/// Where a browser's CDM may be. A Chromium-family browser bundles the component beside itself
/// and lets its component updater put a newer one under its own config folder, where the
/// pointer file names either; a Firefox keeps its copy in the profile that fetched it.
#[cfg(target_os = "linux")]
fn places() -> Vec<Place> {
    let mut places = Vec::new();
    for vendor in [
        "google/chrome",
        "google/chrome-beta",
        "google/chrome-unstable",
        "microsoft/msedge",
        "microsoft/msedge-beta",
        "brave.com/brave",
        "vivaldi",
    ] {
        places.push(bundled("/opt", vendor));
    }
    for lib in ["/usr/lib", "/usr/lib64"] {
        for vendor in ["chromium", "chromium-browser", "opera", "vivaldi"] {
            places.push(bundled(lib, vendor));
        }
    }
    let Some(home) = dirs::home_dir() else {
        return places;
    };
    let config = home.join(".config");
    for vendor in [
        "google-chrome",
        "google-chrome-beta",
        "google-chrome-unstable",
        "chromium",
        "microsoft-edge",
        "microsoft-edge-beta",
        "BraveSoftware/Brave-Browser",
        "vivaldi",
        "opera",
    ] {
        places.push(pointer(&config, vendor));
    }
    let flatpak = home.join(".var/app");
    for vendor in [
        "com.google.Chrome/config/google-chrome",
        "org.chromium.Chromium/config/chromium",
        "com.microsoft.Edge/config/microsoft-edge",
        "com.brave.Browser/config/BraveSoftware/Brave-Browser",
        "com.vivaldi.Vivaldi/config/vivaldi",
    ] {
        places.push(pointer(&flatpak, vendor));
    }
    for root in [
        home.join(".mozilla/firefox"),
        home.join(".librewolf"),
        home.join(".zen"),
        home.join(".waterfox"),
        flatpak.join("org.mozilla.firefox/.mozilla/firefox"),
        flatpak.join("io.gitlab.librewolf-community/.librewolf"),
        home.join("snap/firefox/common/.mozilla/firefox"),
    ] {
        places.push(Place::Profiles(root));
    }
    places
}

/// The macOS places. A Chromium-family browser keeps the component inside the versioned
/// framework of its own bundle, reached through the framework's `Libraries` symlink so the
/// version never has to be known; its component updater keeps a newer one under Application
/// Support.
#[cfg(target_os = "macos")]
fn places() -> Vec<Place> {
    let component = component();
    let mut places = Vec::new();
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    for (identifier, framework) in [
        ("com.google.Chrome", "Google Chrome Framework"),
        ("com.google.Chrome.beta", "Google Chrome Framework"),
        ("com.google.Chrome.canary", "Google Chrome Framework"),
        ("org.chromium.Chromium", "Chromium Framework"),
        ("com.brave.Browser", "Brave Browser Framework"),
        ("com.microsoft.edgemac", "Microsoft Edge Framework"),
        ("com.vivaldi.Vivaldi", "Vivaldi Framework"),
        ("com.operasoftware.Opera", "Opera Framework"),
        ("company.thebrowser.Browser", "ArcCore"),
    ] {
        let identifier = objc2_foundation::NSString::from_str(identifier);
        if let Some(path) = workspace
            .URLForApplicationWithBundleIdentifier(&identifier)
            .and_then(|url| url.path())
        {
            places.push(Place::Exact(PathBuf::from(path.to_string()).join(format!(
                "Contents/Frameworks/{framework}.framework/Libraries/WidevineCdm/{component}"
            ))));
        }
    }
    let bundles = [
        "Google Chrome",
        "Google Chrome Beta",
        "Google Chrome Canary",
        "Chromium",
        "Brave Browser",
        "Microsoft Edge",
        "Vivaldi",
        "Opera",
        "Arc",
    ];
    let mut roots = vec![PathBuf::from("/Applications")];
    let home = dirs::home_dir();
    if let Some(home) = &home {
        roots.push(home.join("Applications"));
    }
    for root in &roots {
        for bundle in bundles {
            places.push(Place::Exact(root.join(format!(
                "{bundle}.app/Contents/Frameworks/{bundle} Framework.framework/Libraries/WidevineCdm/{component}"
            ))));
        }
    }
    let Some(home) = home else {
        return places;
    };
    let support = home.join("Library/Application Support");
    for vendor in [
        "Google/Chrome",
        "Google/Chrome Beta",
        "Google/Chrome Canary",
        "Chromium",
        "BraveSoftware/Brave-Browser",
        "Microsoft Edge",
        "Vivaldi",
        "com.operasoftware.Opera",
        "Arc/User Data",
    ] {
        places.push(pointer(&support, vendor));
    }
    for vendor in ["Firefox", "zen", "LibreWolf", "Waterfox"] {
        places.push(Place::Profiles(support.join(vendor)));
    }
    places
}

/// The Windows places. A Chromium-family browser installs under the versioned application
/// folder, which only the pointer file under its user data names, so a browser that has never
/// run is not found.
#[cfg(target_os = "windows")]
fn places() -> Vec<Place> {
    let mut places = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for vendor in [
            "Microsoft/Edge",
            "Google/Chrome",
            "Google/Chrome Beta",
            "BraveSoftware/Brave-Browser",
            "Vivaldi",
            "Chromium",
        ] {
            places.push(pointer(&local, &format!("{vendor}/User Data")));
        }
    }
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        for vendor in ["Mozilla/Firefox", "zen", "librewolf", "Waterfox"] {
            places.push(Place::Profiles(roaming.join(vendor)));
        }
    }
    places
}

/// Every other platform has nowhere to look, which is the same answer as having no host.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn places() -> Vec<Place> {
    Vec::new()
}

/// The module a Chromium-family browser bundles beside itself, which carries no version folder.
/// Only the Linux search builds these, since a macOS bundle spells its path in full.
#[cfg(target_os = "linux")]
fn bundled(base: impl Into<PathBuf>, vendor: &str) -> Place {
    Place::Exact(
        base.into()
            .join(vendor)
            .join("WidevineCdm")
            .join(component()),
    )
}

/// The `WidevineCdm` folder a browser keeps its own record in.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn pointer(base: impl Into<PathBuf>, vendor: &str) -> Place {
    Place::Pointed(base.into().join(vendor).join("WidevineCdm"))
}

/// The version a path carries, read from the deepest folder that is nothing but numbers and
/// dots. A path with no such folder sorts below every path that has one.
fn version_of(path: &Path) -> Vec<u64> {
    path.components()
        .rev()
        .filter_map(|part| part.as_os_str().to_str())
        .find_map(version)
        .unwrap_or_default()
}

/// A folder name that is nothing but numbers and dots, as a version that sorts.
pub(crate) fn version(name: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = name.split('.').map(|part| part.parse().ok()).collect();
    parts.filter(|parts| parts.len() > 1)
}
