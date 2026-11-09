//! The main window from the host's side: where it opens and how it is framed, and what the host
//! follows of it once it exists. Rune owns the window; this keeps a handle to it and, from the
//! app's hooks and `poll`, saves its placement, tells the wake lock about focus and the
//! fullscreen view, reports the system's light or dark appearance and applies the platform's
//! corner rounding.

use std::cell::{Cell, RefCell};

use router::Destination;
use rune::{Cx, El, Tx, WindowHandle, WindowOptions};
use state::{AppSettings, Area, Placement, Store};
use ui::{Backdrop, ThemeKind};
use views::Ui;

/// The smallest window, in points.
pub const LEAST_SIZE: (f32, f32) = (480., 400.);
/// The window's size on a first launch, before any placement was saved.
pub const FIRST_SIZE: (f32, f32) = (920., 640.);
/// How long, in seconds of the app clock, a poll waits before reading the window's placement
/// again. Reading it is a round trip to the X server on X11, so frames do not each pay for it;
/// focus changes and closing read it at once.
const PLACEMENT_EVERY: f64 = 1.;

/// Where AppKit's traffic-light buttons are centred from, and how they move with the settings.
#[cfg(target_os = "macos")]
mod lights {
    /// The height of AppKit's traffic-light buttons, which the bar centres them by.
    pub const SIZE: f32 = 14.;
    /// The extra room round corners leave before the traffic lights, since their wider curve
    /// otherwise crowds the close button.
    pub const ROUND_PAD: f32 = 4.;
    /// The bar's bottom border, which sits inside its height and so out of the centring.
    pub const BORDER: f32 = 1.;
    /// Where hidden traffic lights go: past the window's left edge, where nothing draws or
    /// hits them.
    pub const HIDDEN: f32 = -200.;
}

thread_local! {
    /// What the host keeps of the window. Every hook and poll runs on the UI thread.
    static HOST: RefCell<Host> = RefCell::new(Host::default());
    /// The appearance the view last saw the system report, when it differs from the one the
    /// theme was resolved with; the next poll applies it.
    static REPORTED_DARK: Cell<Option<bool>> = const { Cell::new(None) };
}

#[derive(Default)]
struct Host {
    /// The native window, from its first creation on.
    window: Option<WindowHandle>,
    /// The last placement saved while the window was neither maximized nor full screen, which
    /// a maximized window keeps as the size it restores to.
    normal: Option<Placement>,
    /// When a poll last read the placement.
    checked: Option<f64>,
    /// Whether the window was full screen at that read.
    full_screen: bool,
    /// Whether the fullscreen view showed at the last poll, as the wake lock last heard it.
    fullscreen_view: Option<bool>,
    /// The corner rounding last handed to the platform, and whether full screen forced it off.
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    rounded: Option<ui::Rounding>,
}

/// What the window opens with: where and how large, framed as the settings say. A saved
/// placement is restored onto the display it was on while that display is still connected;
/// otherwise the window opens centred on the primary display.
pub fn options(settings: &AppSettings, displays: &[rune::Display]) -> (WindowOptions, bool) {
    let areas = displays.iter().enumerate().map(|(at, display)| {
        let scale = display.scale as f32;
        let area = Area {
            x: display.position.0 as f32 / scale,
            y: display.position.1 as f32 / scale,
            width: display.size.0 as f32 / scale,
            height: display.size.1 as f32 / scale,
        };
        (at, area)
    });
    let restored = settings.window_placement(LEAST_SIZE.0, LEAST_SIZE.1, areas);
    let (position, size, maximized) = match restored {
        Some((placement, at)) => {
            let scale = displays[at].scale as f32;
            HOST.with_borrow_mut(|host| host.normal = Some(placement));
            (
                Some((
                    (placement.x * scale).round() as i32,
                    (placement.y * scale).round() as i32,
                )),
                Some((
                    (placement.width * scale).round() as u32,
                    (placement.height * scale).round() as u32,
                )),
                placement.maximized,
            )
        }
        None => (centred(displays), None, false),
    };

    let look = settings.look();
    let backdrop = ui::backdrop(look.blur_window, look.transparent);
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let borderless = !settings.server_side_decorations();
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let borderless = false;
    let options = WindowOptions {
        transparent: backdrop != Backdrop::Opaque,
        borderless,
        position,
        physical_size: size,
        min_size: Some(LEAST_SIZE),
        ..WindowOptions::default()
    };
    (options, maximized)
}

