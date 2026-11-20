//! Runs the Soloist daemon and translates its WebSocket reports into playback events.
//! The account connection publishes the queue while this bridge controls the audio engine.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use futures::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::daemon::{RUNNING, follow, launch, stop};
use super::wire::{self, Event, Options, Playback, Queued, Status};
use super::{QUALITIES, binary, data, key, paired_account, prefs, recheck};
use crate::connect::{Collection, PREVIOUS, QueuedTrack, Repeat, Snapshot, Transport, UPCOMING};
use crate::spotify::connect_collection;
use crate::{Normalisation, PlaybackConfig, PlaybackEvent, Quality, TrackSignal};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const TRACK_PREFIX: &str = "spotify:track:";
/// The most audio the daemon keeps on disk, in megabytes. It keeps everything without a limit.
const CACHE_MB: &str = "512";
/// The files the daemon writes its WebSocket address into once it listens.
const WS_ADDR: &str = "ws.addr";
const WS_PORT: &str = "ws.port";
/// What the daemon decodes every track to.
const RATE: u32 = 44_100;
const CHANNELS: u16 = 2;

/// How long the daemon gets to open its WebSocket.
const STARTUP: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
/// How often seeks are looked at again.
const TICK: Duration = Duration::from_millis(250);
const SEEK_RETRY: Duration = Duration::from_millis(500);
/// How long a seek is waited on before the track is let through wherever it stands.
const SEEK_PATIENCE: Duration = Duration::from_secs(15);
/// How close to a seek's target a position report has to be for the seek to have landed.
const SEEK_NEAR: Duration = Duration::from_secs(3);
/// A start this close to the beginning needs no seek.
const OPENING: Duration = Duration::from_secs(1);
/// How long the daemon gets to acknowledge the command that starts a track before its reports
/// are trusted without one.
const ACK_PATIENCE: Duration = Duration::from_secs(3);
/// How many sign-ins in a row Spotify may turn down before the setup is taken to be broken. One
/// refusal can be a passing fault the next start does not repeat.
const REFUSALS: u8 = 2;

/// What the daemon prints when Spotify turns its sign-in down.
const REFUSED: &str = "login failed";
/// What it prints while it has no sign-in to restore, and also while it is still restoring one.
const UNPAIRED: &str = "waiting for login";

/// Transport control of a Soloist daemon. Dropping it stops the daemon.
pub(crate) struct Engine {
    commands: UnboundedSender<Command>,
    device: Arc<OnceLock<String>>,
}

impl Engine {
    /// Plays a track from `at`, or with `play` off keeps it ready there for `play`.
    pub(crate) fn load(&self, track_id: &str, at: Duration, play: bool) {
        self.ask(Command::Load {
            id: track_id.to_owned(),
            at,
            play,
        });
    }

    pub(crate) fn connect_device(&self) -> Option<&str> {
        self.device.get().map(String::as_str)
    }

    pub(crate) fn ask(&self, command: Command) {
        self.commands.send(command).ok();
    }
}

/// Builds the engine and its daemon task for the provider's Tokio runtime.
pub(crate) fn start(
    config: &PlaybackConfig,
) -> (
    Engine,
    UnboundedReceiver<PlaybackEvent>,
    impl Future<Output = ()> + Send + use<>,
) {
    let setup = Setup {
        normalise: config.normalisation,
        gapless: config.gapless,
        quality: config.quality,
        interval: config.position_interval,
    };
    let (commands, asked) = unbounded_channel();
    let (events, reported) = unbounded_channel();
    let brain = Brain::new(setup, percent(config.gain), events);
    let engine = Engine {
        commands,
        device: brain.device.clone(),
    };
    (engine, reported, brain.run(asked))
}

/// A transport command sent to the Soloist task.
pub(crate) enum Command {
    Load {
        id: String,
        at: Duration,
        play: bool,
    },
    Play,
    Pause,
    Seek(Duration),
    Gain(f32),
    RepeatOne(bool),
    RefreshQueue,
}

/// What a daemon is started with, beyond the key.
#[derive(Clone, Copy)]
struct Setup {
    normalise: bool,
    gapless: bool,
    quality: Quality,
    /// How often progress is reported while a track plays.
    interval: Duration,
}

