use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::Channel;
use super::install::Install;

const MANIFEST: &str = "staged.json";

/// An update an earlier run downloaded and prepared in the background, which the next launch
/// puts in place before it opens a window. It is kept as `staged.json` in [`folder`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Staged {
    pub version: String,
    pub channel: Channel,
    /// What [`Install::swap`] takes. `None` once a launch tried it and the old version still
    /// started, which leaves that version to the update card.
    pub prepared: Option<PathBuf>,
    /// Set by the launch that starts the swap, so the one after can tell it did not take.
    #[serde(default)]
    pub tried: bool,
}

impl Staged {
    pub fn read() -> Option<Self> {
        let bytes = std::fs::read(folder()?.join(MANIFEST)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn write(&self) -> Result<()> {
        let folder = folder().context("there is no cache folder to stage the update in")?;
        std::fs::create_dir_all(&folder).context("cannot make the update folder")?;
        let bytes = serde_json::to_vec(self).context("cannot write the staged update")?;
        std::fs::write(folder.join(MANIFEST), bytes).context("cannot keep the staged update")
    }

    /// Whether this is the given version on the given channel, prepared and not tried yet.
    pub fn holds(&self, version: &str, channel: Channel) -> bool {
        self.version == version && self.channel == channel && self.prepared.is_some() && !self.tried
    }

    /// Marks the update as tried, then puts it in place and schedules the new version to start
    /// once this process exits. The caller quits right after a success.
    pub fn begin(mut self) -> Result<()> {
        let prepared = self
            .prepared
            .clone()
            .context("the staged update was already tried")?;
        self.tried = true;
        self.write()?;
        Install::current().swap(&prepared)
    }

    /// Drops the prepared files but keeps the manifest as a note that this version would not go
    /// in by itself, so the update card offers it instead of staging it again.
    pub fn give_up(mut self) {
        if let Some(prepared) = self.prepared.take() {
            super::install::discard(&prepared);
        }
        self.tried = true;
        if let Err(error) = self.write() {
            log::warn!("updates: cannot note the failed update: {error:#}");
        }
    }

    /// Deletes the staged update, its prepared files, whatever an abandoned preparation left
    /// beside the running copy and everything else in the update folder.
    pub fn clear() {
        if let Some(prepared) = Self::read().and_then(|staged| staged.prepared) {
            super::install::discard(&prepared);
        }
        if let Some(beside) = Install::current().beside() {
            super::install::discard(&beside);
        }
        if let Some(folder) = folder() {
            super::install::discard(&folder);
        }
    }
}

/// Where downloads and the staged update's manifest live.
pub(super) fn folder() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("sonora").join("updates"))
}

/// Puts an update an earlier run staged in place and arranges for the new version to start once
/// this process exits. Returns whether it did, in which case the caller exits at once without
/// opening a window. A launch that carries files or links leaves the update for the next one,
/// since the new version would start without them.
pub fn start_staged(carries_items: bool) -> bool {
    if !Install::current().updates_itself() {
        return false;
    }
    let Some(staged) = Staged::read() else {
        if let Some(folder) = folder().filter(|folder| folder.exists()) {
            super::install::discard(&folder);
        }
        return false;
    };
    if !super::wanted(staged.channel, &staged.version) {
        Staged::clear();
        return false;
    }
    if carries_items || staged.prepared.is_none() {
        return false;
    }
    if staged.tried {
        log::warn!(
            "updates: {} was put in place on the last launch but did not start",
            staged.version
        );
        staged.give_up();
        return false;
    }
    let version = staged.version.clone();
    match staged.clone().begin() {
        Ok(()) => {
            log::info!("updates: installing {version} before starting");
            true
        }
        Err(error) => {
            log::warn!("updates: cannot install {version}: {error:#}");
            staged.give_up();
            false
        }
    }
}