/// The titlebar Sonora draws itself on macOS and Windows, with the platform's own controls over
/// it. Linux keeps the system frame, or none at all (`server_side_decorations` off), and draws
/// its controls in the title bar.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
pub fn titlebar(settings: &AppSettings) -> rune::Titlebar {
    let height = ui::Metrics::new(settings.font_size()).title_bar;
    let titlebar = rune::Titlebar::new(height);
    #[cfg(target_os = "macos")]
    let titlebar = {
        let (x, y) = traffic_lights(settings, height);
        titlebar.traffic_lights(x, y)
    };
    titlebar
}

/// The root view: the app's view, with the titlebar kept in step with the theme's size and the
/// system's appearance passed on to the next poll.
pub fn view(ui: &Ui, cx: &mut Cx<Ui>) -> El {
    let dark = cx.system_dark();
    if dark != ui.system_dark && REPORTED_DARK.get() != Some(dark) {
        REPORTED_DARK.set(Some(dark));
        crate::handle().wake();
    }
    #[cfg(target_os = "macos")]
    {
        let height = ui.theme.metrics.title_bar;
        let (x, y) = traffic_lights(&ui.store.settings, height);
        cx.set_titlebar(rune::Titlebar::new(height).traffic_lights(x, y));
    }
    #[cfg(windows)]
    {
        cx.titlebar_height(ui.theme.metrics.title_bar);
        cx.caption_color(ui.theme.foreground.color());
    }
    views::view(ui, cx)
}

/// Runs after every native creation of the window: keeps its handle, gives it its icon and
/// platform styling, and hands it to the system media controls.
pub fn created(window: &WindowHandle) {
    HOST.with_borrow_mut(|host| {
        host.window = Some(window.clone());
        host.rounded = None;
    });
    #[cfg(target_os = "linux")]
    crate::x11_icon::install(window);

    // the handle as an address, so it can cross into an update
    #[cfg(windows)]
    let hwnd = hwnd(window);
    #[cfg(not(windows))]
    let hwnd: Option<usize> = None;
    #[cfg(windows)]
    if let Some(hwnd) = hwnd {
        crate::app_icon::install(hwnd as *mut std::ffi::c_void);
        crate::thumbbar::install(hwnd as *mut std::ffi::c_void);
        state::install_rounded_window_hook(move |rounding| set_corner_preference(hwnd, rounding));
    }
    #[cfg(target_os = "macos")]
    {
        let window = window.clone();
        state::install_rounded_window_hook(move |rounding| set_toolbar_style(&window, rounding));
    }

    crate::handle().update(move |ui: &mut Ui, _| {
        ui.store
            .attach_remote(hwnd.map(|hwnd| hwnd as *mut std::ffi::c_void));
    });
}

/// Brings the window back from the tray or the Dock, creating it again where the platform
/// destroyed it.
pub fn show() {
    #[cfg(target_os = "macos")]
    crate::handle().set_dock_icon(true);
    crate::handle().open();
}

/// macOS: the Dock icon was clicked or Sonora opened again from Finder. Rune's own reopening
/// skips an accessory app, which Sonora is while its window is closed to the tray, so this
/// brings the window back itself.
pub fn on_reopen(_ui: &mut Ui, visible: bool, _tx: &mut Tx) -> bool {
    if !visible {
        show();
    }
    true
}

/// The window's close button, ⌘W or `AppHandle::close`. With the icon in a tray and
/// `close_to_tray` on, the window hides and Sonora keeps playing; otherwise Sonora quits.
pub fn on_close(ui: &mut Ui, tx: &mut Tx) {
    remember(ui);
    let lingers = !rune::quitting() && ui.store.settings.close_to_tray() && crate::tray::lingers();
    if !lingers {
        return quit(ui, tx);
    }
    let Store {
        wake,
        settings,
        playback,
        ..
    } = &mut ui.store;
    wake.windows_closed(settings, playback);
    HOST.with_borrow_mut(|host| host.fullscreen_view = None);
    #[cfg(target_os = "macos")]
    crate::handle().set_dock_icon(false);
}

/// Runs in the window as the app starts to quit, whatever asked: saves where the window is and
/// lets the audio device go.
pub fn on_quit(ui: &mut Ui, _tx: &mut Tx) {
    remember(ui);
    ui.store.outputs.on_quit();
}

