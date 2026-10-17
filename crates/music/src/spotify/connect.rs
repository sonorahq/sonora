//! Sonora as a Spotify Connect device. It tells Spotify what plays, so Spotify's apps and
//! Discord show it, takes the commands their controls send, and lists and drives the account's
//! other devices.
//!
//! librespot's own `Spirc` does the same but wants to own the queue, so this speaks the same
//! protocol straight through the session's dealer and spclient and leaves the queue to Sonora.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt as _;
use http::{HeaderMap, HeaderValue, Method, header::CONTENT_TYPE};
use librespot_core::dealer::manager::{BoxedStream, BoxedStreamResult, Reply, RequestReply};
use librespot_core::dealer::protocol::{
    Command as Wire, Message, PayloadValue, PlayCommand, TransferOptions,
};
use librespot_core::error::ErrorKind;
use librespot_core::spclient::TransferRequest;
use librespot_core::version::{SEMVER, SPOTIFY_SPIRC_VERSION};
use librespot_core::{Error, Session, SpotifyId};
use librespot_protocol::connect::{
    Capabilities, Cluster, ClusterUpdate, Device as WireDevice, DeviceInfo, MemberType,
    PutStateReason, PutStateRequest, SetVolumeCommand,
};
use librespot_protocol::context_track::ContextTrack;
use librespot_protocol::devices::DeviceType;
use librespot_protocol::media::AudioQuality;
use librespot_protocol::player::{
    ContextIndex, ContextPlayerOptions, PlayOrigin, PlayerState, ProvidedTrack, Suppressions,
};
use librespot_protocol::transfer_state::TransferState;
use protobuf::rt::WireType;
use protobuf::{CodedInputStream, EnumOrUnknown, Message as _, MessageField};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;

use crate::connect::{
    Collection, Command, Connect, Device, DeviceKind, Elsewhere, Event, Naming, NowPlaying,
    RepeatMode, Roster, Start,
};

const DEVICE_NAME: &str = "Sonora";
const TRACK_PREFIX: &str = "spotify:track:";
/// The context a state names when it has none, which Spotify needs to see to keep the device
/// active.
const UNKNOWN_CONTEXT: &str = "spotify:unknown";
/// How long a command waits for the app to act on it before the state is put regardless, so the
/// device that sent it always hears that it was handled.
const ACK_DELAY: Duration = Duration::from_millis(600);
/// How long after claiming playback a report of another active device is taken to be older than
/// the claim.
const CLAIM_GRACE: Duration = Duration::from_secs(3);
/// The most upcoming tracks a start carries over from the device that sent it.
const CARRIED: usize = 40;
/// How long a new name settles before it is put, so a name typed in is put once.
const RENAME_DELAY: Duration = Duration::from_millis(800);
/// The least time between two puts of the state. A change that comes sooner waits for the next
/// slot and is put with everything that changed meanwhile.
const PUT_GAP: Duration = Duration::from_secs(1);
/// How long puts wait after Spotify refuses one for coming too often, and the most they wait
/// after being refused again and again.
const BACKOFF_FIRST: Duration = Duration::from_secs(2);
const BACKOFF_LAST: Duration = Duration::from_secs(60);
const VOLUME_STEPS: u32 = 64;
/// The protobuf tags of a collection update: its items, and in each its kind, raw id and whether
/// it was removed.
const ITEM: u32 = 1 << 3 | 2;
const ITEM_KIND: u32 = 1 << 3;
const ITEM_ID: u32 = 2 << 3 | 2;
const ITEM_REMOVED: u32 = 6 << 3;

enum Input {
    Enable(bool),
    Rename(Naming),
    Publish(Option<NowPlaying>),
    Transfer(String),
    Control(String, Command),
}

/// The handle the app holds. The work happens on a task of the session's runtime, which ends
/// when this is dropped.
pub struct Connection {
    device: String,
    inputs: UnboundedSender<Input>,
    events: Mutex<Option<UnboundedReceiver<Event>>>,
}

impl Connection {
    /// Prepares the device on `session`. Nothing leaves the machine until `enable(true)`.
    pub fn new(session: Session) -> Self {
        let (inputs, receiver) = unbounded_channel();
        let (out, events) = unbounded_channel();
        let device = session.device_id().to_owned();
        let worker = Worker::new(session.clone(), out);
        session.spawn(worker.run(receiver));
        Self {
            device,
            inputs,
            events: Mutex::new(Some(events)),
        }
    }
}

impl Connect for Connection {
    fn enable(&self, on: bool) {
        self.inputs.send(Input::Enable(on)).ok();
    }

    fn rename(&self, naming: Naming) {
        self.inputs.send(Input::Rename(naming)).ok();
    }

    fn publish(&self, now: Option<NowPlaying>) {
        self.inputs.send(Input::Publish(now)).ok();
    }

    fn events(&self) -> Option<UnboundedReceiver<Event>> {
        self.events.lock().ok()?.take()
    }

    fn device(&self) -> String {
        self.device.clone()
    }

    fn transfer(&self, to: &str) {
        self.inputs.send(Input::Transfer(to.to_owned())).ok();
    }

    fn control(&self, device: &str, command: Command) {
        self.inputs
            .send(Input::Control(device.to_owned(), command))
            .ok();
    }
}

