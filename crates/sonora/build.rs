use std::env;
use std::fs;
use std::path::PathBuf;

/// The artwork `artwork!` embeds, relative to the workspace's `assets` folder.
const ARTWORK: &[&str] = &[
    "linux/sonora.svg",
    "linux/icons/hicolor/16x16/apps/sonora.png",
    "linux/icons/hicolor/22x22/apps/sonora.png",
    "linux/icons/hicolor/24x24/apps/sonora.png",
    "linux/icons/hicolor/32x32/apps/sonora.png",
    "linux/icons/hicolor/48x48/apps/sonora.png",
    "linux/icons/hicolor/64x64/apps/sonora.png",
    "linux/icons/hicolor/128x128/apps/sonora.png",
    "linux/icons/hicolor/256x256/apps/sonora.png",
    "linux/icons/hicolor/512x512/apps/sonora.png",
    "macos/sonora.svg",
    "macos/sonora.icns",
    "tray/template-18.png",
    "tray/template-36.png",
    "windows/sonora.svg",
];

fn main() {
    fonts();
    artwork();

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

/// Stages the artwork into `OUT_DIR`, where `artwork!` embeds it from. Embedding the copies instead
/// of the files under `assets` keeps a target directory copied to another checkout fresh, since
/// cargo records an embedded file outside the crate by its absolute path.
fn artwork() {
    let assets = workspace().join("assets");
    for file in ARTWORK {
        let file = assets.join(file);
        embed::rerun_if_changed(&file);
        embed::staged(&file);
    }
}

/// The workspace root, reached through the crate's own manifest directory without touching the
/// filesystem, since `canonicalize` fails on a shared drive inside a Windows VM. Keeping the
/// manifest directory in the path lets cargo record every file this script reads relative to the
/// crate, so a target directory copied to another checkout stays fresh.
fn workspace() -> PathBuf {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo names the crate"));
    manifest.join("..").join("..")
}
