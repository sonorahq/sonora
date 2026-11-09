use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use futures::future::BoxFuture;
use music::connect::{self, Command, Repeat};
use music::vault::Vault;
use music::{
    Analyzer, Meter, MusicApi, PlaybackConfig, PlaybackEvent as BackendEvent, PlaybackEvents,
    PlaybackFactory, Player, Quality, Spectrum, Track, TrackSignal, Transcode,
};
use serde::{Deserialize, Serialize};
use ui::{Pin, PinKind};

use crate::devices::collection_of;
use crate::queue::{Queue, QueuePlacement};
use crate::{
    AppSettings, Change, Cx, Devices, Downloads, Drm, Elsewhere, Equalizer, Event, Job, Network,
    Outcome, Session, SessionEvent, SessionState, Store, Target, Toast, Topic,
};

const POSITION_INTERVAL: Duration = Duration::from_millis(500);
const CLOCK_SETTLE: Duration = Duration::from_secs(1);
/// Position reports that can still arrive from before a seek: one already in the channel and
/// one for a packet decoded before the engine read the command. A successful seek keeps the
/// engine busy, so more than this while one is in flight means it never left the old position.
const STALE_POSITIONS: u8 = 2;
/// How far before a seek target the engine may land and still be shown at the target. A decoder
/// settles on the packet holding the target, a few tens of milliseconds early at most; showing
/// the target keeps a lyric clicked at its first word on that line, not the one before.
const SEEK_SNAP: Duration = Duration::from_millis(100);
const PRELOAD_BEFORE_END: Duration = Duration::from_secs(10);
const SKIP_DEBOUNCE: Duration = Duration::from_millis(250);
const RESTART_WINDOW: Duration = Duration::from_secs(3);
const KEY_COOLDOWN: Duration = Duration::from_secs(1);
/// How long after a play or pause press an engine event saying the opposite is taken for the
/// echo of an earlier press rather than news. Past it the engine is believed again, so a pause
/// the engine makes on its own is not lost for good.
const ECHO_WINDOW: Duration = Duration::from_secs(2);
/// How many loads in a row may fail before the queue stops moving on by itself and waits for
/// play.
const FAILURE_LIMIT: u8 = 3;
/// The wait before a track the provider turned down for now is asked for again. Each refusal in
/// a row doubles it, up to `THROTTLE_CAP`.
const THROTTLE_WAIT: Duration = Duration::from_secs(2);
const THROTTLE_CAP: Duration = Duration::from_secs(30);
/// How many refusals in a row a track is retried through before it is held paused until play.
const THROTTLE_LIMIT: u8 = 6;
const RESUME_STEP: Duration = Duration::from_secs(5);
/// How long a pause may last before play reloads the track instead of resuming the engine. What
/// an engine holds goes stale while paused, and resuming it plays out the queued audio and then
/// reports the track as ended. Apple's license asks for renewal 13 minutes after it is issued,
/// and that clock starts at the load rather than the pause, so this leaves room for a long track.
const STALE_PAUSE: Duration = Duration::from_secs(5 * 60);
const TAPER_DB: f32 = 50.;
const SIMILAR_LIMIT: usize = 20;

/// How few tracks may be left to play before radio asks for the next batch of suggestions.
/// Asking this early keeps the wait for the station off the gap between two tracks.
const RADIO_LOOKAHEAD: usize = 10;

/// How many continuations in a row may bring nothing the queue has not heard before the station
/// is dropped. An empty stretch lands with the queue still short, which asks for the next one
/// straight away, so a provider serving the same tracks over would otherwise never be let go of.
const STATION_DRY_LIMIT: u8 = 3;

/// How many unreadable files a toast names before it cuts the list short.
const LISTED_FILES: usize = 5;
/// How many tracks' signal reports are kept: the one playing, the one queued behind it, and
/// room for a skip that has not been heard yet.
const SIGNALS: usize = 4;
/// The FAQ entry on why Spotify may refuse an account the keys to its audio.
const KEYS_FAQ: &str = "https://sonorahq.org/faq#spotify-playback-keys";

/// Why the provider will play nothing more this session. Every later load fails the same way
/// without asking the engine again.
#[derive(Clone, Copy, PartialEq)]
enum Refusal {
    /// Spotify denied an audio key for the account.
    Keys,
    /// The provider streams only to a signed-in listener.
    SignIn,
}

/// How a load came about, which decides its debounce and whether the engine may keep its queue.
#[derive(Clone, Copy, PartialEq)]
enum Start {
    /// The user chose a track.
    Pick,
    /// The user skipped once.
    Skip,
    /// The user is skipping repeatedly; the load waits for the skipping to stop.
    Burst,
    /// The queue moved on by itself, so a gapless engine keeps the tail of the last track.
    Segue,
    /// The provider turned the last load down for now and the same track is asked for again.
    Retry,
}

impl Start {
    fn debounce(self) -> Duration {
        match self {
            Self::Burst => SKIP_DEBOUNCE,
            Self::Pick | Self::Skip | Self::Segue | Self::Retry => Duration::ZERO,
        }
    }
}

/// What the user asked for last. It moves the moment they act, while `PlaybackState` waits for
/// the engine to confirm, so a control can follow the intent without waiting on the network.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    Play,
    Pause,
}

/// A track the engine was asked to fetch ahead, and whether it was also lined up to follow the
/// current one gaplessly.
struct Preloaded {
    id: String,
    segue: bool,
}

/// The position shown between the engine's reports. It runs on wall time from `reset` and is
/// nudged toward each report by `correct`, spread over a moment so the progress bar and the
/// lyrics glide instead of stepping. Parked, it holds `base`.
struct LiveClock {
    base: Duration,
    since: Option<Instant>,
    /// Seconds still to fold in from the last report, negative when the clock was ahead.
    correction: f64,
    /// Seconds over which the correction is folded in.
    settle: f64,
}

impl LiveClock {
    fn new() -> Self {
        Self {
            base: Duration::ZERO,
            since: None,
            correction: 0.,
            settle: CLOCK_SETTLE.as_secs_f64(),
        }
    }

    /// Puts the clock at `at`, running from now or parked there.
    fn reset(&mut self, at: Duration, running: bool) {
        self.base = at;
        self.since = running.then(Instant::now);
        self.correction = 0.;
        self.settle = CLOCK_SETTLE.as_secs_f64();
    }

    /// Folds a reported position in without a jump.
    fn correct(&mut self, toward: Duration) {
        self.correct_at(toward, Instant::now());
    }

    fn correct_at(&mut self, toward: Duration, now: Instant) {
        let shown = self.at(now);
        self.base = shown;
        self.since = Some(now);
        self.correction = signed_gap(toward, shown);
        // A negative correction is spread over longer than the discrepancy itself, which keeps
        // the clock moving forward while it converges instead of ever snapping back.
        self.settle = CLOCK_SETTLE.as_secs_f64() + self.correction.abs();
    }

    fn now(&self) -> Duration {
        self.at(Instant::now())
    }

    fn at(&self, now: Instant) -> Duration {
        let Some(since) = self.since else {
            return self.base;
        };
        let elapsed = now.saturating_duration_since(since).as_secs_f64();
        let blend = (elapsed / self.settle).clamp(0., 1.);
        shifted(self.base, elapsed + self.correction * blend)
    }
}

/// Seconds from `from` to `to`, negative when `to` is earlier.
fn signed_gap(to: Duration, from: Duration) -> f64 {
    match to >= from {
        true => (to - from).as_secs_f64(),
        false => -(from - to).as_secs_f64(),
    }
}

/// `base` moved by `seconds`, never below zero.
fn shifted(base: Duration, seconds: f64) -> Duration {
    match seconds >= 0. {
        true => base.saturating_add(Duration::from_secs_f64(seconds)),
        false => base.saturating_sub(Duration::from_secs_f64(-seconds)),
    }
}

/// The linear gain for a volume level, on a decibel taper that spans `TAPER_DB` and is silent
/// at zero.
fn gain(level: f32) -> f32 {
    match level.clamp(0., 1.) {
        level if level <= 0. => 0.,
        level => 10f32.powf(TAPER_DB * (level - 1.) / 20.),
    }
}

/// What the engine has confirmed. `Playing` means audio is reaching the output; a track being
/// fetched, or a seek still buffering, reads `Loading`. What the user asked for lives in
/// `Intent` and can run ahead of this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Idle,
    Playing,
    Paused,
    Loading,
    /// Nothing plays and this is why, in developer English.
    Failed(String),
}

/// Why a streaming track cannot play while the network is gone. Each reason is the Fluent key
/// of the toast that says so, and a row that cannot play shows the same words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreachable {
    /// The track is downloaded, but the named provider opens its downloads only online, as
    /// Apple Music does.
    Download(String),
    /// Offline mode keeps Sonora from streaming.
    OfflineMode,
    /// The network is gone.
    Offline,
}

impl Unreachable {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Download(_) => "toast-offline-unsupported",
            Self::OfflineMode => "toast-offline-mode",
            Self::Offline => "toast-offline",
        }
    }

    /// The provider the message names, for the `name` argument of its key.
    pub fn provider(&self) -> Option<&str> {
        match self {
            Self::Download(provider) => Some(provider),
            Self::OfflineMode | Self::Offline => None,
        }
    }

    fn toast(&self, cx: &mut Cx) {
        let toast = Toast::new(Outcome::Failed, self.key());
        cx.emit(Event::Toast(match self.provider() {
            Some(provider) => toast.name(provider.to_owned()),
            None => toast,
        }));
    }
}

/// What other models hear from `Playback`: history records a start, the sheet closes on an
/// end, and scrobbling tells the provider about every pause and seek.
#[derive(Clone, Debug)]
pub enum PlaybackEvent {
    StartedPlayback,
    EndedPlayback,
    Paused,
    /// The engine landed a seek, playing or paused.
    Seeked,
}

/// A one-shot request to pause after wall-clock time or when the current track ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sleep {
    After(Duration),
    EndOfTrack,
}

/// What kind of collection a queued track came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Whence {
    Album,
    Playlist,
    Artist,
    Radio,
    Saved,
    Local,
}

/// The collection a queued track came from. While one of its tracks plays, its page shows a pause
/// button and the queue names it. Two origins are the same collection when kind and id match, and
/// the name is only for display.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Origin {
    pub whence: Whence,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl PartialEq for Origin {
    fn eq(&self, other: &Self) -> bool {
        self.whence == other.whence && self.id == other.id
    }
}

impl Eq for Origin {}

impl Origin {
    pub fn album(id: impl Into<String>) -> Self {
        Self::of(Whence::Album, id)
    }

    pub fn playlist(id: impl Into<String>) -> Self {
        Self::of(Whence::Playlist, id)
    }

    pub fn artist(id: impl Into<String>) -> Self {
        Self::of(Whence::Artist, id)
    }

    pub fn radio(id: impl Into<String>) -> Self {
        Self::of(Whence::Radio, id)
    }

    pub fn saved() -> Self {
        Self::of(Whence::Saved, String::new())
    }

    pub fn local() -> Self {
        Self::of(Whence::Local, String::new())
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Loads the collection once for playback or queue placement, retaining radio continuation.
    pub(crate) async fn load(&self, client: &dyn MusicApi) -> Result<(Vec<Track>, Option<String>)> {
        let tracks = match self.whence {
            Whence::Album => client.album_tracks(&self.id).await?,
            Whence::Playlist => client.playlist_tracks(&self.id).await?,
            Whence::Artist => client.artist_tracks(&self.id).await?,
            Whence::Saved => client.saved_tracks().await?,
            Whence::Radio => {
                let (mut tracks, next) = client.track_radio(&self.id, None).await?;
                tracks.retain(|track| track.playable);
                return Ok((tracks, next));
            }
            Whence::Local => anyhow::bail!("local collections are supplied by the library"),
        };
        Ok((tracks, None))
    }

    fn of(whence: Whence, id: impl Into<String>) -> Self {
        Self {
            whence,
            id: id.into(),
            name: None,
        }
    }
}

impl From<&Pin> for Origin {
    fn from(pin: &Pin) -> Self {
        let origin = match pin.kind {
            PinKind::Album => Origin::album(pin.id.clone()),
            PinKind::Playlist => Origin::playlist(pin.id.clone()),
            PinKind::Artist => Origin::artist(pin.id.clone()),
            PinKind::Song => Origin::radio(pin.id.clone()),
        };
        origin.named(pin.title.clone())
    }
}

/// A station the queue was started from, and the continuation its next stretch is fetched with.
#[derive(Clone, PartialEq)]
struct Station {
    origin: Origin,
    next: String,
}

/// A file handed in from the OS that could not be read as a track.
struct Unread {
    name: String,
    /// What the system said when the file was looked up, or `None` when the file is there but
    /// is not audio Sonora can read.
    error: Option<String>,
}

impl Unread {
    /// Names the file at `path` and keeps the error if it could not be reached at all.
    fn of(path: &Path) -> Self {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let error = std::fs::metadata(path).err().map(|error| error.to_string());
        Self { name, error }
    }
}

/// The models playback reads and moves along with its own state, borrowed off the store for one
/// call. Playback holds none of them; every method that needs one is handed these, and
/// [`Store::playback_parts`] splits them off.
pub(crate) struct Peers<'a> {
    pub(crate) queue: &'a mut Queue,
    pub(crate) session: &'a mut Session,
    pub(crate) settings: &'a mut AppSettings,
    /// The device network, asked whether another device has playback to steer.
    pub(crate) devices: &'a mut Devices,
    pub(crate) network: &'a Network,
    pub(crate) downloads: &'a Downloads,
    pub(crate) drm: &'a Drm,
    pub(crate) cx: &'a mut Cx,
}

