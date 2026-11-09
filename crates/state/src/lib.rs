mod artist;
mod core;
mod react;
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
mod keychain;
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

pub use artist::ArtistDetail;
pub use crate::core::{Apply, Cx, Job, Link};
pub use control::Control;
pub use cover::Cover;
pub use detail::{Collection, Detail, Header};
pub use devices::Devices;
pub use downloads::{Active, Downloads, Pending, Progress, Remaining, Subject};
pub use drm::{CdmState, Drm};
pub use equalizer::{DeviceRule, Equalizer, RuleDevice, Sound};
pub use genre::{GenreDetails, Genres};
pub use history::{History, HistoryState};
pub use home::Home;
pub use library::{
    Addition, CoverEdit, Leaving, Library, LibraryEvent, LibraryPart, LibraryState, Problem, Ready,
    Shelf, cover_problem,
};
pub use links::Links;
pub use listening::{CODE_LENGTH as SESSION_CODE_LENGTH, Listening, Member as RoomMember, Room};
pub use logging::log_file;
pub use lyrics::{Lyrics, LyricsState};
pub use music::connect::{Elsewhere, Repeat};
pub use network::Network;
pub use outputs::Outputs;
pub use overload::OutputLevel;
pub use pins::{PinSort, Pins};
pub use playback::{Origin, Playback, PlaybackEvent, PlaybackState, Sleep, Unreachable, Whence};
pub use profile::Profile;
pub use queue::{
    Queue, QueueKind, QueuePlacement, QueuePosition, QueueSections, QueueSlot, QueueView, Resume,
    Stub,
};
pub use remote::Remote;
pub use requirements::{Requirement, Requirements};
pub use scan::Scan;
pub use scrobble::{ScrobbleRow, ScrobbleState, Scrobbling};
pub use search::{AlbumHit, ArtistHit, Hit, Kind, PlaylistHit, Search, Searching};
pub use session::{Failure, ProviderInfo, Session, SessionEvent, SessionState};
pub use settings::{
    AppSettings, Area, ConnectName, DiscordName, FilterValue, FullscreenControlsAutohide,
    LYRICS_SOURCES, Placement, PlayerBarLayout, RomanizationScripts, SYSTEM_FONT, SideTab,
    ThemeFamily, ThemeOffer, ThemeVariant, equalizer_profile_label,
};
pub use signal::{Equalizing, FLOAT_BITS, Following, Signal, SignalChange, SignalPath};
pub use song::SongDetail;
pub use tags::{TagState, Tags};
pub use toast::{Outcome, Target, Toast, ToastAction, ToastButton, ToastUpdate, Toasts};
pub use updates::{Channel, Release, UpdateState, Updates, updates_itself, version};
pub use usage::Usage;
pub use vault::CACHE_GB_RANGE;
pub use wake::Wake;
pub use window_shape::{apply_window_rounding, install_rounded_window_hook};

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use music::{LyricsProvider, MusicProvider};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct Io(Arc<Runtime>);

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
/// How many rounds of reactions one change may set off before the store stops following them,
/// which only a model that keeps re-raising its own change would reach.
const ROUNDS: usize = 32;

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

/// Hands the free pages of the malloc heap back to the kernel after a burst of short-lived
/// allocations. The app runs on one malloc arena, which otherwise keeps what a burst freed resident.
/// It locks the heap while it walks it, so call it off the main thread and only once a burst is over.
pub(crate) fn trim_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Flattens a failed call's reason and tells `Network` about it, so one screen's failure puts
/// the whole app offline rather than only its own page.
pub(crate) fn blamed(error: &anyhow::Error, cx: &mut Cx) -> String {
    let reason = format!("{error:#}");
    cx.emit(Event::NetworkFailed(reason.clone()));
    reason
}

