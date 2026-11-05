//! A provider's remote-control surface: what the app reports to the service as playing, what
//! the service's other apps ask of it, and the account's other devices. Nothing here names a
//! provider, so `state` and `views` use the same channel while each provider owns its worker.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The collection a track plays from, as far as the remote end needs to know it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Collection {
    Album(String),
    Playlist(String),
    /// The listener's saved tracks.
    Saved,
    /// The radio of a track, which goes on with similar songs once the track ends.
    Radio(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// How the app is named on the account's device list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Naming {
    /// The app's name alone.
    App,
    /// The app's name and the computer's.
    #[default]
    AppOnComputer,
    /// The computer's name alone.
    Computer,
    /// A name the listener chose. An empty one falls back to the app's name and the computer's.
    Custom(String),
}

/// A particular occurrence of a provider track in the queue, stable as it moves through playback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedTrack {
    pub id: String,
    pub uid: String,
    pub queued: bool,
    pub added_by: Option<String>,
}

impl QueuedTrack {
    pub fn new(id: String, queued: bool) -> Self {
        Self {
            id,
            queued,
            uid: String::new(),
            added_by: None,
        }
    }
}

/// What the app is playing, including the recent and upcoming queue windows. Track IDs belong
/// to the provider; history is chronological and each occurrence keeps its own provenance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub track: Option<QueuedTrack>,
    /// Recent tracks in chronological order, with the most recent last.
    pub previous: Vec<QueuedTrack>,
    pub upcoming: Vec<QueuedTrack>,
    pub context: Option<Collection>,
    /// Where `track` sits in `context`, counted from 0, when that is known.
    pub index: Option<usize>,
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    /// From 0 to 1.
    pub volume: f32,
    pub shuffle: Option<bool>,
    pub repeat: Option<Repeat>,
    /// The wall-clock time at which `position` was measured, absent for play commands.
    pub stamp: Option<SystemTime>,
}

impl Snapshot {
    pub fn tracks(&self) -> impl Iterator<Item = &QueuedTrack> {
        self.track
            .iter()
            .chain(&self.upcoming)
            .chain(&self.previous)
    }

    /// Ignores a steadily advancing clock and identifies changes that need publishing.
    pub fn change(&self, before: &Self) -> Option<&'static str> {
        Some(match () {
            () if self.track != before.track => "track",
            () if self.previous != before.previous || self.upcoming != before.upcoming => "queue",
            () if self.context != before.context || self.index != before.index => "context",
            () if self.playing != before.playing => "play state",
            () if self.duration != before.duration => "duration",
            () if self.shuffle != before.shuffle => "shuffle",
            () if self.repeat != before.repeat => "repeat",
            () if self.position.abs_diff(before.live_position()) > Duration::from_secs(2) => {
                "position"
            }
            () if self.volume != before.volume => "volume",
            _ => return None,
        })
    }

    /// Extrapolates the published clock without mutating or periodically republishing the snapshot.
    pub fn live_position(&self) -> Duration {
        let moved = match self.playing {
            true => self
                .stamp
                .and_then(|stamp| stamp.elapsed().ok())
                .unwrap_or_default(),
            false => Duration::ZERO,
        };
        match self.duration.is_zero() {
            true => self.position + moved,
            false => (self.position + moved).min(self.duration),
        }
    }
}

/// A play request or exact transfer, both using the same playback snapshot as device reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Start {
    pub state: std::sync::Arc<Snapshot>,
    /// Uses the supplied history and upcoming order instead of reconstructing the collection.
    pub preserve_queue: bool,
    /// Starts playback here; a queue edit only replaces the play order.
    pub restart: bool,
}

