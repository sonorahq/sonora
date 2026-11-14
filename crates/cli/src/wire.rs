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
    /// The Sonora Session Sonora is in, `None` while it is in none.
    Session(Option<SessionInfo>),
    /// A read-only snapshot of the queue, as `sonora queue` prints it.
    Queue(QueueSnapshot),
    /// The lyrics of the current track as Sonora holds them, as `sonora lyrics` prints them.
    Lyrics(LyricsSnapshot),
}

/// At most this many lines of lyrics are reported, from the start, so a very long text never
/// makes an unbounded reply. `current` and `next` are picked before the cut.
pub const LYRICS_LIMIT: usize = 500;

/// The lyrics of the current track as `sonora lyrics` prints them. This is a public contract,
/// so a field is only ever added, never renamed or removed, without bumping `VERSION`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LyricsSnapshot {
    pub state: LyricsStatus,
    /// The name of the source the lyrics come from, `None` while there are none.
    pub source: Option<String>,
    pub instrumental: bool,
    /// Whether every line carries the time it starts at.
    pub synced: bool,
    /// The line sung now, `None` for plain lyrics and before a synced line has started.
    pub current: Option<LyricsLine>,
    /// The line that follows `current`, `None` for plain lyrics and after the last line.
    pub next: Option<LyricsLine>,
    /// At most `LYRICS_LIMIT` lines, in order.
    pub lines: Vec<LyricsLine>,
    /// Whether the lyrics held more than `LYRICS_LIMIT` lines.
    #[serde(default)]
    pub truncated: bool,
}

/// How far the lyrics of the current track have got.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LyricsStatus {
    /// Nothing is loaded, or the track has no id to look lyrics up by.
    Idle,
    /// Lyrics are turned off in the settings.
    Disabled,
    Loading,
    Ready,
    Instrumental,
    /// Every source answered and none had the lyrics.
    Missing,
    Failed,
}

/// One line of lyrics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsLine {
    /// Milliseconds into the track the line starts at, `None` for plain lyrics.
    pub start_ms: Option<u64>,
    /// Empty for a pause between verses.
    pub text: String,
    /// The line written in Latin letters, when its script has a romanization.
    pub romanized: Option<String>,
}

impl LyricsSnapshot {
    /// Lyrics that are not there, for `state` to say why.
    pub fn without(state: LyricsStatus) -> Self {
        Self {
            state,
            source: None,
            instrumental: state == LyricsStatus::Instrumental,
            synced: false,
            current: None,
            next: None,
            lines: Vec::new(),
            truncated: false,
        }
    }

    /// Keeps the first `LYRICS_LIMIT` of `lines`. `active` is the index of the line sung now in
    /// the whole of them, and only a synced text has `current` and `next`, the latter being the
    /// first line while none has started.
    pub fn bounded(
        mut self,
        synced: bool,
        mut lines: Vec<LyricsLine>,
        active: Option<usize>,
    ) -> Self {
        (self.current, self.next) = match synced {
            true => (
                active.and_then(|at| lines.get(at)).cloned(),
                lines.get(active.map_or(0, |at| at + 1)).cloned(),
            ),
            false => (None, None),
        };
        self.synced = synced;
        self.truncated = lines.len() > LYRICS_LIMIT;
        lines.truncate(LYRICS_LIMIT);
        self.lines = lines;
        self
    }
}

/// At most this many tracks are reported per section of a queue snapshot, so a very long
/// playlist never makes an unbounded reply. `previous` keeps its most recent tracks and
/// `upcoming` its next ones, both in play order; `truncated` says when either was cut.
pub const QUEUE_LIMIT: usize = 100;

/// The queue as `sonora queue` prints it: what played, what plays and what follows, in play
/// order. This is a public contract, so a field is only ever added, never renamed or
/// removed, without bumping `VERSION`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub previous: Vec<QueueEntry>,
    pub current: Option<QueueEntry>,
    pub upcoming: Vec<QueueEntry>,
    /// Whether `previous` or `upcoming` held more than `QUEUE_LIMIT` tracks.
    #[serde(default)]
    pub truncated: bool,
}

