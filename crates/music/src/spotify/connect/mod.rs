//! Sonora as a Spotify Connect device. It tells Spotify what plays, so Spotify's apps and
//! Discord show it, takes the commands their controls send, and lists and drives the account's
//! other devices.
//!
//! The worker keeps Sonora's queue authoritative and exchanges state with Spotify's Dealer and
//! Connect endpoints.

mod jam;
pub(super) mod sequence;

pub(super) use jam::INVITE;
use jam::Read;
use spotifyapis::proto::jam::JAM_TOPIC;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::StatusCode;

use crate::spotify::{auth::sign_in_error, session::Session, wire};
use futures::future::BoxFuture;
use sha1::{Digest as _, Sha1};
use spotifyapis::dealer::{self, Command as Wire, Dealer, Message, PlayCommand, Reply, Request};
use spotifyapis::proto::collection::{CollectionUpdate, UPDATE_TOPIC as COLLECTION_TOPIC};
use spotifyapis::proto::connect::{
    CLUSTER_TOPIC, Capabilities, Cluster, ClusterUpdate, ContextIndex, ContextPlayerOptions,
    Device as WireDevice, DeviceInfo, DeviceType, PlayerState, ProvidedTrack, PutStateReason,
    PutStateRequest, SetVolumeCommand, SetVolumeRequest, Suppressions, VOLUME_TOPIC,
};
use spotifyapis::proto::connect::{ContextPage, ContextTrack, TransferState};
use spotifyapis::proto::playlist::{PLAYLIST_TOPIC, PlaylistModificationInfo, USER_TOPIC};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use crate::connect::{
    Collection, Command, Connect, Device, DeviceKind, Elsewhere, Event, Mode, Naming, QueuedTrack,
    Repeat, Request as Input, Roster, Snapshot, Start,
};

const DEVICE_NAME: &str = "Sonora";
const SEMVER: &str = env!("CARGO_PKG_VERSION");
const SPIRC_VERSION: &str = "3.2.6";
const CONNECT_STATE_MEMBER: i32 = 2;
const VERY_HIGH_AUDIO_QUALITY: i32 = 4;
const TRACK_PREFIX: &str = "spotify:track:";
const VOLUME_STEPS: i32 = 64;
/// The context a state names when it has none, which Spotify needs to see to keep the device
/// active.
const UNKNOWN_CONTEXT: &str = "spotify:unknown";
/// How long a command waits for the app to act on it before the state is put regardless, so the
/// device that sent it always hears that it was handled.
const ACK_DELAY: Duration = Duration::from_millis(600);
/// How long after claiming playback a report of another active device is taken to be older than
/// the claim.
const CLAIM_GRACE: Duration = Duration::from_secs(3);
/// Delay before retrying a failed Dealer subscription.
const OBSERVER_RETRY: Duration = Duration::from_secs(10);
/// How many characters of the digest make the uid Spotify's apps give a track they find listed
/// without one.
const DERIVED_UID: usize = 20;
/// The most upcoming tracks a start carries over from the device that sent it.
const CARRIED: usize = 80;
/// The recent history window Spotify includes in its player state.
const PREVIOUS: usize = 10;
/// A fresh cluster snapshot carries the exact shuffled queue and previous-track order.
const QUEUE_SNAPSHOT_AGE: Duration = Duration::from_secs(10);
/// The metadata keys a queued track may name the person who queued it under.
const ADDERS: [&str; 4] = ["added_by", "added_by_username", "adder", "queued_by"];
/// How long a new name settles before it is put, so a name typed in is put once.
const RENAME_DELAY: Duration = Duration::from_millis(800);
/// The least time between two puts of the state. A change that comes sooner waits for the next
/// slot and is put with everything that changed meanwhile.
const PUT_GAP: Duration = Duration::from_secs(1);
/// Reuse an acknowledged playback state only within the observed desktop handoff window.
const TRANSFER_STATE_AGE: Duration = Duration::from_secs(2);
/// How long puts wait after Spotify refuses one for coming too often, and the most they wait
/// after being refused again and again.
const BACKOFF_FIRST: Duration = Duration::from_secs(2);
const BACKOFF_LAST: Duration = Duration::from_secs(60);

/// Starts one worker on the session runtime; it stays idle until device discovery is enabled.
pub fn open(session: Session) -> Connect {
    let (connection, requests, events) = Connect::new(session.device_id().to_owned());
    tokio::spawn(Worker::new(session, events).run(requests));
    connection.with_queue_jump()
}

struct Worker {
    session: Session,
    out: UnboundedSender<Event>,
    name: String,
    dealer: Option<DealerLink>,
    mode: Mode,
    soloist_device: Option<String>,
    seen_connection: bool,
    clock_offset_ms: i64,
    /// A Jam link asked about before the dealer connected, looked up once it has.
    held: Option<String>,
    now: Option<Arc<Snapshot>>,
    /// The id of the run of tracks being reported, new whenever another collection starts, and
    /// the id of the current track's play, new for every track. Spotify's apps name both, and
    /// listeners of a Jam tell a new track from a seek by them.
    run: String,
    play: String,
    /// The uids Spotify itself gives the tracks of the collection being played. Its apps find a
    /// track in a collection by them, so a track reported under any other uid is one they cannot
    /// place, and a listener of a Jam then plays the collection from its top.
    uids: Uids,
    uid_read: Option<BoxFuture<'static, anyhow::Result<sequence::Resolved>>>,
    /// The last state received on this Dealer connection, including a known absence.
    jam: Option<Option<Read>>,
    /// The initial Jam snapshot, which a complete Dealer update can supersede.
    jam_read: Option<BoxFuture<'static, anyhow::Result<Option<Read>>>>,
    active_since: Option<SystemTime>,
    claimed_at: Option<Instant>,
    /// The id and sender of the last command handled, which the next state echoes back.
    last_command: (u32, String),
    ack_at: Option<Instant>,
    rename_at: Option<Instant>,
    playback: ClusterPlayback,
    /// The reason of a put that is due but held back, and when it may go out.
    pending: Option<(Instant, PutStateReason)>,
    last_put: Option<Instant>,
    confirmed_transfer_state: Option<Instant>,
    blocked_until: Option<Instant>,
    backoff: Duration,
}

/// The running Dealer task and its pending observer registration retry.
struct DealerLink {
    socket: Dealer,
    retry_at: Option<Instant>,
}

/// Playback from the latest cluster, with the last remote snapshot available for transfers.
#[derive(Default)]
struct ClusterPlayback {
    device: String,
    player: Option<PlayerState>,
    soloist_options: Option<(bool, Repeat)>,
    soloist_queue: Option<Arc<Snapshot>>,
    queue: Option<TransferQueue>,
}

/// A published remote queue and its age for matching incoming transfers.
struct TransferQueue {
    snapshot: Arc<Snapshot>,
    received: Instant,
}

impl Worker {
    fn new(session: Session, out: UnboundedSender<Event>) -> Self {
        let name = device_name(&Naming::default());
        Self {
            session,
            out,
            name,
            dealer: None,
            mode: Mode::Off,
            soloist_device: None,
            seen_connection: false,
            clock_offset_ms: 0,
            held: None,
            now: None,
            run: String::new(),
            play: String::new(),
            uids: Uids::default(),
            uid_read: None,
            jam: None,
            jam_read: None,
            active_since: None,
            claimed_at: None,
            last_command: (0, String::new()),
            ack_at: None,
            rename_at: None,
            playback: ClusterPlayback::default(),
            pending: None,
            last_put: None,
            confirmed_transfer_state: None,
            blocked_until: None,
            backoff: Duration::ZERO,
        }
    }