/// Everything the UI knows about what is playing, and the only thing that drives an engine.
/// Holds one engine for the streaming provider and one for local files, and a clock for the
/// position between the engine's reports; what comes next is the store's `Queue`. Views act
/// through the [`Store`] methods at the bottom of this file; the engine answers through
/// `on_backend_event`; nothing else touches a `Player`.
pub struct Playback {
    state: PlaybackState,
    /// The last position the engine reported, or the one asked of it.
    position: Duration,
    clock: LiveClock,
    track: Option<Track>,
    engine: Option<Box<dyn Player>>,
    local_engine: Option<Box<dyn Player>>,
    level: f32,
    /// The level the volume slider is being dragged to, heard and shown but not yet saved.
    sliding: Option<f32>,
    normalisation: bool,
    gapless: bool,
    transcode: Transcode,
    quality: Quality,
    /// Whether tracks that played are cached in `vault`, so playing one again downloads nothing.
    keep_tracks: bool,
    /// How many gigabytes the cache in `vault` may take.
    cache_gb: u32,
    /// Where tracks are kept, the downloads and the cache, shared with every engine.
    vault: Arc<Vault>,
    /// Reads the vault's master key from the keychain. The vault keeps nothing until it ends.
    _unlock: Job,
    /// Evicts what no longer fits after the cache limit came down.
    resize: Option<Job>,
    /// What the cache and the downloads take on disk, in bytes, as last measured.
    usage: Option<(u64, u64)>,
    /// Measures the vault, or clears its cache and then measures it.
    measuring: Option<Job>,
    /// The `state::Equalizer`'s reader, shared with every engine started here, so a change
    /// reaches the output without a restart.
    equalizer: music::Equalizer,
    /// Shared with every engine like the equalizer's reader, and read only while watched.
    meter: Meter,
    /// Shared with every engine like the meter, so the equalizer screen can hear the music ahead
    /// of the equalizer.
    analyzer: Analyzer,
    /// What the engines reported about the sound of the last few tracks they opened, newest
    /// last, for the signal path.
    signals: VecDeque<(String, TrackSignal)>,
    repeat: Repeat,
    radio: bool,
    /// The track the current similar-tracks suggestions were drawn from.
    seeded: Option<String>,
    /// The station the queue is playing, while the provider has more of it.
    station: Option<Station>,
    /// The fetch of the station's next stretch; done once it lands, so one runs at a time.
    topping: Option<Job>,
    /// Continuations in a row that brought no track the queue had not already heard.
    dry: u8,
    /// The streaming engine's event pump; dropping it stops listening.
    task: Option<Job>,
    local_task: Option<Job>,
    /// The pending load, held through its debounce; a newer load cancels it.
    load: Option<Job>,
    /// The collection being fetched to start or extend the queue.
    fetch: Option<Job>,
    enqueue: Option<Job>,
    suggest: Option<Job>,
    /// The track the engine was asked to fetch ahead, so it is asked once.
    preloaded: Option<Preloaded>,
    /// When the user last skipped, for telling a burst of skips from one.
    skipped: Option<Instant>,
    /// No load goes out before this after a track failed, so a bad key cannot be hammered.
    blocked_until: Option<Instant>,
    /// Loads that failed since audio last played or the user last chose a track.
    failures: u8,
    /// Loads the provider turned down for now since audio last played or the user last picked a
    /// track. It sets how long the next retry waits.
    throttles: u8,
    /// When the engine last reported a pause. It is wall-clock time so a suspend counts toward
    /// `STALE_PAUSE`, and a clock set back counts as stale.
    paused_at: Option<SystemTime>,
    refused: Option<Refusal>,
    /// Where the restored track resumes. Set until the engine has it ready or the user plays.
    resume_at: Option<Duration>,
    /// The seek the engine is carrying out while playing. One is in flight at a time: the clock
    /// waits at its target until the engine reports audio from there, and position reports
    /// from before it are ignored.
    seek_in_flight: Option<Duration>,
    /// The latest seek asked for while one was in flight. It goes out when that one lands, so
    /// a burst of seeks costs the engine two rather than one per click.
    seek_next: Option<Duration>,
    /// Position reports seen while a seek is in flight.
    stale_positions: u8,
    /// Where the last seek asked to go, until the engine reports where it landed.
    seek_target: Option<Duration>,
    intent: Intent,
    /// When the user last pressed play or pause, until the engine reports the state they asked
    /// for. Engines answer each press in order, so a quick pause and play gets a late `Paused`
    /// after the play, which must not overwrite `intent`.
    pressed: Option<Instant>,
    /// Whether the engine holds the restored track paused at `resume_at`, so play is instant.
    resume_ready: bool,
    /// Whether playback stopped on a stale session and continues once it reconnects.
    awaiting_reconnect: bool,
    /// The position last written to settings for resuming.
    stored: Duration,
    sleep: Option<Sleep>,
    sleep_task: Option<Job>,
    open: Option<Job>,
    /// Whether the call being carried out is this app's own doing, which never steers.
    local: bool,
    /// How many seeks the listener asked for, which a Sonora Session watches to share them. Seeks
    /// made for the session itself, through `locally`, are not counted.
    seeks: u64,
}

impl Playback {
    pub fn new(
        session: &Session,
        settings: &AppSettings,
        equalizer: &Equalizer,
        cx: &mut Cx,
    ) -> Self {
        let level = settings.volume();
        let normalisation = settings.normalisation();
        let gapless = settings.gapless();
        let transcode = settings.transcode();
        let quality = settings.quality();
        let keep_tracks = settings.keep_tracks();
        let cache_gb = settings.cache_gb();
        let vault = Arc::new(Vault::standard(crate::vault::bytes(cache_gb)));
        vault.set_caching(keep_tracks);
        let unlock = crate::vault::unlock(vault.clone(), cx);
        let (equalizer, meter) = (equalizer.reader(), equalizer.meter());
        let repeat = settings.repeat();
        let radio = settings.radio();

        let mut playback = Self {
            state: PlaybackState::Idle,
            position: Duration::ZERO,
            clock: LiveClock::new(),
            track: None,
            engine: None,
            local_engine: None,
            level,
            sliding: None,
            normalisation,
            gapless,
            transcode,
            quality,
            keep_tracks,
            cache_gb,
            vault,
            _unlock: unlock,
            resize: None,
            usage: None,
            measuring: None,
            equalizer,
            meter,
            analyzer: Analyzer::default(),
            signals: VecDeque::new(),
            repeat,
            radio,
            seeded: None,
            station: None,
            topping: None,
            dry: 0,
            task: None,
            local_task: None,
            load: None,
            fetch: None,
            enqueue: None,
            suggest: None,
            preloaded: None,
            skipped: None,
            blocked_until: None,
            failures: 0,
            throttles: 0,
            paused_at: None,
            refused: None,
            resume_at: None,
            seek_in_flight: None,
            seek_next: None,
            stale_positions: 0,
            seek_target: None,
            intent: Intent::Pause,
            pressed: None,
            resume_ready: false,
            awaiting_reconnect: false,
            stored: Duration::ZERO,
            sleep: None,
            sleep_task: None,
            open: None,
            local: false,
            seeks: 0,
        };
        // Start the local engine at startup so files can play before the first scan.
        if let Some(factory) = session.local_playback() {
            playback.start_local_engine(factory, cx);
        }
        playback
    }

    /// What a session event means for the engines: a sign-in builds the streaming engine and
    /// restores the last queue, a reconnect moves onto a fresh one, a sign-out drops it.
    fn on_session(&mut self, event: &SessionEvent, peers: &mut Peers) {
        match event {
            SessionEvent::SignedIn => {
                let Some(playback) = peers.session.playback() else {
                    return;
                };
                // an engine left from playing offline hands its track over where it stands
                match self.engine.is_some() && !self.local_active() {
                    true => self.rebind(playback, peers),
                    false => self.start_engine(playback, peers.cx),
                }
                self.adopt(peers);
            }
            SessionEvent::Reconnected => {
                let Some(playback) = peers.session.playback() else {
                    return;
                };
                self.rebind(playback, peers);
            }
            SessionEvent::SignedOut => self.teardown(peers.cx),
            // The online engine gives way to the one that plays downloads, picking the current
            // track up where it was. A provider with none stops streaming altogether.
            SessionEvent::WentOffline => match peers.session.playback() {
                Some(playback) if self.local_active() => self.start_engine(playback, peers.cx),
                Some(playback) => self.rebind(playback, peers),
                None => self.teardown(peers.cx),
            },
            SessionEvent::LocalChanged => {
                if self.local_engine.is_none()
                    && let Some(playback) = peers.session.local_playback()
                {
                    self.start_local_engine(playback, peers.cx);
                }
            }
        }
    }

    /// An account out of reach still plays its downloads, through an engine the session builds
    /// from the stored sign-in alone.
    fn follow_session(&mut self, session: &Session, cx: &mut Cx) {
        if self.engine.is_none()
            && matches!(session.state(), SessionState::Offline(_))
            && let Some(playback) = session.playback()
        {
            self.start_engine(playback, cx);
        }
    }

    /// The other device that has playback, when one does and a user action is to reach it
    /// instead of the engine here. `None` while this app acts on its own engine, through
    /// [`Store::locally`].
    pub(crate) fn steering(&self, devices: &Devices) -> Option<Elsewhere> {
        if self.local {
            return None;
        }
        devices.steered()
    }

    /// The other device that has playback, which is shown in place of this app's own track.
    pub fn steered(&self, devices: &Devices) -> Option<Elsewhere> {
        self.steering(devices)
    }

    /// The track the player shows: the other device's while one is steered, else this app's own.
    pub fn shown_track(&self, devices: &Devices) -> Option<Track> {
        match self.steering(devices) {
            Some(_) => devices.queue_tracks(crate::QueuePosition::Current),
            None => self.track.clone(),
        }
    }

    /// How far into the shown track playback is, and how long that track is.
    pub fn shown_time(&self, devices: &Devices) -> (Duration, Duration) {
        match self.steered(devices) {
            Some(remote) => {
                let duration = match remote.state.duration.is_zero() {
                    true => self
                        .shown_track(devices)
                        .map_or(Duration::ZERO, |track| track.duration),
                    false => remote.state.duration,
                };
                (remote.state.live_position(), duration)
            }
            None => (
                self.position,
                self.track
                    .as_ref()
                    .map_or(Duration::ZERO, |track| track.duration),
            ),
        }
    }

    /// The volume the controls show and change, on the other device while one is steered.
    pub fn shown_volume(&self, devices: &Devices) -> f32 {
        if let Some(level) = self.sliding {
            return level;
        }
        self.steered(devices)
            .map_or(self.level, |steered| steered.device.volume)
    }

    /// Sends the command `make` builds to the other device that has playback, if one does, and
    /// says whether it did. A caller that got `true` leaves its own engine and queue alone, except
    /// that a `Start` the device refuses is played here once it comes back as an event.
    fn steer(&self, make: impl FnOnce(&Elsewhere) -> Command, devices: &Devices) -> bool {
        let Some(steered) = self.steering(devices) else {
            return false;
        };
        devices.control(&steered.device.id, make(&steered));
        true
    }

    /// Has the other device that has playback start the track at `index` of `tracks`, from the
    /// collection `origin` names, and says whether it did. A local file has no place there and
    /// plays here.
    fn hand_over(
        &self,
        tracks: &[Track],
        index: usize,
        origin: Option<&Origin>,
        devices: &Devices,
    ) -> bool {
        let Some(id) = tracks
            .get(index)
            .and_then(|track| track.id.clone())
            .filter(|id| !music::is_local_id(id))
        else {
            return false;
        };
        self.steer(
            |_| {
                Command::Start(connect::Start {
                    state: Arc::new(connect::Snapshot {
                        context: origin.and_then(|origin| match origin.whence {
                            Whence::Radio => Some(connect::Collection::Radio(origin.id.clone())),
                            _ => collection_of(origin),
                        }),
                        track: Some(connect::QueuedTrack::new(id, false)),
                        playing: true,
                        ..Default::default()
                    }),
                    restart: true,
                    ..Default::default()
                })
            },
            devices,
        )
    }

    /// Plays a track the user picked, from its start.
    fn play(&mut self, track: &Track, peers: &mut Peers) {
        self.load_after(track, Start::Pick, peers);
    }

    /// The engine a track id belongs to: local files have their own.
    fn engine_for(&self, id: &str) -> Option<&dyn Player> {
        match music::is_local_id(id) {
            true => self.local_engine.as_deref(),
            false => self.engine.as_deref(),
        }
    }

    fn active_engine(&self) -> Option<&dyn Player> {
        let id = self.track.as_ref()?.id.as_deref()?;
        self.engine_for(id)
    }

    /// The spectrum of the music on its way into the equalizer, on every output.
    pub fn analyzer(&self) -> &Analyzer {
        &self.analyzer
    }

    /// The playing engine's spectrum, set to hear the track before or after the volume as the
    /// visualizer setting asks. Unavailable while another device has playback.
    pub fn spectrum(&self, devices: &Devices, settings: &AppSettings) -> Option<Spectrum> {
        if self.steered(devices).is_some() {
            return None;
        }
        let spectrum = self.active_engine()?.spectrum()?;
        spectrum.set_absolute(settings.visualizer_absolute());
        Some(spectrum)
    }

    /// For the signal path, `None` while no engine is up for the track loaded now. Otherwise
    /// what its engine reported about the track's sound, `None` inside until it opened it.
    pub fn track_signal(&self) -> Option<Option<&TrackSignal>> {
        let id = self.track.as_ref()?.id.as_deref()?;
        self.engine_for(id)?;
        Some(
            self.signals
                .iter()
                .rev()
                .find(|(known, _)| known == id)
                .map(|(_, track)| track),
        )
    }

    /// Keeps what an engine reported about the sound of track `id`, in place of any older
    /// report for it.
    fn learn_signal(&mut self, id: String, track: TrackSignal, cx: &mut Cx) {
        self.signals.retain(|(known, _)| *known != id);
        if self.signals.len() == SIGNALS {
            self.signals.pop_front();
        }
        self.signals.push_back((id, track));
        cx.changed(Topic::Playback);
    }

    /// Pauses the engine the new track does not belong to, so the two never sound at once.
    fn silence_other(&self, id: &str) {
        let other = match music::is_local_id(id) {
            true => self.engine.as_deref(),
            false => self.local_engine.as_deref(),
        };
        if let Some(engine) = other {
            engine.pause();
        }
    }

    /// Fetches a track ahead, as when the pointer rests on its row, so playing it starts at
    /// once. Asked once per track; the current track is never preloaded.
    pub fn preload(&mut self, track: &Track) {
        self.preload_internal(track, false);
    }

