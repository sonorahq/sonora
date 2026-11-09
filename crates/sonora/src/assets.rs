//! What the host embeds beside the icon packs: the bundled font faces, the Linux icon family the
//! window and the tray draw, and the app id the desktop matches the window to.

include!(concat!(env!("OUT_DIR"), "/fonts.rs"));

/// Every bundled face, in the shape `AppSpec::fonts` takes. Rune ships Inter's regular, medium
/// and semibold weights itself; these add the bold and italic faces the screens use.
pub static FACES: [&[u8]; FONTS.len()] = faces();

/// The macOS app icon, which `rune::set_app_icon` puts on the Dock for launches outside an app
/// bundle too.
#[cfg(target_os = "macos")]
pub const MAC_ICON: &[u8] = include_bytes!("../../../assets/macos/sonora.icns");

/// RGBA icon sizes shared by X11 windows and status notifier pixmaps.
#[cfg(target_os = "linux")]
pub const LINUX_ICONS: &[(u32, &[u8])] = &[
    (
        16,
        include_bytes!("../../../assets/linux/icons/hicolor/16x16/apps/sonora.png"),
    ),
    (
        22,
        include_bytes!("../../../assets/linux/icons/hicolor/22x22/apps/sonora.png"),
    ),
    (
        24,
        include_bytes!("../../../assets/linux/icons/hicolor/24x24/apps/sonora.png"),
    ),
    (
        32,
        include_bytes!("../../../assets/linux/icons/hicolor/32x32/apps/sonora.png"),
    ),
    (
        48,
        include_bytes!("../../../assets/linux/icons/hicolor/48x48/apps/sonora.png"),
    ),
    (
        64,
        include_bytes!("../../../assets/linux/icons/hicolor/64x64/apps/sonora.png"),
    ),
    (
        128,
        include_bytes!("../../../assets/linux/icons/hicolor/128x128/apps/sonora.png"),
    ),
    (
        256,
        include_bytes!("../../../assets/linux/icons/hicolor/256x256/apps/sonora.png"),
    ),
    (
        512,
        include_bytes!("../../../assets/linux/icons/hicolor/512x512/apps/sonora.png"),
    ),
];

/// Matches the installed desktop file for launcher icons and window grouping on Linux.
pub fn app_id() -> &'static str {
    match cfg!(target_os = "linux") && std::path::Path::new("/.flatpak-info").exists() {
        true => "io.github.nolight132.sonora",
        false => "sonora",
    }
}

/// The bytes of every face in [`FONTS`], without their asset names.
const fn faces() -> [&'static [u8]; FONTS.len()] {
    let mut faces: [&[u8]; FONTS.len()] = [&[]; FONTS.len()];
    let mut at = 0;
    while at < FONTS.len() {
        faces[at] = FONTS[at].1;
        at += 1;
    }
    faces
}
