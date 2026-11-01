mod artist;
mod catalog;
mod control;
mod cover;
mod detail;
mod devices;
mod discord;
mod downloads;
mod drm;
mod equalizer;
mod genre;
mod history;
mod home;
mod library;
mod links;
mod listening;
mod logging;
mod lyrics;
mod mosaic;
mod network;
mod outputs;
mod overload;
mod pins;
mod playback;
mod potoken;
mod profile;
mod queue;
mod remote;
mod requirements;
mod scan;
mod scrobble;
mod search;
mod session;
mod settings;
mod sheets;
mod signal;
mod snapshot;
mod song;
mod tags;
mod toast;
mod updates;
mod usage;
mod vault;
mod wake;
mod window_shape;

pub use artist::{ArtistDetail, ArtistDetailEvent};
pub use control::Control;
pub use cover::Cover;
pub use detail::{Collection, Detail, Header};
pub use devices::Devices;
pub use downloads::{Downloads, DownloadsChanged, Progress, downloaded};
pub use drm::{CdmState, Drm};
pub use equalizer::{DeviceRule, Equalizer, RuleDevice, Sound};
pub use genre::{GenreDetails, Genres};
pub use history::{History, HistoryState};
pub use home::Home;
pub use library::{
    Addition, CoverEdit, Leaving, Library, LibraryEvent, LibraryPart, LibraryState, Problem, Ready,
    Shelf, cover_problem,
};
pub use links::{Links, Opened};
pub use listening::{CODE_LENGTH as SESSION_CODE_LENGTH, Listening, Member as RoomMember, Room};
pub use logging::log_file;
pub use lyrics::{Lyrics, LyricsState};
pub use music::connect::{Elsewhere, Repeat};
pub use network::{Network, Reconnected};
pub use outputs::Outputs;
pub use overload::OutputLevel;
pub use pins::{PinSort, Pins};
pub use playback::{Origin, Playback, PlaybackState, Sleep, Unreachable, Whence};
pub use profile::Profile;
pub use queue::{
    Queue, QueueKind, QueuePlacement, QueuePosition, QueueSections, QueueSlot, QueueView, Resume,
    Stub,
};
pub use remote::{Remote, attach as attach_remote};
pub use requirements::{Requirement, Requirements};
pub use scan::Scan;
pub use scrobble::{ScrobbleRow, ScrobbleState, Scrobbling};
pub use search::{AlbumHit, ArtistHit, Hit, Kind, PlaylistHit, Search};
pub use session::{Failure, ProviderInfo, Session, SessionEvent, SessionState};
pub use settings::{
    AppSettings, ConnectName, DiscordName, FilterValue, FullscreenControlsAutohide, LYRICS_SOURCES,
    PlayerBarLayout, Reloaded, RomanizationScripts, SYSTEM_FONT, SideTab, ThemeFamily, ThemeOffer,
    ThemeVariant, equalizer_profile_label, remember_window, window_placement,
};
pub use signal::{Change, Equalizing, FLOAT_BITS, Following, Signal, SignalPath};
pub use song::SongDetail;
pub use tags::{TagState, Tags};
pub use toast::{Outcome, Target, Toast, ToastAction, Toasts};
pub use updates::{Channel, Release, UpdateState, Updates, VERSION, updates_itself};
pub use usage::Usage;
pub use vault::CACHE_GB_RANGE;
pub use wake::Wake;
pub use window_shape::{apply_window_rounding, install_rounded_window_hook};

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gpui::{App, AppContext as _, Context, Entity, Global, Task};
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
/// The ceiling on blocking threads, which is where the sqlite reads, the artwork disk cache and
/// DNS lookups go. The artwork cache sits behind one lock, so threads past a handful only queue
/// on it while each holds a stack and an allocator arena. The default is 512.
const BLOCKING: usize = 8;
/// How long an idle blocking thread lingers before it exits. A burst of cover reads should not
/// leave its threads parked for the default ten seconds.
const LINGER: Duration = Duration::from_secs(2);

impl Io {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .max_blocking_threads(BLOCKING)
            .thread_keep_alive(LINGER)
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

