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
        #[serde(default)]
        is_active: bool,
    },
    PlaybackState(Playback),
    PlaybackChanged(Playback),
    TrackChanged,
    ContextChanged {
        context: Option<Entity>,
    },
    OptionsChanged {
        options: Options,
    },
    PositionSync {
        position: Position,
    },
    QueueChanged {
        #[serde(default)]
        previous: Vec<Queued>,
        #[serde(default)]
        upcoming: Vec<Queued>,
    },
    VolumeChanged {
        volume: u8,
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
    pub status: Option<Status>,
    pub is_active: Option<bool>,
    pub item: Option<Entity>,
    pub context: Option<Entity>,
    pub options: Option<Options>,
    pub position: Option<Position>,
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

/// Playback options included in the state or changed independently.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Options {
    pub shuffle: Option<bool>,
    pub repeat: Option<String>,
}

/// A playable Spotify item.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Entity {
    pub uri: String,
}

/// One entry of what the daemon plays after the current track.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Queued {
    pub uid: String,
    pub source: String,
    pub item: Option<Entity>,
}

impl Queued {
    pub(super) fn track(self) -> Option<crate::connect::QueuedTrack> {
        Some(crate::connect::QueuedTrack {
            id: self.item?.uri.strip_prefix("spotify:track:")?.to_owned(),
            uid: self.uid,
            queued: self.source == "queue",
            suggested: self.source == "autoplay",
            added_by: None,
        })
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

/// A command frame: `name` with whatever fields it takes.
pub(super) fn command(name: &str, fields: Value) -> String {
    let mut frame = json!({ "type": "command", "command": name });
    if let (Some(frame), Value::Object(fields)) = (frame.as_object_mut(), fields) {
        frame.extend(fields);
    }
    frame.to_string()
}