    async fn run(mut self, mut inputs: UnboundedReceiver<Input>) {
        loop {
            let retry_at = self.dealer.as_ref().and_then(|link| link.retry_at);
            tokio::select! {
                input = inputs.recv() => {
                    let Some(input) = input else { break };
                    self.input(input).await;
                }
                event = async { self.dealer.as_mut()?.socket.next().await }, if self.dealer.is_some() => {
                    match event {
                        Some(dealer::Event::Connected(id)) => self.connection_id(id).await,
                        Some(dealer::Event::Disconnected) => self.disconnected(),
                        Some(dealer::Event::Message(message)) => {
                            if let Err(error) = self.dealer_message(message) {
                                log::warn!("connect: bad dealer update: {error}");
                            }
                        }
                        Some(dealer::Event::Command(request, reply)) => self.request(*request, reply),
                        None => {
                            self.disconnected();
                            self.dealer = None;
                        }
                    }
                },
                Some(tracks) = async { Some(self.uid_read.as_mut()?.await) }, if self.uid_read.is_some() => {
                    self.learn(tracks);
                }
                Some(read) = async { Some(self.jam_read.as_mut()?.await) }, if self.jam_read.is_some() => {
                    self.finish_jam_read(read);
                }
                () = acknowledge(self.rename_at) => {
                    self.rename_at = None;
                    if self.mode == Mode::Publish
                        && self.dealer_ready()
                        && let Some(cluster) = self.put(PutStateReason::NewDevice).await
                    {
                        self.report_cluster(cluster);
                    }
                }
                () = acknowledge(retry_at) => self.refresh_roster().await,
                () = acknowledge(self.pending.map(|(at, _)| at)) => self.flush().await,
                () = acknowledge(self.ack_at) => {
                    self.ack_at = None;
                    if self.now.is_some() {
                        self.want_put(PutStateReason::PlayerStateChanged);
                    }
                }
            }
            self.sync_soloist_queue().await;
        }
        self.disconnected();
    }

    async fn input(&mut self, input: Input) {
        match input {
            Input::Mode(mode) => self.set_mode(mode).await,
            Input::Rename(naming) => {
                self.name = device_name(&naming);
                if self.mode == Mode::Publish && self.dealer_ready() {
                    self.rename_at = Some(Instant::now() + RENAME_DELAY);
                }
            }
            Input::Publish(now) => self.publish(now).await,
            Input::Transfer(to) => self.transfer(to).await,
            Input::Join(link) => self.join(&link).await,
            Input::Peek(link) => self.peek(&link).await,
            Input::Start => self.start_jam().await,
            Input::Leave => self.leave().await,
            Input::Kick(member) => self.kick(&member).await,
            Input::Control(device, command) => control(&self.session, &device, command).await,
            Input::Jump {
                device,
                track,
                forward,
            } => self.jump(&device, &track, forward).await,
        }
    }

    /// Moves playback between Sonora and another Connect device.
    async fn transfer(&mut self, to: String) {
        if self.playback.device == to {
            return;
        }
        let mine = self.session.device_id().to_owned();
        if self.mode != Mode::Publish && (to == mine || self.playback.device.is_empty()) {
            return;
        }
        let from = match (
            to == mine,
            self.active_since.is_some(),
            self.playback.device.is_empty(),
        ) {
            // playback is here already
            (true, true, _) => return,
            (true, false, _) | (false, true, _) => mine.clone(),
            (false, false, false) => self.playback.device.clone(),
            (false, false, true) => to.clone(),
        };
        // The target starts from the last published state.
        let blocked = self
            .blocked_until
            .is_some_and(|until| until > Instant::now());
        let outbound = from == mine && to != mine && self.now.is_some();
        let reuse = outbound
            && self.pending.is_none()
            && self
                .confirmed_transfer_state
                .is_some_and(|sent| sent.elapsed() <= TRANSFER_STATE_AGE);
        let put = outbound && !blocked && !reuse;
        if put {
            self.put(PutStateReason::PlayerStateChanged).await;
        }
        log::info!(
            "connect: moving playback, from here {}, state put {put}, reused state {reuse}, held back {blocked}, at {:?}s, playing {:?}",
            from == mine,
            self.now.as_ref().map(|now| now.live_position().as_secs()),
            self.now.as_ref().map(|now| now.playing),
        );
        // Spotify answers a move here only once this device has answered the command it sends
        // for it, which the loop can give only if it is not waiting on the move itself
        if to == mine {
            let session = self.session.clone();
            tokio::spawn(async move {
                if let Err(error) = move_playback(&session, &from, &to).await {
                    log::warn!("connect: cannot move playback here: {error}");
                }
            });
            return;
        }
        let answer = move_playback(&self.session, &from, &to).await;
        match answer {
            Ok(_) => {
                log::info!("connect: moved playback");
                // the cluster naming the new device is no longer older than the claim
                if from == mine && to != mine {
                    self.claimed_at = None;
                    self.confirmed_transfer_state = None;
                }
            }
            Err(error) => log::warn!("connect: cannot move playback: {error}"),
        }
    }