    /// Loads on tokio and applies the result on the entity's thread. The caller retains the task.
    pub fn load<E: 'static, T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T>> + Send + 'static,
        apply: impl FnOnce(&mut E, Result<T>, &mut Context<E>) + 'static,
        cx: &mut Context<E>,
    ) -> Task<()> {
        let io = self.clone();
        cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(work)).await;
            this.update(cx, |this, cx| apply(this, loaded, cx)).ok();
        })
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
    pub control: Entity<Control>,
    pub cover: Entity<Cover>,
    pub devices: Entity<Devices>,
    pub downloads: Entity<Downloads>,
    pub drm: Entity<Drm>,
    pub equalizer: Entity<Equalizer>,
    pub library: Entity<Library>,
    pub links: Entity<Links>,
    pub listening: Entity<Listening>,
    pub history: Entity<History>,
    pub lyrics: Entity<Lyrics>,
    pub network: Entity<Network>,
    pub outputs: Entity<Outputs>,
    pub pins: Entity<Pins>,
    pub playback: Entity<Playback>,
    /// The window that mints YouTube's proof-of-origin token. Nothing reads it; it is held so
    /// that it keeps ticking.
    pub potoken: Entity<potoken::PoToken>,
    pub queue: Entity<Queue>,
    pub requirements: Entity<Requirements>,
    pub scan: Entity<Scan>,
    pub scrobbling: Entity<Scrobbling>,
    pub settings: Entity<AppSettings>,
    pub signal: Entity<Signal>,
    pub updates: Entity<Updates>,
    pub usage: Entity<Usage>,
    pub wake: Entity<Wake>,
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
    let settings = cx.new(|cx| {
        let mut settings = AppSettings::load(database.clone());
        settings.watch_files(cx);
        settings.report_broken(cx);
        settings
    });
    // Spotify sets up its own file cache when its session opens, which can be before Playback
    // exists, so it hears the setting first.
    music::vault::Vault::set_keeping(
        settings.read(cx).keep_tracks(),
        vault::bytes(settings.read(cx).cache_gb()),
    );
    let session =
        cx.new(|cx| Session::new(providers, local_provider, settings.clone(), io.clone(), cx));
    let network = cx.new(|cx| Network::new(session.clone(), settings.clone(), io.clone(), cx));
    // A run that started without a network never signed out, so the account it kept is tried
    // again the moment there is one.
    session.update(cx, |_, cx| {
        cx.subscribe(&network, |this, _, _: &Reconnected, cx| {
            this.restore_if_offline(cx)
        })
        .detach();
    });
    // A toast can carry an Undo that acts on the account it was shown for, so no such toast
    // outlives it. A plain one stays, since the sign-out can be what it explains.
    cx.subscribe(&session, |_, event, cx| {
        if let SessionEvent::SignedOut = event {
            Toasts::clear_account(cx);
        }
    })
    .detach();
    let cache = storage::Cache::standard();
    let library = cx.new(|cx| Library::new(session.clone(), io.clone(), cache, cx));
    let queue = cx.new(|cx| Queue::new(session.clone(), settings.clone(), cx));
    // Playback starts no engine until a provider is up, so the saved device is handed over
    // before any stream opens.
    let equalizer = cx.new(|cx| Equalizer::new(settings.clone(), cx));
    let playback = cx.new(|cx| {
        Playback::new(
            session.clone(),
            queue.clone(),
            settings.clone(),
            &equalizer,
            cx,
        )
    });
    let outputs = cx.new(|cx| Outputs::new(settings.clone(), playback.clone(), cx));
    let downloads = cx.new(|cx| {
        Downloads::new(
            session.clone(),
            playback.clone(),
            &network,
            database.clone(),
            io.clone(),
            cx,
        )
    });
    equalizer.update(cx, |equalizer, cx| {
        equalizer.follow(&playback, &outputs, cx)
    });
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
    let scrobbling = cx.new(|cx| {
        Scrobbling::new(
            playback.clone(),
            session.clone(),
            settings.clone(),
            io.clone(),
            cx,
        )
    });
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
    let requirements = cx.new(|cx| {
        Requirements::new(
            settings.clone(),
            session.clone(),
            outputs.clone(),
            io.clone(),
            cx,
        )
    });
    let usage = cx.new(|cx| Usage::new(session.clone(), database, io.clone(), cx));
    let pins = cx.new(|cx| Pins::new(settings.clone(), library.clone(), session.clone(), cx));
    let potoken = potoken::attach(cx);
    let signal = cx.new(|cx| Signal::new(playback.clone(), outputs.clone(), equalizer.clone(), cx));
    let wake = cx.new(|cx| Wake::new(settings.clone(), playback.clone(), io.clone(), cx));
    let devices = cx.new(|cx| {
        Devices::new(
            playback.clone(),
            queue.clone(),
            library.clone(),
            settings.clone(),
            session.clone(),
            io.clone(),
            cx,
        )
    });
    let listening = cx.new(|cx| {
        Listening::new(
            playback.clone(),
            queue.clone(),
            session.clone(),
            settings.clone(),
            io.clone(),
            cx,
        )
    });
    let links = cx.new(|cx| Links::new(session.clone(), devices.clone(), settings.clone(), cx));
    let control = cx.new(|cx| Control::new(session.clone(), playback.clone(), library.clone(), cx));
    discord::attach(
        playback.clone(),
        settings.clone(),
        session.clone(),
        cover.clone(),
        devices.clone(),
        io,
        cx,
    );

    cx.set_global(Sonora {
        session,
        control,
        cover,
        devices,
        downloads,
        drm,
        equalizer,
        library,
        links,
        listening,
        history,
        lyrics,
        network,
        outputs,
        pins,
        playback,
        potoken,
        queue,
        requirements,
        scan,
        scrobbling,
        settings,
        signal,
        updates,
        usage,
        wake,
    });
}
