use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::timeout;

use crate::{AppSnapshot, Command, CommandReply, ControlError, ControlEvent};

/// How many requests may wait for the host before new ones fail fast instead of queueing.
pub const MAX_PENDING_REQUESTS: usize = 32;
/// How many events a slow subscriber may lag before its oldest ones drop.
pub const MAX_BUFFERED_EVENTS: usize = 64;
/// How long a client waits for the host to answer one request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The host's answer to `Request::Execute`.
pub type CommandResult = Result<CommandReply, ControlError>;

/// One question for the GPUI-owned host. The `id` is minted by `Client` for logging; the
/// `reply` sender is what carries the answer back.
#[derive(Debug)]
pub enum Request {
    GetSnapshot {
        id: u64,
        reply: oneshot::Sender<AppSnapshot>,
    },
    Execute {
        id: u64,
        command: Command,
        reply: oneshot::Sender<CommandResult>,
    },
}

impl Request {
    /// The client-minted id of this request, for logging.
    pub fn id(&self) -> u64 {
        match self {
            Self::GetSnapshot { id, .. } => *id,
            Self::Execute { id, .. } => *id,
        }
    }
}

/// Why a control request produced no reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// The host ran the command and refused it.
    Rejected(ControlError),
    /// The host did not answer in time.
    Timeout,
    /// The bounded request queue is full.
    Overloaded,
    /// The host is gone.
    Unavailable,
}

impl From<RequestError> for ControlError {
    /// Maps a transport failure to the error a plugin answers the client with.
    fn from(error: RequestError) -> Self {
        match error {
            RequestError::Rejected(error) => error,
            RequestError::Timeout => Self::unavailable("the player did not answer in time"),
            RequestError::Overloaded => {
                Self::resource_exhausted("too many control requests at once")
            }
            RequestError::Unavailable => Self::unavailable("the player is unavailable"),
        }
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(error) => write!(f, "{error}"),
            Self::Timeout => write!(f, "the player did not answer in time"),
            Self::Overloaded => write!(f, "too many control requests at once"),
            Self::Unavailable => write!(f, "the player is unavailable"),
        }
    }
}

impl std::error::Error for RequestError {}

/// A state change with its broadcast sequence. A subscriber that lags past
/// `MAX_BUFFERED_EVENTS` must resubscribe from a fresh snapshot instead of trusting the gap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SequencedEvent {
    pub seq: u64,
    pub event: ControlEvent,
}

/// Returns the linked `(Host, Client)` pair over `initial`, which is what a subscriber sees
/// before the host publishes anything.
pub fn pair(initial: AppSnapshot) -> (Host, Client) {
    let (requests, receiver) = mpsc::channel(MAX_PENDING_REQUESTS);
    let (snapshots, current) = watch::channel(initial);
    let (events, _) = broadcast::channel(MAX_BUFFERED_EVENTS);
    let host = Host {
        requests: receiver,
        publisher: Publisher {
            snapshots,
            events: events.clone(),
        },
    };
    let client = Client {
        requests,
        snapshots: current,
        events,
        next_id: Arc::new(AtomicU64::new(1)),
    };
    (host, client)
}

/// The host half: receives requests on the GPUI thread and publishes snapshots and events.
/// Built by `pair`, alongside its `Client`.
pub struct Host {
    requests: mpsc::Receiver<Request>,
    publisher: Publisher,
}

/// The publishing half of `Host`: snapshots and events without the request receiver. The GPUI
/// entity keeps this while the receiver moves into its event loop. Built by `Host::split`.
pub struct Publisher {
    snapshots: watch::Sender<AppSnapshot>,
    events: broadcast::Sender<SequencedEvent>,
}

impl Host {
    /// Waits for the next request. `None` means every client is gone.
    pub async fn recv(&mut self) -> Option<Request> {
        self.requests.recv().await
    }

    /// Publishes the latest snapshot for new and polling subscribers.
    pub fn publish(&self, snapshot: AppSnapshot) {
        self.publisher.publish(snapshot);
    }

    /// Broadcasts one event, and reports how many subscribers will see it.
    pub fn emit(&self, event: SequencedEvent) -> usize {
        self.publisher.emit(event)
    }

    /// Splits the request receiver from the publisher, so the GPUI entity can move the
    /// receiver into its event loop while keeping the publisher for its observers.
    pub fn split(self) -> (mpsc::Receiver<Request>, Publisher) {
        (self.requests, self.publisher)
    }
}

impl Publisher {
    /// Whether any event subscriber is listening. The host samples positions only then.
    pub fn has_receivers(&self) -> bool {
        self.events.receiver_count() > 0
    }

    /// Publishes the latest snapshot for new and polling subscribers.
    pub fn publish(&self, snapshot: AppSnapshot) {
        self.snapshots.send_replace(snapshot);
    }

    /// Broadcasts one event, and reports how many subscribers will see it.
    pub fn emit(&self, event: SequencedEvent) -> usize {
        self.events.send(event).unwrap_or(0)
    }
}

