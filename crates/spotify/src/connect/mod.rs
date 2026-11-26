//! Publishes Sonora playback and handles Spotify Connect commands and device state.

mod jam;
pub(super) mod sequence;

pub(super) use jam::INVITE;
use jam::Read;
use spotifyapis::proto::social_connect::JAM_TOPIC;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use http::StatusCode;

use crate::{auth::sign_in_error, session::Session, wire};
use futures::future::BoxFuture;
use sha1::{Digest as _, Sha1};
use spotifyapis::dealer::{self, Command as Wire, Dealer, Message, PlayCommand, Reply, Request};
use spotifyapis::id::{CatalogueId, TRACK_PREFIX, track_id as uri_id, track_uri};
use spotifyapis::player_command::{ConnectDevice, Play};
use spotifyapis::proto::collection::{CollectionUpdate, UPDATE_TOPIC as COLLECTION_TOPIC};
use spotifyapis::proto::connect_state::{
    CLUSTER_TOPIC, Capabilities, Cluster, ClusterUpdate, ContextIndex, ContextPlayerOptions,
    Device as WireDevice, DeviceInfo, DeviceType, PlayerState, ProvidedTrack, PutStateReason,
    PutStateRequest, SetVolumeCommand, Suppressions, VOLUME_TOPIC,
};
use spotifyapis::proto::connect_state::{ContextTrack, TransferState};
use spotifyapis::proto::playlist_v2::{PLAYLIST_TOPIC, PlaylistModificationInfo, USER_TOPIC};
use spotifyapis::proto::sequence_proxy::ResolvedSequence;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use music::connect::{
    Collection, Command, Connect, Device, DeviceKind, Elsewhere, Event, Mode, Naming, QueuedTrack,
    Repeat, Request as Input, Roster, Snapshot, Start,
};

const DEVICE_NAME: &str = "Sonora";
const SEMVER: &str = env!("CARGO_PKG_VERSION");
const SPIRC_VERSION: &str = "3.2.6";
const CONNECT_STATE_MEMBER: i32 = 2;
const VERY_HIGH_AUDIO_QUALITY: i32 = 4;
const VOLUME_STEPS: i32 = 64;
// Idle state needs a placeholder context to remain registered.
const UNKNOWN_CONTEXT: &str = "spotify:unknown";
/// The provider Spotify gives a song that smart shuffle mixed into a playlist.
const RECOMMENDED: &str = "enhanced_recommendation";
/// The lens that asks Spotify for a playlist with smart shuffle's recommendations mixed in.
const LENS: &str = "?spotify-apply-lenses=enhance";
/// The player mode that says whether smart shuffle is on.
const ENHANCEMENT: &str = "context_enhancement";
/// The value of `ENHANCEMENT` when smart shuffle is on.
const RECOMMENDATION: &str = "RECOMMENDATION";
/// Smart shuffle is offered on playlists alone.
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const DERIVED_UID: usize = 20;
const CARRIED: usize = 80;
const PREVIOUS: usize = 10;

// Give playback time to apply a remote command before publishing its resulting state.
const ACK_DELAY: Duration = Duration::from_millis(600);
const RENAME_DELAY: Duration = Duration::from_millis(800);
const OBSERVER_RETRY: Duration = Duration::from_secs(10);
// Give cluster updates time to reflect a new local playback claim.
const CLAIM_GRACE: Duration = Duration::from_secs(3);
const QUEUE_SNAPSHOT_AGE: Duration = Duration::from_secs(10);

// State publication has separate thresholds for routine updates, handoffs, and throttling.
const PUT_GAP: Duration = Duration::from_secs(1);
const TRANSFER_STATE_AGE: Duration = Duration::from_secs(2);
const BACKOFF_FIRST: Duration = Duration::from_secs(2);
const BACKOFF_LAST: Duration = Duration::from_secs(60);
const TRANSIENT_PUT_RETRIES: u8 = 3;

/// Starts the worker, idle until device discovery is enabled.
pub fn open(session: Session) -> Connect {
    let (connection, requests, events) = Connect::new(session.device_id().to_owned());
    tokio::spawn(Worker::new(session, events).run(requests));
    connection.with_queue_jump()
}

struct Worker {
    session: Session,
    out: UnboundedSender<Event>,
    name: String,
    dealer: Option<Dealer>,
    registration_retry: Option<Instant>,
    mode: Mode,
    soloist_device: Option<String>,
    soloist_ready: bool,
    local_control: Option<(Instant, Command)>,
    paused_soloist: Option<Elsewhere>,
    seen_connection: bool,
    clock_offset_ms: i64,
    /// Jam invite waiting for Dealer to connect.
    held: Option<String>,
    now: Option<Arc<Snapshot>>,
    /// Playback IDs distinguish context changes, track changes, and seeks in Jam.
    run: String,
    play: String,
    /// Spotify's occurrence IDs let other clients locate tracks in the shared context.
    uids: Uids,
    uid_read: Option<BoxFuture<'static, anyhow::Result<ResolvedSequence>>>,
    /// A play command waiting for its context, and the playlist it plays through smart shuffle.
    play_read: Option<(PlayRead, Reply)>,
    /// Whether the listener switched smart shuffle on for this device. A phone sets it once and
    /// then sends plain play commands, which this mode turns into smart shuffle ones.
    smart_mode: bool,
    /// The playlist that plays here with smart shuffle's recommendations mixed in.
    smart: Option<String>,
    /// The playlist read through the smart shuffle lens after smart shuffle was switched on
    /// during playback.
    smart_read: Option<BoxFuture<'static, (String, anyhow::Result<ResolvedSequence>)>>,
    /// Jam state for this connection, including confirmed absence.
    jam: Option<Option<Read>>,
    /// The initial Jam snapshot, which a complete Dealer update can supersede.
    jam_read: Option<BoxFuture<'static, anyhow::Result<Option<Read>>>>,
    active_since: Option<SystemTime>,
    claimed_at: Option<Instant>,
    /// Command ID and sender echoed in the next state update.
    last_command: (u32, String),
    ack_at: Option<Instant>,
    rename_at: Option<Instant>,
    playback: ClusterPlayback,
    pending: Option<PutStateReason>,
    last_put: Option<Instant>,
    confirmed_transfer_state: Option<Instant>,
    backoff: Option<(Instant, Duration)>,
    /// Retries scheduled since the last success, terminal failure, or disconnect.
    transient_put_retries: u8,
}

/// A play command's start once its context is read, and the playlist it mixes recommendations
/// into when it plays through smart shuffle.
type PlayRead = BoxFuture<'static, (Option<Start>, Option<String>)>;