/// The track Sonora asked for and how far the daemon has got with it.
struct Track {
    id: String,
    /// Whether the daemon has been told to play it.
    sent: bool,
    /// Waits for the daemon to accept a local start before trusting its reports.
    starting: Option<(&'static str, Instant)>,
    /// Whether the daemon has said it plays it.
    started: bool,
    /// Whether the listener wants it paused.
    held: bool,
    /// Whether `Playing` was the last state reported, so each change is reported once.
    playing: bool,
    seek: Option<Seek>,
    /// Where it stood at `stamp`.
    position: Duration,
    stamp: Instant,
}

impl Track {
    fn new(id: String, at: Duration, held: bool) -> Self {
        Self {
            id,
            sent: false,
            starting: None,
            started: false,
            held,
            playing: false,
            seek: None,
            position: at,
            stamp: Instant::now(),
        }
    }

    /// Where the track is now: the last report, run on by the clock while it plays.
    fn live(&self) -> Duration {
        match self.playing {
            true => self.position + self.stamp.elapsed(),
            false => self.position,
        }
    }

    fn place(&mut self, at: Duration) {
        self.position = at;
        self.stamp = Instant::now();
    }

    /// Puts the track back to before the daemon heard of it, to start again from where it is.
    fn rewind(&mut self) {
        self.place(self.live());
        self.sent = false;
        self.starting = None;
        self.started = false;
        self.playing = false;
        self.seek = None;
    }
}

/// A seek the daemon has not confirmed yet.
struct Seek {
    to: Duration,
    /// When it was first and last sent.
    since: Instant,
    asked: Instant,
    /// Whether the daemon took it. It turns a seek down while the track is still loading, and
    /// a position reported before it took one says nothing of where the seek went.
    taken: bool,
    /// Whether it is the seek that starts a track somewhere other than its beginning, which
    /// ends in `Playing` rather than `Seeked`.
    opening: bool,
}

impl Seek {
    fn new(to: Duration, opening: bool) -> Self {
        let now = Instant::now();
        Self {
            to,
            since: now,
            asked: now,
            taken: false,
            opening,
        }
    }
}

/// What the daemon last said it is doing.
#[derive(Default)]
struct Heard {
    uri: String,
    context: Option<Collection>,
    shuffle: Option<bool>,
    repeat: Option<Repeat>,
    status: Status,
    position: Duration,
}

/// The engine's own loop: commands in, the daemon's reports in, events out.
struct Brain {
    setup: Setup,
    events: UnboundedSender<PlaybackEvent>,
    device: Arc<OnceLock<String>>,
    link: Option<Link>,
    /// Whether the daemon is signed in, before which it takes no command.
    signed_in: bool,
    /// Sonora's volume, from 0 to 100.
    volume: u8,
    /// Whether the daemon's volume is held at zero while an opening seek lands, so the start of
    /// the track is not heard before the place it was asked to start from.
    muted: bool,
    track: Option<Track>,
    heard: Heard,
    active: bool,
    syncing: bool,
    repeat_one: bool,
    next: Option<String>,
    /// While refreshing, tracks whether the playback reply has arrived before the queue reply.
    queue_requested: Option<bool>,
    /// When progress was last reported.
    reported: Instant,
    /// Sign-ins Spotify turned down since the last one it took.
    refusals: u8,
    /// Whether a too-old build was already replaced once, so it is not fetched in a loop.
    refreshed: bool,
    /// Whether the setup proved broken. Nothing is started again until a new engine.
    dead: bool,
}

impl Brain {
    fn new(setup: Setup, volume: u8, events: UnboundedSender<PlaybackEvent>) -> Self {
        Self {
            setup,
            events,
            device: Arc::default(),
            link: None,
            signed_in: false,
            volume,
            muted: false,
            track: None,
            heard: Heard::default(),
            active: false,
            syncing: false,
            repeat_one: false,
            next: None,
            queue_requested: None,
            reported: Instant::now(),
            refusals: 0,
            refreshed: false,
            dead: false,
        }
    }