/// The plugin half: cloneable, `Send`, and free of any GPUI handle, so Tokio workers can hold
/// it. Built by `pair`, alongside its `Host`.
#[derive(Clone)]
pub struct Client {
    requests: mpsc::Sender<Request>,
    snapshots: watch::Receiver<AppSnapshot>,
    events: broadcast::Sender<SequencedEvent>,
    next_id: Arc<AtomicU64>,
}

impl Client {
    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Runs a command on the host and waits up to `REQUEST_TIMEOUT` for its answer. Values no
    /// host could honor fail before anything crosses threads.
    pub async fn execute(&self, command: Command) -> Result<CommandReply, RequestError> {
        self.execute_timeout(command, REQUEST_TIMEOUT).await
    }

    /// Runs a command with an explicit deadline instead of `REQUEST_TIMEOUT`.
    pub async fn execute_timeout(
        &self,
        command: Command,
        wait: Duration,
    ) -> Result<CommandReply, RequestError> {
        if let Err(error) = command.validate() {
            return Err(RequestError::Rejected(error));
        }
        let (reply, answer) = oneshot::channel();
        let request = Request::Execute {
            id: self.id(),
            command,
            reply,
        };
        self.send(request)?;
        Self::wait(answer, wait)
            .await?
            .map_err(RequestError::Rejected)
    }

    /// Asks the host for a fresh snapshot, sampling the live player clock.
    pub async fn snapshot(&self) -> Result<AppSnapshot, RequestError> {
        self.snapshot_timeout(REQUEST_TIMEOUT).await
    }

    /// Asks the host for a fresh snapshot with an explicit deadline.
    pub async fn snapshot_timeout(&self, wait: Duration) -> Result<AppSnapshot, RequestError> {
        let (reply, answer) = oneshot::channel();
        let request = Request::GetSnapshot {
            id: self.id(),
            reply,
        };
        self.send(request)?;
        Self::wait(answer, wait).await
    }

    /// The latest published snapshot, without asking the host.
    pub fn current(&self) -> AppSnapshot {
        self.snapshots.borrow().clone()
    }

    /// A receiver that sees every published snapshot from here on.
    pub fn watch(&self) -> watch::Receiver<AppSnapshot> {
        self.snapshots.clone()
    }

    /// A receiver for the event broadcast. On lag, resubscribe from `current`.
    pub fn subscribe(&self) -> broadcast::Receiver<SequencedEvent> {
        self.events.subscribe()
    }