/// What the dealer hands the worker: the connection id Spotify gave this session, changes to the
/// account's devices, volume requests, playback commands, and likes made in other apps.
struct Streams {
    connection_ids: BoxedStreamResult<String>,
    clusters: BoxedStreamResult<ClusterUpdate>,
    volumes: BoxedStreamResult<SetVolumeCommand>,
    commands: BoxedStream<RequestReply>,
    likes: BoxedStreamResult<Vec<(String, bool)>>,
}

impl Streams {
    /// Streams that never yield, for the time before the dealer is started and after one ends.
    fn idle() -> Self {
        Self {
            connection_ids: Box::pin(futures::stream::pending()),
            clusters: Box::pin(futures::stream::pending()),
            volumes: Box::pin(futures::stream::pending()),
            commands: Box::pin(futures::stream::pending()),
            likes: Box::pin(futures::stream::pending()),
        }
    }

    /// Subscribes to everything the device listens to. This has to happen before the dealer
    /// starts, because starting it closes the list.
    fn subscribe(session: &Session) -> Result<Self, Error> {
        let dealer = session.dealer();
        Ok(Self {
            connection_ids: dealer.listen_for("hm://pusher/v1/connections/", |message| {
                message
                    .headers
                    .get("Spotify-Connection-Id")
                    .cloned()
                    .ok_or_else(|| Error::failed_precondition("no connection id in the message"))
            })?,
            clusters: dealer.listen_for("hm://connect-state/v1/cluster", Message::from_raw)?,
            volumes: dealer
                .listen_for("hm://connect-state/v1/connect/volume", Message::from_raw)?,
            commands: dealer.handle_for("hm://connect-state/v1/player/command")?,
            likes: dealer.listen_for("hm://collection/collection/", |message| {
                match message.payload {
                    PayloadValue::Raw(bytes) => Ok(likes(&bytes)?),
                    // the same update comes again as JSON, which carries nothing more
                    _ => Ok(Vec::new()),
                }
            })?,
        })
    }
}

struct Worker {
    session: Session,
    out: UnboundedSender<Event>,
    info: DeviceInfo,
    streams: Streams,
    enabled: bool,
    /// The dealer is started once and cannot be restarted on the same session.
    started: bool,
    connected: bool,
    now: Option<NowPlaying>,
    /// When `now` was reported, which its position is as of.
    now_at: Instant,
    active: bool,
    active_since: Option<SystemTime>,
    claimed_at: Option<Instant>,
    /// The id and sender of the last command handled, which the next state echoes back.
    last_command: (u32, String),
    ack_at: Option<Instant>,
    rename_at: Option<Instant>,
    /// The device that has playback on the account, as the last cluster said.
    active_device: String,
    /// The reason of a put that is due but held back, and when it may go out.
    pending: Option<PutStateReason>,
    flush_at: Option<Instant>,
    last_put: Option<Instant>,
    blocked_until: Option<Instant>,
    backoff: Duration,
}

impl Worker {
    fn new(session: Session, out: UnboundedSender<Event>) -> Self {
        let info = device_info(&session, &Naming::default());
        Self {
            session,
            out,
            info,
            streams: Streams::idle(),
            enabled: false,
            started: false,
            connected: false,
            now: None,
            now_at: Instant::now(),
            active: false,
            active_since: None,
            claimed_at: None,
            last_command: (0, String::new()),
            ack_at: None,
            rename_at: None,
            active_device: String::new(),
            pending: None,
            flush_at: None,
            last_put: None,
            blocked_until: None,
            backoff: Duration::ZERO,
        }
    }

    async fn run(mut self, mut inputs: UnboundedReceiver<Input>) {
        loop {
            tokio::select! {
                input = inputs.recv() => {
                    let Some(input) = input else { break };
                    self.input(input).await;
                }
                id = self.streams.connection_ids.next() => match id {
                    Some(Ok(id)) => self.connection_id(id).await,
                    Some(Err(error)) => log::warn!("connect: bad connection id message: {error}"),
                    None => self.streams.connection_ids = Box::pin(futures::stream::pending()),
                },
                likes = self.streams.likes.next() => match likes {
                    Some(Ok(likes)) => {
                        for (track, liked) in likes {
                            self.out.send(Event::Liked { track, liked }).ok();
                        }
                    }
                    Some(Err(error)) => log::warn!("connect: bad collection update: {error}"),
                    None => self.streams.likes = Box::pin(futures::stream::pending()),
                },
                update = self.streams.clusters.next() => match update {
                    Some(Ok(update)) => self.cluster(update),
                    Some(Err(error)) => log::warn!("connect: bad cluster update: {error}"),
                    None => self.streams.clusters = Box::pin(futures::stream::pending()),
                },
                volume = self.streams.volumes.next() => match volume {
                    Some(Ok(volume)) => self.volume(volume),
                    Some(Err(error)) => log::warn!("connect: bad volume request: {error}"),
                    None => self.streams.volumes = Box::pin(futures::stream::pending()),
                },
                request = self.streams.commands.next() => match request {
                    Some(request) => self.request(request),
                    None => self.streams.commands = Box::pin(futures::stream::pending()),
                },
                () = acknowledge(self.rename_at) => {
                    self.rename_at = None;
                    if self.enabled && self.connected {
                        self.announce().await;
                    }
                }
                () = acknowledge(self.flush_at) => self.flush().await,
                () = acknowledge(self.ack_at) => {
                    self.ack_at = None;
                    if self.now.is_some() {
                        self.want_put(PutStateReason::PLAYER_STATE_CHANGED).await;
                    }
                }
            }
        }

        // the dealer's socket closing is what takes the device off the account's list
        if self.started {
            self.session.dealer().close().await;
        }
    }

