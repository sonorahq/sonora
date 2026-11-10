//! System-wide typing activity for soft volume ducking.
//!
//! Observes keyboard activity on the whole computer and only reports that something was typed —
//! never the key, never the character, never to disk or the network. In-app fields still notify
//! through `ui::hear_typing` as a fallback when the OS tap cannot start.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use gpui::{App, Entity, Global};
use tokio::sync::mpsc;

use crate::{AppSettings, Playback};

/// How often a flooded key stream is collapsed into one duck pulse.
const PULSE: std::time::Duration = std::time::Duration::from_millis(40);

struct Installed(Option<Watch>);

impl Global for Installed {}

struct Watch {
    stop: Arc<AtomicBool>,
    _pump: JoinHandle<()>,
    _bridge: gpui::Task<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts or stops the system keyboard observer as the setting changes, and keeps the in-app
/// field fallback. Safe to call once at init.
pub fn attach(playback: Entity<Playback>, settings: Entity<AppSettings>, cx: &mut App) {
    let duck = playback.clone();
    ui::watch_typing(cx, move |cx| {
        duck.update(cx, |playback, cx| playback.note_typing(cx));
    });

    apply(&playback, settings.read(cx).duck_while_typing(), cx);

    cx.observe(&settings, {
        let playback = playback.clone();
        move |settings, cx| {
            apply(&playback, settings.read(cx).duck_while_typing(), cx);
        }
    })
    .detach();
}

fn apply(playback: &Entity<Playback>, want: bool, cx: &mut App) {
    let running = cx
        .try_global::<Installed>()
        .is_some_and(|installed| installed.0.is_some());

    if want == running {
        return;
    }

    // Replacing the global drops any running observer and joins its stop flag.
    cx.set_global(Installed(None));
    if !want {
        return;
    }

    match start(playback.clone(), cx) {
        Some(watch) => cx.set_global(Installed(Some(watch))),
        None => {
            log::warn!("typing: system keyboard observer unavailable; in-app fields still duck");
        }
    }
}

fn start(playback: Entity<Playback>, cx: &mut App) -> Option<Watch> {
    let stop = Arc::new(AtomicBool::new(false));
    let pending = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();

    let pump = platform::spawn(tx, stop.clone(), pending.clone())?;
    let playback = playback.downgrade();

    let bridge = cx.spawn(async move |cx| {
        while rx.recv().await.is_some() {
            pending.store(false, Ordering::Relaxed);
            let Some(playback) = playback.upgrade() else {
                break;
            };
            if playback
                .update(cx, |playback, cx| playback.note_typing(cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(PULSE).await;
            while rx.try_recv().is_ok() {
                pending.store(false, Ordering::Relaxed);
            }
        }
    });

    Some(Watch {
        stop,
        _pump: pump,
        _bridge: bridge,
    })
}

/// Shared bits the platform threads use to wake the app without saying which key it was.
mod pulse {
    use super::*;

    pub fn kick(tx: &mpsc::UnboundedSender<()>, pending: &AtomicBool) {
        if pending.swap(true, Ordering::Relaxed) {
            return;
        }
        if tx.send(()).is_err() {
            pending.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::ptr;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    /// Send-safe bits the hook needs. The HHOOK itself stays on the pump thread.
    struct Shared {
        tx: mpsc::UnboundedSender<()>,
        pending: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    }

    static SHARED: OnceLock<std::sync::Mutex<Option<Shared>>> = OnceLock::new();

    pub fn spawn(
        tx: mpsc::UnboundedSender<()>,
        stop: Arc<AtomicBool>,
        pending: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        let shared = SHARED.get_or_init(|| std::sync::Mutex::new(None));
        std::thread::Builder::new()
            .name("typing-duck".into())
            .spawn(move || {
                let hook = unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), ptr::null_mut(), 0)
                };
                if hook.is_null() {
                    log::warn!("typing: SetWindowsHookEx failed");
                    return;
                }
                let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                *shared.lock().unwrap() = Some(Shared {
                    tx,
                    pending,
                    stop: stop.clone(),
                });

                let stopper = stop.clone();
                let watcher = std::thread::spawn(move || {
                    while !stopper.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    unsafe {
                        PostThreadMessageW(thread, WM_QUIT, 0, 0);
                    }
                });

                let mut msg = MSG {
                    hwnd: ptr::null_mut(),
                    message: 0,
                    wParam: 0,
                    lParam: 0,
                    time: 0,
                    pt: POINT { x: 0, y: 0 },
                };
                unsafe {
                    while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {}
                    UnhookWindowsHookEx(hook);
                }
                if let Ok(mut slot) = shared.lock()
                    && slot
                        .as_ref()
                        .is_some_and(|held| Arc::ptr_eq(&held.stop, &stop))
                {
                    *slot = None;
                }
                let _ = watcher.join();
            })
            .ok()
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && (wparam == WM_KEYDOWN as WPARAM || wparam == WM_SYSKEYDOWN as WPARAM) {
            let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if typing_key(info.vkCode) && !shortcut_held() {
                if let Ok(shared) = SHARED.get().unwrap().lock() {
                    if let Some(shared) = shared.as_ref() {
                        if !shared.stop.load(Ordering::Relaxed) {
                            pulse::kick(&shared.tx, &shared.pending);
                        }
                    }
                }
            }
        }
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    fn shortcut_held() -> bool {
        unsafe {
            GetAsyncKeyState(VK_CONTROL as i32) < 0
                || GetAsyncKeyState(VK_MENU as i32) < 0
                || GetAsyncKeyState(VK_LWIN as i32) < 0
                || GetAsyncKeyState(VK_RWIN as i32) < 0
        }
    }

    /// Keys that mean the user is writing text, not steering the desktop.
    fn typing_key(vk: u32) -> bool {
        matches!(
            vk,
            0x08 | // backspace
            0x09 | // tab
            0x0D | // enter
            0x20 | // space
            0x30..=0x39 | // digits
            0x41..=0x5A | // letters
            0x60..=0x6F | // numpad
            0xBA..=0xC0 | // ;=,-./`
            0xDB..=0xDF // [\]'
        )
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::ptr;

    type CGEventRef = *mut c_void;
    type CFMachPortRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFAllocatorRef = *const c_void;
    type CGEventTapProxy = *mut c_void;
    type CGEventType = u32;
    type CGEventMask = u64;
    type CGEventTapLocation = u32;
    type CGEventTapPlacement = u32;
    type CGEventTapOptions = u32;

    const K_CG_SESSION_EVENT_TAP: CGEventTapLocation = 1;
    const K_CG_HEAD_INSERT_EVENT_TAP: CGEventTapPlacement = 0;
    const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: CGEventTapOptions = 1;
    const K_CG_EVENT_KEY_DOWN: CGEventType = 10;
    const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    const KEY_CONTROL: u16 = 59;
    const KEY_RIGHT_CONTROL: u16 = 62;
    const KEY_OPTION: u16 = 58;
    const KEY_RIGHT_OPTION: u16 = 61;
    const KEY_COMMAND: u16 = 55;
    const KEY_RIGHT_COMMAND: u16 = 54;

    type Callback =
        unsafe extern "C" fn(CGEventTapProxy, CGEventType, CGEventRef, *mut c_void) -> CGEventRef;

    struct TapCtx {
        tx: mpsc::UnboundedSender<()>,
        pending: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventTapCreate(
            place: CGEventTapLocation,
            placement: CGEventTapPlacement,
            options: CGEventTapOptions,
            events: CGEventMask,
            callback: Callback,
            user: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventSourceKeyState(state_id: u32, key: u16) -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFMachPortCreateRunLoopSource(
            allocator: CFAllocatorRef,
            port: CFMachPortRef,
            order: i64,
        ) -> CFRunLoopSourceRef;
        fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: *const c_void);
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopRun();
        fn CFRunLoopStop(rl: CFRunLoopRef);
        fn CFRelease(cf: *const c_void);
        static kCFRunLoopCommonModes: *const c_void;
    }

    pub fn spawn(
        tx: mpsc::UnboundedSender<()>,
        stop: Arc<AtomicBool>,
        pending: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        let mask = 1u64 << K_CG_EVENT_KEY_DOWN;
        std::thread::Builder::new()
            .name("typing-duck".into())
            .spawn(move || {
                let ctx = Box::new(TapCtx {
                    tx,
                    pending,
                    stop: stop.clone(),
                });
                let raw = Box::into_raw(ctx);
                let tap = unsafe {
                    CGEventTapCreate(
                        K_CG_SESSION_EVENT_TAP,
                        K_CG_HEAD_INSERT_EVENT_TAP,
                        K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                        mask,
                        tap_callback,
                        raw as *mut c_void,
                    )
                };
                if tap.is_null() {
                    unsafe {
                        drop(Box::from_raw(raw));
                    }
                    log::warn!(
                        "typing: CGEventTap unavailable; grant Input Monitoring to Sonora in System Settings"
                    );
                    return;
                }
                unsafe {
                    let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
                    let rl = CFRunLoopGetCurrent();
                    CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
                    CGEventTapEnable(tap, true);
                    let stopper = stop.clone();
                    let rl_ptr = rl as usize;
                    std::thread::spawn(move || {
                        while !stopper.load(Ordering::Relaxed) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        CFRunLoopStop(rl_ptr as CFRunLoopRef);
                    });
                    CFRunLoopRun();
                    CFRelease(source as *const c_void);
                    CFRelease(tap as *const c_void);
                    drop(Box::from_raw(raw));
                }
            })
            .ok()
    }

    unsafe extern "C" fn tap_callback(
        _proxy: CGEventTapProxy,
        ty: CGEventType,
        event: CGEventRef,
        info: *mut c_void,
    ) -> CGEventRef {
        if ty == K_CG_EVENT_KEY_DOWN && !info.is_null() {
            let ctx = unsafe { &*(info as *const TapCtx) };
            if !ctx.stop.load(Ordering::Relaxed) {
                let key = unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) }
                    as u16;
                if typing_key(key) && !shortcut_held() {
                    pulse::kick(&ctx.tx, &ctx.pending);
                }
            }
        }
        event
    }

    fn shortcut_held() -> bool {
        unsafe {
            CGEventSourceKeyState(1, KEY_CONTROL)
                || CGEventSourceKeyState(1, KEY_RIGHT_CONTROL)
                || CGEventSourceKeyState(1, KEY_OPTION)
                || CGEventSourceKeyState(1, KEY_RIGHT_OPTION)
                || CGEventSourceKeyState(1, KEY_COMMAND)
                || CGEventSourceKeyState(1, KEY_RIGHT_COMMAND)
        }
    }

    fn typing_key(key: u16) -> bool {
        matches!(key, 0..=50 | 36 | 48 | 49 | 51 | 76) && !matches!(key, 53 | 55..=63)
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use super::*;
    use std::ffi::{c_char, c_int, c_void};
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;
    use std::ptr;
    use std::time::Duration;

    const EV_KEY: u16 = 1;
    const KEY_A: usize = 30;
    const KEY_SPACE: usize = 57;
    const KEY_ENTER: usize = 28;
    const KEY_BACKSPACE: usize = 14;
    const KEY_TAB: usize = 15;
    const KEY_LEFTCTRL: usize = 29;
    const KEY_LEFTALT: usize = 56;
    const KEY_LEFTMETA: usize = 125;
    const KEY_RIGHTCTRL: usize = 97;
    const KEY_RIGHTALT: usize = 100;
    const KEY_RIGHTMETA: usize = 126;
    /// X11 keycodes on a PC keyboard are the Linux evdev codes plus eight.
    const X11_EVDEV_OFFSET: u8 = 8;

    #[repr(C)]
    struct InputEvent {
        _time: libc::timeval,
        type_: u16,
        code: u16,
        value: i32,
    }

    pub fn spawn(
        tx: mpsc::UnboundedSender<()>,
        stop: Arc<AtomicBool>,
        pending: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        if let Some(handle) = spawn_evdev(tx.clone(), stop.clone(), pending.clone()) {
            return Some(handle);
        }
        if let Some(handle) = spawn_x11(tx, stop, pending) {
            return Some(handle);
        }
        log::warn!(
            "typing: no system keyboard observer (needs /dev/input access, Flatpak device=input, or an X11 display); in-app fields still duck"
        );
        None
    }

    /// Reads kernel keyboards when the process may open them (input group, Flatpak
    /// `device=input`, or a native install that already can).
    fn spawn_evdev(
        tx: mpsc::UnboundedSender<()>,
        stop: Arc<AtomicBool>,
        pending: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        let devices = keyboards();
        if devices.is_empty() {
            return None;
        }
        std::thread::Builder::new()
            .name("typing-duck".into())
            .spawn(move || {
                let mut files: Vec<File> = devices
                    .into_iter()
                    .filter_map(|path| File::open(path).ok())
                    .collect();
                if files.is_empty() {
                    return;
                }
                // Non-blocking reads so the stop flag is noticed.
                for file in &files {
                    unsafe {
                        let flags = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
                        if flags >= 0 {
                            libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
                        }
                    }
                }
                let mut buf = [0u8; std::mem::size_of::<InputEvent>()];
                let mut mods = [false; 6];
                while !stop.load(Ordering::Relaxed) {
                    let mut progressed = false;
                    for file in &mut files {
                        loop {
                            match file.read(&mut buf) {
                                Ok(n) if n == buf.len() => {
                                    progressed = true;
                                    let event = unsafe {
                                        std::ptr::read_unaligned(buf.as_ptr() as *const InputEvent)
                                    };
                                    if event.type_ != EV_KEY {
                                        continue;
                                    }
                                    update_mods(&mut mods, event.code, event.value != 0);
                                    if event.value == 0 {
                                        continue;
                                    }
                                    if typing_key(event.code) && !mods.iter().any(|on| *on) {
                                        pulse::kick(&tx, &pending);
                                    }
                                }
                                Ok(_) => break,
                                Err(error)
                                    if error.kind() == std::io::ErrorKind::WouldBlock
                                        || error.kind() == std::io::ErrorKind::Interrupted =>
                                {
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    if !progressed {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            })
            .ok()
    }

    /// Polls the X keymap when `/dev/input` is closed off. Works for X11 sessions and for
    /// apps under XWayland when Sonora itself is on X11; pure Wayland needs device access.
    fn spawn_x11(
        tx: mpsc::UnboundedSender<()>,
        stop: Arc<AtomicBool>,
        pending: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        // Only attempt an X connection when a display is advertised.
        if std::env::var_os("DISPLAY").is_none() {
            return None;
        }
        type OpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
        type QueryKeymap = unsafe extern "C" fn(*mut c_void, *mut u8) -> c_int;
        type CloseDisplay = unsafe extern "C" fn(*mut c_void) -> c_int;
        let lib = [c"libX11.so.6", c"libX11.so"].into_iter().find_map(|name| {
            let lib = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY) };
            (!lib.is_null()).then_some(lib)
        })?;
        let load = |name: &std::ffi::CStr| -> Option<*mut c_void> {
            let sym = unsafe { libc::dlsym(lib, name.as_ptr()) };
            (!sym.is_null()).then_some(sym)
        };
        let Some(open) = load(c"XOpenDisplay") else {
            unsafe {
                libc::dlclose(lib);
            }
            return None;
        };
        let Some(query) = load(c"XQueryKeymap") else {
            unsafe {
                libc::dlclose(lib);
            }
            return None;
        };
        let Some(close) = load(c"XCloseDisplay") else {
            unsafe {
                libc::dlclose(lib);
            }
            return None;
        };
        let open: OpenDisplay = unsafe { std::mem::transmute(open) };
        let query: QueryKeymap = unsafe { std::mem::transmute(query) };
        let close: CloseDisplay = unsafe { std::mem::transmute(close) };
        let display = unsafe { open(ptr::null()) };
        if display.is_null() {
            unsafe {
                libc::dlclose(lib);
            }
            return None;
        }
        std::thread::Builder::new()
            .name("typing-duck".into())
            .spawn(move || {
                let mut prev = [0u8; 32];
                unsafe {
                    query(display, prev.as_mut_ptr());
                }
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(25));
                    let mut keys = [0u8; 32];
                    unsafe {
                        query(display, keys.as_mut_ptr());
                    }
                    for (byte_i, (now, was)) in keys.iter().zip(prev.iter()).enumerate() {
                        let pressed = now & !was;
                        if pressed == 0 {
                            continue;
                        }
                        for bit in 0..8 {
                            if pressed & (1 << bit) == 0 {
                                continue;
                            }
                            let x_code = (byte_i * 8 + bit) as u8;
                            let Some(evdev) = x_code.checked_sub(X11_EVDEV_OFFSET) else {
                                continue;
                            };
                            if typing_key(u16::from(evdev)) && !x11_shortcut_held(&keys) {
                                pulse::kick(&tx, &pending);
                            }
                        }
                    }
                    prev = keys;
                }
                unsafe {
                    close(display);
                    libc::dlclose(lib);
                }
            })
            .ok()
            .or_else(|| {
                unsafe {
                    close(display);
                    libc::dlclose(lib);
                }
                None
            })
    }

    fn keyboards() -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir("/dev/input") else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("event"))
                    && looks_like_keyboard(path)
            })
            .collect()
    }

    fn looks_like_keyboard(path: &std::path::Path) -> bool {
        let Ok(file) = File::open(path) else {
            return false;
        };
        // EVIOCGBIT(0x20 + EV_KEY) with a 128-byte buffer: _IOC(_IOC_READ, 'E', …).
        let mut bits = [0u8; 128];
        let request = eviocgbit(EV_KEY, bits.len());
        let got = unsafe { libc::ioctl(file.as_raw_fd(), request, bits.as_mut_ptr()) };
        if got < 0 {
            return true;
        }
        bit(&bits, KEY_A) || bit(&bits, KEY_SPACE)
    }

    fn eviocgbit(ev: u16, len: usize) -> libc::c_ulong {
        let typ = b'E';
        let nr = 0x20 + ev as u64;
        let size = len as u64;
        // _IOC(2, type, nr, size) on Linux.
        (2u64 << 30) | (size << 16) | ((typ as u64) << 8) | nr
    }

    fn bit(bits: &[u8], code: usize) -> bool {
        bits.get(code / 8)
            .is_some_and(|byte| byte & (1 << (code % 8)) != 0)
    }

    fn update_mods(mods: &mut [bool; 6], code: u16, down: bool) {
        let idx = match code as usize {
            KEY_LEFTCTRL => 0,
            KEY_RIGHTCTRL => 1,
            KEY_LEFTALT => 2,
            KEY_RIGHTALT => 3,
            KEY_LEFTMETA => 4,
            KEY_RIGHTMETA => 5,
            _ => return,
        };
        mods[idx] = down;
    }

    /// Ctrl/Alt/Super held in an X keymap, using the same evdev+8 mapping as the keys.
    fn x11_shortcut_held(keys: &[u8; 32]) -> bool {
        [
            KEY_LEFTCTRL,
            KEY_RIGHTCTRL,
            KEY_LEFTALT,
            KEY_RIGHTALT,
            KEY_LEFTMETA,
            KEY_RIGHTMETA,
        ]
        .iter()
        .any(|&code| {
            let x_code = code + usize::from(X11_EVDEV_OFFSET);
            keys.get(x_code / 8)
                .is_some_and(|byte| byte & (1 << (x_code % 8)) != 0)
        })
    }

    /// Linux keycodes that mean the user is writing text, not steering the desktop.
    fn typing_key(code: u16) -> bool {
        matches!(
            code as usize,
            2..=13 // digits, '-', '='
            | 16..=27 // Q..P, '[', ']'
            | 30..=41 // A..L, ';', '\'', '`'
            | 43..=53 // '\\', Z..M, ',', '.', '/'
            | KEY_SPACE
            | KEY_ENTER
            | KEY_BACKSPACE
            | KEY_TAB
        )
    }

    #[cfg(test)]
    mod tests {
        use super::typing_key;

        #[test]
        fn typing_key_covers_letters_not_shift() {
            assert!(typing_key(30)); // A
            assert!(typing_key(45)); // X
            assert!(typing_key(50)); // M
            assert!(typing_key(53)); // /
            assert!(!typing_key(42)); // Left Shift
            assert!(!typing_key(54)); // Right Shift
            assert!(!typing_key(29)); // Left Ctrl
        }
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd"
)))]
mod platform {
    use super::*;

    pub fn spawn(
        _tx: mpsc::UnboundedSender<()>,
        _stop: Arc<AtomicBool>,
        _pending: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        None
    }
}