    /// Switches between a playable device and a hidden Dealer subscription.
    async fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        let previous = std::mem::replace(&mut self.mode, mode);
        if previous == Mode::Publish {
            self.uid_read = None;
            self.uids = Uids::default();
        }
        if previous != Mode::Off {
            self.withdraw().await;
        }
        self.ack_at = None;
        self.rename_at = None;
        self.pending = None;
        self.soloist_device = None;
        if mode == Mode::Off {
            self.disconnected();
            self.dealer = None;
        } else if self.dealer_ready() {
            self.refresh_roster().await;
        } else if self.dealer.is_none() {
            self.dealer = Some(DealerLink {
                socket: Dealer::start(self.session.api().clone()),
                retry_at: None,
            });
        }
    }

    /// Registers this Dealer connection for playback updates.
    async fn refresh_roster(&mut self) {
        self.retry_registration_at(None);
        match self.mode {
            Mode::Off => {}
            Mode::Publish => self.announce().await,
            Mode::Observe => {
                if self.soloist_device.is_none() {
                    self.soloist_device = tokio::task::spawn_blocking(crate::soloist::device_id)
                        .await
                        .ok()
                        .flatten();
                }
                match register_observer(&self.session).await {
                    Ok(cluster) => self.report_cluster(cluster),
                    Err(error) => {
                        log::warn!("connect: cannot subscribe to playback updates: {error:#}");
                        self.retry_registration_at(Some(Instant::now() + OBSERVER_RETRY));
                    }
                }
            }
        }
    }

    fn retry_registration_at(&mut self, at: Option<Instant>) {
        if let Some(link) = &mut self.dealer {
            link.retry_at = at;
        }
    }

    fn dealer_ready(&self) -> bool {
        self.dealer.is_some() && !self.session.connection_id().is_empty()
    }

    /// Routes account notifications, playback updates, and Jam events.
    fn dealer_message(&mut self, message: Message) -> anyhow::Result<()> {
        let uri = &message.uri;
        if dealer::topic(uri, CLUSTER_TOPIC) {
            let update: ClusterUpdate = message.decode_protobuf()?;
            if let Some(cluster) = update.cluster {
                self.report_cluster(cluster);
            }
        } else if dealer::topic(uri, VOLUME_TOPIC) {
            self.volume(&message.decode_protobuf()?);
        } else if dealer::topic(uri, COLLECTION_TOPIC) {
            if message.has_protobuf() {
                for event in collection_changes(message.decode_protobuf()?) {
                    self.out.send(event).ok();
                }
            }
        } else if self.mode == Mode::Publish && dealer::topic(uri, JAM_TOPIC) {
            self.jam_update(message);
        } else if dealer::topic(uri, PLAYLIST_TOPIC) {
            if !message.has_protobuf() {
                return Ok(());
            }
            let change: PlaylistModificationInfo = message.decode_protobuf()?;
            let uri = change.uri();
            if let Some(id) = uri.strip_prefix("spotify:playlist:") {
                self.out.send(Event::PlaylistChanged(id.to_owned())).ok();
            }
        } else if dealer::topic(uri, USER_TOPIC)
            || dealer::topic(
                uri,
                &format!("hm://playlist/v2/user/{}/rootlist", self.session.username()),
            )
        {
            if message
                .rootlist_revision(self.session.username())
                .is_some_and(|revision| self.session.has_rootlist_revision(&revision))
            {
                return Ok(());
            }
            self.out.send(Event::PlaylistListChanged).ok();
        }
        Ok(())
    }

    /// Clears connection-scoped state until Dealer returns a new connection ID.
    fn disconnected(&mut self) {
        self.confirmed_transfer_state = None;
        self.retry_registration_at(None);
        self.ack_at = None;
        self.pending = None;
        let was_ready = self.dealer_ready();
        self.session.set_connection_id("");
        self.playback = ClusterPlayback::default();
        self.jam = None;
        self.jam_read = None;
        if was_ready {
            self.out.send(Event::Roster(Roster::default())).ok();
        }
        if self.held.take().is_some() {
            self.out.send(Event::Invite(None)).ok();
        }
    }

    /// The dealer's first message carries the connection id Spotify files this session under,
    /// and the device can only be announced once it is known.
    async fn connection_id(&mut self, id: String) {
        self.confirmed_transfer_state = None;
        self.jam = None;
        self.jam_read = None;
        self.session.set_connection_id(&id);
        if self.seen_connection {
            self.out.send(Event::LibraryChanged).ok();
        }
        self.seen_connection = true;
        self.refresh_roster().await;
        if let Some(link) = self.held.take() {
            self.peek(&link).await;
        }
    }

    async fn announce(&mut self) {
        let Some(cluster) = self.put(PutStateReason::NewDevice).await else {
            return;
        };
        self.emit_roster(cluster);
        // the listener may have joined a jam from another app before this one was listed
        self.begin_jam_read();
        // something may have been playing before the device was listed
        if self.now.is_some() && self.active_since.is_none() {
            self.read_uids();
            self.sync(None).await;
        }
    }

    async fn withdraw(&mut self) {
        self.confirmed_transfer_state = None;
        self.active_since = None;
        self.jam = None;
        self.jam_read = None;
        if !self.dealer_ready() {
            return;
        }
        if let Err(error) = self
            .session
            .api()
            .unregister_device(self.session.device_id())
            .await
            .map_err(sign_in_error)
        {
            log::warn!("connect: cannot leave the device list: {error}");
        }
    }

    async fn publish(&mut self, now: Option<Arc<Snapshot>>) {
        self.confirmed_transfer_state = None;
        let before = std::mem::replace(&mut self.now, now);
        self.identify(before.as_deref());
        if self.mode == Mode::Publish {
            self.read_uids();
        }
        self.sync(before).await;
    }

    /// Reads the uids of the collection now playing, unless they are known or being read.
    fn read_uids(&mut self) {
        let Some(context) = self.now.as_deref().and_then(shared) else {
            self.uid_read = None;
            self.uids = Uids::default();
            return;
        };
        let uri = collection_uri(context, self.session.username());
        if self.uids.context == uri {
            return;
        }
        self.uids = Uids {
            context: uri.clone(),
            ..Default::default()
        };
        let session = self.session.clone();
        self.uid_read = Some(Box::pin(
            async move { sequence::resolve(&session, &uri).await },
        ));
    }

    /// Reports the current queue again with Spotify's occurrence IDs.
    fn learn(&mut self, tracks: anyhow::Result<sequence::Resolved>) {
        self.uid_read = None;
        match tracks {
            Ok(tracks) => {
                self.uids.tracks = Some(tracks);
                self.want_put(PutStateReason::PlayerStateChanged);
            }
            Err(error) => {
                log::debug!(
                    "connect: cannot read occurrence IDs for {}: {error}",
                    self.uids.context
                );
                self.uids = Uids::default();
            }
        }
    }

    /// Gives what plays now new ids where it is not what played `before`: the track's play for
    /// another track, and the run as well for another collection, or for another track when
    /// there is no collection and each track is its own.
    fn identify(&mut self, before: Option<&Snapshot>) {
        let Some(now) = self.now.as_deref() else {
            return;
        };
        let moved = before.is_none_or(|before| before.track != now.track);
        let started = before.is_none_or(|before| before.context != now.context)
            || (moved && shared(now).is_none());
        if started || self.run.is_empty() {
            self.run = play_id();
        }
        if moved || started || self.play.is_empty() {
            self.play = play_id();
        }
    }

    /// Puts the state the device is in now, given what it reported `before`.
    async fn sync(&mut self, before: Option<Arc<Snapshot>>) {
        if self.mode != Mode::Publish || !self.dealer_ready() {
            return;
        }
        let Some(now) = &self.now else {
            if self.active_since.take().is_some()
                && let Err(error) = {
                    let connection = self.session.connection_id().to_owned();
                    self.session
                        .api()
                        .deactivate_device(self.session.device_id(), &connection)
                        .await
                        .map_err(sign_in_error)
                }
            {
                log::warn!("connect: cannot go inactive: {error}");
            }
            return;
        };

        if self.active_since.is_none() {
            self.active_since = Some(SystemTime::now());
            self.claimed_at = Some(Instant::now());
        }
        let only_volume = before.is_some_and(|before| now.change(&before) == Some("volume"));
        let reason = match only_volume {
            true => PutStateReason::VolumeChanged,
            false => PutStateReason::PlayerStateChanged,
        };
        self.want_put(reason);
    }

    /// Coalesces state updates until the normal spacing and rate-limit delay have passed.
    fn want_put(&mut self, reason: PutStateReason) {
        let reason = self
            .pending
            .map(|(_, held)| held)
            .filter(|held| rank(*held) > rank(reason))
            .unwrap_or(reason);
        let at = self.last_put.map_or_else(Instant::now, |at| at + PUT_GAP);
        self.pending = Some((self.blocked_until.map_or(at, |limit| at.max(limit)), reason));
    }

    async fn flush(&mut self) {
        let Some((_, reason)) = self.pending.take() else {
            return;
        };
        if self.mode == Mode::Publish && self.dealer_ready() && self.now.is_some() {
            self.put(reason).await;
        }
    }

    /// Puts the device's state and returns the account's devices as Spotify answers it.
    async fn put(&mut self, reason: PutStateReason) -> Option<Cluster> {
        let request = self.request_for(reason);
        let active = request.is_active && self.now.as_ref().is_some_and(|now| now.track.is_some());
        let sent = Instant::now();
        self.last_put = Some(sent);
        self.confirmed_transfer_state = None;
        self.pending = None;
        self.ack_at = None;
        match put_state(&self.session, request).await {
            Ok(cluster) => {
                if active && cluster.active_device_id == self.session.device_id() {
                    self.confirmed_transfer_state = Some(sent);
                }
                self.backoff = Duration::ZERO;
                self.blocked_until = None;
                self.observe_clock(&cluster);
                Some(cluster)
            }
            Err(error)
                if spotifyapis::http_status(&error) == Some(StatusCode::TOO_MANY_REQUESTS) =>
            {
                self.backoff = (self.backoff * 2).clamp(BACKOFF_FIRST, BACKOFF_LAST);
                let until = Instant::now() + self.backoff;
                self.blocked_until = Some(until);
                self.want_put(reason);
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
        let mut info = device_info(&self.session, &self.name, false);
        if let Some(now) = &self.now {
            info.volume = volume_word(now.volume);
        }
        PutStateRequest {
            member_type: CONNECT_STATE_MEMBER,
            put_state_reason: reason as i32,
            is_active: self.active_since.is_some() && self.now.is_some(),
            started_playing_at: self.active_since.map(millis).unwrap_or_default(),
            client_side_timestamp_ms: stamp,
            last_command_message_id: self.last_command.0,
            last_command_sent_by_device_id: self.last_command.1.clone(),
            device: Some(WireDevice {
                device_info: Some(info),
                player_state: Some(self.player_state(stamp)),
            }),
            ..Default::default()
        }
    }

    fn player_state(&self, stamp: u64) -> PlayerState {
        let mut state = PlayerState {
            session_id: self.session.session_id().to_owned(),
            is_system_initiated: true,
            playback_speed: 1.,
            play_origin: Some(Default::default()),
            suppressions: Some(Suppressions::default()),
            options: Some(ContextPlayerOptions::default()),
            context_uri: UNKNOWN_CONTEXT.to_owned(),
            context_url: format!("context://{UNKNOWN_CONTEXT}"),
            ..Default::default()
        };
        let Some(now) = &self.now else {
            return state;
        };
        let Some(track) = &now.track else {
            return state;
        };

        // A standalone song names itself when there is no collection to load on transfer.
        let collection = shared(now);
        let uri = match collection {
            Some(context) => collection_uri(context, self.session.username()),
            None => format!("{TRACK_PREFIX}{}", track.id),
        };
        let entry = |track: &QueuedTrack, index| {
            let uid = self.uids.at(&uri, track, index, collection.is_none());
            provided(track, &uri, uid)
        };
        state.track = Some(entry(track, now.index));
        state.prev_tracks = now
            .previous
            .iter()
            .map(|track| entry(track, None))
            .collect();
        state.next_tracks = now
            .upcoming
            .iter()
            .map(|track| entry(track, None))
            .collect();
        state.context_url = format!("context://{uri}");
        state.context_uri = uri;
        if !self.run.is_empty() {
            state.session_id = self.run.clone();
        }
        state.playback_id = self.play.clone();
        // a device taking over finds the track by where it sits as well as by its uri
        state.index = now.index.map(|track| ContextIndex {
            page: 0,
            track: track as u32,
        });
        state.timestamp_ms = (stamp as i64).saturating_add(self.clock_offset_ms);
        state.position_at_timestamp_ms = now.live_position().as_millis() as i64;
        state.duration_ms = now.duration.as_millis() as i64;
        state.options = Some(ContextPlayerOptions {
            shuffling_context: now.shuffle.unwrap_or_default(),
            repeating_context: now.repeat == Some(Repeat::All),
            repeating_track: now.repeat == Some(Repeat::One),
            ..Default::default()
        });
        let mut hasher = DefaultHasher::new();
        state
            .prev_tracks
            .iter()
            .chain(state.track.iter())
            .chain(&state.next_tracks)
            .for_each(|track| {
                track.uri.hash(&mut hasher);
                track.uid.hash(&mut hasher);
                track.provider.hash(&mut hasher);
            });
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

    fn report_cluster(&mut self, cluster: Cluster) {
        if self.mode == Mode::Off {
            return;
        }
        self.observe_clock(&cluster);
        let mine = self.session.device_id();
        let elsewhere = !cluster.active_device_id.is_empty() && cluster.active_device_id != mine;
        let fresh = self
            .claimed_at
            .is_none_or(|claimed| claimed.elapsed() > CLAIM_GRACE);
        if self.active_since.is_some() && elsewhere && fresh {
            self.active_since = None;
            self.out.send(Event::Command(Command::Released)).ok();
        }
        self.emit_roster(cluster);
    }

    fn emit_roster(&mut self, cluster: Cluster) {
        let roster = self.roster(&cluster);
        let owns_soloist = self.soloist_device.as_deref()
            == Some(cluster.active_device_id.as_str())
            && self.session.soloist_playback().load(Ordering::Relaxed);
        if owns_soloist {
            self.follow_soloist_options(cluster.player_state.as_ref());
        } else {
            self.playback.soloist_options = None;
        }
        self.playback.device = cluster.active_device_id;
        self.playback.player = cluster
            .player_state
            .filter(|_| owns_soloist || roster.elsewhere.is_some());
        if let Some(remote) = &roster.elsewhere
            && self.playback.player.is_some()
        {
            self.playback.queue = Some(TransferQueue {
                snapshot: remote.state.clone(),
                received: Instant::now(),
            });
        }
        self.out.send(Event::Roster(roster)).ok();
    }

    /// Sends Sonora's queue to the active Soloist without publishing another device's state.
    async fn sync_soloist_queue(&mut self) {
        let device = &self.playback.device;
        if self.mode != Mode::Observe
            || self.soloist_device.as_ref() != Some(device)
            || !self.session.soloist_playback().load(Ordering::Relaxed)
        {
            self.playback.soloist_queue = None;
            return;
        }
        let (Some(now), Some(player)) = (&self.now, &self.playback.player) else {
            return;
        };
        if player.track.as_ref().and_then(provided_id)
            != now.track.as_ref().map(|track| track.id.as_str())
            || now.track.is_none()
            || self
                .playback
                .soloist_queue
                .as_ref()
                .is_some_and(|sent| now.same_queue(sent))
        {
            return;
        }
        let matches = |local: &[QueuedTrack], remote: &[ProvidedTrack]| {
            local
                .iter()
                .map(|track| (track.id.as_str(), track.queued))
                .eq(remote
                    .iter()
                    .filter_map(|track| Some((provided_id(track)?, is_queued(track)))))
        };
        if matches(&now.previous, &player.prev_tracks)
            && matches(&now.upcoming, &player.next_tracks)
        {
            return;
        }
        let entries = |tracks: &[QueuedTrack]| {
            tracks
                .iter()
                .map(|track| {
                    let entry = provided(track, &player.context_uri, track.uid.clone());
                    serde_json::json!({
                        "uri": entry.uri,
                        "uid": entry.uid,
                        "provider": entry.provider,
                        "metadata": entry.metadata,
                    })
                })
                .collect::<Vec<_>>()
        };
        let command = serde_json::json!({
            "endpoint": "set_queue",
            "prev_tracks": entries(&now.previous),
            "next_tracks": entries(&now.upcoming),
            "queue_revision": player.queue_revision,
        });
        self.playback.soloist_queue = Some(now.clone());
        if let Err(error) = player_command(&self.session, device, command).await {
            log::warn!("connect: cannot sync the Soloist queue: {error}");
        }
    }

    /// Imports changes after Soloist takes playback without replacing Sonora's starting options.
    fn follow_soloist_options(&mut self, state: Option<&PlayerState>) {
        let Some(options) = state.and_then(|state| state.options.as_ref()) else {
            return;
        };
        let shuffle = options.shuffling_context;
        let repeat = repeat_mode(
            Some(options.repeating_track),
            Some(options.repeating_context),
        )
        .unwrap_or_default();
        let Some((old_shuffle, old_repeat)) =
            self.playback.soloist_options.replace((shuffle, repeat))
        else {
            return;
        };
        if shuffle != old_shuffle {
            self.out
                .send(Event::Command(Command::Shuffle(shuffle)))
                .ok();
        }
        if repeat != old_repeat {
            self.out.send(Event::Command(Command::Repeat(repeat))).ok();
        }
    }

    /// Jumps to an occurrence in the current device's queue with one player command.
    async fn jump(&mut self, device: &str, track: &QueuedTrack, forward: bool) {
        let Some(player) = self
            .playback
            .player
            .as_ref()
            .filter(|_| self.playback.device == device)
        else {
            return;
        };
        let entries = if forward {
            &player.next_tracks
        } else {
            &player.prev_tracks
        };
        let key = Occurrence::from_queue(track);
        let Some(index) = key.resolve(entries.iter().map(Occurrence::from_wire), None, true) else {
            return;
        };
        // A command without a UID cannot distinguish the target from the current occurrence.
        if key
            .resolve(player.track.iter().map(Occurrence::from_wire), None, true)
            .is_some()
        {
            return;
        }
        let target = &entries[index];
        let command = if forward {
            serde_json::json!({
                "endpoint": "skip_next",
                "track": { "uri": target.uri, "uid": target.uid, "metadata": target.metadata },
            })
        } else {
            history_play_command(player, target)
        };
        if let Err(error) = player_command(&self.session, device, command).await {
            log::warn!("connect: cannot jump in the remote queue: {error}");
        }
    }

    /// Uses Connect's server clock so transfers and remote positions share the same time base.
    fn observe_clock(&mut self, cluster: &Cluster) {
        if cluster.server_timestamp_ms > 0 {
            self.clock_offset_ms = cluster
                .server_timestamp_ms
                .saturating_sub(unix_millis() as i64);
        }
    }

    /// Uses a matching fresh player report to preserve queue order that transfer contexts can omit.
    fn transferred(&self, state: TransferState) -> Start {
        let now = (unix_millis() as i64).saturating_add(self.clock_offset_ms);
        let start = started_by_transfer(state, now);
        if let Some(remote) = &self.playback.queue {
            let queue = &remote.snapshot;
            let matching_context = start
                .state
                .context
                .as_ref()
                .is_none_or(|context| queue.context.as_ref() == Some(context));
            let same_occurrence = start.state.track.as_ref().is_some_and(|track| {
                let current = queue.track.iter();
                let neighbors = queue.previous.iter().chain(&queue.upcoming);
                let entries = current
                    .chain(neighbors.filter(|_| track.uid.is_empty()))
                    .map(Occurrence::from_queue);
                queue.track.is_some()
                    && Occurrence::from_queue(track).resolve(entries, None, true) == Some(0)
            });
            if remote.received.elapsed() <= QUEUE_SNAPSHOT_AGE
                && matching_context
                && same_occurrence
            {
                return Start {
                    state: queue.clone(),
                    preserve_queue: true,
                    restart: true,
                };
            }
        }
        start
    }

    fn roster(&self, cluster: &Cluster) -> Roster {
        let mine = self.session.device_id();
        let is_soloist = |id: &str| self.soloist_device.as_deref() == Some(id);
        let known = |id: &str, info: &DeviceInfo| Device {
            id: id.to_owned(),
            name: info.name.clone(),
            kind: kind(info),
            volume: info.volume as f32 / u16::MAX as f32,
        };

        let mut devices = cluster
            .device
            .iter()
            .filter(|(id, info)| {
                id.as_str() != mine
                    && !is_soloist(id)
                    && info.can_play
                    && !info.capabilities.as_ref().is_some_and(|caps| caps.hidden)
            })
            .map(|(id, info)| known(id, info))
            .collect::<Vec<_>>();
        devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

        let local = self.soloist_device.as_deref().and_then(|id| {
            cluster
                .device
                .get(id)
                .filter(|info| info.can_play)
                .map(|info| known(id, info))
        });
        let active = cluster.active_device_id.as_str();
        let owned = self.session.soloist_playback().load(Ordering::Relaxed);
        let elsewhere = cluster
            .device
            .get(&cluster.active_device_id)
            .filter(|_| active != mine && !(is_soloist(active) && owned))
            .map(|info| {
                let empty = PlayerState::default();
                let state = cluster.player_state.as_ref().unwrap_or(&empty);
                let options = state.options.as_ref();
                let millis = |value: i64| Duration::from_millis(value.max(0) as u64);
                Elsewhere {
                    device: known(&cluster.active_device_id, info),
                    state: Arc::new(Snapshot {
                        track: state.track.as_ref().and_then(listed_track),
                        playing: state.is_playing && !state.is_paused,
                        position: millis(state.position_at_timestamp_ms),
                        stamp: Some(
                            UNIX_EPOCH
                                + millis(state.timestamp_ms.saturating_sub(self.clock_offset_ms)),
                        ),
                        duration: millis(state.duration_ms),
                        shuffle: Some(options.is_some_and(|options| options.shuffling_context)),
                        repeat: repeat_mode(
                            Some(options.is_some_and(|options| options.repeating_track)),
                            Some(options.is_some_and(|options| options.repeating_context)),
                        ),
                        volume: info.volume as f32 / u16::MAX as f32,
                        context: collection(&state.context_uri),
                        index: state.index.as_ref().map(|index| index.track as usize),
                        previous: listed(&state.prev_tracks, state.prev_tracks.len()),
                        upcoming: listed(&state.next_tracks, CARRIED),
                    }),
                }
            });
        Roster {
            devices,
            elsewhere,
            local,
        }
    }

    fn volume(&self, command: &SetVolumeCommand) {
        let level = command.volume.clamp(0, u16::MAX as i32) as f32 / u16::MAX as f32;
        self.out.send(Event::Command(Command::Volume(level))).ok();
    }

    fn request(&mut self, request: Request, reply: Reply) {
        if self.mode != Mode::Publish {
            reply.send(false);
            return;
        }
        self.last_command = (request.message_id, request.sent_by_device_id.clone());
        let commands = match request.command {
            Wire::Play(play) => {
                self.play(*play);
                Some(Vec::new())
            }
            command => self.translate(command),
        };
        reply.send(commands.is_some());
        for command in commands.into_iter().flatten() {
            self.out.send(Event::Command(command)).ok();
        }
        self.ack_at = Some(Instant::now() + ACK_DELAY);
    }

    /// Starts what another device asked to play. A phone names the song by its uid alone, and a
    /// jam names a context Sonora cannot read itself. Both send no tracks along, so the context
    /// is read back first to find the song and what follows it.
    fn play(&self, play: PlayCommand) {
        let out = self.out.clone();
        let session = self.session.clone();
        let blind = play.context.pages.iter().all(|page| page.tracks.is_empty());
        let by_uid = play.options.skip_to.as_ref().is_some_and(|skip| {
            skip.track_uri.as_deref().is_none_or(str::is_empty)
                && skip.track_uid.as_deref().is_some_and(|uid| !uid.is_empty())
        });
        let Some(uri) = play
            .context
            .uri
            .as_deref()
            .filter(|uri| blind && (by_uid || collection(uri).is_none()))
            .map(str::to_owned)
        else {
            out.send(Event::Command(Command::Start(started_by_play(play, None))))
                .ok();
            return;
        };
        tokio::spawn(async move {
            let resolved = match sequence::resolve(&session, &uri).await {
                Ok(context) => Some(context),
                Err(error) => {
                    log::warn!("connect: cannot read {uri} back: {error}");
                    None
                }
            };
            out.send(Event::Command(Command::Start(started_by_play(
                play, resolved,
            ))))
            .ok();
        });
    }

    /// Maps Spotify's published occurrence back to Sonora's local queue identity.
    fn select_queue(&self, target: &ProvidedTrack, forward: bool) -> Option<Command> {
        let now = self.now.as_ref()?;
        let state = self.player_state(unix_millis());
        let (listed, local) = if forward {
            (&state.next_tracks, &now.upcoming)
        } else {
            (&state.prev_tracks, &now.previous)
        };
        let target = Occurrence::from_wire(target);
        let index = target.resolve(listed.iter().map(Occurrence::from_wire), None, true)?;
        if target
            .resolve(state.track.iter().map(Occurrence::from_wire), None, true)
            .is_some()
        {
            return None;
        }
        let uid = &local.get(index)?.uid;
        (!uid.is_empty()).then(|| Command::SelectQueue(uid.clone()))
    }

    /// What a command from Spotify asks of the app, or `None` for one this device does not
    /// know.
    fn translate(&mut self, command: Wire) -> Option<Vec<Command>> {
        Some(match command {
            Wire::Transfer { data } => {
                let state = TransferState::from_command_data(&data).ok()?;
                vec![Command::Start(self.transferred(state))]
            }
            Wire::Play(_) | Wire::Unknown => return None,
            Wire::Pause => vec![Command::Pause],
            Wire::Resume => vec![Command::Play],
            Wire::SkipNext { track: Some(track) } => vec![self.select_queue(&track, true)?],
            Wire::SkipPrev { track: Some(track) } => vec![self.select_queue(&track, false)?],
            Wire::SkipNext { track: None } => vec![Command::Next],
            Wire::SkipPrev { track: None } => vec![Command::Previous],
            Wire::SeekTo { value } => vec![Command::Seek(Duration::from_millis(value))],
            Wire::SetShufflingContext { value } => vec![Command::Shuffle(value)],
            Wire::SetRepeatingContext { value } => {
                vec![Command::Repeat(self.repeat(Some(value), None))]
            }
            Wire::SetRepeatingTrack { value } => {
                vec![Command::Repeat(self.repeat(None, Some(value)))]
            }
            Wire::SetOptions {
                shuffling_context,
                repeating_context,
                repeating_track,
            } => {
                let mut commands = Vec::new();
                if let Some(on) = shuffling_context {
                    commands.push(Command::Shuffle(on));
                }
                if repeating_context.is_some() || repeating_track.is_some() {
                    commands.push(Command::Repeat(
                        self.repeat(repeating_context, repeating_track),
                    ));
                }
                commands
            }
            Wire::AddToQueue { track } => track_id(&track.uri)
                .map(|id| Command::Enqueue {
                    id,
                    added_by: adder(&track),
                })
                .into_iter()
                .collect(),
            Wire::SetQueue {
                next_tracks: tracks,
            } => {
                // an edit that added songs runs past the window it was shown, and all of it counts
                let mut upcoming = listed(&tracks, tracks.len());
                if let Some(now) = self.now.as_deref().filter(|now| shared(now).is_none()) {
                    keep_kinds(&mut upcoming, &now.upcoming);
                }
                // Acknowledge the edit with its new queue while the host resolves missing metadata.
                // The history an edit carries is left alone: a phone sends its own or none at
                // all, and no app can edit it, so taking it would only wipe what played here.
                if let Some(now) = self.now.as_mut() {
                    Arc::make_mut(now).upcoming.clone_from(&upcoming);
                }
                vec![Command::SetQueue { upcoming }]
            }
            Wire::UpdateContext => Vec::new(),
        })
    }

    /// The repeat mode after a request to switch the context or the track repeat, each of which
    /// leaves the other as it is.
    fn repeat(&self, context: Option<bool>, track: Option<bool>) -> Repeat {
        let current = self
            .now
            .as_ref()
            .and_then(|now| now.repeat)
            .unwrap_or_default();
        match (context, track) {
            (_, Some(true)) => Repeat::One,
            (Some(true), _) => Repeat::All,
            (Some(false), _) if current == Repeat::All => Repeat::Off,
            (_, Some(false)) if current == Repeat::One => Repeat::Off,
            _ => current,
        }
    }
}

/// Sends device state with the connection ID assigned by Sonora's Dealer socket.
/// Registers this Dealer connection as a hidden, non-playable observer.
async fn register_observer(session: &Session) -> anyhow::Result<Cluster> {
    let request = PutStateRequest {
        device: Some(WireDevice {
            device_info: Some(device_info(session, DEVICE_NAME, true)),
            player_state: Some(PlayerState {
                session_id: session.session_id().to_owned(),
                playback_speed: 1.0,
                options: Some(ContextPlayerOptions::default()),
                suppressions: Some(Suppressions::default()),
                ..Default::default()
            }),
        }),
        member_type: CONNECT_STATE_MEMBER,
        put_state_reason: PutStateReason::NewDevice as i32,
        client_side_timestamp_ms: unix_millis(),
        ..Default::default()
    };
    put_state(session, request).await
}

/// Sends a Connect state update and decodes the account cluster returned by Spotify.
async fn put_state(session: &Session, request: PutStateRequest) -> anyhow::Result<Cluster> {
    let connection = session.connection_id().to_owned();
    session
        .api()
        .put_device_state(session.device_id(), &connection, &request)
        .await
        .map_err(sign_in_error)
}

/// Asks Spotify to move playback from one device to another, restoring a paused state.
async fn move_playback(session: &Session, from: &str, to: &str) -> anyhow::Result<()> {
    let connection = session.connection_id().to_owned();
    session
        .api()
        .transfer_playback(from, to, &connection)
        .await
        .map_err(sign_in_error)
}

/// Sends a control to the remote device without changing the local queue.
async fn control(session: &Session, device: &str, command: Command) {
    let result = match command {
        Command::Volume(level) => {
            let connection = session.connection_id().to_owned();
            session
                .api()
                .set_volume(
                    session.device_id(),
                    device,
                    &connection,
                    &SetVolumeRequest {
                        volume: Some(volume_word(level) as i32),
                    },
                )
                .await
                .map_err(sign_in_error)
        }
        command => {
            let Some(command) = wire_command(&command, session.username()) else {
                return;
            };
            player_command(session, device, command).await
        }
    };
    if let Err(error) = result {
        log::warn!("connect: cannot control {device}: {error}");
    }
}

/// Sends a prepared player command to another Connect device.
async fn player_command(
    session: &Session,
    device: &str,
    command: serde_json::Value,
) -> anyhow::Result<()> {
    let connection = session.connection_id().to_owned();
    session
        .api()
        .player_command(session.device_id(), device, &connection, command)
        .await
        .map_err(sign_in_error)
}

/// How much a put matters when two are due at once: the stronger one goes out.
fn rank(reason: PutStateReason) -> u8 {
    match reason {
        PutStateReason::NewDevice => 3,
        PutStateReason::PlayerStateChanged => 2,
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

/// Capabilities advertised by both Dealer modes, with controls enabled only for publishing.
fn device_info(session: &Session, name: &str, observer: bool) -> DeviceInfo {
    let mut info = DeviceInfo {
        name: name.to_owned(),
        device_id: session.device_id().to_owned(),
        client_id: session.api().client_id().to_owned(),
        device_type: DeviceType::Observer as i32,
        ..Default::default()
    };
    let mut capabilities = Capabilities {
        needs_full_player_state: true,
        is_observable: true,
        supports_gzip_pushes: true,
        supports_playlist_v2: true,
        supported_types: vec!["audio/track".into(), "audio/episode".into()],
        hidden: observer,
        ..Default::default()
    };
    if !observer {
        info.can_play = true;
        info.volume = u16::MAX as u32 / 2;
        info.device_type = DeviceType::Computer as i32;
        info.device_software_version = SEMVER.to_owned();
        info.spirc_version = SPIRC_VERSION.to_owned();
        capabilities = Capabilities {
            volume_steps: VOLUME_STEPS,
            gaia_eq_connect_id: true,
            can_be_player: true,
            is_controllable: true,
            supported_types: vec![
                "audio/episode".into(),
                "audio/track".into(),
                "audio/local".into(),
            ],
            supports_transfer_command: true,
            supports_command_request: true,
            supports_set_options_command: true,
            supported_audio_quality: VERY_HIGH_AUDIO_QUALITY,
            command_acks: true,
            ..capabilities
        };
    }
    info.capabilities = Some(capabilities);
    info
}

/// A command for another device in the JSON the player command endpoint takes.
fn wire_command(command: &Command, username: &str) -> Option<serde_json::Value> {
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
            "repeating_context": *mode != Repeat::Off,
            "repeating_track": *mode == Repeat::One,
            "logging_params": {},
        }),
        Command::Enqueue { id, .. } => serde_json::json!({
            "endpoint": "add_to_queue",
            "track": {
                "uri": format!("{TRACK_PREFIX}{id}"),
                "metadata": { "is_queued": "true" },
                "provider": "queue",
            },
            "logging_params": {},
        }),
        Command::Start(start) => play_command(start, username)?,
        _ => return None,
    })
}