    async fn input(&mut self, input: Input) {
        match input {
            Input::Enable(on) => self.enable(on).await,
            Input::Rename(naming) => {
                self.info.name = device_name(&naming);
                self.rename_at = Some(Instant::now() + RENAME_DELAY);
            }
            Input::Publish(now) => self.publish(now).await,
            Input::Transfer(to) => self.transfer(to).await,
            Input::Control(device, command) => {
                let start = match &command {
                    Command::Start(start) => Some(start.clone()),
                    _ => None,
                };
                let sent = control(self.session.clone(), &device, command).await;
                // a pick the other device refused plays here rather than going nowhere
                if let Some(start) = start.filter(|_| !sent) {
                    log::info!("connect: {device} did not take the pick, so it plays here");
                    self.out.send(Event::Command(Command::Start(start))).ok();
                }
            }
        }
    }

    /// Moves playback to `to`. This app takes it the way librespot does, by naming itself on both
    /// ends, and hands it on from itself, or from whichever device the last cluster named.
    async fn transfer(&mut self, to: String) {
        let mine = self.session.device_id().to_owned();
        let from = match (to == mine, self.active, self.active_device.is_empty()) {
            // playback is here already
            (true, true, _) => return,
            (true, false, _) | (false, true, _) => mine.clone(),
            (false, false, false) => self.active_device.clone(),
            (false, false, true) => to.clone(),
        };
        // the device taking over starts from the last state put, so put one as of now
        let blocked = self
            .blocked_until
            .is_some_and(|until| until > Instant::now());
        let put = from == mine && to != mine && self.now.is_some() && !blocked;
        if put {
            self.put(PutStateReason::PLAYER_STATE_CHANGED).await;
        }
        log::info!(
            "connect: moving playback, from here {}, state put {put}, held back {blocked}, at {:?}s, playing {:?}",
            from == mine,
            self.now
                .as_ref()
                .map(|now| self.live_position(now).as_secs()),
            self.now.as_ref().map(|now| now.playing),
        );
        // without this the device that takes over may start paused
        let request = TransferRequest {
            transfer_options: TransferOptions {
                restore_paused: Some("restore".to_owned()),
                ..Default::default()
            },
        };
        // Spotify answers a move here only once this device has answered the command it sends
        // for it, which the loop can give only if it is not waiting on the move itself
        if to == mine {
            let session = self.session.clone();
            self.session.spawn(async move {
                let answer = session
                    .spclient()
                    .transfer(&from, &to, Some(&request))
                    .await;
                if let Err(error) = answer {
                    log::warn!("connect: cannot move playback here: {error}");
                }
            });
            return;
        }
        let answer = self
            .session
            .spclient()
            .transfer(&from, &to, Some(&request))
            .await;
        match answer {
            Ok(_) => {
                log::info!("connect: moved playback");
                // the cluster naming the new device is no longer older than the claim
                if from == mine && to != mine {
                    self.claimed_at = None;
                }
            }
            Err(error) => log::warn!("connect: cannot move playback: {error}"),
        }
    }

    async fn enable(&mut self, on: bool) {
        if self.enabled == on {
            return;
        }
        self.enabled = on;
        match (on, self.started) {
            (true, false) => self.start().await,
            (true, true) if self.connected => self.announce().await,
            (true, true) => {}
            (false, _) => self.withdraw().await,
        }
    }

    async fn start(&mut self) {
        self.started = true;
        match Streams::subscribe(&self.session) {
            Ok(streams) => self.streams = streams,
            Err(error) => return log::warn!("connect: cannot listen to the dealer: {error}"),
        }
        if let Err(error) = self.session.dealer().start().await {
            self.streams = Streams::idle();
            log::warn!("connect: cannot start the dealer: {error}");
        }
    }

    /// The dealer's first message carries the connection id Spotify files this session under,
    /// and the device can only be announced once it is known.
    async fn connection_id(&mut self, id: String) {
        self.session.set_connection_id(&id);
        self.connected = true;
        if self.enabled {
            self.announce().await;
        }
    }

    async fn announce(&mut self) {
        let Some(cluster) = self.put(PutStateReason::NEW_DEVICE).await else {
            return;
        };
        self.active_device = cluster.active_device_id.clone();
        let roster = self.roster(&cluster);
        let elsewhere = match roster.elsewhere.is_some() {
            true => "playback on another device",
            false => "no playback elsewhere",
        };
        log::info!(
            "connect: listed, other devices: {}, {elsewhere}",
            roster.devices.len()
        );
        self.out.send(Event::Roster(roster)).ok();
        // something may have been playing before the device was listed
        if self.now.is_some() {
            self.sync(None).await;
        }
    }

    async fn withdraw(&mut self) {
        self.active = false;
        self.active_since = None;
        self.out.send(Event::Roster(Roster::default())).ok();
        if !self.connected {
            return;
        }
        if let Err(error) = self.session.spclient().delete_connect_state_request().await {
            log::warn!("connect: cannot leave the device list: {error}");
        }
    }

    async fn publish(&mut self, now: Option<NowPlaying>) {
        let before = std::mem::replace(&mut self.now, now);
        self.now_at = Instant::now();
        self.sync(before).await;
    }