    fn preload_internal(&mut self, track: &Track, segue: bool) {
        let Some(id) = track.id.as_deref() else {
            return;
        };
        if self.engine_for(id).is_none() {
            return;
        }
        if !track.playable || self.track.as_ref().and_then(|track| track.id.as_deref()) == Some(id)
        {
            return;
        }
        if self
            .preloaded
            .as_ref()
            .is_some_and(|held| held.id == id && (held.segue || !segue))
        {
            return;
        }
        self.preloaded = Some(Preloaded {
            id: id.to_owned(),
            segue,
        });
        let Some(engine) = self.engine_for(id) else {
            return;
        };
        if let Err(error) = engine.preload(id, segue) {
            self.preloaded = None;
            log::warn!("playback: cannot preload {}: {error:#}", track.name);
        }
    }

    fn load_after(&mut self, track: &Track, start: Start, peers: &mut Peers) {
        self.load_from(track, Duration::ZERO, start, peers);
    }

    /// Loads a track to play from `at`, after the debounce `start` asks for. The state reads
    /// Loading from here until the engine reports audio; a refusal or an unplayable track fails
    /// without reaching the engine.
    fn load_from(&mut self, track: &Track, at: Duration, start: Start, peers: &mut Peers) {
        let Some(id) = track.id.clone() else {
            return self.failed(format!("{} has no track id", track.name), peers.cx);
        };
        let is_local = music::is_local_id(&id);
        if !is_local {
            match self.refused {
                Some(Refusal::Keys) => return self.refuse(peers.cx),
                Some(Refusal::SignIn) => return self.gate(peers),
                None => {}
            }
        }
        if !track.playable {
            return self.failed(
                format!("{} is not available to stream", track.name),
                peers.cx,
            );
        }
        if let Some(reason) = self.reason(&id, peers.network, peers.downloads, peers.session) {
            return Self::unreachable(reason, start, peers.cx);
        }
        if self.engine_for(&id).is_none() {
            return;
        }
        self.silence_other(&id);
        if !matches!(start, Start::Segue | Start::Retry) {
            self.failures = 0;
        }
        if start == Start::Pick {
            self.throttles = 0;
        }

        self.track = Some(track.clone());
        self.state = PlaybackState::Loading;
        self.position = at;
        self.clock.reset(at, false);
        self.preloaded = None;
        self.resume_at = None;
        self.seek_in_flight = None;
        self.seek_next = None;
        self.seek_target = None;
        self.intent = Intent::Play;
        self.resume_ready = false;
        peers.cx.changed(Topic::Playback);

        let wait = self
            .blocked_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
            .unwrap_or_default()
            .max(start.debounce());

        self.load = Some(peers.cx.spawn(move |link| async move {
            tokio::time::sleep(wait).await;
            link.post(move |store| {
                let (this, peers) = store.playback_parts();
                if this.intent == Intent::Pause {
                    return this.hold(at, peers.cx);
                }
                let Some(engine) = this.engine_for(&id) else {
                    return;
                };
                if let Err(error) = engine.load(&id, at, start == Start::Segue) {
                    let reason = crate::blamed(&error, peers.cx);
                    this.failed(reason, peers.cx);
                }
            });
        }));
    }

    /// Replaces the queue with `tracks` and plays the one at `index`.
    fn start(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        peers: &mut Peers,
    ) {
        self.fetch = None;
        self.begin(tracks, index, origin, peers);
    }

    /// Replaces the queue with `tracks` and plays the one at `index` here from `at`, as another
    /// device handed it over. A seek after `start` would be lost while the track loads.
    fn start_at(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        at: Duration,
        peers: &mut Peers,
    ) {
        self.fetch = None;
        let Some(track) = peers.queue.start(tracks, index, origin, peers.cx) else {
            return;
        };
        self.leave_station();
        self.load_from(&track, at, Start::Pick, peers);
    }

    /// Replaces the queue with `tracks` and plays the first playable one, or a random one when
    /// shuffle is on.
    fn start_any(&mut self, tracks: Vec<Track>, origin: Option<Origin>, peers: &mut Peers) {
        let index = self.opener(&tracks, peers).unwrap_or_default();
        self.start(tracks, index, origin, peers);
    }

    /// Turn shuffle on and start the tracks from a random playable one.
    fn shuffle_any(&mut self, tracks: Vec<Track>, origin: Option<Origin>, peers: &mut Peers) {
        peers.queue.set_shuffle(true, peers.cx);
        self.start_any(tracks, origin, peers);
    }

    /// Which of `tracks` a collection opens with.
    fn opener(&self, tracks: &[Track], peers: &Peers) -> Option<usize> {
        let playable = tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                track.playable
                    && self
                        .unreachable_reason(track, peers.network, peers.downloads, peers.session)
                        .is_none()
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match peers.queue.shuffle() {
            true => fastrand::choice(&playable).copied(),
            false => playable.first().copied(),
        }
    }

    /// Plays `seed` now and fills the queue behind it with its radio once that arrives.
    fn play_radio(&mut self, seed: &Track, peers: &mut Peers) {
        let Some(id) = seed.id.clone() else {
            return self.failed(format!("{} has no track id", seed.name), peers.cx);
        };
        if !seed.playable {
            return self.failed(
                format!("{} is not available to stream", seed.name),
                peers.cx,
            );
        }

        let origin = Origin::radio(id.clone()).named(seed.name.clone());
        let Some(client) = self.client_for(&id, peers.session) else {
            return;
        };

        self.fetch = None;
        if !self.begin(vec![seed.clone()], 0, Some(origin.clone()), peers) {
            return;
        }

        let seed_id = seed.id.clone();
        let title = seed.name.clone();
        self.fetch = Some(peers.cx.load(
            async move {
                let (mut tracks, next) = client.track_radio(&id, None).await?;
                tracks.retain(|track| track.id != seed_id && track.playable);
                Ok((tracks, next))
            },
            move |store, loaded| {
                let (this, peers) = store.playback_parts();
                match loaded {
                    Ok((tracks, next)) if this.origin(peers.queue) == Some(&origin) => {
                        this.station = next.map(|next| Station {
                            origin: origin.clone(),
                            next,
                        });
                        peers.queue.extend_context(tracks, origin, peers.cx);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        log::error!("playback: cannot load radio queue: {error:#}");
                        crate::noted(&error, peers.cx);
                        if this.origin(peers.queue) == Some(&origin) {
                            peers.cx.emit(Event::Toast(
                                Toast::new(Outcome::Failed, "toast-radio-failed").name(title),
                            ));
                        }
                    }
                }
            },
        ));
    }

    /// Places tracks the user picked and says where they went, naming one track and counting
    /// several. Nothing is said when they start playback instead, since playing is the answer.
    fn enqueue_tracks(
        &mut self,
        tracks: Vec<Track>,
        origin: Option<Origin>,
        placement: QueuePlacement,
        peers: &mut Peers,
    ) {
        let count = tracks.len();
        let named = match tracks.as_slice() {
            [only] => Some((only.name.clone(), song_target(only))),
            _ => None,
        };
        if count == 0 {
            return;
        }
        let Some(placed) = self.place(tracks, origin, placement, peers) else {
            return;
        };
        match named {
            Some((name, target)) => {
                if let Some(key) = placed.toast("track") {
                    peers.cx.emit(Event::Toast(
                        Toast::new(Outcome::Done, key).name(name).target(target),
                    ));
                }
            }
            None => {
                if let Some(key) = placed.toast("tracks") {
                    peers
                        .cx
                        .emit(Event::Toast(Toast::new(Outcome::Done, key).count(count)));
                }
            }
        }
    }

    /// Puts tracks into the queue at `placement` without a word, or starts playing them when
    /// nothing is queued. While another device has playback they go to the end of its queue
    /// instead. Returns where they went, for the toast, or `None` when they started playback or
    /// went nowhere.
    fn place(
        &mut self,
        tracks: Vec<Track>,
        origin: Option<Origin>,
        placement: QueuePlacement,
        peers: &mut Peers,
    ) -> Option<QueuePlacement> {
        let playing = peers.queue.current().is_some();
        if tracks.is_empty() {
            return playing.then_some(placement);
        }
        if let Some(sent) = self.queue_elsewhere(&tracks, peers) {
            let placed = match placement {
                QueuePlacement::Gap(gap) => QueuePlacement::Gap(gap),
                _ => QueuePlacement::End,
            };
            return sent.then_some(placed);
        }
        if !playing {
            self.begin(tracks, 0, origin, peers);
            return None;
        }
        // In a Jam a song the listener queues names them, as it does for everyone else. A song
        // the device network queues here already names whoever queued it.
        let you = match (placement, self.local) {
            (QueuePlacement::Last, _) | (_, true) => None,
            _ => peers.devices.you(),
        };
        let tracks = tracks.into_iter().map(|mut track| {
            if track.added_by.is_none() {
                track.added_by = you.clone();
            }
            track
        });
        peers.queue.insert(tracks, origin, placement, peers.cx);
        Some(placement)
    }

    /// Adds tracks to the end of the queue of the other device that has playback, and says
    /// whether all of them went. `None` while no other device is steered. Local files have no
    /// place there, so they stay behind with a toast saying so.
    fn queue_elsewhere(&self, tracks: &[Track], peers: &mut Peers) -> Option<bool> {
        let steered = self.steering(peers.devices)?;
        let mut kept = false;
        for track in tracks {
            match track.id.clone() {
                Some(id) if music::is_local_id(&id) => kept = true,
                Some(id) => peers
                    .devices
                    .control(&steered.device.id, Command::Enqueue { id, added_by: None }),
                None => {}
            }
        }
        if kept {
            peers.cx.emit(Event::Toast(Toast::new(
                Outcome::Info,
                "toast-queue-local-elsewhere",
            )));
        }
        Some(!kept)
    }

    /// Reads the files handed in from the OS on the tokio runtime and plays what it could read.
    /// Files it could not read are reported on a toast, as a failure when none could be read and
    /// as a warning otherwise.
    fn resolve_paths(&mut self, paths: Vec<PathBuf>, peers: &mut Peers) {
        let provider = peers.session.local_provider();
        self.open = Some(peers.cx.load(
            async move {
                let mut tracks = Vec::new();
                let mut unread = Vec::new();
                for path in paths {
                    match provider.track_from_path(&path) {
                        Some(track) => tracks.push(track),
                        None => {
                            log::warn!("local: cannot open {}", path.display());
                            unread.push(Unread::of(&path));
                        }
                    }
                }
                anyhow::Ok((tracks, unread))
            },
            |store, loaded| match loaded {
                Ok((mut tracks, unread)) => {
                    report_unread(&unread, tracks.is_empty(), &mut store.cx);
                    match tracks.len() {
                        0 => {}
                        1 => store.locally(|store| {
                            store.play_next(tracks.remove(0));
                            store.next_track();
                        }),
                        _ => store.enqueue_tracks(tracks, None, QueuePlacement::Next),
                    }
                }
                Err(error) => log::warn!("playback: cannot open files: {error:#}"),
            },
        ));
    }

    fn client_for(&self, id: &str, session: &Session) -> Option<Arc<dyn MusicApi>> {
        match music::is_local_id(id) {
            true => session.local_client(),
            false => session.client(),
        }
    }

    /// Whether the provider that owns `id` can seed a station. Without one there is nothing to
    /// keep a queue going, so the radio paths leave it alone rather than asking and getting
    /// nothing back.
    fn stations(&self, id: &str, session: &Session) -> bool {
        session.capabilities_of(crate::Shelf::of(id)).radio
    }

    /// Fetches tracks for the queue on the tokio runtime and places them on arrival, with a
    /// toast once they land or the fetch fails. One fetch runs at a time, and a second request
    /// while one runs is dropped before it shows anything. The tracks keep `origin` as the
    /// collection they were queued from.
    fn enqueue_from(
        &mut self,
        id: &str,
        origin: Option<Origin>,
        placement: QueuePlacement,
        peers: &mut Peers,
    ) {
        if self.enqueue.is_some() {
            return;
        }
        let Some(client) = self.client_for(id, peers.session) else {
            return;
        };
        let source = match origin.as_ref().map(|origin| origin.whence) {
            Some(Whence::Album) => "album",
            Some(Whence::Playlist) => "playlist",
            Some(Whence::Artist) => "artist",
            _ => "track",
        };
        let id = id.to_owned();
        let from = origin.clone();
        self.enqueue = Some(peers.cx.load(
            async move {
                match from {
                    Some(origin) => origin.load(&*client).await.map(|(tracks, _)| tracks),
                    None => client.track(&id).await.map(|track| vec![track]),
                }
            },
            move |store, loaded| {
                let (this, mut peers) = store.playback_parts();
                this.enqueue = None;
                match loaded {
                    Ok(tracks) => {
                        let placed = this.place(tracks, origin, placement, &mut peers);
                        if let Some(key) = placed.and_then(|placed| placed.toast(source)) {
                            peers.cx.emit(Event::Toast(Toast::new(Outcome::Done, key)));
                        }
                    }
                    Err(error) => {
                        log::error!("playback: cannot enqueue {source}: {error:#}");
                        peers.cx.emit(Event::Toast(Toast::new(
                            Outcome::Failed,
                            "toast-queue-failed",
                        )));
                    }
                }
            },
        ));
    }

