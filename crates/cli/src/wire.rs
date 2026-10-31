use std::fmt;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::commands::{Kind, Repeat, Request};

/// The protocol both ends speak. Bump it on any change an older Sonora would misread.
pub const VERSION: u32 = 1;

/// One line on the socket: the protocol version and the request or reply it carries. The body
/// is decoded only once the version matches, so a newer request never half-parses into a wrong
/// command.
#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    v: u32,
    body: T,
}

/// What the running Sonora answers to a request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reply {
    /// The command was carried out and has nothing to report.
    Done,
    /// The value a query asked for.
    Value(Value),
    /// The command was refused, and why.
    Failed(Failure),
}

/// The answer to a command that reads state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Value {
    Current(Option<Current>),
    Volume(u8),
    Shuffle(bool),
    Repeat(Repeat),
    Providers(Vec<Provider>),
    /// The current provider's slug, `None` while none is chosen.
    Provider(Option<String>),
    /// What a search found, best first.
    Hits(Vec<Hit>),
}

/// A search hit as `sonora search` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub kind: Kind,
    pub title: String,
    /// The artists, or a playlist's owner. Empty for an artist.
    pub by: String,
    /// In whole seconds, for a song.
    pub duration: Option<u64>,
    /// What `sonora add` and `sonora play-next` take.
    pub link: String,
}

/// A provider as `sonora providers` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// What `sonora provider` takes to switch to it.
    pub slug: String,
    /// Whether an account, or a guest session, is stored to switch to.
    pub stored: bool,
    pub current: bool,
}

/// What plays now, as `sonora current` prints it. This is a public contract, so a field is
/// only ever added, never renamed or removed, without bumping `VERSION`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Current {
    pub state: State,
    pub track: Track,
    /// Whole seconds into the track.
    pub position: u64,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// The provider the track comes from, as `sonora providers` lists it, `local` for a file.
    /// `None` while no provider is chosen.
    #[serde(default)]
    pub provider: Option<String>,
    /// Whether the track is saved to the library.
    #[serde(default)]
    pub liked: bool,
    /// The collection the track was queued from, `None` when it was queued on its own.
    #[serde(default)]
    pub origin: Option<Origin>,
}

/// A collection a track plays from, as `sonora current` prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub kind: OriginKind,
    /// The provider's id for it, empty for liked songs.
    pub id: String,
    /// What the app calls it, when it knows.
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OriginKind {
    Album,
    Playlist,
    /// An artist's top tracks.
    Artist,
    Radio,
    /// The liked songs of the library.
    Liked,
    /// The local library.
    Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Playing,
    Paused,
    Loading,
    Idle,
}

/// A track as the CLI shows it, kept apart from `music::Track` so its internals never leak
/// into the contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub cover: Option<String>,
    /// In whole seconds, which is as fine as most providers report it.
    pub duration: u64,
}

/// Why the running Sonora refused a request. Each is turned into a Fluent message on the
/// side that prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Failure {
    /// The request spoke another protocol version; this is the one the running Sonora speaks.
    Version(u32),
    /// The request was not a request at all.
    Malformed,
    /// The command is known but not carried out yet.
    Unsupported,
    /// No provider is signed in to read a link with.
    SignedOut,
    /// A link belongs to another provider than the current one. Both are named.
    OtherProvider { link: String, current: String },
    /// No provider reads this link, or it points at nothing that can be queued.
    Unqueueable(String),
    /// A file to queue does not read as music.
    Unreadable(String),
    /// A link to queue could not be fetched from its provider.
    Unfetched(String),
    /// No provider goes by this slug.
    UnknownProvider(String),
    /// The provider, by name, has no account stored to switch to.
    NoAccount(String),
    /// The provider, by name, did not sign in after the switch.
    SignInFailed(String),
    /// A sign-in is already on its way, so the provider cannot change now.
    Busy,
    /// A search to queue the best hit of found nothing.
    NothingFound(String),
}

/// Why a line could not be read off the socket.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The other end closed without a line. An older Sonora that never answers looks like this.
    Closed,
    /// The line spoke this protocol version instead of ours.
    Version(u32),
    Malformed(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot use the socket: {error}"),
            Self::Closed => write!(f, "the other end closed without answering"),
            Self::Version(v) => write!(f, "the other end speaks protocol {v}, not {VERSION}"),
            Self::Malformed(error) => write!(f, "cannot decode the line: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Writes `body` as one line of JSON and flushes it, so the other end can read it whole.
pub fn send<T: Serialize>(writer: &mut impl Write, body: &T) -> Result<(), Error> {
    let mut line = serde_json::to_vec(&Envelope { v: VERSION, body }).map_err(Error::Malformed)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()?;
    Ok(())
}

/// Reads one line and decodes its body, refusing a line of another protocol version.
pub fn receive<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T, Error> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(Error::Closed);
    }
    let envelope: Envelope<Box<RawValue>> =
        serde_json::from_str(&line).map_err(Error::Malformed)?;
    if envelope.v != VERSION {
        return Err(Error::Version(envelope.v));
    }
    serde_json::from_str(envelope.body.get()).map_err(Error::Malformed)
}

/// Sends `request` and waits for the reply, as the short-lived process does.
pub fn exchange<S: Read + Write>(stream: &mut S, request: &Request) -> Result<Reply, Error> {
    send(stream, request)?;
    receive(&mut BufReader::new(stream)).map_err(|error| match error {
        Error::Io(error) if error.kind() == ErrorKind::UnexpectedEof => Error::Closed,
        error => error,
    })
}
