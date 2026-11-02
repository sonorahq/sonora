//! The WebSocket to the Sonora Session server, run on tokio. Requests go in through a channel and
//! replies come out of another, so the entity never touches the socket itself.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

use super::wire::{Reply, Request};
use crate::Io;

/// How often the server's clock is asked for. It also keeps the connection from going idle,
/// which the server hangs up on after a minute.
const PING: Duration = Duration::from_secs(5);
/// How long connecting may take before it counts as out of reach.
const CONNECT: Duration = Duration::from_secs(10);

/// What the link hears.
pub(crate) enum Heard {
    Reply(Box<Reply>),
    /// The connection could not be made or dropped. Nothing more comes after it.
    Lost,
}

/// An open link. Dropping it closes the socket once the requests already sent are written.
pub(crate) struct Link {
    requests: UnboundedSender<Request>,
}

impl Link {
    /// Connects to `url` on tokio and sends `first` once it is open. What the server answers comes
    /// out of the receiver, which ends with `Heard::Lost` when the socket goes.
    pub fn open(io: &Io, url: String, first: Request) -> (Self, UnboundedReceiver<Heard>) {
        let (requests, outgoing) = mpsc::unbounded_channel();
        let (heard, incoming) = mpsc::unbounded_channel();
        io.spawn(run(url, first, outgoing, heard));
        (Self { requests }, incoming)
    }

    pub fn send(&self, request: Request) {
        self.requests.send(request).ok();
    }
}

/// This client's clock in milliseconds since the Unix epoch.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

async fn run(
    url: String,
    first: Request,
    mut outgoing: UnboundedReceiver<Request>,
    heard: UnboundedSender<Heard>,
) {
    let socket = match tokio::time::timeout(CONNECT, tokio_tungstenite::connect_async(&url)).await {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(error)) => {
            log::warn!("listening: cannot reach {url}: {error}");
            heard.send(Heard::Lost).ok();
            return;
        }
        Err(_) => {
            log::warn!("listening: {url} did not answer in time");
            heard.send(Heard::Lost).ok();
            return;
        }
    };
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    let mut pending = Some(first);

    loop {
        let request = match pending.take() {
            Some(request) => Some(request),
            None => tokio::select! {
                request = outgoing.recv() => match request {
                    Some(request) => Some(request),
                    None => {
                        sink.close().await.ok();
                        return;
                    }
                },
                message = stream.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                            Ok(reply) => {
                                heard.send(Heard::Reply(Box::new(reply))).ok();
                            }
                            Err(error) => log::warn!("listening: unreadable reply: {error}"),
                        },
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                            heard.send(Heard::Lost).ok();
                            return;
                        }
                        Some(Ok(_)) => {}
                    }
                    None
                }
                _ = ping.tick() => Some(Request::Ping { t: now() }),
            },
        };
        let Some(request) = request else {
            continue;
        };
        let text = match serde_json::to_string(&request) {
            Ok(text) => text,
            Err(error) => {
                log::error!("listening: cannot encode a request: {error}");
                continue;
            }
        };
        if let Err(error) = sink.send(Message::Text(text.into())).await {
            log::warn!("listening: cannot send to the server: {error}");
            heard.send(Heard::Lost).ok();
            return;
        }
    }
}
