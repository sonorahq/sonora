//! The typed control API Sonora's plugins share. These commands, snapshots and events are
//! transport-agnostic: the REST and WebSocket plugins speak this same schema, and the only
//! implementation lives behind `channel::Client` on the GPUI thread. This crate never touches
//! GPUI, providers or sockets itself.

pub mod channel;

use serde::{Deserialize, Serialize};

/// The wire schema version every reply and snapshot envelope carries as `v`.
pub const API_VERSION: u32 = 1;

/// A command a plugin client asks Sonora to run. The JSON shape is shared by REST and
/// WebSocket, e.g. `{"command":"set_volume","level":0.6}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek {
        position_ms: u64,
    },
    /// Normalized volume from 0.0 (mute) to 1.0 (full scale).
    SetVolume {
        level: f32,
    },
    SetRepeat {
        mode: RepeatMode,
    },
    SetShuffle {
        enabled: bool,
    },
    QueuePlayUpcoming {
        index: usize,
        expected_revision: u64,
    },
    QueueRemoveUpcoming {
        index: usize,
        expected_revision: u64,
    },
    QueueMoveUpcoming {
        from: usize,
        to: usize,
        expected_revision: u64,
    },
    QueueClearUpcoming {
        expected_revision: u64,
    },
}

impl Command {
    /// Rejects values no host could honor, before the request crosses threads. Queue indices
    /// and revisions are checked against the live queue by the host instead.
    pub fn validate(&self) -> Result<(), ControlError> {
        match self {
            Self::SetVolume { level } if !level.is_finite() || !(0.0..=1.0).contains(level) => Err(
                ControlError::invalid_argument("volume must be a finite level between 0.0 and 1.0"),
            ),
            _ => Ok(()),
        }
    }
}

/// The repeat mode on the wire: `off`, `all` or `one`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// What the player looks like from outside. `Failed` carries no reason: internal error
/// strings stay out of API responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Idle,
    Playing,
    Paused,
    Loading,
    Failed,
}

/// Which account state the session is in. Never carries profile details or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    SignedOut,
    Restoring,
    Authorizing,
    SignedIn,
    Offline,
    Failed,
}

/// The only track shape plugins ever see. Local tracks report `is_local` without any id: a
/// raw local id is an absolute filesystem path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackSummary {
    pub name: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u64,
    /// The provider's id for a remote track. Always absent for local files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_track_id: Option<String>,
    pub is_local: bool,
}

/// The player: status, current track, clock, volume and modes. Times are milliseconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub track: Option<TrackSummary>,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
}

/// The queue around the current track. `upcoming` holds what the user queued, without the
/// radio suggestions in `suggested`; `revision` detects stale queue commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub revision: u64,
    pub past: Vec<TrackSummary>,
    pub current: Option<TrackSummary>,
    pub upcoming: Vec<TrackSummary>,
    pub suggested: Vec<TrackSummary>,
    pub manual_count: usize,
    pub shuffle: bool,
}

/// The session: normalized status plus the active provider's public slug, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub status: SessionStatus,
    pub provider: Option<String>,
}

/// The whole player state at one revision. The host bumps `snapshot_revision` on every
/// change it publishes, so clients can tell a stale view from a fresh one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub snapshot_revision: u64,
    pub playback: PlaybackSnapshot,
    pub queue: QueueSnapshot,
    pub session: SessionSnapshot,
}

/// A state change broadcast to subscribers. Serializes to
/// `{"event":"playback.changed","data":{...}}`, the same object the WebSocket plugin embeds
/// in its envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum ControlEvent {
    #[serde(rename = "playback.changed")]
    PlaybackChanged {
        snapshot_revision: u64,
        status: PlaybackStatus,
    },
    #[serde(rename = "queue.changed")]
    QueueChanged {
        snapshot_revision: u64,
        queue_revision: u64,
    },
    #[serde(rename = "session.changed")]
    SessionChanged {
        snapshot_revision: u64,
        status: SessionStatus,
    },
    #[serde(rename = "position.changed")]
    PositionChanged {
        snapshot_revision: u64,
        position_ms: u64,
    },
    #[serde(rename = "resync.required")]
    ResyncRequired { snapshot_revision: u64 },
}

impl ControlEvent {
    /// The snapshot revision this event belongs to.
    pub fn snapshot_revision(&self) -> u64 {
        match *self {
            Self::PlaybackChanged {
                snapshot_revision, ..
            } => snapshot_revision,
            Self::QueueChanged {
                snapshot_revision, ..
            } => snapshot_revision,
            Self::SessionChanged {
                snapshot_revision, ..
            } => snapshot_revision,
            Self::PositionChanged {
                snapshot_revision, ..
            } => snapshot_revision,
            Self::ResyncRequired { snapshot_revision } => snapshot_revision,
        }
    }
}

