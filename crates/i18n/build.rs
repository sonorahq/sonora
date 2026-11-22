use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

const KIND: &str = "ftl";

fn main() {
    let locales = workspace().join("assets/i18n");
    let folders = embed::tree(&locales, KIND);

    let mut arms = String::new();
    for (locale, items) in folders {
        let mut rows = String::new();
        for item in items {
            embed::rerun_if_changed(&item.file);
            let file = embed::staged(&item.file);
            writeln!(rows, "            ({:?}, include_str!({file})),", item.name).unwrap();
        }
        writeln!(arms, "        {locale:?} => &[\n{rows}        ],").unwrap();
    }

    let source = format!(
        "pub(crate) fn sources(id: &str) -> &'static [(&'static str, &'static str)] {{\n    match id {{\n{arms}        _ => &[],\n    }}\n}}\n"
    );

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the output")).join("locales.rs");
    fs::write(&out, source).expect("cannot write the locale registry");
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
