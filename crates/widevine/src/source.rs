//! Finding a CDM, which is never Sonora's to ship or to fetch.
//!
//! Google licenses the Widevine module to browser and device vendors and publishes nothing
//! redistributable, so a release can never carry one and Sonora never goes looking for one on
//! the network either. What it does is use a copy the machine already has: every
//! Chromium-family browser bundles or component-updates one, Firefox fetches one the first time
//! a page asks for protected playback, and `SONORA_WIDEVINE_CDM` names one directly for
//! anything those patterns miss. Every copy is read where it lies, never copied.

use std::path::{Path, PathBuf};

use crate::CDM_PATH;

/// The file name the CDM has on this platform.
#[cfg(target_os = "windows")]
pub const LIBRARY: &str = "widevinecdm.dll";
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "libwidevinecdm.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const LIBRARY: &str = "libwidevinecdm.so";

/// The operating system as the `_platform_specific` folders of a component name it.
#[cfg(target_os = "windows")]
const OS: &str = "win";
#[cfg(target_os = "macos")]
const OS: &str = "mac";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const OS: &str = "linux";

/// The processor those folders name, empty on a machine Google builds no CDM for.
#[cfg(target_arch = "x86_64")]
const ARCH: &str = "x64";
#[cfg(target_arch = "aarch64")]
const ARCH: &str = "arm64";
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const ARCH: &str = "";

/// How a CDM was come by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Named by `SONORA_WIDEVINE_CDM`.
    Configured,
    /// A copy a browser or another application on this machine already had.
    Installed,
}

/// A CDM on disk and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub origin: Origin,
}

/// A place to look: a directory that exists on this platform, and the names below it, where a
/// `*` stands for every name at that level.
struct Place {
    base: PathBuf,
    under: Vec<String>,
}

/// The CDM this process settled on. Only a search that found something is remembered: with
/// nothing found the next call looks again, so a browser installed part way through a run is
/// picked up without a restart.
static FOUND: std::sync::OnceLock<Found> = std::sync::OnceLock::new();

/// The CDM this process would use, or nothing when the machine has none. Cheap after the first
/// call, which matters because every track asks.
pub fn find() -> Option<Found> {
    if let Some(found) = FOUND.get() {
        return Some(found.clone());
    }
    let found = search()?;
    Some(FOUND.get_or_init(|| found).clone())
}

