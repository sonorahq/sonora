//! The X11 window icon. Wayland resolves the icon through the app id, so this only does
//! anything on an X11 window.

use anyhow::{Context as _, Result};
use rune::raw_window_handle::RawWindowHandle;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;

/// Publishes every icon size on the window as `_NET_WM_ICON`. Rune only lends the window's
/// handle, not its display connection, so the property goes over a short-lived connection of
/// its own, which X11 allows for any window.
pub fn install(window: &rune::WindowHandle) {
    let id = window.with_raw_handle(|handle| match handle.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        _ => None,
    });
    let Ok(Some(id)) = id else {
        return;
    };
    if let Err(error) = set_icons(id) {
        log::warn!("icon: cannot set the X11 window icons: {error:#}");
    }
}

fn set_icons(window: u32) -> Result<()> {
    let (connection, _) = x11rb::connect(None).context("cannot reach the X11 display")?;
    let atom = connection
        .intern_atom(false, b"_NET_WM_ICON")?
        .reply()?
        .atom;
    let mut pixels = Vec::new();
    for &(size, png) in crate::assets::LINUX_ICONS {
        let image = image::load_from_memory(png)
            .context("cannot decode the window icon")?
            .into_rgba8();
        pixels.extend([size, size]);
        pixels.extend(image.pixels().map(|pixel| {
            let [r, g, b, a] = pixel.0;
            u32::from_be_bytes([a, r, g, b])
        }));
    }
    connection
        .change_property32(PropMode::REPLACE, window, atom, AtomEnum::CARDINAL, &pixels)?
        .check()?;
    connection.flush().context("cannot flush the window icons")
}
