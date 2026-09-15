#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod assets;
mod dock;
mod http;
mod logging;
mod memory;
mod single;
#[cfg(windows)]
mod thumbbar;
mod tray;

use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Pixels, QuitMode, Size, TitlebarOptions, WindowBounds,
    WindowOptions, point, px, size,
};
use music::LyricsProvider;
use router::Screen;
use state::Sonora;
use ui::ActiveTheme as _;
use ui::ThemeKind;
use views::Root;

const LEAST_SIZE: Size<Pixels> = size(px(480.), px(400.));
const FIRST_SIZE: Size<Pixels> = size(px(920.), px(640.));
/// Some file managers launch a fresh process per selected file instead of one with every path,
/// so a multi-file "Open With" arrives here as several single-file hand-offs within milliseconds
/// of each other. This is how long to wait for the burst to go quiet before acting on it, so
/// they land as one batch instead of racing each other into the engine one at a time.
const OPEN_COALESCE: Duration = Duration::from_millis(250);

fn main() {
    logging::init();

    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let (sender, mut links) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
    match single::claim(&args, sender.clone()) {
        single::Instance::First => {}
        single::Instance::Running => return,
        single::Instance::Failed => exit(1),
    }
    // Only a lone spotify:/web link overrides the startup screen; one or more file paths from
    // an "Open With" launch are handled after state exists, once the window loop is running.
    let opened_start = match args.as_slice() {
        [single] => router::destination(single),
        _ => None,
    };

    // Two rustls backends are compiled in: librespot, oauth2 and ytmusic still ask for ring,
    // while reqwest 0.13 and opensubsonic ask for aws-lc-rs. rustls refuses to guess between
    // them, so one is picked here. A second install only means another crate got there first.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let io = match state::Io::new() {
        Ok(io) => io,
        Err(error) => {
            eprintln!("sonora: cannot start runtime: {error:#}");
            return;
        }
    };

    let app = gpui_platform::application()
        .with_assets(assets::Assets)
        .with_http_client(Arc::new(http::Client::new(io.handle())));
    let opened_urls_sender = sender.clone();
    app.on_open_urls(move |opened| {
        opened_urls_sender.send(opened).ok();
    });
    app.on_reopen(show_window);

    app.run(move |cx: &mut App| {
        if let Err(error) = assets::Assets.load_fonts(cx) {
            log::error!("sonora: cannot load bundled fonts: {error:#}");
        }

        let database = storage::Database::standard();
        let providers: Vec<Arc<dyn music::MusicProvider>> = vec![
            Arc::new(music::spotify::SpotifyProvider::from_env()),
            Arc::new(music::youtube::YouTubeProvider::new()),
            Arc::new(music::subsonic::SubsonicProvider::new()),
            Arc::new(music::deezer::DeezerProvider::new()),
            Arc::new(music::apple::AppleProvider::new()),
        ];
        let local_provider: Arc<dyn music::MusicProvider> =
            Arc::new(music::local::LocalProvider::new(
                dirs::cache_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("sonora"),
                database.clone(),
            ));
        let lyrics: Vec<Arc<dyn LyricsProvider>> = vec![
            Arc::new(music::spotify::SpotifyLyrics::from_env()),
            Arc::new(music::youtube::YouTubeLyrics::new()),
            Arc::new(music::binimum::Binimum::new()),
            Arc::new(music::musixmatch::Musixmatch::new()),
            Arc::new(music::lrclib::LrcLib::new()),
            Arc::new(music::kugou::Kugou::new()),
            Arc::new(music::netease::NetEase::new()),
        ];
        state::init(cx, io, database, providers, local_provider, lyrics);
        #[cfg(target_os = "windows")]
        state::install_rounded_window_hook(set_corner_preference, cx);
        let opened_a_destination = opened_start.is_some();
        let start = opened_start.unwrap_or_else(|| {
            let startup = Sonora::global(cx).settings.read(cx).startup().to_owned();
            Screen::from_id(&startup)
                .unwrap_or(Screen::Home)
                .destination()
        });
        router::init(start, cx);
        let (look, overrides, language, pack, stillness, pace, remembered) = {
            let settings = Sonora::global(cx).settings.read(cx);
            (
                settings.look(),
                settings.theme_overrides().clone(),
                settings.language().to_owned(),
                settings.icons().to_owned(),
                settings.stillness(),
                settings.pace(),
                settings.system_theme(),
            )
        };
        i18n::set(i18n::resolve(&language));
        icons::set(&pack);
        ui::motion::apply(stillness, pace, cx);
        // linux answers late
        let reported = match cfg!(any(target_os = "linux", target_os = "freebsd")) {
            true => None,
            false => Some(ThemeKind::reported(cx)),
        };
        ThemeKind::assume(reported.unwrap_or(remembered));
        if let Some(reported) = reported.filter(|reported| *reported != remembered) {
            Sonora::global(cx)
                .settings
                .clone()
                .update(cx, |settings, cx| settings.set_system_theme(reported, cx));
        }
        ui::Theme::init(look, &overrides, cx);

        let lingers = tray::install(show_window, cx);
        if lingers {
            cx.set_quit_mode(QuitMode::Explicit);
        }
        actions::register(lingers, cx);
        memory::watch(cx);

        open_window(cx);
        let session = Sonora::global(cx).session.clone();
        session.update(cx, |session, cx| session.restore(cx));

        // A cold "Open With" launch reaches Playback through the same batch a hot hand-off
        // uses, now that state exists to open the files into.
        if !opened_a_destination && !args.is_empty() {
            sender.send(args.clone()).ok();
        }

        cx.spawn(async move |cx| {
            let mut pending: Vec<String> = Vec::new();
            loop {
                match links.recv().await {
                    Some(items) => pending.extend(items),
                    None => break,
                }
                // Drain whatever else arrives in the same burst before acting on any of it.
                loop {
                    cx.background_executor().timer(OPEN_COALESCE).await;
                    let mut more = false;
                    while let Ok(items) = links.try_recv() {
                        pending.extend(items);
                        more = true;
                    }
                    if !more {
                        break;
                    }
                }
                let batch = std::mem::take(&mut pending);
                cx.update(|cx| follow(&batch, cx));
            }
        })
        .detach();

        cx.activate(true);
    });
}

