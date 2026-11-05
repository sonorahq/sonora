use std::env;
use std::fs;
use std::path::PathBuf;

const KIND: &str = "json";

fn main() {
    let root = workspace();

    let themes = root.join("assets/themes");
    let mut bundled = embed::folder(&themes, KIND);
    for (family, variants) in embed::tree(&themes, KIND) {
        bundled.extend(variants.into_iter().map(|item| embed::Item {
            name: format!("{family}/{}", item.name),
            file: item.file,
        }));
    }
    let source = format!(
        "pub(super) static BUNDLED: &[(&str, &[u8])] = {};\n",
        embed::embedded(&bundled, |item| item.name.clone()),
    );

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the output")).join("themes.rs");
    fs::write(&out, source).expect("cannot write the theme registry");
    println!("cargo:rerun-if-changed=build.rs");
}

/// The workspace root, taken from the crate's own manifest directory without touching the
/// filesystem, since `canonicalize` fails on a shared drive inside a Windows VM.
fn workspace() -> PathBuf {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo names the crate"));
    manifest
        .ancestors()
        .nth(2)
        .expect("the crate lives two levels under the workspace")
        .to_path_buf()
}
