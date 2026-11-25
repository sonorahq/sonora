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
            let file = staged(&item.file);
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

/// Copies `file` into the build script's `OUT_DIR` and returns the argument for `include_bytes!` or
/// `include_str!` that embeds the copy. Cargo records the files a crate embeds relative to the
/// target directory when they lie inside it, but by absolute path when they lie outside the crate,
/// so embedding the copy keeps a target directory copied to another checkout fresh. The generated
/// code also carries no path of the machine that ran the build script.
pub fn staged(file: impl AsRef<Path>) -> String {
    let file = file.as_ref();
    let Some(relative) = relative_to_manifest(file) else {
        return format!("{:?}", file.to_str().expect("a utf-8 path"));
    };
    let relative = format!("/embedded/{}", relative.trim_start_matches("../"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets the output"));
    let copy = out.join(relative.trim_start_matches('/'));
    fs::create_dir_all(copy.parent().expect("a file lives in a folder"))
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", copy.display()));
    fs::copy(file, &copy).unwrap_or_else(|error| panic!("cannot copy {}: {error}", file.display()));
    format!("concat!(env!(\"OUT_DIR\"), {relative:?})")
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