impl QueueSnapshot {
    /// Bounds a snapshot to `QUEUE_LIMIT` tracks per section, keeping the most recent
    /// previous tracks and the next upcoming ones, in play order.
    pub fn bounded(
        previous: Vec<QueueEntry>,
        current: Option<QueueEntry>,
        upcoming: Vec<QueueEntry>,
    ) -> Self {
        let truncated = previous.len() > QUEUE_LIMIT || upcoming.len() > QUEUE_LIMIT;
        let previous = match previous.len() > QUEUE_LIMIT {
            true => previous[previous.len() - QUEUE_LIMIT..].to_vec(),
            false => previous,
        };
        let upcoming = match upcoming.len() > QUEUE_LIMIT {
            true => upcoming[..QUEUE_LIMIT].to_vec(),
            false => upcoming,
        };
        Self {
            previous,
            current,
            upcoming,
            truncated,
        }
    }
}

/// One track in a queue snapshot, with why it is in the queue when that is known.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub track: Track,
    pub kind: QueueEntryKind,
}

/// Why a track is in the queue, as far as the snapshot knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueEntryKind {
    /// Part of the album, playlist or other collection being played.
    #[default]
    Context,
    /// Added by hand, after the current track or at the end of the queue.
    Queued,
    /// A recommendation that follows the queue.
    Suggested,
}

