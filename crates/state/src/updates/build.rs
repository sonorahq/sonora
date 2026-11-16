use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

use super::Channel;

/// The package's version and update channel, independent of the executable it contains.
#[derive(Deserialize)]
pub(super) struct Build {
    pub version: String,
    pub channel: Channel,
    /// The full hash of the commit a nightly was built from. Stable builds and older metadata
    /// leave it out.
    #[serde(default)]
    pub commit: Option<String>,
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
                    commit: None,
                })
                .with_compiled_commit()
        })
    }

    /// Falls back to the commit compiled in through `SONORA_COMMIT` when the metadata names none.
    fn with_compiled_commit(mut self) -> Self {
        if self.commit.is_none() {
            self.commit = option_env!("SONORA_COMMIT")
                .filter(|commit| !commit.is_empty())
                .map(str::to_owned);
        }
        self
    }
}

/// Searches beside the executable, in a macOS bundle's `Resources` and under the installation
/// prefix for package metadata. The bundle keeps it out of `Contents/MacOS` because `codesign`
/// rejects anything there that is not signed code.
fn metadata() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(directory) = exe.parent() else {
        return Vec::new();
    };
    let mut paths = vec![directory.join("sonora-build.json")];
    if let Some(prefix) = directory.parent() {
        paths.push(prefix.join("Resources/sonora-build.json"));
        paths.push(prefix.join("lib/sonora/build.json"));
    }
    paths
}
