//! The Soloist engine: the program run as a daemon and steered over its WebSocket.
//!
//! The daemon is a whole Spotify player with a queue, autoplay and a volume of its own, and
//! Sonora wants none of that. It is handed one track at a time and everything it does beyond
//! that track is undone: shuffle and repeat are switched off, its volume is held where Sonora's
//! is, and a track it moves on to by itself is stopped and reported as the end of the one that
//! was asked for. Sonora's queue then says what plays next.
//!
//! The one thing borrowed from the daemon's queue is the join between two tracks. With gapless
//! playback on, the track Sonora says comes next is put on the daemon's queue in the last
//! seconds of the current one, so the daemon goes from one to the other by itself. When it does,
//! the current track is reported as ended and the load that follows finds the next one already
//! playing. Queued any earlier, the daemon would crossfade into it whenever the account's own
//! Spotify settings ask for a crossfade.
//!
//! Four things in here are learned from the daemon rather than its documentation. A bare `play`
//! starts sound without making the daemon the account's active device, so `activate` goes first.
//! Becoming active, the daemon takes over whatever the account played last and reports that
//! before the command that follows lands, so nothing it says counts for a track until it has
//! acknowledged the command that starts it.
//! A seek sent while a track is still loading is dropped, so every seek is sent again until a
//! position report lands near it. And the end of a track arrives as whichever of three things
//! the daemon feels like: another track, `idle`, or a pause nobody asked for.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use futures::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::daemon::{RUNNING, follow, launch, stop};
use super::wire::{self, Event, Playback, Status};
use super::{QUALITIES, binary, data, key, paired_account, prefs, recheck};
use crate::{
    Encoding, Normalisation, PlaybackConfig, PlaybackEvent, PlaybackEvents, Quality, TrackSignal,
};

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
/// The stretch at the end of a track in which a pause nobody asked for means it is over.
const TAIL: Duration = Duration::from_secs(10);
/// How far past its length a track's clock may run before it is taken to have ended unsaid.
const OVERRUN: Duration = Duration::from_secs(3);
/// How long before the end of a track the next one goes on the daemon's queue.
const FEED_LEAD: Duration = Duration::from_secs(8);
/// How long a track the daemon went on to by itself may play before Sonora has to have asked
/// for it. Past that it is stopped, as at the end of Sonora's queue.
const ADOPT_PATIENCE: Duration = Duration::from_secs(2);
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

    pub(crate) fn play(&self) {
        self.ask(Command::Play);
    }

    pub(crate) fn pause(&self) {
        self.ask(Command::Pause);
    }

    pub(crate) fn seek(&self, position: Duration) {
        self.ask(Command::Seek(position));
    }

    /// Names the track that follows the current one, which a gapless engine joins to it.
    pub(crate) fn line_up(&self, track_id: &str) {
        self.ask(Command::Next(track_id.to_owned()));
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.ask(Command::Volume(percent(gain)));
    }

    fn ask(&self, command: Command) {
        self.commands.send(command).ok();
    }
}

/// The engine's events, in the order the daemon's reports led to them.
pub(crate) struct Events(UnboundedReceiver<PlaybackEvent>);

#[async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        self.0.recv().await
    }
}

/// Starts a daemon and the thread that steers it. The daemon comes up at once, so the first
/// track does not wait for it to sign in.
pub(crate) fn start(config: &PlaybackConfig) -> (Engine, Events) {
    let setup = Setup {
        normalise: config.normalisation,
        gapless: config.gapless,
        quality: config.quality,
        interval: config.position_interval,
    };
    begin(setup, percent(config.gain))
}

fn begin(setup: Setup, volume: u8) -> (Engine, Events) {
    let (commands, asked) = unbounded_channel();
    let (events, reported) = unbounded_channel();
    let brain = Brain::new(setup, volume, events);
    let spawned = std::thread::Builder::new()
        .name("soloist".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(brain.run(asked)),
                Err(error) => log::error!("playback: cannot build the soloist runtime: {error}"),
            }
        });
    if let Err(error) = spawned {
        log::error!("playback: cannot spawn the soloist thread: {error}");
    }
    (Engine { commands }, Events(reported))
}