/// A Sonora Session as `sonora session` prints it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// The six digits friends join with.
    pub code: String,
    /// The slug of the provider the session plays from.
    pub provider: String,
    /// Whether this Sonora hosts the session.
    pub hosting: bool,
    /// Everyone in the session by name, the host first.
    pub members: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, kind: QueueEntryKind) -> QueueEntry {
        QueueEntry {
            track: Track {
                id: Some(title.to_owned()),
                title: title.to_owned(),
                artists: vec!["artist".to_owned()],
                album: "album".to_owned(),
                cover: None,
                duration: 60,
            },
            kind,
        }
    }

    #[test]
    fn an_empty_queue_snapshot_round_trips() {
        let reply = Reply::Value(Value::Queue(QueueSnapshot::default()));
        let mut line = Vec::new();
        send(&mut line, &reply).unwrap();
        let read: Reply = receive(&mut BufReader::new(line.as_slice())).unwrap();
        assert_eq!(read, reply);
        match read {
            Reply::Value(Value::Queue(queue)) => {
                assert!(queue.previous.is_empty());
                assert_eq!(queue.current, None);
                assert!(queue.upcoming.is_empty());
                assert!(!queue.truncated);
            }
            other => panic!("expected a queue snapshot, got {other:?}"),
        }
    }

    #[test]
    fn a_bounded_snapshot_keeps_order_and_the_ends_that_matter() {
        let previous: Vec<_> = (0..QUEUE_LIMIT + 5)
            .map(|at| entry(&at.to_string(), QueueEntryKind::Context))
            .collect();
        let upcoming: Vec<_> = (0..QUEUE_LIMIT + 5)
            .map(|at| entry(&at.to_string(), QueueEntryKind::Queued))
            .collect();
        let queue = QueueSnapshot::bounded(
            previous,
            Some(entry("now", QueueEntryKind::Context)),
            upcoming,
        );
        assert!(queue.truncated);
        assert_eq!(queue.previous.len(), QUEUE_LIMIT);
        assert_eq!(queue.previous.first().unwrap().track.title, "5");
        assert_eq!(queue.upcoming.len(), QUEUE_LIMIT);
        assert_eq!(queue.upcoming.last().unwrap().track.title, "99");
        assert_eq!(queue.current.unwrap().track.title, "now");
    }

    fn line(start_ms: Option<u64>, text: &str) -> LyricsLine {
        LyricsLine {
            start_ms,
            text: text.to_owned(),
            romanized: None,
        }
    }

    fn synced(count: u64) -> Vec<LyricsLine> {
        (0..count)
            .map(|at| line(Some(at * 1000), &at.to_string()))
            .collect()
    }

    fn snapshot(lines: Vec<LyricsLine>, active: Option<usize>) -> LyricsSnapshot {
        LyricsSnapshot::without(LyricsStatus::Ready).bounded(true, lines, active)
    }

    #[test]
    fn missing_lyrics_round_trip_empty() {
        let reply = Reply::Value(Value::Lyrics(LyricsSnapshot::without(
            LyricsStatus::Missing,
        )));
        let mut line = Vec::new();
        send(&mut line, &reply).unwrap();
        let read: Reply = receive(&mut BufReader::new(line.as_slice())).unwrap();
        assert_eq!(read, reply);
        let Reply::Value(Value::Lyrics(lyrics)) = read else {
            panic!("expected lyrics, got {read:?}");
        };
        assert_eq!(lyrics.state, LyricsStatus::Missing);
        assert!(lyrics.lines.is_empty() && lyrics.current.is_none() && lyrics.next.is_none());
    }

    #[test]
    fn lyrics_serialize_by_name_with_times_and_romanization() {
        let mut lines = synced(1);
        lines[0].romanized = Some("konnichiwa".to_owned());
        let printed = serde_json::to_string(&snapshot(lines, Some(0))).unwrap();
        assert!(printed.contains("\"state\":\"ready\""), "{printed}");
        assert!(printed.contains("\"start_ms\":0"), "{printed}");
        assert!(
            printed.contains("\"romanized\":\"konnichiwa\""),
            "{printed}"
        );
        let state = serde_json::to_string(&LyricsStatus::Instrumental).unwrap();
        assert_eq!(state, "\"instrumental\"");
    }

    #[test]
    fn the_active_line_has_the_next_one_after_it() {
        let lyrics = snapshot(synced(3), Some(1));
        assert_eq!(lyrics.current.unwrap().text, "1");
        assert_eq!(lyrics.next.unwrap().text, "2");
    }

    #[test]
    fn before_the_first_line_only_the_next_one_is_known() {
        let lyrics = snapshot(synced(3), None);
        assert_eq!(lyrics.current, None);
        assert_eq!(lyrics.next.unwrap().text, "0");
    }

    #[test]
    fn after_the_last_line_nothing_follows() {
        let lyrics = snapshot(synced(3), Some(2));
        assert_eq!(lyrics.current.unwrap().text, "2");
        assert_eq!(lyrics.next, None);
    }

    #[test]
    fn plain_lyrics_have_no_current_or_next_line() {
        let lines = vec![line(None, "one"), line(None, "two")];
        let lyrics = LyricsSnapshot::without(LyricsStatus::Ready).bounded(false, lines, None);
        assert!(!lyrics.synced);
        assert_eq!((lyrics.current, lyrics.next), (None, None));
        assert_eq!(lyrics.lines.len(), 2);
    }

    #[test]
    fn bounding_plain_lines_clears_a_synced_current_and_next() {
        let lines = vec![line(None, "one")];
        let lyrics = snapshot(synced(3), Some(1)).bounded(false, lines, None);
        assert!(!lyrics.synced);
        assert_eq!((lyrics.current, lyrics.next), (None, None));
        assert_eq!(lyrics.lines.len(), 1);
    }

    #[test]
    fn long_lyrics_are_cut_after_the_active_line_is_picked() {
        let total = LYRICS_LIMIT as u64 + 5;
        let lyrics = snapshot(synced(total), Some(LYRICS_LIMIT + 2));
        assert!(lyrics.truncated);
        assert_eq!(lyrics.lines.len(), LYRICS_LIMIT);
        assert_eq!(
            lyrics.lines.last().unwrap().text,
            (LYRICS_LIMIT - 1).to_string()
        );
        assert_eq!(lyrics.current.unwrap().text, (LYRICS_LIMIT + 2).to_string());
        assert_eq!(lyrics.next.unwrap().text, (LYRICS_LIMIT + 3).to_string());
    }

    #[test]
    fn queue_kinds_serialize_by_name() {
        let queue = QueueSnapshot::bounded(
            Vec::new(),
            None,
            vec![
                entry("by-hand", QueueEntryKind::Queued),
                entry("similar", QueueEntryKind::Suggested),
            ],
        );
        let printed = serde_json::to_string(&queue).unwrap();
        assert!(printed.contains("\"kind\":\"queued\""), "{printed}");
        assert!(printed.contains("\"kind\":\"suggested\""), "{printed}");
    }
}
