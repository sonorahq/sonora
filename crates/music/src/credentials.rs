use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// The file every provider keeps its sign-in under, inside its own cache folder.
pub(crate) const FILE: &str = "credentials.json";

/// Sonora's cache root, `$XDG_CACHE_HOME/sonora`, falling back to the temp dir.
pub(crate) fn root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
}

/// A provider's own cache folder, named by its slug.
pub(crate) fn dir(slug: &str) -> PathBuf {
    root().join(slug)
}

/// Writes a credential file readable by the owner alone, creating its folder first.
pub(crate) fn write(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))?;
    secure(path);
    Ok(())
}

/// Restricts an existing credential file to its owner. A missing file is left alone.
pub(crate) fn secure(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(error) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("credentials: cannot restrict {}: {error}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Deletes a credential file, treating one that is already gone as success.
pub(crate) fn remove(path: &Path) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("credentials: cannot remove {}: {error}", path.display());
    }
}

/// Moves every provider's sign-in from where releases before 0.31 kept it to its own
/// folder, rewriting each file so it ends up owner-only. Runs once at startup, before
/// any provider reads its credentials. Nothing else in the crate looks at the old paths.
pub fn migrate() {
    crate::spotify::auth::migrate();
    crate::youtube::migrate();
}

/// Rewrites a legacy credential file at its new path and deletes the old one. An existing
/// new file wins, so the old one is only removed.
pub(crate) fn adopt(from: &Path, to: &Path) {
    if !from.exists() {
        return;
    }
    if !to.exists() {
        let copied = std::fs::read(from)
            .with_context(|| format!("cannot read {}", from.display()))
            .and_then(|body| write(to, &body));
        if let Err(error) = copied {
            log::warn!("credentials: cannot adopt {}: {error:#}", from.display());
            return;
        }
    }
    remove(from);
}
