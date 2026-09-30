use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Json;
use axum::extract::Request;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use control::{
    API_VERSION, Command, ControlError, ControlEvent, ErrorCode, ErrorReply, PlaybackReply,
    QueueReply, SnapshotReply, SuccessReply, WsCommand, WsErrorData, WsResultData, WsServerMessage,
};
use serde::Serialize;
use state::{Plugin, PluginContext};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, oneshot, watch};
use tower_http::limit::RequestBodyLimitLayer;

/// The largest command body the API reads, over HTTP or a WebSocket text frame. Commands are
/// small JSON objects.
const MAX_BODY: usize = 64 * 1024;
/// How many WebSocket clients one listener serves at once. Past this, upgrades fail closed.
const MAX_CONNECTIONS: usize = 8;
/// How often a live WebSocket connection is pinged. A failed send ends the connection, so a
/// dead peer holds its slot for at most one interval past its death. Idle-but-live connections
/// stay: a connected remote outlives a paused player.
const HEARTBEAT: Duration = Duration::from_secs(30);

/// Remote control: versioned JSON over authenticated loopback HTTP, plus live snapshots,
/// commands and events over WebSocket upgrades on the same listener.
pub struct RemotePlugin;

#[async_trait::async_trait]
impl Plugin for RemotePlugin {
    fn id(&self) -> &'static str {
        "remote"
    }

    async fn serve(
        &self,
        ctx: PluginContext,
        listener: tokio::net::TcpListener,
        shutdown: oneshot::Receiver<()>,
    ) {
        let (stop_tx, stop_rx) = watch::channel(false);
        let state = RemoteState {
            ctx,
            stop: stop_rx,
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            heartbeat: HEARTBEAT,
        };
        serve_on(listener, state, shutdown, stop_tx).await;
    }
}

/// Serves `listener` until `shutdown` fires. Tests pass a custom state through here.
async fn serve_on(
    listener: tokio::net::TcpListener,
    state: RemoteState,
    shutdown: oneshot::Receiver<()>,
    stop: watch::Sender<bool>,
) {
    let app = Router::new()
        .route("/v1/state", get(snapshot))
        .route("/v1/playback", get(playback))
        .route("/v1/queue", get(queue))
        .route("/v1/commands", post(commands))
        .route("/v1/ws", get(upgrade))
        .fallback(unknown)
        .layer(RequestBodyLimitLayer::new(MAX_BODY))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .with_state(state);
    if let Err(error) = axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(async move {
            shutdown.await.ok();
            stop.send(true).ok();
        })
        .await
    {
        log::warn!("remote: serve ended: {error}");
    }
}

#[derive(Clone)]
struct RemoteState {
    ctx: PluginContext,
    stop: watch::Receiver<bool>,
    slots: Arc<Semaphore>,
    heartbeat: Duration,
}

async fn require_auth(State(st): State<RemoteState>, request: Request, next: Next) -> Response {
    // Open mode skips every check, origins included: the user turned the token off knowing any
    // local client, browser pages among them, can drive playback.
    if !st.ctx.auth_required {
        return next.run(request).await;
    }
    match authorized(&st.ctx, request.headers()) {
        true => next.run(request).await,
        false => unauthorized(),
    }
}

/// Whether the request headers carry the session credential.
fn authorized(ctx: &PluginContext, headers: &HeaderMap) -> bool {
    let header = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    ctx.auth.check(header)
}

/// The 401 answer for missing or wrong credentials.
fn unauthorized() -> Response {
    let mut denied = reply(
        StatusCode::UNAUTHORIZED,
        ErrorReply::new(ControlError::permission_denied(
            "missing or invalid bearer token",
        )),
    );
    denied
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    denied
}

async fn snapshot(State(st): State<RemoteState>) -> Response {
    match st.ctx.client.snapshot().await {
        Ok(snapshot) => reply(StatusCode::OK, SnapshotReply::new(snapshot)),
        Err(error) => failure(error.into()),
    }
}