    /// The collection the current track was queued from. `None` when the user added it on its
    /// own.
    pub fn origin<'a>(&self, queue: &'a Queue) -> Option<&'a Origin> {
        queue.origin()
    }

    /// What a play button for `origin` shows, read like `control`. `None` when the current track
    /// came from somewhere else, so pressing it starts `origin` afresh.
    pub fn playing_from(&self, origin: &Origin, queue: &Queue) -> Option<bool> {
        match self.origin(queue) == Some(origin) {
            true => self.control(),
            false => None,
        }
    }

    /// Drops the station the queue was playing, continuation and fetch alike, once the queue
    /// is something else.
    fn leave_station(&mut self) {
        self.station = None;
        self.topping = None;
        self.dry = 0;
    }

    /// Hands a fetched collection to the queue, every track tagged with `origin`, and plays the
    /// chosen one. While another device has playback it starts the track there instead and leaves
    /// the queue alone, which is when this returns `false`. A chosen track that cannot play here
    /// while the network is gone leaves the queue alone too, and says why.
    fn begin(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        peers: &mut Peers,
    ) -> bool {
        if self.hand_over(&tracks, index, origin.as_ref(), peers.devices) {
            return false;
        }
        // A track that cannot play here leaves the queue and whatever plays as they were.
        if let Some(reason) = tracks.get(index).and_then(|track| {
            self.unreachable_reason(track, peers.network, peers.downloads, peers.session)
        }) {
            Self::unreachable(reason, Start::Pick, peers.cx);
            return false;
        }
        let Some(track) = peers.queue.start(tracks, index, origin, peers.cx) else {
            return true;
        };
        self.leave_station();
        self.play(&track, peers);
        true
    }

    /// Fetches a collection and starts the queue from it, keeping the continuation if it is a
    /// station. Shows Loading only when nothing plays yet, so a failure mid-playback is logged
    /// rather than shown.
    fn gather(&mut self, origin: Origin, shuffled: bool, peers: &mut Peers) {
        let Some(client) = self.client_for(&origin.id, peers.session) else {
            return;
        };

        if !self.has_active_playback() && self.steering(peers.devices).is_none() {
            self.state = PlaybackState::Loading;
            peers.cx.changed(Topic::Playback);
        }

        let from = origin.clone();
        self.fetch = Some(peers.cx.load(
            async move { from.load(&*client).await },
            move |store, loaded| {
                let (this, mut peers) = store.playback_parts();
                match loaded {
                    Ok((tracks, next)) => {
                        if shuffled && !tracks.is_empty() {
                            peers.queue.set_shuffle(true, peers.cx);
                        }
                        let index = this.opener(&tracks, &peers).unwrap_or_default();
                        let station = next.map(|next| Station {
                            origin: origin.clone(),
                            next,
                        });
                        if this.begin(tracks, index, Some(origin), &mut peers) {
                            this.station = station;
                        }
                    }
                    Err(error) if this.has_active_playback() => {
                        log::error!("playback: cannot load context: {error:#}");
                    }
                    Err(error) => {
                        let reason = crate::blamed(&error, peers.cx);
                        this.failed(reason, peers.cx);
                    }
                }
            },
        ));
    }

    /// Skips to the next playable track.
    fn next(&mut self, peers: &mut Peers) {
        if self.steer(|_| Command::Next, peers.devices) {
            return;
        }
        self.fetch = None;
        let start = self.burst();
        self.follow_queue(start, peers);
    }

    /// Removes tracks from the queue, and moves on when the current one was among them.
    fn remove_from_queue(&mut self, ids: &[String], peers: &mut Peers) {
        let current_removed = peers.queue.remove_tracks(ids, peers.cx);
        if !current_removed {
            return;
        }
        self.pause(peers);
        self.track = None;
        self.next(peers);
    }

    /// Notes a skip and says whether it is part of a burst, which loads only once the skipping
    /// stops.
    fn burst(&mut self) -> Start {
        let now = Instant::now();
        let rapid = self
            .skipped
            .replace(now)
            .is_some_and(|last| now.duration_since(last) < SKIP_DEBOUNCE);
        match rapid {
            true => Start::Burst,
            false => Start::Skip,
        }
    }

    /// Lines the next track up behind the current one once it is near its end, so a gapless
    /// engine can join them. An engine that fetched it already only queues it here.
    fn preload_next(&mut self, position: Duration, queue: &Queue) {
        let Some(duration) = self.track.as_ref().map(|track| track.duration) else {
            return;
        };
        if duration.is_zero()
            || self.state != PlaybackState::Playing
            || duration.saturating_sub(position) > PRELOAD_BEFORE_END
        {
            return;
        }
        self.preload_upcoming(true, queue);
    }

    /// Asks the engine to fetch the track that plays after this one. With `segue` it is also
    /// lined up to follow gaplessly, which waits for the end because the queue can still change.
    fn preload_upcoming(&mut self, segue: bool, queue: &Queue) {
        let next = match self.repeat != Repeat::One {
            true => queue.upcoming().next().cloned(),
            false => None,
        };
        let Some(next) = next else {
            self.preloaded = None;
            return;
        };

        self.preload_internal(&next, segue);
    }

    pub fn radio(&self) -> bool {
        self.radio
    }

    fn toggle_radio(&mut self, peers: &mut Peers) {
        self.radio = !self.radio;
        let radio = self.radio;
        peers.settings.set_radio(radio, peers.cx);
        match radio {
            true => self.suggest_similar(true, peers),
            false => self.forget_similar(peers),
        }
    }

    fn forget_similar(&mut self, peers: &mut Peers) {
        self.seeded = None;
        self.suggest = None;
        peers.queue.clear_similar(peers.cx);
    }

    /// The track the suggestions are drawn from, which is whatever is playing. It is the track
    /// the listener chose and it is playable by definition, so a station is never seeded from
    /// something at the end of the queue that will be skipped for being unavailable.
    fn seed(&self, queue: &Queue) -> Option<Track> {
        queue.current().cloned()
    }

    /// Fetches the station's next stretch once fewer than `RADIO_LOOKAHEAD` tracks are left to
    /// play, so the queue never reaches its end while the provider has more. What the queue
    /// has already heard is left out. The station is the queue itself, so it goes on whether
    /// or not radio suggestions are on; a fetch that fails drops the continuation and hands the
    /// end of the queue back to `advance` through `stranded`.
    fn continue_station(&mut self, peers: &mut Peers) {
        if self.topping.is_some() || peers.queue.ahead() >= RADIO_LOOKAHEAD {
            return;
        }
        let Some(station) = self.station.clone() else {
            return;
        };
        let Some(client) = self.client_for(&station.origin.id, peers.session) else {
            return;
        };

        let heard = peers.queue.ids();
        let seed = station.origin.id.clone();
        let token = station.next.clone();
        self.topping = Some(peers.cx.load(
            async move {
                let (mut tracks, next) = client.track_radio(&seed, Some(&token)).await?;
                unheard(&mut tracks, &heard);
                anyhow::Ok((tracks, next))
            },
            move |store, loaded| {
                let (this, mut peers) = store.playback_parts();
                this.topping = None;
                if this.station.as_ref() != Some(&station) {
                    return;
                }
                match loaded {
                    Ok((tracks, next)) => {
                        this.dry = match tracks.is_empty() {
                            true => this.dry.saturating_add(1),
                            false => 0,
                        };
                        this.station =
                            next.filter(|_| this.dry < STATION_DRY_LIMIT)
                                .map(|next| Station {
                                    origin: station.origin.clone(),
                                    next,
                                });
                        let idle = this.track.is_none() && !peers.queue.has_next();
                        peers
                            .queue
                            .extend_context(tracks, station.origin.clone(), peers.cx);
                        if idle {
                            this.follow_queue(Start::Segue, &mut peers);
                        }
                    }
                    Err(error) => {
                        this.station = None;
                        log::warn!("playback: cannot continue the station: {error:#}");
                    }
                }
                if this.station.is_none() {
                    this.stranded(&mut peers);
                }
            },
        ));
    }

    /// Picks playback back up when the station that was covering the end of the queue stops
    /// answering, by putting the track that ended through `advance` again, which is where radio
    /// takes over. Does nothing while anything is still playing or waiting to.
    fn stranded(&mut self, peers: &mut Peers) {
        if self.track.is_some() || peers.queue.has_next() {
            return;
        }
        let ended = self.seed(peers.queue);
        self.advance(ended, peers);
    }

    /// Fills the suggestions from the current track's radio when radio is on and there are
    /// none, and tops them up once fewer than `RADIO_LOOKAHEAD` tracks are left to play, so the
    /// next batch has arrived long before the queue reaches it. What is already queued is left
    /// out, and the provider's best `SIMILAR_LIMIT` of the rest are kept. A failure is shown
    /// only when `asked`, that is when the user just turned radio on, since a top-up failing
    /// on its own is retried at the next change.
    fn suggest_similar(&mut self, asked: bool, peers: &mut Peers) {
        if !self.radio {
            return;
        }
        let held = peers.queue.similar().count();
        if held > 0 && peers.queue.upcoming().len() >= RADIO_LOOKAHEAD {
            return;
        }
        let Some(seed) = self.seed(peers.queue) else {
            return self.forget_similar(peers);
        };
        let Some(id) = seed.id.clone() else {
            return self.forget_similar(peers);
        };
        if !self.stations(&id, peers.session) {
            return self.forget_similar(peers);
        }
        if self.seeded.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(client) = self.client_for(&id, peers.session) else {
            return self.forget_similar(peers);
        };

        let queued = peers.queue.ids();
        self.seeded = Some(id.clone());
        let title = seed.name.clone();
        let origin = Origin::radio(id.clone()).named(seed.name);
        self.suggest = Some(peers.cx.load(
            async move {
                let (mut tracks, _) = client.track_radio(&id, None).await?;
                unheard(&mut tracks, &queued);
                tracks.truncate(SIMILAR_LIMIT);
                anyhow::Ok(tracks)
            },
            move |store, loaded| {
                let (this, peers) = store.playback_parts();
                match loaded {
                    Ok(_) if !this.radio => {}
                    Ok(tracks) => match held {
                        0 => peers.queue.suggest(tracks, origin, peers.cx),
                        _ => peers.queue.extend_similar(tracks, origin, peers.cx),
                    },
                    Err(error) => {
                        this.seeded = None;
                        log::warn!("playback: cannot load similar tracks: {error:#}");
                        crate::noted(&error, peers.cx);
                        if asked && this.radio {
                            peers.cx.emit(Event::Toast(
                                Toast::new(Outcome::Failed, "toast-similar-failed").name(title),
                            ));
                        }
                    }
                }
            },
        ));
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    /// Switches the repeat mode and remembers it in settings.
    fn set_repeat(&mut self, repeat: Repeat, peers: &mut Peers) {
        if self.steer(|_| Command::Repeat(repeat), peers.devices) {
            return;
        }
        if self.repeat == repeat {
            return;
        }
        self.repeat = repeat;
        peers.settings.set_repeat(repeat, peers.cx);
        peers.cx.changed(Topic::Playback);
    }

    fn cycle_repeat(&mut self, peers: &mut Peers) {
        let repeat = match self.shown_repeat(peers.devices) {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        self.set_repeat(repeat, peers);
    }

    /// A binary on/off flip for surfaces (tray, dock menu) that do not fit the three-way
    /// cycle the player bar's button drives; `One` counts as on and flips straight to `Off`.
    fn toggle_repeat(&mut self, peers: &mut Peers) {
        let repeat = match self.shown_repeat(peers.devices) {
            Repeat::Off => Repeat::All,
            Repeat::All | Repeat::One => Repeat::Off,
        };
        self.set_repeat(repeat, peers);
    }

    /// The repeat setting of the device that has playback, which is this app's own unless
    /// another device is being steered.
    pub fn shown_repeat(&self, devices: &Devices) -> Repeat {
        self.steered(devices).map_or(self.repeat, |remote| {
            remote.state.repeat.unwrap_or_default()
        })
    }

    /// Turns shuffle on or off, on the other device when one has playback.
    fn set_shuffle(&mut self, on: bool, peers: &mut Peers) {
        if self.steer(|_| Command::Shuffle(on), peers.devices) {
            return;
        }
        peers.queue.set_shuffle(on, peers.cx);
    }

    /// Flips shuffle from what the player shows, which is the other device's when it has playback.
    fn toggle_shuffle(&mut self, peers: &mut Peers) {
        let on = !self.shown_shuffle(peers.queue, peers.devices);
        self.set_shuffle(on, peers);
    }

    /// Whether the device that has playback shuffles, which is this app's queue unless another
    /// device is being steered.
    pub fn shown_shuffle(&self, queue: &Queue, devices: &Devices) -> bool {
        self.steered(devices).map_or_else(
            || queue.shuffle(),
            |remote| remote.state.shuffle.unwrap_or_default(),
        )
    }

    /// Decides what follows a track that ended: the same one on repeat-one, the queue's start
    /// on repeat-all, more radio when it is on and the queue ran out, else the next in line.
    fn advance(&mut self, ended: Option<Track>, peers: &mut Peers) {
        match self.repeat {
            Repeat::One => match ended {
                Some(track) => self.load_after(&track, Start::Segue, peers),
                None => self.segue_queue(peers),
            },
            Repeat::All if !peers.queue.has_next() => {
                self.fetch = None;
                if let Some(track) = peers.queue.rewind(peers.cx) {
                    self.follow_after(track, Start::Segue, peers);
                }
            }
            // The station's next stretch is on its way; playing resumes when it lands.
            _ if self.topping.is_some() && !peers.queue.has_next() => {
                self.fetch = None;
            }
            _ if self.radio && !peers.queue.has_next() => {
                let seed = ended.or_else(|| self.track.clone()).filter(|seed| {
                    seed.id
                        .as_deref()
                        .is_some_and(|id| self.stations(id, peers.session))
                });
                match seed {
                    Some(seed) => self.extend_radio(&seed, peers),
                    None => self.segue_queue(peers),
                }
            }
            _ => self.segue_queue(peers),
        }
    }

    fn segue_queue(&mut self, peers: &mut Peers) {
        self.fetch = None;
        self.follow_queue(Start::Segue, peers);
    }

    /// Appends the seed's radio to the queue and moves on, or just moves on if it fails.
    fn extend_radio(&mut self, seed: &Track, peers: &mut Peers) {
        let Some(id) = seed.id.clone() else {
            return self.next(peers);
        };
        let Some(client) = self.client_for(&id, peers.session) else {
            return self.next(peers);
        };

        let origin = Origin::radio(id.clone()).named(seed.name.clone());
        let heard = peers.queue.ids();
        self.fetch = Some(peers.cx.load(
            async move {
                let (mut tracks, _) = client.track_radio(&id, None).await?;
                unheard(&mut tracks, &heard);
                anyhow::Ok(tracks)
            },
            move |store, loaded| {
                let (this, mut peers) = store.playback_parts();
                match loaded {
                    Ok(tracks) if !tracks.is_empty() => {
                        peers
                            .queue
                            .insert(tracks, Some(origin), QueuePlacement::End, peers.cx);
                        this.leave_station();
                        this.follow_queue(Start::Segue, &mut peers);
                    }
                    Ok(_) => log::warn!("playback: radio returned no tracks"),
                    Err(error) => log::warn!("playback: cannot extend radio: {error:#}"),
                }
            },
        ));
    }

    fn follow_queue(&mut self, start: Start, peers: &mut Peers) {
        let Some(track) = self.playable_next(peers) else {
            return;
        };
        self.load_after(&track, start, peers);
    }

    /// The next playable track the queue has, dropping the ones that are not. Reaching the
    /// suggestions means radio has taken over from the station the queue was playing.
    fn playable_next(&mut self, peers: &mut Peers) -> Option<Track> {
        loop {
            let suggested = peers.queue.next_is_suggested();
            let track = peers.queue.next(peers.cx)?;
            if suggested {
                self.leave_station();
            }
            if track.playable {
                return Some(track);
            }
        }
    }

    fn follow_after(&mut self, track: Track, start: Start, peers: &mut Peers) {
        let track = match track.playable {
            true => Some(track),
            false => self.playable_next(peers),
        };
        let Some(track) = track else {
            return;
        };
        self.load_after(&track, start, peers);
    }

    pub fn has_previous(&self, queue: &Queue) -> bool {
        self.track.is_some() || queue.has_previous()
    }

    /// Whether the previous button restarts the track rather than going back: a few seconds
    /// in, or nothing to go back to.
    fn restarts(&self, queue: &Queue) -> bool {
        self.track.is_some() && (self.live_position() > RESTART_WINDOW || !queue.has_previous())
    }

    fn previous(&mut self, peers: &mut Peers) {
        if self.steer(|_| Command::Previous, peers.devices) {
            return;
        }
        if self.restarts(peers.queue) {
            return self.seek(Duration::ZERO, peers);
        }
        self.fetch = None;
        let start = self.burst();
        let Some(track) = peers.queue.previous(peers.cx) else {
            return;
        };
        self.load_after(&track, start, peers);
    }

    /// Selects a stable local occurrence; selecting current toggles playback.
    fn play_queue(&mut self, uid: &str, peers: &mut Peers) {
        use crate::QueuePosition;
        let Some(position) = peers.queue.position(uid) else {
            return;
        };
        if position == QueuePosition::Current {
            return self.toggle_play(peers);
        }
        self.fetch = None;
        let suggested = peers.queue.kind(position) == Some(crate::QueueKind::Suggested);
        let Some(track) = peers.queue.select(position, peers.cx) else {
            return;
        };
        if suggested {
            self.leave_station();
        }
        self.load_after(&track, Start::Pick, peers);
    }

    /// Plays on. A restored track the engine does not hold yet, or one paused for longer than
    /// `STALE_PAUSE`, is loaded afresh at its position.
    fn resume(&mut self, peers: &mut Peers) {
        if self.steer(|_| Command::Play, peers.devices) {
            return;
        }
        self.intent = Intent::Play;
        self.pressed = Some(Instant::now());
        if let Some(at) = self.resume_at {
            if !self.resume_ready {
                return self.reload_and_seek(at, peers);
            }
            self.resume_at = None;
            self.resume_ready = false;
        }
        if self
            .paused_at
            .take()
            .is_some_and(|since| since.elapsed().unwrap_or(STALE_PAUSE) >= STALE_PAUSE)
        {
            log::info!("playback: paused too long to trust the engine, reloading the track");
            return self.reload_and_seek(self.position, peers);
        }
        if let Some(engine) = self.active_engine() {
            engine.play();
            peers.cx.changed(Topic::Playback);
        }
    }

    fn reload_and_seek(&mut self, at: Duration, peers: &mut Peers) {
        let Some(track) = self.track.clone() else {
            return;
        };
        if self.active_engine().is_none() {
            return;
        }
        self.load_from(&track, at, Start::Pick, peers);
    }

    /// Has the engine fetch the restored track and hold it paused at `resume_at`, so the first
    /// play is instant. Until it says so, play loads the track instead.
    fn prepare_resume(&mut self, cx: &mut Cx) {
        self.resume_ready = false;
        let Some(at) = self.resume_at else {
            return;
        };
        let Some(track) = self.track.clone() else {
            return;
        };
        let Some(id) = track.id.as_deref().filter(|_| track.playable) else {
            return;
        };
        let Some(engine) = self.engine_for(id) else {
            return;
        };
        match engine.load_paused_at(id, at) {
            Ok(()) => {
                self.resume_ready = true;
                self.state = PlaybackState::Paused;
                self.position = at;
                self.clock.reset(at, false);
            }
            Err(error) => log::warn!(
                "playback: cannot prepare {} for resume: {error:#}",
                track.name
            ),
        }
        cx.changed(Topic::Playback);
    }

    /// Restores the last session's track and queue from settings, paused where it stopped.
    /// Only when nothing is playing or queued, and only for the provider that saved it.
    fn adopt(&mut self, peers: &mut Peers) {
        if self.track.is_some() || !peers.queue.is_empty() {
            return;
        }
        let Some(slug) = peers.session.provider_slug() else {
            return;
        };
        let Some(resume) = peers
            .settings
            .resume()
            .filter(|resume| resume.provider == slug)
            .cloned()
        else {
            return;
        };

        let at = Duration::from_secs_f32(resume.position.max(0.));
        let Some(track) = peers.queue.revive(resume, peers.cx) else {
            return;
        };
        self.track = Some(track);
        self.state = PlaybackState::Paused;
        self.position = at;
        self.clock.reset(at, false);
        self.stored = at;
        self.resume_at = Some(at);
        self.prepare_resume(peers.cx);
    }

    /// Writes the position to settings for resuming, every few seconds or when `force`d.
    fn remember(&mut self, force: bool, settings: &mut AppSettings, cx: &mut Cx) {
        let position = self.position;
        if !force && position.abs_diff(self.stored) < RESUME_STEP {
            return;
        }
        self.stored = position;
        let seconds = position.as_secs_f32();
        settings.set_resume_position(seconds, cx);
    }

    fn pause(&mut self, peers: &mut Peers) {
        if self.steer(|_| Command::Pause, peers.devices) {
            return;
        }
        self.intent = Intent::Pause;
        self.pressed = Some(Instant::now());
        if let Some(engine) = self.active_engine() {
            engine.pause();
            peers.cx.changed(Topic::Playback);
        }
    }

    fn toggle_play(&mut self, peers: &mut Peers) {
        let command = |remote: &Elsewhere| match remote.state.playing {
            true => Command::Pause,
            false => Command::Play,
        };
        if self.steer(command, peers.devices) {
            return;
        }
        match self.wants_playing() {
            true => self.pause(peers),
            false => self.resume(peers),
        }
    }

    pub fn sleep(&self) -> Option<Sleep> {
        self.sleep
    }

    /// Replaces the active sleep request, dropping its timer job when cancelled or superseded.
    /// A timer that runs out while music plays pauses it and says so on a toast.
    pub fn set_sleep(&mut self, sleep: Option<Sleep>, cx: &mut Cx) {
        self.sleep = sleep;
        self.sleep_task = match sleep {
            Some(Sleep::After(after)) => Some(cx.after(after, |store| {
                let (this, mut peers) = store.playback_parts();
                this.sleep = None;
                if this.wants_playing() {
                    peers
                        .cx
                        .emit(Event::Toast(Toast::new(Outcome::Info, "toast-sleep-ended")));
                }
                this.pause(&mut peers);
            })),
            _ => None,
        };
    }

    /// Ends an end-of-track sleep request once its track has ended, holding the next track
    /// paused, and says so on a toast.
    fn doze(&mut self, peers: &mut Peers) {
        peers
            .cx
            .emit(Event::Toast(Toast::new(Outcome::Info, "toast-sleep-ended")));
        self.sleep = None;
        self.sleep_task = None;
        self.fetch = None;
        let Some(track) = self.playable_next(peers) else {
            return;
        };
        self.track = Some(track);
        self.resume_at = Some(Duration::ZERO);
        self.remember(true, peers.settings, peers.cx);
        self.prepare_resume(peers.cx);
    }

    /// Moves to the next playable track and holds it paused without fetching it, so play loads
    /// it then. Leaves nothing loaded when the queue has run out.
    fn hold_next(&mut self, peers: &mut Peers) {
        self.fetch = None;
        self.track = self.playable_next(peers);
        if self.track.is_some() {
            self.hold(Duration::ZERO, peers.cx);
            self.remember(true, peers.settings, peers.cx);
        }
    }

    /// Keeps the current track paused at `at` without asking the engine for it, so play loads
    /// it from there.
    fn hold(&mut self, at: Duration, cx: &mut Cx) {
        self.intent = Intent::Pause;
        self.state = PlaybackState::Paused;
        self.position = at;
        self.clock.reset(at, false);
        self.resume_at = Some(at);
        self.resume_ready = false;
        cx.changed(Topic::Playback);
    }

    /// Whether the user wants the current track playing, whatever the engine has managed so
    /// far. A transport control reads this; anything that follows the sound reads `state`.
    /// Whether the engine reporting `reported` answers a press the user has since reversed.
    fn overtaken(&self, reported: Intent) -> bool {
        reported != self.intent
            && self
                .pressed
                .is_some_and(|pressed| pressed.elapsed() < ECHO_WINDOW)
    }

    pub fn wants_playing(&self) -> bool {
        self.intent == Intent::Play && self.track.is_some()
    }

    /// What a play button for the current track shows. `Some(true)` is pause and `Some(false)`
    /// is play, and pressing either goes to `toggle_play`. `None` means nothing is loaded that a
    /// press could resume, so the button starts its tracks instead.
    pub fn control(&self) -> Option<bool> {
        self.has_active_playback().then(|| self.wants_playing())
    }

    fn play_origin(&mut self, origin: Origin, peers: &mut Peers) {
        match origin.whence {
            Whence::Album | Whence::Playlist | Whence::Artist | Whence::Radio => {
                self.gather(origin, false, peers)
            }
            // the table plays these
            Whence::Saved | Whence::Local => {}
        }
    }

    /// Turns shuffle on and plays `origin` from a random track once its tracks are loaded.
    fn shuffle_origin(&mut self, origin: Origin, peers: &mut Peers) {
        if matches!(
            origin.whence,
            Whence::Album | Whence::Playlist | Whence::Artist | Whence::Radio
        ) {
            self.gather(origin, true, peers);
        }
    }

    /// The play button of a collection page: pauses or resumes when the queue came from it,
    /// starts it otherwise.
    fn toggle_origin(&mut self, origin: &Origin, peers: &mut Peers) {
        match self.playing_from(origin, peers.queue) {
            Some(_) => self.toggle_play(peers),
            None => self.play_origin(origin.clone(), peers),
        }
    }

    /// Moves to `position`. While playing, the clock parks there until the engine reports
    /// audio from it, and a second seek meanwhile waits for the first to land. A restored track
    /// not yet held by the engine only moves its resume point.
    fn seek(&mut self, position: Duration, peers: &mut Peers) {
        if !self.local {
            self.seeks += 1;
        }
        if self.steer(|_| Command::Seek(position), peers.devices) {
            return;
        }
        if self.resume_at.is_some() {
            self.resume_at = Some(position);
            if self.resume_ready
                && let Some(engine) = self.active_engine()
            {
                engine.seek(position);
            }
            self.position = position;
            self.clock.reset(position, false);
            self.remember(true, peers.settings, peers.cx);
            peers.cx.changed(Topic::Playback);
            return;
        }
        if self.active_engine().is_none() {
            return;
        }
        // The clock parks at the target; the engine's Seeked starts it again once audio from
        // there is playing. A loading or paused engine takes the seek at once and reports the
        // position with its Playing, so only a playing one needs the seek tracked.
        self.position = position;
        self.clock.reset(position, false);
        self.seek_target = Some(position);
        match (self.state == PlaybackState::Playing, self.seek_in_flight) {
            (true, Some(_)) => self.seek_next = Some(position),
            (true, None) => {
                self.seek_in_flight = Some(position);
                self.stale_positions = 0;
                self.send_seek(position);
            }
            (false, _) => self.send_seek(position),
        }
        peers.cx.changed(Topic::Playback);
    }

    fn send_seek(&self, position: Duration) {
        if let Some(engine) = self.active_engine() {
            engine.seek(position);
        }
    }

    /// The position to show for where the engine reports it landed: the seek target itself
    /// when the engine stopped just short of it on a packet boundary.
    fn landed(&mut self, at: Duration) -> Duration {
        match self.seek_target.take() {
            Some(target) if at < target && target - at < SEEK_SNAP => target,
            _ => at,
        }
    }

    /// Sends the seek that queued up behind the one that just landed.
    fn follow_up_seek(&mut self, peers: &mut Peers) {
        if let Some(next) = self.seek_next.take() {
            self.seek(next, peers);
        }
    }

    /// Seeks to a share of the track, as the progress bar asks.
    fn seek_fraction(&mut self, fraction: f32, peers: &mut Peers) {
        let seek = |remote: &Elsewhere| {
            Command::Seek(remote.state.duration.mul_f32(fraction.clamp(0., 1.)))
        };
        if self.steer(seek, peers.devices) {
            return;
        }
        let Some(total) = self
            .track
            .as_ref()
            .map(|track| track.duration)
            .filter(|total| !total.is_zero())
        else {
            return;
        };

        let position = Duration::from_secs_f32(total.as_secs_f32() * fraction.clamp(0., 1.));
        self.seek(position, peers);
    }

    /// How many seeks the listener asked for since launch.
    pub fn seeks(&self) -> u64 {
        self.seeks
    }

    pub fn state(&self) -> &PlaybackState {
        &self.state
    }

    /// The position as last reported or asked for. It only moves on events, and moving it
    /// raises nothing: a window that shows progress polls [`Self::live_position`] instead.
    pub fn position(&self) -> Duration {
        self.position
    }

    /// The position right now: the clock while playing, else `position`. Never past the end.
    /// Cheap enough to read every frame, which is how the progress bar and the lyrics follow
    /// the music without the store raising anything between the engine's reports.
    pub fn live_position(&self) -> Duration {
        let live = match self.state == PlaybackState::Playing {
            true => self.clock.now(),
            false => self.position,
        };
        match self.track.as_ref().map(|track| track.duration) {
            Some(total) if !total.is_zero() => live.min(total),
            _ => live,
        }
    }

    pub fn track(&self) -> Option<&Track> {
        self.track.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, PlaybackState::Loading)
    }

    /// Whether a track is loaded, whatever it is doing.
    fn has_active_playback(&self) -> bool {
        self.track.is_some()
            && matches!(
                self.state,
                PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Loading
            )
    }

    pub fn volume(&self) -> f32 {
        self.level
    }

    /// The gain the volume applies, 1 at full volume.
    pub fn volume_gain(&self) -> f32 {
        gain(self.level)
    }

    fn set_volume(&mut self, level: f32, peers: &mut Peers) {
        self.sliding = None;
        if let Some(steered) = self.steering(peers.devices) {
            peers
                .devices
                .set_volume(&steered.device.id, level.clamp(0., 1.), peers.cx);
            peers.cx.changed(Topic::Playback);
            return;
        }
        self.level = level.clamp(0., 1.);
        peers.settings.set_volume(self.level, peers.cx);
        self.apply_gain();
        peers.cx.changed(Topic::Playback);
    }

    /// Follows the volume slider while it is dragged: the level is heard at once, but nothing is
    /// saved and no follower wakes, since nearly every model and the media controls follow
    /// playback. The window redraws after the handler as it always does, and calls
    /// `Store::end_volume_slide` when the slider is let go.
    pub fn slide_volume(&mut self, level: f32, devices: &Devices) {
        let level = level.clamp(0., 1.);
        self.sliding = Some(level);
        if self.steer(|_| Command::Volume(level), devices) {
            return;
        }
        self.level = level;
        self.apply_gain();
    }

    /// Saves the level the slider was let go at and tells every follower, once per drag.
    fn end_volume_slide(&mut self, peers: &mut Peers) {
        let Some(level) = self.sliding else {
            return;
        };
        self.set_volume(level, peers);
    }

    /// Sends the gain for the current level to both engines.
    fn apply_gain(&self) {
        let level = gain(self.level);
        if let Some(engine) = self.engine.as_ref() {
            engine.set_gain(level);
        }
        if let Some(engine) = self.local_engine.as_ref() {
            engine.set_gain(level);
        }
    }

    pub fn normalisation(&self) -> bool {
        self.normalisation
    }

    pub fn gapless(&self) -> bool {
        self.gapless
    }

    pub fn transcode(&self) -> Transcode {
        self.transcode
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    pub fn keep_tracks(&self) -> bool {
        self.keep_tracks
    }

    pub fn cache_gb(&self) -> u32 {
        self.cache_gb
    }

    /// What the cache and the downloads take on disk, in bytes, as last measured.
    pub fn usage(&self) -> Option<(u64, u64)> {
        self.usage
    }

    /// Measures what the cache and the downloads take on disk, off the main thread.
    pub fn measure_storage(&mut self, cx: &mut Cx) {
        let vault = self.vault.clone();
        self.measured(cx, move || vault.usage());
    }

    /// Removes every download of every provider and measures the vault again after. The lists
    /// of what was downloaded are [`Downloads`](crate::Downloads)' to clear.
    pub(crate) fn remove_downloads(&mut self, cx: &mut Cx) {
        let vault = self.vault.clone();
        self.measured(cx, move || {
            vault.forget_downloads();
            vault.usage()
        });
    }

    /// Removes every cached track, downloads aside, and measures the vault again after.
    pub fn clear_cache(&mut self, cx: &mut Cx) {
        let vault = self.vault.clone();
        self.measured(cx, move || {
            vault.forget();
            vault.usage()
        });
    }

    /// Runs `work` off the main thread and keeps the usage it answers.
    fn measured(&mut self, cx: &mut Cx, work: impl FnOnce() -> (u64, u64) + Send + 'static) {
        self.measuring = Some(cx.spawn(move |link| async move {
            let Ok(usage) = tokio::task::spawn_blocking(work).await else {
                return;
            };
            link.post(move |store| {
                let this = &mut store.playback;
                if this.usage != Some(usage) {
                    this.usage = Some(usage);
                }
            });
        }));
    }

    /// Changes how many gigabytes the cache of played tracks may take. A lower limit evicts
    /// the least recently played tracks right away. Downloads do not count toward it, and
    /// Spotify's own files do but only follow a new limit at its next sign-in.
    pub fn set_cache_gb(&mut self, gb: u32, settings: &mut AppSettings, cx: &mut Cx) {
        let (low, high) = crate::vault::CACHE_GB_RANGE;
        let gb = gb.clamp(low, high);
        if self.cache_gb == gb {
            return;
        }
        self.cache_gb = gb;
        settings.set_cache_gb(gb, cx);
        let vault = self.vault.clone();
        let bytes = crate::vault::bytes(gb);
        Vault::set_keeping(self.keep_tracks, bytes);
        self.resize = Some(cx.spawn(move |_| async move {
            tokio::task::spawn_blocking(move || vault.set_limit(bytes))
                .await
                .ok();
        }));
    }

    /// Fetches all of a streaming track into the downloads without playing it, through the
    /// engine that would play it. `None` while there is no streaming engine.
    pub fn download(
        &self,
        id: &str,
        progress: music::Fraction,
    ) -> Option<BoxFuture<'static, Result<()>>> {
        self.engine
            .as_deref()
            .map(|engine| engine.download(id, progress))
    }

    /// Whether a streaming track plays with no network, which takes a download the provider can
    /// open offline. Until the vault has its key the engine cannot tell which tracks are
    /// downloaded, so the list of downloads answers for it, and the engine waits for the key.
    fn plays_offline(&self, id: &str, downloads: &Downloads) -> bool {
        let Some(engine) = self.engine.as_deref() else {
            return false;
        };
        match self.vault.unlocking() {
            true => engine.plays_offline(id) && downloads.has(id),
            false => engine.plays_offline(id),
        }
    }

    /// Removes the download of a streaming track.
    pub fn discard(&self, id: &str) {
        if let Some(engine) = self.engine.as_deref() {
            engine.discard(id);
        }
    }

    /// The vault downloads and the cache share, for removing every download at once.
    pub fn vault(&self) -> &Arc<Vault> {
        &self.vault
    }

    /// Turns caching played tracks on or off. Turning it off also removes every cached track,
    /// and downloads stay. Spotify keeps its own files and follows at its next sign-in.
    pub fn set_keep_tracks(&mut self, on: bool, settings: &mut AppSettings, cx: &mut Cx) {
        if self.keep_tracks == on {
            return;
        }
        self.keep_tracks = on;
        Vault::set_keeping(on, crate::vault::bytes(self.cache_gb));
        self.vault.set_caching(on);
        settings.set_keep_tracks(on, cx);
        if !on {
            self.clear_cache(cx);
        }
    }

    fn set_gapless(&mut self, on: bool, peers: &mut Peers) {
        if self.gapless == on {
            return;
        }
        self.gapless = on;
        peers.settings.set_gapless(on, peers.cx);
        self.restart_engine(peers);
    }

    /// Changes how a converting server is asked to send tracks. The streaming engine restarts,
    /// so the current track reloads in the new format where it was.
    fn set_transcode(&mut self, transcode: Transcode, peers: &mut Peers) {
        if self.transcode == transcode {
            return;
        }
        self.transcode = transcode;
        peers.settings.set_transcode(transcode, peers.cx);
        self.restart_engine(peers);
    }

    /// Changes the stream level asked of a streaming service. The streaming engine restarts, so
    /// the current track reloads at the new level where it was.
    fn set_quality(&mut self, quality: Quality, peers: &mut Peers) {
        if self.quality == quality {
            return;
        }
        self.quality = quality;
        peers.settings.set_quality(quality, peers.cx);
        self.restart_engine(peers);
    }

    fn set_normalisation(&mut self, on: bool, peers: &mut Peers) {
        if self.normalisation == on {
            return;
        }
        self.normalisation = on;
        peers.settings.set_normalisation(on, peers.cx);
        self.restart_engine(peers);
    }

    /// Whether the current track plays through the local engine.
    fn local_active(&self) -> bool {
        self.track
            .as_ref()
            .and_then(|track| track.id.as_deref())
            .is_some_and(music::is_local_id)
    }

    /// Rebuilds the streaming engine after a setting it was configured with changed, resuming
    /// where it was unless a local track is playing.
    fn restart_engine(&mut self, peers: &mut Peers) {
        let playback = match self.engine.is_some() {
            true => peers.session.playback(),
            false => None,
        };
        let Some(playback) = playback else {
            return;
        };

        match self.local_active() {
            true => self.start_engine(playback, peers.cx),
            false => self.rebind(playback, peers),
        }
    }

    /// Rebuilds every running engine on the audio device it should play on now, as when the
    /// device is chosen in settings.
    fn restart_outputs(&mut self, peers: &mut Peers) {
        self.restart_output(false, None, peers);
        self.restart_output(true, None, peers);
    }

    /// Rebuilds the local or the streaming engine, if it is running, on the audio device it
    /// should play on now. The engine of the current track reloads it where the clock stood,
    /// playing if it was and held paused there if it was not; the other one, or one with nothing
    /// loaded, comes back empty, so its stream still reopens at the device's new rate. A failed
    /// track is left as it is.
    fn restart_output(
        &mut self,
        local: bool,
        heard: Option<(String, Duration)>,
        peers: &mut Peers,
    ) {
        let running = match local {
            true => self.local_engine.is_some(),
            false => self.engine.is_some(),
        };
        if !running {
            return;
        }
        let playback = match local {
            true => peers.session.local_playback(),
            false => peers.session.playback(),
        };
        let Some(playback) = playback else {
            return;
        };
        let current = self
            .track
            .clone()
            .filter(|track| track.id.is_some() && local == self.local_active());
        // what the output had played beats the clock, which kept running while the device
        // switched and would put the reload past the start of a track that just began
        let at = heard
            .filter(|(id, _)| {
                current.as_ref().and_then(|track| track.id.as_deref()) == Some(id.as_str())
            })
            .map(|(_, at)| at)
            .unwrap_or_else(|| self.live_position());
        let playing = match self.state {
            PlaybackState::Playing => Some(true),
            PlaybackState::Loading => Some(self.intent == Intent::Play),
            PlaybackState::Paused | PlaybackState::Idle => Some(false),
            PlaybackState::Failed(_) => None,
        };
        // the engine holds a paused track where it stood as it starts, as it does a restored one
        if current.is_some() && playing == Some(false) {
            self.resume_at = Some(at);
        }

        log::info!("playback: restarting after the audio output changed");
        match local {
            true => {
                self.local_task = None;
                retire(self.local_engine.take());
                self.start_local_engine(playback, peers.cx);
            }
            false => {
                self.task = None;
                retire(self.engine.take());
                self.start_engine(playback, peers.cx);
            }
        }
        if let Some(track) = current
            && playing == Some(true)
        {
            self.load_from(&track, at, Start::Pick, peers);
        }
    }

    /// Whether a track's failure is the session having gone stale, in which case the session
    /// reconnects and playback continues from `rebind`.
    fn ask_for_reconnect(&mut self, peers: &mut Peers) -> bool {
        if self.track.is_none() {
            return false;
        }
        self.awaiting_reconnect = peers.session.reconnect_if_stale(peers.cx);
        self.awaiting_reconnect
    }

    /// Moves playback onto a fresh engine, as after a reconnect, and resumes where it was if it
    /// was playing, loading to play, or waiting to.
    fn rebind(&mut self, playback: Arc<dyn PlaybackFactory>, peers: &mut Peers) {
        let resume = match self.state {
            PlaybackState::Playing => true,
            PlaybackState::Loading => self.intent == Intent::Play,
            _ => false,
        } || self.awaiting_reconnect;
        self.awaiting_reconnect = false;
        let at = self.live_position();
        self.task = None;
        self.engine = None;
        self.preloaded = None;
        self.blocked_until = None;
        if self.track.is_some() {
            self.resume_at = Some(at);
        }
        self.start_engine(playback, peers.cx);
        if resume {
            self.resume(peers);
        }
    }

    /// The engine settings every engine started here shares.
    fn config(&self) -> PlaybackConfig {
        PlaybackConfig {
            normalisation: self.normalisation,
            gapless: self.gapless,
            position_interval: POSITION_INTERVAL,
            gain: gain(self.level),
            transcode: self.transcode,
            quality: self.quality,
            equalizer: self.equalizer.clone(),
            meter: self.meter.clone(),
            analyzer: self.analyzer.clone(),
            vault: Some(self.vault.clone()),
        }
    }

    /// Builds the streaming engine and starts pumping its events.
    fn start_engine(&mut self, playback: Arc<dyn PlaybackFactory>, cx: &mut Cx) {
        let (engine, events) = playback.start(self.config());

        self.listen(events, false, cx);
        self.engine = Some(engine);
        self.refused = None;
        if !self.local_active() {
            self.state = PlaybackState::Idle;
            self.position = Duration::ZERO;
            self.clock.reset(Duration::ZERO, false);
        }
        self.prepare_resume(cx);
        cx.changed(Topic::Playback);
    }

    fn start_local_engine(&mut self, playback: Arc<dyn PlaybackFactory>, cx: &mut Cx) {
        let (engine, events) = playback.start(self.config());

        self.listen(events, true, cx);
        self.local_engine = Some(engine);
        self.prepare_resume(cx);
    }

    /// Pumps an engine's events into `on_backend_event` until the engine is dropped. Each event
    /// is posted to the UI thread on its own; dropping the job stops the pump and anything it
    /// posted that has not run yet.
    fn listen(&mut self, mut events: Box<dyn PlaybackEvents>, local: bool, cx: &mut Cx) {
        let task = Some(cx.spawn(move |link| async move {
            while let Some(event) = events.next().await {
                link.post(move |store| {
                    let quiet = matches!(
                        event,
                        BackendEvent::Position { .. }
                            | BackendEvent::Downloaded { .. }
                            | BackendEvent::Length { .. }
                    );
                    if quiet {
                        store.cx.hush();
                    }
                    let (this, mut peers) = store.playback_parts();
                    this.on_backend_event(event, local, &mut peers);
                });
            }
        }));
        match local {
            true => self.local_task = task,
            false => self.task = task,
        }
    }

    /// Applies what an engine reports. Events from the engine not playing the current track,
    /// or about another track, are dropped, so a late event from a previous load cannot move
    /// the clock. A position report, a download finishing or a length that did not change
    /// raises nothing: the window reads the clock as it draws.
    fn on_backend_event(&mut self, event: BackendEvent, local: bool, peers: &mut Peers) {
        // an engine whose output has to reopen is rebuilt whether it plays the current track
        // or sits idle, since a dead engine would take the next track and play nothing
        if let BackendEvent::OutputChanged { heard } = event {
            self.restart_output(local, heard, peers);
            return;
        }
        // kept for whichever track it names, since a queued track reports before it plays
        if let BackendEvent::Signal { id, track } = event {
            self.learn_signal(id, track, peers.cx);
            return;
        }
        if local != self.local_active() {
            return;
        }
        let current_id = self.track.as_ref().and_then(|track| track.id.as_deref());
        if let Some(event_id) = event.id()
            && current_id != Some(event_id)
        {
            return;
        }
        let quiet = matches!(
            event,
            BackendEvent::Position { .. }
                | BackendEvent::Downloaded { .. }
                | BackendEvent::Length { .. }
        );
        match event {
            // A Loading with the current id is the engine restarting the load at a seek
            // target, so a seek queued behind it must survive.
            BackendEvent::Position { .. }
            | BackendEvent::Length { .. }
            | BackendEvent::Downloaded { .. }
            | BackendEvent::Loading { .. } => {}
            BackendEvent::Playing { .. }
            | BackendEvent::Paused { .. }
            | BackendEvent::Seeked { .. } => {
                self.seek_in_flight = None;
            }
            _ => {
                self.seek_in_flight = None;
                self.seek_next = None;
            }
        }
        match event {
            // rebuilt or kept before the filters above
            BackendEvent::OutputChanged { .. } | BackendEvent::Signal { .. } => {}
            BackendEvent::Unavailable { .. }
            | BackendEvent::Throttled { .. }
            | BackendEvent::Refused
                if self.resume_ready =>
            {
                self.resume_ready = false;
                self.state = PlaybackState::Paused;
                log::warn!("playback: cannot hold the restored track, waiting for play");
            }
            BackendEvent::Playing { .. } if self.overtaken(Intent::Play) => {
                log::debug!("playback: ignoring a play the user has since paused");
            }
            BackendEvent::Paused { .. } if self.overtaken(Intent::Pause) => {
                log::debug!("playback: ignoring a pause the user has since undone");
            }
            BackendEvent::Loading { at, .. } => {
                self.state = PlaybackState::Loading;
                self.position = at;
                self.clock.reset(at, false);
            }
            BackendEvent::Playing { at, .. } => {
                let at = self.landed(at);
                let started = self.state != PlaybackState::Playing;
                self.intent = Intent::Play;
                self.pressed = None;
                self.state = PlaybackState::Playing;
                self.paused_at = None;
                self.failures = 0;
                self.throttles = 0;
                self.position = at;
                self.clock.reset(at, true);
                if started {
                    peers
                        .cx
                        .emit(Event::Playback(PlaybackEvent::StartedPlayback));
                }
                self.follow_up_seek(peers);
            }
            BackendEvent::Paused { at, .. } => {
                self.intent = Intent::Pause;
                self.pressed = None;
                self.state = PlaybackState::Paused;
                self.paused_at = Some(SystemTime::now());
                self.position = at;
                self.clock.reset(at, false);
                self.remember(true, peers.settings, peers.cx);
                peers.cx.emit(Event::Playback(PlaybackEvent::Paused));
                self.follow_up_seek(peers);
            }
            BackendEvent::Seeked { at, .. } => {
                let at = self.landed(at);
                self.position = at;
                self.clock.reset(at, self.state == PlaybackState::Playing);
                self.remember(true, peers.settings, peers.cx);
                peers.cx.emit(Event::Playback(PlaybackEvent::Seeked));
                self.follow_up_seek(peers);
            }
            BackendEvent::Position { at, .. } => {
                let mut failed = false;
                if self.seek_in_flight.is_some() {
                    self.stale_positions += 1;
                    if self.stale_positions <= STALE_POSITIONS {
                        return;
                    }
                    log::warn!("playback: the seek did not land, carrying on from {at:?}");
                    self.seek_in_flight = None;
                    failed = true;
                }
                self.position = at;
                match (self.state == PlaybackState::Playing, failed) {
                    (true, true) => self.clock.reset(at, true),
                    (true, false) => self.clock.correct(at),
                    (false, _) => self.clock.reset(at, false),
                }
                self.remember(false, peers.settings, peers.cx);
                self.preload_next(at, peers.queue);
                if failed {
                    peers.cx.changed(Topic::Playback);
                    self.follow_up_seek(peers);
                }
            }
            BackendEvent::Downloaded { .. } => self.preload_upcoming(false, peers.queue),
            BackendEvent::Length { duration, .. } => {
                if let Some(track) = self.track.as_mut()
                    && !duration.is_zero()
                    && track.duration != duration
                {
                    track.duration = duration;
                    peers.cx.changed(Topic::Playback);
                }
            }
            BackendEvent::Ended { .. } => {
                let ended = self.track.take();
                self.state = PlaybackState::Idle;
                self.position = Duration::ZERO;
                self.clock.reset(Duration::ZERO, false);
                peers.cx.emit(Event::Playback(PlaybackEvent::EndedPlayback));
                match self.sleep == Some(Sleep::EndOfTrack) {
                    true => self.doze(peers),
                    false => self.advance(ended, peers),
                }
            }
            BackendEvent::Throttled { .. } => self.throttled(peers),
            BackendEvent::Unavailable { .. } if self.ask_for_reconnect(peers) => {
                self.state = PlaybackState::Loading;
                log::warn!("playback: the provider went stale, waiting for a reconnect");
            }
            BackendEvent::Unavailable { .. } => {
                let failed = self.track.take();
                let target = failed.as_ref().and_then(song_target);
                let name = failed
                    .as_ref()
                    .map_or_else(|| "?".to_owned(), |track| track.name.clone());
                log::warn!(
                    "playback: {name} failed to load, backing off {}s",
                    KEY_COOLDOWN.as_secs()
                );
                self.blocked_until = Some(Instant::now() + KEY_COOLDOWN);
                self.failures = self.failures.saturating_add(1);
                self.state = PlaybackState::Idle;
                self.position = Duration::ZERO;
                self.clock.reset(Duration::ZERO, false);
                let exhausted = self.failures >= FAILURE_LIMIT;
                let toast = match exhausted {
                    true => {
                        log::warn!("playback: {FAILURE_LIMIT} tracks failed in a row, stopping");
                        match self.module_suspect(peers) {
                            true => Toast::new(Outcome::Failed, "toast-widevine-failed")
                                .note("toast-widevine-borrowed")
                                .open("toast-open-settings", Target::PlaybackSettings)
                                .lasting(),
                            false => Toast::new(Outcome::Failed, "toast-playback-stopped"),
                        }
                    }
                    false => Toast::new(Outcome::Failed, "toast-track-unplayable")
                        .name(name)
                        .target(target),
                };
                peers.cx.emit(Event::Toast(toast));
                peers.cx.emit(Event::Playback(PlaybackEvent::EndedPlayback));
                match (exhausted || self.intent == Intent::Pause, self.repeat) {
                    (true, _) => self.hold_next(peers),
                    (false, Repeat::One) => self.segue_queue(peers),
                    (false, _) => self.advance(failed, peers),
                }
            }
            BackendEvent::Refused => {
                self.refuse(peers.cx);
                peers.cx.emit(Event::Playback(PlaybackEvent::EndedPlayback));
            }
            BackendEvent::Gated => {
                self.gate(peers);
                peers.cx.emit(Event::Playback(PlaybackEvent::EndedPlayback));
            }
        }
        if !quiet {
            peers.cx.changed(Topic::Playback);
        }
    }

    /// Whether failing tracks point at the Widevine module: the current provider needs one,
    /// and the one in use is a browser's copy rather than one the user downloaded.
    fn module_suspect(&self, peers: &Peers) -> bool {
        peers.session.wants_drm() && peers.drm.borrowed()
    }

    /// Drops the streaming engine and everything derived from its session on sign-out. A local
    /// track keeps playing.
    fn teardown(&mut self, cx: &mut Cx) {
        self.task = None;
        self.engine = None;
        self.signals.retain(|(id, _)| music::is_local_id(id));

        if !self.local_active() {
            self.load = None;
            self.fetch = None;
            self.enqueue = None;
            self.suggest = None;
            self.seeded = None;
            self.leave_station();
            self.preloaded = None;
            self.skipped = None;
            self.blocked_until = None;
            self.failures = 0;
            self.throttles = 0;
            self.paused_at = None;
            self.refused = None;
            self.track = None;
            self.state = PlaybackState::Idle;
            self.position = Duration::ZERO;
            self.clock.reset(Duration::ZERO, false);
            self.resume_at = None;
            self.seek_in_flight = None;
            self.seek_next = None;
            self.seek_target = None;
            self.resume_ready = false;
            self.awaiting_reconnect = false;
            self.stored = Duration::ZERO;
            self.sleep = None;
            self.sleep_task = None;
        }
        cx.changed(Topic::Playback);
    }

    /// Stops with a reason the UI can show.
    fn failed(&mut self, problem: String, cx: &mut Cx) {
        log::error!("playback: {problem}");
        self.state = PlaybackState::Failed(problem);
        cx.changed(Topic::Playback);
    }

    /// Asks for the current track again after the provider turned it down for now. The wait
    /// doubles with each refusal in a row and only the first one toasts. Past `THROTTLE_LIMIT`,
    /// or once the user has paused, the track is held until play.
    fn throttled(&mut self, peers: &mut Peers) {
        let Some(track) = self.track.clone() else {
            return;
        };
        let provider = peers
            .session
            .provider_name()
            .unwrap_or("this provider")
            .to_owned();
        let at = self.position;
        self.throttles = self.throttles.saturating_add(1);
        if self.intent == Intent::Pause {
            return self.hold(at, peers.cx);
        }
        if self.throttles > THROTTLE_LIMIT {
            log::warn!(
                "playback: {provider} kept turning {} down, waiting for play",
                track.name
            );
            peers.cx.emit(Event::Toast(
                Toast::new(Outcome::Failed, "toast-still-throttled").name(provider),
            ));
            return self.hold(at, peers.cx);
        }
        let wait = throttle_wait(self.throttles);
        log::warn!(
            "playback: {provider} turned {} down for now, retrying in {}s",
            track.name,
            wait.as_secs()
        );
        if self.throttles == 1 {
            peers.cx.emit(Event::Toast(
                Toast::new(Outcome::Failed, "toast-throttled").name(provider),
            ));
        }
        self.blocked_until = Some(Instant::now() + wait);
        self.load_from(&track, at, Start::Retry, peers);
    }

    /// Turns down a track that has to be streamed while the network is gone. Whatever plays
    /// keeps playing, since a local file needs nothing, and only a track the user picked says
    /// so out loud: the queue moving on by itself would otherwise toast once a track.
    fn unreachable(reason: Unreachable, start: Start, cx: &mut Cx) {
        log::warn!("playback: nothing streams while the network is gone");
        if start == Start::Pick {
            reason.toast(cx);
        }
    }

    /// Why `track` cannot play here while the network is gone, or `None` when it can. A local
    /// file and a download the provider opens offline always play.
    pub fn unreachable_reason(
        &self,
        track: &Track,
        network: &Network,
        downloads: &Downloads,
        session: &Session,
    ) -> Option<Unreachable> {
        self.reason(track.id.as_deref()?, network, downloads, session)
    }

    fn reason(
        &self,
        id: &str,
        network: &Network,
        downloads: &Downloads,
        session: &Session,
    ) -> Option<Unreachable> {
        if music::is_local_id(id) || !network.lost() || self.plays_offline(id, downloads) {
            return None;
        }
        Some(match downloads.has(id) {
            true => {
                let provider = session.provider_name();
                Unreachable::Download(provider.unwrap_or("this provider").to_owned())
            }
            false if network.offline() => Unreachable::OfflineMode,
            false => Unreachable::Offline,
        })
    }

    /// Records that the provider wants a signed-in listener and stops until sign-in.
    fn gate(&mut self, peers: &mut Peers) {
        let first = self.refused.is_none();
        self.refused = Some(Refusal::SignIn);
        self.track = None;
        self.blocked_until = None;
        let provider = peers.session.provider_name().unwrap_or("this provider");
        self.state = PlaybackState::Failed(format!(
            "{provider} only streams to a signed-in listener; nothing will play until you sign in"
        ));
        if first {
            log::warn!(
                "playback: {provider} only streams to a signed-in listener, nothing will play \
                 until you sign in"
            );
        }
        peers.cx.emit(Event::Toast(
            Toast::new(Outcome::Failed, "toast-sign-in-to-play").name(provider.to_owned()),
        ));
        peers.cx.changed(Topic::Playback);
    }

    /// Records that Spotify denied an audio key and stops for the rest of the session.
    fn refuse(&mut self, cx: &mut Cx) {
        let first = self.refused.is_none();
        self.refused = Some(Refusal::Keys);
        self.track = None;
        self.blocked_until = None;
        self.state = PlaybackState::Failed(
            "spotify denied an audio key for this account; nothing will play in this session"
                .to_owned(),
        );
        if first {
            log::error!(
                "playback: spotify denied an audio key for this account; nothing will play in \
                 this session"
            );
        }
        cx.emit(Event::Toast(
            Toast::new(Outcome::Failed, "toast-keys-error")
                .note("toast-keys-refused")
                .visit("toast-why", KEYS_FAQ),
        ));
        cx.changed(Topic::Playback);
    }
}