    /// How far into its track `now` is at this moment. The app only reports the position when it
    /// jumps, so in between it has moved on by the time since the report.
    fn live_position(&self, now: &NowPlaying) -> Duration {
        let position = match now.playing {
            true => now.position + self.now_at.elapsed(),
            false => now.position,
        };
        match now.duration.is_zero() {
            true => position,
            false => position.min(now.duration),
        }
    }

    /// Puts the state the device is in now, given what it reported `before`.
    async fn sync(&mut self, before: Option<NowPlaying>) {
        if !self.enabled || !self.connected {
            return;
        }
        let Some(now) = self.now.clone() else {
            if self.active {
                self.active = false;
                self.active_since = None;
                if let Err(error) = self
                    .session
                    .spclient()
                    .put_connect_state_inactive(false)
                    .await
                {
                    log::warn!("connect: cannot go inactive: {error}");
                }
            }
            return;
        };

        if !self.active {
            self.active = true;
            self.active_since = Some(SystemTime::now());
            self.claimed_at = Some(Instant::now());
        }
        let only_volume = before.is_some_and(|before| {
            let mut same = before;
            same.volume = now.volume;
            same == now
        });
        let reason = match only_volume {
            true => PutStateReason::VOLUME_CHANGED,
            false => PutStateReason::PLAYER_STATE_CHANGED,
        };
        self.want_put(reason).await;
    }

    /// Asks for the state to be put. Puts are spaced out, and held back for longer each time
    /// Spotify refuses one for coming too often. Whatever the wait, the put that goes out
    /// carries the state as it is then, so the changes in between cost nothing.
    async fn want_put(&mut self, reason: PutStateReason) {
        self.pending = Some(match self.pending {
            Some(held) if rank(held) > rank(reason) => held,
            _ => reason,
        });

        let now = Instant::now();
        let due = [self.last_put.map(|at| at + PUT_GAP), self.blocked_until]
            .into_iter()
            .flatten()
            .max();
        match due {
            Some(due) if due > now => self.flush_at = Some(due),
            _ => self.flush().await,
        }
    }

    /// Puts the state that was asked for, if it still means anything.
    async fn flush(&mut self) {
        self.flush_at = None;
        let Some(reason) = self.pending.take() else {
            return;
        };
        if self.enabled && self.connected && self.now.is_some() {
            self.put(reason).await;
        }
    }

    /// Puts the device's state and returns the account's devices as Spotify answers it.
    async fn put(&mut self, reason: PutStateReason) -> Option<Cluster> {
        let request = self.request_for(reason);
        self.last_put = Some(Instant::now());
        let answer = self
            .session
            .spclient()
            .put_connect_state_request(&request)
            .await;
        match answer {
            Ok(bytes) => {
                self.backoff = Duration::ZERO;
                self.blocked_until = None;
                Cluster::parse_from_bytes(&bytes).ok()
            }
            Err(error) if error.kind == ErrorKind::ResourceExhausted => {
                self.backoff = (self.backoff * 2).clamp(BACKOFF_FIRST, BACKOFF_LAST);
                let until = Instant::now() + self.backoff;
                self.blocked_until = Some(until);
                self.flush_at = Some(until);
                self.pending = Some(match self.pending {
                    Some(held) if rank(held) > rank(reason) => held,
                    _ => reason,
                });
                log::warn!(
                    "connect: Spotify asked for fewer state updates, waiting {}s",
                    self.backoff.as_secs()
                );
                None
            }
            Err(error) => {
                log::warn!("connect: cannot put the state: {error}");
                None
            }
        }
    }

