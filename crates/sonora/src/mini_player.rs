//! Lifecycle of the floating mini player window.
//!
//! Rules:
//! - every window manipulation is deferred (`cx.defer`): actions are often
//!   handled while GPUI is still borrowing the window that triggered them;
//! - closing is idempotent: the handle is taken out of the global first.

use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, DisplayId, Global, Pixels, Point,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowHandle, WindowId,
    WindowOptions, point, px, size,
};
use state::{MiniPlayerPrefs, Sonora};
use views::{MiniPlayerView, mini_player_dimensions};

const TOGGLE_DEBOUNCE: Duration = Duration::from_millis(350);
const SCREEN_MARGIN: f32 = 24.;
/// Leaves room for the taskbar / dock on first placement.
const SCREEN_BOTTOM_MARGIN: f32 = 72.;

/// Handle of the main window, set in `main.rs` whenever `open_window` creates it.
pub struct MainWindow(pub AnyWindowHandle);
impl Global for MainWindow {}

#[derive(Default)]
struct MiniPlayerState {
    window: Option<WindowHandle<MiniPlayerView>>,
    opening: bool,
    can_linger: bool,
    minimized_main: Option<AnyWindowHandle>,
    last_origin: Option<Point<Pixels>>,
    last_toggle: Option<Instant>,
    /// Preferences currently applied to the open window, used to detect live setting changes.
    applied: Option<MiniPlayerPrefs>,
}
impl Global for MiniPlayerState {}

/// Initializes the mini player global state and observes live preference changes.
pub fn init(lingers: bool, cx: &mut App) {
    cx.set_global(MiniPlayerState {
        can_linger: lingers,
        ..MiniPlayerState::default()
    });

    let settings = Sonora::global(cx).settings.clone();
    cx.observe(&settings, |settings, cx| {
        let prefs = settings.read(cx).mini_player();
        apply_prefs(prefs, cx);
    })
    .detach();
}

/// Returns the live main window handle, if one is currently open.
pub fn main_window(cx: &App) -> Option<AnyWindowHandle> {
    let main = cx.try_global::<MainWindow>()?.0;
    is_alive(cx, main.window_id()).then_some(main)
}

/// Toggles the floating mini player window (`ToggleMiniPlayer` action).
pub fn toggle(cx: &mut App) {
    let now = Instant::now();
    {
        let state = cx.global_mut::<MiniPlayerState>();
        if state
            .last_toggle
            .is_some_and(|last| now.duration_since(last) < TOGGLE_DEBOUNCE)
        {
            return;
        }
        state.last_toggle = Some(now);
    }

    if live_window(cx).is_some() {
        close(cx);
    } else {
        open(cx);
    }
}

/// Opens the floating mini player, or activates it if already open.
pub fn open(cx: &mut App) {
    if let Some(handle) = live_window(cx) {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        });
        return;
    }

    {
        let state = cx.global_mut::<MiniPlayerState>();
        if state.opening {
            return;
        }
        state.opening = true;
    }

    let prefs = Sonora::global(cx).settings.read(cx).mini_player();
    let (bounds, display_id) = initial_placement(prefs, cx);
    let options = window_options(bounds, display_id, prefs);

    cx.defer(move |cx| {
        let result = cx.open_window(options, |window, cx| {
            cx.new(|cx| MiniPlayerView::new(window, cx))
        });
        cx.global_mut::<MiniPlayerState>().opening = false;

        let handle = match result {
            Ok(handle) => handle,
            Err(err) => {
                log::error!("mini player: cannot open window: {err:#}");
                return;
            }
        };

        let minimized_target = prefs.minimize_main.then(|| main_window(cx)).flatten();
        {
            let state = cx.global_mut::<MiniPlayerState>();
            state.window = Some(handle);
            state.applied = Some(prefs);
            state.minimized_main = minimized_target;
        }

        let _ = handle.update(cx, |_, window, _| {
            platform::apply_frame(window);
            window.on_next_frame(|window, _| {
                platform::reapply_frame(window);
            });
            platform::set_always_on_top(window, prefs.always_on_top);
        });

        if let Some(target) = minimized_target {
            minimize_window(target, cx);
        }
    });
}

/// Closes the mini player window and restores the main window only if opening minimized it.
pub fn close(cx: &mut App) {
    let restore = cx
        .try_global::<MiniPlayerState>()
        .and_then(|state| state.minimized_main);
    close_window(cx, restore);
}