/// A play command asking another device to start `start`: its collection from the track, or the
/// track on its own. `None` when it names neither.
fn play_command(start: &Start, username: &str) -> Option<serde_json::Value> {
    let start = &start.state;
    let track = start
        .track
        .as_ref()
        .map(|track| format!("{TRACK_PREFIX}{}", track.id));
    let context = match &start.context {
        Some(collection) => collection_uri(collection, username),
        None => track.clone()?,
    };
    Some(context_play(
        &context,
        &format!("context://{context}"),
        serde_json::json!({
            "skip_to": track.map(|uri| serde_json::json!({ "track_uri": uri })),
            "seek_to": start.position.as_millis() as u64,
            "initially_paused": !start.playing,
        }),
    ))
}

/// Starts a history entry explicitly because Previous can restart the current song.
fn history_play_command(player: &PlayerState, target: &ProvidedTrack) -> serde_json::Value {
    let context = if is_queued(target) {
        None
    } else {
        target
            .metadata
            .get("context_uri")
            .map(String::as_str)
            .filter(|uri| !uri.is_empty() && *uri != UNKNOWN_CONTEXT)
            .or_else(|| {
                let uri = player.context_uri.as_str();
                (!uri.is_empty() && uri != UNKNOWN_CONTEXT && !uri.starts_with(TRACK_PREFIX))
                    .then_some(uri)
            })
    };
    let skip_to = match context {
        Some(_) if !target.uid.is_empty() => {
            serde_json::json!({ "track_uri": target.uri, "track_uid": target.uid })
        }
        _ => serde_json::json!({ "track_uri": target.uri }),
    };
    let context = context.unwrap_or(&target.uri);
    let url = if context == player.context_uri && !player.context_url.is_empty() {
        player.context_url.clone()
    } else {
        format!("context://{context}")
    };
    let mut options = serde_json::json!({
        "skip_to": skip_to, "seek_to": 0, "initially_paused": false,
    });
    if let Some(current) = &player.options {
        options["player_options_override"] = serde_json::json!({
            "shuffling_context": current.shuffling_context,
            "repeating_context": current.repeating_context,
            "repeating_track": current.repeating_track,
        });
    }
    context_play(context, &url, options)
}