/// A request that crosses the wire in either direction: asked of this app by a remote end, or
/// asked of another device through `Request::Control`.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Next,
    Previous,
    Seek(Duration),
    /// From 0 to 1.
    Volume(f32),
    Shuffle(bool),
    Repeat(Repeat),
    /// Adds a track to the end of the hand-made queue, with the id of whoever queued it when
    /// the provider says who that is. Sent to another device, `added_by` is left out.
    Enqueue {
        id: String,
        added_by: Option<String>,
    },
    /// Plays a collection or a track list, from this app's side only.
    Start(Start),
    /// Replaces the remote queue around the current track without restarting playback.
    SetQueue {
        previous: Vec<QueuedTrack>,
        upcoming: Vec<QueuedTrack>,
    },
    /// Another device took playback over, so this app has to let go of it.
    Released,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceKind {
    Computer,
    Phone,
    Tablet,
    Speaker,
    Tv,
    Console,
    Car,
    #[default]
    Other,
}

/// One of the account's devices.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// From 0 to 1.
    pub volume: f32,
}

/// Playback happening on a device other than this app.
#[derive(Clone, Debug, PartialEq)]
pub struct Elsewhere {
    pub device: Device,
    pub state: std::sync::Arc<Snapshot>,
}

/// The account's devices other than this app, and the one of them that has playback, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roster {
    pub devices: Vec<Device>,
    pub elsewhere: Option<Elsewhere>,
}

/// One person in a group session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JamMember {
    /// What the provider goes by to name the person, as removing them does.
    pub id: String,
    /// The account name, which songs they queue name them by when it differs from `id`.
    pub username: String,
    pub name: String,
    /// The address of their picture, when they have one.
    pub image: Option<String>,
    /// Whether they own the session.
    pub host: bool,
    /// Whether they are the listener.
    pub you: bool,
}

/// A group listening session the listener has joined, in which the host's playback plays here
/// too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Jam {
    /// Everyone in it, the listener too, in the order the provider lists them.
    pub members: Vec<JamMember>,
    /// Whether the listener owns the session, which leaving ends for everyone.
    pub owner: bool,
    /// The link that invites others, when the listener owns the session.
    pub link: Option<String>,
}

/// A group session a link names, read before the listener joins it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    /// The link to join it by, which needs no lookup of its own.
    pub link: String,
    pub jam: Jam,
}

pub enum Event {
    Command(Command),
    Roster(Roster),
    /// Saved collections or the account's playlist list changed in another client.
    LibraryChanged,
    /// A playlist's contents or attributes changed in another client.
    PlaylistChanged(String),
    /// One of the account's apps liked a track, or took the like back. `track` is a provider
    /// track id.
    Liked {
        track: String,
        liked: bool,
    },
    /// The group session changed: joined, updated, or `None` once it ended or was left.
    Jam(Option<Jam>),
    /// What `Request::Peek` found, or `None` when the link names no group session.
    Invite(Option<Invite>),
    /// Starting or joining a group session failed.
    JamFailed,
    /// Leaving or ending the group session failed. The session as it stands follows as a `Jam`.
    LeaveFailed,
}

/// Requests sent to the provider's worker; replies arrive through `Event`.
pub enum Request {
    Enable(bool),
    Rename(Naming),
    /// Publishing claims playback, so only listener-initiated playback should be reported.
    Publish(Option<std::sync::Arc<Snapshot>>),
    Transfer(String),
    /// A refused start returns as a command to play locally.
    Control(String, Command),
    Join(String),
    Peek(String),
    Start,
    Leave,
    Kick(String),
}

/// A provider-independent channel to the device worker. Dropping the last handle ends it.
pub struct Connect {
    device: String,
    requests: UnboundedSender<Request>,
    events: Mutex<Option<UnboundedReceiver<Event>>>,
}

impl Connect {
    /// Creates the app handle and the worker's request receiver and event sender.
    pub fn new(device: String) -> (Self, UnboundedReceiver<Request>, UnboundedSender<Event>) {
        let (requests, incoming) = unbounded_channel();
        let (outgoing, events) = unbounded_channel();
        (
            Self {
                device,
                requests,
                events: Mutex::new(Some(events)),
            },
            incoming,
            outgoing,
        )
    }

    pub fn send(&self, request: Request) {
        self.requests.send(request).ok();
    }

    /// Taken once by the state entity that owns the device list.
    pub fn events(&self) -> Option<UnboundedReceiver<Event>> {
        self.events.lock().ok()?.take()
    }

    pub fn device(&self) -> &str {
        &self.device
    }
}
