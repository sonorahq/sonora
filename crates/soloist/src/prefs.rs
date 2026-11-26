//! Writes the audio preferences Soloist reads at startup.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use music::Quality;

const CROSSFADE: &str = "audio.crossfade_v2";
const CROSSFADE_TIME: &str = "audio.crossfade.time_v2";
const NORMALISE: &str = "audio.normalize_v2";
const GAPLESS: &str = "audio.gapless_v2";
/// Quality settings and the flag that enables a separate unmetered level.
const LEVEL: &str = "audio.play_bitrate_enumeration";
const LEVEL_UNMETERED: &str = "audio.play_bitrate_non_metered_enumeration";
const LEVEL_MIGRATED: &str = "audio.play_bitrate_non_metered_migrated";

/// Updates shared and per-account settings, preserving other lines. Blocks on disk I/O.
pub(crate) fn write(data: &Path, normalise: bool, gapless: bool, quality: Quality) -> Result<()> {
    let level = level(quality);
    let ours = [
        (CROSSFADE, "false".to_owned()),
        (NORMALISE, normalise.to_string()),
        (GAPLESS, gapless.to_string()),
        (LEVEL, level.to_string()),
        (LEVEL_UNMETERED, level.to_string()),
        (LEVEL_MIGRATED, "true".to_owned()),
    ];
    let managed = |key: &str| key == CROSSFADE_TIME || ours.iter().any(|(ours, _)| *ours == key);

    for store in stores(data) {
        // Leave unreadable preference files untouched.
        let kept = match std::fs::read_to_string(&store) {
            Ok(kept) => kept,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", store.display()));
            }
        };
        let mut lines: Vec<String> = kept
            .lines()
            .filter(|line| !managed(line.split('=').next().unwrap_or_default()))
            .map(str::to_owned)
            .collect();
        lines.extend(ours.iter().map(|(key, value)| format!("{key}={value}")));
        let folder = store.parent().context("the settings store has no folder")?;
        std::fs::create_dir_all(folder)
            .with_context(|| format!("cannot create {}", folder.display()))?;
        let mut part = tempfile::NamedTempFile::new_in(folder)
            .with_context(|| format!("cannot stage {}", store.display()))?;
        part.write_all((lines.join("\n") + "\n").as_bytes())
            .with_context(|| format!("cannot write {}", store.display()))?;
        part.persist(&store)
            .map_err(std::io::Error::from)
            .with_context(|| format!("cannot replace {}", store.display()))?;
    }
    Ok(())
}

/// Shared and per-account preference files.
fn stores(data: &Path) -> Vec<PathBuf> {
    let settings = data.join("settings");
    let mut stores = vec![settings.join("prefs")];
    if let Ok(users) = std::fs::read_dir(settings.join("Users")) {
        stores.extend(
            users
                .flatten()
                .map(|user| user.path())
                .filter(|user| user.is_dir())
                .map(|user| user.join("prefs")),
        );
    }
    stores
}

/// Maps quality to the Soloist preference values.
fn level(quality: Quality) -> u8 {
    match quality.within(super::QUALITIES) {
        Quality::HiRes | Quality::Lossless => 5,
        Quality::High => 4,
        Quality::Normal => 3,
        Quality::Low => 2,
    }
}
