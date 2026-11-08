use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    fonts();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/windows/sonora.ico"
        );
        println!("cargo:rerun-if-changed={icon}");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(icon);
        resource.set("ProductName", "Sonora");
        resource.set("FileDescription", "Sonora");
        resource.compile().expect("cannot embed the windows icon");
    }
}

fn fonts() {
    let assets = workspace().join("assets/fonts");

    let faces = embed::folder(&assets, "ttf");
    let embedded = embed::embedded(&faces, |item| format!("fonts/{}.ttf", item.name));
    let source = format!("const FONTS: &[(&str, &[u8])] = {embedded};\n");

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the output")).join("fonts.rs");
    fs::write(&out, source).expect("cannot write the font registry");
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