    async fn run(mut self, mut commands: UnboundedReceiver<Command>) {
        self.wake();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.command(command),
                    None => break,
                },
                incoming = next(&mut self.link) => self.incoming(incoming),
                _ = ticker.tick() => self.tick(),
            }
        }
        if let Some(link) = self.link.take() {
            link.close().await;
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Load { id, at, play } => self.load(id, at, play),
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::Seek(to) => self.seek(to),
            Command::Gain(gain) => {
                let volume = percent(gain);
                self.volume = volume;
                if !self.muted {
                    self.send("set_volume", json!({ "volume": volume }));
                }
            }
            Command::RefreshQueue => self.refresh_queue(),
            Command::RepeatOne(enabled) => {
                if self.repeat_one != enabled {
                    self.repeat_one = enabled;
                    self.send("set_repeat_track", json!({ "enabled": self.repeat_one }));
                }
            }
        }
    }

    fn load(&mut self, id: String, at: Duration, play: bool) {
        if self.dead {
            return self.report(PlaybackEvent::Refused);
        }
        self.unmute();
        self.report(PlaybackEvent::Loading {
            id: Some(id.clone()),
            at,
        });
        self.report(PlaybackEvent::Signal {
            id: id.clone(),
            track: self.signal(),
        });
        self.track = Some(Track::new(id.clone(), at, !play));
        match play {
            true => self.advance(),
            false => {
                if self.active {
                    self.send("pause", json!({}));
                }
                self.report(PlaybackEvent::Paused { id: Some(id), at });
            }
        }
    }

    /// Hands the wanted track to the daemon once both are ready for it.
    fn advance(&mut self) {
        self.wake();
        if !self.signed_in {
            return;
        }
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if track.held || track.sent {
            return;
        }
        track.sent = true;
        let uri = format!("{TRACK_PREFIX}{}", track.id);
        let opening = track.position > OPENING;
        let starter = match self.active && !opening && self.heard.uri != uri {
            true if self.next.as_ref() == Some(&uri) => "skip_next",
            _ => "play",
        };
        let now = Instant::now();
        track.starting = Some((starter, now));
        self.heard.status = Status::Unknown;
        self.heard.position = Duration::ZERO;
        if opening {
            self.mute();
        }
        if !self.active {
            self.send("activate", json!({}));
        }
        self.send(
            starter,
            match starter {
                "play" => json!({ "uri": uri }),
                _ => json!({}),
            },
        );
    }

    fn play(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        track.held = false;
        match track.started {
            true => self.send("play", json!({})),
            false => self.advance(),
        }
    }

    fn pause(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        track.held = true;
        if track.started {
            return self.send("pause", json!({}));
        }
        if !track.sent {
            let (id, at) = (Some(track.id.clone()), track.position);
            self.report(PlaybackEvent::Paused { id, at });
        }
    }

    fn seek(&mut self, to: Duration) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !track.started {
            track.place(to);
            let (id, sent) = (Some(track.id.clone()), track.sent);
            return match sent {
                // the daemon is loading it, and the opening seek takes it there once it plays
                true if to > OPENING => self.mute(),
                true => {}
                false => self.report(PlaybackEvent::Seeked { id, at: to }),
            };
        }
        track.seek = Some(Seek::new(to, false));
        self.send("seek", json!({ "position_ms": millis(to) }));
    }

    /// Sends an unconfirmed seek again, gives up on one that never lands, and reports progress.
    fn tick(&mut self) {
        let now = Instant::now();
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if track
            .starting
            .is_some_and(|(_, asked)| now.duration_since(asked) > ACK_PATIENCE)
        {
            log::debug!("soloist: the daemon acknowledged no start, going by its reports");
            track.starting = None;
            self.send("get_state", json!({}));
            return;
        }
        if let Some(seek) = track.seek.as_mut() {
            if now.duration_since(seek.since) > SEEK_PATIENCE {
                log::warn!("soloist: the daemon never confirmed a seek");
                return self.landed();
            }
            if now.duration_since(seek.asked) >= SEEK_RETRY {
                seek.asked = now;
                let to = seek.to;
                self.send("seek", json!({ "position_ms": millis(to) }));
            }
            return;
        }
        if track.playing && now.duration_since(self.reported) >= self.setup.interval {
            let (id, at) = (Some(track.id.clone()), track.live());
            self.reported = now;
            self.report(PlaybackEvent::Position { id, at });
        }
    }

    fn incoming(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Event(event) => self.event(event),
            Incoming::Line(line) => self.line(&line),
            Incoming::Gone(end) => self.gone(end),
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::AuthState {
                logged_in,
                is_active,
            } => {
                self.set_active(is_active);
                let was = std::mem::replace(&mut self.signed_in, logged_in);
                if logged_in && !was {
                    self.refusals = 0;
                    self.settle();
                    self.advance();
                }
            }
            Event::PlaybackState(playback) => {
                self.hear(playback, true);
                if self.queue_requested == Some(false) {
                    self.queue_requested = Some(true);
                    self.send("get_queue", json!({}));
                }
            }
            Event::PlaybackChanged(playback) => self.hear(playback, false),
            Event::TrackChanged => self.refresh_queue(),
            Event::ContextChanged { context } => {
                self.heard.context = context.and_then(|context| connect_collection(&context.uri));
                self.refresh_queue();
            }
            Event::PositionSync { position } => {
                self.heard.position = position.at();
                self.follow_position();
            }
            Event::OptionsChanged { options } => self.options(options),
            Event::QueueChanged { previous, upcoming } => self.queue(previous, upcoming),
            Event::VolumeChanged { volume } => self.hear_volume(volume),
            Event::DeviceChanged { is_active } => {
                self.set_active(is_active);
                if is_active {
                    self.refresh_queue();
                }
            }
            Event::CommandResult { command } => self.acknowledged(&command),
            Event::Error { message } if message.starts_with("seek failed") => {
                log::debug!("soloist: the daemon is not ready to seek: {message}");
            }
            Event::Error { message } => {
                log::warn!("soloist: the daemon turned a command down: {message}");
            }
            Event::Other => {}
        }
    }

    /// Notes that the daemon took a command: the one that starts the wanted track, after which
    /// its reports are about that track, or a seek.
    fn acknowledged(&mut self, command: &str) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if command == "seek" {
            if let Some(seek) = track.seek.as_mut() {
                seek.taken = true;
            }
            return;
        }
        if !track
            .starting
            .is_some_and(|(starter, _)| command == starter)
        {
            return;
        }
        track.starting = None;
        // the report that named the track may have come ahead of the acknowledgement
        self.heard.status = Status::Unknown;
        self.heard.position = Duration::ZERO;
        self.send("get_state", json!({}));
    }

    /// Takes in a playback report and acts on what changed.
    fn hear(&mut self, playback: Playback, full: bool) {
        if let Some(active) = playback.is_active {
            self.set_active(active);
        }
        if let Some(item) = playback.item
            && self.heard.uri != item.uri
        {
            self.heard.position = Duration::ZERO;
            self.heard.uri = item.uri;
        }
        if let Some(status) = playback.status {
            self.heard.status = status;
        }
        if full || playback.context.is_some() {
            self.heard.context = playback
                .context
                .and_then(|context| connect_collection(&context.uri));
        }
        if let Some(options) = playback.options {
            self.options(options);
        }
        let placed = playback.position.is_some();
        if let Some(position) = playback.position {
            self.heard.position = position.at();
        }
        self.observe(placed);
        if placed {
            self.follow_position();
        }
    }

    /// Refreshes both reports in order so a queue can be anchored to the current track.
    fn refresh_queue(&mut self) {
        if self.active && self.signed_in && self.queue_requested.is_none() {
            self.queue_requested = Some(false);
            self.send("get_state", json!({}));
        }
    }

    fn options(&mut self, options: Options) {
        let repeat = match options.repeat.as_deref() {
            Some("track") => Some(Repeat::One),
            Some("context") => Some(Repeat::All),
            Some("off") => Some(Repeat::Off),
            _ => None,
        };
        let before = (self.heard.shuffle, self.heard.repeat);
        self.heard.shuffle = options.shuffle.or(before.0);
        self.heard.repeat = repeat.or(before.1);
        if self.active && before != (self.heard.shuffle, self.heard.repeat) {
            self.report(PlaybackEvent::DeviceModes {
                shuffle: options.shuffle,
                repeat,
            });
        }
    }

    /// Broadcasts may be capped, so request the full window before adopting one.
    fn queue(&mut self, mut previous: Vec<Queued>, mut upcoming: Vec<Queued>) {
        self.next = upcoming
            .first()
            .and_then(|entry| entry.item.as_ref())
            .map(|item| item.uri.clone());
        if self.queue_requested != Some(true) {
            self.refresh_queue();
            return;
        }
        self.queue_requested = None;
        let Some(id) = self.heard.uri.strip_prefix(TRACK_PREFIX) else {
            return;
        };
        if !self.active || !matches!(self.heard.status, Status::Playing | Status::Paused) {
            return;
        }
        previous.truncate(PREVIOUS);
        previous.reverse();
        upcoming.truncate(UPCOMING);
        let tracks = |rows: Vec<Queued>| rows.into_iter().filter_map(Queued::track).collect();
        self.report(PlaybackEvent::DeviceQueue(Arc::new(Snapshot {
            track: Some(QueuedTrack::new(id.to_owned(), false)),
            context: self.heard.context.clone(),
            previous: tracks(previous),
            upcoming: tracks(upcoming),
            transport: Transport {
                playing: self.heard.status == Status::Playing,
                position: self.heard.position,
                shuffle: self.heard.shuffle,
                repeat: self.heard.repeat,
                stamp: Some(std::time::SystemTime::now()),
                ..Default::default()
            },
            ..Default::default()
        })));
    }

    /// Applies a playback report once an outstanding local start has taken effect.
    fn observe(&mut self, placed: bool) {
        if !self.active {
            return;
        }
        let heard = self.heard.uri.strip_prefix(TRACK_PREFIX);
        let synced = heard.is_some()
            && matches!(self.heard.status, Status::Playing | Status::Paused)
            && std::mem::take(&mut self.syncing);
        if self.track.as_ref().is_none_or(|track| {
            if track.started {
                Some(track.id.as_str()) != heard
            } else {
                !track.sent && (synced || self.heard.status == Status::Playing)
            }
        }) && matches!(self.heard.status, Status::Playing | Status::Paused)
            && let Some(id) = heard
        {
            let id = id.to_owned();
            let at = self.heard.position;
            let playing = self.heard.status == Status::Playing;
            self.track = Some(Track {
                sent: true,
                started: true,
                playing,
                ..Track::new(id.clone(), at, !playing)
            });
            self.unmute();
            let id = Some(id);
            self.report(if playing {
                PlaybackEvent::Playing { id, at }
            } else {
                PlaybackEvent::Paused { id, at }
            });
            return;
        }
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !track.sent || track.starting.is_some() {
            return;
        }
        let id = Some(track.id.clone());
        if !track.started {
            if heard != Some(track.id.as_str()) || self.heard.status != Status::Playing {
                return;
            }
            track.started = true;
            let (start, held, at) = (track.position, track.held, self.heard.position);
            if start > OPENING {
                track.seek = Some(Seek::new(start, true));
            } else {
                track.playing = !held;
                track.place(at);
            }
            if held {
                self.send("pause", json!({}));
                self.report(PlaybackEvent::Paused {
                    id: id.clone(),
                    at: start,
                });
            }
            if start > OPENING {
                return self.send("seek", json!({ "position_ms": millis(start) }));
            }
            self.unmute();
            if !held {
                self.report(PlaybackEvent::Playing { id, at });
            }
            return;
        }
        if self.heard.status == Status::Idle {
            return self.finish();
        }
        let playing = match self.heard.status {
            Status::Playing if track.seek.is_none() => true,
            Status::Paused => false,
            _ => return,
        };
        if track.playing != playing {
            let at = if placed {
                self.heard.position
            } else {
                track.live()
            };
            track.held = !playing;
            track.playing = playing;
            track.place(at);
            self.report(match playing {
                true => PlaybackEvent::Playing { id, at },
                false => PlaybackEvent::Paused { id, at },
            });
        }
    }

    /// Acts on a position report: a seek that landed, or progress.
    fn follow_position(&mut self) {
        let at = self.heard.position;
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !self.active
            || !track.started
            || self.heard.uri.strip_prefix(TRACK_PREFIX) != Some(track.id.as_str())
        {
            return;
        }
        let landed = track
            .seek
            .as_ref()
            .map(|seek| seek.taken && at.abs_diff(seek.to) <= SEEK_NEAR);
        match landed {
            Some(true) => self.landed(),
            Some(false) => {}
            None => {
                let seeked = !track.playing || track.live().abs_diff(at) > Duration::from_secs(1);
                track.place(at);
                let id = Some(track.id.clone());
                self.reported = Instant::now();
                self.report(match seeked {
                    true => PlaybackEvent::Seeked { id, at },
                    false => PlaybackEvent::Position { id, at },
                });
            }
        }
    }

    /// Closes the seek under way and reports where the track now is.
    fn landed(&mut self) {
        let at = self.heard.position;
        let Some(track) = self.track.as_mut() else {
            return;
        };
        let Some(seek) = track.seek.take() else {
            return;
        };
        track.place(at);
        let id = Some(track.id.clone());
        let starts = seek.opening && !track.held;
        if starts {
            track.playing = true;
        }
        self.unmute();
        match starts {
            true => self.report(PlaybackEvent::Playing { id, at }),
            false => self.report(PlaybackEvent::Seeked { id, at }),
        }
    }

    /// Reports that the daemon reached the end of playback.
    fn finish(&mut self) {
        let Some(track) = self.track.take() else {
            return;
        };
        self.unmute();
        self.report(PlaybackEvent::Ended { id: Some(track.id) });
    }

    /// Invalidates the cached snapshot whenever Spotify Connect changes ownership.
    fn set_active(&mut self, active: bool) {
        if self.active == active {
            return;
        }
        self.active = active;
        self.queue_requested = None;
        self.syncing = active;
        self.heard = Heard::default();
        if !active {
            self.taken();
        }
    }

    /// Another device took playback, which stops the daemon. The track is kept where it was,
    /// paused, so `play` takes playback back and goes on from there.
    fn taken(&mut self) {
        self.next = None;
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !track.started {
            return;
        }
        log::info!("soloist: another device took playback");
        let was_playing = track.playing;
        track.rewind();
        track.held = true;
        let (id, at) = (Some(track.id.clone()), track.position);
        self.unmute();
        if was_playing {
            self.report(PlaybackEvent::Paused { id, at });
        }
    }

    fn line(&mut self, line: &str) {
        if let Some((_, id)) = line
            .split_once("ready, device_name=")
            .and_then(|(_, device)| device.rsplit_once(" device_id="))
            && id.len() == 36
            && id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        {
            self.device.set(id.to_owned()).ok();
        }
        if line.contains(REFUSED) {
            self.refusals += 1;
            log::warn!("soloist: spotify turned the sign-in down");
            match self.refusals >= REFUSALS {
                true => self.refuse(),
                false => {
                    self.hang_up();
                    self.fail();
                }
            }
        } else if line.contains(UNPAIRED) && paired_account().is_none() {
            log::warn!("soloist: the daemon has no sign-in left and has to be paired again");
            self.refuse();
        }
    }

    fn gone(&mut self, end: End) {
        self.link = None;
        self.signed_in = false;
        self.muted = false;
        self.active = false;
        self.heard = Heard::default();
        self.next = None;
        match end {
            End::Closed => {}
            End::Expired if !self.refreshed => {
                log::info!("soloist: the build is too old, fetching the current one");
                self.refreshed = true;
                if let Some(track) = self.track.as_mut() {
                    track.rewind();
                }
                self.link = Some(Link::open(self.setup, self.volume, true));
            }
            End::Unready => {
                recheck();
                self.refuse();
            }
            End::Expired | End::Failed => self.fail(),
        }
    }

    /// Gives up on the wanted track. The next load starts a new daemon.
    fn fail(&mut self) {
        if let Some(track) = self.track.take() {
            self.report(PlaybackEvent::Unavailable { id: Some(track.id) });
        }
    }

    /// Reports that the setup no longer works and stops for good.
    fn refuse(&mut self) {
        self.dead = true;
        self.track = None;
        self.hang_up();
        self.report(PlaybackEvent::Refused);
    }

    /// Starts a daemon when none runs.
    fn wake(&mut self) {
        if self.link.is_none() && !self.dead {
            self.link = Some(Link::open(self.setup, self.volume, false));
        }
    }

    /// Stops the daemon without waiting for it.
    fn hang_up(&mut self) {
        self.signed_in = false;
        self.muted = false;
        if let Some(link) = self.link.take() {
            tokio::spawn(link.close());
        }
    }

    /// Puts a daemon that just signed in the way Sonora needs it.
    fn settle(&mut self) {
        self.send("set_repeat_track", json!({ "enabled": self.repeat_one }));
        let volume = self.volume;
        self.send("set_volume", json!({ "volume": volume }));
        self.refresh_queue();
    }

    /// Reports remote volume changes, ignoring the temporary mute during an opening seek.
    fn hear_volume(&mut self, volume: u8) {
        if self.active && !self.muted && self.volume != volume {
            self.volume = volume;
            self.report(PlaybackEvent::RemoteGain(f32::from(volume) / 100.0));
        }
    }

    fn mute(&mut self) {
        if !std::mem::replace(&mut self.muted, true) {
            self.send("set_volume", json!({ "volume": 0 }));
        }
    }

    fn unmute(&mut self) {
        if std::mem::replace(&mut self.muted, false) {
            let volume = self.volume;
            self.send("set_volume", json!({ "volume": volume }));
        }
    }

    /// What every track played here is: the level asked for, decoded by the daemon to 44.1 kHz
    /// stereo and normalised there. The daemon never says what it fetched, so the level is the
    /// ceiling Spotify's own apps show too.
    fn signal(&self) -> TrackSignal {
        TrackSignal {
            encoding: self.setup.quality.offered_encoding(QUALITIES),
            rate: RATE,
            channels: CHANNELS,
            normalisation: match self.setup.normalise {
                true => Normalisation::Limited,
                false => Normalisation::Off,
            },
            direct: true,
        }
    }

    /// Sends a command, which the daemon only takes once it is signed in.
    fn send(&self, name: &str, fields: Value) {
        if let Some(link) = self.link.as_ref().filter(|_| self.signed_in) {
            link.outgoing.send(wire::command(name, fields)).ok();
        }
    }

    fn report(&self, event: PlaybackEvent) {
        self.events.send(event).ok();
    }
}