impl Store {
    /// Playback and the models it works with, split off the store so playback can move them in
    /// one call.
    pub(crate) fn playback_parts(&mut self) -> (&mut Playback, Peers<'_>) {
        let Store {
            playback,
            queue,
            session,
            settings,
            devices,
            network,
            downloads,
            drm,
            cx,
            ..
        } = self;
        let peers = Peers {
            queue,
            session,
            settings,
            devices,
            network,
            downloads,
            drm,
            cx,
        };
        (playback, peers)
    }

    /// Runs `act` as this app's own doing, on the engine here even while another device has
    /// playback. What the device network asks of this app goes through it.
    pub(crate) fn locally(&mut self, act: impl FnOnce(&mut Store)) {
        self.playback.local = true;
        act(self);
        self.playback.local = false;
    }

    /// How playback follows the session and the queue: the engines are built, rebuilt and
    /// dropped as the account signs in, reconnects, goes offline or signs out, and a queue
    /// running short tops up its station and its radio suggestions.
    pub(crate) fn react_playback(&mut self, change: &Change) {
        let (this, mut peers) = self.playback_parts();
        match change {
            Change::Event(Event::Session(event)) => this.on_session(event, &mut peers),
            Change::Topic(Topic::Session) => this.follow_session(peers.session, peers.cx),
            Change::Topic(Topic::Queue) => {
                this.continue_station(&mut peers);
                this.suggest_similar(false, &mut peers);
            }
            _ => {}
        }
    }