/// Playback from the latest cluster, with the last remote snapshot available for transfers.
#[derive(Default)]
struct ClusterPlayback {
    device: String,
    player: Option<PlayerState>,
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
            registration_retry: None,
            mode: Mode::Off,
            soloist_device: None,
            soloist_ready: false,
            local_control: None,
            paused_soloist: None,
            seen_connection: false,
            clock_offset_ms: 0,
            held: None,
            now: None,
            run: String::new(),
            play: String::new(),
            uids: Uids::default(),
            uid_read: None,
            play_read: None,
            smart_mode: false,
            smart: None,
            smart_read: None,
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
            backoff: None,
            transient_put_retries: 0,
        }
    }

    async fn run(mut self, mut inputs: UnboundedReceiver<Input>) {
        loop {
            let put_at = self.put_at();
            tokio::select! {
                input = inputs.recv() => {
                    let Some(input) = input else { break };
                    self.input(input).await;
                }
                event = when_ready(self.dealer.as_mut().map(Dealer::next)) => {
                    match event {
                        Some(dealer::Event::Connected(id)) => self.connection_id(id).await,
                        Some(dealer::Event::Disconnected) => self.disconnected(),
                        Some(dealer::Event::Message(message)) => {
                            if let Err(error) = self.dealer_message(message).await {
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
                tracks = when_ready(self.uid_read.as_mut()) => {
                    self.learn(tracks);
                }
                (start, smart) = when_ready(self.play_read.as_mut().map(|(read, _)| read)) => {
                    if let Some((_, reply)) = self.play_read.take() {
                        self.smart = smart;
                        self.finish_play(start, reply);
                    }
                }
                (uri, read) = when_ready(self.smart_read.as_mut()) => {
                    self.smart_read = None;
                    self.enhanced(uri, read);
                }
                read = when_ready(self.jam_read.as_mut()) => {
                    self.finish_jam_read(read);
                }
                () = wait_until(self.rename_at) => {
                    self.rename_at = None;
                    if self.mode == Mode::Publish
                        && self.dealer_ready()
                        && self.registration_retry.is_none()
                    {
                        self.want_put(PutStateReason::NewDevice);
                    }
                }
                () = wait_until(self.registration_retry) => self.refresh_roster().await,
                () = wait_until(put_at) => self.flush().await,
                () = wait_until(self.ack_at) => {
                    self.ack_at = None;
                    if self.now.is_some() {
                        self.want_put(PutStateReason::PlayerStateChanged);
                    }
                }
            }
            if self.soloist_ready
                && let Some((at, command)) = self.local_control.take()
                && (matches!(&command, Command::Start(start) if !start.state.playing)
                    || at.elapsed() < Duration::from_secs(30))
                && let Some(device) = self.soloist_device.as_deref()
            {
                control(&self.session, device, command).await;
            }
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
            Input::Control(device, command)
                if self.mode == Mode::Observe
                    && self.soloist_device.as_deref() == Some(&device) =>
            {
                self.control_soloist(command).await;
            }
            Input::Control(device, command) => control(&self.session, &device, command).await,
            Input::ControlLocal(command) if self.mode == Mode::Observe => {
                self.control_soloist(command).await;
            }
            Input::ControlLocal(_) => {}
            Input::Jump {
                device,
                track,
                forward,
            } => self.jump(&device, &track, forward).await,
        }
    }

    /// Keeps the user's transport intent while Soloist is absent from the device list.
    async fn control_soloist(&mut self, command: Command) {
        if matches!(command, Command::Start(_)) {
            self.paused_soloist = None;
        }
        if let Some(device) = self
            .soloist_device
            .as_deref()
            .filter(|_| self.soloist_ready)
        {
            control(&self.session, device, command).await;
            return;
        }
        match command {
            Command::Start(_) => self.local_control = Some((Instant::now(), command)),
            Command::Play | Command::Pause => {
                if let Some((at, Command::Start(start))) = &mut self.local_control {
                    *at = Instant::now();
                    Arc::make_mut(&mut start.state).playing = matches!(command, Command::Play);
                } else {
                    self.local_control =
                        matches!(command, Command::Play).then(|| (Instant::now(), Command::Play));
                }
            }
            Command::Seek(position) => {
                if let Some((_, Command::Start(start))) = &mut self.local_control {
                    Arc::make_mut(&mut start.state).position = position;
                }
            }
            _ => {}
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
        let active_here = self.active_since.is_some();
        let inbound = to == mine;
        if inbound && active_here {
            return;
        }
        let from = if inbound || active_here {
            mine.clone()
        } else if self.playback.device.is_empty() {
            to.clone()
        } else {
            self.playback.device.clone()
        };
        // The target starts from the last published state.
        let blocked = self
            .backoff
            .is_some_and(|(until, _)| until > Instant::now());
        let outbound = from == mine && !inbound && self.now.is_some();
        if outbound
            && (self.pending.is_some()
                || self
                    .confirmed_transfer_state
                    .is_none_or(|sent| sent.elapsed() > TRANSFER_STATE_AGE))
        {
            if !blocked {
                self.put(PutStateReason::PlayerStateChanged).await;
            }
            if blocked || self.confirmed_transfer_state.is_none() {
                log::warn!(
                    "connect: cannot move playback without confirmation of the current state"
                );
                return;
            }
        }
        // Keep Dealer responsive while Spotify waits for the transfer acknowledgement.
        if inbound {
            let session = self.session.clone();
            tokio::spawn(async move {
                if let Err(error) = move_playback(&session, &from, &to).await {
                    log::warn!("connect: cannot move playback here: {error}");
                }
            });
            return;
        }
        match move_playback(&self.session, &from, &to).await {
            Ok(()) if from == mine => {
                self.claimed_at = None;
                self.confirmed_transfer_state = None;
            }
            Ok(()) => {}
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
            self.now = None;
            self.uid_read = None;
            self.uids = Uids::default();
        }
        if previous != Mode::Off {
            self.withdraw().await;
        }
        self.rename_at = None;
        self.soloist_device = None;
        self.local_control = None;
        self.paused_soloist = None;
        // Changing roles requires a fresh Dealer connection after unregistering.
        self.disconnected();
        self.dealer = (mode != Mode::Off).then(|| Dealer::start(self.session.api().clone()));
    }

    /// Registers this Dealer connection for playback updates.
    async fn refresh_roster(&mut self) {
        self.registration_retry = None;
        if !self.dealer_ready() || self.mode == Mode::Off {
            return;
        }
        let observer = self.mode == Mode::Observe;
        let reason = PutStateReason::NewDevice;
        let cluster = if observer {
            let request = self.request_for(reason);
            put_state(&self.session, request)
                .await
                .inspect_err(|error| {
                    log::warn!("connect: cannot subscribe to playback updates: {error:#}")
                })
                .ok()
        } else {
            self.put(reason).await
        };
        let Some(cluster) = cluster else {
            self.pending = None;
            let now = Instant::now();
            self.registration_retry = Some(
                self.backoff
                    .map(|(until, _)| until)
                    .filter(|until| !observer && *until > now)
                    .unwrap_or(now + OBSERVER_RETRY),
            );
            return;
        };
        if observer {
            self.report_cluster(cluster).await;
        } else {
            self.emit_roster(cluster);
            self.begin_jam_read();
            if self.now.is_some() && self.active_since.is_none() {
                self.read_uids();
                self.sync(None).await;
            }
        }
    }

    fn dealer_ready(&self) -> bool {
        self.dealer.is_some() && !self.session.connection_id().is_empty()
    }

    /// Routes account notifications, playback updates, and Jam events.
    async fn dealer_message(&mut self, message: Message) -> anyhow::Result<()> {
        let uri = &message.uri;
        if dealer::topic(uri, CLUSTER_TOPIC) {
            let update: ClusterUpdate = message.decode_protobuf()?;
            if let Some(cluster) = update.cluster {
                self.report_cluster(cluster).await;
            }
        } else if dealer::topic(uri, VOLUME_TOPIC) {
            self.volume(&message.decode_protobuf()?);
        } else if dealer::topic(uri, COLLECTION_TOPIC) && message.has_protobuf() {
            for event in collection_changes(message.decode_protobuf()?) {
                self.out.send(event).ok();
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
                self.out
                    .send(Event::PlaylistChanged {
                        id: id.to_owned(),
                        revision: change
                            .new_revision
                            .filter(|value| !value.is_empty())
                            .map(|value| value.to_vec()),
                    })
                    .ok();
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
        self.cancel_play();
        self.registration_retry = None;
        self.ack_at = None;
        self.pending = None;
        self.transient_put_retries = 0;
        let was_ready = self.dealer_ready();
        self.session.set_connection_id("");
        self.playback = ClusterPlayback::default();
        self.soloist_ready = false;
        self.reset_jam();
        if was_ready {
            self.out
                .send(Event::Roster(Roster {
                    elsewhere: self.paused_soloist.clone(),
                    ..Default::default()
                }))
                .ok();
        }
        if self.held.take().is_some() {
            self.out.send(Event::Invite(None)).ok();
        }
    }

    /// Register only after Dealer supplies a connection ID.
    async fn connection_id(&mut self, id: String) {
        self.confirmed_transfer_state = None;
        self.reset_jam();
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

    async fn withdraw(&mut self) {
        self.confirmed_transfer_state = None;
        self.cancel_play();
        self.active_since = None;
        self.reset_jam();
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
        if let Some(now) = self.now.clone() {
            // smart shuffle is a kind of shuffle, and it holds for its own playlist alone
            if now.shuffle == Some(false) {
                self.smart_mode = false;
                self.smart = None;
                self.smart_read = None;
            } else if self.smart.is_some() && self.playing(&now) != self.smart {
                self.smart = None;
            }
        }
        self.identify(before.as_deref());
        if self.mode == Mode::Publish {
            self.read_uids();
        }
        self.sync(before).await;
    }

    /// Fetches occurrence IDs once per playback context.
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
    fn learn(&mut self, tracks: anyhow::Result<ResolvedSequence>) {
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

    /// Renew play IDs on track changes and run IDs on context changes or standalone track changes.
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

    /// Publishes playback changes relative to the previous snapshot.
    async fn sync(&mut self, before: Option<Arc<Snapshot>>) {
        if self.mode != Mode::Publish || !self.dealer_ready() {
            return;
        }
        let Some(now) = &self.now else {
            if self.active_since.take().is_some()
                && let Err(error) = {
                    let connection = self.session.connection_id();
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
        self.want_put(
            if before.is_some_and(|before| now.change(&before) == Some("volume")) {
                PutStateReason::VolumeChanged
            } else {
                PutStateReason::PlayerStateChanged
            },
        );
    }

    /// Coalesces updates without losing the most important pending reason.
    fn want_put(&mut self, reason: PutStateReason) {
        self.pending = Some(
            self.pending
                .filter(|held| rank(*held) > rank(reason))
                .unwrap_or(reason),
        );
    }

    /// A pending update waits for both the ordinary interval and any active backoff.
    fn put_at(&self) -> Option<Instant> {
        self.pending.filter(|_| self.registration_retry.is_none())?;
        let at = self
            .last_put
            .map_or_else(Instant::now, |sent| sent + PUT_GAP);
        Some(self.backoff.map_or(at, |(limit, _)| at.max(limit)))
    }

    async fn flush(&mut self) {
        let Some(reason) = self.pending.take() else {
            return;
        };
        if self.mode != Mode::Publish || !self.dealer_ready() {
            return;
        }
        if matches!(reason, PutStateReason::NewDevice) {
            if let Some(cluster) = self.put(reason).await {
                self.report_cluster(cluster).await;
            }
        } else if self.now.is_some() {
            self.put(reason).await;
        }
    }

    /// Publishes device state and returns the account cluster.
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
                self.backoff = None;
                self.transient_put_retries = 0;
                self.observe_clock(&cluster);
                Some(cluster)
            }
            Err(error)
                if spotifyapis::http_status(&error) == Some(StatusCode::TOO_MANY_REQUESTS) =>
            {
                let delay = self.retry_put(reason);
                log::warn!(
                    "connect: Spotify asked for fewer state updates, waiting {}s",
                    delay.as_secs()
                );
                None
            }
            Err(error)
                if self.transient_put_retries < TRANSIENT_PUT_RETRIES
                    && spotifyapis::transient_http(&error) =>
            {
                self.transient_put_retries += 1;
                let delay = self.retry_put(reason);
                log::warn!(
                    "connect: cannot put the state, retrying in {}s: {error:#}",
                    delay.as_secs()
                );
                None
            }
            Err(error) => {
                self.transient_put_retries = 0;
                self.backoff = None;
                log::warn!("connect: cannot put the state: {error}");
                None
            }
        }
    }

    /// Keeps a failed publication scheduled and increases the retry delay up to the cap.
    fn retry_put(&mut self, reason: PutStateReason) -> Duration {
        let delay = self
            .backoff
            .map_or(BACKOFF_FIRST, |(_, delay)| (delay * 2).min(BACKOFF_LAST));
        self.backoff = Some((Instant::now() + delay, delay));
        self.want_put(reason);
        delay
    }

    fn request_for(&self, reason: PutStateReason) -> PutStateRequest {
        let stamp = unix_millis();
        let session = &self.session;
        let observer = self.mode == Mode::Observe;
        let name = if observer { DEVICE_NAME } else { &self.name };
        let mut info = device_info(session, name, observer);
        let mut request = PutStateRequest {
            member_type: CONNECT_STATE_MEMBER,
            put_state_reason: reason as i32,
            client_side_timestamp_ms: stamp,
            ..Default::default()
        };
        let player = if observer {
            initial_player_state(session)
        } else {
            if let Some(now) = &self.now {
                info.volume = volume_word(now.volume).into();
            }
            request.is_active = self.active_since.is_some() && self.now.is_some();
            request.started_playing_at = self.active_since.map(millis).unwrap_or_default();
            request.last_command_message_id = self.last_command.0;
            request
                .last_command_sent_by_device_id
                .clone_from(&self.last_command.1);
            self.player_state(stamp)
        };
        request.device = Some(WireDevice {
            device_info: Some(info),
            player_state: Some(player),
        });
        request
    }

    /// The published context URI and whether the current song is its own context.
    fn playback_context(&self, now: &Snapshot, track: &QueuedTrack) -> (String, bool) {
        match shared(now) {
            Some(context) => (collection_uri(context, self.session.username()), false),
            None => (track_uri(&track.id), true),
        }
    }

    fn player_state(&self, stamp: u64) -> PlayerState {
        let mut state = PlayerState {
            is_system_initiated: true,
            play_origin: Some(Default::default()),
            context_uri: UNKNOWN_CONTEXT.to_owned(),
            context_url: format!("context://{UNKNOWN_CONTEXT}"),
            ..initial_player_state(&self.session)
        };
        let Some(now) = &self.now else { return state };
        let Some(track) = &now.track else {
            return state;
        };

        let (uri, standalone) = self.playback_context(now, track);
        let entry = |track: &QueuedTrack, index| {
            let uid = self.uids.at(&uri, track, index, standalone);
            let mut provided = ProvidedTrack::for_queue(
                &track.id,
                uid,
                &uri,
                track.queued,
                track.suggested,
                track.added_by.as_deref(),
            );
            if track.recommended && !track.queued {
                RECOMMENDED.clone_into(&mut provided.provider);
            }
            provided
        };
        state.track = Some(entry(track, now.index));
        let queue =
            |tracks: &[QueuedTrack]| tracks.iter().map(|track| entry(track, None)).collect();
        state.prev_tracks = queue(&now.previous);
        state.next_tracks = queue(&now.upcoming);
        state.context_url = match self.smart.as_deref() == Some(uri.as_str()) {
            true => format!("context://{uri}{LENS}"),
            false => format!("context://{uri}"),
        };
        state.context_uri = uri;
        if !self.run.is_empty() {
            state.session_id = self.run.clone();
        }
        state.playback_id = self.play.clone();
        // Send the context index so transfers can distinguish duplicate tracks.
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
            modes: self
                .smart_mode
                .then(|| (ENHANCEMENT.to_owned(), RECOMMENDATION.to_owned()))
                .into_iter()
                .collect(),
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

        // Spotify expects paused players to report both playing and buffering.
        state.is_playing = true;
        state.is_paused = !now.playing;
        state.is_buffering = !now.playing;
        state.playback_speed = if now.playing { 1. } else { 0. };
        state
    }

    async fn report_cluster(&mut self, cluster: Cluster) {
        if self.mode == Mode::Off {
            return;
        }
        if self.mode == Mode::Observe && self.soloist_device.is_none() {
            self.soloist_device = tokio::task::spawn_blocking(soloist::device_id)
                .await
                .ok()
                .flatten();
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
        let mut roster = self.roster(&cluster);
        self.soloist_ready = roster.local.is_some();
        self.playback.player = cluster.player_state.filter(|_| roster.elsewhere.is_some());
        if cluster.active_device_id != self.session.device_id() {
            self.playback.queue = roster
                .elsewhere
                .as_ref()
                .filter(|_| self.playback.player.is_some())
                .map(|remote| TransferQueue {
                    snapshot: remote.state.clone(),
                    received: Instant::now(),
                });
        }
        if self.mode == Mode::Observe {
            self.keep_paused_soloist(&mut roster, &cluster.active_device_id);
        }
        self.playback.device = cluster.active_device_id;
        self.out.send(Event::Roster(roster)).ok();
    }

    /// Spotify can clear its active device during a pause. Keep the last paused queue visible.
    fn keep_paused_soloist(&mut self, roster: &mut Roster, active: &str) {
        let local = self.soloist_device.as_deref();
        if !active.is_empty() && local != Some(active) {
            self.paused_soloist = None;
            if matches!(self.local_control, Some((_, Command::Play))) {
                self.local_control = None;
            }
        } else if let Some(remote) = roster.elsewhere.as_ref()
            && remote.state.track.is_some()
        {
            self.paused_soloist = (!remote.state.playing).then(|| remote.clone());
        } else if self.paused_soloist.is_some() {
            roster.elsewhere.clone_from(&self.paused_soloist);
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
        let soloist = self.soloist_device.as_deref() == Some(device);
        let Some((_, target)) =
            queue_target(player, Occurrence::from_queue(track), forward, soloist)
        else {
            return;
        };
        let connection = self.session.connection_id();
        let device =
            self.session
                .api()
                .connect_device(self.session.device_id(), device, &connection);
        let result = if forward {
            device.skip_to(target).await
        } else if target.is_queued() {
            return;
        } else {
            play_history(&device, player, target).await
        };
        if let Err(error) = result.map_err(sign_in_error) {
            log::warn!("connect: cannot jump in the remote queue: {error}");
        }
    }

    /// Aligns transfer and position timestamps with Spotify's clock.
    fn observe_clock(&mut self, cluster: &Cluster) {
        if cluster.server_timestamp_ms > 0 {
            self.clock_offset_ms = cluster
                .server_timestamp_ms
                .saturating_sub(unix_millis() as i64);
        }
    }

    /// Uses a recent player report to recover queue order omitted by a transfer. A transfer
    /// also carries whether smart shuffle is on, and the collection it is on for.
    fn transferred(&mut self, state: TransferState) -> Start {
        self.smart_read = None;
        self.smart_mode = state
            .options
            .as_ref()
            .is_some_and(|options| recommends(&options.modes));
        self.smart = state
            .current_session
            .as_ref()
            .and_then(|session| session.context.as_ref())
            .and_then(|context| context.uri.clone())
            .filter(|uri| self.smart_mode && uri.starts_with(PLAYLIST_PREFIX));
        let now = (unix_millis() as i64).saturating_add(self.clock_offset_ms);
        let recent = self
            .playback
            .queue
            .as_ref()
            .filter(|queue| queue.received.elapsed() <= QUEUE_SNAPSHOT_AGE)
            .map(|queue| queue.snapshot.as_ref());
        started_by_transfer(state, now, recent)
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
        let soloist = is_soloist(active);
        let elsewhere = cluster
            .device
            .get(&cluster.active_device_id)
            .filter(|_| active != mine)
            .map(|info| {
                let empty = PlayerState::default();
                let state = cluster.player_state.as_ref().unwrap_or(&empty);
                let options = state.options.as_ref();
                let millis = |value: i64| Duration::from_millis(value.max(0) as u64);
                Elsewhere {
                    device: known(&cluster.active_device_id, info),
                    state: Arc::new(Snapshot {
                        track: state
                            .track
                            .as_ref()
                            .and_then(|track| listed_track(track, soloist)),
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
                        index: state
                            .index
                            .as_ref()
                            .filter(|index| index.page == 0)
                            .map(|index| index.track as usize),
                        previous: listed(&state.prev_tracks, state.prev_tracks.len(), soloist),
                        upcoming: listed(&state.next_tracks, CARRIED, soloist),
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
            Wire::Play(play) => return self.play(*play, reply),
            command => self.translate(command),
        };
        reply.send(commands.is_some());
        for command in commands.into_iter().flatten() {
            self.out.send(Event::Command(command)).ok();
        }
        self.ack_at = Some(Instant::now() + ACK_DELAY);
    }

    /// Rejects a pending Play when its context read is superseded or the connection closes.
    fn cancel_play(&mut self) {
        if let Some((_, reply)) = self.play_read.take() {
            reply.send(false);
        }
    }

    fn finish_play(&mut self, start: Option<Start>, reply: Reply) {
        let delivered =
            start.is_some_and(|start| self.out.send(Event::Command(Command::Start(start))).is_ok());
        reply.send(delivered);
        self.ack_at = Some(Instant::now() + ACK_DELAY);
    }

    /// Starts from supplied rows when possible, resolving the context for missing occurrences.
    /// A playlist played through smart shuffle is read through its lens instead, which mixes the
    /// recommendations in.
    fn play(&mut self, mut play: PlayCommand, reply: Reply) {
        self.cancel_play();
        self.smart_read = None;
        if let Some(on) = asked_mode(&play) {
            self.smart_mode = on;
        }
        let rows = ResolvedSequence::from_context_pages(std::mem::take(&mut play.context.pages));
        let supplied = started_by_play(&play, &rows);
        let enhanced = play
            .context
            .uri
            .clone()
            .filter(|uri| self.smart_mode && uri.starts_with(PLAYLIST_PREFIX));
        if let Some(uri) = enhanced {
            let session = self.session.clone();
            let read = async move {
                match sequence::resolve(&session, &format!("{uri}{LENS}")).await {
                    Ok(rows) => (started_smart(&play, &rows), Some(uri)),
                    Err(error) => {
                        log::warn!("connect: cannot read {uri} for smart shuffle: {error:#}");
                        (supplied, None)
                    }
                }
            };
            self.play_read = Some((Box::pin(read), reply));
            return;
        }
        self.smart = None;
        let occurrence = play.options.skip_to.as_ref().is_some_and(|skip| {
            skip.track_index.is_some()
                || skip.track_uid.as_deref().is_some_and(|uid| !uid.is_empty())
        });
        let Some(uri) = play
            .context
            .uri
            .as_deref()
            .filter(|uri| {
                !uri.is_empty()
                    && uri_id(uri).is_none()
                    && ((occurrence && supplied.is_none())
                        || (rows.tracks.is_empty() && collection(uri).is_none()))
            })
            .map(str::to_owned)
        else {
            return self.finish_play(supplied, reply);
        };
        let session = self.session.clone();
        self.play_read = Some((
            Box::pin(async move {
                let start = match sequence::resolve(&session, &uri)
                    .await
                    .inspect_err(|error| log::warn!("connect: cannot read {uri} back: {error}"))
                {
                    Ok(rows) => started_by_play(&play, &rows),
                    Err(_) => supplied,
                };
                (start, None)
            }),
            reply,
        ));
    }

    /// Maps Spotify's published occurrence back to Sonora's local queue identity.
    fn select_queue(&self, target: &ProvidedTrack, forward: bool) -> Option<Command> {
        let now = self.now.as_ref()?;
        let current = now.track.as_ref()?;
        let (context, standalone) = self.playback_context(now, current);
        let target = Occurrence::from_wire(target, false);
        let matches = |track: &QueuedTrack, index| {
            target.matches_id(Some(track.id.as_bytes()))
                && target
                    .uid
                    .filter(|uid| !uid.is_empty())
                    .is_none_or(|uid| self.uids.at(&context, track, index, standalone) == uid)
        };
        if matches(current, now.index) {
            return None;
        }
        let local = if forward {
            &now.upcoming
        } else {
            &now.previous
        };
        let index =
            target.resolve_matches(local.iter().map(|track| matches(track, None)), None, true)?;
        let uid = &local.get(index)?.uid;
        (!uid.is_empty()).then(|| Command::SelectQueue(uid.clone()))
    }

    /// Translates supported Connect commands into local playback actions.
    fn translate(&mut self, command: Wire) -> Option<Vec<Command>> {
        Some(match command {
            Wire::Transfer { data } => {
                let state = TransferState::from_command_data(&data).ok()?;
                self.cancel_play();
                vec![Command::Start(self.transferred(state))]
            }
            Wire::Play(_) | Wire::Unknown => return None,
            Wire::Pause => {
                self.cancel_play();
                vec![Command::Pause]
            }
            Wire::Resume => vec![Command::Play],
            Wire::SkipNext { track: Some(track) } => vec![self.select_queue(&track, true)?],
            Wire::SkipPrev { track: Some(track) } => vec![self.select_queue(&track, false)?],
            Wire::SkipNext { track: None } => vec![Command::Next],
            Wire::SkipPrev { track: None } => vec![Command::Previous],
            Wire::SeekTo { value } => vec![Command::Seek(Duration::from_millis(value))],
            Wire::SetShufflingContext { value } => {
                // smart shuffle is a kind of shuffle, so shuffle off switches it off too
                let off = (!value).then(|| self.switch_smart(false)).flatten();
                off.into_iter().chain([Command::Shuffle(value)]).collect()
            }
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
                modes,
            } => {
                // smart shuffle is a kind of shuffle, so shuffle off switches it off too
                let unshuffled = shuffling_context == Some(false);
                let smart = modes
                    .as_ref()
                    .map(|modes| recommends(modes) && !unshuffled)
                    .or_else(|| unshuffled.then_some(false))
                    .and_then(|on| self.switch_smart(on));
                let repeat = (repeating_context.is_some() || repeating_track.is_some())
                    .then(|| Command::Repeat(self.repeat(repeating_context, repeating_track)));
                smart
                    .into_iter()
                    .chain(shuffling_context.map(Command::Shuffle))
                    .chain(repeat)
                    .collect()
            }
            Wire::AddToQueue { track } => track_id(&track.uri)
                .map(|id| Command::Enqueue {
                    id,
                    added_by: track.added_by().map(str::to_owned),
                })
                .into_iter()
                .collect(),
            Wire::SetQueue {
                next_tracks: tracks,
            } => {
                // Keep the entire edited queue, including entries beyond the published window.
                let mut upcoming = listed(&tracks, tracks.len(), false);
                if let Some(now) = self.now.as_deref().filter(|now| shared(now).is_none())
                    && let Some(current) = &now.track
                {
                    let (context, standalone) = self.playback_context(now, current);
                    keep_kinds(&mut upcoming, &now.upcoming, |track| {
                        self.uids.at(&context, track, None, standalone)
                    });
                }
                // Acknowledge the new queue before metadata loads and keep local playback history.
                if let Some(now) = self.now.as_mut().filter(|now| now.upcoming != upcoming) {
                    Arc::make_mut(now).upcoming.clone_from(&upcoming);
                    self.confirmed_transfer_state = None;
                    self.want_put(PutStateReason::PlayerStateChanged);
                }
                vec![Command::SetQueue { upcoming }]
            }
            Wire::UpdateContext { context } => {
                let playing = self.now.as_deref().and_then(|now| self.playing(now));
                context
                    .filter(|context| context.uri.is_some() && context.uri == playing)
                    .and_then(|context| {
                        let lens = context.url.is_some_and(|url| url.ends_with(LENS));
                        self.switch_smart(lens)
                    })
                    .into_iter()
                    .collect()
            }
        })
    }

    /// The uri of the collection that plays now, as Spotify names it.
    fn playing(&self, now: &Snapshot) -> Option<String> {
        shared(now).map(|context| collection_uri(context, self.session.username()))
    }

    /// Switches smart shuffle for this device and for the playlist that plays now. Switching it
    /// on reads the playlist through the lens, and the read replaces what comes next once it
    /// lands. Switching it off answers the queue without the recommendations.
    fn switch_smart(&mut self, on: bool) -> Option<Command> {
        self.smart_mode = on;
        let now = self.now.clone()?;
        let uri = self
            .playing(&now)
            .filter(|uri| uri.starts_with(PLAYLIST_PREFIX))?;
        if on {
            if self.smart.as_deref() != Some(uri.as_str()) && self.smart_read.is_none() {
                let session = self.session.clone();
                self.smart_read = Some(Box::pin(async move {
                    let read = sequence::resolve(&session, &format!("{uri}{LENS}")).await;
                    (uri, read)
                }));
            }
            return None;
        }
        self.smart_read = None;
        self.smart.take()?;
        log::debug!("connect: smart shuffle off");
        let upcoming = now
            .upcoming
            .iter()
            .filter(|track| track.queued || !track.recommended)
            .cloned()
            .collect();
        Some(Command::SetQueue { upcoming })
    }

    /// Mixes a playlist read through the smart shuffle lens into what comes next, keeping the
    /// songs queued by hand first. A read for a playlist that no longer plays, or that smart
    /// shuffle was switched off for meanwhile, is dropped.
    fn enhanced(&mut self, uri: String, read: anyhow::Result<ResolvedSequence>) {
        let Some(now) = self.now.clone() else {
            return;
        };
        if !self.smart_mode || self.playing(&now).as_deref() != Some(uri.as_str()) {
            return;
        }
        let rows = match read {
            Ok(rows) => rows,
            Err(error) => {
                return log::warn!("connect: cannot read {uri} for smart shuffle: {error:#}");
            }
        };
        let current = now.track.as_ref().map(|track| track.id.as_str());
        let mut following: Vec<QueuedTrack> = rows
            .tracks
            .iter()
            .filter_map(|track| context_entry(track, false))
            .filter(|track| Some(track.id.as_str()) != current)
            .collect();
        fastrand::shuffle(&mut following);
        let upcoming = now
            .upcoming
            .iter()
            .filter(|track| track.queued)
            .cloned()
            .chain(following)
            .collect();
        self.smart = Some(uri);
        // a handover replaces the whole queue, where a queue edit keeps the old order past its
        // window and plays songs from the window a second time
        let start = Start {
            state: Arc::new(Snapshot {
                upcoming,
                shuffle: Some(true),
                ..(*now).clone()
            }),
            preserve_queue: true,
            restart: true,
        };
        self.out.send(Event::Command(Command::Start(start))).ok();
    }

    /// Updates one repeat flag while preserving the other.
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

fn initial_player_state(session: &Session) -> PlayerState {
    PlayerState {
        session_id: session.session_id().to_owned(),
        playback_speed: 1.,
        options: Some(ContextPlayerOptions::default()),
        suppressions: Some(Suppressions::default()),
        ..Default::default()
    }
}

/// Sends a Connect state update and decodes the account cluster returned by Spotify.
async fn put_state(session: &Session, request: PutStateRequest) -> anyhow::Result<Cluster> {
    let connection = session.connection_id();
    session
        .api()
        .put_device_state(session.device_id(), &connection, &request)
        .await
        .map_err(sign_in_error)
}

/// Asks Spotify to move playback from one device to another, restoring a paused state.
async fn move_playback(session: &Session, from: &str, to: &str) -> anyhow::Result<()> {
    let connection = session.connection_id();
    session
        .api()
        .connect_device(from, to, &connection)
        .transfer()
        .await
        .map_err(sign_in_error)
}

/// Sends a control to the remote device without changing the local queue.
async fn control(session: &Session, device: &str, command: Command) {
    let connection = session.connection_id();
    let remote = session
        .api()
        .connect_device(session.device_id(), device, &connection);
    let result = match command {
        Command::Play => remote.resume().await,
        Command::Pause => remote.pause().await,
        Command::Next => remote.next().await,
        Command::Previous => remote.previous().await,
        Command::Seek(at) => remote.seek(at).await,
        Command::Shuffle(on) => remote.shuffle(on).await,
        Command::Repeat(mode) => {
            remote
                .repeat(mode != Repeat::Off, mode == Repeat::One)
                .await
        }
        Command::Enqueue { id, .. } => remote.enqueue(&track_uri(&id)).await,
        Command::Start(start) => play_from(&remote, &start, session.username()).await,
        Command::Volume(level) => remote.volume(volume_word(level)).await,
        _ => return,
    };
    if let Err(error) = result.map_err(sign_in_error) {
        log::warn!("connect: cannot control {device}: {error}");
    }
}

/// Prioritizes registration and playback changes over routine updates.
fn rank(reason: PutStateReason) -> u8 {
    match reason {
        PutStateReason::NewDevice => 3,
        PutStateReason::PlayerStateChanged => 2,
        _ => 1,
    }
}

/// Waits for an optional deadline, leaving an unscheduled timer dormant.
async fn wait_until(at: Option<Instant>) {
    when_ready(at.map(tokio::time::sleep_until)).await
}

async fn when_ready<T>(future: Option<impl Future<Output = T>>) -> T {
    match future {
        Some(future) => future.await,
        None => std::future::pending().await,
    }
}

/// Display name shown in the Connect device list.
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
        supported_types: vec!["audio/track".into()],
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
            supports_smart_shuffle_mode: true,
            supported_audio_quality: VERY_HIGH_AUDIO_QUALITY,
            command_acks: true,
            ..capabilities
        };
    }
    info.capabilities = Some(capabilities);
    info
}

/// Starts playback on another device from Sonora’s selected context.
async fn play_from(
    device: &ConnectDevice<'_>,
    start: &Start,
    username: &str,
) -> anyhow::Result<()> {
    let start = &start.state;
    let track = start.track.as_ref().map(|track| track_uri(&track.id));
    let Some(context) = start
        .context
        .as_ref()
        .map(|context| collection_uri(context, username))
        .or_else(|| track.clone())
    else {
        anyhow::bail!("Spotify play command has no context or track");
    };
    device
        .play(Play {
            context_uri: &context,
            context_url: &format!("context://{context}"),
            target: track
                .as_deref()
                .zip(start.track.as_ref().map(|track| track.uid.as_str())),
            position: start.position,
            paused: !start.playing,
            shuffle: start.shuffle,
            repeat_context: start.repeat.map(|repeat| repeat == Repeat::All),
            repeat_track: start.repeat.map(|repeat| repeat == Repeat::One),
            origin: ("sonora", SEMVER),
        })
        .await
}

/// Starts a history entry explicitly because Previous can restart the current song.
async fn play_history(
    device: &ConnectDevice<'_>,
    player: &PlayerState,
    target: &ProvidedTrack,
) -> anyhow::Result<()> {
    let context = target
        .metadata
        .get("context_uri")
        .map(String::as_str)
        .filter(|uri| !uri.is_empty() && *uri != UNKNOWN_CONTEXT && !uri.starts_with(TRACK_PREFIX))
        .or_else(|| {
            let uri = player.context_uri.as_str();
            (!uri.is_empty() && uri != UNKNOWN_CONTEXT && !uri.starts_with(TRACK_PREFIX))
                .then_some(uri)
        });
    let context = context
        .filter(|_| !target.uid.is_empty())
        .context("Spotify did not identify the history occurrence in a collection")?;
    let url = if context == player.context_uri && !player.context_url.is_empty() {
        player.context_url.clone()
    } else {
        format!("context://{context}")
    };
    let options = player.options.as_ref();
    device
        .play(Play {
            context_uri: context,
            context_url: &url,
            target: Some((&target.uri, &target.uid)),
            position: Duration::ZERO,
            paused: false,
            shuffle: options.map(|options| options.shuffling_context),
            repeat_context: options.map(|options| options.repeating_context),
            repeat_track: options.map(|options| options.repeating_track),
            origin: ("sonora", SEMVER),
        })
        .await
}

/// Maps optional Spotify repeat flags to a repeat mode.
fn repeat_mode(track: Option<bool>, context: Option<bool>) -> Option<Repeat> {
    match (track, context) {
        (Some(true), _) => Some(Repeat::One),
        (_, Some(true)) => Some(Repeat::All),
        (Some(false), _) | (_, Some(false)) => Some(Repeat::Off),
        _ => None,
    }
}

/// Share stable contexts and publish radio as an explicit queue.
fn shared(now: &Snapshot) -> Option<&Collection> {
    now.context.as_ref().filter(|context| {
        !matches!(
            context,
            Collection::Radio(_) | Collection::CollectionRadio(..)
        )
    })
}

/// Builds the Spotify URI for a playback context.
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

impl<'a> Occurrence<'a> {
    fn from_wire(track: &'a ProvidedTrack, soloist: bool) -> Self {
        Self {
            id: reported_track_id(track, soloist).map(CatalogueId::Borrowed),
            uid: Some(&track.uid),
        }
    }

    fn from_context(track: &'a ContextTrack) -> Self {
        Self {
            id: track.track_id(),
            uid: track.uid.as_deref(),
        }
    }

    fn from_queue(track: &'a QueuedTrack) -> Self {
        Self {
            id: Some(CatalogueId::Borrowed(&track.id)),
            uid: Some(&track.uid),
        }
    }

    /// Prefer position, then UID, and require a complete list for track-ID uniqueness.
    fn resolve(
        &self,
        entries: impl IntoIterator<Item = Self>,
        position: Option<usize>,
        complete: bool,
    ) -> Option<usize> {
        self.resolve_matches(
            entries.into_iter().map(|entry| {
                self.matches_id(entry.id.as_ref().map(CatalogueId::as_bytes))
                    && self
                        .uid
                        .filter(|uid| !uid.is_empty())
                        .is_none_or(|uid| entry.uid == Some(uid))
            }),
            position,
            complete,
        )
    }

    fn matches_id(&self, id: Option<&[u8]>) -> bool {
        self.id
            .as_ref()
            .is_none_or(|key| id == Some(key.as_bytes()))
    }

    fn resolve_matches(
        &self,
        entries: impl Iterator<Item = bool>,
        position: Option<usize>,
        complete: bool,
    ) -> Option<usize> {
        let mut matches = entries
            .enumerate()
            .filter_map(|(index, hit)| hit.then_some(index));
        let index = matches.next()?;
        if Some(index) == position {
            return Some(index);
        }
        let duplicate = matches.next();
        if duplicate.is_none()
            && (self.uid.is_some_and(|uid| !uid.is_empty()) || (complete && self.id.is_some()))
        {
            return Some(index);
        }
        position.filter(|&position| {
            duplicate == Some(position) || matches.any(|index| index == position)
        })
    }
}

/// Resolves a directional queue target without mistaking the current track for a duplicate.
fn queue_target<'a>(
    state: &'a PlayerState,
    target: Occurrence<'_>,
    forward: bool,
    soloist: bool,
) -> Option<(usize, &'a ProvidedTrack)> {
    let entries = if forward {
        &state.next_tracks
    } else {
        &state.prev_tracks
    };
    let occurrence = |track| Occurrence::from_wire(track, soloist);
    if target
        .resolve(state.track.iter().map(occurrence), None, true)
        .is_some()
    {
        return None;
    }
    let index = target.resolve(entries.iter().map(occurrence), None, true)?;
    Some((index, &entries[index]))
}

/// The occurrence IDs Spotify supplies for the currently published context.
#[derive(Default)]
struct Uids {
    context: String,
    tracks: Option<ResolvedSequence>,
}

impl Uids {
    /// Keeps local occurrence IDs for a standalone queue and Spotify IDs for context rows.
    fn at(
        &self,
        context: &str,
        track: &QueuedTrack,
        index: Option<usize>,
        standalone: bool,
    ) -> String {
        if track.queued || (standalone && !track.uid.is_empty()) {
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
                sequence_position(rows, key, index).map(|index| &rows.tracks[index])
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

/// Derives a missing UID from the track URI's SHA-1 prefix.
fn derived_uid(id: &str) -> String {
    let mut hash = Sha1::new();
    hash.update(TRACK_PREFIX);
    hash.update(id);
    let digest = hash.finalize();
    let mut uid = String::with_capacity(DERIVED_UID);
    wire::append_hex(&mut uid, &digest[..DERIVED_UID / 2]);
    uid
}

/// Restores kinds from the published occurrence IDs, then from unambiguous track IDs.
fn keep_kinds(
    upcoming: &mut [QueuedTrack],
    before: &[QueuedTrack],
    published_uid: impl Fn(&QueuedTrack) -> String,
) {
    let mut pool: Vec<_> = before
        .iter()
        .map(|track| (published_uid(track), track))
        .collect();
    for prefer_uid in [true, false] {
        for track in upcoming
            .iter_mut()
            .filter(|track| track.uid.is_empty() != prefer_uid)
        {
            let mut matches = pool.iter().enumerate().filter(|(_, (uid, known))| {
                known.id == track.id && (!prefer_uid || *uid == track.uid)
            });
            if let Some((index, _)) = matches.next()
                && (prefer_uid || matches.next().is_none())
            {
                let (_, known) = pool.remove(index);
                if track.queued {
                    track.queued = known.queued;
                    track.suggested = known.suggested;
                }
            }
        }
    }
}

fn track_id(uri: &str) -> Option<String> {
    uri_id(uri).map(str::to_owned)
}

/// Reads queue entries with contributor attribution when available.
fn listed(tracks: &[ProvidedTrack], limit: usize, soloist: bool) -> Vec<QueuedTrack> {
    tracks
        .iter()
        .filter_map(|track| listed_track(track, soloist))
        .take(limit)
        .collect()
}

// Soloist queue entries can carry the URI of the original play command as requested_uri.
fn reported_track_id(track: &ProvidedTrack, soloist: bool) -> Option<&str> {
    if soloist {
        uri_id(&track.uri)
    } else {
        track.track_id()
    }
}

/// Retains occurrence provenance alongside the catalogue ID used for display.
fn listed_track(track: &ProvidedTrack, soloist: bool) -> Option<QueuedTrack> {
    Some(QueuedTrack {
        id: reported_track_id(track, soloist)?.to_owned(),
        uid: track.uid.clone(),
        queued: track.is_queued(),
        suggested: track.provider == "autoplay",
        added_by: track.added_by().map(str::to_owned),
        recommended: track.provider == RECOMMENDED,
    })
}

/// Retains Spotify's occurrence UID when reconstructing a local queue from context rows.
fn context_entry(track: &ContextTrack, queued: bool) -> Option<QueuedTrack> {
    let mut entry = QueuedTrack::new(track.track_id()?.into_owned(), queued);
    entry.uid = track.uid.clone().unwrap_or_default();
    entry.recommended =
        !queued && track.metadata.get("provider").map(String::as_str) == Some(RECOMMENDED);
    Some(entry)
}

/// Whether a set of player modes has smart shuffle on.
fn recommends(modes: &HashMap<String, String>) -> bool {
    modes.get(ENHANCEMENT).map(String::as_str) == Some(RECOMMENDATION)
}

/// What a play command asks of smart shuffle: on when it names the lens or the mode, off when
/// it plays unshuffled or names the mode off, and `None` when it says nothing either way.
fn asked_mode(play: &PlayCommand) -> Option<bool> {
    let lens = play
        .context
        .url
        .as_deref()
        .is_some_and(|url| url.ends_with(LENS));
    let options = play.options.player_options_override.as_ref();
    let modes = options
        .map(|options| &options.modes)
        .filter(|modes| !modes.is_empty());
    // smart shuffle is a kind of shuffle, so a play asked unshuffled leaves it off
    let unshuffled = options.and_then(|options| options.shuffling_context) == Some(false);
    match (lens, modes) {
        _ if unshuffled => Some(false),
        (true, _) => Some(true),
        (false, modes) => modes.map(recommends),
    }
}

/// Extracts album, playlist, or saved-track contexts.
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

fn sequence_position(
    rows: &ResolvedSequence,
    key: Occurrence<'_>,
    index: Option<usize>,
) -> Option<usize> {
    key.resolve(
        rows.tracks.iter().map(Occurrence::from_context),
        index,
        rows.complete,
    )
}

fn following(
    rows: &ResolvedSequence,
    index: Option<usize>,
) -> impl Iterator<Item = QueuedTrack> + '_ {
    index
        .into_iter()
        .flat_map(|index| &rows.tracks[index + 1..])
        .filter_map(|track| context_entry(track, false))
}

/// Reuses remote queue order only if its current occurrence matches the transfer.
fn same_transfer_track(track: &QueuedTrack, remote: &Snapshot) -> bool {
    let entries = remote.track.iter().chain(
        remote
            .previous
            .iter()
            .chain(&remote.upcoming)
            .filter(|_| track.uid.is_empty()),
    );
    remote.track.is_some()
        && Occurrence::from_queue(track).resolve(entries.map(Occurrence::from_queue), None, true)
            == Some(0)
}

/// Reconstructs the transferred queue and overlays the authoritative playback clock and options.
fn started_by_transfer(state: TransferState, now: i64, remote: Option<&Snapshot>) -> Start {
    let playback = state.playback.unwrap_or_default();
    let queue = state.queue.unwrap_or_default();
    let options = state.options.unwrap_or_default();

    let from_queue = queue.is_playing_queue.unwrap_or_default();
    let current = match from_queue {
        true => queue.tracks.first(),
        false => playback.current_track.as_ref(),
    };
    let context = state
        .current_session
        .and_then(|session| session.context)
        .unwrap_or_default();
    let origin = context.uri.as_deref().and_then(collection);
    let track = current.and_then(|track| context_entry(track, from_queue));
    let rows = ResolvedSequence::from_context_pages(context.pages);
    let place = playback
        .current_track
        .as_ref()
        .and_then(|track| sequence_position(&rows, Occurrence::from_context(track), None));
    let queued = queue
        .tracks
        .iter()
        .skip(usize::from(from_queue))
        .filter_map(|track| context_entry(track, true));
    let mut snapshot = if let Some(remote) = remote.filter(|remote| {
        origin
            .as_ref()
            .is_none_or(|origin| remote.context.as_ref() == Some(origin))
            && track
                .as_ref()
                .is_some_and(|track| same_transfer_track(track, remote))
    }) {
        let mut snapshot = remote.clone();
        snapshot.index = snapshot.index.or(place);
        // The transfer owns the explicit queue. Only the context order comes from the report.
        snapshot.upcoming = queued
            .chain(snapshot.upcoming.into_iter().filter(|track| !track.queued))
            .take(CARRIED)
            .collect();
        snapshot
    } else {
        let mut previous: Vec<_> = state
            .play_history
            .iter()
            .flat_map(|history| &history.backward_items)
            .filter_map(|item| {
                let key = Occurrence {
                    uid: item.uid.as_deref(),
                    ..Default::default()
                };
                let index = sequence_position(&rows, key, None)?;
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
        Snapshot {
            context: origin,
            track,
            index: place,
            previous,
            upcoming: queued
                .chain(following(&rows, place))
                .take(CARRIED)
                .collect(),
            playing: true,
            ..Default::default()
        }
    };
    let playing = playback
        .is_paused
        .map_or(snapshot.playing, |paused| !paused);
    snapshot.position = playback.position_at_timestamp_ms.map_or_else(
        || snapshot.live_position(),
        |position| {
            let elapsed = playback
                .timestamp_ms
                .filter(|_| playing)
                .map_or(0, |stamp| now.saturating_sub(stamp).clamp(0, 60_000));
            Duration::from_millis(position.max(0) as u64 + elapsed as u64)
        },
    );
    snapshot.playing = playing;
    snapshot.shuffle = options.shuffling_context.or(snapshot.shuffle);
    snapshot.repeat =
        repeat_mode(options.repeating_track, options.repeating_context).or(snapshot.repeat);
    snapshot.stamp = Some(SystemTime::now());
    Start {
        state: Arc::new(snapshot),
        preserve_queue: true,
        restart: true,
    }
}

/// Builds local playback state from a remote Play command.
fn started_by_play(play: &PlayCommand, rows: &ResolvedSequence) -> Option<Start> {
    let context_uri = play.context.uri.as_deref().filter(|uri| !uri.is_empty());
    let skip = play.options.skip_to.as_ref();
    let uri = skip
        .and_then(|skip| skip.track_uri.as_deref())
        .filter(|uri| !uri.is_empty());
    let id = match uri {
        Some(uri) => Some(uri_id(uri)?),
        None => context_uri.and_then(uri_id),
    };
    let uid = skip
        .and_then(|skip| skip.track_uid.as_deref())
        .filter(|uid| !uid.is_empty());
    let key = Occurrence {
        id: id.map(CatalogueId::Borrowed),
        uid,
    };
    let requested = skip
        .and_then(|skip| skip.track_index)
        .map(|index| index as usize);
    let context = context_uri.and_then(collection);
    let index = if let Some(index) = requested.filter(|_| uid.is_none()) {
        rows.track_at(index)
            .and_then(ContextTrack::track_id)
            .filter(|id| key.matches_id(Some(id.as_bytes())))
            .map(|_| index)
    } else if context.is_none() && uid.is_none() && id.is_none() {
        rows.tracks.iter().position(|row| row.track_id().is_some())
    } else {
        sequence_position(rows, key, requested)
    };
    let track = match index {
        Some(index) => Some(context_entry(&rows.tracks[index], false)?),
        None if context_uri.is_some_and(|uri| uri_id(uri).is_none())
            && (uid.is_some() || requested.is_some()) =>
        {
            return None;
        }
        None => id.map(|id| QueuedTrack::new(id.to_owned(), false)),
    };
    if context.is_none() && track.is_none() {
        return None;
    }
    let upcoming = following(rows, index).take(CARRIED).collect();
    let overrides = play.options.player_options_override.as_ref();
    Some(Start {
        state: Arc::new(Snapshot {
            context,
            track,
            index,
            upcoming,
            position: Duration::from_millis(play.options.seek_to.unwrap_or_default()),
            playing: !play.options.initially_paused.unwrap_or_default(),
            shuffle: overrides.and_then(|options| options.shuffling_context),
            repeat: overrides.and_then(|options| {
                repeat_mode(options.repeating_track, options.repeating_context)
            }),
            ..Default::default()
        }),
        preserve_queue: false,
        restart: true,
    })
}

/// A smart shuffle start from a playlist read through the lens: the song asked for, or the first
/// the shuffle picks, and then the rest of the enhanced playlist in shuffled order. The queue
/// goes along whole, since the playlist the app reads itself has no recommendations in it.
fn started_smart(play: &PlayCommand, rows: &ResolvedSequence) -> Option<Start> {
    let asked = started_by_play(play, rows);
    let mut rest: Vec<QueuedTrack> = rows
        .tracks
        .iter()
        .filter_map(|track| context_entry(track, false))
        .collect();
    fastrand::shuffle(&mut rest);
    let base = match &asked {
        Some(start) => (*start.state).clone(),
        None => {
            let overrides = play.options.player_options_override.as_ref();
            Snapshot {
                context: play.context.uri.as_deref().and_then(collection),
                position: Duration::from_millis(play.options.seek_to.unwrap_or_default()),
                playing: !play.options.initially_paused.unwrap_or_default(),
                repeat: overrides.and_then(|options| {
                    repeat_mode(options.repeating_track, options.repeating_context)
                }),
                ..Default::default()
            }
        }
    };
    let track = match base.track.clone() {
        Some(track) => {
            let same = |row: &QueuedTrack| match track.uid.is_empty() {
                true => row.id == track.id,
                false => row.uid == track.uid,
            };
            if let Some(at) = rest.iter().position(same) {
                rest.remove(at);
            }
            track
        }
        None if rest.is_empty() => return None,
        None => rest.remove(0),
    };
    Some(Start {
        state: Arc::new(Snapshot {
            track: Some(track),
            index: base.index.filter(|_| asked.is_some()),
            upcoming: rest,
            shuffle: Some(true),
            ..base
        }),
        preserve_queue: true,
        restart: true,
    })
}

/// Generates a 32-digit hexadecimal playback ID.
fn play_id() -> String {
    format!("{:032x}", fastrand::u128(..))
}

/// A volume from 0 to 1 on Spotify's 16-bit scale.
fn volume_word(level: f32) -> u16 {
    (level.clamp(0., 1.) * u16::MAX as f32).round() as u16
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
    use spotifyapis::proto::connect_state::ContextPage;

    #[test]
    fn remote_queue_resolves_an_occurrence_before_the_last_page() {
        let a = "000000000000000000000a";
        let b = "000000000000000000000b";
        let tracks = [(a, "first"), (a, "second"), (b, "last")]
            .map(|(id, uid)| ContextTrack {
                uri: Some(format!("spotify:track:{id}")),
                uid: Some(uid.into()),
                ..Default::default()
            })
            .into();
        let mut rows = ResolvedSequence::from_context_pages(vec![ContextPage {
            tracks,
            next_page_url: Some("next".into()),
            ..Default::default()
        }]);
        let track = |id| Occurrence {
            id: Some(CatalogueId::Borrowed(id)),
            uid: None,
        };
        let index = sequence_position(&rows, Occurrence::from_context(&rows.tracks[1]), None);
        assert_eq!(index, Some(1));
        assert_eq!(
            following(&rows, index).map(|track| track.uid).next(),
            Some("last".into())
        );
        assert_eq!(sequence_position(&rows, track(a), None), None);
        assert_eq!(sequence_position(&rows, track(b), None), None);
        rows.complete = true;
        assert_eq!(sequence_position(&rows, track(b), None), Some(2));
        let rows = ResolvedSequence::from_context_pages(vec![
            ContextPage {
                page_url: Some("unresolved".into()),
                ..Default::default()
            },
            ContextPage {
                tracks: rows.tracks,
                ..Default::default()
            },
        ]);
        assert!(!rows.complete);
        assert_eq!(sequence_position(&rows, track(b), None), None);

        for (loading, expected) in [(true, None), (false, Some(0))] {
            let page = serde_json::from_value(serde_json::json!({
                "tracks": [{ "uri": format!("spotify:track:{b}") }],
                "loading": loading,
            }))
            .unwrap();
            let rows = ResolvedSequence::from_context_pages(vec![page]);
            assert_eq!(rows.complete, !loading);
            assert_eq!(sequence_position(&rows, track(b), None), expected);
        }
    }
}
