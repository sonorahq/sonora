mod artist;
mod catalog;
mod cover;
mod detail;
mod discord;
mod drm;
mod genre;
mod history;
mod home;
mod library;
mod logging;
mod lyrics;
mod mosaic;
mod network;
mod pins;
mod playback;
mod potoken;
mod profile;
mod queue;
mod remote;
mod scan;
mod scrobble;
mod search;
mod session;
mod settings;
mod sheets;
mod snapshot;
mod song;
mod tags;
mod toast;
mod updates;
mod usage;
mod window_shape;

pub use artist::ArtistDetail;
pub use cover::Cover;
pub use detail::{Collection, Detail, Header};
pub use drm::{CdmState, Drm};
pub use genre::{GenreDetails, Genres};
pub use history::{History, HistoryState};
pub use home::Home;
pub use library::{Library, LibraryEvent, LibraryPart, LibraryState, Problem, Ready, Shelf};
pub use logging::log_file;
pub use lyrics::{Lyrics, LyricsState};
pub use network::{Network, Reconnected};
pub use pins::{PinSort, Pins};
pub use playback::{Origin, Playback, PlaybackState, Repeat, Sleep, Whence};
pub use profile::Profile;
pub use queue::{Named, Queue, Resume, Stub};
pub use remote::{Remote, attach as attach_remote};
pub use scan::Scan;
pub use scrobble::{ScrobbleRow, ScrobbleState, Scrobbling};
pub use search::{AlbumHit, ArtistHit, Hit, Kind, PlaylistHit, Search};
pub use session::{Failure, ProviderInfo, Session, SessionEvent, SessionState};
pub use settings::{
    AppSettings, DiscordName, FilterValue, FullscreenControlsAutohide, MAX_PARTICLES,
    RomanizationScripts, SYSTEM_FONT, SideTab, remember_window, window_placement,
};
pub use song::SongDetail;
pub use tags::{TagState, Tags};
pub use toast::{Outcome, Target, Toast, Toasts};
pub use updates::{Release, UpdateState, Updates};
pub use usage::Usage;
pub use window_shape::{apply_window_rounding, install_rounded_window_hook};

use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use gpui::{App, AppContext as _, Entity, Global};
use music::{LyricsProvider, MusicProvider};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct Io(Arc<Runtime>);

impl Global for Io {}

/// Worker threads for the tokio runtime. The work here is network calls and the json they
/// answer with, never a long computation, so the default of one worker per core buys nothing
/// and costs a stack and an allocator arena each.
const WORKERS: usize = 4;
/// The ceiling on blocking threads, which is where the sqlite reads and the tag writes go. The
/// default is 512, far past anything Sonora queues at once.
const BLOCKING: usize = 16;

impl Io {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .max_blocking_threads(BLOCKING)
            .thread_name("sonora-io")
            .enable_all()
            .build()?;

        Ok(Self(Arc::new(runtime)))
    }

    pub fn global(cx: &App) -> Self {
        cx.global::<Self>().clone()
    }

    pub fn handle(&self) -> tokio::runtime::Handle {
        self.0.handle().clone()
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.0.spawn(future)
    }

    pub fn spawn_blocking<F, R>(&self, func: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.0.spawn_blocking(func)
    }
}

pub(crate) async fn join<T>(handle: JoinHandle<Result<T>>) -> Result<T> {
    handle.await?
}

/// Flattens a failed call's reason and tells `Network` about it, so one screen's failure puts
/// the whole app offline rather than only its own page.
pub(crate) fn blamed(error: &anyhow::Error, cx: &mut gpui::App) -> String {
    let reason = format!("{error:#}");
    Network::failed(&reason, cx);
    reason
}

/// Tells `Network` about a failure a caller only logs. An outage is then noticed at the first
/// call that runs into it, rather than the first one that happens to put its reason on a page.
pub(crate) fn noted(error: &anyhow::Error, cx: &mut gpui::App) {
    Network::failed(&format!("{error:#}"), cx);
}