fn context_play(uri: &str, url: &str, options: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "endpoint": "play",
        "context": { "uri": uri, "url": url, "metadata": {} },
        "play_origin": { "feature_identifier": "sonora", "feature_version": SEMVER },
        "options": options,
        "logging_params": {},
    })
}

/// The repeat mode a player state's two flags name, when it says either.
fn repeat_mode(track: Option<bool>, context: Option<bool>) -> Option<Repeat> {
    match (track, context) {
        (Some(true), _) => Some(Repeat::One),
        (_, Some(true)) => Some(Repeat::All),
        (Some(false), _) | (_, Some(false)) => Some(Repeat::Off),
        _ => None,
    }
}

/// The collection `now` plays from when another app reads the same songs from it. A radio is not
/// one: Spotify draws it afresh for whoever asks, so the songs this app got are told as a list
/// of their own, the way a song from no collection is.
fn shared(now: &Snapshot) -> Option<&Collection> {
    now.context.as_ref().filter(|context| {
        !matches!(
            context,
            Collection::Radio(_) | Collection::CollectionRadio(..)
        )
    })
}

/// The uri Spotify knows a collection by.
fn collection_uri(collection: &Collection, username: &str) -> String {
    match collection {
        Collection::Album(id) => format!("spotify:album:{id}"),
        Collection::Playlist(id) => format!("spotify:playlist:{id}"),
        Collection::Saved => format!("spotify:user:{username}:collection"),
        Collection::Radio(id) => format!("spotify:station:track:{id}"),
        Collection::CollectionRadio(seed, id) => {
            format!("spotify:station:{}:{id}", sequence::radio_kind(*seed))
        }
    }
}

