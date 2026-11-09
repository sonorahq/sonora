#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
#[cfg(windows)]
mod app_icon;
mod assets;
mod console;
#[cfg(target_os = "macos")]
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
mod window;
#[cfg(target_os = "linux")]
mod x11_icon;

use std::path::PathBuf;
use std::process::{ExitCode, exit};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cli::commands::CommandType;
use music::LyricsProvider;
use rune::{AppHandle, AppSpec, Waker};
use state::{Link, Outcome, Store, Toasts};
use ui::Artwork;
use views::Ui;

/// Some file managers launch a fresh process per selected file instead of one with every path,
/// so a multi-file "Open With" arrives here as several single-file hand-offs within milliseconds
/// of each other. This is how long to wait for the burst to go quiet before acting on it, so
/// they land as one batch instead of racing each other into the engine one at a time.
const OPEN_COALESCE: Duration = Duration::from_millis(250);

/// The running app, for the threads and platform callbacks (tray, Dock, taskbar, gestures)
/// that reach the window state. Set once, before the app runs.
static HANDLE: OnceLock<AppHandle<Ui>> = OnceLock::new();

/// The running app's handle. Every caller runs after `main` set it.
pub fn handle() -> &'static AppHandle<Ui> {
    HANDLE
        .get()
        .expect("the app handle is set before the app runs")
}

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
    let command = cli::parse(args, state::version());
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
    // reqwest is built without a crypto provider, and opensubsonic still compiles aws-lc-rs in
    // beside ring, so rustls cannot pick one by itself. Every client uses ring from here on. A
    // second install only means another crate got there first.
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
    rune::prewarm_gpu();
    #[cfg(target_os = "macos")]
    dock::install_icon();
    rune::set_app_id(assets::app_id());
    actions::install_labels();

    let spec = AppSpec {
        min_size: Some(window::LEAST_SIZE),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(views::on_key),
        fonts: &assets::FACES,
        ..AppSpec::new("Sonora", window::FIRST_SIZE, unreachable_init, window::view)
    };
    // Closing the window keeps the app running; `window::on_close` quits unless the window
    // went to the tray.
    let app = rune::App::new(spec)
        .keep_running()
        .standard_menus()
        .on_close(window::on_close)
        .on_quit(window::on_quit)
        .on_focus(window::on_focus)
        .on_reopen(window::on_reopen)
        .on_window(window::created);
    let handle = app.handle();
    HANDLE.set(handle.clone()).ok();
    memory::watch(&io, handle.clone());

    // Commands from `sonora …` in a terminal skip the open batching, since nothing arrives in
    // bursts there.
    let commanded = handle.clone();
    io.spawn(async move {
        while let Some(asked) = commands.recv().await {
            commanded.update(move |ui: &mut Ui, _| {
                match asked {
                    single::Asked::Once(request, reply) => ui.store.answer_control(request, reply),
                    single::Asked::Watch(watch, sink) => ui.store.watch_control(watch, sink),
                }
                ui.settle();
            });
        }
    });

    // Files and links handed over by a launch or by a second process.
    let opened = handle.clone();
    io.spawn(async move {
        let mut pending: Vec<String> = Vec::new();
        while let Some(items) = links.recv().await {
            pending.extend(items);
            // Drain whatever else arrives in the same burst before acting on any of it.
            loop {
                tokio::time::sleep(OPEN_COALESCE).await;
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
            window::show();
            opened.update(move |ui: &mut Ui, _| follow(ui, &batch));
        }
    });
    if !args.is_empty() {
        sender.send(args).ok();
    }

    // The window factory sees the connected displays, which a saved placement is restored
    // onto, so the store (and with it the settings) is built here rather than in `App::init`.
    rune::run_windows(move |displays| {
        let store = build_store(io, handle);
        let (options, maximized) = window::options(&store.settings, displays);
        let app = app.window_options(options).maximized(maximized);
        #[cfg(any(target_os = "macos", windows))]
        let app = app.titlebar(window::titlebar(&store.settings));
        let app = app.init(move || Ui::new(store));
        vec![rune::WindowSpec::from_app("main", app)]
    });
    ExitCode::SUCCESS
}

/// `AppSpec::init` is replaced by the closure handed to `App::init`, which builds the store.
fn unreachable_init() -> Ui {
    unreachable!("the app's state is built by the closure handed to App::init")
}

/// Every provider, the local files and the lyrics sources, in one store whose jobs reach the
/// window through `handle`.
fn build_store(io: state::Io, handle: AppHandle<Ui>) -> Store {
    let database = storage::Database::standard();
    let providers: Vec<Arc<dyn music::MusicProvider>> = vec![
        Arc::new(music::spotify::SpotifyProvider::from_env()),
        Arc::new(music::apple::AppleProvider::new()),
        Arc::new(music::youtube::YouTubeProvider::new()),
        Arc::new(music::subsonic::SubsonicProvider::new()),
        Arc::new(music::deezer::DeezerProvider::new()),
        Arc::new(music::tidal::TidalProvider::new()),
    ];
    let local_provider: Arc<dyn music::MusicProvider> = Arc::new(music::local::LocalProvider::new(
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
    // A job's answer runs on the UI thread with the whole window state, then the store settles
    // what it raised.
    let link = Link::new(move |apply| {
        // A posted change draws unless it said it shows nothing (`Cx::hush`), so the engine's
        // position reports never run the view: the window follows them through live values.
        handle.update_if(move |ui: &mut Ui, _| {
            ui.store.cx.refresh();
            apply(&mut ui.store);
            ui.settle();
            ui.store.take_dirty()
        });
    });
    Store::new(io, link, database, providers, local_provider, lyrics)
}

/// Installs what needs the window's waker, restores the last account and starts the tray.
fn setup(ui: &mut Ui, waker: Waker) {
    let http = http::Client::new(ui.store.cx.io.handle());
    let http_for_tray = http.clone();
    let fetch: ui::Fetch = Arc::new(move |url: String| {
        let http = http.clone();
        Box::pin(async move { http.fetch(url).await })
    });
    let io = ui.store.cx.io.clone();
    let spawn: ui::Spawn = Arc::new(move |future| {
        io.spawn(future);
    });
    ui.artwork = Some(Artwork::new(fetch, spawn, waker));
    window::assume_appearance(ui);
    ui.store.restore_session();
    ui.settle();
    tray::start(ui, http_for_tray);
    #[cfg(target_os = "macos")]
    gestures::install();
}

/// Before every frame, and on every wake of a hidden window: a quit an update asked for, what
/// the host follows of the window, the tray and taskbar menus, then the views' live values.
fn poll(ui: &mut Ui, clock: f64, tx: &mut rune::Tx) -> bool {
    if ui.store.take_quit() {
        window::quit(ui, tx);
    }
    let changed = window::poll(ui, clock);
    tray::publish(ui);
    #[cfg(windows)]
    thumbbar::publish(ui);
    views::poll(ui, tx) | changed
}

/// Opens what a launch or a hand-off passed: a provider's link opens on that provider, and
/// anything else is read as a file to play. An address no provider knows is refused with a
/// toast rather than tried as a file.
fn follow(ui: &mut Ui, items: &[String]) {
    let mut paths: Vec<PathBuf> = Vec::new();
    for item in items {
        if ui.store.open_link(item) {
            continue;
        }
        match cli::item::is_address(item) {
            true => Toasts::show(Outcome::Failed, "toast-link-unreadable", &mut ui.store.cx),
            false => paths.push(cli::item::local_path(item)),
        }
    }
    if !paths.is_empty() {
        ui.store.open_paths(paths);
    }
    ui.settle();
}