    fn send(&self, request: Request) -> Result<(), RequestError> {
        self.requests
            .try_send(request)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RequestError::Overloaded,
                mpsc::error::TrySendError::Closed(_) => RequestError::Unavailable,
            })
    }

    async fn wait<T>(answer: oneshot::Receiver<T>, wait: Duration) -> Result<T, RequestError> {
        match timeout(wait, answer).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(RequestError::Unavailable),
            Err(_) => Err(RequestError::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ErrorCode, PlaybackSnapshot, PlaybackStatus, QueueSnapshot, RepeatMode, SessionSnapshot,
        SessionStatus,
    };

    use super::*;

    fn sample(snapshot_revision: u64) -> AppSnapshot {
        AppSnapshot {
            snapshot_revision,
            playback: PlaybackSnapshot {
                status: PlaybackStatus::Playing,
                track: None,
                position_ms: 1_000,
                duration_ms: None,
                volume: 0.6,
                repeat: RepeatMode::Off,
                shuffle: false,
            },
            queue: QueueSnapshot {
                revision: 3,
                past: Vec::new(),
                current: None,
                upcoming: Vec::new(),
                suggested: Vec::new(),
                manual_count: 0,
                shuffle: false,
            },
            session: SessionSnapshot {
                status: SessionStatus::SignedIn,
                provider: Some("spotify".to_owned()),
            },
        }
    }

    fn event(seq: u64) -> SequencedEvent {
        SequencedEvent {
            seq,
            event: ControlEvent::PlaybackChanged {
                snapshot_revision: 7,
                status: PlaybackStatus::Paused,
            },
        }
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn client_crosses_to_tokio_workers() {
        assert_send_sync::<Client>();
    }

    #[tokio::test]
    async fn execute_carries_a_command_to_the_host_and_back() {
        let (mut host, client) = pair(sample(0));
        let answered = tokio::spawn(async move {
            let Some(Request::Execute { id, command, reply }) = host.recv().await else {
                panic!("expected an execute request");
            };
            assert_eq!(command, Command::Toggle);
            reply
                .send(Ok(CommandReply {
                    accepted: true,
                    snapshot_revision: 7,
                }))
                .unwrap();
            id
        });

        let reply = client.execute(Command::Toggle).await.unwrap();
        assert!(reply.accepted);
        assert_eq!(reply.snapshot_revision, 7);
        assert_ne!(answered.await.unwrap(), 0);
    }

    #[tokio::test]
    async fn rejections_and_snapshots_round_trip() {
        let (mut host, client) = pair(sample(0));
        tokio::spawn(async move {
            while let Some(request) = host.recv().await {
                match request {
                    Request::Execute { reply, .. } => {
                        reply.send(Err(ControlError::conflict("stale"))).unwrap();
                    }
                    Request::GetSnapshot { reply, .. } => {
                        reply.send(sample(3)).unwrap();
                    }
                }
            }
        });

        let error = client.execute(Command::Next).await.unwrap_err();
        assert_eq!(
            error,
            RequestError::Rejected(ControlError::conflict("stale"))
        );
        assert_eq!(client.snapshot().await.unwrap().snapshot_revision, 3);
    }

    #[tokio::test]
    async fn invalid_volumes_fail_before_reaching_the_host() {
        let (mut host, client) = pair(sample(0));
        let error = client
            .execute(Command::SetVolume { level: 2.0 })
            .await
            .unwrap_err();
        assert_eq!(
            error,
            RequestError::Rejected(ControlError::invalid_argument(
                "volume must be a finite level between 0.0 and 1.0"
            ))
        );
        assert!(host.requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn silent_hosts_time_out() {
        let (_host, client) = pair(sample(0));
        let error = client
            .execute_timeout(Command::Play, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(error, RequestError::Timeout);
    }

    #[tokio::test]
    async fn full_queues_fail_fast() {
        let (requests, held) = mpsc::channel(1);
        let (_, snapshots) = watch::channel(sample(0));
        let (events, _) = broadcast::channel(1);
        let client = Client {
            requests: requests.clone(),
            snapshots,
            events,
            next_id: Arc::new(AtomicU64::new(1)),
        };
        let (reply, _) = oneshot::channel();
        requests
            .try_send(Request::GetSnapshot { id: 0, reply })
            .unwrap();

        let error = client
            .execute_timeout(Command::Play, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(error, RequestError::Overloaded);
        drop(held);
    }

    #[tokio::test]
    async fn gone_hosts_read_as_unavailable() {
        let (host, client) = pair(sample(0));
        drop(host);
        assert_eq!(
            client.execute(Command::Play).await.unwrap_err(),
            RequestError::Unavailable
        );
        assert_eq!(
            client.snapshot().await.unwrap_err(),
            RequestError::Unavailable
        );
    }

    #[tokio::test]
    async fn dropped_replies_read_as_unavailable() {
        let (mut host, client) = pair(sample(0));
        tokio::spawn(async move {
            let _ = host.recv().await;
        });
        assert_eq!(
            client.execute(Command::Play).await.unwrap_err(),
            RequestError::Unavailable
        );
    }

    #[tokio::test]
    async fn split_halves_stay_wired_to_the_client() {
        let (host, client) = pair(sample(0));
        let (mut requests, publisher) = host.split();
        let mut subscribed = client.subscribe();

        let moved = client.clone();
        let sent = tokio::spawn(async move { moved.snapshot().await });
        let Some(Request::GetSnapshot { reply, .. }) = requests.recv().await else {
            panic!("expected a snapshot request");
        };
        reply.send(sample(2)).unwrap();
        assert_eq!(sent.await.unwrap().unwrap().snapshot_revision, 2);

        publisher.publish(sample(4));
        assert_eq!(client.current().snapshot_revision, 4);
        assert_eq!(publisher.emit(event(6)), 1);
        assert_eq!(subscribed.recv().await.unwrap(), event(6));
    }

    #[tokio::test]
    async fn publish_and_emit_reach_subscribers() {
        let (host, client) = pair(sample(0));
        let mut watched = client.watch();
        let mut subscribed = client.subscribe();
        assert_eq!(client.current().snapshot_revision, 0);

        host.publish(sample(5));
        assert_eq!(host.emit(event(9)), 1);

        watched.changed().await.unwrap();
        assert_eq!(client.current().snapshot_revision, 5);
        assert_eq!(subscribed.recv().await.unwrap(), event(9));
    }

    #[test]
    fn receivers_are_counted() {
        let (host, client) = pair(sample(0));
        assert!(!host.publisher.has_receivers());
        let subscribed = client.subscribe();
        assert!(host.publisher.has_receivers());
        drop(subscribed);
        assert!(!host.publisher.has_receivers());
    }

    #[test]
    fn transport_failures_map_to_stable_errors() {
        let cases = [
            (
                RequestError::Timeout,
                ErrorCode::Unavailable,
                "the player did not answer in time",
            ),
            (
                RequestError::Overloaded,
                ErrorCode::ResourceExhausted,
                "too many control requests at once",
            ),
            (
                RequestError::Unavailable,
                ErrorCode::Unavailable,
                "the player is unavailable",
            ),
        ];
        for (error, code, message) in cases {
            let mapped = ControlError::from(error);
            assert_eq!(mapped.code, code);
            assert_eq!(mapped.message, message);
        }
        let rejected = ControlError::conflict("stale");
        assert_eq!(
            ControlError::from(RequestError::Rejected(rejected.clone())),
            rejected
        );
    }
}