enum Command {
    Load {
        id: String,
        at: Duration,
        play: bool,
    },
    Play,
    Pause,
    Seek(Duration),
    Volume(u8),
    Next(String),
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
    uri: String,
    /// Where it starts.
    start: Duration,
    /// Whether it waits for `play` before the daemon hears of it.
    parked: bool,
    /// Whether the daemon has been told to play it.
    sent: bool,
    /// The command that starts it: `play`, or a skip onto the daemon's queue.
    starter: &'static str,
    /// When that command went out.
    asked: Instant,
    /// Whether the daemon took that command. What it reports before then is the account's last
    /// playback, which can be this very track at the very place it is to start from.
    accepted: bool,
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
    duration: Option<Duration>,
}

impl Track {
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

    /// Whether the track is within `TAIL` of its end, or far enough in when its length is not
    /// known.
    fn in_tail(&self) -> bool {
        match self.duration {
            Some(duration) => self.live() + TAIL >= duration,
            None => self.live() >= TAIL,
        }
    }

    /// Puts the track back to before the daemon heard of it, to start again from where it is.
    fn rewind(&mut self) {
        self.start = self.live();
        self.place(self.start);
        self.sent = false;
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

/// What the daemon last said it is doing.
#[derive(Default)]
struct Heard {
    uri: String,
    status: Status,
    position: Duration,
    /// The last track it gave a length for, and that length.
    length: Option<(String, Duration)>,
}

/// The engine's own loop: commands in, the daemon's reports in, events out.
struct Brain {
    setup: Setup,
    events: UnboundedSender<PlaybackEvent>,
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
    /// The track Sonora says comes next, held until the current one is near its end.
    next: Option<String>,
    /// The track put on the daemon's queue to follow the current one.
    fed: Option<String>,
    /// The tracks on the daemon's queue as it last listed them. The account's own queued
    /// tracks are among them, and those are not Sonora's to move or skip.
    queue: Vec<String>,
    /// The queued track the daemon went on to, and when, until Sonora asks for it.
    adopted: Option<(String, Instant)>,
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
            link: None,
            signed_in: false,
            volume,
            muted: false,
            track: None,
            heard: Heard::default(),
            next: None,
            fed: None,
            queue: Vec::new(),
            adopted: None,
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
            Command::Volume(volume) => {
                self.volume = volume;
                if !self.muted {
                    self.send("set_volume", json!({ "volume": volume }));
                }
            }
            Command::Next(id) => {
                if self.setup.gapless {
                    self.next = Some(format!("{TRACK_PREFIX}{id}"));
                    self.feed();
                }
            }
        }
    }

