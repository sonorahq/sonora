//! Sonora as a Spotify Connect device. It tells Spotify what plays, so Spotify's apps and
//! Discord show it, takes the commands their controls send, and lists and drives the account's
//! other devices.
//!
//! librespot's own `Spirc` does the same but wants to own the queue, so this speaks the same
//! protocol straight through the session's dealer and spclient and leaves the queue to Sonora.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{StreamExt as _, stream::SelectAll};
use http::{HeaderMap, HeaderValue, Method, header::CONTENT_TYPE};
use librespot_core::dealer::manager::{BoxedStreamResult, Reply, RequestReply};
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
use librespot_protocol::playlist4_external::PlaylistModificationInfo;
use librespot_protocol::social_connect_v2::Session as SocialSession;
use librespot_protocol::transfer_state::TransferState;
use protobuf::rt::WireType;
use protobuf::{CodedInputStream, EnumOrUnknown, Message as _, MessageField};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use crate::connect::{
    Collection, Command, Connect, Device, DeviceKind, Elsewhere, Event, Invite, Jam, JamMember,
    Naming, QueuedTrack, Repeat, Request as Input, Roster, Snapshot, Start,
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
/// How long puts wait after Spotify refuses one for coming too often, and the most they wait
/// after being refused again and again.
const BACKOFF_FIRST: Duration = Duration::from_secs(2);
const BACKOFF_LAST: Duration = Duration::from_secs(60);
/// How long the dealer waits to start again after Spotify could not be reached, and the most it
/// waits after failing again and again.
const START_FIRST: Duration = Duration::from_secs(5);
const START_LAST: Duration = Duration::from_secs(300);

/// Where a Jam's token goes to make a link that opens in Spotify's apps and on the web.
pub(super) const INVITE: &str = "https://open.spotify.com/socialsession/";
const VOLUME_STEPS: u32 = 64;
/// The protobuf tags of a collection update: its items, and in each its kind, raw id and whether
/// it was removed.
const ITEM: u32 = 1 << 3 | 2;
const ITEM_KIND: u32 = 1 << 3;
const ITEM_ID: u32 = 2 << 3 | 2;
const ITEM_REMOVED: u32 = 6 << 3;
/// The dealer topic that reports changes to the group session the listener is in.
const JAM_TOPIC: &str = "social-connect/v2/session_update";
const JAM_JOIN: &str = "/social-connect/v2/sessions/join";
const JAM_CURRENT: &str = "/social-connect/v2/sessions/current";
const JAM_START: &str = "/social-connect/v2/sessions/current_or_new";
const JAM_INFO: &str = "/social-connect/v2/sessions/info";
const JAM_END: &str = "/social-connect/v3/sessions";

/// Starts one worker on the session runtime; it stays idle until device discovery is enabled.
pub fn open(session: Session) -> Connect {
    let (connection, requests, events) = Connect::new(session.device_id().to_owned());
    session.spawn(Worker::new(session.clone(), events).run(requests));
    connection
}

/// What the dealer hands the worker: the connection id Spotify gave this session, changes to the
/// account's devices, volume requests, playback commands, and likes made in other apps.
enum Incoming {
    Connection(String),
    Cluster(ClusterUpdate),
    Volume(SetVolumeCommand),
    Command(Box<RequestReply>),
    Changes(Vec<Event>),
    Jam(Message),
}

/// Dealer subscriptions share dispatch, errors and automatic removal of closed streams.
type Subscriptions = SelectAll<BoxedStreamResult<Incoming>>;

/// Subscribes to everything the device listens to. This has to happen before the dealer
/// starts, because starting it closes the list.
fn subscribe(session: &Session) -> Result<Subscriptions, Error> {
    let dealer = session.dealer();
    Ok(Subscriptions::from_iter([
        dealer.listen_for("hm://pusher/v1/connections/", |message| {
            message
                .headers
                .get("Spotify-Connection-Id")
                .cloned()
                .map(Incoming::Connection)
                .ok_or_else(|| Error::failed_precondition("no connection id in the message"))
        })?,
        dealer.listen_for("hm://connect-state/v1/cluster", |message| {
            Message::from_raw(message).map(Incoming::Cluster)
        })?,
        dealer.listen_for("hm://connect-state/v1/connect/volume", |message| {
            Message::from_raw(message).map(Incoming::Volume)
        })?,
        dealer
            .handle_for("hm://connect-state/v1/player/command")?
            .map(|request| Ok(Incoming::Command(Box::new(request))))
            .boxed(),
        dealer.listen_for("hm://collection/collection/", |message| {
            match message.payload {
                PayloadValue::Raw(bytes) => Ok(Incoming::Changes(collection_changes(&bytes)?)),
                // the same update comes again as JSON, which carries nothing more
                _ => Ok(Incoming::Changes(Vec::new())),
            }
        })?,
        dealer.listen_for(JAM_TOPIC, |message| Ok(Incoming::Jam(message)))?,
        dealer.listen_for("hm://playlist/v2/playlist/", |message| {
            let change: PlaylistModificationInfo = Message::from_raw(message)?;
            let uri = String::from_utf8(change.uri.unwrap_or_default())?;
            let changes = uri
                .strip_prefix("spotify:playlist:")
                .map(|id| Event::PlaylistChanged(id.to_owned()))
                .into_iter()
                .collect();
            Ok(Incoming::Changes(changes))
        })?,
        dealer.listen_for(
            format!("hm://playlist/v2/user/{}/rootlist", session.username()),
            |_| Ok(Incoming::Changes(vec![Event::LibraryChanged])),
        )?,
    ]))
}

struct Worker {
    session: Session,
    out: UnboundedSender<Event>,
    info: DeviceInfo,
    streams: Subscriptions,
    enabled: bool,
    /// The dealer is started once and cannot be restarted on the same session.
    started: bool,
    /// Whether that one start failed, which leaves the device off until the session is replaced.
    launch_failed: bool,
    /// When the dealer tries to start again after Spotify could not be reached, and how long the
    /// try after that waits.
    start_at: Option<Instant>,
    start_wait: Duration,
    connected: bool,
    /// A Jam link asked about before the dealer connected, looked up once it has.
    held: Option<String>,
    now: Option<Arc<Snapshot>>,
    active_since: Option<SystemTime>,
    claimed_at: Option<Instant>,
    /// The id and sender of the last command handled, which the next state echoes back.
    last_command: (u32, String),
    ack_at: Option<Instant>,
    rename_at: Option<Instant>,
    /// The device that has playback on the account, as the last cluster said.
    active_device: String,
    /// Last queue published by another active device, retained across a transfer here.
    remote_queue: Option<(Arc<Snapshot>, Instant)>,
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
            streams: Subscriptions::new(),
            enabled: false,
            started: false,
            launch_failed: false,
            start_at: None,
            start_wait: START_FIRST,
            connected: false,
            held: None,
            now: None,
            active_since: None,
            claimed_at: None,
            last_command: (0, String::new()),
            ack_at: None,
            rename_at: None,
            active_device: String::new(),
            remote_queue: None,
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
                message = self.streams.next(), if !self.streams.is_empty() => match message {
                    Some(Ok(Incoming::Connection(id))) => self.connection_id(id).await,
                    Some(Ok(Incoming::Cluster(update))) => self.cluster(update),
                    Some(Ok(Incoming::Volume(volume))) => self.volume(volume),
                    Some(Ok(Incoming::Command(request))) => self.request(*request),
                    Some(Ok(Incoming::Jam(update))) => self.jam_update(update),
                    Some(Ok(Incoming::Changes(changes))) => {
                        for event in changes { self.out.send(event).ok(); }
                    }
                    Some(Err(error)) => log::warn!("connect: bad dealer update: {error}"),
                    None => {}
                },
                () = acknowledge(self.rename_at) => {
                    self.rename_at = None;
                    if self.enabled && self.connected {
                        self.announce().await;
                    }
                }
                () = acknowledge(self.flush_at) => self.flush().await,
                () = acknowledge(self.start_at) => self.start().await,
                () = acknowledge(self.ack_at) => {
                    self.ack_at = None;
                    if self.now.is_some() {
                        self.want_put(PutStateReason::PLAYER_STATE_CHANGED).await;
                    }
                }
            }
        }

        // The dealer's socket closing takes the device off the account's list.
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
            Input::Join(link) => self.join(&link).await,
            Input::Peek(link) => self.peek(&link).await,
            Input::Start => self.start_jam().await,
            Input::Leave => self.leave().await,
            Input::Kick(member) => self.kick(&member).await,
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

    /// Joins the group session a shared link names, as a listener on this device. Spotify then
    /// moves the host's playback here the way it does for any transfer.
    async fn join(&mut self, link: &str) {
        if !self.enabled || !self.connected {
            return log::warn!("connect: cannot join a jam while not listed as a device");
        }
        let Some(token) = jam_token(link).await else {
            return log::warn!("connect: no jam token in the link");
        };
        let endpoint = format!(
            "{JAM_JOIN}/{token}?playback_control=listen_and_control&join_type=deeplinking&local_device_id={}",
            self.session.device_id()
        );
        match jam_read(&self.session, &Method::POST, &endpoint).await {
            Ok(Some(read)) => {
                log::info!("connect: joined a jam");
                // the update that follows has the members, and this has them too when it reads
                self.out.send(Event::Jam(Some(read.jam))).ok();
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("connect: cannot join the jam: {error}");
                self.out.send(Event::JamFailed).ok();
            }
        }
    }

    /// The group session the account is in now, if any.
    async fn current_jam(session: &Session) -> Result<Option<Jam>, Error> {
        let jam = Self::current_session(session).await?.map(|read| read.jam);
        match &jam {
            Some(jam) => log::info!("connect: in a jam of {}", jam.members.len()),
            None => log::info!("connect: not in a jam"),
        }
        Ok(jam)
    }

    /// The group session the account is in. Spotify answers 404 when there is none, which is
    /// `Ok(None)`, unlike a request that failed.
    async fn current_session(session: &Session) -> Result<Option<Read>, Error> {
        match jam_read(session, &Method::GET, JAM_CURRENT).await {
            Ok(Some(read)) => Ok(Some(read)),
            Ok(None) => Err(Error::data_loss("the jam answer cannot be read")),
            Err(error) if error.kind == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Reads the group session a link names without joining it, and says what it found in an
    /// `Event::Invite`, which is `None` when the link names none or it cannot be read.
    async fn peek(&mut self, link: &str) {
        if self.enabled && !self.connected && !self.launch_failed {
            self.held = Some(link.to_owned());
            return;
        }
        let found = self.peeked(link).await;
        self.out.send(Event::Invite(found)).ok();
    }

    async fn peeked(&mut self, link: &str) -> Option<Invite> {
        if !self.enabled || !self.connected {
            log::warn!("connect: cannot look up a jam while not listed as a device");
            return None;
        }
        let Some(token) = jam_token(link).await else {
            log::warn!("connect: no jam token in the link");
            return None;
        };
        let read = jam_read(&self.session, &Method::GET, &format!("{JAM_INFO}/{token}"))
            .await
            .inspect_err(|error| log::warn!("connect: cannot look up the jam: {error}"))
            .ok()??;
        Some(Invite {
            link: format!("{INVITE}{token}"),
            jam: read.jam,
        })
    }

    /// Starts a group session with this device as the one playing, or finds the one that runs.
    /// The link to invite others with comes back in the `Event::Jam`.
    async fn start_jam(&mut self) {
        if !self.enabled || !self.connected {
            return log::warn!("connect: cannot start a jam while not listed as a device");
        }
        // activate is what the desktop app asks for, and what has the session show in the others
        let endpoint = format!(
            "{JAM_START}?activate=true&local_device_id={}&type=REMOTE",
            self.session.device_id()
        );
        match jam_read(&self.session, &Method::GET, &endpoint).await {
            Ok(Some(read)) => {
                log::info!("connect: started a jam, link {}", read.jam.link.is_some());
                self.out.send(Event::Jam(Some(read.jam))).ok();
            }
            Ok(None) => {
                log::warn!("connect: the jam that started cannot be read");
                self.out.send(Event::JamFailed).ok();
            }
            Err(error) => {
                log::warn!("connect: cannot start a jam: {error}");
                self.out.send(Event::JamFailed).ok();
            }
        }
    }

    /// Leaves the group session. The owner ends it for everyone instead, since leaving would
    /// only hand it on. A session that cannot be read is left as it is, since it may be owned.
    async fn leave(&mut self) {
        let id = match Self::current_session(&self.session).await {
            Ok(Some(read)) if is_path_id(&read.id, "_-") => Some((read.id, read.jam.owner)),
            Ok(_) => None,
            Err(error) => {
                log::warn!("connect: cannot read the jam to leave it: {error}");
                return self.leave_failed().await;
            }
        };
        let Some((id, owner)) = id else {
            return self.leave_failed().await;
        };
        let device = self.session.device_id();
        let answer = match owner {
            true => {
                let endpoint = format!("{JAM_END}/{id}?local_device_id={device}");
                jam_request(&self.session, &Method::DELETE, &endpoint).await
            }
            false => {
                let endpoint = format!("{JAM_END}/{id}/leave?local_device_id={device}");
                jam_request(&self.session, &Method::POST, &endpoint).await
            }
        };
        match answer {
            Ok(_) => {
                self.out.send(Event::Jam(None)).ok();
            }
            Err(error) => {
                log::warn!("connect: cannot leave the jam: {error}");
                self.leave_failed().await;
            }
        }
    }

    /// Says a leave did not go through, and then what the group session is now, since the app
    /// takes the Jam off screen before it asks.
    async fn leave_failed(&mut self) {
        self.out.send(Event::LeaveFailed).ok();
        if let Ok(jam) = Self::current_jam(&self.session).await {
            self.out.send(Event::Jam(jam)).ok();
        }
    }

    /// Removes `member` from the group session, which only its owner may do, and then reports
    /// who is left without waiting for the dealer to.
    async fn kick(&mut self, member: &str) {
        // the id goes into the path, so nothing else than what an id is made of is let through
        if !is_path_id(member, "._-") {
            return log::warn!("connect: not a member id to remove");
        }
        let id = match Self::current_session(&self.session).await {
            Ok(Some(read)) if read.jam.owner && is_path_id(&read.id, "_-") => read.id,
            Ok(_) => return log::warn!("connect: only the owner of a jam can remove a member"),
            Err(error) => {
                return log::warn!("connect: cannot read the jam to remove from: {error}");
            }
        };
        let endpoint = format!(
            "{JAM_END}/{id}/member/{member}/kick?local_device_id={}",
            self.session.device_id()
        );
        let answer = jam_request(&self.session, &Method::POST, &endpoint).await;
        match answer {
            Ok(_) => {
                log::info!("connect: removed a member from the jam");
                if let Ok(Some(read)) = Self::current_session(&self.session).await {
                    self.out.send(Event::Jam(Some(read.jam))).ok();
                }
            }
            Err(error) => log::warn!("connect: cannot remove the member: {error}"),
        }
    }

    /// Reads session updates without requiring the reason enum to be known to librespot.
    fn jam_update(&self, message: Message) {
        let PayloadValue::Json(json) = message.payload else {
            return;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&json) else {
            return log::warn!("connect: unreadable jam update");
        };
        let reason = json.get("reason").and_then(|reason| reason.as_str());
        log::info!("connect: jam update {reason:?}");
        let ended = matches!(
            reason,
            Some("SESSION_DELETED" | "YOU_LEFT" | "YOU_WERE_KICKED")
        );
        let jam = json
            .get("session")
            .and_then(|session| social_json(session.clone()))
            .map(|session| read_social(session, &self.session.username()).jam);
        match (ended, jam) {
            (true, _) => self.out.send(Event::Jam(None)).ok(),
            // an update with no session says nothing about who is in it
            (false, Some(jam)) => self.out.send(Event::Jam(Some(jam))).ok(),
            (false, None) => None,
        };
    }

    /// Moves playback to `to`. This app takes it the way librespot does, by naming itself on both
    /// ends, and hands it on from itself, or from whichever device the last cluster named.
    async fn transfer(&mut self, to: String) {
        let mine = self.session.device_id().to_owned();
        let from = match (
            to == mine,
            self.active_since.is_some(),
            self.active_device.is_empty(),
        ) {
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
            self.now.as_ref().map(|now| now.live_position().as_secs()),
            self.now.as_ref().map(|now| now.playing),
        );
        // Spotify answers a move here only once this device has answered the command it sends
        // for it, which the loop can give only if it is not waiting on the move itself
        if to == mine {
            let session = self.session.clone();
            self.session.spawn(async move {
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
            (false, _) => {
                self.start_at = None;
                self.withdraw().await;
            }
        }
    }

    /// Starts the dealer once Spotify can be reached. It launches only once per session, so the
    /// token it needs is fetched first, and a failure there tries again later rather than spend
    /// the launch. A Jam link held for the dealer is answered as unreadable whenever a try fails.
    async fn start(&mut self) {
        self.start_at = None;
        if !self.enabled || self.started {
            return;
        }
        // the token is cached, so the launch does not ask for it again, and the dealer's address
        // is left alone, since resolving one uses it up
        if let Err(error) = self.session.login5().auth_token().await {
            let wait = self.start_wait;
            self.start_wait = (wait * 2).min(START_LAST);
            self.start_at = Some(Instant::now() + wait);
            log::warn!(
                "connect: cannot reach the dealer, trying again in {}s: {error}",
                wait.as_secs()
            );
            return self.drop_held();
        }
        self.start_wait = START_FIRST;
        self.started = true;
        let launched = match subscribe(&self.session) {
            Ok(streams) => {
                self.streams = streams;
                self.session.dealer().start().await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = launched {
            self.streams = Subscriptions::new();
            self.launch_failed = true;
            log::warn!("connect: cannot start the dealer, off until the next session: {error}");
            self.drop_held();
        }
    }

    /// Answers the Jam link held for the dealer as unreadable, as the dealer did not come up.
    fn drop_held(&mut self) {
        if self.held.take().is_some() {
            self.out.send(Event::Invite(None)).ok();
        }
    }

    /// The dealer's first message carries the connection id Spotify files this session under,
    /// and the device can only be announced once it is known.
    async fn connection_id(&mut self, id: String) {
        self.session.set_connection_id(&id);
        if self.connected {
            self.out.send(Event::LibraryChanged).ok();
        }
        self.connected = true;
        if self.enabled {
            self.announce().await;
        }
        if let Some(link) = self.held.take() {
            self.peek(&link).await;
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
        // the listener may have joined a jam from another app before this one was listed
        match Self::current_jam(&self.session).await {
            Ok(jam) => {
                self.out.send(Event::Jam(jam)).ok();
            }
            Err(error) => log::warn!("connect: cannot read the jam: {error}"),
        }
        // something may have been playing before the device was listed
        if self.now.is_some() {
            self.sync(None).await;
        }
    }

    async fn withdraw(&mut self) {
        self.active_since = None;
        self.out.send(Event::Roster(Roster::default())).ok();
        if !self.connected {
            return;
        }
        if let Err(error) = self.session.spclient().delete_connect_state_request().await {
            log::warn!("connect: cannot leave the device list: {error}");
        }
    }

    async fn publish(&mut self, now: Option<Arc<Snapshot>>) {
        let before = std::mem::replace(&mut self.now, now);
        self.sync(before).await;
    }

    /// Puts the state the device is in now, given what it reported `before`.
    async fn sync(&mut self, before: Option<Arc<Snapshot>>) {
        if !self.enabled || !self.connected {
            return;
        }
        let Some(now) = &self.now else {
            if self.active_since.take().is_some()
                && let Err(error) = self
                    .session
                    .spclient()
                    .put_connect_state_inactive(false)
                    .await
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
            true => PutStateReason::VOLUME_CHANGED,
            false => PutStateReason::PLAYER_STATE_CHANGED,
        };
        self.want_put(reason).await;
    }

    /// Keeps `reason` as the pending put unless one that matters more is already held.
    fn hold(&mut self, reason: PutStateReason) {
        self.pending = Some(match self.pending {
            Some(held) if rank(held) > rank(reason) => held,
            _ => reason,
        });
    }

    /// Asks for the state to be put. Puts are spaced out, and held back for longer each time
    /// Spotify refuses one for coming too often. Whatever the wait, the put that goes out
    /// carries the state as it is then, so the changes in between cost nothing.
    async fn want_put(&mut self, reason: PutStateReason) {
        self.hold(reason);

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
                self.hold(reason);
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
            is_active: self.active_since.is_some() && self.now.is_some(),
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
        let Some(track) = &now.track else {
            return state;
        };

        // A standalone song names itself when there is no collection to load on transfer.
        let uri = match &now.context {
            Some(context) => collection_uri(context, &self.session.username()),
            None => format!("{TRACK_PREFIX}{}", track.id),
        };
        state.track = MessageField::some(provided(track, &uri));
        state.prev_tracks = now
            .previous
            .iter()
            .map(|track| provided(track, &uri))
            .collect();
        state.next_tracks = now
            .upcoming
            .iter()
            .map(|track| provided(track, &uri))
            .collect();
        state.context_url = format!("context://{uri}");
        state.context_uri = uri;
        // a device taking over finds the track by where it sits as well as by its uri
        state.index = MessageField::from_option(now.index.map(|track| ContextIndex {
            track: track as u32,
            ..Default::default()
        }));
        state.timestamp =
            (stamp as i64).saturating_add(self.session.time_delta().saturating_mul(1000));
        state.position_as_of_timestamp = now.live_position().as_millis() as i64;
        state.duration = now.duration.as_millis() as i64;
        state.options = MessageField::some(ContextPlayerOptions {
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
        if self.active_since.is_some() && elsewhere && fresh {
            self.active_since = None;
            self.out.send(Event::Command(Command::Released)).ok();
        }
        let roster = self.roster(cluster);
        self.out.send(Event::Roster(roster)).ok();
    }

    /// Uses a matching fresh player report to preserve queue order that transfer contexts can omit.
    fn transferred(&self, state: TransferState) -> Start {
        let now =
            (unix_millis() as i64).saturating_add(self.session.time_delta().saturating_mul(1000));
        let start = started_by_transfer(state, now);
        if let Some((queue, stamp)) = &self.remote_queue {
            let matching_context = start
                .state
                .context
                .as_ref()
                .is_none_or(|context| queue.context.as_ref() == Some(context));
            if stamp.elapsed() <= QUEUE_SNAPSHOT_AGE
                && matching_context
                && queue.track.as_ref().map(|track| &track.id)
                    == start.state.track.as_ref().map(|track| &track.id)
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

    fn roster(&mut self, cluster: &Cluster) -> Roster {
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
                    state: Arc::new(Snapshot {
                        track: state.track.as_ref().and_then(listed_track),
                        playing: state.is_playing && !state.is_paused,
                        position: millis(state.position_as_of_timestamp),
                        stamp: Some(
                            UNIX_EPOCH
                                + millis(state.timestamp.saturating_sub(
                                    self.session.time_delta().saturating_mul(1000),
                                )),
                        ),
                        duration: millis(state.duration),
                        shuffle: Some(state.options.shuffling_context),
                        repeat: repeat_mode(
                            Some(state.options.repeating_track),
                            Some(state.options.repeating_context),
                        ),
                        volume: info.volume as f32 / u16::MAX as f32,
                        context: collection(&state.context_uri),
                        index: state.index.as_ref().map(|index| index.track as usize),
                        previous: listed(&state.prev_tracks, state.prev_tracks.len()),
                        upcoming: listed(&state.next_tracks, CARRIED),
                    }),
                }
            });
        if let Some(remote) = &elsewhere {
            self.remote_queue = Some((remote.state.clone(), Instant::now()));
        }
        Roster { devices, elsewhere }
    }

    fn volume(&mut self, command: SetVolumeCommand) {
        let level = command.volume.clamp(0, u16::MAX as i32) as f32 / u16::MAX as f32;
        self.out.send(Event::Command(Command::Volume(level))).ok();
    }

    fn request(&mut self, (request, reply): RequestReply) {
        self.last_command = (request.message_id, request.sent_by_device_id.clone());
        log::debug!("connect: asked, {}", request.command);
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

    /// Starts what another device asked to play. A phone names the song by its uid alone, and a
    /// jam names a context Sonora cannot read itself. Both send no tracks along, so the context
    /// is read back first to find the song and what follows it.
    fn play(&self, mut play: PlayCommand) {
        let uri = play.context.uri.clone();
        let out = self.out.clone();
        let session = self.session.clone();
        let blind = play.context.pages.iter().all(|page| page.tracks.is_empty());
        let by_uid = play.options.skip_to.as_ref().is_some_and(|skip| {
            skip.track_uri.as_deref().is_none_or(str::is_empty)
                && skip.track_uid.as_deref().is_some_and(|uid| !uid.is_empty())
        });
        let (Some(uri), true) = (uri, blind && (by_uid || collection_is_unread(&play))) else {
            out.send(Event::Command(Command::Start(started_by_play(&play))))
                .ok();
            return;
        };
        self.session.spawn(async move {
            match session.spclient().get_context(&uri).await {
                Ok(context) => play.context.pages = context.pages,
                Err(error) => log::warn!("connect: cannot read {} back: {error}", redacted(&uri)),
            }
            out.send(Event::Command(Command::Start(started_by_play(&play))))
                .ok();
        });
    }

    /// What a command from Spotify asks of the app, or `None` for one this device does not
    /// know.
    fn translate(&mut self, command: Wire) -> Option<Vec<Command>> {
        Some(match command {
            Wire::Transfer(transfer) => vec![Command::Start(self.transferred(transfer.data?))],
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
            Wire::AddToQueue(add) => {
                log::debug!(
                    "connect: a track was queued, it carries {:?}",
                    keys(&add.track)
                );
                track_id(&add.track.uri)
                    .map(|id| Command::Enqueue {
                        id,
                        added_by: adder(&add.track),
                    })
                    .into_iter()
                    .collect()
            }
            Wire::SetQueue(queue) => {
                let previous = listed(&queue.prev_tracks, queue.prev_tracks.len());
                // an edit that added songs runs past the window it was shown, and all of it counts
                let upcoming = listed(&queue.next_tracks, queue.next_tracks.len());
                // Acknowledge the edit with its new queue while the host resolves missing metadata.
                if let Some(now) = self.now.as_mut() {
                    let now = Arc::make_mut(now);
                    now.previous = previous.clone();
                    now.upcoming = upcoming.clone();
                }
                vec![Command::SetQueue { previous, upcoming }]
            }
            Wire::UpdateContext(_) => Vec::new(),
            Wire::Unknown(value) => {
                // the field names only, since a transfer carries the listener's whole state
                let fields = value
                    .as_object()
                    .map(|object| object.keys().collect::<Vec<_>>());
                log::debug!("connect: unsupported player command, it carries {fields:?}");
                return None;
            }
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

/// Asks Spotify to move playback from one device to another, restoring a paused state.
async fn move_playback(session: &Session, from: &str, to: &str) -> Result<(), Error> {
    let request = TransferRequest {
        transfer_options: TransferOptions {
            restore_paused: Some("restore".to_owned()),
            ..Default::default()
        },
    };
    session
        .spclient()
        .transfer(from, to, Some(&request))
        .await
        .map(|_| ())
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
        command => {
            let Some(command) = wire_command(&command, &session.username()) else {
                log::warn!("connect: nothing to send for {what}");
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

/// The token of a group session in a shared link. Only an https `spotify.link` short link is
/// fetched, and then its address or its page names the session; no other text makes the app
/// fetch anything.
async fn jam_token(link: &str) -> Option<String> {
    let link = link.trim();
    let short = crate::link::address(link)
        .is_some_and(|url| url.scheme() == "https" && url.host_str() == Some("spotify.link"));
    if !short {
        return token_in(link);
    }
    // a short link sometimes answers with a page that names the session instead of redirecting
    let followed = crate::link::follow(link).await?;
    token_in(&followed.address).or_else(|| token_in(&followed.page))
}

/// The token that follows `socialsession/` in a link or a page, or `sessions/join/` in the
/// address Spotify itself gives a session. It is letters and digits only, since it goes into a
/// request.
fn token_in(text: &str) -> Option<String> {
    let rest = ["socialsession/", "sessions/join/"]
        .into_iter()
        .find_map(|marker| text.split(marker).nth(1))?;
    let token = rest.split(|c: char| !c.is_ascii_alphanumeric()).next()?;
    (!token.is_empty()).then(|| token.to_owned())
}

/// The link that invites others to the session at `url`, which Spotify gives as an address only
/// its own apps open.
fn invite(url: &str) -> Option<String> {
    match token_in(url) {
        Some(token) => Some(format!("{INVITE}{token}")),
        None => url.starts_with("https://").then(|| url.to_owned()),
    }
}

/// A group session as Spotify answers for it: what the app shows, the id to end it by, and
/// whether the account owns it.
struct Read {
    jam: Jam,
    id: String,
}

/// Reads JSON through librespot's protobuf schema, ignoring fields the app does not use.
fn social_json(mut json: serde_json::Value) -> Option<SocialSession> {
    let object = json.as_object_mut()?;
    // This unused enum can acquire names newer than the pinned schema.
    object.remove("initial_session_type");
    object.remove("initialSessionType");
    librespot_core::deserialize_with::json_proto(json).ok()
}

/// Reads a group session from a REST answer, or `None` when the answer names none.
async fn jam_read(
    session: &Session,
    method: &Method,
    endpoint: &str,
) -> Result<Option<Read>, Error> {
    let body = jam_request(session, method, endpoint).await?;
    Ok(read_session(&body, &session.username()))
}

/// Both REST encodings use the same session conversion as dealer updates.
fn read_session(body: &[u8], own: &str) -> Option<Read> {
    let session = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(json) => social_json(json.get("session").cloned().unwrap_or(json))?,
        Err(_) => SocialSession::parse_from_bytes(body).ok()?,
    };
    (!session.session_id.is_empty()).then(|| read_social(session, own))
}

/// Converts one decoded session, retaining owner-only invite links.
fn read_social(session: SocialSession, own: &str) -> Read {
    let link = session
        .is_session_owner
        .then(|| invite(&session.join_session_url))
        .flatten();
    let listed = session.session_members.iter().map(|member| {
        let image = match member.image_url.is_empty() {
            true => &member.large_image_url,
            false => &member.image_url,
        };
        [&member.id, &member.username, &member.display_name, image].map(String::as_str)
    });
    Read {
        jam: Jam {
            members: members_of(
                listed,
                &session.session_owner_id,
                session.is_session_owner,
                own,
            ),
            owner: session.is_session_owner,
            link,
        },
        id: session.session_id,
    }
}

/// The people in a session, from what it lists of each as `[id, username, name, image]`.
/// `owner` is the id of whoever owns the session when it says, and `owned` whether the account
/// `own` does, which makes the account the host when no id names one.
fn members_of<'a>(
    listed: impl Iterator<Item = [&'a str; 4]>,
    owner: &str,
    owned: bool,
    own: &str,
) -> Vec<JamMember> {
    let mut members = listed
        .filter_map(|[id, username, name, image]| {
            let id = [id, username].into_iter().find(|id| !id.is_empty())?;
            Some(JamMember {
                id: id.to_owned(),
                username: username.to_owned(),
                name: [name, username, id]
                    .into_iter()
                    .find(|name| !name.is_empty())?
                    .to_owned(),
                image: image_of(image),
                host: !owner.is_empty() && (id == owner || username == owner),
                you: id == own || username == own,
            })
        })
        .collect::<Vec<_>>();
    if owned
        && !members.iter().any(|member| member.host)
        && let Some(you) = members.iter_mut().find(|member| member.you)
    {
        you.host = true;
    }
    members
}

/// The address of a picture Spotify names either by a link or by an `spotify:image:` uri.
fn image_of(image: &str) -> Option<String> {
    match image.strip_prefix("spotify:image:") {
        Some(id) => Some(format!("https://i.scdn.co/image/{id}")),
        None => image.starts_with("https://").then(|| image.to_owned()),
    }
}

/// Ties group-session requests to the dealer; spclient supplies Content-Length itself.
async fn jam_request(
    session: &Session,
    method: &Method,
    endpoint: &str,
) -> librespot_core::spclient::SpClientResult {
    let mut headers = HeaderMap::new();
    if let Ok(id) = HeaderValue::from_str(&session.connection_id())
        && !id.is_empty()
    {
        headers.insert("spotify-connection-id", id);
    }
    session
        .spclient()
        .request(method, endpoint, Some(headers), None)
        .await
}

/// Whether `id` is non-empty and made only of letters, digits and the `extra` characters, which
/// makes it safe to put in a URL path.
fn is_path_id(id: &str, extra: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
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
    Some(serde_json::json!({
        "endpoint": "play",
        "context": { "uri": context, "url": format!("context://{context}"), "metadata": {} },
        "play_origin": { "feature_identifier": "sonora", "feature_version": SEMVER },
        "options": {
            "skip_to": track.map(|uri| serde_json::json!({ "track_uri": uri })),
            "seek_to": start.position.as_millis() as u64,
            "initially_paused": !start.playing,
        },
        "logging_params": {},
    }))
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

/// Reports the occurrence's identity and provenance consistently in every part of play order.
fn provided(track: &QueuedTrack, context: &str) -> ProvidedTrack {
    let mut provided = ProvidedTrack {
        uri: format!("{TRACK_PREFIX}{}", track.id),
        uid: track.uid.clone(),
        provider: match track.queued {
            true => "queue",
            false => "context",
        }
        .to_owned(),
        metadata: [("context_uri", context), ("entity_uri", context)]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
        ..Default::default()
    };
    if let Some(adder) = &track.added_by {
        provided
            .metadata
            .insert("added_by".to_owned(), adder.clone());
    }
    provided
}

fn track_id(uri: &str) -> Option<String> {
    uri.strip_prefix(TRACK_PREFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// The first tracks of a player state's queue, with who queued each when the state says.
fn listed(tracks: &[ProvidedTrack], limit: usize) -> Vec<QueuedTrack> {
    let listed = tracks
        .iter()
        .filter_map(listed_track)
        .take(limit)
        .collect::<Vec<_>>();
    if let Some(track) = tracks.iter().find(|track| track.provider == "queue") {
        log::debug!("connect: a queued track carries {:?}", keys(track));
    }
    listed
}

/// Reads the same occurrence provenance for current, previous and upcoming tracks.
fn listed_track(track: &ProvidedTrack) -> Option<QueuedTrack> {
    Some(QueuedTrack {
        id: track_id(&track.uri)?,
        uid: track.uid.clone(),
        queued: is_queued(track),
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

/// The metadata keys a track carries, sorted, so a log shows what Spotify sends without the values.
fn keys(track: &ProvidedTrack) -> Vec<&String> {
    let mut keys = track.metadata.keys().collect::<Vec<_>>();
    keys.sort();
    keys
}

/// `uri` with the name after each `user:` replaced, so a log never carries a Spotify user name.
fn redacted(uri: &str) -> String {
    let mut owner = false;
    uri.split(':')
        .map(|part| {
            let shown = match owner {
                true => "<user>",
                false => part,
            };
            owner = part == "user";
            shown
        })
        .collect::<Vec<_>>()
        .join(":")
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

/// The occurrences a list of provider ids names, all with the same provenance.
fn occurrences(ids: impl IntoIterator<Item = String>, queued: bool) -> Vec<QueuedTrack> {
    ids.into_iter()
        .map(|id| QueuedTrack::new(id, queued))
        .collect()
}

/// Where a transfer from another device leaves off: its track, how far in, and what follows.
fn started_by_transfer(state: TransferState, now: i64) -> Start {
    let playback = &state.playback;
    let paused = playback.is_paused.unwrap_or_default();
    let mut position = playback.position_as_of_timestamp.unwrap_or_default().max(0) as u64;
    if let (false, Some(stamp)) = (paused, playback.timestamp) {
        // the position was true when the timestamp was taken, and the music went on since
        position += now.saturating_sub(stamp).clamp(0, 60_000) as u64;
    }

    let from_queue = state.queue.is_playing_queue.unwrap_or_default();
    let current = match from_queue {
        true => state.queue.tracks.first(),
        false => playback.current_track.as_ref(),
    };
    let track = current.and_then(listed_id);

    let context = &state.current_session.context;
    let context_tracks: Vec<_> = context.pages.iter().flat_map(|page| &page.tracks).collect();
    let anchor = match from_queue {
        true => playback.current_track.as_ref().and_then(listed_id),
        false => track.clone(),
    };
    let place = context_tracks
        .iter()
        .position(|item| listed_id(item).as_ref() == anchor.as_ref());
    let mut previous: Vec<_> = state
        .play_history
        .backward_items
        .iter()
        .filter_map(|item| {
            let uid = item.uid.as_deref()?;
            context_tracks
                .iter()
                .find(|track| track.uid.as_deref() == Some(uid))
                .and_then(|track| listed_id(track))
        })
        .collect();
    if previous.is_empty() {
        previous = context_tracks
            .iter()
            .take(place.unwrap_or(0) + usize::from(from_queue && place.is_some()))
            .filter_map(|track| listed_id(track))
            .collect();
    }
    previous.drain(..previous.len().saturating_sub(PREVIOUS));
    let queued: Vec<_> = state
        .queue
        .tracks
        .iter()
        .skip(from_queue as usize)
        .filter_map(listed_id)
        .collect();
    let following = context_tracks
        .iter()
        .skip(place.map(|place| place + 1).unwrap_or(0))
        .filter_map(|track| listed_id(track));
    let repeat = repeat_mode(
        state.options.repeating_track,
        state.options.repeating_context,
    );
    Start {
        state: Arc::new(Snapshot {
            context: context.uri.as_deref().and_then(collection),
            upcoming: occurrences(queued, true)
                .into_iter()
                .chain(occurrences(following, false))
                .take(CARRIED)
                .collect(),
            previous: occurrences(previous, false),
            shuffle: state.options.shuffling_context,
            repeat,
            track: track.map(|id| QueuedTrack::new(id, from_queue)),
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
fn collection_is_unread(play: &PlayCommand) -> bool {
    play.context
        .uri
        .as_deref()
        .is_some_and(|uri| collection(uri).is_none())
}

/// What a play command from another device asks for.
fn started_by_play(play: &PlayCommand) -> Start {
    let tracks: Vec<_> = play
        .context
        .pages
        .iter()
        .flat_map(|page| &page.tracks)
        .collect();
    let listed: Vec<_> = tracks.iter().filter_map(|track| listed_id(track)).collect();

    // the same order librespot resolves a skip in: uri, then uid, then place
    let skip = play.options.skip_to.as_ref();
    log::debug!(
        "connect: play in {:?}, skip to {skip:?}, {} pages with {} tracks",
        play.context.uri.as_deref().map(redacted),
        play.context.pages.len(),
        listed.len(),
    );
    let track = skip
        .and_then(|skip| skip.track_uri.as_deref())
        .and_then(track_id)
        .or_else(|| {
            let uid = skip?.track_uid.as_deref().filter(|uid| !uid.is_empty())?;
            tracks
                .iter()
                .find(|track| track.uid.as_deref() == Some(uid))
                .and_then(|track| listed_id(track))
        })
        .or_else(|| {
            let index = skip.and_then(|skip| skip.track_index)? as usize;
            // the place counts every entry of the page, including ones that name no track
            tracks.get(index).and_then(|track| listed_id(track))
        });
    let index = skip
        .and_then(|skip| skip.track_index)
        .map(|index| index as usize);
    let upcoming = occurrences(
        listed
            .into_iter()
            .skip_while(|id| Some(id) != track.as_ref())
            .skip(1)
            .take(CARRIED),
        false,
    );

    Start {
        state: Arc::new(Snapshot {
            context: play.context.uri.as_deref().and_then(collection),
            track: track.map(|id| QueuedTrack::new(id, false)),
            index,
            upcoming,
            position: Duration::from_millis(play.options.seek_to.unwrap_or_default().into()),
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

/// Track-like deltas and invalidations for other saved collections reported by the dealer.
/// librespot has no message for the update, so its few fields are read by their tags: a list of
/// items, each with its kind (0 for a track), raw id and whether it was removed.
fn collection_changes(bytes: &[u8]) -> protobuf::Result<Vec<Event>> {
    let mut input = CodedInputStream::from_bytes(bytes);
    let mut likes = Vec::new();
    while let Some(tag) = input.read_raw_tag_or_eof()? {
        match tag {
            ITEM => likes.extend(collection_item(&input.read_bytes()?)?),
            tag => match WireType::new(tag & 7) {
                Some(wire) => input.skip_field(wire)?,
                None => break,
            },
        }
    }
    Ok(likes)
}

/// Reads a track delta exactly; other item kinds invalidate the library without guessing IDs.
fn collection_item(bytes: &[u8]) -> protobuf::Result<Option<Event>> {
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
    Ok(match kind {
        0 => SpotifyId::from_raw(&gid).ok().map(|id| Event::Liked {
            track: id.to_base62(),
            liked: !removed,
        }),
        _ => Some(Event::LibraryChanged),
    })
}