/// Tells `Network` about a failure a caller only logs. An outage is then noticed at the first
/// call that runs into it, rather than the first one that happens to put its reason on a page.
pub(crate) fn noted(error: &anyhow::Error, cx: &mut Cx) {
    cx.emit(Event::NetworkFailed(format!("{error:#}")));
}

/// Reports a network call's outcome to `Network` and turns its failure into the reason a screen
/// stores. A success is what puts the app back online the moment one load gets through.
pub(crate) fn settled<T>(result: Result<T>, cx: &mut Cx) -> std::result::Result<T, String> {
    match result {
        Ok(value) => {
            cx.emit(Event::NetworkReached);
            Ok(value)
        }
        Err(error) => Err(blamed(&error, cx)),
    }
}

/// A model whose changes others follow. Raising one with [`Cx::changed`] stands for "this model
/// changed"; the store hands it to every follower once the current change is done, and several
/// raises of one topic before then reach them once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Topic {
    Session,
    Settings,
    Network,
    Library,
    Playback,
    Queue,
    Cover,
    Devices,
    Outputs,
    Equalizer,
    Downloads,
    Lyrics,
    History,
    Pins,
    Listening,
}

/// Something that happened that other models act on. Unlike a [`Topic`], each one is delivered
/// as raised, in order.
#[derive(Clone, Debug)]
pub enum Event {
    Session(SessionEvent),
    /// The network came back, so a screen that gave up can load again.
    Reconnected,
    /// A call failed; only a reason that reads as a lost connection puts the app offline.
    NetworkFailed(String),
    /// A call reached the network, which is the quickest way back online.
    NetworkReached,
    /// The settings file changed on disk and was read again.
    SettingsReloaded,
    Library(LibraryEvent),
    Playback(PlaybackEvent),
    /// The set of downloaded items changed.
    DownloadsChanged,
    /// The artist a page showed is gone from the provider.
    ArtistGone(String),
    /// A link the app was opened with resolved to something to show.
    LinkOpened(music::link::Linked),
    /// A message for the toast stack.
    Toast(Toast),
    /// A change to a toast already on the stack.
    ToastUpdate(ToastUpdate),
    /// The app should quit, after its quit hooks ran.
    Quit,
}

/// One thing the store routes: a topic that changed or an event that happened.
#[derive(Clone, Debug)]
pub enum Change {
    Topic(Topic),
    Event(Event),
}

/// Every model the app holds, each a plain struct. The window owns the store; handlers and the
/// closures jobs post change it directly and then call [`Store::settle`] (posted closures are
/// settled by the host), which hands each [`Change`] they raised to the models that follow it.
pub struct Store {
    pub cx: Cx,
    pub settings: AppSettings,
    pub session: Session,
    pub network: Network,
    pub control: Control,
    pub cover: Cover,
    pub devices: Devices,
    pub downloads: Downloads,
    pub drm: Drm,
    pub equalizer: Equalizer,
    pub library: Library,
    pub links: Links,
    pub listening: Listening,
    pub history: History,
    pub lyrics: Lyrics,
    pub outputs: Outputs,
    pub pins: Pins,
    pub playback: Playback,
    pub queue: Queue,
    pub requirements: Requirements,
    pub scan: Scan,
    pub scrobbling: Scrobbling,
    pub signal: Signal,
    pub toasts: Toasts,
    pub updates: Updates,
    pub usage: Usage,
    pub wake: Wake,
    pub home: Home,
    pub genres: Genres,
    /// What the pages that show one item at a time are showing.
    pub pages: Pages,
    /// Workers that run as long as the app does and keep nothing the window shows, such as
    /// YouTube's proof-of-origin window.
    pub(crate) background: Vec<Job>,
    /// Discord's rich presence, made from what plays and handed to its worker.
    pub(crate) discord: discord::Discord,
    /// The system's media controls, once [`Store::attach_remote`] reached them.
    pub(crate) remote: Option<Remote>,
}