fn kind(info: &DeviceInfo) -> DeviceKind {
    match DeviceType::try_from(info.device_type) {
        Ok(DeviceType::Computer | DeviceType::Chromebook) => DeviceKind::Computer,
        Ok(DeviceType::Smartphone | DeviceType::Smartwatch) => DeviceKind::Phone,
        Ok(DeviceType::Tablet) => DeviceKind::Tablet,
        Ok(
            DeviceType::Speaker
            | DeviceType::Avr
            | DeviceType::AudioDongle
            | DeviceType::CastAudio
            | DeviceType::HomeThing,
        ) => DeviceKind::Speaker,
        Ok(DeviceType::Tv | DeviceType::Stb | DeviceType::CastVideo) => DeviceKind::Tv,
        Ok(DeviceType::GameConsole) => DeviceKind::Console,
        Ok(DeviceType::Automobile | DeviceType::CarThing) => DeviceKind::Car,
        _ => DeviceKind::Other,
    }
}

/// Track identity shared by Connect's context, player state and command payloads.
#[derive(Default)]
struct Occurrence<'a> {
    id: Option<CatalogueId<'a>>,
    uid: Option<&'a str>,
}

/// A borrowed catalogue ID or the inline base-62 form of a binary GID.
enum CatalogueId<'a> {
    Borrowed(&'a str),
    Encoded([u8; 22]),
}