/// The environment first, so a package that ships its own module can say where it is, then
/// whatever else on the machine has one.
fn search() -> Option<Found> {
    if let Some(path) = configured() {
        return Some(Found {
            path,
            origin: Origin::Configured,
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

/// The newest CDM belonging to something else on this machine, searched in the order of
/// [`places`]: the first place that has one answers.
pub fn installed() -> Option<PathBuf> {
    places()
        .into_iter()
        .find_map(|place| newest(hunt(place.base, &place.under)))
}

/// The component layout below a version folder.
fn component() -> String {
    format!("_platform_specific/{OS}_{ARCH}/{LIBRARY}")
}

/// Where another application's CDM may be, newest-first within each place and in the order the
/// places are listed. Chromium-family applications either bundle the component beside the
/// browser or let the component updater put it under their own config folder, and Firefox keeps
/// its copy in the profile it fetched it for.
#[cfg(target_os = "linux")]
fn places() -> Vec<Place> {
    let mut places = Vec::new();
    let component = component();
    for vendor in [
        "google/chrome",
        "google/chrome-beta",
        "google/chrome-unstable",
        "microsoft/msedge",
        "microsoft/msedge-beta",
        "brave.com/brave",
        "vivaldi",
    ] {
        places.push(Place {
            base: PathBuf::from("/opt"),
            under: steps(&format!("{vendor}/WidevineCdm/*/{component}")),
        });
    }
    for lib in ["/usr/lib", "/usr/lib64"] {
        for vendor in ["chromium", "chromium-browser", "opera", "vivaldi"] {
            places.push(Place {
                base: PathBuf::from(lib),
                under: steps(&format!("{vendor}/WidevineCdm/*/{component}")),
            });
        }
    }
    let Some(home) = dirs::home_dir() else {
        return places;
    };
    // Chromium-family applications that component-update their own copy, one and two folders
    // deep: Chrome and Chromium are the first, Brave is the second.
    places.push(Place {
        base: home.join(".config"),
        under: steps(&format!("*/WidevineCdm/*/{component}")),
    });
    places.push(Place {
        base: home.join(".config"),
        under: steps(&format!("*/*/WidevineCdm/*/{component}")),
    });
    places.push(Place {
        base: home.join(".var/app"),
        under: steps(&format!("*/config/*/WidevineCdm/*/{component}")),
    });
    for profiles in [
        home.join(".mozilla/firefox"),
        home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
    ] {
        places.push(Place {
            base: profiles,
            under: steps(&format!("*/gmp-widevinecdm/*/{LIBRARY}")),
        });
    }
    places
}

/// The macOS places. A Chromium-family browser keeps the component inside the versioned
/// framework of its own bundle, so one pattern finds every one of them.
#[cfg(target_os = "macos")]
fn places() -> Vec<Place> {
    let mut places = Vec::new();
    let component = component();
    let bundled = format!(
        "*.app/Contents/Frameworks/*.framework/Versions/*/Libraries/WidevineCdm/{component}"
    );
    places.push(Place {
        base: PathBuf::from("/Applications"),
        under: steps(&bundled),
    });
    let Some(home) = dirs::home_dir() else {
        return places;
    };
    places.push(Place {
        base: home.join("Applications"),
        under: steps(&bundled),
    });
    let support = home.join("Library/Application Support");
    places.push(Place {
        base: support.clone(),
        under: steps(&format!("*/WidevineCdm/*/{component}")),
    });
    places.push(Place {
        base: support.clone(),
        under: steps(&format!("*/*/WidevineCdm/*/{component}")),
    });
    places.push(Place {
        base: support.join("Firefox/Profiles"),
        under: steps(&format!("*/gmp-widevinecdm/*/{LIBRARY}")),
    });
    places
}

/// The Windows places. Edge is the one that matters: it is part of the system, so the module is
/// on every machine already. Chrome, Brave and the rest keep theirs the same way, beside the
/// versioned application, with a component-updated copy under the browser's user data.
#[cfg(target_os = "windows")]
fn places() -> Vec<Place> {
    let mut places = Vec::new();
    let component = component();
    for root in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LOCALAPPDATA",
        "APPDATA",
    ] {
        let Some(base) = std::env::var_os(root).map(PathBuf::from) else {
            continue;
        };
        places.push(Place {
            base: base.clone(),
            under: steps(&format!("*/*/Application/*/WidevineCdm/{component}")),
        });
        places.push(Place {
            base: base.clone(),
            under: steps(&format!("*/*/User Data/WidevineCdm/*/{component}")),
        });
        places.push(Place {
            base: base.clone(),
            under: steps(&format!("*/*/*/User Data/WidevineCdm/*/{component}")),
        });
        places.push(Place {
            base,
            under: steps(&format!(
                "Mozilla/Firefox/Profiles/*/gmp-widevinecdm/*/{LIBRARY}"
            )),
        });
    }
    places
}

/// Every other platform has nowhere to look, which is the same answer as having no host.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn places() -> Vec<Place> {
    Vec::new()
}

/// Splits a slash-separated tail into the names [`hunt`] walks.
fn steps(under: &str) -> Vec<String> {
    under.split('/').map(str::to_string).collect()
}

/// Every existing path under `base` that matches `under`, where a `*` is one directory name.
fn hunt(base: PathBuf, under: &[String]) -> Vec<PathBuf> {
    let Some((head, tail)) = under.split_first() else {
        return match base.is_file() {
            true => vec![base],
            false => Vec::new(),
        };
    };
    if head != "*" {
        return hunt(base.join(head), tail);
    }
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .flat_map(|entry| hunt(entry.path(), tail))
        .collect()
}

/// The path holding the highest version number, so several copies of different ages pick the
/// newest rather than whichever the filesystem listed first.
fn newest(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().max_by_key(|path| version(path))
}

/// The version a path carries, read from the deepest folder that is nothing but numbers and
/// dots. A path with no such folder sorts below every path that has one.
fn version(path: &Path) -> Vec<u64> {
    path.components()
        .rev()
        .filter_map(|part| part.as_os_str().to_str())
        .find_map(|name| {
            let parts: Option<Vec<u64>> = name.split('.').map(|part| part.parse().ok()).collect();
            parts.filter(|parts| parts.len() > 1)
        })
        .unwrap_or_default()
}