/// What a daemon sends the brain.
enum Incoming {
    Event(Event),
    /// A line the daemon printed.
    Line(String),
    /// The daemon is gone, which is always the last thing sent.
    Gone(End),
}

/// How a daemon came to its end.
enum End {
    /// The brain let go of it.
    Closed,
    /// Its build is too old for Spotify's servers.
    Expired,
    /// The key or the program is missing, so the setup has to be done again.
    Unready,
    /// It could not start or stopped by itself.
    Failed,
}

/// One running daemon: commands go in as JSON text, its reports come back.
struct Link {
    outgoing: UnboundedSender<String>,
    incoming: UnboundedReceiver<Incoming>,
    task: JoinHandle<()>,
}

impl Link {
    /// Starts a daemon, after fetching the current build when `refresh` asks.
    fn open(setup: Setup, volume: u8, refresh: bool) -> Self {
        let (outgoing, asked) = unbounded_channel();
        let (reports, incoming) = unbounded_channel();
        let task = tokio::spawn(serve(setup, volume, refresh, asked, reports));
        Self {
            outgoing,
            incoming,
            task,
        }
    }

    /// Stops the daemon and waits until it has let go of the data folder.
    async fn close(self) {
        drop(self.outgoing);
        drop(self.incoming);
        self.task.await.ok();
    }
}

