//! Sonora as a Spotify Connect device. It tells Spotify what plays, so Spotify's apps and
//! Discord show it, takes the commands their controls send, and lists and drives the account's
//! other devices.
//!
//! librespot's own `Spirc` does the same but wants to own the queue, so this speaks the same
//! protocol straight through the session's dealer and spclient and leaves the queue to Sonora.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::StreamExt as _;
use futures::stream::{self, SelectAll};
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
use librespot_protocol::context::Context;
use librespot_protocol::context_page::ContextPage;
use librespot_protocol::context_track::ContextTrack;
use librespot_protocol::devices::DeviceType;
use librespot_protocol::media::AudioQuality;
use librespot_protocol::player::{
    ContextIndex, ContextPlayerOptions, PlayOrigin, PlayerState, ProvidedTrack, Suppressions,
};
use librespot_protocol::playlist4_external::PlaylistModificationInfo;
use librespot_protocol::social_connect_v2::Session as SocialSession;
use librespot_protocol::transfer_state::TransferState;
use music::RadioSeed;
use music::connect::{
    Collection, Command, Connect, Device, DeviceKind, Elsewhere, Event, Invite, Jam, JamMember,
    Naming, QueuedTrack, Repeat, Request as Input, Roster, Snapshot, Start,
};
use protobuf::rt::WireType;
use protobuf::{CodedInputStream, EnumOrUnknown, Message as _, MessageField};
use sha1::{Digest as _, Sha1};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;

const DEVICE_NAME: &str = "Sonora";
const TRACK_PREFIX: &str = "spotify:track:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
/// The context a state names when it has none, which Spotify needs to see to keep the device
/// active.
const UNKNOWN_CONTEXT: &str = "spotify:unknown";
/// How long a command waits for the app to act on it before the state is put regardless, so the
/// device that sent it always hears that it was handled.
const ACK_DELAY: Duration = Duration::from_millis(600);
/// How long after claiming playback a report of another active device is taken to be older than
/// the claim.
const CLAIM_GRACE: Duration = Duration::from_secs(3);
/// How long the song playing is told as a pick of Spotify's after someone joins the Jam.
const WELCOME: Duration = Duration::from_secs(5);
/// How long a skip made for the Jam while paused may show as playing before its pause lands.
const ADVANCING: Duration = Duration::from_secs(3);
/// The most pages a collection only links to that are read for its songs.
const UID_PAGES: usize = 40;
/// How many characters of the digest make the uid Spotify's apps give a track they find listed
/// without one.
const DERIVED_UID: usize = 20;
/// How far this device may be from where a Jam says its song is before it is moved there. A
/// seek is heard, and a transfer arrives for every change a listener makes.
const JAM_SLACK: Duration = Duration::from_millis(1500);
/// The most upcoming tracks a start carries over from the device that sent it.
const CARRIED: usize = 80;
/// The recent history window Spotify includes in its player state.
const PREVIOUS: usize = 10;
/// The lens that asks Spotify for a playlist with smart shuffle's recommendations mixed in.
const LENS: &str = "?spotify-apply-lenses=enhance";
/// The player mode that says whether smart shuffle is on.
const ENHANCEMENT: &str = "context_enhancement";
/// The value of `ENHANCEMENT` when smart shuffle is on.
const RECOMMENDATION: &str = "RECOMMENDATION";
/// The provider Spotify gives a song that smart shuffle mixed into a playlist.
const RECOMMENDED: &str = "enhanced_recommendation";
/// `Capabilities.supports_smart_shuffle_mode`, newer than librespot's protocol files. Spotify
/// moves playback away from a device without it when smart shuffle is switched on.
const SMART_SHUFFLE_CAPABILITY: u32 = 33;
/// A fresh cluster snapshot carries the exact shuffled queue and previous-track order.
const QUEUE_SNAPSHOT_AGE: Duration = Duration::from_secs(10);
/// How many tracks of a queue a log line spells out.
const OUTLINED: usize = 16;
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
    Enhanced(Box<EnhancedRead>),
}

/// Dealer subscriptions and one-shot reads share dispatch, errors and automatic removal of closed
/// streams.
type Subscriptions = SelectAll<BoxedStreamResult<Incoming>>;

/// The collection smart shuffle is on for, and the songs it mixed into it.
struct Enhanced {
    context: String,
    recommended: HashSet<String>,
    /// Where each song sits in the enhanced collection, and its uid there. A device taking over
    /// loads that collection and finds the track by these, not by its place in the plain one.
    places: HashMap<String, (u32, String)>,
}