fn follow(items: &[String], cx: &mut App) {
    show_window(cx);
    let mut destination = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    for item in items {
        match router::destination(item) {
            Some(found) => destination = Some(found),
            None => paths.extend(local_path_from_arg(item)),
        }
    }
    if let Some(destination) = destination {
        router::navigate(destination, cx);
    }
    if !paths.is_empty() {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.open_paths(paths, cx));
    }
}

/// A bare filesystem path, or a `file://` URI decoded back into one — the two shapes an "Open
/// With" launch or drop hands us across platforms.
fn local_path_from_arg(arg: &str) -> Option<PathBuf> {
    match arg.strip_prefix("file://") {
        Some(rest) => Some(PathBuf::from(percent_decode(rest))),
        None => Some(PathBuf::from(arg)),
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn show_window(cx: &mut App) {
    match cx.windows().first() {
        Some(window) => {
            window
                .update(cx, |_, window, _| window.activate_window())
                .ok();
        }
        None => {
            dock::show(true);
            open_window(cx);
        }
    }
    cx.activate(true);
}

fn open_window(cx: &mut App) {
    let Sonora {
        session,
        cover: _,
        drm: _,
        library,
        history: _,
        lyrics: _,
        pins: _,
        playback,
        queue,
        scrobbling: _,
        settings: _,
        updates: _,
        usage: _,
    } = Sonora::global(cx);
    let (session, library, playback, queue) = (
        session.clone(),
        library.clone(),
        playback.clone(),
        queue.clone(),
    );
    let (placement, display_id) = state::window_placement(LEAST_SIZE, cx)
        .map(|(placement, display_id)| (placement, Some(display_id)))
        .unwrap_or_else(|| {
            (
                WindowBounds::Windowed(Bounds::centered(None, FIRST_SIZE, cx)),
                None,
            )
        });

    let settings = Sonora::global(cx).settings.read(cx);
    let saver = settings.saver();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let decorations = settings.window_decorations();
    let look = settings.look();
    let background = ui::backdrop(look.blur, look.transparent);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(placement),
            display_id,
            window_background: background,
            titlebar: Some(TitlebarOptions {
                title: Some("Sonora".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(9.), px(9.))),
            }),
            inactive_frame_interval: saver.interval(),
            is_movable: true,
            is_resizable: true,
            app_id: Some("sonora".into()),
            window_min_size: Some(LEAST_SIZE),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            window_decorations: Some(decorations),
            ..Default::default()
        },
        |window, cx| {
            window.set_rem_size(cx.theme().font_size);
            #[cfg(target_os = "windows")]
            set_corner_preference(
                window,
                Sonora::global(cx).settings.read(cx).window_rounding(),
            );
            let handle = platform_handle(window);
            state::attach_remote(handle, cx);
            #[cfg(windows)]
            if let Some(handle) = handle {
                thumbbar::install(window.window_handle().window_id(), handle, cx);
            }
            state::remember_window(window, cx);
            cx.new(|cx| Root::new(session, library, playback, queue, window, cx))
        },
    )
    .expect("failed to open window");
}

