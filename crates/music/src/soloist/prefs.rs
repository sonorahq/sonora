//! The audio settings Soloist takes from files rather than from its command line.
//!
//! The program has no flag for loudness normalisation, gapless joins, crossfade or stream
//! quality. It reads them at startup from the preference stores Spotify's desktop app uses,
//! plain `key=value` lines in its data folder, so they are written before every start.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::Quality;

const CROSSFADE: &str = "audio.crossfade_v2";
const CROSSFADE_TIME: &str = "audio.crossfade.time_v2";
const NORMALISE: &str = "audio.normalize_v2";
const GAPLESS: &str = "audio.gapless_v2";
/// The level on a metered connection and on any other, plus the marker that makes the program
/// read the second rather than derive it from the first.
const LEVEL: &str = "audio.play_bitrate_enumeration";
const LEVEL_UNMETERED: &str = "audio.play_bitrate_non_metered_enumeration";
const LEVEL_MIGRATED: &str = "audio.play_bitrate_non_metered_migrated";

/// Writes the settings into the shared store and every account's own, whose values win over the
/// shared ones. Lines the program wrote itself are kept. Blocking.
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
        let kept = std::fs::read_to_string(&store).unwrap_or_default();
        let mut lines: Vec<String> = kept
            .lines()
            .filter(|line| !managed(line.split('=').next().unwrap_or_default()))
            .map(str::to_owned)
            .collect();
        lines.extend(ours.iter().map(|(key, value)| format!("{key}={value}")));
        let folder = store.parent().context("the settings store has no folder")?;
        std::fs::create_dir_all(folder)
            .with_context(|| format!("cannot create {}", folder.display()))?;
        let part = store.with_extension("part");
        std::fs::write(&part, lines.join("\n") + "\n")
            .and_then(|()| std::fs::rename(&part, &store))
            .with_context(|| format!("cannot write {}", store.display()))?;
    }
    Ok(())
}

/// The shared store and the one of every account the program has signed in.
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

/// The program's number for a level: 2 is about 96 kbps, 3 about 160, 4 about 320 and 5
/// lossless. It falls back to 160 kbps for any number outside 1 to 5.
fn level(quality: Quality) -> u8 {
    match quality.within(super::QUALITIES) {
        Quality::HiRes | Quality::Lossless => 5,
        Quality::High => 4,
        Quality::Normal => 3,
        Quality::Low => 2,
    }
}