/// Quits Sonora from a handler or an update.
pub fn quit(ui: &mut Ui, tx: &mut Tx) {
    remember(ui);
    ui.store.outputs.on_quit();
    tx.quit();
}

/// The window took or lost focus, which the display lock needs. Losing it is also when a move
/// made without any frame drawn gets saved.
pub fn on_focus(ui: &mut Ui, focused: bool, _tx: &mut Tx) {
    let Store {
        wake,
        settings,
        playback,
        ..
    } = &mut ui.store;
    wake.set_focused(focused, settings, playback);
    if !focused {
        remember(ui);
    }
}

/// Before every frame: the system appearance the view saw, the fullscreen view for the wake
/// lock, the window's placement (at most once a [`PLACEMENT_EVERY`]) and its corner rounding.
/// Returns whether something the view reads changed.
pub fn poll(ui: &mut Ui, clock: f64) -> bool {
    let mut changed = false;
    if let Some(dark) = REPORTED_DARK.take()
        && dark != ui.system_dark
    {
        ui.system_dark = dark;
        let kind = match dark {
            true => ThemeKind::Dark,
            false => ThemeKind::Light,
        };
        ui.store.settings.set_system_theme(kind, &mut ui.store.cx);
        ui.settle();
        changed = true;
    }

    let fullscreen = matches!(ui.nav.current(), Destination::Fullscreen);
    let announced =
        HOST.with_borrow_mut(|host| host.fullscreen_view.replace(fullscreen) != Some(fullscreen));
    if announced {
        let Store {
            wake,
            settings,
            playback,
            ..
        } = &mut ui.store;
        wake.set_fullscreen(fullscreen, settings, playback);
    }

    let due = HOST.with_borrow(|host| {
        host.checked
            .is_none_or(|checked| clock - checked >= PLACEMENT_EVERY)
    });
    if due && crate::handle().is_visible() {
        HOST.with_borrow_mut(|host| host.checked = Some(clock));
        remember(ui);
    }
    round(ui);
    changed
}

/// The appearance a launch assumes before the window reports one: what the system said last
/// time, so a light desktop does not open on a dark frame.
pub fn assume_appearance(ui: &mut Ui) {
    ui.system_dark = ui.store.settings.system_theme() != ThemeKind::Light;
    ui.sync();
}

/// Saves where the window is, if it shows. A minimized or full-screen window keeps what was
/// saved before, and a maximized one keeps the size it restores to.
fn remember(ui: &mut Ui) {
    let Some(window) = HOST.with_borrow(|host| host.window.clone()) else {
        return;
    };
    // winit's X11 backend unwraps the X reply, so a window the server already destroyed (a
    // client killing it from outside) panics instead of answering an error
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| window.placement()));
    let Ok(Some(placement)) = read else {
        return;
    };
    HOST.with_borrow_mut(|host| host.full_screen = placement.full_screen);
    if placement.minimized || placement.full_screen {
        return;
    }
    let scale = placement.scale.max(f64::EPSILON) as f32;
    let current = Placement {
        x: placement.position.0 as f32 / scale,
        y: placement.position.1 as f32 / scale,
        width: placement.content_size.0 as f32 / scale,
        height: placement.content_size.1 as f32 / scale,
        maximized: placement.maximized,
    };
    let saved = HOST.with_borrow_mut(|host| match placement.maximized {
        true => Placement {
            maximized: true,
            ..host.normal.unwrap_or(current)
        },
        false => {
            host.normal = Some(current);
            current
        }
    });
    let Store { settings, cx, .. } = &mut ui.store;
    settings.remember_window(saved, cx);
}