/// A smart shuffle collection read for a command, handed back to the worker so that it applies
/// to what plays when the read lands. `play` is the play command it was read for, or `None` when
/// smart shuffle was switched on during playback.
struct EnhancedRead {
    ticket: u64,
    uri: String,
    play: Option<PlayCommand>,
    read: Result<(Context, HashSet<String>), Error>,
}

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
    /// The id of the run of tracks being reported, new whenever another collection starts, and
    /// the id of the current track's play, new for every track. Spotify's apps name both, and
    /// listeners of a Jam tell a new track from a seek by them.
    run: String,
    play: String,
    /// The uids Spotify itself gives the tracks of the collection being played. Its apps find a
    /// track in a collection by them, so a track reported under any other uid is one they cannot
    /// place, and a listener of a Jam then plays the collection from its top.
    uids: Uids,
    /// Where a collection's uids come back to once they are read, and where they are taken from.
    learned: UnboundedSender<(String, Option<Listed>)>,
    learning: UnboundedReceiver<(String, Option<Listed>)>,
    /// Whether this device hosts a Jam that others listen to, and how many are in it.
    hosting: bool,
    members: usize,
    /// Until when the song playing is told as one Spotify picked rather than one queued by hand.
    /// An app that joins a Jam while a queued song plays cannot place it: it starts the next song
    /// of the collection, or the queued one from its top, and stays apart until the song ends.
    /// Told as a pick, the song is taken as it is, and telling it as queued again afterwards
    /// leaves everyone in step with the right songs to follow.
    welcome: Option<Instant>,
    /// When the song the Jam hears ends while this device is paused apart from it, and until
    /// when the skip that follows it there may be seen playing before its pause lands.
    advance_at: Option<Instant>,
    advancing: Option<Instant>,
    /// What this device really plays while it is paused on its own in the Jam it hosts. A pause
    /// in a Jam is the listener's own: the others' music goes on, and Spotify hands the lead to
    /// whoever still plays once it hears of the pause, after which nothing this device does
    /// reaches them. So the pause is kept from Spotify, `now` goes on saying what the Jam hears,
    /// and resuming catches up with it.
    apart: Option<Arc<Snapshot>>,
    active_since: Option<SystemTime>,
    claimed_at: Option<Instant>,
    /// The id and sender of the last command handled, which the next state echoes back.
    last_command: (u32, String),
    /// When that command came in. A state put since then has answered it already.
    asked_at: Option<Instant>,
    ack_at: Option<Instant>,
    rename_at: Option<Instant>,
    /// The device that has playback on the account, as the last cluster said.
    active_device: String,
    /// Last queue published by another active device, retained across a transfer here.
    remote_queue: Option<(Arc<Snapshot>, Instant)>,
    smart: Option<Enhanced>,
    /// Whether the listener switched smart shuffle on for this device. A phone sets it once and
    /// then sends plain play commands, which this mode turns into smart shuffle ones.
    smart_mode: bool,
    /// Counts the commands that change what plays, so a read made for an older one is dropped.
    ticket: u64,
    /// Whether a play command waits for its smart shuffle read. A switch of the mode meanwhile
    /// leaves the read alone, and the play follows the mode when the read lands.
    awaiting: bool,
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
        let (learned, learning) = unbounded_channel();
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
            run: String::new(),
            play: String::new(),
            uids: Uids::default(),
            learned,
            learning,
            hosting: false,
            members: 0,
            welcome: None,
            advance_at: None,
            advancing: None,
            apart: None,
            active_since: None,
            claimed_at: None,
            last_command: (0, String::new()),
            asked_at: None,
            ack_at: None,
            rename_at: None,
            active_device: String::new(),
            remote_queue: None,
            smart: None,
            smart_mode: false,
            ticket: 0,
            awaiting: false,
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
                    Some(Ok(Incoming::Enhanced(read))) => self.enhanced_read(*read),
                    Some(Ok(Incoming::Changes(changes))) => {
                        for event in changes { self.out.send(event).ok(); }
                    }
                    Some(Err(error)) => log::warn!("connect: bad dealer update: {error}"),
                    None => {}
                },
                Some((context, tracks)) = self.learning.recv() => self.learn(context, tracks).await,
                () = acknowledge(self.welcome) => {
                    self.welcome = None;
                    if self.now.is_some() {
                        self.want_put(PutStateReason::PLAYER_STATE_CHANGED).await;
                    }
                }
                () = acknowledge(self.advance_at) => self.advance(),
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
                    // the app usually acts at once, and the state it put then named the command
                    let heard = self
                        .asked_at
                        .zip(self.last_put)
                        .is_some_and(|(asked, put)| put >= asked);
                    if self.now.is_some() && !heard {
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
                self.tell_jam(Some(read.jam));
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
                self.tell_jam(Some(read.jam));
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
                self.tell_jam(None);
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
            self.tell_jam(jam);
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
                    self.tell_jam(Some(read.jam));
                }
            }
            Err(error) => log::warn!("connect: cannot remove the member: {error}"),
        }
    }

    /// Reads session updates without requiring the reason enum to be known to librespot.
    fn jam_update(&mut self, message: Message) {
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
            (true, _) => self.tell_jam(None),
            // an update with no session says nothing about who is in it
            (false, Some(jam)) => self.tell_jam(Some(jam)),
            (false, None) => {}
        }
    }

    /// Reports the Jam this device is in, and notes whether it hosts one that others listen to.
    /// Once it no longer does, a pause it kept to itself is told like any other.
    fn tell_jam(&mut self, jam: Option<Jam>) {
        let members = jam.as_ref().map_or(0, |jam| jam.members.len());
        let joined = std::mem::replace(&mut self.members, members) < members;
        self.hosting = jam.as_ref().is_some_and(|jam| jam.owner && members > 1);
        // while still hosting, the pause stays this device's own
        let paused = match self.hosting {
            true => None,
            false => self.apart.take(),
        };
        if let Some(paused) = paused {
            self.now = Some(paused);
            self.advance_at = None;
            self.advancing = None;
            self.hold(PutStateReason::PLAYER_STATE_CHANGED);
            self.flush_at.get_or_insert_with(Instant::now);
        }
        if self.hosting && joined {
            self.welcome = Some(Instant::now() + WELCOME);
            self.hold(PutStateReason::PLAYER_STATE_CHANGED);
            self.flush_at.get_or_insert_with(Instant::now);
        }
        self.out.send(Event::Jam(jam)).ok();
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
                self.tell_jam(jam);
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
        let Some(now) = self.heard(now) else {
            return;
        };
        let before = std::mem::replace(&mut self.now, now);
        let moved = match (before.as_deref(), self.now.as_deref()) {
            (Some(before), Some(now)) => before.context != now.context,
            _ => false,
        };
        // a collection started here while a smart shuffle read waits overrides the command
        if self.awaiting && moved {
            self.ticket += 1;
            self.awaiting = false;
        }
        self.identify(before.as_deref());
        self.read_uids();
        self.sync(before).await;
    }

    /// What to tell Spotify plays when the device is in the state `now`, or `None` when there
    /// is nothing new to tell. It differs from `now` only while this device is paused on its own
    /// in a Jam it hosts: the Jam then hears the song go on, and a resume first moves the device
    /// to where the Jam has got to.
    fn heard(&mut self, now: Option<Arc<Snapshot>>) -> Option<Option<Arc<Snapshot>>> {
        let (Some(told), Some(real), true) = (self.now.clone(), now.clone(), self.hosting) else {
            self.apart = None;
            self.advance_at = None;
            self.advancing = None;
            return Some(now);
        };
        let same = told.track.as_ref().map(|track| &track.id)
            == real.track.as_ref().map(|track| &track.id);
        match (self.apart.is_some(), real.playing) {
            // a pause of a song the Jam hears is kept from it
            (false, false) if told.playing && same => {
                log::debug!("connect: pausing apart from the jam");
                self.apart = Some(real);
                self.advance_at = ends(&told);
                None
            }
            (false, _) => Some(now),
            // whatever else changed while paused is told, with the song still going on
            (true, false) => {
                let mut heard = (*real).clone();
                heard.playing = true;
                match same {
                    true => {
                        heard.position = told.position;
                        heard.stamp = told.stamp;
                    }
                    // another song starts for the Jam where it stands here
                    false => {
                        heard.stamp = Some(SystemTime::now());
                        self.advancing = None;
                    }
                }
                self.apart = Some(real);
                self.advance_at = ends(&heard);
                Some(Some(Arc::new(heard)))
            }
            // the song skipped to for the Jam plays for an instant before its pause lands
            (true, true) if self.advancing.is_some_and(|until| Instant::now() < until) => None,
            (true, true) => {
                self.apart = None;
                self.advance_at = None;
                self.advancing = None;
                if !same {
                    return Some(now);
                }
                let at = told.live_position();
                let over = !told.duration.is_zero() && at >= told.duration;
                let command = match over {
                    true => Command::Next,
                    false if at.abs_diff(real.live_position()) > JAM_SLACK => Command::Seek(at),
                    false => return Some(now),
                };
                log::debug!("connect: catching up with the jam at {at:?}");
                // the song is told again once the device is where the Jam is
                self.out.send(Event::Command(command)).ok();
                None
            }
        }
    }

    /// Reads the uids of the collection now playing, unless they are known or being read.
    fn read_uids(&mut self) {
        let Some(context) = self.now.as_deref().and_then(shared) else {
            return;
        };
        let uri = collection_uri(context, &self.session.username());
        if self.uids.context == uri {
            return;
        }
        self.uids = Uids {
            context: uri.clone(),
            ..Default::default()
        };
        let session = self.session.clone();
        let learned = self.learned.clone();
        self.session.spawn(async move {
            let tracks = listed_uids(&session, &uri)
                .await
                .inspect_err(|error| log::warn!("connect: cannot read the uids of {uri}: {error}"))
                .ok();
            learned.send((uri, tracks)).ok();
        });
    }

    /// Moves on to the next song without a sound, as the Jam does when the song it hears ends
    /// while this device is paused apart from it. The skip starts the song and the pause that
    /// follows stops it before anything is heard, and the Jam is told the song from its top.
    fn advance(&mut self) {
        self.advance_at = None;
        if self.apart.is_none() {
            return;
        }
        log::debug!("connect: moving on with the jam while paused");
        self.advancing = Some(Instant::now() + ADVANCING);
        self.out.send(Event::Command(Command::Next)).ok();
        self.out.send(Event::Command(Command::Pause)).ok();
    }

    /// Keeps the uids read for `context` and reports the queue again under them.
    async fn learn(&mut self, context: String, tracks: Option<Listed>) {
        if self.uids.context != context {
            return;
        }
        let Some(tracks) = tracks else {
            // forgetting the collection has the next report of it read the uids again
            self.uids = Uids::default();
            return;
        };
        log::debug!("connect: read {} uids of {context}", tracks.len());
        self.uids.tracks = tracks;
        self.uids.read = true;
        if self.now.is_some() {
            self.want_put(PutStateReason::PLAYER_STATE_CHANGED).await;
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
        let state = &request.device.player_state;
        log::debug!(
            "connect: putting {reason:?}, active {}, {} uid {:?} in {} at {:?}, run {}, play {}, {}ms, playing {}, paused {}, {} before, after: {}",
            request.is_active,
            state.track.uri,
            state.track.uid,
            state.context_uri,
            state.index.as_ref().map(|index| index.track),
            state.session_id,
            state.playback_id,
            state.position_as_of_timestamp,
            state.is_playing,
            state.is_paused,
            state.prev_tracks.len(),
            outline(&state.next_tracks),
        );
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
        let collection = shared(now);
        let uri = match collection {
            Some(context) => collection_uri(context, &self.session.username()),
            None => format!("{TRACK_PREFIX}{}", track.id),
        };
        let known = match collection.is_some() {
            true => self.uids.of(&uri),
            // a song that is its own collection has nothing to read, and its apps name it alike
            false => Known::unlisted(),
        };
        let mut current = provided(track, &uri, known.at(track, now.index));
        if track.queued && self.welcome.is_some() {
            current.provider = "autoplay".to_owned();
            current.metadata.remove("is_queued");
        }
        state.track = MessageField::some(current);
        state.prev_tracks = now
            .previous
            .iter()
            .map(|track| provided(track, &uri, known.at(track, None)))
            .collect();
        state.next_tracks = now
            .upcoming
            .iter()
            .map(|track| provided(track, &uri, known.at(track, None)))
            .collect();
        let smart = self.smart.as_ref().filter(|smart| smart.context == uri);
        if let Some(smart) = smart {
            let tracks = state.prev_tracks.iter_mut().chain(state.track.as_mut());
            for track in tracks.chain(&mut state.next_tracks) {
                let Some(id) = track_id(&track.uri) else {
                    continue;
                };
                if smart.recommended.contains(&id) && track.provider == "context" {
                    track.provider = RECOMMENDED.to_owned();
                }
                // a hand-queued song keeps its own uid, so two entries never share one
                let uid = smart.places.get(&id).map(|(_, uid)| uid);
                if let Some(uid) = uid.filter(|uid| !uid.is_empty() && track.provider != "queue") {
                    track.uid = uid.clone();
                }
            }
        }
        state.context_url = match smart {
            Some(_) => format!("context://{uri}{LENS}"),
            None => format!("context://{uri}"),
        };
        state.context_uri = uri;
        if !self.run.is_empty() {
            state.session_id = self.run.clone();
        }
        state.playback_id = self.play.clone();
        // a device taking over finds the track by where it sits as well as by its uri
        let index = match smart {
            Some(smart) => smart.places.get(&track.id).map(|(at, _)| *at),
            None => now.index.map(|at| at as u32),
        };
        state.index = MessageField::from_option(index.map(|track| ContextIndex {
            track,
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
            // a smart shuffle read still pending for a play here would claim playback back
            if self.awaiting {
                self.ticket += 1;
                self.awaiting = false;
            }
            self.out.send(Event::Command(Command::Released)).ok();
        }
        let roster = self.roster(cluster);
        self.out.send(Event::Roster(roster)).ok();
    }

    /// Uses a matching fresh player report to preserve queue order that transfer contexts can omit.
    fn transferred(&mut self, state: TransferState) -> Start {
        self.ticket += 1;
        self.awaiting = false;
        self.smart_mode = recommends(&state.options.modes);
        self.smart = transferred_smart(&state);
        let now =
            (unix_millis() as i64).saturating_add(self.session.time_delta().saturating_mul(1000));
        let start = marked(started_by_transfer(state, now), self.smart.as_ref());
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
        self.asked_at = Some(Instant::now());
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

    /// Starts what another device asked to play, through smart shuffle when the command or this
    /// device's mode asks for it. A command that names the mode sets it for this device.
    fn play(&mut self, play: PlayCommand) {
        self.ticket += 1;
        if let Some(on) = asked_mode(&play) {
            self.smart_mode = on;
        }
        let uri = smart_shuffled(&play, self.smart_mode);
        self.awaiting = uri.is_some();
        match uri {
            Some(uri) => self.read_enhanced(uri, Some(play)),
            None => {
                self.smart = None;
                self.play_plain(play);
            }
        }
    }

    /// Starts a play command from the pages it carries. A phone names the song by its uid alone,
    /// and a jam names a context Sonora cannot read itself. Both send no tracks along, so the
    /// context is read back first to find the song and what follows it.
    fn play_plain(&self, mut play: PlayCommand) {
        let uri = play.context.uri.clone();
        let out = self.out.clone();
        let session = self.session.clone();
        let blind = play.context.pages.iter().all(|page| page.tracks.is_empty());
        let by_uid = play.options.skip_to.as_ref().is_some_and(|skip| {
            skip.track_uri.as_deref().is_none_or(str::is_empty)
                && skip.track_uid.as_deref().is_some_and(|uid| !uid.is_empty())
        });
        let (Some(uri), true) = (uri, blind && (by_uid || collection_is_unread(&play))) else {
            out.send(Event::Command(Command::Start(started_by_play(&play, None))))
                .ok();
            return;
        };
        self.session.spawn(async move {
            match session.spclient().get_context(&uri).await {
                Ok(context) => play.context.pages = context.pages,
                Err(error) => log::warn!("connect: cannot read {uri} back: {error}"),
            }
            out.send(Event::Command(Command::Start(started_by_play(&play, None))))
                .ok();
        });
    }

    /// Switches smart shuffle for what plays now. The controller names the collection with the
    /// lens to switch it on, and without it to switch it off.
    fn update_context(&mut self, context: Context) {
        let Some(now) = &self.now else {
            return;
        };
        if context.uri != self.playing(now) {
            return log::debug!("connect: a context update names another collection");
        }
        self.switch_smart(context.url.is_some_and(|url| url.ends_with(LENS)));
    }

    /// Switches smart shuffle for this device, and for the playlist that plays now.
    fn switch_smart(&mut self, on: bool) {
        self.smart_mode = on;
        if self.awaiting {
            return;
        }
        let Some(now) = self.now.clone() else {
            return;
        };
        let Some(uri) = self
            .playing(&now)
            .filter(|uri| uri.starts_with(PLAYLIST_PREFIX))
        else {
            return;
        };
        if on
            && self
                .smart
                .as_ref()
                .is_some_and(|smart| smart.context == uri)
        {
            return;
        }
        self.ticket += 1;
        if on {
            return self.read_enhanced(uri, None);
        }
        let Some(smart) = self.smart.take() else {
            return;
        };
        log::debug!("connect: smart shuffle off");
        let upcoming = now
            .upcoming
            .iter()
            .filter(|track| track.queued || !smart.recommended.contains(&track.id))
            .cloned()
            .collect();
        self.out
            .send(Event::Command(Command::SetQueue { upcoming }))
            .ok();
    }

    /// Reads the smart shuffle collection off the worker and hands it back through the
    /// subscriptions, tagged with the ticket of the command it serves.
    fn read_enhanced(&mut self, uri: String, play: Option<PlayCommand>) {
        let ticket = self.ticket;
        let session = self.session.clone();
        let read = async move {
            let read = enhanced(&session, &uri).await;
            Ok::<_, Error>(Incoming::Enhanced(Box::new(EnhancedRead {
                ticket,
                uri,
                play,
                read,
            })))
        };
        self.streams.push(stream::once(read).boxed());
    }

    /// Applies a smart shuffle read, unless a newer command replaced the one it was made for.
    fn enhanced_read(&mut self, read: EnhancedRead) {
        let EnhancedRead {
            ticket,
            uri,
            play,
            read,
        } = read;
        if ticket != self.ticket {
            return log::debug!("connect: dropping a smart shuffle read a newer command replaced");
        }
        self.awaiting = false;
        let play = match (play, self.smart_mode) {
            (Some(play), false) => {
                self.smart = None;
                return self.play_plain(play);
            }
            (play, _) => play,
        };
        let (context, recommended) = match read {
            Ok(read) => read,
            Err(error) => {
                log::warn!(
                    "connect: cannot read {} enhanced: {error}",
                    music::redact(&uri)
                );
                self.smart = None;
                if let Some(play) = play {
                    self.play_plain(play);
                }
                return;
            }
        };
        let enhanced = Enhanced {
            context: uri.clone(),
            places: places(&context),
            recommended,
        };
        if let Some(play) = play {
            let play = enhanced_play(play, context.pages);
            let start = started_by_play(&play, Some(&enhanced.recommended));
            self.smart = Some(enhanced);
            self.out.send(Event::Command(Command::Start(start))).ok();
            return;
        }
        let Some(now) = self.now.clone() else {
            self.smart = Some(enhanced);
            return;
        };
        if self.playing(&now).as_ref() != Some(&uri) {
            self.smart = None;
            return;
        }
        let current = now.track.as_ref().map(|track| &track.id);
        let mut following: Vec<_> = ids(&context).filter(|id| Some(id) != current).collect();
        fastrand::shuffle(&mut following);
        let upcoming = now
            .upcoming
            .iter()
            .filter(|track| track.queued)
            .cloned()
            .chain(picks(following, &enhanced.recommended))
            .collect();
        self.smart = Some(enhanced);
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

    /// The uri of the collection that plays now, as Spotify names it.
    fn playing(&self, now: &Snapshot) -> Option<String> {
        now.context
            .as_ref()
            .map(|context| collection_uri(context, &self.session.username()))
    }

    /// What a command from Spotify asks of the app, or `None` for one this device does not
    /// know.
    fn translate(&mut self, command: Wire) -> Option<Vec<Command>> {
        Some(match command {
            Wire::Transfer(transfer) => vec![Command::Start(self.transferred(transfer.data?))],
            Wire::Play(play) => vec![Command::Start(started_by_play(&play, None))],
            Wire::Pause(_) => vec![Command::Pause],
            Wire::Resume(_) => vec![Command::Play],
            Wire::SkipNext(_) => vec![Command::Next],
            Wire::SkipPrev(_) => vec![Command::Previous],
            Wire::SeekTo(seek) => vec![Command::Seek(Duration::from_millis(seek.value.into()))],
            Wire::SetShufflingContext(set) => {
                // smart shuffle is a kind of shuffle, so shuffle off switches it off too
                if !set.value {
                    self.switch_smart(false);
                }
                vec![Command::Shuffle(set.value)]
            }
            Wire::SetRepeatingContext(set) => {
                vec![Command::Repeat(self.repeat(Some(set.value), None))]
            }
            Wire::SetRepeatingTrack(set) => {
                vec![Command::Repeat(self.repeat(None, Some(set.value)))]
            }
            Wire::SetOptions(options) => {
                let mode = options.modes.as_ref().map(recommends);
                // smart shuffle is a kind of shuffle, so shuffle off switches it off too
                let unshuffled = options.shuffling_context == Some(false);
                if let Some(on) = mode
                    .map(|on| on && !unshuffled)
                    .or_else(|| unshuffled.then_some(false))
                {
                    self.switch_smart(on);
                }
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
                log::debug!(
                    "connect: queue edit, {} before, after: {}",
                    queue.prev_tracks.len(),
                    outline(&queue.next_tracks),
                );
                // an edit that added songs runs past the window it was shown, and all of it counts
                let mut upcoming = listed(&queue.next_tracks, queue.next_tracks.len());
                if let Some(now) = self.now.as_deref().filter(|now| shared(now).is_none()) {
                    keep_kinds(&mut upcoming, &now.upcoming);
                }
                // Acknowledge the edit with its new queue while the host resolves missing metadata.
                // The history an edit carries is left alone: a phone sends its own or none at
                // all, and no app can edit it, so taking it would only wipe what played here.
                if let Some(now) = self.now.as_mut() {
                    Arc::make_mut(now).upcoming = upcoming.clone();
                }
                vec![Command::SetQueue { upcoming }]
            }
            Wire::UpdateContext(update) => {
                self.update_context(update.context);
                Vec::new()
            }
            Wire::Unknown(value) => {
                if let Some(state) = jam_transfer(&value) {
                    log::debug!("connect: a jam brings this device back in step");
                    let start = self.transferred(state);
                    return Some(self.in_step(start));
                }
                // the field names only, since a transfer carries the listener's whole state
                let fields = value
                    .as_object()
                    .map(|object| object.keys().collect::<Vec<_>>());
                log::debug!("connect: unsupported player command, it carries {fields:?}");
                return None;
            }
        })
    }

    /// What it takes to play as `start` says. A song that already plays here is not loaded again,
    /// which would be heard: repeat is taken, the queue after the song, and its place when that
    /// is audibly off. Another shuffle reorders the whole queue, more of it than a report
    /// shows, so that is started over like another song.
    fn in_step(&mut self, start: Start) -> Vec<Command> {
        let wanted = &start.state;
        let Some(now) = self.now.as_ref().filter(|now| {
            now.track.as_ref().map(|track| &track.id)
                == wanted.track.as_ref().map(|track| &track.id)
                && now.playing == wanted.playing
                && wanted
                    .shuffle
                    .is_none_or(|shuffle| now.shuffle == Some(shuffle))
        }) else {
            return vec![Command::Start(start)];
        };
        let mut commands = Vec::new();
        if let Some(repeat) = wanted.repeat.filter(|repeat| now.repeat != Some(*repeat)) {
            commands.push(Command::Repeat(repeat));
        }
        let mut upcoming = wanted.upcoming.clone();
        if shared(now).is_none() {
            keep_kinds(&mut upcoming, &now.upcoming);
        }
        let listed = |tracks: &[QueuedTrack]| -> Vec<(String, bool)> {
            tracks
                .iter()
                .map(|track| (track.id.clone(), track.queued))
                .collect()
        };
        if listed(&upcoming) != listed(&now.upcoming) {
            commands.push(Command::SetQueue {
                upcoming: upcoming.clone(),
            });
        }
        let at = wanted.live_position();
        if at.abs_diff(now.live_position()) > JAM_SLACK {
            commands.push(Command::Seek(at));
        }
        if let Some(now) = self.now.as_mut() {
            Arc::make_mut(now).upcoming = upcoming;
        }
        commands
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
    let short = music::link::address(link)
        .is_some_and(|url| url.scheme() == "https" && url.host_str() == Some("spotify.link"));
    if !short {
        return token_in(link);
    }
    // a short link sometimes answers with a page that names the session instead of redirecting
    let followed = music::link::follow(link).await?;
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
    let mut info = DeviceInfo {
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
    };
    info.capabilities
        .mut_or_insert_default()
        .special_fields
        .mut_unknown_fields()
        .add_varint(SMART_SHUFFLE_CAPABILITY, 1);
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
            let kind = match seed {
                RadioSeed::Album => "album",
                RadioSeed::Playlist => "playlist",
                RadioSeed::Artist => "artist",
            };
            format!("spotify:station:{kind}:{id}")
        }
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

/// The uri and the uid of each track a collection lists, in its order.
type Listed = Vec<(String, String)>;

/// The uids Spotify gives the tracks of one collection, in the order it lists them.
#[derive(Default)]
struct Uids {
    /// The uri of the collection, empty before any was played.
    context: String,
    /// Each listed track's uri and uid. Empty until they are read, and it stays so for a
    /// collection that lists its tracks without uids, as an album does.
    tracks: Listed,
    /// Whether the collection was read, which tells one without uids from one not read yet.
    read: bool,
}

impl Uids {
    /// The uids as they hold for `context`, none when another collection is the one known.
    fn of(&self, context: &str) -> Known<'_> {
        match self.context == context {
            true => Known {
                listed: &self.tracks,
                derived: self.read && self.tracks.is_empty(),
            },
            false => Known {
                listed: &[],
                derived: false,
            },
        }
    }
}

/// The uids of the collection being reported.
#[derive(Clone, Copy)]
struct Known<'a> {
    listed: &'a [(String, String)],
    /// Whether the collection lists its tracks without uids, so that Spotify's apps make each
    /// one's up from its uri, and this app has to name the tracks the same way.
    derived: bool,
}

impl Known<'_> {
    /// The uids of a collection that lists none.
    fn unlisted() -> Self {
        Self {
            listed: &[],
            derived: true,
        }
    }

    /// The uid Spotify's apps know `track` by in the collection: the one listed at `index` when
    /// the track sits there, or else that of its first listing. A track the collection lists
    /// under no uid goes by the one made from its uri, and one not found in a collection that
    /// has uids, or in one not read yet, goes without, which leaves the apps to find it by uri.
    fn at(self, track: &QueuedTrack, index: Option<usize>) -> String {
        if track.queued {
            return track.uid.clone();
        }
        let uri = format!("{TRACK_PREFIX}{}", track.id);
        let listed = index
            .and_then(|index| self.listed.get(index))
            .filter(|(listed, _)| *listed == uri)
            .or_else(|| self.listed.iter().find(|(listed, _)| *listed == uri));
        match (listed, self.derived) {
            (Some((_, uid)), _) => uid.clone(),
            (None, true) => derived_uid(&uri),
            (None, false) => String::new(),
        }
    }
}

/// The uid Spotify's apps give a track listed without one: the start of the SHA-1 of its uri.
fn derived_uid(uri: &str) -> String {
    let digest = Sha1::digest(uri.as_bytes());
    let mut uid: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    uid.truncate(DERIVED_UID);
    uid
}

/// The uri and uid of every track `uri` lists that has a uid, in the order it lists them.
async fn listed_uids(session: &Session, uri: &str) -> Result<Listed, Error> {
    let mut context = session.spclient().get_context(uri).await?;
    read_pages(session, &mut context).await?;
    let mut tracks: Listed = context
        .pages
        .iter()
        .flat_map(|page| &page.tracks)
        .filter_map(|track| Some((track.uri.clone()?, track.uid.clone()?)))
        .collect();
    tracks.retain(|(_, uid)| !uid.is_empty());
    log::debug!(
        "connect: {uri} lists {} uids over {} pages",
        tracks.len(),
        context.pages.len()
    );
    Ok(tracks)
}

/// Reads the songs of a collection that its pages only link to, up to `UID_PAGES` reads. A page
/// with no songs of its own says where they are, and a long collection names its next page.
async fn read_pages(session: &Session, context: &mut Context) -> Result<(), Error> {
    let mut reads = 0;
    for page in &mut context.pages {
        let Some(url) = page
            .page_url
            .clone()
            .filter(|url| !url.is_empty() && page.tracks.is_empty())
        else {
            continue;
        };
        if reads == UID_PAGES {
            return Ok(());
        }
        reads += 1;
        let (tracks, next) = linked_page(&session.spclient().get_next_page(&url).await?);
        page.tracks = tracks;
        page.next_page_url = page
            .next_page_url
            .take()
            .filter(|url| !url.is_empty())
            .or(next);
    }
    let mut next = context
        .pages
        .last()
        .and_then(|page| page.next_page_url.clone());
    while let Some(url) = next
        .take()
        .filter(|url| !url.is_empty() && reads < UID_PAGES)
    {
        reads += 1;
        let (tracks, more) = linked_page(&session.spclient().get_next_page(&url).await?);
        context.pages.push(ContextPage {
            tracks,
            ..Default::default()
        });
        next = more;
    }
    Ok(())
}

/// The songs of a page read from its link, and the link to the page after it.
fn linked_page(body: &[u8]) -> (Vec<ContextTrack>, Option<String>) {
    let page: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let text = |value: &serde_json::Value, key: &str| {
        Some(value.get(key)?.as_str()?.to_owned()).filter(|text| !text.is_empty())
    };
    let tracks = page
        .get("tracks")
        .and_then(|tracks| tracks.as_array())
        .into_iter()
        .flatten()
        .map(|track| ContextTrack {
            uri: text(track, "uri"),
            uid: text(track, "uid"),
            metadata: track
                .get("metadata")
                .and_then(|metadata| metadata.as_object())
                .into_iter()
                .flatten()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                .collect(),
            ..Default::default()
        })
        .collect();
    let next = text(&page, "next_page_url").or_else(|| text(&page, "nextPageUrl"));
    (tracks, next)
}

/// When the song `told` to the Jam plays out, when its length is known.
fn ends(told: &Snapshot) -> Option<Instant> {
    let left = told.duration.checked_sub(told.live_position())?;
    (!told.duration.is_zero()).then(|| Instant::now() + left)
}

/// The state of a transfer librespot could not read. A Jam brings a listener who paused back in
/// step with a transfer that leaves out the logging details librespot insists on, so the command
/// arrives as one it does not know.
fn jam_transfer(command: &serde_json::Value) -> Option<TransferState> {
    if command.get("endpoint")?.as_str()? != "transfer" {
        return None;
    }
    let data = STANDARD.decode(command.get("data")?.as_str()?).ok()?;
    TransferState::parse_from_bytes(&data).ok()
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
            track.recommended = known.recommended;
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
        // what Spotify plays on by itself when a collection runs out
        suggested: track.provider == "autoplay",
        added_by: adder(track),
        recommended: track.provider == RECOMMENDED,
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

/// How many tracks a queue holds and the first of them, each as where it comes from, the start
/// of its id and its uid, so a log shows what an edit moved without listing a whole playlist.
fn outline(tracks: &[ProvidedTrack]) -> String {
    let shown = tracks
        .iter()
        .take(OUTLINED)
        .map(|track| {
            let id = track.uri.rsplit(':').next().unwrap_or_default();
            let id = id.get(..6).unwrap_or(id);
            format!("{}/{id}/{}", track.provider, track.uid)
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} [{shown}]", tracks.len())
}

/// The metadata keys a track carries, sorted, so a log shows what Spotify sends without the values.
fn keys(track: &ProvidedTrack) -> Vec<&String> {
    let mut keys = track.metadata.keys().collect::<Vec<_>>();
    keys.sort();
    keys
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

/// Marks the songs of a transferred start that smart shuffle recommended, since a transfer lists
/// its songs without the provider that says so.
fn marked(start: Start, smart: Option<&Enhanced>) -> Start {
    let Some(smart) = smart else {
        return start;
    };
    let mut state = (*start.state).clone();
    let tracks = state.previous.iter_mut().chain(state.track.as_mut());
    for track in tracks.chain(&mut state.upcoming) {
        track.recommended |= !track.queued && smart.recommended.contains(&track.id);
    }
    Start {
        state: Arc::new(state),
        ..start
    }
}

/// The smart shuffle collection as queue occurrences, each marked when Spotify recommended it.
fn picks(ids: Vec<String>, recommended: &HashSet<String>) -> Vec<QueuedTrack> {
    ids.into_iter()
        .map(|id| QueuedTrack {
            recommended: recommended.contains(&id),
            ..QueuedTrack::new(id, false)
        })
        .collect()
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
/// The song a play command names, resolved in the same order librespot resolves a skip in: uri,
/// then uid, then place in the pages it carries.
fn chosen(play: &PlayCommand) -> Option<String> {
    let skip = play.options.skip_to.as_ref();
    let mut tracks = play.context.pages.iter().flat_map(|page| &page.tracks);
    skip.and_then(|skip| skip.track_uri.as_deref())
        .and_then(track_id)
        .or_else(|| {
            let uid = skip?.track_uid.as_deref().filter(|uid| !uid.is_empty())?;
            tracks
                .clone()
                .find(|track| track.uid.as_deref() == Some(uid))
                .and_then(listed_id)
        })
        .or_else(|| {
            let index = skip.and_then(|skip| skip.track_index)? as usize;
            // the place counts every entry of the page, including ones that name no track
            tracks.nth(index).and_then(listed_id)
        })
}

/// `play` over the smart shuffle collection's `pages`. The song it names is pinned by uri first,
/// since a place counts in the plain pages, which the smart shuffle ones reorder.
fn enhanced_play(mut play: PlayCommand, pages: Vec<ContextPage>) -> PlayCommand {
    if let (Some(id), Some(skip)) = (chosen(&play), play.options.skip_to.as_mut()) {
        skip.track_uri = Some(format!("{TRACK_PREFIX}{id}"));
    }
    play.context.pages = pages;
    play
}

/// `enhanced` holds the recommendations when the pages are the smart shuffle collection, which this
/// app shuffles itself and hands over as the queue.
fn started_by_play(play: &PlayCommand, enhanced: Option<&HashSet<String>>) -> Start {
    let tracks: Vec<_> = play
        .context
        .pages
        .iter()
        .flat_map(|page| &page.tracks)
        .collect();
    let listed: Vec<_> = tracks.iter().filter_map(|track| listed_id(track)).collect();

    let skip = play.options.skip_to.as_ref();
    log::debug!(
        "connect: play in {:?}, skip to {skip:?}, {} pages with {} tracks",
        play.context.uri.as_deref(),
        play.context.pages.len(),
        listed.len(),
    );
    let track = chosen(play);
    let index = skip
        .and_then(|skip| skip.track_index)
        .map(|index| index as usize);
    let (track, upcoming) = match enhanced {
        Some(recommended) => {
            let mut rest: Vec<_> = listed
                .into_iter()
                .filter(|id| Some(id) != track.as_ref())
                .collect();
            fastrand::shuffle(&mut rest);
            // with no song named, a shuffle starts anywhere rather than on the first song
            let track = track.or_else(|| rest.pop());
            // the whole collection, uncapped, so the recommendations reach past the first songs
            (track, picks(rest, recommended))
        }
        None => {
            let upcoming = occurrences(
                listed
                    .into_iter()
                    .skip_while(|id| Some(id) != track.as_ref())
                    .skip(1)
                    .take(CARRIED),
                false,
            );
            (track, upcoming)
        }
    };

    Start {
        state: Arc::new(Snapshot {
            context: play.context.uri.as_deref().and_then(collection),
            track: track.map(|id| QueuedTrack {
                recommended: enhanced.is_some_and(|recommended| recommended.contains(&id)),
                ..QueuedTrack::new(id, false)
            }),
            index,
            upcoming,
            position: Duration::from_millis(play.options.seek_to.unwrap_or_default().into()),
            playing: !play.options.initially_paused.unwrap_or_default(),
            shuffle: match enhanced {
                Some(_) => Some(true),
                None => play
                    .options
                    .player_options_override
                    .as_ref()
                    .and_then(|options| options.shuffling_context),
            },
            ..Default::default()
        }),
        preserve_queue: enhanced.is_some(),
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

/// The playlist a play command starts with smart shuffle, when `mode`, this device's mode once
/// the command set it, is on.
fn smart_shuffled(play: &PlayCommand, mode: bool) -> Option<String> {
    // shortcut: playlists only, as smart shuffle is offered there; extend once albums show it
    play.context
        .uri
        .clone()
        .filter(|uri| mode && uri.starts_with(PLAYLIST_PREFIX))
}

/// The smart shuffle mode a play command names: on through the lens or its options, off through
/// options that ask for no shuffle or name other modes, or `None` when it says nothing.
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

/// Whether player modes say smart shuffle is on.
fn recommends(modes: &HashMap<String, String>) -> bool {
    modes.get(ENHANCEMENT).map(String::as_str) == Some(RECOMMENDATION)
}

/// Smart shuffle as a transfer hands it over, with the songs it already mixed in.
fn transferred_smart(state: &TransferState) -> Option<Enhanced> {
    let on = recommends(&state.options.modes);
    let context = state.current_session.context.uri.clone().filter(|_| on)?;
    let recommended = recommended(&state.current_session.context).collect();
    Some(Enhanced {
        context,
        recommended,
        places: places(&state.current_session.context),
    })
}

/// Reads a playlist with smart shuffle's recommendations mixed in, and which songs those are.
async fn enhanced(session: &Session, uri: &str) -> Result<(Context, HashSet<String>), Error> {
    let spclient = session.spclient();
    let mut context = spclient.get_context(&format!("{uri}{LENS}")).await?;
    read_pages(session, &mut context).await?;
    let mut recommended: HashSet<String> = recommended(&context).collect();
    // without a provider on the songs, the recommendations are what the plain playlist lacks
    let plain = match recommended.is_empty() {
        true => {
            let mut plain = spclient.get_context(uri).await?;
            read_pages(session, &mut plain).await?;
            Some(plain)
        }
        false => None,
    };
    // a page that only links to its songs would make its songs look recommended
    if let Some(plain) =
        plain.filter(|plain| plain.pages.iter().all(|page| !page.tracks.is_empty()))
    {
        let plain: HashSet<String> = ids(&plain).collect();
        recommended = ids(&context).filter(|id| !plain.contains(id)).collect();
    }
    log::debug!(
        "connect: smart shuffle on, {} recommended in {}",
        recommended.len(),
        music::redact(uri)
    );
    Ok((context, recommended))
}

/// The songs of a collection's loaded pages, in order. A page that only links to its songs is
/// left out.
fn ids(context: &Context) -> impl Iterator<Item = String> + '_ {
    context
        .pages
        .iter()
        .flat_map(|page| &page.tracks)
        .filter_map(listed_id)
}

/// Where each song first sits in a collection, counting every entry, and its uid there.
fn places(context: &Context) -> HashMap<String, (u32, String)> {
    let mut places = HashMap::new();
    let tracks = context.pages.iter().flat_map(|page| &page.tracks);
    for (at, track) in tracks.enumerate() {
        if let Some(id) = listed_id(track) {
            let uid = track.uid.clone().unwrap_or_default();
            places.entry(id).or_insert((at as u32, uid));
        }
    }
    places
}

fn recommended(context: &Context) -> impl Iterator<Item = String> + '_ {
    context
        .pages
        .iter()
        .flat_map(|page| &page.tracks)
        .filter(|track| track.metadata.get("provider").map(String::as_str) == Some(RECOMMENDED))
        .filter_map(listed_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(ids: &[&str]) -> ContextPage {
        ContextPage {
            tracks: ids
                .iter()
                .map(|id| ContextTrack {
                    uri: Some(format!("{TRACK_PREFIX}{id}")),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_place_keeps_its_song_over_the_smart_shuffle_pages() {
        let play = PlayCommand {
            context: Context {
                uri: Some("spotify:playlist:p".to_owned()),
                pages: vec![page(&["a", "b"])],
                ..Default::default()
            },
            play_origin: PlayOrigin::default(),
            options: serde_json::from_value(serde_json::json!({ "skip_to": { "track_index": 1 } }))
                .unwrap(),
            logging_params: serde_json::from_value(serde_json::json!({})).unwrap(),
        };
        let recommended = HashSet::from(["r".to_owned()]);
        let play = enhanced_play(play, vec![page(&["a", "r", "b"])]);
        let start = started_by_play(&play, Some(&recommended));
        let track = start.state.track.as_ref().map(|track| track.id.as_str());
        assert_eq!(track, Some("b"));
    }

    #[test]
    fn a_linked_page_keeps_who_provided_each_song() {
        let body = serde_json::json!({
            "tracks": [
                { "uri": "spotify:track:a", "uid": "u1" },
                { "uri": "spotify:track:r", "metadata": { "provider": RECOMMENDED } },
            ],
            "next_page_url": "hm://next",
        });
        let (tracks, next) = linked_page(body.to_string().as_bytes());
        let context = Context {
            pages: vec![ContextPage {
                tracks,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(ids(&context).collect::<Vec<_>>(), ["a", "r"]);
        assert_eq!(recommended(&context).collect::<Vec<_>>(), ["r"]);
        assert_eq!(next.as_deref(), Some("hm://next"));
    }
}
