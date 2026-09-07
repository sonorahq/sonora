use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::{App, Context, Entity, EventEmitter, SharedString, Task};
use music::{
    MusicApi, PlaybackConfig, PlaybackEvent as BackendEvent, PlaybackEvents, PlaybackFactory,
    Player, Spectrum, Track,
};
use ui::{Pin, PinKind};

type Fetch = std::pin::Pin<Box<dyn Future<Output = Result<Vec<Track>>> + Send>>;

#[derive(Clone, Copy, PartialEq)]
enum Refusal {
    Keys,
    SignIn,
}

#[derive(Clone, Copy, PartialEq)]
enum Start {
    Pick,
    Skip,
    Burst,
    Segue,
}

impl Start {
    fn debounce(self) -> Duration {
        match self {
            Self::Burst => SKIP_DEBOUNCE,
            Self::Pick | Self::Skip | Self::Segue => Duration::ZERO,
        }
    }
}

#[derive(Clone, Copy)]
enum QueuePlacement {
    Next,
    End,
    Gap(usize),
}

impl QueuePlacement {
    fn toast(self, source: &str) -> Option<String> {
        match self {
            Self::Next => Some(format!("toast-next-{source}")),
            Self::End => Some(format!("toast-queued-{source}")),
            Self::Gap(_) => None,
        }
    }
}

use crate::queue::Queue;
use serde::{Deserialize, Serialize};

use crate::{AppSettings, Io, Outcome, Session, SessionEvent, Target, Toasts, join};

const POSITION_INTERVAL: Duration = Duration::from_millis(500);
const CLOCK_SETTLE: Duration = Duration::from_secs(1);
const PRELOAD_BEFORE_END: Duration = Duration::from_secs(10);
const SKIP_DEBOUNCE: Duration = Duration::from_millis(250);
const RESTART_WINDOW: Duration = Duration::from_secs(3);
const KEY_COOLDOWN: Duration = Duration::from_secs(6);
const RESUME_STEP: Duration = Duration::from_secs(5);
const SEEK_CATCHUP: Duration = Duration::from_millis(400);
const SEEK_HOLD: Duration = Duration::from_secs(2);
const TAPER_DB: f32 = 50.;
const LOCAL_FAVORITES: &str = "favorites";
const SIMILAR_LIMIT: usize = 20;

struct LiveClock {
    base: Duration,
    since: Option<Instant>,
    correction: f64,
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

    fn reset(&mut self, at: Duration, running: bool) {
        self.base = at;
        self.since = running.then(Instant::now);
        self.correction = 0.;
        self.settle = CLOCK_SETTLE.as_secs_f64();
    }

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

fn away(left: Duration, right: Duration) -> Duration {
    match left >= right {
        true => left - right,
        false => right - left,
    }
}

fn signed_gap(to: Duration, from: Duration) -> f64 {
    match to >= from {
        true => (to - from).as_secs_f64(),
        false => -(from - to).as_secs_f64(),
    }
}

fn shifted(base: Duration, seconds: f64) -> Duration {
    match seconds >= 0. {
        true => base.saturating_add(Duration::from_secs_f64(seconds)),
        false => base.saturating_sub(Duration::from_secs_f64(-seconds)),
    }
}

fn gain(level: f32) -> f32 {
    match level.clamp(0., 1.) {
        level if level <= 0. => 0.,
        level => 10f32.powf(TAPER_DB * (level - 1.) / 20.),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Idle,
    Playing,
    Paused,
    Loading,
    Failed(String),
}

pub enum PlaybackEvent {
    StartedPlayback,
    EndedPlayback,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeekStep {
    Five,
    #[default]
    Ten,
    Thirty,
}

impl SeekStep {
    pub const ALL: [Self; 3] = [Self::Five, Self::Ten, Self::Thirty];

    pub fn id(self) -> &'static str {
        match self {
            Self::Five => "5",
            Self::Ten => "10",
            Self::Thirty => "30",
        }
    }

    pub fn secs(self) -> u16 {
        match self {
            Self::Five => 5,
            Self::Ten => 10,
            Self::Thirty => 30,
        }
    }

    pub fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.secs()))
    }

    pub fn from_secs(secs: u16) -> Self {
        match secs {
            5 => Self::Five,
            30 => Self::Thirty,
            _ => Self::Ten,
        }
    }
}

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