/// Closes the mini player and focuses or reopens the main window (`ExpandMiniPlayer` action).
pub fn expand(cx: &mut App) {
    if let Some(state) = cx.try_global::<MiniPlayerState>()
        && state.window.is_some()
    {
        close_window(cx, None);
    }
    crate::show_window(cx);
}

/// Keeps main and mini player lifecycles consistent when either window closes from the OS.
pub fn on_window_closed(cx: &mut App, closed_id: WindowId, stay_in_tray: bool) {
    let Some(state) = cx.try_global::<MiniPlayerState>() else {
        return;
    };

    if let Some(handle) = state.window
        && handle.window_id() == closed_id
    {
        let state = cx.global_mut::<MiniPlayerState>();
        state.window = None;
        state.applied = None;
        let restore = state.minimized_main.take();
        if let Some(target) = restore {
            restore_window(target, cx);
        }
        return;
    }

    if state
        .minimized_main
        .is_some_and(|target| target.window_id() == closed_id)
    {
        cx.global_mut::<MiniPlayerState>().minimized_main = None;
    }

    let can_linger = cx
        .try_global::<MiniPlayerState>()
        .is_some_and(|state| state.can_linger);
    if main_window(cx).is_none()
        && !(can_linger && (stay_in_tray || Sonora::global(cx).settings.read(cx).close_to_tray()))
        && live_window(cx).is_some()
    {
        close_window(cx, None);
    }
}

/// Restores and focuses `window` if it is still alive.
pub fn restore_window(window: AnyWindowHandle, cx: &mut App) {
    if !is_alive(cx, window.window_id()) {
        return;
    }
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _| {
            platform::restore(window);
            window.activate_window();
        });
    });
}

fn close_window(cx: &mut App, restore: Option<AnyWindowHandle>) {
    let handle = {
        let state = cx.global_mut::<MiniPlayerState>();
        state.applied = None;
        state.minimized_main = None;
        state.window.take()
    };

    if let Some(handle) = handle {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |view, window, cx| {
                view.finish_volume_drag(cx);
                let origin = window.window_bounds().get_bounds().origin;
                cx.global_mut::<MiniPlayerState>().last_origin = Some(origin);
                let x = origin.x / px(1.);
                let y = origin.y / px(1.);
                let settings = Sonora::global(cx).settings.clone();
                settings.update(cx, |settings, cx| {
                    settings.set_mini_player_origin(Some((x, y)), cx);
                });
                window.remove_window();
            });
        });
    }

    if let Some(target) = restore {
        restore_window(target, cx);
    }
}

fn minimize_window(main: AnyWindowHandle, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = main.update(cx, |_, window, _| window.minimize_window());
    });
}

fn apply_prefs(prefs: MiniPlayerPrefs, cx: &mut App) {
    let (handle, previous) = {
        let state = cx.global_mut::<MiniPlayerState>();
        let Some(handle) = state.window else {
            return;
        };
        let Some(previous) = state.applied.replace(prefs) else {
            return;
        };
        (handle, previous)
    };

    if previous == prefs {
        return;
    }

    let topmost_changed = previous.always_on_top != prefs.always_on_top;
    let size_changed = previous.size != prefs.size
        || previous.lyrics_mode != prefs.lyrics_mode
        || previous.orientation != prefs.orientation;

    let mut async_cx = cx.to_async();
    cx.foreground_executor()
        .spawn(async move {
            let mut pending_resize = None;
            let _ = handle.update(&mut async_cx, |_, window, cx| {
                cx.notify();
                window.refresh();
                if topmost_changed {
                    platform::set_always_on_top(window, prefs.always_on_top);
                }
                if size_changed {
                    let dims = mini_player_dimensions(prefs);
                    pending_resize = platform::begin_resize(window, dims.width, dims.height);
                }
            });

            if let Some(resize) = pending_resize {
                platform::finish_resize(resize);
                let _ = handle.update(&mut async_cx, |_, window, cx| {
                    cx.notify();
                    window.refresh();
                });
            }
        })
        .detach();
}

fn window_options(
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
    prefs: MiniPlayerPrefs,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: platform::window_kind(prefs.always_on_top),
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some(crate::assets::app_id().into()),
        ..Default::default()
    }
}

