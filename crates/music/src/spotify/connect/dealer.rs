use std::collections::HashMap;
use std::io::Read as _;
use std::time::Duration;

use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use flate2::read::MultiGzDecoder;
use futures::{SinkExt as _, StreamExt as _};
use prost::Message as ProtoMessage;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message as Frame,
};

use crate::spotify::{
    proto::connect::{Context, ProvidedTrack},
    session::Session,
};
use crate::{SignInFailure, SignInProblem};

const PLAYER_COMMAND: &str = "hm://connect-state/v1/player/command";
const CONNECTION: &str = "hm://pusher/v1/connections";
const PING_INTERVAL: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const ANNOUNCEMENT_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_LAST: Duration = Duration::from_secs(60);
const MAX_DECODED: u64 = 16 * 1024 * 1024;

/// The TLS WebSocket used for one Dealer connection.
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A decoded Dealer push, before routing to Connect or the library.
pub(super) struct Message {
    pub(super) uri: String,
    pub(super) payload: PayloadValue,
}

/// The wire representation of a push's single payload.
pub(super) enum PayloadValue {
    Empty,
    Raw(Bytes),
    Json(Value),
}

/// A remote command and the identifiers Sonora echoes in its next state update.
#[derive(Deserialize)]
pub(super) struct Request {
    pub(super) message_id: u32,
    pub(super) sent_by_device_id: String,
    pub(super) command: Command,
}

/// One socket worker and its ordered stream of events. Dropping it closes the connection.
pub(super) struct Dealer {
    task: JoinHandle<()>,
    events: UnboundedReceiver<Event>,
}

/// A notification, command, or connection change from Spotify's Dealer.
pub(super) enum Event {
    Connected(String),
    Disconnected,
    Message(Message),
    Command(Box<Request>, Reply),
}