async fn playback(State(st): State<RemoteState>) -> Response {
    match st.ctx.client.snapshot().await {
        Ok(snapshot) => reply(
            StatusCode::OK,
            PlaybackReply::new(snapshot.snapshot_revision, snapshot.playback),
        ),
        Err(error) => failure(error.into()),
    }
}

async fn queue(State(st): State<RemoteState>) -> Response {
    match st.ctx.client.snapshot().await {
        Ok(snapshot) => reply(
            StatusCode::OK,
            QueueReply::new(snapshot.snapshot_revision, snapshot.queue),
        ),
        Err(error) => failure(error.into()),
    }
}

async fn commands(
    State(st): State<RemoteState>,
    command: Result<Json<Command>, JsonRejection>,
) -> Response {
    let command = match command {
        Ok(Json(command)) => command,
        Err(rejection) => return rejected(rejection),
    };
    match st.ctx.client.execute(command).await {
        Ok(accepted) => reply(StatusCode::OK, SuccessReply::ok(accepted.snapshot_revision)),
        Err(error) => failure(error.into()),
    }
}

async fn upgrade(
    State(st): State<RemoteState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if st.ctx.auth_required && headers.contains_key(header::ORIGIN) {
        return failure(ControlError::permission_denied(
            "browser clients are not supported",
        ));
    }
    let permit = match st.slots.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return failure(ControlError::resource_exhausted("too many connections"));
        }
    };
    let ctx = st.ctx.clone();
    let stop = st.stop.clone();
    let heartbeat = st.heartbeat;
    ws.max_message_size(MAX_BODY)
        .on_upgrade(move |socket| connection(ctx, stop, socket, heartbeat, permit))
}

async fn unknown() -> Response {
    failure(ControlError::not_found("no such endpoint"))
}

/// Maps a JSON rejection to the versioned error envelope. Unknown commands fail
/// deserialization, so they land here as invalid arguments.
fn rejected(rejection: JsonRejection) -> Response {
    match rejection {
        JsonRejection::MissingJsonContentType(_) => reply(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorReply::new(ControlError::invalid_argument("expected application/json")),
        ),
        _ => reply(
            StatusCode::BAD_REQUEST,
            ErrorReply::new(ControlError::invalid_argument("invalid command body")),
        ),
    }
}

