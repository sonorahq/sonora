//! A provider's remote-control surface: what the app reports to the service as playing, what
//! the service's other apps ask of it, and the account's other devices. Nothing here names a
//! provider, so `state` and `views` stay on the trait while one module implements it.

use std::time::{Duration, SystemTime};

use tokio::sync::mpsc::UnboundedReceiver;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
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

/// What the app is playing, in the terms a remote end can show. `track` and `upcoming` are
/// provider track ids.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub track: String,
    pub upcoming: Vec<String>,
    pub context: Option<Collection>,
    /// Where `track` sits in `context`, counted from 0, when that is known.
    pub index: Option<usize>,
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    /// From 0 to 1.
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

/// Where to begin when a remote end starts playback here, or moves it here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Start {
    pub collection: Option<Collection>,
    /// The track to open on, or the collection's first when there is none.
    pub track: Option<String>,
    /// Where the track to open on sits in `collection`, counted from 0, for when the remote end
    /// names it only by its place.
    pub index: Option<usize>,
    /// What follows `track` when there is no collection to read it from.
    pub upcoming: Vec<String>,
    pub position: Duration,
    pub paused: bool,
}

/// A request that crosses the wire in either direction: asked of this app by a remote end, or
/// asked of another device through `Connect::control`.
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
    Repeat(RepeatMode),
    /// Adds a track to the end of the hand-made queue.
    Enqueue(String),
    /// Plays a collection or a track list, from this app's side only.
    Start(Start),
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
    pub track: Option<String>,
    pub playing: bool,
    /// How far into the track the device was at `stamp`. It has moved on since if `playing`.
    pub position: Duration,
    pub stamp: SystemTime,
    pub duration: Duration,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl Elsewhere {
    /// How far into the track the device is now, never past its end.
    pub fn live_position(&self) -> Duration {
        let moved = match self.playing {
            true => self.stamp.elapsed().unwrap_or_default(),
            false => Duration::ZERO,
        };
        match self.duration.is_zero() {
            true => self.position + moved,
            false => (self.position + moved).min(self.duration),
        }
    }
}

/// The account's devices other than this app, and the one of them that has playback, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roster {
    pub devices: Vec<Device>,
    pub elsewhere: Option<Elsewhere>,
}

pub enum Event {
    Command(Command),
    Roster(Roster),
    /// One of the account's apps liked a track, or took the like back. `track` is a provider
    /// track id.
    Liked {
        track: String,
        liked: bool,
    },
}

/// The connection to a provider's device network. Every call is fire-and-forget: what comes
/// back arrives as an `Event`.
pub trait Connect: Send + Sync {
    /// Shows this app on the account's device list, or takes it off. Nothing is announced and no
    /// connection is made until the first `true`.
    fn enable(&self, on: bool);

    /// Changes the name the app goes by on the device list. A name typed in keystroke by keystroke
    /// is fine, since the provider waits for it to settle.
    fn rename(&self, naming: Naming);

    /// Reports what plays, or that nothing does. A report claims playback for this app, so only
    /// something the listener started should be sent.
    fn publish(&self, now: Option<NowPlaying>);

    /// The events from the network. Taken once; later calls get `None`.
    fn events(&self) -> Option<UnboundedReceiver<Event>>;

    /// The id this app goes by on the account's device list.
    fn device(&self) -> String;

    /// Moves playback to another device of the account.
    fn transfer(&self, to: &str);

    /// Sends a command to another device of the account. A `Start` it does not take comes back as
    /// `Event::Command(Command::Start)`, so it plays here instead.
    fn control(&self, device: &str, command: Command);
}