    fn load(&mut self, id: String, at: Duration, play: bool) {
        if self.dead {
            return self.report(PlaybackEvent::Refused);
        }
        let uri = format!("{TRACK_PREFIX}{id}");
        let adopted = self.adopted.take().map(|(adopted, _)| adopted);
        // the daemon went on to this very track from its queue and is playing it already
        let joined =
            play && at <= OPENING && adopted.as_ref() == Some(&uri) && self.heard.uri == uri;
        let sounding = adopted.is_some() || self.track.as_ref().is_some_and(|track| track.started);
        let duration = self
            .heard
            .length
            .as_ref()
            .filter(|(known, _)| *known == uri)
            .map(|(_, duration)| *duration);
        self.next = None;
        self.unmute();
        self.report(PlaybackEvent::Loading {
            id: Some(id.clone()),
            at,
        });
        self.report(PlaybackEvent::Signal {
            id: id.clone(),
            track: self.signal(),
        });
        let position = match joined {
            true => self.heard.position,
            false => at,
        };
        self.track = Some(Track {
            uri,
            id: id.clone(),
            start: at,
            parked: !play,
            sent: joined,
            starter: "play",
            asked: Instant::now(),
            accepted: joined,
            started: joined,
            held: !play,
            playing: joined,
            seek: None,
            position,
            stamp: Instant::now(),
            duration,
        });
        if joined {
            if duration.is_none() {
                self.send("get_state", json!({}));
            }
            return self.report(PlaybackEvent::Playing {
                id: Some(id),
                at: position,
            });
        }
        match play {
            true => self.advance(),
            false => {
                if sounding {
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
        if track.parked || track.sent {
            return;
        }
        track.sent = true;
        let uri = track.uri.clone();
        let opening = track.start > OPENING;
        // what the daemon said of the track before this one says nothing of the new one
        self.heard.status = Status::Unknown;
        self.heard.position = Duration::ZERO;
        // A track Sonora queued and no longer wants next is still on the daemon's queue, where
        // it would play after this one. Moving on to it is the only way to take it off.
        let stale = self.fed.take().filter(|fed| self.queue.contains(fed));
        let wanted = stale.as_ref() == Some(&uri);
        if let Some(track) = self.track.as_mut() {
            track.asked = Instant::now();
            track.starter = match wanted {
                true => "skip_next",
                false => "play",
            };
        }
        if opening || (stale.is_some() && !wanted) {
            self.mute();
        }
        self.send("activate", json!({}));
        if stale.is_some() {
            self.send("skip_next", json!({}));
        }
        if !wanted {
            self.send("play", json!({ "uri": uri }));
        }
    }

    /// Puts the track that comes next on the daemon's queue once the current one is in its
    /// last seconds. It waits while the account has queued tracks of its own, which would play
    /// first, and while the current track's length is not known.
    fn feed(&mut self) {
        let Some(track) = self.track.as_ref() else {
            return;
        };
        let due = track.started
            && track.seek.is_none()
            && track
                .duration
                .is_some_and(|duration| track.live() + FEED_LEAD >= duration);
        if !due || self.fed.is_some() || !self.queue.is_empty() {
            return;
        }
        let Some(next) = self.next.take() else {
            return;
        };
        log::debug!("soloist: queueing the next track on the daemon");
        self.send("add_to_queue", json!({ "uri": next }));
        self.send("get_queue", json!({}));
        self.fed = Some(next);
    }

    fn play(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        track.held = false;
        track.parked = false;
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
            track.parked = true;
            let (id, at) = (Some(track.id.clone()), track.start);
            self.report(PlaybackEvent::Paused { id, at });
        }
    }

    fn seek(&mut self, to: Duration) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !track.started {
            track.start = to;
            track.place(to);
            let (id, sent) = (Some(track.id.clone()), track.sent);
            return match sent {
                // the daemon is loading it, and the opening seek takes it there once it plays
                true if to > OPENING => self.mute(),
                true => {}
                false => self.report(PlaybackEvent::Seeked { id, at: to }),
            };
        }
        let now = Instant::now();
        track.seek = Some(Seek {
            to,
            since: now,
            asked: now,
            taken: false,
            opening: false,
        });
        self.send("seek", json!({ "position_ms": millis(to) }));
    }

    /// Sends an unconfirmed seek again, gives up on one that never lands, ends a track whose
    /// clock ran past its length with no word from the daemon, and reports progress.
    fn tick(&mut self) {
        let now = Instant::now();
        if let Some((_, since)) = &self.adopted
            && now.duration_since(*since) > ADOPT_PATIENCE
        {
            // Sonora had nothing to play after the track before it
            log::debug!("soloist: nothing asked for the queued track, stopping it");
            self.adopted = None;
            self.send("pause", json!({}));
        }
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if track.sent && !track.accepted && now.duration_since(track.asked) > ACK_PATIENCE {
            log::debug!("soloist: the daemon acknowledged no start, going by its reports");
            track.accepted = true;
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
        let overran = track.playing
            && track
                .duration
                .is_some_and(|duration| track.live() > duration + OVERRUN);
        if overran {
            log::debug!("soloist: the track ran past its end unsaid");
            return self.finish();
        }
        if track.playing && now.duration_since(self.reported) >= self.setup.interval {
            let (id, at) = (Some(track.id.clone()), track.live());
            self.reported = now;
            self.report(PlaybackEvent::Position { id, at });
        }
        self.feed();
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
            Event::AuthState { logged_in } => {
                let was = std::mem::replace(&mut self.signed_in, logged_in);
                if logged_in && !was {
                    self.refusals = 0;
                    self.settle();
                    self.advance();
                }
            }
            Event::PlaybackState(playback) | Event::PlaybackChanged(playback) => {
                self.hear(playback);
            }
            Event::TrackChanged { item } => {
                if let Some(item) = item.filter(|item| !item.uri.is_empty()) {
                    self.learn_length(&item);
                    self.heard.uri = item.uri;
                    self.observe(true);
                }
            }
            Event::PositionSync { position } => {
                self.heard.position = position.at();
                self.follow_position();
            }
            Event::QueueChanged { upcoming } => {
                self.queue = upcoming
                    .into_iter()
                    .filter(|entry| entry.queued())
                    .filter_map(|entry| entry.item)
                    .map(|item| item.uri)
                    .collect();
            }
            Event::VolumeChanged { volume } => self.hold_volume(volume),
            Event::OptionsChanged { options } => {
                if options.meddles() {
                    self.settle_options();
                }
            }
            Event::DeviceChanged { is_active } => {
                if !is_active {
                    self.taken();
                }
            }
            Event::CommandResult { command } => self.acknowledged(&command),
            Event::Error { message } => {
                // a seek is turned down while the track loads, and is simply sent again
                let seeking = self
                    .track
                    .as_ref()
                    .is_some_and(|track| track.seek.is_some());
                match seeking && message.starts_with("seek failed") {
                    true => log::debug!("soloist: the daemon is not ready to seek: {message}"),
                    false => log::warn!("soloist: the daemon turned a command down: {message}"),
                }
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
        if !track.sent || track.accepted || command != track.starter {
            return;
        }
        track.accepted = true;
        // the report that named the track may have come ahead of the acknowledgement
        self.heard.status = Status::Unknown;
        self.heard.position = Duration::ZERO;
        self.send("get_state", json!({}));
    }

    /// Takes in a playback report and acts on what changed.
    fn hear(&mut self, playback: Playback) {
        if let Some(item) = playback.item.filter(|item| !item.uri.is_empty()) {
            self.learn_length(&item);
            self.heard.uri = item.uri;
        }
        if let Some(status) = playback.status {
            self.heard.status = status;
        }
        if let Some(volume) = playback.volume {
            self.hold_volume(volume);
        }
        if playback.options.is_some_and(|options| options.meddles()) {
            self.settle_options();
        }
        let placed = playback.position.is_some();
        if let Some(position) = playback.position {
            self.heard.position = position.at();
        }
        self.observe(false);
        if placed {
            self.follow_position();
        }
    }

    /// Keeps the length the daemon gives for a track, and reports it the first time it is the
    /// wanted track's.
    fn learn_length(&mut self, item: &wire::Entity) {
        let Some(duration) = item.duration() else {
            return;
        };
        self.heard.length = Some((item.uri.clone(), duration));
        let Some(track) = self.track.as_mut().filter(|track| track.uri == item.uri) else {
            return;
        };
        if track.duration.replace(duration) != Some(duration) {
            let id = Some(track.id.clone());
            self.report(PlaybackEvent::Length { id, duration });
        }
    }

    /// Compares what the daemon does with what was asked of it. `announced` says the daemon
    /// just named the track it moved to, which counts as playing it even before a status says so.
    fn observe(&mut self, announced: bool) {
        let Some(track) = self.track.as_mut() else {
            // Something was started on the daemon from one of Spotify's apps. Sonora shows
            // none of it, so it does not get to play. A track the daemon went on to from its
            // queue is Sonora's own, and waits for the load that asks for it.
            if self.adopted.is_none() && self.heard.status == Status::Playing {
                self.send("pause", json!({}));
            }
            return;
        };
        if !track.sent || !track.accepted {
            return;
        }
        let same = self.heard.uri == track.uri;
        let id = Some(track.id.clone());
        if !track.started {
            // between taking the command and acting on it the daemon can still name another track
            if !same || !(announced || self.heard.status == Status::Playing) {
                return;
            }
            track.started = true;
            let (start, held) = (track.start, track.held);
            if track.duration.is_none() {
                // only the whole picture is sure to carry the track's length
                self.send("get_state", json!({}));
            }
            if held {
                self.send("pause", json!({}));
                self.report(PlaybackEvent::Paused {
                    id: id.clone(),
                    at: start,
                });
            }
            if start > OPENING {
                let now = Instant::now();
                if let Some(track) = self.track.as_mut() {
                    track.seek = Some(Seek {
                        to: start,
                        since: now,
                        asked: now,
                        taken: false,
                        opening: true,
                    });
                }
                return self.send("seek", json!({ "position_ms": millis(start) }));
            }
            self.unmute();
            if !held {
                let at = self.heard.position;
                if let Some(track) = self.track.as_mut() {
                    track.playing = true;
                    track.place(at);
                }
                self.report(PlaybackEvent::Playing { id, at });
            }
            return;
        }
        if !same {
            return match self.fed.as_ref() == Some(&self.heard.uri) {
                true => self.hand_over(),
                false => {
                    // past the one track it was given the daemon went on by itself
                    log::debug!("soloist: the daemon moved on to another track");
                    self.finish()
                }
            };
        }
        match self.heard.status {
            Status::Idle => self.finish(),
            Status::Paused if !track.held && track.in_tail() => self.finish(),
            Status::Paused => {
                // paused from one of Spotify's apps, which Sonora follows
                track.held = true;
                if std::mem::replace(&mut track.playing, false) {
                    let at = self.heard.position;
                    track.place(at);
                    self.report(PlaybackEvent::Paused { id, at });
                }
            }
            Status::Playing if !track.playing && track.seek.is_none() => {
                let at = self.heard.position;
                track.held = false;
                track.playing = true;
                track.place(at);
                self.report(PlaybackEvent::Playing { id, at });
            }
            Status::Playing | Status::Buffering | Status::Unknown => {}
        }
    }

    /// Acts on a position report: a seek that landed, or progress.
    fn follow_position(&mut self) {
        let at = self.heard.position;
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !track.started || self.heard.uri != track.uri {
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
                track.place(at);
                if track.playing {
                    let id = Some(track.id.clone());
                    self.reported = Instant::now();
                    self.report(PlaybackEvent::Position { id, at });
                }
                self.feed();
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

    /// Ends the wanted track as the daemon goes on to the one queued behind it. That one keeps
    /// playing, since the load Sonora answers the end with is for it.
    fn hand_over(&mut self) {
        let (Some(track), Some(next)) = (self.track.take(), self.fed.take()) else {
            return;
        };
        log::debug!("soloist: the daemon went on to the queued track");
        self.adopted = Some((next, Instant::now()));
        self.report(PlaybackEvent::Ended { id: Some(track.id) });
    }

    /// Ends the wanted track and stops whatever the daemon went on to.
    fn finish(&mut self) {
        let Some(track) = self.track.take() else {
            return;
        };
        self.next = None;
        self.unmute();
        if self.heard.status == Status::Playing {
            self.send("pause", json!({}));
        }
        self.report(PlaybackEvent::Ended { id: Some(track.id) });
    }

    /// Another device took playback, which stops the daemon. The track is kept where it was,
    /// paused, so `play` takes playback back and goes on from there.
    fn taken(&mut self) {
        let Some(track) = self.track.as_mut() else {
            return;
        };
        if !track.started {
            return;
        }
        log::info!("soloist: another device took playback");
        let was_playing = track.playing;
        track.rewind();
        track.parked = true;
        track.held = true;
        self.next = None;
        let (id, at) = (Some(track.id.clone()), track.start);
        self.unmute();
        if was_playing {
            self.report(PlaybackEvent::Paused { id, at });
        }
    }

    fn line(&mut self, line: &str) {
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
        self.next = None;
        self.fed = None;
        self.adopted = None;
        self.queue.clear();
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
        self.settle_options();
        let volume = self.volume;
        self.send("set_volume", json!({ "volume": volume }));
        self.send("get_queue", json!({}));
    }

    /// The daemon restores the account's shuffle and repeat, either of which would have it play
    /// something other than the one track it is given.
    fn settle_options(&mut self) {
        self.send("set_shuffle", json!({ "enabled": false }));
        self.send("set_repeat_track", json!({ "enabled": false }));
        self.send("set_repeat_context", json!({ "enabled": false }));
    }

    /// Puts the daemon's volume back when one of Spotify's apps moved it. Sonora's own volume
    /// is the one the listener sees.
    fn hold_volume(&mut self, volume: u8) {
        let wanted = match self.muted {
            true => 0,
            false => self.volume,
        };
        if volume.abs_diff(wanted) > 1 {
            self.send("set_volume", json!({ "volume": wanted }));
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
            encoding: offered(self.setup.quality),
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
    let mut child = match launch(&key, &flags) {
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
    let mut socket = loop {
        tokio::select! {
            socket = &mut connecting => match socket {
                Ok(socket) => break socket,
                Err(error) => {
                    log::warn!("soloist: {error:#}");
                    stop(&mut child).await;
                    return End::Failed;
                }
            },
            status = child.wait() => return ended(status),
            line = lines.recv(), if printing => match line {
                Some(line) => {
                    reports.send(Incoming::Line(line)).ok();
                }
                None => printing = false,
            },
            () = dropped(asked) => {
                stop(&mut child).await;
                return End::Closed;
            }
        }
    };

    loop {
        tokio::select! {
            frame = socket.next() => match frame {
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
            command = asked.recv() => match command {
                Some(command) => {
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

/// The encoding of the offer the daemon is asked for at a level.
fn offered(quality: Quality) -> Option<Encoding> {
    let level = quality.within(QUALITIES);
    QUALITIES
        .iter()
        .find(|offer| offer.quality == level)
        .map(|offer| offer.encoding)
}