/// Stable machine-readable failure codes. The human message next to them is safe to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidArgument,
    NotFound,
    Conflict,
    Unavailable,
    PermissionDenied,
    ResourceExhausted,
    Internal,
}

/// A refused command. Never carries provider errors, paths or credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlError {
    pub code: ErrorCode,
    pub message: String,
}

impl ControlError {
    fn with(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The request itself is malformed or out of range.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::InvalidArgument, message)
    }

    /// The target of the request does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::NotFound, message)
    }

    /// The queue moved under the client; it should fetch a fresh snapshot and retry.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::Conflict, message)
    }

    /// The player cannot run the command right now.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::Unavailable, message)
    }

    /// The client is not allowed to run this command.
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::PermissionDenied, message)
    }

    /// The client must slow down.
    pub fn resource_exhausted(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::ResourceExhausted, message)
    }

    /// Something unexpected failed. The message stays generic on purpose.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::with(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ControlError {}

/// What a host answers an accepted command with. The command was validated and handed to the
/// player; `snapshot_revision` names the snapshot that will reflect it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReply {
    pub accepted: bool,
    pub snapshot_revision: u64,
}

/// The versioned success envelope: `{"v":1,"accepted":true,"snapshot_revision":42}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessReply {
    pub v: u32,
    pub accepted: bool,
    pub snapshot_revision: u64,
}

impl SuccessReply {
    /// Wraps an accepted command reply in the versioned envelope.
    pub fn ok(snapshot_revision: u64) -> Self {
        Self {
            v: API_VERSION,
            accepted: true,
            snapshot_revision,
        }
    }
}

/// The versioned error envelope: `{"v":1,"error":{"code":"conflict","message":"..."}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReply {
    pub v: u32,
    pub error: ControlError,
}

impl ErrorReply {
    /// Wraps a refused command in the versioned envelope.
    pub fn new(error: ControlError) -> Self {
        Self {
            v: API_VERSION,
            error,
        }
    }
}

/// The versioned snapshot envelope: `{"v":1,"snapshot_revision":N,"playback":{...},...}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotReply {
    pub v: u32,
    #[serde(flatten)]
    pub snapshot: AppSnapshot,
}

impl SnapshotReply {
    /// Wraps a snapshot in the versioned envelope.
    pub fn new(snapshot: AppSnapshot) -> Self {
        Self {
            v: API_VERSION,
            snapshot,
        }
    }
}

/// A command sent by a WebSocket client:
/// `{"v":1,"id":"req-12","type":"command","data":{"command":"seek",...}}`. The `id` is a
/// client-chosen correlation id, echoed back in the result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WsCommand {
    pub v: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: WsCommandKind,
    pub data: Command,
}

/// The only inbound WebSocket message kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsCommandKind {
    Command,
}

/// A message sent to a WebSocket client. Responses to commands carry the command's `id`,
/// events carry the broadcast `seq`, and the opening snapshot carries the state to subscribe
/// from. Serialization only: clients parse these loosely.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsServerMessage {
    /// The first message: the full state the client subscribes from.
    Snapshot { v: u32, data: Box<AppSnapshot> },
    /// The answer to one `WsCommand`, accepted or refused.
    Result {
        v: u32,
        id: String,
        data: WsResultData,
    },
    /// One state change. `seq` is the broadcast sequence; a resync marker carries none.
    Event {
        v: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
        #[serde(flatten)]
        event: ControlEvent,
    },
    /// The client sent something unparsable. No `id` could be trusted from it.
    Error { v: u32, data: WsErrorData },
}

/// The answer payload of one command: the acceptance, or the refusal shaped like a REST
/// error's data.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum WsResultData {
    Accepted {
        accepted: bool,
        snapshot_revision: u64,
    },
    Failed {
        error: ControlError,
    },
}

/// An id-less failure payload, shaped like a refused result's data.
#[derive(Clone, Debug, Serialize)]
pub struct WsErrorData {
    pub error: ControlError,
}

/// The versioned playback section: `{"v":1,"snapshot_revision":N,"playback":{...}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybackReply {
    pub v: u32,
    pub snapshot_revision: u64,
    pub playback: PlaybackSnapshot,
}