// DWM only offers two rounded presets (no arbitrary radius), so the four `Rounding` choices
// collapse onto them: `Square` turns rounding off, `Subtle` gets the small radius, and
// `Rounded`/`Round` both get the normal one, since DWM has nothing rounder than that.
#[cfg(target_os = "windows")]
fn set_corner_preference(window: &gpui::Window, rounding: ui::Rounding) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DWMWCP_ROUND, DWMWCP_ROUNDSMALL, DwmSetWindowAttribute,
    };

    let Ok(RawWindowHandle::Win32(handle)) =
        HasWindowHandle::window_handle(window).map(|handle| handle.as_raw())
    else {
        return;
    };
    let handle = handle.hwnd.get() as *mut std::ffi::c_void;
    let preference: DWM_WINDOW_CORNER_PREFERENCE = match rounding {
        ui::Rounding::Square => DWMWCP_DONOTROUND,
        ui::Rounding::Subtle => DWMWCP_ROUNDSMALL,
        ui::Rounding::Rounded | ui::Rounding::Round => DWMWCP_ROUND,
    };
    unsafe {
        DwmSetWindowAttribute(
            handle,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &preference as *const _ as *const std::ffi::c_void,
            size_of_val(&preference) as u32,
        );
    }
}

#[cfg(target_os = "windows")]
fn platform_handle(window: &gpui::Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let handle = handle.hwnd.get() as *mut std::ffi::c_void;
    hide_system_caption(handle);
    clamp_maximize(handle);
    Some(handle)
}

// DWM draws the caption buttons behind the client area, where an opaque window hides them
// and a transparent or blurred one shows them beside Sonora's own. They come with
// `WS_SYSMENU`, so that is the style to drop: `WS_CAPTION` has to stay, because DWM only
// animates minimize, restore and close on a window that carries it. Alt+F4 and the taskbar
// still close the window; only the Alt+Space menu goes, and Sonora's title bar has no use
// for it.
#[cfg(target_os = "windows")]
fn hide_system_caption(handle: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_SYSMENU,
    };

    unsafe {
        let style = GetWindowLongPtrW(handle, GWL_STYLE);
        if style & WS_SYSMENU as isize == 0 {
            return;
        }
        SetWindowLongPtrW(handle, GWL_STYLE, style & !(WS_SYSMENU as isize));
        SetWindowPos(
            handle,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(target_os = "windows")]
const MAXIMIZE_SUBCLASS: usize = 1;
#[cfg(target_os = "windows")]
fn clamp_maximize(handle: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    unsafe { SetWindowSubclass(handle, Some(work_area), MAXIMIZE_SUBCLASS, 0) };
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn work_area(
    handle: *mut std::ffi::c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    _subclass: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MINMAXINFO, WM_GETMINMAXINFO};

    unsafe {
        if message == WM_GETMINMAXINFO {
            let monitor = MonitorFromWindow(handle, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info: MONITORINFO = std::mem::zeroed();
            monitor_info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut monitor_info) != 0 {
                let (screen, work) = (monitor_info.rcMonitor, monitor_info.rcWork);
                let info = &mut *(lparam as *mut MINMAXINFO);
                info.ptMaxPosition.x += work.left - screen.left;
                info.ptMaxPosition.y += work.top - screen.top;
                info.ptMaxSize.x -= (screen.right - screen.left) - (work.right - work.left);
                info.ptMaxSize.y -= (screen.bottom - screen.top) - (work.bottom - work.top);
            }
        }
        DefSubclassProc(handle, message, wparam, lparam)
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_handle(_window: &gpui::Window) -> Option<*mut std::ffi::c_void> {
    None
}