impl CatalogueId<'_> {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(id) => id.as_bytes(),
            Self::Encoded(id) => id,
        }
    }

    fn into_owned(self) -> String {
        match self {
            Self::Borrowed(id) => id.to_owned(),
            Self::Encoded(id) => String::from_utf8_lossy(&id).into_owned(),
        }
    }
}

impl<'a> Occurrence<'a> {
    fn from_wire(track: &'a ProvidedTrack) -> Self {
        Self {
            id: provided_id(track).map(CatalogueId::Borrowed),
            uid: Some(&track.uid),
        }
    }

    fn from_context(track: &'a ContextTrack) -> Self {
        Self {
            id: context_id(track),
            uid: track.uid.as_deref(),
        }
    }

    fn from_queue(track: &'a QueuedTrack) -> Self {
        Self {
            id: Some(CatalogueId::Borrowed(&track.id)),
            uid: Some(&track.uid),
        }
    }

    /// Prefers a matching position, then a unique UID or track ID. A partial list
    /// cannot establish uniqueness by track ID alone.
    fn resolve(
        &self,
        entries: impl IntoIterator<Item = Self>,
        position: Option<usize>,
        complete: bool,
    ) -> Option<usize> {
        let uid = self.uid.filter(|uid| !uid.is_empty());
        let mut matches = entries.into_iter().enumerate().filter(|(_, entry)| {
            self.id.as_ref().is_none_or(|id| {
                entry.id.as_ref().map(CatalogueId::as_bytes) == Some(id.as_bytes())
            }) && uid.is_none_or(|uid| entry.uid == Some(uid))
        });
        let (index, _) = matches.next()?;
        if Some(index) == position {
            return Some(index);
        }
        let duplicate = matches.next();
        if duplicate.is_none() && (uid.is_some() || (complete && self.id.is_some())) {
            return Some(index);
        }
        let position = position?;
        duplicate
            .into_iter()
            .chain(matches)
            .any(|(index, _)| index == position)
            .then_some(position)
    }
}

/// The occurrence IDs Spotify supplies for the currently published context.
#[derive(Default)]
struct Uids {
    context: String,
    tracks: Option<sequence::Resolved>,
}

impl Uids {
    /// Reports the context occurrence only when its identity can be resolved.
    fn at(
        &self,
        context: &str,
        track: &QueuedTrack,
        index: Option<usize>,
        standalone: bool,
    ) -> String {
        if track.queued {
            return track.uid.clone();
        }
        let derived = || derived_uid(&track.id);
        let listed = self.tracks.as_ref().filter(|_| self.context == context);
        if standalone || listed.is_some_and(|rows| rows.complete && rows.tracks.is_empty()) {
            return derived();
        }
        listed
            .and_then(|rows| {
                let mut key = Occurrence::from_queue(track);
                // Local queue identities have no corresponding UID in Spotify's context.
                key.uid = key.uid.filter(|uid| !uid.starts_with("sonora-"));
                rows.position(key, index).map(|index| &rows.tracks[index])
            })
            .map(|row| {
                row.uid
                    .as_deref()
                    .filter(|uid| !uid.is_empty())
                    .map_or_else(derived, str::to_owned)
            })
            .unwrap_or_default()
    }
}

/// The uid Spotify's apps give a track listed without one: the start of the SHA-1 of its uri.
fn derived_uid(id: &str) -> String {
    let mut hash = Sha1::new();
    hash.update(TRACK_PREFIX);
    hash.update(id);
    let digest = hash.finalize();
    let mut uid = String::with_capacity(DERIVED_UID);
    wire::append_hex(&mut uid, &digest[..DERIVED_UID / 2]);
    uid
}

/// A page can contain tracks or serve as a pointer to the first page with data.
fn continuation(page: &ContextPage) -> Option<&str> {
    page.page_url
        .as_deref()
        .filter(|_| page.tracks.is_empty())
        .or(page.next_page_url.as_deref())
        .filter(|url| !url.is_empty())
}

/// Gives the songs of an edited queue back the kind they have here, where this app plays a list
/// that is no collection of Spotify's. The app that made the edit knows no collection the songs
/// could come from, so it sends every one of them back as queued by hand. A song is taken for
/// one of `before` once, a queued one first, so a song queued a second time stays queued.
fn keep_kinds(upcoming: &mut [QueuedTrack], before: &[QueuedTrack]) {
    let mut pool: Vec<&QueuedTrack> = before.iter().collect();
    for track in upcoming.iter_mut().filter(|track| track.queued) {
        let same = pool
            .iter()
            .position(|known| known.id == track.id && known.queued)
            .or_else(|| pool.iter().position(|known| known.id == track.id));
        if let Some(index) = same {
            let known = pool.remove(index);
            track.queued = known.queued;
            track.suggested = known.suggested;
        }
    }
}

/// Reports the occurrence's provenance consistently in every part of play order, under `uid`.
fn provided(track: &QueuedTrack, context: &str, uid: String) -> ProvidedTrack {
    let mut provided = ProvidedTrack {
        uri: format!("{TRACK_PREFIX}{}", track.id),
        uid,
        // Spotify's apps list what follows a collection apart from it, under this provider
        provider: match (track.queued, track.suggested) {
            (true, _) => "queue",
            (false, true) => "autoplay",
            (false, false) => "context",
        }
        .to_owned(),
        metadata: [("context_uri", context), ("entity_uri", context)]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
        ..Default::default()
    };
    if track.queued {
        // Spotify's apps mark a song queued by hand this way, and one that joins a Jam while
        // such a song plays only takes it for the current one with the mark
        provided
            .metadata
            .insert("is_queued".to_owned(), "true".to_owned());
    }
    if let Some(adder) = &track.added_by {
        provided
            .metadata
            .insert("added_by".to_owned(), adder.clone());
    }
    provided
}

fn track_id(uri: &str) -> Option<String> {
    uri_id(uri).map(str::to_owned)
}

fn uri_id(uri: &str) -> Option<&str> {
    uri.strip_prefix(TRACK_PREFIX).filter(|id| !id.is_empty())
}

/// Preserves the requested recording when Spotify substitutes a playable version.
fn provided_id(track: &ProvidedTrack) -> Option<&str> {
    let reported = uri_id(&track.uri)?;
    Some(
        track
            .metadata
            .get("requested_uri")
            .and_then(|uri| uri_id(uri))
            .filter(|id| id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .unwrap_or(reported),
    )
}

/// The first tracks of a player state's queue, with who queued each when the state says.
fn listed(tracks: &[ProvidedTrack], limit: usize) -> Vec<QueuedTrack> {
    tracks.iter().filter_map(listed_track).take(limit).collect()
}

/// Reads the same occurrence provenance for current, previous and upcoming tracks.
fn listed_track(track: &ProvidedTrack) -> Option<QueuedTrack> {
    Some(QueuedTrack {
        id: provided_id(track)?.to_owned(),
        uid: track.uid.clone(),
        queued: is_queued(track),
        // what Spotify plays on by itself when a collection runs out
        suggested: track.provider == "autoplay",
        added_by: adder(track),
    })
}

/// Who queued a track, read from the first of the metadata keys Spotify names them by.
fn adder(track: &ProvidedTrack) -> Option<String> {
    ADDERS
        .iter()
        .find_map(|key| track.metadata.get(*key))
        .filter(|name| !name.is_empty())
        .cloned()
}

/// Reads explicit queue provenance independently of the track's current position.
fn is_queued(track: &ProvidedTrack) -> bool {
    track.provider == "queue"
        || track
            .metadata
            .get("is_queued")
            .is_some_and(|value| value == "true")
}

/// The id of a track another device lists, read from its uri or, since Spotify's apps often send
/// only that, from its raw id.
fn context_id(track: &ContextTrack) -> Option<CatalogueId<'_>> {
    if let Some(id) = track.uri.as_deref().and_then(uri_id) {
        return Some(CatalogueId::Borrowed(id));
    }
    let gid = track.gid.as_ref()?;
    spotifyapis::id::base62_bytes(gid).map(CatalogueId::Encoded)
}