/// The next thing the daemon sends, or never when none runs.
async fn next(link: &mut Option<Link>) -> Incoming {
    match link {
        Some(link) => link
            .incoming
            .recv()
            .await
            .unwrap_or(Incoming::Gone(End::Failed)),
        None => std::future::pending().await,
    }
}

/// Runs one daemon from start to end, holding the data folder for as long as it lives.
async fn serve(
    setup: Setup,
    volume: u8,
    refresh: bool,
    mut asked: UnboundedReceiver<String>,
    reports: UnboundedSender<Incoming>,
) {
    let _running = tokio::select! {
        running = RUNNING.lock() => running,
        () = dropped(&mut asked) => return,
    };
    let end = daemon(setup, volume, refresh, &mut asked, &reports).await;
    reports.send(Incoming::Gone(end)).ok();
}

async fn daemon(
    setup: Setup,
    volume: u8,
    refresh: bool,
    asked: &mut UnboundedReceiver<String>,
    reports: &UnboundedSender<Incoming>,
) -> End {
    if refresh {
        let fetched = tokio::select! {
            fetched = binary::fetch() => fetched,
            () = dropped(asked) => return End::Closed,
        };
        if let Err(error) = fetched {
            log::warn!("soloist: cannot fetch the current build: {error:#}");
            return End::Failed;
        }
    }
    let prepared = tokio::task::spawn_blocking(move || prepare(setup)).await;
    let Ok(Some(key)) = prepared else {
        log::warn!("soloist: no api key is stored");
        return End::Unready;
    };
    let volume = volume.to_string();
    let flags = [
        "--cache-size",
        CACHE_MB,
        "--initial-volume",
        volume.as_str(),
        "--ws",
        "127.0.0.1:0",
    ]
    .map(OsStr::new);
    let mut child = match launch(&key, &data(), &flags) {
        Ok(child) => child,
        Err(error) => {
            log::error!("soloist: {error:#}");
            return End::Unready;
        }
    };
    let mut lines = follow(&mut child, &key);
    let mut printing = true;

    let connecting = connect();
    tokio::pin!(connecting);
    let mut socket: Option<Socket> = None;
    loop {
        let connected = socket.is_some();
        tokio::select! {
            result = &mut connecting, if !connected => match result {
                Ok(open) => socket = Some(open),
                Err(error) => {
                    log::warn!("soloist: {error:#}");
                    stop(&mut child).await;
                    return End::Failed;
                }
            },
            frame = async { socket.as_mut().expect("connected socket").next().await }, if connected => match frame {
                Some(Ok(Message::Text(text))) => match serde_json::from_str(text.as_str()) {
                    Ok(event) => {
                        reports.send(Incoming::Event(event)).ok();
                    }
                    Err(error) => log::debug!("soloist: cannot read a message: {error}"),
                },
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    // the socket goes when the daemon does
                    stop(&mut child).await;
                    return ended(child.wait().await);
                }
            },
            command = receive_command(asked, connected) => match command {
                Some(command) => {
                    let socket = socket.as_mut().expect("commands require a connected socket");
                    if let Err(error) = socket.send(Message::text(command)).await {
                        log::warn!("soloist: cannot send a command: {error}");
                    }
                }
                None => {
                    stop(&mut child).await;
                    return End::Closed;
                }
            },
            line = lines.recv(), if printing => match line {
                Some(line) => {
                    reports.send(Incoming::Line(line)).ok();
                }
                None => printing = false,
            },
            status = child.wait() => return ended(status),
        }
    }
}