/// Reports a network call's outcome to `Network` and turns its failure into the reason a screen
/// stores. A success is what puts the app back online the moment one load gets through.
pub(crate) fn settled<T>(result: Result<T>, cx: &mut gpui::App) -> std::result::Result<T, String> {
    match result {
        Ok(value) => {
            Network::reached(cx);
            Ok(value)
        }
        Err(error) => Err(blamed(&error, cx)),
    }
}

pub struct Sonora {
    pub session: Entity<Session>,
    pub cover: Entity<Cover>,
    pub drm: Entity<Drm>,
    pub library: Entity<Library>,
    pub history: Entity<History>,
    pub lyrics: Entity<Lyrics>,
    pub network: Entity<Network>,
    pub pins: Entity<Pins>,
    pub playback: Entity<Playback>,
    /// The window that mints YouTube's proof-of-origin token. Nothing reads it; it is held so
    /// that it keeps ticking.
    pub potoken: Entity<potoken::PoToken>,
    pub queue: Entity<Queue>,
    pub scan: Entity<Scan>,
    pub scrobbling: Entity<Scrobbling>,
    pub settings: Entity<AppSettings>,
    pub updates: Entity<Updates>,
    pub usage: Entity<Usage>,
}

impl Global for Sonora {}

impl Sonora {
    pub fn global(cx: &App) -> &Self {
        cx.global()
    }
}

pub fn init(
    cx: &mut App,
    io: Io,
    database: storage::Database,
    providers: Vec<Arc<dyn MusicProvider>>,
    local_provider: Arc<dyn MusicProvider>,
    lyrics_providers: Vec<Arc<dyn LyricsProvider>>,
) {
    cx.set_global(io.clone());
    let settings = cx.new(|_| AppSettings::load(database.clone()));
    let session =
        cx.new(|cx| Session::new(providers, local_provider, settings.clone(), io.clone(), cx));
    let network = cx.new(|_| Network::new(session.clone(), io.clone()));
    // A run that started without a network never signed out, so the account it kept is tried
    // again the moment there is one.
    session.update(cx, |_, cx| {
        cx.subscribe(&network, |this, _, _: &Reconnected, cx| {
            this.restore_if_offline(cx)
        })
        .detach();
    });
    let cache = storage::Cache::standard();
    let library = cx.new(|cx| Library::new(session.clone(), io.clone(), cache, cx));
    let queue = cx.new(|cx| Queue::new(session.clone(), settings.clone(), cx));
    let playback = cx.new(|cx| Playback::new(session.clone(), queue.clone(), settings.clone(), cx));
    let history = cx.new(|cx| {
        History::new(
            session.clone(),
            playback.clone(),
            database.clone(),
            io.clone(),
            cx,
        )
    });
    let scan = cx.new(|cx| Scan::new(session.clone(), cx));
    let scrobbling =
        cx.new(|cx| Scrobbling::new(playback.clone(), settings.clone(), io.clone(), cx));
    let lyrics = cx.new(|cx| {
        Lyrics::new(
            playback.clone(),
            queue.clone(),
            session.clone(),
            settings.clone(),
            lyrics_providers,
            io.clone(),
            cx,
        )
    });
    let cover = cx.new(|cx| Cover::new(session.clone(), playback.clone(), io.clone(), cx));
    let drm = cx.new(|cx| Drm::new(session.clone(), io.clone(), cx));
    let updates = cx.new(|cx| Updates::new(settings.clone(), io.clone(), cx));
    let usage = cx.new(|cx| Usage::new(session.clone(), database, io.clone(), cx));
    let pins = cx.new(|cx| Pins::new(settings.clone(), library.clone(), session.clone(), cx));
    let potoken = potoken::attach(cx);
    discord::attach(
        playback.clone(),
        settings.clone(),
        session.clone(),
        cover.clone(),
        io,
        cx,
    );

    cx.set_global(Sonora {
        session,
        cover,
        drm,
        library,
        history,
        lyrics,
        network,
        pins,
        playback,
        potoken,
        queue,
        scan,
        scrobbling,
        settings,
        updates,
        usage,
    });
}