/// Hands the window-rounding setting to the platform hook when it changed. Full screen has no
/// corners, so it asks for square ones while the window fills the display.
fn round(ui: &Ui) {
    #[cfg(any(target_os = "macos", windows))]
    {
        let wanted = match HOST.with_borrow(|host| host.full_screen) {
            true => ui::Rounding::Square,
            false => ui.store.settings.window_rounding(),
        };
        let changed = HOST.with_borrow_mut(|host| host.rounded.replace(wanted) != Some(wanted));
        if changed {
            state::apply_window_rounding(wanted);
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = ui;
}

/// Centres a first-launch window on the primary display.
fn centred(displays: &[rune::Display]) -> Option<(i32, i32)> {
    let display = displays
        .iter()
        .find(|display| display.primary)
        .or(displays.first())?;
    let scale = display.scale as f32;
    let (width, height) = (FIRST_SIZE.0 * scale, FIRST_SIZE.1 * scale);
    let x = display.position.0 as f32 + (display.size.0 as f32 - width).max(0.) / 2.;
    let y = display.position.1 as f32 + (display.size.1 as f32 - height).max(0.) / 2.;
    Some((x.round() as i32, y.round() as i32))
}

/// Where AppKit's traffic lights sit for the current settings and a title bar of `height`,
/// measured from the window's top-left corner. They are centred in the bar and kept as far from
/// the left edge as from the top, which centres them in the corner, with a little more room for
/// round corners. Hidden controls are moved off the window, since AppKit rebuilds them.
#[cfg(target_os = "macos")]
fn traffic_lights(settings: &AppSettings, height: f32) -> (f32, f32) {
    let offset = ((height - (lights::BORDER + lights::SIZE)) / 2.).max(0.);
    let left = match settings.window_controls() {
        true => match settings.window_rounding() {
            ui::Rounding::Round => offset + lights::ROUND_PAD,
            _ => offset,
        },
        false => lights::HIDDEN,
    };
    (left, offset)
}

/// The window's `HWND`, as an address.
#[cfg(windows)]
fn hwnd(window: &WindowHandle) -> Option<usize> {
    use rune::raw_window_handle::RawWindowHandle;

    window
        .with_raw_handle(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
            _ => None,
        })
        .ok()
        .flatten()
}

// DWM only offers two rounded presets (no arbitrary radius), so the four `Rounding` choices
// collapse onto them: `Square` turns rounding off, `Subtle` gets the small radius, and
// `Rounded`/`Round` both get the normal one, since DWM has nothing rounder than that.
#[cfg(windows)]
fn set_corner_preference(hwnd: usize, rounding: ui::Rounding) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DWMWCP_ROUND, DWMWCP_ROUNDSMALL, DwmSetWindowAttribute,
    };

    let preference: DWM_WINDOW_CORNER_PREFERENCE = match rounding {
        ui::Rounding::Square => DWMWCP_DONOTROUND,
        ui::Rounding::Subtle => DWMWCP_ROUNDSMALL,
        ui::Rounding::Rounded | ui::Rounding::Round => DWMWCP_ROUND,
    };
    // SAFETY: the window outlives the hook (Rune hides it rather than destroying it), and DWM
    // only reads the preference for the length given.
    unsafe {
        DwmSetWindowAttribute(
            hwnd as *mut std::ffi::c_void,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &preference as *const _ as *const std::ffi::c_void,
            size_of_val(&preference) as u32,
        );
    }
}

/// Sizes the window's corners through the toolbar style, the one control AppKit offers over them:
/// a window without a toolbar gets the smallest radius, a compact toolbar a larger one and a
/// unified toolbar the largest. The toolbar stays empty, so only the frame changes. Before
/// macOS 26 every style shares one radius.
#[cfg(target_os = "macos")]
fn set_toolbar_style(window: &WindowHandle, rounding: ui::Rounding) {
    use objc2::{MainThreadMarker, MainThreadOnly as _};
    use objc2_app_kit::{NSToolbar, NSView, NSWindowToolbarStyle};
    use objc2_foundation::NSString;
    use rune::raw_window_handle::RawWindowHandle;

    let Some(mtm) = MainThreadMarker::new() else {
        return log::warn!("window: toolbar style changed off the main thread");
    };
    let style = match rounding {
        ui::Rounding::Square | ui::Rounding::Subtle => None,
        ui::Rounding::Rounded => Some(NSWindowToolbarStyle::UnifiedCompact),
        ui::Rounding::Round => Some(NSWindowToolbarStyle::Unified),
    };
    let applied = window.with_raw_handle(|handle| {
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        // SAFETY: Rune lends its live `NSView` for the length of this call, on the main thread.
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let Some(native) = view.window() else {
            return;
        };
        match style {
            None => native.setToolbar(None),
            Some(style) => {
                if native.toolbar().is_none() {
                    let toolbar = NSToolbar::initWithIdentifier(
                        NSToolbar::alloc(mtm),
                        &NSString::from_str("sonora.corners"),
                    );
                    native.setToolbar(Some(&toolbar));
                }
                native.setToolbarStyle(style);
            }
        }
    });
    if let Err(error) = applied {
        log::debug!("window: no window to round yet: {error}");
    }
}