/// Reads the key, clears the address a dead daemon left behind and writes the audio settings
/// the daemon reads at startup. `None` without a key. Blocking.
fn prepare(setup: Setup) -> Option<String> {
    let key = key()?;
    let data = data();
    for file in [WS_ADDR, WS_PORT] {
        std::fs::remove_file(data.join(file)).ok();
    }
    if let Err(error) = prefs::write(&data, setup.normalise, setup.gapless, setup.quality) {
        log::warn!("soloist: cannot write the audio settings: {error:#}");
    }
    Some(key)
}

/// Waits for the daemon to write down where its WebSocket listens and connects to it.
async fn connect() -> Result<Socket> {
    let data = data();
    let deadline = Instant::now() + STARTUP;
    loop {
        if let Some(address) = endpoint(&data)
            && let Ok((socket, _)) = tokio_tungstenite::connect_async(address).await
        {
            return Ok(socket);
        }
        ensure!(
            Instant::now() < deadline,
            "the daemon did not open its websocket"
        );
        tokio::time::sleep(POLL).await;
    }
}

fn endpoint(data: &Path) -> Option<String> {
    let address = std::fs::read_to_string(data.join(WS_ADDR)).ok()?;
    let port = std::fs::read_to_string(data.join(WS_PORT)).ok()?;
    let port: u16 = port.trim().parse().ok().filter(|port| *port != 0)?;
    Some(format!("ws://{}:{port}", address.trim()))
}

fn ended(status: std::io::Result<std::process::ExitStatus>) -> End {
    match status.ok().and_then(|status| status.code()) {
        Some(binary::EXPIRED) => End::Expired,
        code => {
            log::warn!("soloist: the daemon stopped with code {code:?}");
            End::Failed
        }
    }
}

/// Discards commands until the daemon's socket opens.
async fn receive_command(asked: &mut UnboundedReceiver<String>, connected: bool) -> Option<String> {
    if connected {
        return asked.recv().await;
    }
    dropped(asked).await;
    None
}

/// Resolves once the brain has let go of the daemon. Commands sent before the daemon can take
/// them are dropped, and the brain sends none before the daemon signs in.
async fn dropped(asked: &mut UnboundedReceiver<String>) {
    while asked.recv().await.is_some() {}
}

/// The daemon's volume for a linear gain. It attenuates on Spotify's cubic curve, so the cube
/// root gives back the gain Sonora asked for.
fn percent(gain: f32) -> u8 {
    (gain.clamp(0., 1.).cbrt() * 100.).round() as u8
}

fn millis(position: Duration) -> u64 {
    u64::try_from(position.as_millis()).unwrap_or(u64::MAX)
}