/// The reply is tied to the socket that received the command, even across reconnects.
pub(super) struct Reply {
    key: String,
    frames: UnboundedSender<Frame>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Envelope {
    Message {
        uri: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        payloads: Vec<Payload>,
    },
    Request {
        message_ident: String,
        key: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        payload: Option<Compressed>,
    },
    Pong,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct Compressed {
    compressed: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Payload {
    String(String),
    Bytes(Vec<u8>),
    Json(Value),
}

impl Dealer {
    pub(super) fn start(session: Session) -> Self {
        let (send, events) = unbounded_channel();
        let task = tokio::spawn(run(session, send));
        Self { task, events }
    }

    pub(super) async fn next(&mut self) -> Option<Event> {
        self.events.recv().await
    }
}

impl Drop for Dealer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Message {
    /// Decodes a topic that Spotify sends as either protobuf or JSON.
    pub(super) fn decode<M: ProtoMessage + Default + serde::de::DeserializeOwned>(
        self,
    ) -> Result<M> {
        match self.payload {
            PayloadValue::Json(value) => Ok(serde_json::from_value(value)?),
            _ => self.decode_protobuf(),
        }
    }

    pub(super) fn decode_protobuf<M: ProtoMessage + Default>(self) -> Result<M> {
        let PayloadValue::Raw(bytes) = self.payload else {
            anyhow::bail!("expected a protobuf Dealer payload");
        };
        Ok(M::decode(bytes)?)
    }
}

impl Reply {
    pub(super) fn send(self, success: bool) {
        let body = json!({"type": "reply", "key": self.key, "payload": {"success": success}});
        let _ = self.frames.send(Frame::Text(body.to_string().into()));
    }
}

/// Compares Dealer topics without treating similarly named routes as the same prefix.
pub(super) fn topic(uri: &str, prefix: &str) -> bool {
    let uri = uri.split('?').next().unwrap_or_default();
    uri == prefix
        || uri
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/'))
}

async fn run(session: Session, events: UnboundedSender<Event>) {
    let mut delay = RETRY_FIRST;
    loop {
        let result = match timeout(CONNECT_TIMEOUT, connect(&session)).await {
            Ok(Ok(mut socket)) => receive(&mut socket, &events, &mut delay).await,
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            log::warn!("spotify: dealer disconnected: {error:#}");
            if matches!(
                error.downcast_ref::<SignInFailure>(),
                Some(SignInFailure(SignInProblem::Credentials))
            ) {
                return;
            }
        }
        if events.send(Event::Disconnected).is_err() {
            return;
        }
        tokio::select! {
            () = events.closed() => return,
            () = sleep(delay) => delay = (delay * 2).min(RETRY_LAST),
        }
    }
}

async fn connect(session: &Session) -> Result<Socket> {
    let (urls, token) = tokio::try_join!(session.dealer_urls(), session.access_token())?;
    for mut url in urls {
        url.query_pairs_mut().append_pair("access_token", &token);
        if let Ok(Ok((socket, _))) = timeout(PING_TIMEOUT, connect_async(url.as_str())).await {
            return Ok(socket);
        }
    }
    anyhow::bail!("cannot open Spotify Dealer WebSocket")
}

/// One deadline covers connection setup and the two phases of the heartbeat.
#[derive(PartialEq, Eq)]
enum Heartbeat {
    Announcement,
    Ready,
    Pong,
}

async fn receive(
    socket: &mut Socket,
    events: &UnboundedSender<Event>,
    reconnect_delay: &mut Duration,
) -> Result<()> {
    let deadline = sleep(ANNOUNCEMENT_TIMEOUT);
    tokio::pin!(deadline);
    let mut heartbeat = Heartbeat::Announcement;
    let (send, mut replies) = unbounded_channel();

    loop {
        tokio::select! {
            () = events.closed() => return Ok(()),
            () = &mut deadline => {
                match heartbeat {
                    Heartbeat::Announcement => anyhow::bail!("no Spotify Dealer connection ID"),
                    Heartbeat::Pong => anyhow::bail!("Spotify Dealer did not answer ping"),
                    Heartbeat::Ready => {}
                }
                timeout(PING_TIMEOUT, socket.send(Frame::Text(r#"{"type":"ping"}"#.into()))).await??;
                deadline.as_mut().reset(Instant::now() + PING_TIMEOUT);
                heartbeat = Heartbeat::Pong;
            }
            Some(reply) = replies.recv() => timeout(PING_TIMEOUT, socket.send(reply)).await??,
            frame = socket.next() => match frame.transpose()? {
                Some(frame @ (Frame::Text(_) | Frame::Binary(_))) => {
                    let envelope = match serde_json::from_slice(&frame.into_data()) {
                        Ok(Envelope::Pong) => {
                            if heartbeat == Heartbeat::Pong {
                                heartbeat = Heartbeat::Ready;
                                deadline.as_mut().reset(Instant::now() + PING_INTERVAL);
                            }
                            continue;
                        }
                        Ok(envelope) => envelope,
                        Err(error) => {
                            log::debug!("spotify: invalid Dealer frame: {error}");
                            continue;
                        }
                    };
                    let event = match event(envelope, &send) {
                        Ok(Some(event)) => event,
                        Ok(None) => continue,
                        Err(error) => {
                            log::debug!("spotify: invalid Dealer event: {error:#}");
                            continue;
                        }
                    };
                    if heartbeat == Heartbeat::Announcement && matches!(event, Event::Connected(_)) {
                        *reconnect_delay = RETRY_FIRST;
                        heartbeat = Heartbeat::Ready;
                        deadline.as_mut().reset(Instant::now() + PING_INTERVAL);
                    }
                    if heartbeat != Heartbeat::Announcement {
                        if events.send(event).is_err() {
                            return Ok(());
                        }
                    } else if let Event::Command(_, reply) = event {
                        reply.send(false);
                    }
                }
                Some(Frame::Ping(_)) => timeout(PING_TIMEOUT, socket.flush()).await??,
                Some(Frame::Close(_)) | None => return Ok(()),
                Some(_) => {}
            },
        }
    }
}

fn event(envelope: Envelope, frames: &UnboundedSender<Frame>) -> Result<Option<Event>> {
    match envelope {
        Envelope::Message {
            uri,
            headers,
            mut payloads,
        } => {
            if topic(&uri, CONNECTION) {
                let id = header(&headers, "Spotify-Connection-Id")
                    .filter(|id| !id.is_empty())
                    .context("missing Spotify connection ID")?;
                return Ok(Some(Event::Connected(id.to_owned())));
            }
            let payload = match payloads.pop() {
                None => PayloadValue::Empty,
                Some(Payload::Json(json)) => PayloadValue::Json(json),
                Some(Payload::String(value)) => {
                    PayloadValue::Raw(decode(STANDARD.decode(value)?, &headers)?.into())
                }
                Some(Payload::Bytes(bytes)) => PayloadValue::Raw(decode(bytes, &headers)?.into()),
            };
            anyhow::ensure!(payloads.is_empty(), "multiple Spotify Dealer payloads");
            Ok(Some(Event::Message(Message { payload, uri })))
        }
        Envelope::Request {
            message_ident,
            key,
            headers,
            payload,
        } => {
            let reply = Reply {
                key,
                frames: frames.clone(),
            };
            if topic(&message_ident, PLAYER_COMMAND)
                && let Some(payload) = payload
            {
                let request = STANDARD
                    .decode(payload.compressed)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| decode(bytes, &headers))
                    .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?));
                match request {
                    Ok(request) => return Ok(Some(Event::Command(Box::new(request), reply))),
                    Err(_) => log::debug!("spotify: invalid Dealer player command"),
                }
            }
            reply.send(false);
            Ok(None)
        }
        Envelope::Pong | Envelope::Unknown => Ok(None),
    }
}

fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn decode(bytes: Vec<u8>, headers: &HashMap<String, String>) -> Result<Vec<u8>> {
    if !header(headers, "Transfer-Encoding")
        .is_some_and(|encoding| encoding.eq_ignore_ascii_case("gzip"))
    {
        return Ok(bytes);
    }
    let mut decoded = Vec::new();
    MultiGzDecoder::new(bytes.as_slice())
        .take(MAX_DECODED + 1)
        .read_to_end(&mut decoded)?;
    anyhow::ensure!(
        decoded.len() as u64 <= MAX_DECODED,
        "Spotify Dealer payload exceeds the response limit"
    );
    Ok(decoded)
}

/// The Dealer player commands consumed by Sonora.
#[derive(Deserialize)]
#[serde(tag = "endpoint", rename_all = "snake_case")]
pub(super) enum Command {
    Transfer {
        data: String,
    },
    Play(Box<PlayCommand>),
    Pause,
    Resume,
    SkipNext,
    SkipPrev,
    SeekTo {
        value: u64,
    },
    SetShufflingContext {
        value: bool,
    },
    SetRepeatingContext {
        value: bool,
    },
    SetRepeatingTrack {
        value: bool,
    },
    SetOptions {
        shuffling_context: Option<bool>,
        repeating_context: Option<bool>,
        repeating_track: Option<bool>,
    },
    AddToQueue {
        track: Box<ProvidedTrack>,
    },
    SetQueue {
        next_tracks: Vec<ProvidedTrack>,
    },
    UpdateContext,
    #[serde(other)]
    Unknown,
}

/// The context and start position requested by another device.
#[derive(Deserialize)]
pub(super) struct PlayCommand {
    pub(super) context: Context,
    #[serde(default)]
    pub(super) options: PlayOptions,
}

#[derive(Default, Deserialize)]
pub(super) struct PlayOptions {
    pub(super) skip_to: Option<Skip>,
    pub(super) seek_to: Option<u64>,
    pub(super) initially_paused: Option<bool>,
    pub(super) player_options_override: Option<Overrides>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Skip {
    pub(super) track_uri: Option<String>,
    pub(super) track_uid: Option<String>,
    pub(super) track_index: Option<u32>,
}

#[derive(Deserialize)]
pub(super) struct Overrides {
    pub(super) shuffling_context: Option<bool>,
}
