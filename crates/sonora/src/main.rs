#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod assets;
mod console;
mod dock;
#[cfg(target_os = "macos")]
mod gestures;
mod http;
mod logging;
mod memory;
mod single;
#[cfg(windows)]
mod thumbbar;
mod tray;

use std::path::PathBuf;
use std::process::{ExitCode, exit};
use std::sync::Arc;
use std::time::Duration;

use cli::commands::CommandType;
use gpui::{
    App, AppContext as _, Bounds, Pixels, QuitMode, Size, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use music::LyricsProvider;
use music::link::Linked;
use router::{Destination, Screen};
use state::{Opened, Outcome, Sonora, Toasts};
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

fn main() -> ExitCode {
    memory::tune();
    if let Some(code) = webview::probed() {
        exit(code);
    }
    // The Widevine host is this executable started again by the app. It must not claim the
    // instance socket, write the app's log file or start a runtime or a window.
    if let Some(module) = music::drm::hosted() {
        logging::console();
        if let Err(error) = music::drm::host(&module) {
            log::error!("widevine: the cdm host failed: {error:#}");
            exit(1);
        }
        return ExitCode::SUCCESS;
    }
    // A command for the running Sonora is sent and answered here, without a log file, a
    // runtime or a window. Older macOS adds a `-psn_` argument to a Finder launch.
    let args = std::env::args_os()
        .filter(|arg| !arg.to_string_lossy().starts_with("-psn_"))
        .collect();
    let command = cli::parse(args, state::VERSION);
    if !matches!(command, Ok(CommandType::Launch(_))) {
        console::attach();
    }
    let args = match command {
        Ok(CommandType::Launch(items)) => items,
        Ok(CommandType::Local(local)) => return cli::local(local),
        Ok(CommandType::Remote(request, format)) => {
            return match single::connect(request.streams()) {
                Ok(mut stream) => cli::run(&mut stream, &request, format),
                Err(_) => cli::not_running(),
            };
        }
        Err(refusal) => return refusal.exit(),
    };
    logging::init();

    let (sender, mut links) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
    let (asked, mut commands) = tokio::sync::mpsc::unbounded_channel::<single::Asked>();
    match single::claim(&args, sender.clone(), asked) {
        single::Instance::First => {}
        single::Instance::Running => return ExitCode::SUCCESS,
        single::Instance::Failed => exit(1),
    }
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
            return ExitCode::FAILURE;
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
            Arc::new(music::apple::AppleProvider::new()),
            Arc::new(music::youtube::YouTubeProvider::new()),
            Arc::new(music::subsonic::SubsonicProvider::new()),
            Arc::new(music::deezer::DeezerProvider::new()),
            Arc::new(music::tidal::TidalProvider::new()),
        ];
        let local_provider: Arc<dyn music::MusicProvider> =
            Arc::new(music::local::LocalProvider::new(
                dirs::cache_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("sonora"),
                database.clone(),
                storage::Cache::standard(),
            ));
        let lyrics: Vec<Arc<dyn LyricsProvider>> = vec![
            Arc::new(music::local::LocalLyrics),
            Arc::new(music::subsonic::SubsonicLyrics::new()),
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
        #[cfg(target_os = "macos")]
        state::install_rounded_window_hook(set_toolbar_style, cx);
        let startup = Sonora::global(cx).settings.read(cx).startup().to_owned();
        let start = Screen::from_id(&startup)
            .unwrap_or(Screen::Home)
            .destination();
        router::init(start, cx);
        let (look, overrides, language, pack, stillness, pace, remembered) = {
            let settings = Sonora::global(cx).settings.read(cx);
            (
                settings.look(),
                settings.theme_overrides(),
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
        ui::refresh_primary_paste(cx);
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
        #[cfg(target_os = "macos")]
        gestures::install(cx);
        memory::watch(cx);

        open_window(cx);
        let session = Sonora::global(cx).session.clone();
        session.update(cx, |session, cx| session.restore(cx));

        // A cold launch with files or links goes through the same batch a hot hand-off uses, now
        // that state exists to open them into. A link waits there for its provider to sign in.
        cx.subscribe(
            &Sonora::global(cx).links.clone(),
            |_, opened: &Opened, cx| {
                if let Some(destination) = destination(&opened.0) {
                    router::navigate(destination, cx);
                }
            },
        )
        .detach();
        if !args.is_empty() {
            sender.send(args.clone()).ok();
        }

        // Commands skip the open batching, since nothing arrives in bursts there.
        let control = Sonora::global(cx).control.clone();
        cx.spawn(async move |cx| {
            while let Some(asked) = commands.recv().await {
                control.update(cx, |control, cx| match asked {
                    single::Asked::Once(request, reply) => control.answer(request, reply, cx),
                    single::Asked::Watch(watch, sink) => control.watch(watch, sink, cx),
                });
            }
        })
        .detach();

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
    ExitCode::SUCCESS
}

/// Opens what a launch or a hand-off passed: a provider's link opens on that provider, and
/// anything else is read as a file to play. An address no provider knows is refused with a
/// toast rather than tried as a file.
fn follow(items: &[String], cx: &mut App) {
    show_window(cx);
    let links = Sonora::global(cx).links.clone();
    let mut paths: Vec<PathBuf> = Vec::new();
    for item in items {
        if links.update(cx, |links, cx| links.open(item, cx)) {
            continue;
        }
        match cli::item::is_address(item) {
            true => Toasts::show(Outcome::Failed, "toast-link-unreadable", cx),
            false => paths.push(cli::item::local_path(item)),
        }
    }
    if !paths.is_empty() {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.open_paths(paths, cx));
    }
}

/// The page a link opens on, for the kinds of link that open a page.
fn destination(linked: &Linked) -> Option<Destination> {
    match linked {
        Linked::Track(id) => Some(Destination::Song(id.clone().into())),
        Linked::Album(id) => Some(Destination::Album(id.clone().into())),
        Linked::Artist(id) => Some(Destination::Artist(id.clone().into())),
        Linked::Playlist(id) => Some(Destination::Playlist(id.clone().into())),
        Linked::User(id) => Some(Destination::User(id.clone().into())),
        Linked::Jam(_) | Linked::Short => None,
    }
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
        control: _,
        cover: _,
        devices: _,
        downloads: _,
        drm: _,
        equalizer: _,
        library,
        links: _,
        history: _,
        lyrics: _,
        network: _,
        outputs: _,
        pins: _,
        playback,
        potoken: _,
        queue,
        requirements: _,
        scan: _,
        scrobbling: _,
        settings: _,
        signal: _,
        updates: _,
        usage: _,
        wake: _,
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
    #[cfg(target_os = "macos")]
    let traffic_lights = views::traffic_lights(settings, cx.theme().metrics.title_bar);
    #[cfg(not(target_os = "macos"))]
    let traffic_lights = gpui::point(px(9.), px(9.));
    let look = settings.look();
    let background = ui::backdrop(look.blur_window, look.transparent);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(placement),
            display_id,
            window_background: background,
            titlebar: Some(TitlebarOptions {
                title: Some("Sonora".into()),
                appears_transparent: true,
                traffic_light_position: Some(traffic_lights),
            }),
            inactive_frame_interval: saver.interval(),
            is_movable: true,
            is_resizable: true,
            app_owns_titlebar_drag: true,
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

/// Sizes the window's corners through the toolbar style, the one control AppKit offers over them:
/// a window without a toolbar gets the smallest radius, a compact toolbar a larger one and a
/// unified toolbar the largest. The toolbar stays empty, so only the frame changes. Before
/// macOS 26 every style shares one radius.
#[cfg(target_os = "macos")]
fn set_toolbar_style(window: &gpui::Window, rounding: ui::Rounding) {
    use objc2::{MainThreadMarker, MainThreadOnly as _};
    use objc2_app_kit::{NSToolbar, NSView, NSWindowToolbarStyle};
    use objc2_foundation::NSString;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(mtm) = MainThreadMarker::new() else {
        return log::warn!("window: toolbar style changed off the main thread");
    };
    let Ok(RawWindowHandle::AppKit(handle)) =
        HasWindowHandle::window_handle(window).map(|handle| handle.as_raw())
    else {
        return;
    };
    // SAFETY: GPUI hands out its live `NSView` for the window, and this runs on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(native) = view.window() else {
        return;
    };

    let style = match rounding {
        ui::Rounding::Square | ui::Rounding::Subtle => None,
        ui::Rounding::Rounded => Some(NSWindowToolbarStyle::UnifiedCompact),
        ui::Rounding::Round => Some(NSWindowToolbarStyle::Unified),
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

/// Keeps a maximized window off the taskbar, which Windows covers once `hide_system_caption`
/// drops `WS_SYSMENU`. The clamp works on the final rectangle and not in `WM_GETMINMAXINFO`,
/// because Windows only carries that size over to a larger monitor when it is at least the
/// size of the primary one.
#[cfg(target_os = "windows")]
fn clamp_maximize(handle: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    unsafe { SetWindowSubclass(handle, Some(work_area), MAXIMIZE_SUBCLASS, 0) };
}

/// Pulls each edge of a maximized window that covers its whole monitor back to that monitor's
/// work area, keeping the frame overhang Windows adds past the other edges. Fullscreen strips
/// `WS_CAPTION` but keeps `WS_MAXIMIZE`, so a window without a caption is left alone.
#[cfg(target_os = "windows")]
unsafe extern "system" fn work_area(
    handle: *mut std::ffi::c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    _subclass: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
    };
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, IsZoomed, SWP_NOMOVE, SWP_NOSIZE, WINDOWPOS,
        WM_WINDOWPOSCHANGING, WS_CAPTION,
    };

    unsafe {
        if message == WM_WINDOWPOSCHANGING
            && IsZoomed(handle) != 0
            && GetWindowLongPtrW(handle, GWL_STYLE) & WS_CAPTION as isize == WS_CAPTION as isize
        {
            let position = &mut *(lparam as *mut WINDOWPOS);
            let rect = RECT {
                left: position.x,
                top: position.y,
                right: position.x + position.cx,
                bottom: position.y + position.cy,
            };
            let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info: MONITORINFO = std::mem::zeroed();
            monitor_info.cbSize = size_of::<MONITORINFO>() as u32;
            if position.flags & (SWP_NOMOVE | SWP_NOSIZE) == 0
                && GetMonitorInfoW(monitor, &mut monitor_info) != 0
            {
                let (screen, work) = (monitor_info.rcMonitor, monitor_info.rcWork);
                let covers = rect.left <= screen.left
                    && rect.top <= screen.top
                    && rect.right >= screen.right
                    && rect.bottom >= screen.bottom;
                if covers {
                    position.x += work.left - screen.left;
                    position.y += work.top - screen.top;
                    position.cx -= (screen.right - screen.left) - (work.right - work.left);
                    position.cy -= (screen.bottom - screen.top) - (work.bottom - work.top);
                }
            }
        }
        DefSubclassProc(handle, message, wparam, lparam)
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_handle(_window: &gpui::Window) -> Option<*mut std::ffi::c_void> {
    None
}