    fn request_for(&self, reason: PutStateReason) -> PutStateRequest {
        let stamp = unix_millis();
        let mut info = self.info.clone();
        if let Some(now) = &self.now {
            info.volume = volume_word(now.volume);
        }
        PutStateRequest {
            member_type: EnumOrUnknown::new(MemberType::CONNECT_STATE),
            put_state_reason: EnumOrUnknown::new(reason),
            is_active: self.active && self.now.is_some(),
            started_playing_at: self.active_since.map(millis).unwrap_or_default(),
            client_side_timestamp: stamp,
            last_command_message_id: self.last_command.0,
            last_command_sent_by_device_id: self.last_command.1.clone(),
            device: MessageField::some(WireDevice {
                device_info: MessageField::some(info),
                player_state: MessageField::some(self.player_state(stamp)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn player_state(&self, stamp: u64) -> PlayerState {
        let mut state = PlayerState {
            session_id: self.session.session_id(),
            is_system_initiated: true,
            playback_speed: 1.,
            play_origin: MessageField::some(PlayOrigin::new()),
            suppressions: MessageField::some(Suppressions::new()),
            options: MessageField::some(ContextPlayerOptions::new()),
            context_uri: UNKNOWN_CONTEXT.to_owned(),
            context_url: format!("context://{UNKNOWN_CONTEXT}"),
            ..Default::default()
        };
        let Some(now) = &self.now else {
            return state;
        };

        // a device taking over loads the context by its uri, so a track with no album or playlist
        // behind it names itself, and what follows it in Sonora goes over as a queue
        let uri = match &now.context {
            Some(context) => self.context_uri(context),
            None => format!("{TRACK_PREFIX}{}", now.track),
        };
        let upcoming = match now.context {
            Some(_) => "context",
            None => "queue",
        };
        state.track = MessageField::some(provided(&now.track, "current", "context", &uri));
        state.next_tracks = now
            .upcoming
            .iter()
            .enumerate()
            .map(|(at, id)| provided(id, &at.to_string(), upcoming, &uri))
            .collect();
        state.context_url = format!("context://{uri}");
        state.context_uri = uri;
        // a device taking over finds the track by where it sits as well as by its uri
        state.index = MessageField::from_option(now.index.map(|track| ContextIndex {
            track: track as u32,
            ..Default::default()
        }));
        state.timestamp = stamp as i64;
        state.position_as_of_timestamp = self.live_position(now).as_millis() as i64;
        state.duration = now.duration.as_millis() as i64;
        state.options = MessageField::some(ContextPlayerOptions {
            shuffling_context: now.shuffle,
            repeating_context: now.repeat == RepeatMode::Context,
            repeating_track: now.repeat == RepeatMode::Track,
            ..Default::default()
        });
        let mut hasher = DefaultHasher::new();
        state
            .next_tracks
            .iter()
            .for_each(|track| track.uri.hash(&mut hasher));
        state.queue_revision = hasher.finish().to_string();

        // desktop and mobile apps want every flag set while paused, or their play button greys out
        match now.playing {
            true => {
                state.is_playing = true;
                state.is_paused = false;
                state.is_buffering = false;
            }
            false => {
                state.is_playing = true;
                state.is_paused = true;
                state.is_buffering = true;
                state.playback_speed = 0.;
            }
        }
        state
    }

    fn context_uri(&self, context: &Collection) -> String {
        collection_uri(context, &self.session.username())
    }

    fn cluster(&mut self, update: ClusterUpdate) {
        let Some(cluster) = update.cluster.as_ref() else {
            return;
        };
        let mine = self.session.device_id();
        let elsewhere = !cluster.active_device_id.is_empty() && cluster.active_device_id != mine;
        let fresh = self
            .claimed_at
            .is_none_or(|claimed| claimed.elapsed() > CLAIM_GRACE);
        self.active_device = cluster.active_device_id.clone();
        if self.active && elsewhere && fresh {
            self.active = false;
            self.active_since = None;
            self.out.send(Event::Command(Command::Released)).ok();
        }
        self.out.send(Event::Roster(self.roster(cluster))).ok();
    }

    fn roster(&self, cluster: &Cluster) -> Roster {
        let mine = self.session.device_id();
        let known = |id: &str, info: &DeviceInfo| Device {
            id: id.to_owned(),
            name: info.name.clone(),
            kind: kind(info),
            volume: info.volume as f32 / u16::MAX as f32,
        };

        let mut devices = cluster
            .device
            .iter()
            .filter(|(id, info)| id.as_str() != mine && info.can_play && !info.capabilities.hidden)
            .map(|(id, info)| known(id, info))
            .collect::<Vec<_>>();
        devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

        let elsewhere = cluster
            .device
            .get(&cluster.active_device_id)
            .filter(|_| cluster.active_device_id != mine)
            .map(|info| {
                let state = &cluster.player_state;
                let millis = |value: i64| Duration::from_millis(value.max(0) as u64);
                Elsewhere {
                    device: known(&cluster.active_device_id, info),
                    track: track_id(&state.track.uri),
                    playing: state.is_playing && !state.is_paused,
                    position: millis(state.position_as_of_timestamp),
                    stamp: UNIX_EPOCH + millis(state.timestamp),
                    duration: millis(state.duration),
                    shuffle: state.options.shuffling_context,
                    repeat: match (
                        state.options.repeating_track,
                        state.options.repeating_context,
                    ) {
                        (true, _) => RepeatMode::Track,
                        (false, true) => RepeatMode::Context,
                        (false, false) => RepeatMode::Off,
                    },
                }
            });
        Roster { devices, elsewhere }
    }

    fn volume(&mut self, command: SetVolumeCommand) {
        let level = command.volume.clamp(0, u16::MAX as i32) as f32 / u16::MAX as f32;
        self.out.send(Event::Command(Command::Volume(level))).ok();
    }

    fn request(&mut self, (request, reply): RequestReply) {
        self.last_command = (request.message_id, request.sent_by_device_id.clone());
        if let Wire::Play(play) = request.command {
            self.play(*play);
            reply.send(Reply::Success).ok();
            self.ack_at = Some(Instant::now() + ACK_DELAY);
            return;
        }
        let commands = self.translate(request.command);
        let verdict = match &commands {
            Some(_) => Reply::Success,
            None => Reply::Failure,
        };
        for command in commands.into_iter().flatten() {
            self.out.send(Event::Command(command)).ok();
        }
        reply.send(verdict).ok();
        self.ack_at = Some(Instant::now() + ACK_DELAY);
    }

    /// Starts what another device asked to play. A phone names the song by its uid alone and
    /// sends no tracks along, so the playlist is read back first to find which song that is.
    fn play(&self, mut play: PlayCommand) {
        let uid = play
            .options
            .skip_to
            .as_ref()
            .filter(|skip| skip.track_uri.as_deref().is_none_or(str::is_empty))
            .and_then(|skip| skip.track_uid.clone())
            .filter(|uid| !uid.is_empty());
        let uri = play.context.uri.clone();
        let out = self.out.clone();
        let session = self.session.clone();
        let blind = play.context.pages.iter().all(|page| page.tracks.is_empty());
        let (Some(_), Some(uri), true) = (uid, uri, blind) else {
            out.send(Event::Command(Command::Start(started_by_play(&play))))
                .ok();
            return;
        };
        self.session.spawn(async move {
            match session.spclient().get_context(&uri).await {
                Ok(context) => play.context.pages = context.pages,
                Err(error) => log::warn!("connect: cannot read {uri} back: {error}"),
            }
            out.send(Event::Command(Command::Start(started_by_play(&play))))
                .ok();
        });
    }

    /// What a command from Spotify asks of the app, or `None` for one this device does not
    /// know.
    fn translate(&self, command: Wire) -> Option<Vec<Command>> {
        Some(match command {
            Wire::Transfer(transfer) => vec![Command::Start(started_by_transfer(transfer.data?))],
            Wire::Play(play) => vec![Command::Start(started_by_play(&play))],
            Wire::Pause(_) => vec![Command::Pause],
            Wire::Resume(_) => vec![Command::Play],
            Wire::SkipNext(_) => vec![Command::Next],
            Wire::SkipPrev(_) => vec![Command::Previous],
            Wire::SeekTo(seek) => vec![Command::Seek(Duration::from_millis(seek.value.into()))],
            Wire::SetShufflingContext(set) => vec![Command::Shuffle(set.value)],
            Wire::SetRepeatingContext(set) => {
                vec![Command::Repeat(self.repeat(Some(set.value), None))]
            }
            Wire::SetRepeatingTrack(set) => {
                vec![Command::Repeat(self.repeat(None, Some(set.value)))]
            }
            Wire::SetOptions(options) => {
                let mut commands = Vec::new();
                if let Some(on) = options.shuffling_context {
                    commands.push(Command::Shuffle(on));
                }
                if options.repeating_context.is_some() || options.repeating_track.is_some() {
                    commands.push(Command::Repeat(
                        self.repeat(options.repeating_context, options.repeating_track),
                    ));
                }
                commands
            }
            Wire::AddToQueue(add) => track_id(&add.track.uri)
                .map(Command::Enqueue)
                .into_iter()
                .collect(),
            // the app keeps its own queue, which it reports back with the next state
            Wire::SetQueue(_) | Wire::UpdateContext(_) => Vec::new(),
            Wire::Unknown(_) => return None,
        })
    }

    /// The repeat mode after a request to switch the context or the track repeat, each of which
    /// leaves the other as it is.
    fn repeat(&self, context: Option<bool>, track: Option<bool>) -> RepeatMode {
        let current = self.now.as_ref().map(|now| now.repeat).unwrap_or_default();
        match (context, track) {
            (_, Some(true)) => RepeatMode::Track,
            (Some(true), _) => RepeatMode::Context,
            (Some(false), _) if current == RepeatMode::Context => RepeatMode::Off,
            (_, Some(false)) if current == RepeatMode::Track => RepeatMode::Off,
            _ => current,
        }
    }
}

/// Sends a command to another device of the account through the session. Returns `false` when
/// the request failed, so a caller can play the pick here instead; `true` when it was sent or
/// there was nothing to send.
async fn control(session: Session, device: &str, command: Command) -> bool {
    let mine = session.device_id();
    let what = format!("{command:?}");
    let (method, endpoint, body) = match command {
        Command::Volume(level) => (
            Method::PUT,
            format!("/connect-state/v1/connect/volume/from/{mine}/to/{device}"),
            serde_json::json!({ "volume": volume_word(level) }),
        ),
        Command::Start(start) => {
            let Some(command) = play_command(&start, &session.username()) else {
                return true;
            };
            (
                Method::POST,
                format!("/connect-state/v1/player/command/from/{mine}/to/{device}"),
                serde_json::json!({ "command": command }),
            )
        }
        command => {
            let Some(command) = wire_command(&command) else {
                return true;
            };
            (
                Method::POST,
                format!("/connect-state/v1/player/command/from/{mine}/to/{device}"),
                serde_json::json!({ "command": command }),
            )
        }
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let body = body.to_string();
    let answer = session
        .spclient()
        .request(&method, &endpoint, Some(headers), Some(body.as_bytes()))
        .await;
    match answer {
        Ok(_) => true,
        Err(error) => {
            log::warn!("connect: cannot send {what} to {device}: {error}");
            false
        }
    }
}

/// How much a put matters when two are due at once: the stronger one goes out.
fn rank(reason: PutStateReason) -> u8 {
    match reason {
        PutStateReason::NEW_DEVICE => 3,
        PutStateReason::PLAYER_STATE_CHANGED => 2,
        _ => 1,
    }
}

/// Resolves when the time for a pending acknowledgement comes, and never when there is none.
async fn acknowledge(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// The name the device goes by in Spotify's device list.
fn device_name(naming: &Naming) -> String {
    let host = gethostname::gethostname();
    let host = host.to_string_lossy();
    let host = host.trim();
    match (naming, host) {
        (Naming::Custom(name), _) if !name.trim().is_empty() => name.trim().to_owned(),
        (Naming::App, _) | (_, "") => DEVICE_NAME.to_owned(),
        (Naming::Computer, host) => host.to_owned(),
        (Naming::AppOnComputer | Naming::Custom(_), host) => format!("{DEVICE_NAME} ({host})"),
    }
}

fn device_info(session: &Session, naming: &Naming) -> DeviceInfo {
    DeviceInfo {
        can_play: true,
        volume: u16::MAX as u32 / 2,
        name: device_name(naming),
        device_id: session.device_id().to_owned(),
        device_type: EnumOrUnknown::new(DeviceType::COMPUTER),
        device_software_version: SEMVER.to_string(),
        spirc_version: SPOTIFY_SPIRC_VERSION.to_string(),
        client_id: session.client_id(),
        capabilities: MessageField::some(Capabilities {
            volume_steps: VOLUME_STEPS as i32,
            gaia_eq_connect_id: true,
            can_be_player: true,
            needs_full_player_state: true,
            is_observable: true,
            is_controllable: true,
            supports_gzip_pushes: true,
            supported_types: vec![
                "audio/episode".into(),
                "audio/track".into(),
                "audio/local".into(),
            ],
            supports_playlist_v2: true,
            supports_transfer_command: true,
            supports_command_request: true,
            supports_set_options_command: true,
            supported_audio_quality: EnumOrUnknown::new(AudioQuality::VERY_HIGH),
            command_acks: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// A command for another device in the JSON the player command endpoint takes.
fn wire_command(command: &Command) -> Option<serde_json::Value> {
    Some(match command {
        Command::Play => serde_json::json!({ "endpoint": "resume" }),
        Command::Pause => serde_json::json!({ "endpoint": "pause" }),
        Command::Next => serde_json::json!({ "endpoint": "skip_next" }),
        Command::Previous => serde_json::json!({ "endpoint": "skip_prev" }),
        Command::Seek(at) => {
            serde_json::json!({ "endpoint": "seek_to", "value": at.as_millis() as u64 })
        }
        Command::Shuffle(on) => {
            serde_json::json!({ "endpoint": "set_shuffling_context", "value": on })
        }
        Command::Repeat(mode) => serde_json::json!({
            "endpoint": "set_options",
            "repeating_context": *mode != RepeatMode::Off,
            "repeating_track": *mode == RepeatMode::Track,
            "logging_params": {},
        }),
        _ => return None,
    })
}

/// A play command asking another device to start `start`: its collection from the track, or the
/// track on its own. `None` when it names neither.
fn play_command(start: &Start, username: &str) -> Option<serde_json::Value> {
    let track = start
        .track
        .as_deref()
        .map(|id| format!("{TRACK_PREFIX}{id}"));
    let context = match &start.collection {
        Some(collection) => collection_uri(collection, username),
        None => track.clone()?,
    };
    Some(serde_json::json!({
        "endpoint": "play",
        "context": { "uri": context, "url": format!("context://{context}"), "metadata": {} },
        "play_origin": { "feature_identifier": "sonora", "feature_version": SEMVER },
        "options": {
            "skip_to": track.map(|uri| serde_json::json!({ "track_uri": uri })),
            "seek_to": start.position.as_millis() as u64,
            "initially_paused": start.paused,
        },
        "logging_params": {},
    }))
}

/// The uri Spotify knows a collection by.
fn collection_uri(collection: &Collection, username: &str) -> String {
    match collection {
        Collection::Album(id) => format!("spotify:album:{id}"),
        Collection::Playlist(id) => format!("spotify:playlist:{id}"),
        Collection::Saved => format!("spotify:user:{username}:collection"),
        Collection::Radio(id) => format!("spotify:station:track:{id}"),
    }
}

fn kind(info: &DeviceInfo) -> DeviceKind {
    match info.device_type.enum_value() {
        Ok(DeviceType::COMPUTER | DeviceType::CHROMEBOOK) => DeviceKind::Computer,
        Ok(DeviceType::SMARTPHONE | DeviceType::SMARTWATCH) => DeviceKind::Phone,
        Ok(DeviceType::TABLET) => DeviceKind::Tablet,
        Ok(
            DeviceType::SPEAKER
            | DeviceType::AVR
            | DeviceType::AUDIO_DONGLE
            | DeviceType::CAST_AUDIO
            | DeviceType::HOME_THING,
        ) => DeviceKind::Speaker,
        Ok(DeviceType::TV | DeviceType::STB | DeviceType::CAST_VIDEO) => DeviceKind::Tv,
        Ok(DeviceType::GAME_CONSOLE) => DeviceKind::Console,
        Ok(DeviceType::AUTOMOBILE | DeviceType::CAR_THING) => DeviceKind::Car,
        _ => DeviceKind::Other,
    }
}

/// A track as the player state lists it, from `provider` (the context or the queue), labelled
/// with the context it plays in as librespot labels its own. Spotify's apps look the track up by
/// its uri.
fn provided(id: &str, uid: &str, provider: &str, context: &str) -> ProvidedTrack {
    ProvidedTrack {
        uri: format!("{TRACK_PREFIX}{id}"),
        uid: format!("sonora-{uid}"),
        provider: provider.to_owned(),
        metadata: [("context_uri", context), ("entity_uri", context)]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
        ..Default::default()
    }
}

fn track_id(uri: &str) -> Option<String> {
    uri.strip_prefix(TRACK_PREFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// The id of a track another device lists, read from its uri or, since Spotify's apps often send
/// only that, from its raw id.
fn listed_id(track: &ContextTrack) -> Option<String> {
    if let Some(id) = track.uri.as_deref().and_then(track_id) {
        return Some(id);
    }
    let gid = track.gid.as_deref().filter(|gid| !gid.is_empty())?;
    Some(SpotifyId::from_raw(gid).ok()?.to_base62())
}

/// The album, playlist or saved tracks a context uri names. Anything else, such as an artist or
/// a radio, has no collection to read back.
fn collection(uri: &str) -> Option<Collection> {
    let parts = uri.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["spotify", "album", id] => Some(Collection::Album((*id).to_owned())),
        ["spotify", "playlist", id] | ["spotify", "user", _, "playlist", id] => {
            Some(Collection::Playlist((*id).to_owned()))
        }
        ["spotify", "user", _, "collection"] => Some(Collection::Saved),
        _ => None,
    }
}

/// Where a transfer from another device leaves off: its track, how far in, and what follows.
fn started_by_transfer(state: TransferState) -> Start {
    let playback = &state.playback;
    let paused = playback.is_paused.unwrap_or_default();
    let mut position = playback.position_as_of_timestamp.unwrap_or_default().max(0) as u64;
    if let (false, Some(stamp)) = (paused, playback.timestamp) {
        // the position was true when the timestamp was taken, and the music went on since
        position += (unix_millis() as i64 - stamp).clamp(0, 60_000) as u64;
    }

    let from_queue = state.queue.is_playing_queue.unwrap_or_default();
    let current = match from_queue {
        true => state.queue.tracks.first(),
        false => playback.current_track.as_ref(),
    };
    let track = current.and_then(listed_id);

    let context = &state.current_session.context;
    let queued = state
        .queue
        .tracks
        .iter()
        .skip(from_queue as usize)
        .filter_map(listed_id);
    let following = context
        .pages
        .iter()
        .flat_map(|page| page.tracks.iter())
        .filter_map(listed_id)
        .skip_while(|id| Some(id) != track.as_ref())
        .skip(1);

    Start {
        collection: context.uri.as_deref().and_then(collection),
        upcoming: queued.chain(following).take(CARRIED).collect(),
        track,
        index: None,
        position: Duration::from_millis(position),
        paused,
    }
}

/// What a play command from another device asks for.
fn started_by_play(play: &PlayCommand) -> Start {
    let listed = play
        .context
        .pages
        .iter()
        .flat_map(|page| page.tracks.iter())
        .filter_map(listed_id)
        .collect::<Vec<_>>();

    // the same order librespot resolves a skip in: uri, then uid, then place
    let skip = play.options.skip_to.as_ref();
    log::debug!(
        "connect: play in {:?}, skip to {skip:?}, {} pages with {} tracks",
        play.context.uri,
        play.context.pages.len(),
        listed.len(),
    );
    let track = skip
        .and_then(|skip| skip.track_uri.as_deref())
        .and_then(track_id)
        .or_else(|| {
            let uid = skip?.track_uid.as_deref().filter(|uid| !uid.is_empty())?;
            play.context
                .pages
                .iter()
                .flat_map(|page| page.tracks.iter())
                .find(|track| track.uid.as_deref() == Some(uid))
                .and_then(listed_id)
        })
        .or_else(|| {
            let index = skip.and_then(|skip| skip.track_index)? as usize;
            // the place counts every entry of the page, including ones that name no track
            play.context
                .pages
                .iter()
                .flat_map(|page| page.tracks.iter())
                .nth(index)
                .and_then(listed_id)
        });
    let index = skip
        .and_then(|skip| skip.track_index)
        .map(|index| index as usize);
    let upcoming = listed
        .into_iter()
        .skip_while(|id| Some(id) != track.as_ref())
        .skip(1)
        .take(CARRIED)
        .collect();

    Start {
        collection: play.context.uri.as_deref().and_then(collection),
        track,
        index,
        upcoming,
        position: Duration::from_millis(play.options.seek_to.unwrap_or_default().into()),
        paused: play.options.initially_paused.unwrap_or_default(),
    }
}

/// A volume from 0 to 1 on Spotify's 16-bit scale.
fn volume_word(level: f32) -> u32 {
    (level.clamp(0., 1.) * u16::MAX as f32).round() as u32
}

fn unix_millis() -> u64 {
    millis(SystemTime::now())
}

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// The tracks a collection update likes or unlikes, as their ids and whether they are liked now.
/// librespot has no message for the update, so its few fields are read by their tags: a list of
/// items, each with its kind (0 for a track), raw id and whether it was removed.
fn likes(bytes: &[u8]) -> protobuf::Result<Vec<(String, bool)>> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let mut likes = Vec::new();
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            ITEM => likes.extend(like(&input.read_bytes()?)?),
            tag => match WireType::new(tag & 7) {
                Some(wire) => input.skip_field(wire)?,
                None => break,
            },
        }
    }
    Ok(likes)
}

fn like(bytes: &[u8]) -> protobuf::Result<Option<(String, bool)>> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let (mut kind, mut gid, mut removed) = (0, Vec::new(), false);
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            ITEM_KIND => kind = input.read_uint64()?,
            ITEM_ID => gid = input.read_bytes()?,
            ITEM_REMOVED => removed = input.read_bool()?,
            tag => match WireType::new(tag & 7) {
                Some(wire) => input.skip_field(wire)?,
                None => break,
            },
        }
    }
    let track = (kind == 0)
        .then(|| SpotifyId::from_raw(&gid).ok())
        .flatten()
        .map(|id| id.to_base62());
    Ok(track.map(|track| (track, !removed)))
}
