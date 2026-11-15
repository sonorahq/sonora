use anyhow::{Context as _, Result};
use raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle, RawWindowHandle};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

/// Publishes every icon size on X11 windows. Wayland resolves the icon through the app id.
pub fn install(window: &gpui::Window) {
    if let Err(error) = set_icons(window) {
        log::warn!("icon: cannot set the X11 window icons: {error:#}");
    }
}

/// Borrows GPUI's connection for one property update and leaves ownership and events with GPUI.
fn set_icons(window: &gpui::Window) -> Result<()> {
    let RawWindowHandle::Xcb(handle) =
        raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw()
    else {
        return Ok(());
    };
    let RawDisplayHandle::Xcb(display) = window.display_handle()?.as_raw() else {
        return Ok(());
    };
    let connection = display
        .connection
        .context("cannot find the X11 connection")?;
    let connection = unsafe { XCBConnection::from_raw_xcb_connection(connection.as_ptr(), false) }
        .context("cannot borrow the X11 connection")?;
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
        .change_property32(
            PropMode::REPLACE,
            handle.window.get(),
            atom,
            AtomEnum::CARDINAL,
            &pixels,
        )?
        .check()?;
    connection.flush().context("cannot flush the window icons")
}