/// Maps a control failure to its status code and the versioned error envelope.
fn failure(error: ControlError) -> Response {
    let status = match error.code {
        ErrorCode::InvalidArgument => StatusCode::BAD_REQUEST,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    reply(status, ErrorReply::new(error))
}

/// A JSON response nothing is allowed to cache.
fn reply(status: StatusCode, body: impl Serialize) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Serves one WebSocket client: a snapshot first, then commands, events and resyncs until
/// either side hangs up. Sends are direct, so a slow client stalls only itself; broadcast lag
/// resyncs it. A ping every `heartbeat` drops dead peers; `permit` frees its connection slot
/// when this returns.
async fn connection(
    ctx: PluginContext,
    mut stop: watch::Receiver<bool>,
    mut socket: WebSocket,
    heartbeat: Duration,
    _permit: OwnedSemaphorePermit,
) {
    if *stop.borrow_and_update() {
        return;
    }
    let mut events = ctx.client.subscribe();
    let snapshot = match ctx.client.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(_) => return,
    };
    if !send(
        &mut socket,
        &WsServerMessage::Snapshot {
            v: API_VERSION,
            data: Box::new(snapshot),
        },
    )
    .await
    {
        return;
    }
    let mut beat = tokio::time::interval(heartbeat);
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires at once; the loop wants the next one.
    beat.tick().await;
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = beat.tick() => {
                if socket
                    .send(Message::Ping(Vec::<u8>::new().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            message = socket.recv() => {
                let Some(message) = message else { break };
                match message {
                    Ok(Message::Text(text)) => {
                        if !command(&ctx, &mut socket, &text).await {
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            event = events.recv() => {
                match event {
                    Ok(sequenced) => {
                        let message = WsServerMessage::Event {
                            v: API_VERSION,
                            seq: Some(sequenced.seq),
                            event: sequenced.event,
                        };
                        if !send(&mut socket, &message).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if !resync(&ctx, &mut socket, &mut events).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    socket.send(Message::Close(None)).await.ok();
}

/// Answers one text message. False ends the connection.
async fn command(ctx: &PluginContext, socket: &mut WebSocket, text: &str) -> bool {
    let command: WsCommand = match serde_json::from_str(text) {
        Ok(command) => command,
        Err(_) => {
            return send(
                socket,
                &WsServerMessage::Error {
                    v: API_VERSION,
                    data: WsErrorData {
                        error: ControlError::invalid_argument("invalid message"),
                    },
                },
            )
            .await;
        }
    };
    if command.v != API_VERSION {
        return result(
            socket,
            &command.id,
            WsResultData::Failed {
                error: ControlError::invalid_argument("unsupported version"),
            },
        )
        .await;
    }
    let data = match ctx.client.execute(command.data).await {
        Ok(accepted) => WsResultData::Accepted {
            accepted: accepted.accepted,
            snapshot_revision: accepted.snapshot_revision,
        },
        Err(error) => WsResultData::Failed {
            error: ControlError::from(error),
        },
    };
    result(socket, &command.id, data).await
}

async fn result(socket: &mut WebSocket, id: &str, data: WsResultData) -> bool {
    send(
        socket,
        &WsServerMessage::Result {
            v: API_VERSION,
            id: id.to_owned(),
            data,
        },
    )
    .await
}

/// Resubscribes a lagging client: a seq-less resync marker, then the fresh snapshot it
/// continues from. The receiver is replaced, so nothing stale replays afterwards.
async fn resync(
    ctx: &PluginContext,
    socket: &mut WebSocket,
    events: &mut broadcast::Receiver<control::channel::SequencedEvent>,
) -> bool {
    let Ok(snapshot) = ctx.client.snapshot().await else {
        return false;
    };
    let revision = snapshot.snapshot_revision;
    let ok = send(
        socket,
        &WsServerMessage::Event {
            v: API_VERSION,
            seq: None,
            event: ControlEvent::ResyncRequired {
                snapshot_revision: revision,
            },
        },
    )
    .await
        && send(
            socket,
            &WsServerMessage::Snapshot {
                v: API_VERSION,
                data: Box::new(snapshot),
            },
        )
        .await;
    *events = ctx.client.subscribe();
    ok
}

async fn send(socket: &mut WebSocket, message: &WsServerMessage) -> bool {
    let Ok(text) = serde_json::to_string(message) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use control::channel::{CommandResult, Publisher, Request};
    use control::{
        API_VERSION, AppSnapshot, Command, CommandReply, PlaybackSnapshot, PlaybackStatus,
        QueueSnapshot, RepeatMode, SessionSnapshot, SessionStatus, WsCommandKind,
    };
    use futures::{SinkExt as _, StreamExt as _};
    use state::Auth;
    use tokio::task::JoinHandle;
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    use super::*;

    fn pictured() -> AppSnapshot {
        AppSnapshot {
            snapshot_revision: 3,
            playback: PlaybackSnapshot {
                status: PlaybackStatus::Playing,
                track: None,
                position_ms: 90_000,
                duration_ms: None,
                volume: 0.6,
                repeat: RepeatMode::All,
                shuffle: true,
            },
            queue: QueueSnapshot {
                revision: 17,
                past: Vec::new(),
                current: None,
                upcoming: Vec::new(),
                suggested: Vec::new(),
                manual_count: 0,
                shuffle: true,
            },
            session: SessionSnapshot {
                status: SessionStatus::SignedIn,
                provider: Some("spotify".to_owned()),
            },
        }
    }

    fn accept() -> CommandResult {
        Ok(CommandReply {
            accepted: true,
            snapshot_revision: 9,
        })
    }

    /// A stub host answering snapshots with `pictured` and every command with `verdict`.
    /// Commands are recorded, and `publish` emits events on demand.
    struct Stub {
        ctx: PluginContext,
        publish: Publisher,
        seen: Arc<Mutex<Vec<Command>>>,
    }

    fn stub(verdict: CommandResult) -> Stub {
        let (host, client) = control::channel::pair(pictured());
        let (mut requests, publish) = host.split();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            while let Some(request) = requests.recv().await {
                match request {
                    Request::GetSnapshot { reply, .. } => {
                        let _ = reply.send(pictured());
                    }
                    Request::Execute { command, reply, .. } => {
                        recorded.lock().unwrap().push(command);
                        let _ = reply.send(verdict.clone());
                    }
                }
            }
        });
        let ctx = PluginContext {
            client,
            auth: Auth::new(),
            auth_required: true,
        };
        ctx.auth.ensure();
        Stub { ctx, publish, seen }
    }

    fn stub_token(stub: &Stub) -> String {
        stub.ctx.auth.token().expect("a minted credential")
    }

    async fn serve(ctx: PluginContext) -> (SocketAddr, oneshot::Sender<()>, JoinHandle<()>) {
        serve_with(ctx, MAX_CONNECTIONS, HEARTBEAT).await
    }

    async fn serve_with(
        ctx: PluginContext,
        slots: usize,
        heartbeat: Duration,
    ) -> (SocketAddr, oneshot::Sender<()>, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback listener");
        let addr = listener.local_addr().expect("a bound address");
        assert!(addr.ip().is_loopback());
        let (stop_tx, stop_rx) = watch::channel(false);
        let state = RemoteState {
            ctx,
            stop: stop_rx,
            slots: Arc::new(Semaphore::new(slots)),
            heartbeat,
        };
        let (shutdown, rx) = oneshot::channel();
        let task = tokio::spawn(async move { serve_on(listener, state, rx, stop_tx).await });
        (addr, shutdown, task)
    }

    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn connect(addr: SocketAddr, headers: &[(&str, &str)]) -> Client {
        let (stream, _) = tokio_tungstenite::connect_async(upgrade_request(addr, headers))
            .await
            .expect("an upgrade");
        stream
    }

    fn upgrade_request(addr: SocketAddr, headers: &[(&str, &str)]) -> axum::http::Request<()> {
        let mut request = axum::http::Request::builder()
            .uri(format!("ws://{addr}/v1/ws"))
            .header("host", addr.to_string())
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.body(()).expect("a request")
    }

    /// Reads the next text message as JSON.
    async fn next_json(stream: &mut Client) -> serde_json::Value {
        match stream.next().await {
            Some(Ok(ClientMessage::Text(text))) => serde_json::from_str(&text).expect("json"),
            other => panic!("expected text, saw {other:?}"),
        }
    }

    fn command(id: &str, command: Command) -> String {
        serde_json::to_string(&WsCommand {
            v: API_VERSION,
            id: id.to_owned(),
            kind: WsCommandKind::Command,
            data: command,
        })
        .expect("a command")
    }

    fn send_command(
        client: &reqwest::Client,
        addr: SocketAddr,
        token: &str,
        body: &str,
    ) -> reqwest::RequestBuilder {
        client
            .post(format!("http://{addr}/v1/commands"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(body.to_owned())
    }

    #[tokio::test]
    async fn one_listener_serves_rest_and_websocket() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let http = reqwest::Client::new();

        let state = http
            .get(format!("http://{addr}/v1/state"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("a response");
        assert_eq!(state.status(), StatusCode::OK);

        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        assert_eq!(next_json(&mut stream).await["type"], "snapshot");
    }

    #[tokio::test]
    async fn open_mode_serves_without_credentials() {
        let mut stub = stub(accept());
        stub.ctx.auth_required = false;
        let seen = stub.seen.clone();
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let client = reqwest::Client::new();

        let open = client
            .get(format!("http://{addr}/v1/state"))
            .send()
            .await
            .expect("a response");
        assert_eq!(open.status(), StatusCode::OK);
        let body: SnapshotReply = open.json().await.expect("a snapshot");
        assert_eq!(body.snapshot.snapshot_revision, 3);

        let ran = client
            .post(format!("http://{addr}/v1/commands"))
            .header("content-type", "application/json")
            .body(r#"{"command":"toggle"}"#)
            .send()
            .await
            .expect("a response");
        assert_eq!(ran.status(), StatusCode::OK);
        assert_eq!(*seen.lock().unwrap(), [Command::Toggle]);
    }

    #[tokio::test]
    async fn rejects_unauthenticated_requests() {
        let stub = stub(accept());
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let client = reqwest::Client::new();

        let denied = client
            .get(format!("http://{addr}/v1/state"))
            .send()
            .await
            .expect("a response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            denied
                .headers()
                .get("www-authenticate")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer"
        );
        let body: ErrorReply = denied.json().await.expect("an error envelope");
        assert_eq!(body.v, 1);
        assert_eq!(body.error.code, ErrorCode::PermissionDenied);

        let denied = client
            .get(format!("http://{addr}/v1/state"))
            .header("authorization", "Bearer wrong")
            .send()
            .await
            .expect("a response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let denied = client
            .get(format!("http://{addr}/nope"))
            .send()
            .await
            .expect("a response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn serves_snapshots() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let client = reqwest::Client::new();
        let get = |path: &str| {
            client
                .get(format!("http://{addr}{path}"))
                .header("authorization", format!("Bearer {token}"))
        };

        let state = get("/v1/state").send().await.expect("a response");
        assert_eq!(state.status(), StatusCode::OK);
        assert_eq!(
            state
                .headers()
                .get("cache-control")
                .unwrap()
                .to_str()
                .unwrap(),
            "no-store"
        );
        let state: SnapshotReply = state.json().await.expect("a snapshot");
        assert_eq!(state.v, 1);
        assert_eq!(state.snapshot.snapshot_revision, 3);
        assert_eq!(state.snapshot.playback.status, PlaybackStatus::Playing);
        assert_eq!(state.snapshot.queue.revision, 17);

        let playback = get("/v1/playback").send().await.expect("a response");
        assert_eq!(playback.status(), StatusCode::OK);
        let playback: PlaybackReply = playback.json().await.expect("a section");
        assert_eq!(playback.snapshot_revision, 3);
        assert_eq!(playback.playback.volume, 0.6);

        let queue = get("/v1/queue").send().await.expect("a response");
        assert_eq!(queue.status(), StatusCode::OK);
        let queue: QueueReply = queue.json().await.expect("a section");
        assert_eq!(queue.snapshot_revision, 3);
        assert_eq!(queue.queue.revision, 17);
    }

    #[tokio::test]
    async fn forwards_commands_once() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let seen = stub.seen.clone();
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let client = reqwest::Client::new();

        let done = send_command(&client, addr, &token, r#"{"command":"toggle"}"#)
            .send()
            .await
            .expect("a response");
        assert_eq!(done.status(), StatusCode::OK);
        let done: SuccessReply = done.json().await.expect("a reply");
        assert!(done.accepted);
        assert_eq!(done.snapshot_revision, 9);

        let done = send_command(
            &client,
            addr,
            &token,
            r#"{"command":"queue_remove_upcoming","index":2,"expected_revision":17}"#,
        )
        .send()
        .await
        .expect("a response");
        assert_eq!(done.status(), StatusCode::OK);

        let seen = seen.lock().unwrap();
        assert_eq!(
            *seen,
            [
                Command::Toggle,
                Command::QueueRemoveUpcoming {
                    index: 2,
                    expected_revision: 17,
                },
            ]
        );
    }

    #[tokio::test]
    async fn maps_errors_to_status() {
        let refused = Err(ControlError::conflict(
            "queue changed; request a new snapshot",
        ));
        let stub = stub(refused);
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let client = reqwest::Client::new();

        let conflicted = send_command(&client, addr, &token, r#"{"command":"next"}"#)
            .send()
            .await
            .expect("a response");
        assert_eq!(conflicted.status(), StatusCode::CONFLICT);
        let body: ErrorReply = conflicted.json().await.expect("an error envelope");
        assert_eq!(body.error.code, ErrorCode::Conflict);

        for body in [r#"{"command":"#, r#"{"command":"launch"}"#] {
            let invalid = send_command(&client, addr, &token, body)
                .send()
                .await
                .expect("a response");
            assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
            let body: ErrorReply = invalid.json().await.expect("an error envelope");
            assert_eq!(body.error.code, ErrorCode::InvalidArgument);
        }

        let typed = client
            .post(format!("http://{addr}/v1/commands"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "text/plain")
            .body(r#"{"command":"toggle"}"#)
            .send()
            .await
            .expect("a response");
        assert_eq!(typed.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let missing = client
            .get(format!("http://{addr}/v1/nothing"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("a response");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let body: ErrorReply = missing.json().await.expect("an error envelope");
        assert_eq!(body.error.code, ErrorCode::NotFound);

        let huge = " ".repeat(70 * 1024);
        let limited = send_command(&client, addr, &token, &huge)
            .send()
            .await
            .expect("a response");
        assert_eq!(limited.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn failures_map_to_their_status() {
        for (error, status) in [
            (
                ControlError::invalid_argument("bad"),
                StatusCode::BAD_REQUEST,
            ),
            (ControlError::permission_denied("no"), StatusCode::FORBIDDEN),
            (ControlError::not_found("gone"), StatusCode::NOT_FOUND),
            (ControlError::conflict("moved"), StatusCode::CONFLICT),
            (
                ControlError::resource_exhausted("slow down"),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                ControlError::unavailable("later"),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                ControlError::internal("oops"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            assert_eq!(failure(error).status(), status);
        }
    }

    #[tokio::test]
    async fn stops_gracefully() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, shutdown, serve) = serve(stub.ctx).await;
        let client = reqwest::Client::new();

        let state = client
            .get(format!("http://{addr}/v1/state"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("a response");
        assert_eq!(state.status(), StatusCode::OK);

        shutdown.send(()).expect("shutdown lands");
        tokio::time::timeout(Duration::from_secs(5), serve)
            .await
            .expect("serve ends")
            .expect("serve joins");

        assert!(
            client
                .get(format!("http://{addr}/v1/state"))
                .header("authorization", format!("Bearer {token}"))
                .send()
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn upgrade_requires_auth_and_native_clients() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;

        let error = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/ws"))
            .await
            .expect_err("no upgrade without credentials");
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == axum::http::StatusCode::UNAUTHORIZED
        ));

        let request = upgrade_request(addr, &[("authorization", "Bearer wrong")]);
        let error = tokio_tungstenite::connect_async(request)
            .await
            .expect_err("no upgrade on a wrong credential");
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == axum::http::StatusCode::UNAUTHORIZED
        ));

        let bearer = format!("Bearer {token}");
        let request = upgrade_request(
            addr,
            &[("authorization", &bearer), ("origin", "http://example.com")],
        );
        let error = tokio_tungstenite::connect_async(request)
            .await
            .expect_err("no upgrade for browser origins");
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == axum::http::StatusCode::FORBIDDEN
        ));
    }

    #[tokio::test]
    async fn upgrades_close_past_the_cap() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve_with(stub.ctx, 2, Duration::from_secs(30)).await;
        let auth = format!("Bearer {token}");

        let mut first = connect(addr, &[("authorization", auth.as_str())]).await;
        assert_eq!(next_json(&mut first).await["type"], "snapshot");
        let mut second = connect(addr, &[("authorization", auth.as_str())]).await;
        assert_eq!(next_json(&mut second).await["type"], "snapshot");

        let request = upgrade_request(addr, &[("authorization", auth.as_str())]);
        let error = tokio_tungstenite::connect_async(request)
            .await
            .expect_err("no upgrade past the cap");
        assert!(matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == axum::http::StatusCode::TOO_MANY_REQUESTS
        ));

        // A disconnect frees its slot.
        first.close(None).await.ok();
        drop(first);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let request = upgrade_request(addr, &[("authorization", auth.as_str())]);
            match tokio_tungstenite::connect_async(request).await {
                Ok((mut stream, _)) => {
                    assert_eq!(next_json(&mut stream).await["type"], "snapshot");
                    break;
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("no upgrade after a disconnect: {error}"),
            }
        }
    }

    #[tokio::test]
    async fn idle_connections_are_pinged() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve_with(stub.ctx, 8, Duration::from_millis(50)).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;

        let mut snapshot = false;
        let mut pinged = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !snapshot || !pinged {
            let message = tokio::time::timeout_at(deadline, stream.next())
                .await
                .expect("a frame arrives")
                .expect("the connection stays up")
                .expect("a clean frame");
            match message {
                ClientMessage::Text(text) => {
                    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
                    assert_eq!(json["type"], "snapshot");
                    snapshot = true;
                }
                ClientMessage::Ping(_) => pinged = true,
                other => panic!("expected a snapshot or a ping, saw {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn open_mode_upgrades_without_credentials_or_origin_checks() {
        let mut stub = stub(accept());
        stub.ctx.auth_required = false;
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;

        let mut plain = connect(addr, &[]).await;
        assert_eq!(next_json(&mut plain).await["type"], "snapshot");

        let mut browser = connect(addr, &[("origin", "http://example.com")]).await;
        assert_eq!(next_json(&mut browser).await["type"], "snapshot");
    }

    #[tokio::test]
    async fn snapshot_first_then_results() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;

        let snapshot = next_json(&mut stream).await;
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["v"], 1);
        assert_eq!(snapshot["data"]["snapshot_revision"], 3);

        stream
            .send(ClientMessage::text(command("a", Command::Toggle)))
            .await
            .expect("a command goes out");
        let result = next_json(&mut stream).await;
        assert_eq!(result["type"], "result");
        assert_eq!(result["id"], "a");
        assert_eq!(result["data"]["accepted"], true);
        assert_eq!(result["data"]["snapshot_revision"], 9);
        assert_eq!(*stub.seen.lock().unwrap(), [Command::Toggle]);
    }

    #[tokio::test]
    async fn command_errors_mirror_rest() {
        let refused = Err(ControlError::conflict(
            "queue changed; request a new snapshot",
        ));
        let stub = stub(refused);
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        next_json(&mut stream).await;

        stream
            .send(ClientMessage::text(command("b", Command::Next)))
            .await
            .expect("a command goes out");
        let result = next_json(&mut stream).await;
        assert_eq!(result["id"], "b");
        assert_eq!(result["data"]["error"]["code"], "conflict");
        assert_eq!(
            result["data"]["error"]["message"],
            "queue changed; request a new snapshot"
        );
    }

    #[tokio::test]
    async fn invalid_messages_get_errors_not_disconnects() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        next_json(&mut stream).await;

        for message in [
            "not json".to_owned(),
            r#"{"v":1,"id":"x","type":"subscribe","data":{}}"#.to_owned(),
        ] {
            stream
                .send(ClientMessage::text(message))
                .await
                .expect("a message goes out");
            let error = next_json(&mut stream).await;
            assert_eq!(error["type"], "error");
            assert_eq!(error["data"]["error"]["code"], "invalid_argument");
        }

        stream
            .send(ClientMessage::text(
                r#"{"v":99,"id":"old","type":"command","data":{"command":"toggle"}}"#,
            ))
            .await
            .expect("a message goes out");
        let result = next_json(&mut stream).await;
        assert_eq!(result["type"], "result");
        assert_eq!(result["id"], "old");
        assert_eq!(result["data"]["error"]["code"], "invalid_argument");
        assert!(stub.seen.lock().unwrap().is_empty());

        stream
            .send(ClientMessage::text(command("c", Command::Toggle)))
            .await
            .expect("a command goes out");
        let result = next_json(&mut stream).await;
        assert_eq!(result["id"], "c");
        assert_eq!(result["data"]["accepted"], true);
    }

    #[tokio::test]
    async fn oversized_messages_close_the_connection() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        next_json(&mut stream).await;

        stream
            .send(ClientMessage::text(" ".repeat(70 * 1024)))
            .await
            .expect("a message goes out");
        match stream.next().await {
            Some(Ok(ClientMessage::Close(_))) | Some(Err(_)) | None => {}
            other => panic!("expected a close, saw {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_stream_and_lag_resyncs() {
        use control::channel::SequencedEvent;

        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        next_json(&mut stream).await;

        for seq in 1..=3 {
            stub.publish.emit(SequencedEvent {
                seq,
                event: ControlEvent::PlaybackChanged {
                    snapshot_revision: 3,
                    status: PlaybackStatus::Playing,
                },
            });
        }
        for seq in 1..=3 {
            let event = next_json(&mut stream).await;
            assert_eq!(event["type"], "event");
            assert_eq!(event["seq"], seq);
            assert_eq!(event["event"], "playback.changed");
        }

        for seq in 4..100_004 {
            stub.publish.emit(SequencedEvent {
                seq,
                event: ControlEvent::PositionChanged {
                    snapshot_revision: 3,
                    position_ms: seq,
                },
            });
        }
        let mut resynced = false;
        for _ in 0..1000 {
            let message = next_json(&mut stream).await;
            if message["event"] == "resync.required" {
                assert_eq!(message["type"], "event");
                assert!(message.get("seq").is_none());
                resynced = true;
                break;
            }
        }
        assert!(resynced);
        let snapshot = next_json(&mut stream).await;
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["data"]["snapshot_revision"], 3);
    }

    #[tokio::test]
    async fn binary_frames_are_ignored() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, _shutdown, _serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        next_json(&mut stream).await;

        stream
            .send(ClientMessage::binary(vec![0, 1, 2]))
            .await
            .expect("a frame goes out");
        tokio::time::timeout(Duration::from_millis(200), stream.next())
            .await
            .expect_err("no answer to binary");

        stream
            .send(ClientMessage::text(command("d", Command::Toggle)))
            .await
            .expect("a command goes out");
        let result = next_json(&mut stream).await;
        assert_eq!(result["id"], "d");
    }

    #[tokio::test]
    async fn shutdown_closes_connections() {
        let stub = stub(accept());
        let token = stub_token(&stub);
        let (addr, shutdown, serve) = serve(stub.ctx).await;
        let mut stream = connect(addr, &[("authorization", &format!("Bearer {token}"))]).await;
        next_json(&mut stream).await;

        shutdown.send(()).expect("shutdown lands");
        tokio::time::timeout(Duration::from_secs(5), serve)
            .await
            .expect("serve ends")
            .expect("serve joins");
        match stream.next().await {
            Some(Ok(ClientMessage::Close(_))) | Some(Err(_)) | None => {}
            other => panic!("expected a close, saw {other:?}"),
        }
    }
}