    /// The active queue for every consumer, selected once from local or remote playback.
    pub fn shown_queue(&self) -> crate::QueueView<'_> {
        let playing = matches!(self.playback.state(), PlaybackState::Playing);
        let remote = self.playback.steering(&self.devices);
        crate::QueueView::new(&self.queue, &self.devices, remote, playing)
    }

    /// Why `track` cannot play here while the network is gone, or `None` when it can.
    pub fn unreachable_reason(&self, track: &Track) -> Option<Unreachable> {
        self.playback
            .unreachable_reason(track, &self.network, &self.downloads, &self.session)
    }

    /// Plays a track the user picked, from its start.
    pub fn play_track(&mut self, track: &Track) {
        let (playback, mut peers) = self.playback_parts();
        playback.play(track, &mut peers);
    }

    /// Replaces the queue with `tracks` and plays the one at `index`.
    pub fn play_tracks(&mut self, tracks: Vec<Track>, index: usize, origin: Option<Origin>) {
        let (playback, mut peers) = self.playback_parts();
        playback.start(tracks, index, origin, &mut peers);
    }

    /// Replaces the queue with `tracks` and plays the one at `index` here from `at`, as another
    /// device handed it over. A seek after `play_tracks` would be lost while the track loads.
    pub(crate) fn play_tracks_at(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        at: Duration,
    ) {
        let (playback, mut peers) = self.playback_parts();
        playback.start_at(tracks, index, origin, at, &mut peers);
    }

    /// Replaces the queue with `tracks` and plays the first playable one, or a random one when
    /// shuffle is on.
    pub fn play_any(&mut self, tracks: Vec<Track>, origin: Option<Origin>) {
        let (playback, mut peers) = self.playback_parts();
        playback.start_any(tracks, origin, &mut peers);
    }

    /// Turns shuffle on and starts the tracks from a random playable one.
    pub fn shuffle_tracks(&mut self, tracks: Vec<Track>, origin: Option<Origin>) {
        let (playback, mut peers) = self.playback_parts();
        playback.shuffle_any(tracks, origin, &mut peers);
    }

    /// Plays `seed` now and fills the queue behind it with its radio once that arrives.
    pub fn play_radio(&mut self, seed: &Track) {
        let (playback, mut peers) = self.playback_parts();
        playback.play_radio(seed, &mut peers);
    }

    /// Appends a track, or starts playing it when nothing is queued.
    pub fn enqueue_track(&mut self, track: Track) {
        self.enqueue_tracks(vec![track], None, QueuePlacement::End);
    }

    /// Queues a track right after the current one, or starts playing it when nothing is queued.
    pub fn play_next(&mut self, track: Track) {
        self.enqueue_tracks(vec![track], None, QueuePlacement::Next);
    }

    /// Queues a track after everything already lined up, or starts playing it when nothing is.
    pub fn play_last(&mut self, track: Track) {
        self.enqueue_tracks(vec![track], None, QueuePlacement::Last);
    }

    /// Places tracks the user picked and says where they went, naming one track and counting
    /// several. Nothing is said when they start playback instead, since playing is the answer.
    pub fn enqueue_tracks(
        &mut self,
        tracks: Vec<Track>,
        origin: Option<Origin>,
        placement: QueuePlacement,
    ) {
        let (playback, mut peers) = self.playback_parts();
        playback.enqueue_tracks(tracks, origin, placement, &mut peers);
    }

    /// Opens paths handed in from the OS (a file-association launch or hand-off). A single file
    /// plays right away, since picking one is a request to hear it now; several play next,
    /// right after whatever is already playing, whichever provider it came from — or start right
    /// away if nothing is.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let (playback, mut peers) = self.playback_parts();
        playback.resolve_paths(paths, &mut peers);
    }

    /// Inserts tracks `gap` places into the upcoming list without a word, or starts playing
    /// them when nothing is queued.
    pub fn insert_tracks(&mut self, tracks: Vec<Track>, gap: usize, origin: Option<Origin>) {
        let (playback, mut peers) = self.playback_parts();
        playback.place(tracks, origin, QueuePlacement::Gap(gap), &mut peers);
    }

    /// Fetches a pinned collection and inserts it `gap` tracks ahead, or at the end. A pinned
    /// song goes in on its own, without an origin.
    pub fn enqueue_pin(&mut self, pin: &Pin, gap: Option<usize>) {
        let origin = (pin.kind != PinKind::Song).then(|| Origin::from(pin));
        let (playback, mut peers) = self.playback_parts();
        playback.enqueue_from(
            &pin.id,
            origin,
            QueuePlacement::Gap(gap.unwrap_or(usize::MAX)),
            &mut peers,
        );
    }

    /// Fetches any collection through the same queue placement path.
    pub fn enqueue_collection(&mut self, origin: Origin, placement: QueuePlacement) {
        let id = origin.id.clone();
        let (playback, mut peers) = self.playback_parts();
        playback.enqueue_from(&id, Some(origin), placement, &mut peers);
    }

    /// Starts a collection from its first playable track once its tracks are loaded.
    pub fn play_collection(&mut self, origin: Origin) {
        let (playback, mut peers) = self.playback_parts();
        playback.play_origin(origin, &mut peers);
    }

    /// Turns shuffle on and plays `origin` from a random track once its tracks are loaded.
    pub fn shuffle_collection(&mut self, origin: Origin) {
        let (playback, mut peers) = self.playback_parts();
        playback.shuffle_origin(origin, &mut peers);
    }

    /// The play button of a collection page: pauses or resumes when the queue came from it,
    /// starts it otherwise.
    pub fn toggle_collection(&mut self, origin: &Origin) {
        let (playback, mut peers) = self.playback_parts();
        playback.toggle_origin(origin, &mut peers);
    }

    /// Skips to the next playable track.
    pub fn next_track(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.next(&mut peers);
    }

    /// Restarts the track a few seconds in, else goes back to the one before.
    pub fn previous_track(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.previous(&mut peers);
    }

    /// Removes tracks from the queue, and moves on when the current one was among them.
    pub fn remove_from_queue(&mut self, ids: &[String]) {
        let (playback, mut peers) = self.playback_parts();
        playback.remove_from_queue(ids, &mut peers);
    }

    /// Plays the queue entry `uid`; the current one toggles playback instead.
    pub fn play_queued(&mut self, uid: &str) {
        let (playback, mut peers) = self.playback_parts();
        playback.play_queue(uid, &mut peers);
    }

    /// Plays on, on the other device when one has playback.
    pub fn resume_playback(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.resume(&mut peers);
    }

    /// Pauses, on the other device when one has playback.
    pub fn pause_playback(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.pause(&mut peers);
    }

    /// Pauses what plays, or plays what is paused.
    pub fn toggle_play(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.toggle_play(&mut peers);
    }

    /// Moves to `position` in the current track.
    pub fn seek_to(&mut self, position: Duration) {
        let (playback, mut peers) = self.playback_parts();
        playback.seek(position, &mut peers);
    }

    /// Seeks to a share of the track, as the progress bar asks.
    pub fn seek_fraction(&mut self, fraction: f32) {
        let (playback, mut peers) = self.playback_parts();
        playback.seek_fraction(fraction, &mut peers);
    }

    /// Sets the volume and remembers it, on the other device when one has playback.
    pub fn set_volume(&mut self, level: f32) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_volume(level, &mut peers);
    }

    /// Follows the volume slider while it is dragged, heard at once but saved only on
    /// [`Self::end_volume_slide`].
    pub fn slide_volume(&mut self, level: f32) {
        self.playback.slide_volume(level, &self.devices);
    }

    /// Saves the level the slider was let go at and tells every follower, once per drag.
    pub fn end_volume_slide(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.end_volume_slide(&mut peers);
    }

    /// Switches the repeat mode and remembers it in settings.
    pub fn set_repeat(&mut self, repeat: Repeat) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_repeat(repeat, &mut peers);
    }

    /// Steps the repeat mode through off, all and one.
    pub fn cycle_repeat(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.cycle_repeat(&mut peers);
    }

    /// A binary on/off flip of repeat for the tray and the dock menu.
    pub fn toggle_repeat(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.toggle_repeat(&mut peers);
    }

    /// Turns shuffle on or off, on the other device when one has playback.
    pub fn set_shuffle(&mut self, on: bool) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_shuffle(on, &mut peers);
    }

    /// Flips shuffle from what the player shows.
    pub fn toggle_shuffle(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.toggle_shuffle(&mut peers);
    }

    /// Turns radio suggestions on or off and remembers it.
    pub fn toggle_radio(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.toggle_radio(&mut peers);
    }

    /// Replaces the active sleep request.
    pub fn set_sleep(&mut self, sleep: Option<Sleep>) {
        self.playback.set_sleep(sleep, &mut self.cx);
    }

    /// Turns gapless playback on or off; the streaming engine restarts where it was.
    pub fn set_gapless(&mut self, on: bool) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_gapless(on, &mut peers);
    }

    /// Changes how a converting server is asked to send tracks.
    pub fn set_transcode(&mut self, transcode: Transcode) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_transcode(transcode, &mut peers);
    }

    /// Changes the stream level asked of a streaming service.
    pub fn set_quality(&mut self, quality: Quality) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_quality(quality, &mut peers);
    }

    /// Turns loudness normalisation on or off; the streaming engine restarts where it was.
    pub fn set_normalisation(&mut self, on: bool) {
        let (playback, mut peers) = self.playback_parts();
        playback.set_normalisation(on, &mut peers);
    }

    /// Changes how many gigabytes the cache of played tracks may take.
    pub fn set_cache_gb(&mut self, gb: u32) {
        self.playback
            .set_cache_gb(gb, &mut self.settings, &mut self.cx);
    }

    /// Turns caching played tracks on or off.
    pub fn set_keep_tracks(&mut self, on: bool) {
        self.playback
            .set_keep_tracks(on, &mut self.settings, &mut self.cx);
    }

    /// Rebuilds every running engine on the audio device it should play on now, as when the
    /// device is chosen in settings.
    pub(crate) fn restart_outputs(&mut self) {
        let (playback, mut peers) = self.playback_parts();
        playback.restart_outputs(&mut peers);
    }
}