impl PlaybackReply {
    /// Wraps a playback section with its snapshot revision.
    pub fn new(snapshot_revision: u64, playback: PlaybackSnapshot) -> Self {
        Self {
            v: API_VERSION,
            snapshot_revision,
            playback,
        }
    }
}

/// The versioned queue section: `{"v":1,"snapshot_revision":N,"queue":{...}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueReply {
    pub v: u32,
    pub snapshot_revision: u64,
    pub queue: QueueSnapshot,
}

impl QueueReply {
    /// Wraps a queue section with its snapshot revision.
    pub fn new(snapshot_revision: u64, queue: QueueSnapshot) -> Self {
        Self {
            v: API_VERSION,
            snapshot_revision,
            queue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(local: bool) -> TrackSummary {
        TrackSummary {
            name: "Song".to_owned(),
            artists: "Artist".to_owned(),
            album: "Album".to_owned(),
            duration_ms: 180_000,
            provider_track_id: match local {
                true => None,
                false => Some("provider-track-1".to_owned()),
            },
            is_local: local,
        }
    }

    fn snapshot() -> AppSnapshot {
        AppSnapshot {
            snapshot_revision: 42,
            playback: PlaybackSnapshot {
                status: PlaybackStatus::Playing,
                track: Some(track(false)),
                position_ms: 90_000,
                duration_ms: Some(180_000),
                volume: 0.6,
                repeat: RepeatMode::All,
                shuffle: true,
            },
            queue: QueueSnapshot {
                revision: 17,
                past: Vec::new(),
                current: Some(track(false)),
                upcoming: vec![track(true)],
                suggested: Vec::new(),
                manual_count: 1,
                shuffle: true,
            },
            session: SessionSnapshot {
                status: SessionStatus::SignedIn,
                provider: Some("spotify".to_owned()),
            },
        }
    }

    fn command_shapes() -> Vec<(Command, &'static str)> {
        vec![
            (Command::Play, r#"{"command":"play"}"#),
            (Command::Pause, r#"{"command":"pause"}"#),
            (Command::Toggle, r#"{"command":"toggle"}"#),
            (Command::Next, r#"{"command":"next"}"#),
            (Command::Previous, r#"{"command":"previous"}"#),
            (
                Command::Seek {
                    position_ms: 90_000,
                },
                r#"{"command":"seek","position_ms":90000}"#,
            ),
            (
                Command::SetVolume { level: 0.6 },
                r#"{"command":"set_volume","level":0.6}"#,
            ),
            (
                Command::SetRepeat {
                    mode: RepeatMode::All,
                },
                r#"{"command":"set_repeat","mode":"all"}"#,
            ),
            (
                Command::SetShuffle { enabled: true },
                r#"{"command":"set_shuffle","enabled":true}"#,
            ),
            (
                Command::QueuePlayUpcoming {
                    index: 2,
                    expected_revision: 17,
                },
                r#"{"command":"queue_play_upcoming","index":2,"expected_revision":17}"#,
            ),
            (
                Command::QueueRemoveUpcoming {
                    index: 2,
                    expected_revision: 17,
                },
                r#"{"command":"queue_remove_upcoming","index":2,"expected_revision":17}"#,
            ),
            (
                Command::QueueMoveUpcoming {
                    from: 3,
                    to: 0,
                    expected_revision: 17,
                },
                r#"{"command":"queue_move_upcoming","from":3,"to":0,"expected_revision":17}"#,
            ),
            (
                Command::QueueClearUpcoming {
                    expected_revision: 17,
                },
                r#"{"command":"queue_clear_upcoming","expected_revision":17}"#,
            ),
        ]
    }

    #[test]
    fn commands_round_trip_through_their_documented_shapes() {
        for (command, shape) in command_shapes() {
            assert_eq!(serde_json::to_string(&command).unwrap(), shape);
            assert_eq!(serde_json::from_str::<Command>(shape).unwrap(), command);
        }
    }

    #[test]
    fn unknown_commands_and_bad_values_fail_to_parse() {
        for body in [
            r#"{"command":"launch_missiles"}"#,
            r#"{"command":"seek"}"#,
            r#"{"command":"seek","position_ms":-5}"#,
            r#"{"command":"seek","position_ms":18446744073709551616}"#,
            r#"{"command":"set_volume","level":"loud"}"#,
            r#"{"command":"set_repeat","mode":"sometimes"}"#,
            r#"{"command":"queue_remove_upcoming","index":-1,"expected_revision":17}"#,
            "not json",
        ] {
            assert!(
                serde_json::from_str::<Command>(body).is_err(),
                "should reject {body}"
            );
        }
    }

    #[test]
    fn validation_rejects_volumes_no_host_could_honor() {
        for level in [1.5, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = Command::SetVolume { level }.validate().unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument);
        }
        for level in [0.0, 0.6, 1.0] {
            assert!(Command::SetVolume { level }.validate().is_ok());
        }
    }

    #[test]
    fn validation_accepts_everything_without_wire_bounds() {
        for (command, _) in command_shapes() {
            if matches!(command, Command::SetVolume { .. }) {
                continue;
            }
            assert!(command.validate().is_ok());
        }
    }

    #[test]
    fn local_tracks_serialize_without_any_id() {
        let local = serde_json::to_value(track(true)).unwrap();
        assert_eq!(local["is_local"], true);
        assert!(local.get("provider_track_id").is_none());
        assert_eq!(
            serde_json::from_value::<TrackSummary>(local).unwrap(),
            track(true)
        );

        let remote = serde_json::to_value(track(false)).unwrap();
        assert_eq!(remote["provider_track_id"], "provider-track-1");
    }

    #[test]
    fn replies_use_the_versioned_envelopes() {
        let reply = SuccessReply::ok(42);
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"v":1,"accepted":true,"snapshot_revision":42}"#
        );
        assert_eq!(
            serde_json::from_str::<SuccessReply>(
                r#"{"v":1,"accepted":true,"snapshot_revision":42}"#
            )
            .unwrap(),
            reply
        );

        let error = ErrorReply::new(ControlError::conflict(
            "queue changed; request a new snapshot",
        ));
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"v":1,"error":{"code":"conflict","message":"queue changed; request a new snapshot"}}"#
        );
        assert_eq!(
            serde_json::from_str::<ErrorReply>(
                r#"{"v":1,"error":{"code":"conflict","message":"queue changed; request a new snapshot"}}"#
            )
            .unwrap(),
            error
        );

        let unfolded = serde_json::to_value(SnapshotReply::new(snapshot())).unwrap();
        assert_eq!(unfolded["v"], 1);
        assert_eq!(unfolded["snapshot_revision"], 42);
        assert_eq!(unfolded["playback"]["status"], "playing");
        assert_eq!(unfolded["queue"]["revision"], 17);
        assert_eq!(unfolded["session"]["provider"], "spotify");
        assert_eq!(
            serde_json::from_value::<SnapshotReply>(unfolded)
                .unwrap()
                .snapshot,
            snapshot()
        );

        let played = serde_json::to_value(PlaybackReply::new(42, snapshot().playback)).unwrap();
        assert_eq!(played["v"], 1);
        assert_eq!(played["snapshot_revision"], 42);
        assert_eq!(played["playback"]["status"], "playing");
        assert_eq!(
            serde_json::from_value::<PlaybackReply>(played)
                .unwrap()
                .playback,
            snapshot().playback
        );

        let queued = serde_json::to_value(QueueReply::new(42, snapshot().queue)).unwrap();
        assert_eq!(queued["v"], 1);
        assert_eq!(queued["snapshot_revision"], 42);
        assert_eq!(queued["queue"]["revision"], 17);
        assert_eq!(
            serde_json::from_value::<QueueReply>(queued).unwrap().queue,
            snapshot().queue
        );
    }

    #[test]
    fn error_codes_use_snake_case() {
        let codes = [
            (ErrorCode::InvalidArgument, "invalid_argument"),
            (ErrorCode::NotFound, "not_found"),
            (ErrorCode::Conflict, "conflict"),
            (ErrorCode::Unavailable, "unavailable"),
            (ErrorCode::PermissionDenied, "permission_denied"),
            (ErrorCode::ResourceExhausted, "resource_exhausted"),
            (ErrorCode::Internal, "internal"),
        ];
        for (code, name) in codes {
            let shape = format!("\"{name}\"");
            assert_eq!(serde_json::to_string(&code).unwrap(), shape);
            assert_eq!(serde_json::from_str::<ErrorCode>(&shape).unwrap(), code);
        }
    }

    #[test]
    fn events_use_dotted_names_with_their_revision() {
        let events = [
            (
                ControlEvent::PlaybackChanged {
                    snapshot_revision: 44,
                    status: PlaybackStatus::Playing,
                },
                r#"{"event":"playback.changed","data":{"snapshot_revision":44,"status":"playing"}}"#,
            ),
            (
                ControlEvent::QueueChanged {
                    snapshot_revision: 44,
                    queue_revision: 18,
                },
                r#"{"event":"queue.changed","data":{"snapshot_revision":44,"queue_revision":18}}"#,
            ),
            (
                ControlEvent::SessionChanged {
                    snapshot_revision: 44,
                    status: SessionStatus::SignedOut,
                },
                r#"{"event":"session.changed","data":{"snapshot_revision":44,"status":"signed_out"}}"#,
            ),
            (
                ControlEvent::PositionChanged {
                    snapshot_revision: 44,
                    position_ms: 91_000,
                },
                r#"{"event":"position.changed","data":{"snapshot_revision":44,"position_ms":91000}}"#,
            ),
            (
                ControlEvent::ResyncRequired {
                    snapshot_revision: 44,
                },
                r#"{"event":"resync.required","data":{"snapshot_revision":44}}"#,
            ),
        ];
        for (event, shape) in events {
            assert_eq!(event.snapshot_revision(), 44);
            assert_eq!(serde_json::to_string(&event).unwrap(), shape);
            assert_eq!(serde_json::from_str::<ControlEvent>(shape).unwrap(), event);
        }
    }

    #[test]
    fn ws_commands_parse_with_their_correlation_id() {
        let shape = r#"{"v":1,"id":"req-12","type":"command","data":{"command":"seek","position_ms":90000}}"#;
        let command: WsCommand = serde_json::from_str(shape).unwrap();

        assert_eq!(command.v, 1);
        assert_eq!(command.id, "req-12");
        assert_eq!(command.kind, WsCommandKind::Command);
        assert_eq!(
            command.data,
            Command::Seek {
                position_ms: 90_000
            }
        );
        assert_eq!(
            serde_json::from_str::<WsCommand>(&serde_json::to_string(&command).unwrap()).unwrap(),
            command
        );

        for shape in [
            r#"{"v":1,"id":"req-12","type":"subscribe","data":{"command":"seek","position_ms":90000}}"#,
            r#"{"v":1,"id":"req-12","data":{"command":"seek","position_ms":90000}}"#,
            r#"{"v":1,"type":"command","data":{"command":"seek","position_ms":90000}}"#,
            r#"{"v":1,"id":"req-12","type":"command","data":{"command":"launch"}}"#,
        ] {
            assert!(
                serde_json::from_str::<WsCommand>(shape).is_err(),
                "should reject {shape}"
            );
        }
    }

    #[test]
    fn ws_server_messages_use_the_envelope() {
        let snapshot = serde_json::to_value(WsServerMessage::Snapshot {
            v: API_VERSION,
            data: Box::new(snapshot()),
        })
        .unwrap();
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["v"], 1);
        assert_eq!(snapshot["data"]["snapshot_revision"], 42);

        let accepted = serde_json::to_value(WsServerMessage::Result {
            v: API_VERSION,
            id: "req-12".to_owned(),
            data: WsResultData::Accepted {
                accepted: true,
                snapshot_revision: 43,
            },
        })
        .unwrap();
        assert_eq!(accepted["type"], "result");
        assert_eq!(accepted["id"], "req-12");
        assert_eq!(accepted["data"]["accepted"], true);
        assert_eq!(accepted["data"]["snapshot_revision"], 43);

        let refused = serde_json::to_value(WsServerMessage::Result {
            v: API_VERSION,
            id: "req-13".to_owned(),
            data: WsResultData::Failed {
                error: ControlError::conflict("queue changed; request a new snapshot"),
            },
        })
        .unwrap();
        assert_eq!(refused["data"]["error"]["code"], "conflict");

        let event = serde_json::to_value(WsServerMessage::Event {
            v: API_VERSION,
            seq: Some(44),
            event: ControlEvent::PlaybackChanged {
                snapshot_revision: 44,
                status: PlaybackStatus::Playing,
            },
        })
        .unwrap();
        assert_eq!(event["type"], "event");
        assert_eq!(event["seq"], 44);
        assert_eq!(event["event"], "playback.changed");
        assert_eq!(event["data"]["status"], "playing");

        let resync = serde_json::to_value(WsServerMessage::Event {
            v: API_VERSION,
            seq: None,
            event: ControlEvent::ResyncRequired {
                snapshot_revision: 45,
            },
        })
        .unwrap();
        assert!(resync.get("seq").is_none());
        assert_eq!(resync["event"], "resync.required");

        let error = serde_json::to_value(WsServerMessage::Error {
            v: API_VERSION,
            data: WsErrorData {
                error: ControlError::invalid_argument("invalid message"),
            },
        })
        .unwrap();
        assert_eq!(error["type"], "error");
        assert_eq!(error["data"]["error"]["code"], "invalid_argument");
    }
}