fn clamp_axis(value: Pixels, origin: Pixels, window_span: Pixels, screen_span: Pixels) -> Pixels {
    if screen_span <= window_span {
        return origin;
    }
    value.clamp(origin, origin + screen_span - window_span)
}

fn initial_placement(prefs: MiniPlayerPrefs, cx: &App) -> (Bounds<Pixels>, Option<DisplayId>) {
    let dims = mini_player_dimensions(prefs);
    let window_size = size(px(dims.width), px(dims.height));

    let saved_origin = Sonora::global(cx)
        .settings
        .read(cx)
        .mini_player_origin()
        .map(|(x, y)| point(px(x), px(y)))
        .or_else(|| {
            cx.try_global::<MiniPlayerState>()
                .and_then(|state| state.last_origin)
        });

    if let Some(origin) = saved_origin {
        let bounds = Bounds::new(origin, window_size);
        let center = bounds.center();
        if let Some(display) = cx
            .displays()
            .into_iter()
            .find(|display| display.bounds().contains(&center))
        {
            let screen = display.bounds();
            let x = clamp_axis(
                origin.x,
                screen.origin.x,
                window_size.width,
                screen.size.width,
            );
            let y = clamp_axis(
                origin.y,
                screen.origin.y,
                window_size.height,
                screen.size.height,
            );
            return (Bounds::new(point(x, y), window_size), Some(display.id()));
        }
    }

    match cx.primary_display() {
        Some(display) => {
            let screen = display.bounds();
            let origin = point(
                (screen.origin.x + screen.size.width - window_size.width - px(SCREEN_MARGIN))
                    .max(screen.origin.x),
                (screen.origin.y + screen.size.height
                    - window_size.height
                    - px(SCREEN_BOTTOM_MARGIN))
                .max(screen.origin.y),
            );
            (Bounds::new(origin, window_size), Some(display.id()))
        }
        None => (
            Bounds::new(point(px(SCREEN_MARGIN), px(SCREEN_MARGIN)), window_size),
            None,
        ),
    }
}

fn live_window(cx: &App) -> Option<WindowHandle<MiniPlayerView>> {
    let handle = cx.try_global::<MiniPlayerState>()?.window?;
    is_alive(cx, handle.window_id()).then_some(handle)
}

fn is_alive(cx: &App, id: WindowId) -> bool {
    cx.windows().iter().any(|window| window.window_id() == id)
}

