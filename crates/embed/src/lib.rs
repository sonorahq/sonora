use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub struct Item {
    pub name: String,
    pub file: PathBuf,
}

pub fn folder(dir: impl AsRef<Path>, kind: &str) -> Vec<Item> {
    let dir = dir.as_ref();
    rerun_if_changed(dir);

    let mut items: Vec<Item> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|file| file.extension().is_some_and(|found| found == kind))
        .filter_map(|file| {
            let name = file.file_stem()?.to_str()?.to_owned();
            Some(Item { name, file })
        })
        .collect();

    items.sort_by(|one, other| one.name.cmp(&other.name));
    items
}

pub fn tree(dir: impl AsRef<Path>, kind: &str) -> Vec<(String, Vec<Item>)> {
    let dir = dir.as_ref();
    rerun_if_changed(dir);

    let mut folders: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();

    folders.sort();
    folders
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_owned();
            Some((name, folder(&path, kind)))
        })
        .collect()
}

pub fn embedded(items: &[Item], key: impl Fn(&Item) -> String) -> String {
    let rows: String = items
        .iter()
        .map(|item| {
            let key = key(item);
            let file = include_path(&item.file);
            format!("    ({key:?}, include_bytes!({file}).as_slice()),\n")
        })
        .collect();

    format!("&[\n{rows}]")
}

/// Tells cargo to run the build script again when `path` changes. A path under the crate's manifest
/// directory is printed relative to it, so cargo's record of it stays valid when the target
/// directory is copied to another checkout of the workspace.
pub fn rerun_if_changed(path: impl AsRef<Path>) {
    let path = path.as_ref();
    match relative_to_manifest(path) {
        Some(relative) => println!("cargo:rerun-if-changed={relative}"),
        None => println!("cargo:rerun-if-changed={}", path.display()),
    }
}

/// The argument for `include_bytes!` or `include_str!` in generated code that embeds `file`. A file
/// reached through the crate's manifest directory is named through `CARGO_MANIFEST_DIR`, so the
/// generated code carries no absolute path of the machine that ran the build script and still
/// compiles after the target directory moved to another checkout.
pub fn include_path(file: impl AsRef<Path>) -> String {
    let file = file.as_ref();
    match relative_to_manifest(file) {
        Some(relative) => {
            let relative = format!("/{relative}");
            format!("concat!(env!(\"CARGO_MANIFEST_DIR\"), {relative:?})")
        }
        None => format!("{:?}", file.to_str().expect("a utf-8 path")),
    }
}

/// `path` relative to the manifest directory with `/` separators, when it starts with it.
fn relative_to_manifest(path: &Path) -> Option<String> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR")?);
    let parts: Option<Vec<&str>> = path
        .strip_prefix(&manifest)
        .ok()?
        .components()
        .map(|part| match part {
            Component::ParentDir => Some(".."),
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    Some(parts?.join("/"))
}
