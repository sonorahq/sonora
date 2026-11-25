use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

const BASE: &str = "lucide";
const KIND: &str = "svg";

fn main() {
    let assets = workspace().join("assets/icons");

    let mut folders = embed::tree(&assets, KIND);
    folders.sort_by_key(|(id, _)| (id != BASE, id.clone()));

    let mut icons = String::new();
    let mut packs = String::new();
    let mut names = Vec::new();
    let mut start = 0;

    for (id, gathered) in &folders {
        for item in gathered {
            let name = &item.name;
            let path = format!("icons/{id}/{name}.{KIND}");
            let file = embed::staged(&item.file);
            writeln!(
                icons,
                "    Icon {{ name: {name:?}, path: {path:?}, bytes: include_bytes!({file}) }},"
            )
            .expect("a string never fails");
            names.push(name.clone());
        }

        let end = start + gathered.len();
        writeln!(
            packs,
            "    Pack {{ id: {id:?}, start: {start}, end: {end} }},"
        )
        .expect("a string never fails");
        start = end;
    }

    names.sort();
    names.dedup();
    let names: String = names
        .iter()
        .map(|name| format!("    {name:?},\n"))
        .collect();

    let source = format!(
        "pub(crate) static ICONS: &[Icon] = &[\n{icons}];\n\n\
         pub(crate) static PACKS: &[Pack] = &[\n{packs}];\n\n\
         pub static NAMES: &[&str] = &[\n{names}];\n"
    );

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the output")).join("packs.rs");
    fs::write(&out, source).expect("cannot write the icon registry");
    println!("cargo:rerun-if-changed=build.rs");
}

/// The workspace root, reached through the crate's own manifest directory without touching the
/// filesystem, since `canonicalize` fails on a shared drive inside a Windows VM. Keeping the
/// manifest directory in the path lets cargo record every file this script reads relative to the
/// crate, so a target directory copied to another checkout stays fresh.
fn workspace() -> PathBuf {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo names the crate"));
    manifest.join("..").join("..")
}