// ---------------------------------------------------------------------------
// Platform layer.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;
    use std::mem::size_of;

    use gpui::{Window, WindowKind};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmExtendFrameIntoClientArea,
        DwmSetWindowAttribute,
    };
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic,
        SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        WS_DISABLED, WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_LAYERED,
        WS_EX_STATICEDGE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_POPUP, WS_VISIBLE,
    };

    /// Special `DWMWA_BORDER_COLOR` value removing the 1px DWM frame border on Windows 11.
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;

    pub fn window_kind(_always_on_top: bool) -> WindowKind {
        // Normal + WS_POPUP after creation avoids GPUI's PopUp `WINDOW_STYLE(0)` caption artifact.
        WindowKind::Normal
    }

    fn hwnd(window: &Window) -> Option<HWND> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(win32) => Some(HWND(win32.hwnd.get() as *mut c_void)),
            _ => None,
        }
    }

    /// Strips the DWM border, caption, and drop shadow so only the GPUI capsule draws.
    pub fn apply_frame(window: &Window) {
        if let Some(hwnd) = hwnd(window) {
            apply_frame_hwnd(hwnd);
        }
    }

    fn apply_frame_hwnd(hwnd: HWND) {
        // SAFETY: `hwnd` is a valid Win32 HWND owned by this UI thread.
        unsafe {
            let old_style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let keep = old_style & (WS_VISIBLE.0 | WS_DISABLED.0);
            SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP.0 | keep) as isize);

            let old_ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let remove = WS_EX_APPWINDOW.0
                | WS_EX_LAYERED.0
                | WS_EX_WINDOWEDGE.0
                | WS_EX_CLIENTEDGE.0
                | WS_EX_STATICEDGE.0
                | WS_EX_DLGMODALFRAME.0;
            let new_ex = (old_ex & !remove) | WS_EX_TOOLWINDOW.0;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex as isize);

            let margins = MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 0,
                cyBottomHeight: 0,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

            let border = DWMWA_COLOR_NONE;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &border as *const u32 as *const c_void,
                size_of::<u32>() as u32,
            );

            let corner_value: i32 = DWMWCP_DONOTROUND.0;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner_value as *const i32 as *const c_void,
                size_of::<i32>() as u32,
            );

            let policy_value: i32 = DWMNCRP_DISABLED.0;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_NCRENDERING_POLICY,
                &policy_value as *const i32 as *const c_void,
                size_of::<i32>() as u32,
            );

            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED
                    | SWP_NOMOVE
                    | SWP_NOSIZE
                    | SWP_NOZORDER
                    | SWP_NOACTIVATE
                    | SWP_NOOWNERZORDER,
            );
        }
    }

    pub fn reapply_frame(window: &Window) {
        apply_frame(window);
    }

    /// Captures the Win32 target size so `SetWindowPos` can run outside any GPUI window borrow.
    pub fn begin_resize(window: &mut Window, width: f32, height: f32) -> Option<(isize, i32, i32)> {
        let hwnd = hwnd(window)?.0 as isize;
        let scale = window.scale_factor();
        let pixel_w = (width * scale).round().max(1.) as i32;
        let pixel_h = (height * scale).round().max(1.) as i32;
        Some((hwnd, pixel_w, pixel_h))
    }

    /// Resizes the popup window and reapplies the frameless style outside GPUI borrows so
    /// `WM_SIZE` -> `bounds_changed` never hits an already-borrowed `AppCell`.
    pub fn finish_resize((raw_hwnd, pixel_w, pixel_h): (isize, i32, i32)) {
        let hwnd = HWND(raw_hwnd as *mut c_void);
        // SAFETY: `hwnd` belongs to a live `WS_POPUP` window without Win32 non-client frame.
        let result = unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                pixel_w,
                pixel_h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
        if let Err(err) = result {
            log::warn!("mini player: cannot resize window: {err}");
        }
        apply_frame_hwnd(hwnd);
    }

    pub fn set_always_on_top(window: &Window, on: bool) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        // SAFETY: `hwnd` belongs to a live window on this thread; flags only alter Z-order.
        let result = unsafe {
            SetWindowPos(
                hwnd,
                Some(insert_after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        if let Err(err) = result {
            log::warn!("mini player: cannot update topmost state: {err}");
        }
    }

    pub fn restore(window: &Window) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        // SAFETY: `hwnd` belongs to a live window owned by this process.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use gpui::{Window, WindowKind, px, size};
    use objc2::rc::Retained;
    use objc2_app_kit::{NSFloatingWindowLevel, NSNormalWindowLevel, NSView, NSWindow};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub fn window_kind(always_on_top: bool) -> WindowKind {
        match always_on_top {
            true => WindowKind::Floating,
            false => WindowKind::Normal,
        }
    }

    fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
        let Ok(RawWindowHandle::AppKit(handle)) =
            HasWindowHandle::window_handle(window).map(|handle| handle.as_raw())
        else {
            return None;
        };
        // SAFETY: GPUI hands out a live `NSView` for the window and this runs on the main thread.
        let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    /// Drops the rectangular system shadow so only the rounded GPUI capsule shows.
    pub fn apply_frame(window: &Window) {
        if let Some(native) = native_window(window) {
            native.setHasShadow(false);
        }
    }

    pub fn reapply_frame(_window: &Window) {}

    pub fn begin_resize(window: &mut Window, width: f32, height: f32) -> Option<()> {
        window.resize(size(px(width), px(height)));
        None
    }

    pub fn finish_resize((): ()) {}

    pub fn set_always_on_top(window: &Window, on: bool) {
        if let Some(native) = native_window(window) {
            let level = match on {
                true => NSFloatingWindowLevel,
                false => NSNormalWindowLevel,
            };
            native.setLevel(level);
        }
    }

    pub fn restore(window: &Window) {
        if let Some(native) = native_window(window) {
            native.deminiaturize(None);
            native.orderFront(None);
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use gpui::{Window, WindowKind, px, size};

    pub fn window_kind(_always_on_top: bool) -> WindowKind {
        // Keep the mini player independent so minimizing the main window never hides it.
        WindowKind::Normal
    }

    #[cfg(target_os = "linux")]
    pub fn apply_frame(window: &Window) {
        if let Err(error) = x11::mark_utility(window) {
            log::debug!("mini player: cannot apply X11 utility hints: {error:#}");
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn apply_frame(_window: &Window) {}

    pub fn reapply_frame(_window: &Window) {}

    pub fn begin_resize(window: &mut Window, width: f32, height: f32) -> Option<()> {
        window.resize(size(px(width), px(height)));
        None
    }

    pub fn finish_resize((): ()) {}

    #[cfg(target_os = "linux")]
    pub fn set_always_on_top(window: &Window, on: bool) {
        if let Err(error) = x11::set_above(window, on) {
            log::debug!("mini player: cannot update X11 above state: {error:#}");
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn set_always_on_top(_window: &Window, _on: bool) {}

    pub fn restore(_window: &Window) {}

    #[cfg(target_os = "linux")]
    mod x11 {
        use anyhow::{Context as _, Result};
        use gpui::Window;
        use raw_window_handle::{
            HasDisplayHandle as _, HasWindowHandle as _, RawDisplayHandle, RawWindowHandle,
        };
        use x11rb::connection::Connection as _;
        use x11rb::protocol::xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode,
        };
        use x11rb::wrapper::ConnectionExt as _;
        use x11rb::xcb_ffi::XCBConnection;

        const NET_WM_STATE_REMOVE: u32 = 0;
        const NET_WM_STATE_ADD: u32 = 1;

        fn handles(window: &Window) -> Result<Option<(XCBConnection, u32, u32)>> {
            let RawWindowHandle::Xcb(handle) = window.window_handle()?.as_raw() else {
                return Ok(None);
            };
            let RawDisplayHandle::Xcb(display) = window.display_handle()?.as_raw() else {
                return Ok(None);
            };
            let ptr = display
                .connection
                .context("cannot find the X11 connection")?;
            // SAFETY: borrows GPUI's live XCB connection without taking ownership.
            let connection = unsafe { XCBConnection::from_raw_xcb_connection(ptr.as_ptr(), false) }
                .context("cannot borrow the X11 connection")?;
            let root = connection
                .setup()
                .roots
                .get(display.screen as usize)
                .context("cannot find the X11 root screen")?
                .root;
            Ok(Some((connection, handle.window.get(), root)))
        }

        pub(super) fn mark_utility(window: &Window) -> Result<()> {
            let Some((connection, win, _)) = handles(window)? else {
                return Ok(());
            };
            let type_atom = connection
                .intern_atom(false, b"_NET_WM_WINDOW_TYPE")?
                .reply()?
                .atom;
            let utility = connection
                .intern_atom(false, b"_NET_WM_WINDOW_TYPE_UTILITY")?
                .reply()?
                .atom;
            connection
                .change_property32(
                    PropMode::REPLACE,
                    win,
                    type_atom,
                    AtomEnum::ATOM,
                    &[utility],
                )?
                .check()?;

            let state_atom = connection
                .intern_atom(false, b"_NET_WM_STATE")?
                .reply()?
                .atom;
            let skip_taskbar = connection
                .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")?
                .reply()?
                .atom;
            let skip_pager = connection
                .intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER")?
                .reply()?
                .atom;
            connection
                .change_property32(
                    PropMode::APPEND,
                    win,
                    state_atom,
                    AtomEnum::ATOM,
                    &[skip_taskbar, skip_pager],
                )?
                .check()?;
            connection.flush().context("cannot flush X11 properties")
        }

        pub(super) fn set_above(window: &Window, on: bool) -> Result<()> {
            let Some((connection, win, root)) = handles(window)? else {
                return Ok(());
            };
            let state_atom = connection
                .intern_atom(false, b"_NET_WM_STATE")?
                .reply()?
                .atom;
            let above_atom = connection
                .intern_atom(false, b"_NET_WM_STATE_ABOVE")?
                .reply()?
                .atom;
            let action = match on {
                true => NET_WM_STATE_ADD,
                false => NET_WM_STATE_REMOVE,
            };
            let event = ClientMessageEvent::new(32, win, state_atom, [action, above_atom, 0, 1, 0]);
            connection
                .send_event(
                    false,
                    root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    event,
                )?
                .check()?;
            connection.flush().context("cannot flush X11 event")
        }
    }
}