// same thing, same origin
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Origin {
    pub whence: Whence,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<SharedString>,
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

    pub fn local_favorites() -> Self {
        Self::of(Whence::Local, LOCAL_FAVORITES)
    }

    pub fn named(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
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

pub struct Playback {
    state: PlaybackState,
    origin: Option<Origin>,
    position: Duration,
    clock: LiveClock,
    track: Option<Track>,
    engine: Option<Box<dyn Player>>,
    local_engine: Option<Box<dyn Player>>,
    session: Entity<Session>,
    queue: Entity<Queue>,
    settings: Entity<AppSettings>,
    level: f32,
    normalisation: bool,
    gapless: bool,
    repeat: Repeat,
    radio: bool,
    seeded: Option<String>,
    task: Option<Task<()>>,
    local_task: Option<Task<()>>,
    load: Option<Task<()>>,
    fetch: Option<Task<()>>,
    enqueue: Option<Task<()>>,
    suggest: Option<Task<()>>,
    preloaded: Option<String>,
    skipped: Option<Instant>,
    blocked_until: Option<Instant>,
    refused: Option<Refusal>,
    resume_at: Option<Duration>,
    seek_on_play: Option<Duration>,
    sought: Option<(Duration, Instant)>,
    resume_ready: bool,
    awaiting_reconnect: bool,
    stored: Duration,
}

impl EventEmitter<PlaybackEvent> for Playback {}

impl Playback {
    pub fn new(
        session: Entity<Session>,
        queue: Entity<Queue>,
        settings: Entity<AppSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, session, event, cx| match event {
            SessionEvent::SignedIn => {
                let Some(playback) = session.read(cx).playback() else {
                    return;
                };
                this.start_engine(playback, cx);
                this.adopt(cx);
            }
            SessionEvent::Reconnected => {
                let Some(playback) = session.read(cx).playback() else {
                    return;
                };
                this.rebind(playback, cx);
            }
            SessionEvent::SignedOut => this.teardown(cx),
            SessionEvent::LocalChanged => {
                if this.local_engine.is_none()
                    && let Some(playback) = session.read(cx).local_playback()
                {
                    this.start_local_engine(playback, cx);
                }
            }
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.suggest_similar(cx))
            .detach();

        let level = settings.read(cx).volume();
        let normalisation = settings.read(cx).normalisation();
        let gapless = settings.read(cx).gapless();
        let repeat = settings.read(cx).repeat();
        let radio = settings.read(cx).radio();

        Self {
            state: PlaybackState::Idle,
            origin: None,
            position: Duration::ZERO,
            clock: LiveClock::new(),
            track: None,
            engine: None,
            local_engine: None,
            session,
            queue,
            settings,
            level,
            normalisation,
            gapless,
            repeat,
            radio,
            seeded: None,
            task: None,
            local_task: None,
            load: None,
            fetch: None,
            enqueue: None,
            suggest: None,
            preloaded: None,
            skipped: None,
            blocked_until: None,
            refused: None,
            resume_at: None,
            seek_on_play: None,
            sought: None,
            resume_ready: false,
            awaiting_reconnect: false,
            stored: Duration::ZERO,
        }
    }

    pub fn play(&mut self, track: &Track, cx: &mut Context<Self>) {
        self.load_after(track, Start::Pick, cx);
    }

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

    pub fn spectrum(&self) -> Option<Spectrum> {
        self.active_engine()?.spectrum()
    }

    fn silence_other(&self, id: &str) {
        let other = match music::is_local_id(id) {
            true => self.engine.as_deref(),
            false => self.local_engine.as_deref(),
        };
        if let Some(engine) = other {
            engine.pause();
        }
    }

    pub fn preload(&mut self, track: &Track) {
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
        if self.preloaded.as_deref() == Some(id) {
            return;
        }
        self.preloaded = Some(id.to_owned());
        let Some(engine) = self.engine_for(id) else {
            return;
        };
        if let Err(error) = engine.preload(id) {
            self.preloaded = None;
            log::warn!("playback: cannot preload {}: {error:#}", track.name);
        }
    }

    fn load_after(&mut self, track: &Track, start: Start, cx: &mut Context<Self>) {
        match self.refused {
            Some(Refusal::Keys) => return self.refuse(cx),
            Some(Refusal::SignIn) => return self.gate(cx),
            None => {}
        }
        let Some(id) = track.id.clone() else {
            return self.failed(format!("{} has no track id", track.name), cx);
        };
        if !track.playable {
            return self.failed(format!("{} is not available to stream", track.name), cx);
        }
        if self.engine_for(&id).is_none() {
            return;
        }
        self.silence_other(&id);

        self.track = Some(track.clone());
        self.state = PlaybackState::Loading;
        self.position = Duration::ZERO;
        self.clock.reset(Duration::ZERO, false);
        self.preloaded = None;
        self.resume_at = None;
        self.seek_on_play = None;
        self.resume_ready = false;
        cx.notify();

        let wait = self
            .blocked_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
            .unwrap_or_default()
            .max(start.debounce());

        self.load = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                let Some(engine) = this.engine_for(&id) else {
                    return;
                };
                if let Err(error) = engine.load(&id, start == Start::Segue) {
                    this.failed(format!("{error:#}"), cx);
                }
            })
            .ok();
        }));
    }

    pub fn start(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        self.fetch = None;
        self.begin(tracks, index, origin, cx);
    }

    pub fn start_any(
        &mut self,
        tracks: Vec<Track>,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        let index = self.opener(&tracks, cx).unwrap_or_default();
        self.start(tracks, index, origin, cx);
    }

    fn opener(&self, tracks: &[Track], cx: &Context<Self>) -> Option<usize> {
        let playable = tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.playable)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match self.queue.read(cx).shuffle() {
            true => fastrand::choice(&playable).copied(),
            false => playable.first().copied(),
        }
    }

    pub fn play_radio(&mut self, seed: &Track, cx: &mut Context<Self>) {
        let Some(id) = seed.id.clone() else {
            return self.failed(format!("{} has no track id", seed.name), cx);
        };
        if !seed.playable {
            return self.failed(format!("{} is not available to stream", seed.name), cx);
        }

        let origin = Origin::radio(id.clone()).named(seed.name.clone());
        let Some(client) = self.client_for(&id, cx) else {
            return;
        };

        self.fetch = None;
        self.begin(vec![seed.clone()], 0, Some(origin.clone()), cx);

        let seed_id = seed.id.clone();
        let io = Io::global(cx);
        self.fetch = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move {
                let mut tracks = client.track_radio(&id).await?;
                tracks.retain(|track| track.id != seed_id && track.playable);
                fastrand::shuffle(&mut tracks);
                Ok(tracks)
            }))
            .await;

            this.update(cx, |this, cx| match loaded {
                Ok(tracks) if this.origin.as_ref() == Some(&origin) => {
                    this.queue
                        .update(cx, |queue, cx| queue.extend_context(tracks, cx));
                }
                Ok(_) => {}
                Err(error) => log::error!("playback: cannot load radio queue: {error:#}"),
            })
            .ok();
        }));
    }

    fn play_radio_of(&mut self, origin: Origin, cx: &mut Context<Self>) {
        let id = origin.id.clone();
        self.gather(origin, cx, move |client| {
            Box::pin(async move {
                let mut tracks = client.track_radio(&id).await?;
                tracks.retain(|track| track.playable);
                Ok(tracks)
            })
        });
    }

    pub fn enqueue(&mut self, track: Track, cx: &mut Context<Self>) {
        if self.queue.read(cx).current().is_none() {
            self.begin(vec![track], 0, None, cx);
            return;
        }
        let name = track.name.clone();
        let target = song_target(&track);
        self.queue.update(cx, |queue, cx| queue.append(track, cx));
        Toasts::linked(Outcome::Done, "toast-queued-track", name, target, cx);
    }

    pub fn play_next(&mut self, track: Track, cx: &mut Context<Self>) {
        if self.queue.read(cx).current().is_none() {
            self.begin(vec![track], 0, None, cx);
            return;
        }
        let name = track.name.clone();
        let target = song_target(&track);
        self.queue.update(cx, |queue, cx| queue.prepend(track, cx));
        Toasts::linked(Outcome::Done, "toast-next-track", name, target, cx);
    }

    pub fn enqueue_all(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        if self.queue.read(cx).current().is_none() {
            self.begin(tracks, 0, None, cx);
            return;
        }
        self.queue
            .update(cx, |queue, cx| queue.append_all(tracks, cx));
    }

    pub fn play_next_all(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        if self.queue.read(cx).current().is_none() {
            self.begin(tracks, 0, None, cx);
            return;
        }
        self.queue
            .update(cx, |queue, cx| queue.prepend_all(tracks, cx));
    }

    pub fn insert_all(&mut self, tracks: Vec<Track>, gap: usize, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        if self.queue.read(cx).current().is_none() {
            self.begin(tracks, 0, None, cx);
            return;
        }
        self.queue
            .update(cx, |queue, cx| queue.insert_upcoming(gap, tracks, cx));
    }

    pub fn enqueue_pin(&mut self, pin: &Pin, gap: Option<usize>, cx: &mut Context<Self>) {
        let placement = QueuePlacement::Gap(gap.unwrap_or(usize::MAX));
        let id = pin.id.clone();

        match pin.kind {
            PinKind::Song => {
                let track = id.clone();
                self.enqueue_from("track", &id, placement, cx, move |client| {
                    Box::pin(async move { client.track(&track).await.map(|track| vec![track]) })
                });
            }
            PinKind::Album => {
                let album = id.clone();
                self.enqueue_from("album", &id, placement, cx, move |client| {
                    Box::pin(async move { client.album_tracks(&album).await })
                });
            }
            PinKind::Playlist => {
                let playlist = id.clone();
                self.enqueue_from("playlist", &id, placement, cx, move |client| {
                    Box::pin(async move { client.playlist_tracks(&playlist).await })
                });
            }
            PinKind::Artist => {
                let artist = id.clone();
                self.enqueue_from("artist", &id, placement, cx, move |client| {
                    Box::pin(
                        async move { client.artist(&artist).await.map(|found| found.top_tracks) },
                    )
                });
            }
        }
    }

    pub fn enqueue_album(&mut self, album: &str, cx: &mut Context<Self>) {
        let id = album.to_owned();
        let album = album.to_owned();
        self.enqueue_from("album", &id, QueuePlacement::End, cx, move |client| {
            Box::pin(async move { client.album_tracks(&album).await })
        });
    }

    pub fn play_album_next(&mut self, album: &str, cx: &mut Context<Self>) {
        let id = album.to_owned();
        let album = album.to_owned();
        self.enqueue_from("album", &id, QueuePlacement::Next, cx, move |client| {
            Box::pin(async move { client.album_tracks(&album).await })
        });
    }

    fn play_album_of(&mut self, origin: Origin, cx: &mut Context<Self>) {
        let album = origin.id.clone();
        self.gather(origin, cx, move |client| {
            Box::pin(async move { client.album_tracks(&album).await })
        });
    }

    fn play_artist_of(&mut self, origin: Origin, cx: &mut Context<Self>) {
        let artist = origin.id.clone();
        self.gather(origin, cx, move |client| {
            Box::pin(async move { client.artist(&artist).await.map(|found| found.top_tracks) })
        });
    }

    pub fn play_artist_next(&mut self, artist: &str, cx: &mut Context<Self>) {
        let id = artist.to_owned();
        let artist = artist.to_owned();
        self.enqueue_from("artist", &id, QueuePlacement::Next, cx, move |client| {
            Box::pin(async move { client.artist(&artist).await.map(|found| found.top_tracks) })
        });
    }

    pub fn enqueue_artist(&mut self, artist: &str, cx: &mut Context<Self>) {
        let id = artist.to_owned();
        let artist = artist.to_owned();
        self.enqueue_from("artist", &id, QueuePlacement::End, cx, move |client| {
            Box::pin(async move { client.artist(&artist).await.map(|found| found.top_tracks) })
        });
    }

    pub fn enqueue_playlist(&mut self, playlist: &str, cx: &mut Context<Self>) {
        let id = playlist.to_owned();
        let playlist = playlist.to_owned();
        self.enqueue_from("playlist", &id, QueuePlacement::End, cx, move |client| {
            Box::pin(async move { client.playlist_tracks(&playlist).await })
        });
    }

    pub fn play_playlist_next(&mut self, playlist: &str, cx: &mut Context<Self>) {
        let id = playlist.to_owned();
        let playlist = playlist.to_owned();
        self.enqueue_from("playlist", &id, QueuePlacement::Next, cx, move |client| {
            Box::pin(async move { client.playlist_tracks(&playlist).await })
        });
    }

    fn play_playlist_of(&mut self, origin: Origin, cx: &mut Context<Self>) {
        let playlist = origin.id.clone();
        self.gather(origin, cx, move |client| {
            Box::pin(async move { client.playlist_tracks(&playlist).await })
        });
    }

    fn client_for(&self, id: &str, cx: &Context<Self>) -> Option<Arc<dyn MusicApi>> {
        let session = self.session.read(cx);
        match music::is_local_id(id) {
            true => session.local_client(),
            false => session.client(),
        }
    }

    fn enqueue_from<F>(
        &mut self,
        source: &'static str,
        id: &str,
        placement: QueuePlacement,
        cx: &mut Context<Self>,
        tracks: F,
    ) where
        F: FnOnce(Arc<dyn MusicApi>) -> Fetch + Send + 'static,
    {
        if self.enqueue.is_some() {
            return;
        }
        let Some(client) = self.client_for(id, cx) else {
            return;
        };
        let io = Io::global(cx);
        self.enqueue = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move { tracks(client).await })).await;
            this.update(cx, |this, cx| {
                this.enqueue = None;
                match loaded {
                    Ok(tracks) => {
                        let queued = this.queue.read(cx).current().is_some();
                        match placement {
                            QueuePlacement::Next => this.play_next_all(tracks, cx),
                            QueuePlacement::End => this.enqueue_all(tracks, cx),
                            QueuePlacement::Gap(gap) => this.insert_all(tracks, gap, cx),
                        }
                        if queued && let Some(key) = placement.toast(source) {
                            Toasts::show(Outcome::Done, key, cx);
                        }
                    }
                    Err(error) => {
                        log::error!("playback: cannot enqueue {source}: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-queue-failed", cx);
                    }
                }
            })
            .ok();
        }));
    }

    pub fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }

    pub fn playing_from(&self, origin: &Origin) -> Option<PlaybackState> {
        (self.origin.as_ref() == Some(origin)).then(|| self.state.clone())
    }

    fn begin(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        let Some(track) = self
            .queue
            .update(cx, |queue, cx| queue.start(tracks, index, cx))
        else {
            return;
        };
        self.origin = origin;
        let stored = self.origin.clone();
        self.settings
            .update(cx, |settings, cx| settings.set_resume_origin(stored, cx));
        self.play(&track, cx);
    }

    fn gather<F>(&mut self, origin: Origin, cx: &mut Context<Self>, tracks: F)
    where
        F: FnOnce(Arc<dyn MusicApi>) -> Fetch + Send + 'static,
    {
        let Some(client) = self.client_for(&origin.id, cx) else {
            return;
        };

        let io = Io::global(cx);
        if !self.has_active_playback() {
            self.state = PlaybackState::Loading;
            cx.notify();
        }

        self.fetch = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move { tracks(client).await })).await;

            this.update(cx, |this, cx| match loaded {
                Ok(tracks) => {
                    let index = this.opener(&tracks, cx).unwrap_or_default();
                    this.begin(tracks, index, Some(origin), cx)
                }
                Err(error) if this.has_active_playback() => {
                    log::error!("playback: cannot load context: {error:#}");
                }
                Err(error) => this.failed(format!("{error:#}"), cx),
            })
            .ok();
        }));
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.fetch = None;
        let start = self.burst();
        self.follow_queue(start, cx);
    }

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

    fn preload_next(&mut self, position: Duration, cx: &Context<Self>) {
        let Some(duration) = self.track.as_ref().map(|track| track.duration) else {
            return;
        };
        if duration.is_zero()
            || self.state != PlaybackState::Playing
            || duration.saturating_sub(position) > PRELOAD_BEFORE_END
        {
            return;
        }

        let next = match self.repeat != Repeat::One {
            true => self.queue.read(cx).upcoming().next().cloned(),
            false => None,
        };
        let Some(next) = next else {
            self.preloaded = None;
            return;
        };

        self.preload(&next);
    }

    pub fn radio(&self) -> bool {
        self.radio
    }

    pub fn toggle_radio(&mut self, cx: &mut Context<Self>) {
        self.radio = !self.radio;
        let radio = self.radio;
        self.settings
            .update(cx, |settings, cx| settings.set_radio(radio, cx));
        match radio {
            true => self.suggest_similar(cx),
            false => self.forget_similar(cx),
        }
    }

    pub fn play_similar(&mut self, index: usize, cx: &mut Context<Self>) {
        self.fetch = None;
        let Some(track) = self
            .queue
            .update(cx, |queue, cx| queue.play_similar(index, cx))
        else {
            return;
        };
        self.load_after(&track, Start::Pick, cx);
    }

    fn forget_similar(&mut self, cx: &mut Context<Self>) {
        self.seeded = None;
        self.suggest = None;
        self.queue.update(cx, |queue, cx| queue.clear_similar(cx));
        cx.notify();
    }

    fn seed(&self, cx: &Context<Self>) -> Option<Track> {
        let queue = self.queue.read(cx);
        queue.upcoming().last().or_else(|| queue.current()).cloned()
    }

    fn suggest_similar(&mut self, cx: &mut Context<Self>) {
        if !self.radio || self.queue.read(cx).similar().len() > 0 {
            return;
        }
        let Some(id) = self.seed(cx).and_then(|seed| seed.id) else {
            return self.forget_similar(cx);
        };
        if self.seeded.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(client) = self.client_for(&id, cx) else {
            return self.forget_similar(cx);
        };

        let queued = self.queue.read(cx).ids();
        self.seeded = Some(id.clone());
        let io = Io::global(cx);
        self.suggest = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move {
                let mut tracks = client.track_radio(&id).await?;
                tracks.retain(|track| {
                    track.playable
                        && track
                            .id
                            .as_ref()
                            .is_some_and(|id| !queued.contains(id.as_str()))
                });
                fastrand::shuffle(&mut tracks);
                tracks.truncate(SIMILAR_LIMIT);
                anyhow::Ok(tracks)
            }))
            .await;

            this.update(cx, |this, cx| match loaded {
                Ok(_) if !this.radio => {}
                Ok(tracks) => this.queue.update(cx, |queue, cx| queue.suggest(tracks, cx)),
                Err(error) => log::warn!("playback: cannot load similar tracks: {error:#}"),
            })
            .ok();
        }));
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        let repeat = self.repeat;
        self.settings
            .update(cx, |settings, cx| settings.set_repeat(repeat, cx));
        cx.notify();
    }

    fn advance(&mut self, ended: Option<Track>, cx: &mut Context<Self>) {
        match self.repeat {
            Repeat::One => match ended {
                Some(track) => self.load_after(&track, Start::Segue, cx),
                None => self.segue_queue(cx),
            },
            Repeat::All if !self.queue.read(cx).has_next() => {
                self.fetch = None;
                if let Some(track) = self.queue.update(cx, |queue, cx| queue.rewind(cx)) {
                    self.follow_after(track, Start::Segue, cx);
                }
            }
            _ if self.radio && !self.queue.read(cx).has_next() => {
                match ended.or_else(|| self.track.clone()) {
                    Some(seed) => self.extend_radio(&seed, cx),
                    None => self.segue_queue(cx),
                }
            }
            _ => self.segue_queue(cx),
        }
    }

    fn segue_queue(&mut self, cx: &mut Context<Self>) {
        self.fetch = None;
        self.follow_queue(Start::Segue, cx);
    }

    fn extend_radio(&mut self, seed: &Track, cx: &mut Context<Self>) {
        let Some(id) = seed.id.clone() else {
            return self.next(cx);
        };
        let Some(client) = self.client_for(&id, cx) else {
            return self.next(cx);
        };

        let io = Io::global(cx);
        let heard = seed.id.clone();
        self.fetch = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move {
                let mut tracks = client.track_radio(&id).await?;
                tracks.retain(|track| track.id != heard && track.playable);
                fastrand::shuffle(&mut tracks);
                anyhow::Ok(tracks)
            }))
            .await;

            this.update(cx, |this, cx| match loaded {
                Ok(tracks) if !tracks.is_empty() => {
                    this.queue.update(cx, |queue, cx| {
                        for track in tracks {
                            queue.append(track, cx);
                        }
                    });
                    this.follow_queue(Start::Segue, cx);
                }
                Ok(_) => log::warn!("playback: radio returned no tracks"),
                Err(error) => log::warn!("playback: cannot extend radio: {error:#}"),
            })
            .ok();
        }));
    }

    fn follow_queue(&mut self, start: Start, cx: &mut Context<Self>) {
        let Some(track) = self.queue.update(cx, |queue, cx| queue.next(cx)) else {
            return;
        };
        self.follow_after(track, start, cx);
    }

    fn follow_after(&mut self, mut track: Track, start: Start, cx: &mut Context<Self>) {
        while !track.playable {
            let Some(next) = self.queue.update(cx, |queue, cx| queue.next(cx)) else {
                return;
            };
            track = next;
        }
        self.load_after(&track, start, cx);
    }

    pub fn has_previous(&self, cx: &App) -> bool {
        self.track.is_some() || self.queue.read(cx).has_previous()
    }

    fn restarts(&self, cx: &App) -> bool {
        self.track.is_some()
            && (self.live_position() > RESTART_WINDOW || !self.queue.read(cx).has_previous())
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        if self.restarts(cx) {
            return self.seek(Duration::ZERO, cx);
        }
        self.fetch = None;
        let start = self.burst();
        let Some(track) = self.queue.update(cx, |queue, cx| queue.previous(cx)) else {
            return;
        };
        self.load_after(&track, start, cx);
    }

    pub fn play_past(&mut self, index: usize, cx: &mut Context<Self>) {
        self.fetch = None;
        let Some(track) = self
            .queue
            .update(cx, |queue, cx| queue.play_past(index, cx))
        else {
            return;
        };
        self.load_after(&track, Start::Pick, cx);
    }

    pub fn play_upcoming(&mut self, index: usize, cx: &mut Context<Self>) {
        self.fetch = None;
        let Some(track) = self
            .queue
            .update(cx, |queue, cx| queue.play_upcoming(index, cx))
        else {
            return;
        };
        self.load_after(&track, Start::Pick, cx);
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.resume_at {
            if !self.resume_ready {
                return self.reload_and_seek(at, cx);
            }
            self.resume_at = None;
            self.resume_ready = false;
        }
        if let Some(engine) = self.active_engine() {
            engine.play();
            cx.notify();
        }
    }

    fn reload_and_seek(&mut self, at: Duration, cx: &mut Context<Self>) {
        let Some(track) = self.track.clone() else {
            return;
        };
        if self.active_engine().is_none() {
            return;
        }
        self.load_after(&track, Start::Pick, cx);
        self.seek_on_play = Some(at);
    }

    fn prepare_resume(&mut self, cx: &mut Context<Self>) {
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
        cx.notify();
    }

    fn adopt(&mut self, cx: &mut Context<Self>) {
        if self.track.is_some() || !self.queue.read(cx).is_empty() {
            return;
        }
        let Some(slug) = self.session.read(cx).provider_slug() else {
            return;
        };
        let Some(resume) = self
            .settings
            .read(cx)
            .resume()
            .filter(|resume| resume.provider == slug)
            .cloned()
        else {
            return;
        };

        let at = Duration::from_secs_f32(resume.position.max(0.));
        let origin = resume.origin.clone();
        let Some(track) = self.queue.update(cx, |queue, cx| queue.revive(resume, cx)) else {
            return;
        };
        self.origin = origin;
        self.track = Some(track);
        self.state = PlaybackState::Paused;
        self.position = at;
        self.clock.reset(at, false);
        self.stored = at;
        self.resume_at = Some(at);
        self.prepare_resume(cx);
    }

    fn remember(&mut self, force: bool, cx: &mut Context<Self>) {
        let position = self.position;
        if !force && position.abs_diff(self.stored) < RESUME_STEP {
            return;
        }
        self.stored = position;
        let seconds = position.as_secs_f32();
        self.settings
            .update(cx, |settings, cx| settings.set_resume_position(seconds, cx));
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if let Some(engine) = self.active_engine() {
            engine.pause();
            cx.notify();
        }
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.state == PlaybackState::Playing {
            self.pause(cx);
        } else {
            self.resume(cx);
        }
    }

    pub fn play_origin(&mut self, origin: Origin, cx: &mut Context<Self>) {
        match origin.whence {
            Whence::Album => self.play_album_of(origin, cx),
            Whence::Playlist => self.play_playlist_of(origin, cx),
            Whence::Artist => self.play_artist_of(origin, cx),
            Whence::Radio => self.play_radio_of(origin, cx),
            // the table plays these
            Whence::Saved | Whence::Local => {}
        }
    }

    pub fn toggle_origin(&mut self, origin: &Origin, cx: &mut Context<Self>) {
        match self.playing_from(origin) {
            Some(PlaybackState::Playing) => self.pause(cx),
            Some(PlaybackState::Paused) => self.resume(cx),
            _ => self.play_origin(origin.clone(), cx),
        }
    }

    pub fn seek(&mut self, position: Duration, cx: &mut Context<Self>) {
        if self.state != PlaybackState::Playing {
            self.seek_on_play = Some(position);
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
            self.sought = Some((position, Instant::now()));
            self.remember(true, cx);
            cx.notify();
            return;
        }
        if let Some(engine) = self.active_engine() {
            engine.seek(position);
            self.position = position;
            self.clock
                .reset(position, self.state == PlaybackState::Playing);
            self.sought = Some((position, Instant::now()));
            cx.notify();
        }
    }

    pub fn seek_fraction(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let Some(total) = self
            .track
            .as_ref()
            .map(|track| track.duration)
            .filter(|total| !total.is_zero())
        else {
            return;
        };

        let position = Duration::from_secs_f32(total.as_secs_f32() * fraction.clamp(0., 1.));
        self.seek(position, cx);
    }

    pub fn seek_by(&mut self, step: Duration, forward: bool, cx: &mut Context<Self>) {
        let Some(end) = self.track.as_ref().map(|track| track.duration) else {
            return;
        };
        let at = self.live_position();
        let target = match forward {
            true => at.saturating_add(step).min(end),
            false => at.saturating_sub(step),
        };
        self.seek(target, cx);
    }

    pub fn seek_back(&mut self, cx: &mut Context<Self>) {
        let step = self.settings.read(cx).seek_step().duration();
        self.seek_by(step, false, cx);
    }

    pub fn seek_forward(&mut self, cx: &mut Context<Self>) {
        let step = self.settings.read(cx).seek_step().duration();
        self.seek_by(step, true, cx);
    }

    fn holding_seek(&self, reported: Duration) -> bool {
        let Some((target, since)) = self.sought else {
            return false;
        };
        since.elapsed() < SEEK_HOLD && away(reported, target) > SEEK_CATCHUP
    }

    pub fn state(&self) -> &PlaybackState {
        &self.state
    }

    pub fn position(&self) -> Duration {
        self.position
    }

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

    pub fn progress(&self) -> f32 {
        let Some(total) = self.track.as_ref().map(|track| track.duration) else {
            return 0.;
        };
        if total.is_zero() {
            return 0.;
        }
        (self.position.as_secs_f32() / total.as_secs_f32()).clamp(0., 1.)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, PlaybackState::Loading)
    }

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

    pub fn set_volume(&mut self, level: f32, cx: &mut Context<Self>) {
        self.level = level.clamp(0., 1.);
        self.settings
            .update(cx, |settings, cx| settings.set_volume(self.level, cx));
        let level = gain(self.level);
        if let Some(engine) = self.engine.as_ref() {
            engine.set_gain(level);
        }
        if let Some(engine) = self.local_engine.as_ref() {
            engine.set_gain(level);
        }
        cx.notify();
    }

    pub fn normalisation(&self) -> bool {
        self.normalisation
    }

    pub fn gapless(&self) -> bool {
        self.gapless
    }

    pub fn set_gapless(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.gapless == on {
            return;
        }
        self.gapless = on;
        self.settings
            .update(cx, |settings, cx| settings.set_gapless(on, cx));
        self.restart_engine(cx);
    }

    pub fn set_normalisation(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.normalisation == on {
            return;
        }
        self.normalisation = on;
        self.settings
            .update(cx, |settings, cx| settings.set_normalisation(on, cx));
        self.restart_engine(cx);
    }

    fn local_active(&self) -> bool {
        self.track
            .as_ref()
            .and_then(|track| track.id.as_deref())
            .is_some_and(music::is_local_id)
    }

    fn restart_engine(&mut self, cx: &mut Context<Self>) {
        let playback = match self.engine.is_some() {
            true => self.session.read(cx).playback(),
            false => None,
        };
        let Some(playback) = playback else {
            return cx.notify();
        };

        match self.local_active() {
            true => self.start_engine(playback, cx),
            false => self.rebind(playback, cx),
        }
    }

    fn restart_output(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.track.clone() else {
            return;
        };
        let Some(id) = track.id.as_deref() else {
            return;
        };
        let at = self.live_position();
        let local = music::is_local_id(id);
        let playback = match local {
            true => self.session.read(cx).local_playback(),
            false => self.session.read(cx).playback(),
        };
        let Some(playback) = playback else {
            return;
        };

        log::info!("playback: restarting after the audio output changed");
        match local {
            true => {
                self.local_task = None;
                self.local_engine = None;
                self.start_local_engine(playback, cx);
            }
            false => {
                self.task = None;
                self.engine = None;
                self.start_engine(playback, cx);
            }
        }
        self.load_after(&track, Start::Pick, cx);
        self.seek_on_play = Some(at);
    }

    fn ask_for_reconnect(&mut self, cx: &mut Context<Self>) -> bool {
        if self.track.is_none() {
            return false;
        }
        self.awaiting_reconnect = self
            .session
            .update(cx, |session, cx| session.reconnect_if_stale(cx));
        self.awaiting_reconnect
    }

    fn rebind(&mut self, playback: Arc<dyn PlaybackFactory>, cx: &mut Context<Self>) {
        let resume = self.state == PlaybackState::Playing || self.awaiting_reconnect;
        self.awaiting_reconnect = false;
        let at = self.position;
        self.task = None;
        self.engine = None;
        self.preloaded = None;
        self.blocked_until = None;
        if self.track.is_some() {
            self.resume_at = Some(at);
        }
        self.start_engine(playback, cx);
        if resume {
            self.resume(cx);
        }
    }

    fn start_engine(&mut self, playback: Arc<dyn PlaybackFactory>, cx: &mut Context<Self>) {
        let config = PlaybackConfig {
            normalisation: self.normalisation,
            gapless: self.gapless,
            position_interval: POSITION_INTERVAL,
            gain: gain(self.level),
        };
        let (engine, events) = playback.start(config);

        self.listen(events, false, cx);
        self.engine = Some(engine);
        self.refused = None;
        if !self.local_active() {
            self.state = PlaybackState::Idle;
            self.position = Duration::ZERO;
            self.clock.reset(Duration::ZERO, false);
        }
        self.prepare_resume(cx);
        cx.notify();
    }

    fn start_local_engine(&mut self, playback: Arc<dyn PlaybackFactory>, cx: &mut Context<Self>) {
        let config = PlaybackConfig {
            normalisation: self.normalisation,
            gapless: self.gapless,
            position_interval: POSITION_INTERVAL,
            gain: gain(self.level),
        };
        let (engine, events) = playback.start(config);

        self.listen(events, true, cx);
        self.local_engine = Some(engine);
        self.prepare_resume(cx);
    }

    fn listen(&mut self, mut events: Box<dyn PlaybackEvents>, local: bool, cx: &mut Context<Self>) {
        let task = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this, cx| this.on_backend_event(event, local, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        match local {
            true => self.local_task = task,
            false => self.task = task,
        }
    }

    fn on_backend_event(&mut self, event: BackendEvent, local: bool, cx: &mut Context<Self>) {
        if local != self.local_active() {
            return;
        }
        match event {
            BackendEvent::OutputChanged => self.restart_output(cx),
            BackendEvent::Unavailable | BackendEvent::Refused if self.resume_ready => {
                self.resume_ready = false;
                self.state = PlaybackState::Paused;
                log::warn!("playback: cannot hold the restored track, waiting for play");
            }
            BackendEvent::Loading(position) => {
                self.state = PlaybackState::Loading;
                self.position = position;
                self.clock.reset(position, false);
            }
            BackendEvent::Playing(position) => {
                let started = self.state != PlaybackState::Playing;
                self.state = PlaybackState::Playing;
                self.position = position;
                self.clock.reset(position, true);
                if let Some(at) = self.seek_on_play.take() {
                    self.seek(at, cx);
                }
                if started {
                    cx.emit(PlaybackEvent::StartedPlayback);
                }
            }
            BackendEvent::Paused(position) => {
                self.state = PlaybackState::Paused;
                self.position = position;
                self.clock.reset(position, false);
                self.remember(true, cx);
            }
            BackendEvent::Position(position) if self.holding_seek(position) => {}
            BackendEvent::Position(position) => {
                self.sought = None;
                self.position = position;
                match self.state == PlaybackState::Playing {
                    true => self.clock.correct(position),
                    false => self.clock.reset(position, false),
                }
                self.remember(false, cx);
                self.preload_next(position, cx);
            }
            BackendEvent::Length(duration) => {
                if let Some(track) = self.track.as_mut()
                    && !duration.is_zero()
                    && track.duration != duration
                {
                    track.duration = duration;
                }
            }
            BackendEvent::Ended => {
                let ended = self.track.take();
                self.state = PlaybackState::Idle;
                self.position = Duration::ZERO;
                self.clock.reset(Duration::ZERO, false);
                cx.emit(PlaybackEvent::EndedPlayback);
                self.advance(ended, cx);
            }
            BackendEvent::Unavailable if self.ask_for_reconnect(cx) => {
                self.state = PlaybackState::Loading;
                log::warn!("playback: the provider went stale, waiting for a reconnect");
            }
            BackendEvent::Unavailable => {
                let failed = self.track.take();
                let target = failed.as_ref().and_then(song_target);
                let name = failed.map_or_else(|| "?".to_owned(), |track| track.name);
                log::warn!(
                    "playback: {name} failed to load, backing off {}s",
                    KEY_COOLDOWN.as_secs()
                );
                self.blocked_until = Some(Instant::now() + KEY_COOLDOWN);
                self.state = PlaybackState::Idle;
                self.position = Duration::ZERO;
                self.clock.reset(Duration::ZERO, false);
                Toasts::linked(Outcome::Failed, "toast-track-unplayable", name, target, cx);
                cx.emit(PlaybackEvent::EndedPlayback);
            }
            BackendEvent::Refused => {
                self.refuse(cx);
                cx.emit(PlaybackEvent::EndedPlayback);
            }
            BackendEvent::Gated => {
                self.gate(cx);
                cx.emit(PlaybackEvent::EndedPlayback);
            }
        }
        cx.notify();
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.engine = None;

        if !self.local_active() {
            self.load = None;
            self.fetch = None;
            self.enqueue = None;
            self.suggest = None;
            self.seeded = None;
            self.preloaded = None;
            self.skipped = None;
            self.blocked_until = None;
            self.refused = None;
            self.track = None;
            self.origin = None;
            self.state = PlaybackState::Idle;
            self.position = Duration::ZERO;
            self.clock.reset(Duration::ZERO, false);
            self.resume_at = None;
            self.seek_on_play = None;
            self.sought = None;
            self.resume_ready = false;
            self.awaiting_reconnect = false;
            self.stored = Duration::ZERO;
        }
        cx.notify();
    }

    fn failed(&mut self, problem: String, cx: &mut Context<Self>) {
        log::error!("playback: {problem}");
        self.state = PlaybackState::Failed(problem);
        cx.notify();
    }

    fn gate(&mut self, cx: &mut Context<Self>) {
        let first = self.refused.is_none();
        self.refused = Some(Refusal::SignIn);
        self.track = None;
        self.blocked_until = None;
        let provider = self
            .session
            .read(cx)
            .provider_name()
            .unwrap_or("this provider");
        self.state = PlaybackState::Failed(format!(
            "{provider} only streams to a signed-in listener; nothing will play until you sign in"
        ));
        if first {
            log::warn!(
                "playback: {provider} only streams to a signed-in listener, nothing will play \
                 until you sign in"
            );
        }
        Toasts::about(
            Outcome::Failed,
            "toast-sign-in-to-play",
            provider.to_owned(),
            cx,
        );
        cx.notify();
    }

    fn refuse(&mut self, cx: &mut Context<Self>) {
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
        Toasts::show(Outcome::Failed, "toast-keys-refused", cx);
        cx.notify();
    }
}

fn song_target(track: &Track) -> Option<Target> {
    track
        .id
        .as_deref()
        .map(|id| Target::Song(SharedString::from(id.to_owned())))
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
