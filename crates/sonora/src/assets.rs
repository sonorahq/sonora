use std::borrow::Cow;

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};

include!(concat!(env!("OUT_DIR"), "/fonts.rs"));

/// Where About asks for the app icon, and the icon this platform shows for the app.
const APP_ICON: &str = "brand/sonora.svg";
#[cfg(target_os = "macos")]
const APP_ICON_SVG: &[u8] = include_bytes!("../../../assets/macos/sonora.svg");
#[cfg(target_os = "windows")]
const APP_ICON_SVG: &[u8] = include_bytes!("../../../assets/windows/sonora.svg");
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const APP_ICON_SVG: &[u8] = include_bytes!("../../../assets/linux/sonora.svg");

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

/// Embedded fonts and platform artwork exposed to GPUI alongside the active icon pack.
pub struct Assets;

impl Assets {
    pub fn load_fonts(&self, cx: &App) -> Result<()> {
        let embedded = FONTS
            .iter()
            .map(|(_, bytes)| Cow::Borrowed(*bytes))
            .collect();

        cx.text_system().add_fonts(embedded)
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(APP_ICON_SVG)));
        }
        if let Some(bytes) = icons::asset(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some((_, bytes)) = FONTS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        log::warn!("assets: {path} is not registered");
        Ok(None)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut listed = icons::Assets.list(path)?;
        listed.extend(
            FONTS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::new_static(name)),
        );
        Ok(listed)
    }
}

/// Matches the installed desktop file for launcher icons and window grouping on Linux.
pub fn app_id() -> &'static str {
    match cfg!(target_os = "linux") && std::path::Path::new("/.flatpak-info").exists() {
        true => "io.github.nolight132.sonora",
        false => "sonora",
    }
}