/// Keeps the playable tracks that are not among `heard`, the ids a queue already holds.
fn unheard(tracks: &mut Vec<Track>, heard: &HashSet<String>) {
    tracks.retain(|track| {
        track.playable
            && track
                .id
                .as_ref()
                .is_some_and(|id| !heard.contains(id.as_str()))
    });
}

fn song_target(track: &Track) -> Option<Target> {
    track
        .id
        .as_deref()
        .map(|id| Target::Song(id.to_owned().into()))
}

/// Says which files handed in from the OS could not be opened. One file is named with its
/// reason, several are counted with their names below, and when others did open it is a
/// warning, since those still play.
fn report_unread(unread: &[Unread], nothing_read: bool, cx: &mut Cx) {
    let toast = match (unread, nothing_read) {
        ([], _) => return,
        ([only], true) => match &only.error {
            Some(error) => Toast::new(Outcome::Failed, "toast-open-failed")
                .name(only.name.clone())
                .detail(error.clone()),
            None => Toast::new(Outcome::Failed, "toast-open-unsupported").name(only.name.clone()),
        },
        (_, true) => Toast::new(Outcome::Failed, "toast-open-all-failed")
            .count(unread.len())
            .detail(listed(unread)),
        (_, false) => Toast::new(Outcome::Warning, "toast-open-some-failed").detail(listed(unread)),
    };
    cx.emit(Event::Toast(toast));
}