/// Retains Spotify's occurrence UID when reconstructing a local queue from context rows.
fn context_entry(track: &ContextTrack, queued: bool) -> Option<QueuedTrack> {
    let mut entry = QueuedTrack::new(context_id(track)?.into_owned(), queued);
    entry.uid = track.uid.clone().unwrap_or_default();
    Some(entry)
}

/// The album, playlist or saved tracks a context uri names. Anything else, such as an artist or
/// a radio, has no collection to read back.
fn collection(uri: &str) -> Option<Collection> {
    let mut parts = uri.strip_prefix("spotify:")?.split(':');
    let kind = parts.next()?;
    let id = parts.next()?;
    match (kind, parts.next(), parts.next(), parts.next()) {
        ("album", None, None, None) => Some(Collection::Album(id.to_owned())),
        ("playlist", None, None, None) => Some(Collection::Playlist(id.to_owned())),
        ("user", Some("playlist"), Some(id), None) => Some(Collection::Playlist(id.to_owned())),
        ("user", Some("collection"), None, None) => Some(Collection::Saved),
        _ => None,
    }
}

impl sequence::Resolved {
    fn from_pages(pages: Vec<ContextPage>) -> Self {
        Self {
            complete: pages.last().and_then(continuation).is_none(),
            tracks: pages.into_iter().flat_map(|page| page.tracks).collect(),
        }
    }

    fn position(&self, key: Occurrence<'_>, index: Option<usize>) -> Option<usize> {
        key.resolve(
            self.tracks.iter().map(Occurrence::from_context),
            index,
            self.complete,
        )
    }

    fn following(&self, index: Option<usize>) -> impl Iterator<Item = QueuedTrack> + '_ {
        index
            .into_iter()
            .flat_map(|index| &self.tracks[index + 1..])
            .filter_map(|track| context_entry(track, false))
    }
}

/// Where a transfer from another device leaves off: its track, how far in, and what follows.
fn started_by_transfer(state: TransferState, now: i64) -> Start {
    let playback = state.playback.unwrap_or_default();
    let queue = state.queue.unwrap_or_default();
    let options = state.options.unwrap_or_default();
    let paused = playback.is_paused.unwrap_or_default();
    let mut position = playback.position_at_timestamp_ms.unwrap_or_default().max(0) as u64;
    if let (false, Some(stamp)) = (paused, playback.timestamp_ms) {
        // the position was true when the timestamp was taken, and the music went on since
        position += now.saturating_sub(stamp).clamp(0, 60_000) as u64;
    }

    let from_queue = queue.is_playing_queue.unwrap_or_default();
    let current = match from_queue {
        true => queue.tracks.first(),
        false => playback.current_track.as_ref(),
    };
    let context = state
        .current_session
        .and_then(|session| session.context)
        .unwrap_or_default();
    let rows = sequence::Resolved::from_pages(context.pages);
    let place = playback
        .current_track
        .as_ref()
        .and_then(|track| rows.position(Occurrence::from_context(track), None));
    let mut previous: Vec<_> = state
        .play_history
        .iter()
        .flat_map(|history| &history.backward_items)
        .filter_map(|item| {
            let key = Occurrence {
                uid: item.uid.as_deref(),
                ..Default::default()
            };
            let index = rows.position(key, None)?;
            context_entry(&rows.tracks[index], false)
        })
        .collect();
    if previous.is_empty() {
        previous = rows
            .tracks
            .iter()
            .take(place.unwrap_or(0) + usize::from(from_queue && place.is_some()))
            .filter_map(|track| context_entry(track, false))
            .collect();
    }
    previous.drain(..previous.len().saturating_sub(PREVIOUS));
    let queued = queue
        .tracks
        .iter()
        .skip(from_queue as usize)
        .filter_map(|track| context_entry(track, true));
    let repeat = repeat_mode(options.repeating_track, options.repeating_context);
    Start {
        state: Arc::new(Snapshot {
            context: context.uri.as_deref().and_then(collection),
            upcoming: queued.chain(rows.following(place)).take(CARRIED).collect(),
            previous,
            shuffle: options.shuffling_context,
            repeat,
            track: current.and_then(|track| context_entry(track, from_queue)),
            index: place,
            position: Duration::from_millis(position),
            playing: !paused,
            stamp: Some(SystemTime::now()),
            ..Default::default()
        }),
        preserve_queue: true,
        restart: true,
    }
}

/// Whether a play command names a context that Sonora has no collection to read back, such as a
/// jam, so its tracks can only come from the context itself.
/// What a play command from another device asks for.
fn started_by_play(play: PlayCommand, resolved: Option<sequence::Resolved>) -> Start {
    let rows = resolved.unwrap_or_else(|| sequence::Resolved::from_pages(play.context.pages));
    let skip = play.options.skip_to.as_ref();
    let id = skip
        .and_then(|skip| skip.track_uri.as_deref())
        .and_then(uri_id);
    let key = Occurrence {
        id: id.map(CatalogueId::Borrowed),
        uid: skip.and_then(|skip| skip.track_uid.as_deref()),
    };
    let requested = skip
        .and_then(|skip| skip.track_index)
        .map(|index| index as usize);
    let index = rows.position(key, requested);
    let track = index
        .and_then(|index| context_entry(&rows.tracks[index], false))
        .or_else(|| id.map(|id| QueuedTrack::new(id.to_owned(), false)));
    let upcoming = rows.following(index).take(CARRIED).collect();
    Start {
        state: Arc::new(Snapshot {
            context: play.context.uri.as_deref().and_then(collection),
            track,
            index: index.or_else(|| requested.filter(|_| rows.tracks.is_empty())),
            upcoming,
            position: Duration::from_millis(play.options.seek_to.unwrap_or_default()),
            playing: !play.options.initially_paused.unwrap_or_default(),
            shuffle: play
                .options
                .player_options_override
                .as_ref()
                .and_then(|options| options.shuffling_context),
            ..Default::default()
        }),
        preserve_queue: false,
        restart: true,
    }
}

/// A fresh id for a run of tracks or one track's play, in the 32 hex digits Spotify's apps use.
fn play_id() -> String {
    format!("{:032x}", fastrand::u128(..))
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

/// Track deltas and invalidations for other saved collections reported by Dealer.
fn collection_changes(update: CollectionUpdate) -> impl Iterator<Item = Event> {
    update.items.into_iter().filter_map(|item| match item.kind {
        0 => spotifyapis::id::base62(item.gid.as_ref()?).map(|id| Event::Liked {
            track: id,
            liked: !item.removed,
        }),
        _ => Some(Event::LibraryChanged),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_queue_resolves_an_occurrence_before_the_last_page() {
        let tracks = [("a", "first"), ("a", "second"), ("b", "last")]
            .map(|(id, uid)| ContextTrack {
                uri: Some(format!("spotify:track:{id}")),
                uid: Some(uid.into()),
                ..Default::default()
            })
            .into();
        let mut rows = sequence::Resolved::from_pages(vec![ContextPage {
            tracks,
            next_page_url: Some("next".into()),
            ..Default::default()
        }]);
        let track = |id| Occurrence {
            id: Some(CatalogueId::Borrowed(id)),
            uid: None,
        };
        let index = rows.position(Occurrence::from_context(&rows.tracks[1]), None);
        assert_eq!(index, Some(1));
        assert_eq!(
            rows.following(index).map(|track| track.uid).next(),
            Some("last".into())
        );
        assert_eq!(rows.position(track("a"), None), None);
        assert_eq!(rows.position(track("b"), None), None);
        rows.complete = true;
        assert_eq!(rows.position(track("b"), None), Some(2));
    }
}
