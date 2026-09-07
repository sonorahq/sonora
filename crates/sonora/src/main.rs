#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod assets;
mod dock;
mod http;
mod logging;
mod memory;
mod single;
mod tray;

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, Pixels, QuitMode, Size, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};
use music::LyricsProvider;
use router::Screen;
use state::Sonora;
use ui::ActiveTheme as _;
use ui::ThemeKind;
use views::Root;

const LEAST_SIZE: Size<Pixels> = size(px(480.), px(400.));
const FIRST_SIZE: Size<Pixels> = size(px(920.), px(640.));

fn main() {
    logging::init();

    let opened = std::env::args().skip(1).find(|arg| !arg.starts_with('-'));
    let (sender, mut links) = tokio::sync::mpsc::unbounded_channel();
    if let single::Instance::Running = single::claim(opened.as_deref(), sender.clone()) {
        return;
    }
    let opened_start = opened.as_deref().and_then(router::destination);

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
    app.on_open_urls(move |opened| {
        for link in opened {
            sender.send(link).ok();
        }
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
        ];
        let local_provider: Arc<dyn music::MusicProvider> =
            Arc::new(music::local::LocalProvider::new(
                dirs::config_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("sonora"),
                database.clone(),
            ));
        let lyrics: Vec<Arc<dyn LyricsProvider>> = vec![
            Arc::new(music::binimum::Binimum::new()),
            Arc::new(music::musixmatch::Musixmatch::new()),
            Arc::new(music::lrclib::LrcLib::new()),
            Arc::new(music::kugou::Kugou::new()),
            Arc::new(music::netease::NetEase::new()),
        ];
        state::init(cx, io, database, providers, local_provider, lyrics);
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

        cx.spawn(async move |cx| {
            while let Some(link) = links.recv().await {
                cx.update(|cx| follow(&link, cx));
            }
        })
        .detach();

        cx.activate(true);
    });
}

fn follow(link: &str, cx: &mut App) {
    show_window(cx);
    if let Some(destination) = router::destination(link) {
        router::navigate(destination, cx);
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
        cover: _,
        library,
        history: _,
        scrobbling: _,
        lyrics: _,
        playback,
        queue,
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
    let placement = state::window_placement(LEAST_SIZE, cx)
        .unwrap_or_else(|| WindowBounds::Windowed(Bounds::centered(None, FIRST_SIZE, cx)));

    let settings = Sonora::global(cx).settings.read(cx);
    let saver = settings.saver();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let decorations = settings.window_decorations();

    cx.open_window(
        WindowOptions {
            window_bounds: Some(placement),
            window_background: WindowBackgroundAppearance::Transparent,
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
            state::attach_remote(platform_handle(window), cx);
            state::remember_window(window, cx);
            cx.new(|cx| Root::new(session, library, playback, queue, window, cx))
        },
    )
    .expect("failed to open window");
}

#[cfg(target_os = "windows")]
fn platform_handle(window: &gpui::Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY, DwmSetWindowAttribute,
    };

    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    let handle = handle.hwnd.get() as *mut std::ffi::c_void;
    unsafe {
        DwmSetWindowAttribute(
            handle,
            DWMWA_NCRENDERING_POLICY as u32,
            &DWMNCRP_DISABLED as *const _ as *const std::ffi::c_void,
            size_of_val(&DWMNCRP_DISABLED) as u32,
        );
    }
    Some(handle)
}

#[cfg(not(target_os = "windows"))]
fn platform_handle(_window: &gpui::Window) -> Option<*mut std::ffi::c_void> {
    None
}
