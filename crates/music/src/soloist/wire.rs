//! The messages of Soloist's local WebSocket. Every frame is JSON text with a `type`. The
//! program adds fields as it grows, so everything here is read leniently: an unknown message is
//! `Other` and a missing field takes its default.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

/// What the program reports. `PlaybackState` is the whole picture, sent on connect and in
/// answer to `get_state`; the others carry one change each.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Event {
    AuthState {
        #[serde(default)]
        logged_in: bool,
    },
    PlaybackState(Playback),
    PlaybackChanged(Playback),
    TrackChanged {
        #[serde(default)]
        item: Option<Entity>,
    },
    PositionSync {
        position: Position,
    },
    QueueChanged {
        #[serde(default)]
        upcoming: Vec<Queued>,
    },
    VolumeChanged {
        volume: u8,
    },
    OptionsChanged {
        options: Options,
    },
    DeviceChanged {
        #[serde(default)]
        is_active: bool,
    },
    /// The daemon took a command. The change it leads to is reported on its own.
    CommandResult {
        #[serde(default)]
        command: String,
    },
    Error {
        #[serde(default)]
        message: String,
    },
    #[serde(other)]
    Other,
}

/// The playback picture, whole in a `playback_state` and partial in a `playback_changed`.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Playback {
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub item: Option<Entity>,
    #[serde(default)]
    pub position: Option<Position>,
    #[serde(default)]
    pub volume: Option<u8>,
    #[serde(default)]
    pub options: Option<Options>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    #[default]
    Idle,
    Playing,
    Paused,
    Buffering,
    #[serde(other)]
    Unknown,
}

/// A track or whatever else the program plays. `decorations` holds its name, artwork and
/// length, of which only the length is read.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Entity {
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    decorations: Value,
}

impl Entity {
    pub(super) fn duration(&self) -> Option<Duration> {
        self.decorations["playback"]["duration_ms"]
            .as_u64()
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis)
    }
}

/// One entry of what the daemon plays after the current track.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Queued {
    #[serde(default)]
    source: String,
    #[serde(default)]
    pub item: Option<Entity>,
}

impl Queued {
    /// Whether somebody queued the entry by hand, as opposed to it coming from the album or
    /// playlist that plays, or from autoplay.
    pub(super) fn queued(&self) -> bool {
        self.source == "queue"
    }
}

/// Where playback stood when the message was written.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub(super) struct Position {
    #[serde(default)]
    position_ms: u64,
}

impl Position {
    pub(super) fn at(self) -> Duration {
        Duration::from_millis(self.position_ms)
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct Options {
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    repeat: String,
}

impl Options {
    /// Whether the program would reorder or loop what it is given.
    pub(super) fn meddles(&self) -> bool {
        self.shuffle || !(self.repeat.is_empty() || self.repeat == "off")
    }
}

/// A command frame: `name` with whatever fields it takes.
pub(super) fn command(name: &str, fields: Value) -> String {
    let mut frame = json!({ "type": "command", "command": name });
    if let (Some(frame), Value::Object(fields)) = (frame.as_object_mut(), fields) {
        frame.extend(fields);
    }
    frame.to_string()
}