/// The names of unreadable files for a toast's detail line, cut short after `LISTED_FILES`.
fn listed(unread: &[Unread]) -> String {
    let mut names = unread
        .iter()
        .take(LISTED_FILES)
        .map(|file| file.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if unread.len() > LISTED_FILES {
        names.push_str(", …");
    }
    names
}

/// How long to wait before the retry that follows the `throttles`th refusal in a row.
fn throttle_wait(throttles: u8) -> Duration {
    let doublings = u32::from(throttles.saturating_sub(1)).min(8);
    THROTTLE_WAIT
        .saturating_mul(1 << doublings)
        .min(THROTTLE_CAP)
}

/// Drops an engine on a thread of its own. Spotify's waits for its player thread to end, which
/// sits out an output device that takes CoreAudio ten seconds to give up on, so dropping it here
/// would hold the window for as long.
fn retire(engine: Option<Box<dyn Player>>) {
    let Some(engine) = engine else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("engine-retire".to_owned())
        .spawn(move || drop(engine));
    if let Err(error) = spawned {
        log::warn!("playback: cannot retire the old engine off the main thread: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{LiveClock, gain};

    #[test]
    fn never_amplifies_past_unity() {
        assert_eq!(gain(1.), 1.);
        for step in 0..=100 {
            assert!(gain(step as f32 / 100.) <= 1.);
        }
    }

    #[test]
    fn silences_a_closed_slider() {
        assert_eq!(gain(0.), 0.);
        assert_eq!(gain(-1.), 0.);
        assert_eq!(gain(2.), 1.);
    }

    #[test]
    fn rises_with_the_slider() {
        let mut last = gain(0.);
        for step in 1..=100 {
            let next = gain(step as f32 / 100.);
            assert!(next > last, "gain fell at {step}");
            last = next;
        }
    }

    #[test]
    fn halves_the_slider_to_the_taper_midpoint() {
        let expected = 10f32.powf(-super::TAPER_DB / 40.);
        assert!((gain(0.5) - expected).abs() < 1e-6);
    }

    #[test]
    fn live_clock_does_not_jump_when_corrected() {
        let began = Instant::now();
        let mut clock = LiveClock {
            base: Duration::from_secs(10),
            since: Some(began),
            correction: 0.,
            settle: 1.,
        };
        let corrected = began + Duration::from_millis(500);
        let before = clock.at(corrected);

        clock.correct_at(Duration::from_secs(10), corrected);

        assert_eq!(clock.at(corrected), before);
    }

    #[test]
    fn live_clock_slews_backward_corrections_without_reversing() {
        let began = Instant::now();
        let corrected = began + Duration::from_secs(1);
        let mut clock = LiveClock {
            base: Duration::from_secs(10),
            since: Some(began),
            correction: 0.,
            settle: 1.,
        };
        clock.correct_at(Duration::from_secs(9), corrected);

        let samples = (0..=30).map(|step| clock.at(corrected + Duration::from_millis(step * 100)));
        let mut previous = Duration::ZERO;
        for sample in samples {
            assert!(sample >= previous);
            previous = sample;
        }
    }
}