/// The models behind the pages that show one item: each is replaced as another item opens and
/// dropped, with its loads, when it closes.
#[derive(Default)]
pub struct Pages {
    pub album: Option<Detail>,
    pub playlist: Option<Detail>,
    pub artist: Option<ArtistDetail>,
    pub genre: Option<GenreDetails>,
    pub song: Option<SongDetail>,
    pub user: Option<Profile>,
    /// The search page's results for the submitted query.
    pub search: Option<Search>,
    /// What the search field suggests while a query is typed.
    pub suggest: Option<Search>,
    pub tags: Option<Tags>,
}

impl Store {
    /// Builds every model, in the order their reactions run, and starts what runs from launch:
    /// the settings watch, the account restore and the background workers. `link` is how jobs
    /// reach the window.
    pub fn new(
        io: Io,
        link: Link,
        database: storage::Database,
        providers: Vec<Arc<dyn MusicProvider>>,
        local_provider: Arc<dyn MusicProvider>,
        lyrics_providers: Vec<Arc<dyn LyricsProvider>>,
    ) -> Self {
        let mut cx = Cx::new(io, link);
        let mut settings = AppSettings::load(database.clone());
        settings.watch_files(&mut cx);
        settings.report_broken(&mut cx);
        // Spotify sets up its own file cache when its session opens, which can be before Playback
        // exists, so it hears the setting first.
        music::vault::Vault::set_keeping(
            settings.keep_tracks(),
            vault::bytes(settings.cache_gb()),
        );
        let session = Session::new(providers, local_provider, &settings, &mut cx);
        let network = Network::new(&settings);
        let library = Library::new(&session, storage::Cache::standard(), &mut cx);
        let queue = Queue::new(&settings);
        let equalizer = Equalizer::new(&settings);
        let outputs = Outputs::new(&settings, &mut cx);
        let playback = Playback::new(&session, &settings, &equalizer, &mut cx);
        let downloads = Downloads::new(database.clone(), &mut cx);
        let history = History::new(database.clone(), &session, &mut cx);
        let scan = Scan::new(&session, &mut cx);
        let scrobbling = Scrobbling::new(&settings, &mut cx);
        let lyrics = Lyrics::new(&settings, lyrics_providers, &mut cx);
        let updates = Updates::new(&settings, &mut cx);
        let requirements = Requirements::new(&outputs, &mut cx);
        let usage = Usage::new(database, &mut cx);
        let wake = Wake::new(&mut cx);
        let devices = Devices::new(&session, &settings, &playback, &queue, &mut cx);
        let home = Home::new(&session, &mut cx);
        let discord = discord::Discord::new(&mut cx);
        let background = vec![potoken::attach(&cx)];

        Self {
            settings,
            session,
            network,
            control: Control::new(),
            cover: Cover::new(),
            devices,
            downloads,
            drm: Drm::new(),
            equalizer,
            library,
            links: Links::new(),
            listening: Listening::new(),
            history,
            lyrics,
            outputs,
            pins: Pins::new(),
            playback,
            queue,
            requirements,
            scan,
            scrobbling,
            signal: Signal::new(),
            toasts: Toasts::new(),
            updates,
            usage,
            wake,
            home,
            genres: Genres::new(),
            pages: Pages::default(),
            discord,
            remote: None,
            background,
            cx,
        }
    }

    /// Hands every change raised so far to the models that follow it, round after round, until
    /// no model raises another.
    pub fn settle(&mut self) {
        for _ in 0..ROUNDS {
            let changes = self.cx.take_changes();
            if changes.is_empty() {
                return;
            }
            for change in &changes {
                self.react(change);
            }
        }
        log::warn!("state: changes kept raising more after {ROUNDS} rounds; the rest wait");
    }

    /// Whether anything changed since the host last drew, for a poll that runs outside a commit.
    pub fn take_dirty(&mut self) -> bool {
        self.cx.take_dirty()
    }
}
