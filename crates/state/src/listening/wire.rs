//! The messages traded with the Sonora Session server, one JSON object per WebSocket text frame.
//! The server keeps the same types in its own repository, so a field added on either side has to
//! be optional until both know it.

use music::Track;
use serde::{Deserialize, Serialize};

/// The protocol this client speaks.
pub(crate) const VERSION: u32 = 1;

/// What the client asks of the server.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Request {
    /// Opens a session on `provider` with this client as its host.
    Create {
        version: u32,
        name: String,
        image: Option<String>,
        provider: String,
    },
    /// Joins the session `code` names, which has to play from one of `providers`.
    Join {
        version: u32,
        code: String,
        name: String,
        image: Option<String>,
        providers: Vec<String>,
    },
    /// What plays here now. `seq` counts this client's own states, which the server echoes.
    State { state: Box<Shared>, seq: u64 },
    /// Leaves the session, which ends it for everyone when the host leaves.
    Leave,
    /// Removes a member from the session this client hosts.
    Kick { member: String },
    /// Asks for the server's clock. `t` is this client's own, in milliseconds.
    Ping { t: u64 },
}

/// What the server tells the client.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Reply {
    Joined {
        code: String,
        provider: String,
        you: String,
        members: Vec<Member>,
        state: Option<Shared>,
    },
    Members {
        members: Vec<Member>,
    },
    /// What plays in the session now, as member `by` set it in their state `seq`.
    State {
        state: Shared,
        by: String,
        seq: u64,
    },
    Pong {
        t: u64,
        now: u64,
    },
    /// The host ended the session.
    Ended,
    /// The host removed this client from the session.
    Kicked,
    Error {
        reason: Reason,
        #[serde(default)]
        provider: Option<String>,
    },
}

/// Why the server turned a request down.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    NotFound,
    Provider,
    Full,
    RateLimited,
    Busy,
    /// This address already started as many live sessions as one may.
    TooMany,
    Version,
    Invalid,
    #[serde(other)]
    Unknown,
}

/// A person in a session, as the server lists them.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub image: Option<String>,
    pub host: bool,
}

/// Playback as one member shares it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Shared {
    /// The track playing, or `None` while the member plays nothing that can be shared, such as a
    /// local file.
    pub track: Option<Track>,
    #[serde(default)]
    pub upcoming: Vec<Track>,
    pub playing: bool,
    /// Where the track was, in milliseconds, at `at`.
    pub position: u64,
    /// The server's clock in milliseconds when the state reached it.
    #[serde(default)]
    pub at: u64,
}
