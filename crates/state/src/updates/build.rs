use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

use super::Channel;

/// The package's version and update channel, independent of the executable it contains.
#[derive(Deserialize)]
pub(super) struct Build {
    pub version: String,
    pub channel: Channel,
}

impl Build {
    /// Reads package metadata once and falls back to the compiled build for portable copies.
    pub fn current() -> &'static Self {
        static BUILD: OnceLock<Build> = OnceLock::new();
        BUILD.get_or_init(|| {
            metadata()
                .into_iter()
                .find_map(|path| {
                    let bytes = std::fs::read(path).ok()?;
                    let build: Self = serde_json::from_slice(&bytes).ok()?;
                    super::ordered(&build.version)?;
                    Some(build)
                })
                .unwrap_or_else(|| Self {
                    version: option_env!("SONORA_VERSION")
                        .unwrap_or(env!("CARGO_PKG_VERSION"))
                        .to_owned(),
                    channel: match option_env!("SONORA_CHANNEL") {
                        Some("nightly") => Channel::Nightly,
                        _ => Channel::Stable,
                    },
                })
        })
    }
}

/// Searches beside the executable and under its installation prefix for package metadata.
fn metadata() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(directory) = exe.parent() else {
        return Vec::new();
    };
    let mut paths = vec![directory.join("sonora-build.json")];
    if let Some(prefix) = directory.parent() {
        paths.push(prefix.join("lib/sonora/build.json"));
    }
    paths
}
