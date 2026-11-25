//! Remote control through the server's sessions: Sonora listed among the devices Jellyfin's apps
//! and dashboard can control, the account's other controllable apps to play on, and changes
//! another app makes to favorites and the library.
//!
//! Commands reach this device over the server's WebSocket, which also tells it when favorites
//! and the library change. The other sessions are read over HTTP every few seconds, since the
//! server only pushes them to administrators.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use futures::{SinkExt as _, StreamExt as _};
use music::connect::{
    Command, Connect, Device, DeviceKind, Elsewhere, Event, Invite, QueuedTrack, Repeat, Request,
    Roster, Snapshot, Start,
};
use reqwest::Method;
use serde::Deserialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use crate::client::JellyfinClient;
use crate::wire;

/// How often the other sessions are read.
const ROSTER_EVERY: Duration = Duration::from_secs(10);
/// Sessions that have not been heard from for this long are left off the device list.
const ACTIVE_WITHIN: u64 = 960;
/// How long to wait before opening the WebSocket again after it closed.
const RECONNECT: Duration = Duration::from_secs(5);
/// How often to say the socket is alive before the server first says how often it wants it.
const KEEP_ALIVE: Duration = Duration::from_secs(30);
/// How far a volume step from a remote moves the volume.
const VOLUME_STEP: f32 = 0.05;
/// How many upcoming tracks a transfer hands to the other app.
const CARRIED: usize = 100;
/// The commands this device carries out, as the server's capabilities name them.
const COMMANDS: [&str; 10] = [
    "SetVolume",
    "VolumeUp",
    "VolumeDown",
    "Mute",
    "Unmute",
    "ToggleMute",
    "SetShuffleQueue",
    "SetRepeatMode",
    "PlayState",
    "PlayNext",
];

/// Starts the worker of one signed-in account and answers its handle, which ends the worker
/// once the last copy drops. Jellyfin has no group sessions, so the handle offers none.
pub fn start(client: JellyfinClient) -> Arc<Connect> {
    let (connect, requests, out) = Connect::new(client.device_id().to_owned());
    tokio::spawn(
        Worker {
            client,
            out,
            shown: false,
            tasks: Vec::new(),
            shared: Arc::new(Shared::default()),
        }
        .run(requests),
    );
    Arc::new(connect.without_jams())
}

/// What the worker and its tasks share: what this device plays and the other sessions.
#[derive(Default)]
struct Shared {
    /// The last snapshot of this device's playback, which a transfer hands on.
    now: Mutex<Option<Arc<Snapshot>>>,
    /// The other sessions as last read, by session id.
    sessions: Mutex<Vec<Session>>,
    /// The volume a remote mute took away, which an unmute gives back.
    muted_from: Mutex<Option<f32>>,
}

struct Worker {
    client: JellyfinClient,
    out: UnboundedSender<Event>,
    /// Whether the listener wants this device listed.
    shown: bool,
    /// The socket and roster tasks, which run while the device is listed.
    tasks: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl Worker {
    async fn run(mut self, mut requests: UnboundedReceiver<Request>) {
        while let Some(request) = requests.recv().await {
            if let Err(error) = self.handle(request).await {
                log::warn!("jellyfin: remote control failed: {error:#}");
            }
        }
        self.stop();
    }

    async fn handle(&mut self, request: Request) -> Result<()> {
        match request {
            Request::Enable(on) => self.enable(on).await?,
            Request::Publish(now) => {
                *self
                    .shared
                    .now
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = now;
            }
            Request::Transfer(device) => self.transfer(&device).await?,
            Request::Control(device, command) => control(&self.client, &device, command).await?,
            Request::Peek(_) => {
                self.out.send(Event::Invite(None::<Invite>)).ok();
            }
            Request::Join(_) | Request::Start => {
                self.out.send(Event::JamFailed).ok();
            }
            Request::Leave => {
                self.out.send(Event::LeaveFailed).ok();
            }
            Request::Rename(_) | Request::Kick(_) => {}
        }
        Ok(())
    }

    /// Lists this device as controllable, or takes it off, and starts or stops listening.
    async fn enable(&mut self, on: bool) -> Result<()> {
        if on == self.shown {
            return Ok(());
        }
        self.stop();
        self.capabilities(on).await?;
        self.shown = on;
        if !on {
            self.out.send(Event::Roster(Roster::default())).ok();
            return Ok(());
        }
        let (client, out, shared) = (self.client.clone(), self.out.clone(), self.shared.clone());
        self.tasks.push(tokio::spawn(listen(client, out, shared)));
        let (client, out, shared) = (self.client.clone(), self.out.clone(), self.shared.clone());
        self.tasks.push(tokio::spawn(poll(client, out, shared)));
        Ok(())
    }

    fn stop(&mut self) {
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }

    /// Tells the server what this device can be asked to do. With media control off, the
    /// server's apps do not offer it as a device.
    async fn capabilities(&self, on: bool) -> Result<()> {
        let body = serde_json::json!({
            "PlayableMediaTypes": ["Audio"],
            "SupportedCommands": match on {
                true => COMMANDS.to_vec(),
                false => Vec::new(),
            },
            "SupportsMediaControl": on,
            "SupportsPersistentIdentifier": true,
        });
        self.client
            .request(Method::POST, "/Sessions/Capabilities/Full", Some(body))
            .await
            .context("cannot tell jellyfin what this device can do")
    }

    /// Moves playback to `device`. To this device it takes over what the other app plays and
    /// stops it there. To another app it hands over the queue and the place in the track, and
    /// lets go of it here.
    async fn transfer(&self, device: &str) -> Result<()> {
        if device == self.client.device_id() {
            let playing = playing(
                &self
                    .shared
                    .sessions
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            )
            .map(|(session, state)| (session.id.clone(), state));
            let Some((session, state)) = playing else {
                return Ok(());
            };
            self.out
                .send(Event::Command(Command::Start(Start {
                    state: Arc::new(state),
                    preserve_queue: true,
                    restart: true,
                })))
                .ok();
            let path = format!("/Sessions/{}/Playing/Stop", session);
            return self.client.request(Method::POST, &path, None).await;
        }
        let now = self
            .shared
            .now
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(now) = now else {
            return Ok(());
        };
        play_on(&self.client, device, &now, now.live_position()).await?;
        self.out.send(Event::Command(Command::Released)).ok();
        Ok(())
    }
}

/// One of the account's other sessions, as `GET /Sessions` lists it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct Session {
    id: String,
    /// When the server reported this state, which the position was measured at.
    #[serde(skip)]
    seen: Option<SystemTime>,
    device_id: Option<String>,
    device_name: Option<String>,
    client: Option<String>,
    supports_remote_control: bool,
    now_playing_item: Option<wire::Item>,
    play_state: PlayState,
    now_playing_queue: Vec<Queued>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct PlayState {
    position_ticks: Option<u64>,
    is_paused: bool,
    volume_level: Option<u32>,
    repeat_mode: Option<String>,
    shuffle_mode: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct Queued {
    id: String,
}

impl Session {
    fn device(&self) -> Device {
        let client = self.client.clone().unwrap_or_default();
        let name = self
            .device_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| client.clone());
        Device {
            id: self.id.clone(),
            name: match client.is_empty() || name == client {
                true => name,
                false => format!("{client} on {name}"),
            },
            kind: kind(&client),
            volume: self.play_state.volume_level.unwrap_or(100) as f32 / 100.,
        }
    }

    /// What the session plays, when it plays a song.
    fn snapshot(&self) -> Option<Snapshot> {
        let item = self
            .now_playing_item
            .as_ref()
            .filter(|item| item.is_audio())?;
        let ids: Vec<&str> = self
            .now_playing_queue
            .iter()
            .map(|queued| queued.id.as_str())
            .collect();
        let at = ids.iter().position(|id| *id == item.id);
        let queued = |ids: &[&str]| -> Vec<QueuedTrack> {
            ids.iter()
                .map(|id| QueuedTrack::new((*id).to_owned(), false))
                .collect()
        };
        let (previous, upcoming) = match at {
            Some(at) => (queued(&ids[..at]), queued(&ids[at + 1..])),
            None => (Vec::new(), Vec::new()),
        };
        Some(Snapshot {
            track: Some(QueuedTrack::new(item.id.clone(), false)),
            previous,
            upcoming,
            context: None,
            index: at,
            playing: !self.play_state.is_paused,
            position: wire::ticks(self.play_state.position_ticks.unwrap_or(0)),
            duration: item.duration(),
            volume: self.play_state.volume_level.unwrap_or(100) as f32 / 100.,
            shuffle: self
                .play_state
                .shuffle_mode
                .as_deref()
                .map(|mode| mode == "Shuffle"),
            repeat: self.play_state.repeat_mode.as_deref().map(repeat),
            stamp: Some(self.seen.unwrap_or_else(SystemTime::now)),
        })
    }
}

/// One message on the server's WebSocket.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct Notice {
    message_type: String,
    data: serde_json::Value,
}

/// Keeps the WebSocket open while the device is listed, opening it again whenever it closes,
/// and turns what it carries into commands and library changes.
async fn listen(client: JellyfinClient, out: UnboundedSender<Event>, shared: Arc<Shared>) {
    loop {
        if let Err(error) = socket(&client, &out, &shared).await {
            log::info!("jellyfin: the remote control socket closed: {error:#}");
        }
        tokio::time::sleep(RECONNECT).await;
    }
}

async fn socket(
    client: &JellyfinClient,
    out: &UnboundedSender<Event>,
    shared: &Shared,
) -> Result<()> {
    let address = client.server().replacen("http", "ws", 1) + "/socket";
    let mut request = address
        .as_str()
        .into_client_request()
        .context("cannot build the socket request")?;
    request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        client
            .header
            .parse()
            .context("cannot put the token in the socket request")?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("cannot open the jellyfin socket")?;
    log::info!("jellyfin: listening for remote control");
    let mut beat = tokio::time::interval(KEEP_ALIVE);
    beat.tick().await;
    loop {
        tokio::select! {
            _ = beat.tick() => {
                socket
                    .send(Message::text(r#"{"MessageType":"KeepAlive"}"#))
                    .await
                    .context("cannot keep the socket alive")?;
            }
            message = socket.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => anyhow::bail!("the server closed it"),
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => return Err(error).context("the socket broke"),
                };
                let Ok(message) = serde_json::from_str::<Notice>(&text) else {
                    continue;
                };
                if message.message_type == "ForceKeepAlive" {
                    let every = message.data.as_u64().unwrap_or(60).max(10) / 2;
                    beat = tokio::time::interval(Duration::from_secs(every));
                    beat.tick().await;
                    continue;
                }
                for event in events(client, shared, message).await {
                    out.send(event).ok();
                }
            }
        }
    }
}

/// What one socket message asks of the app.
async fn events(client: &JellyfinClient, shared: &Shared, message: Notice) -> Vec<Event> {
    let data = message.data;
    let text = |key: &str| data.get(key).and_then(|value| value.as_str()).unwrap_or("");
    let commands = match message.message_type.as_str() {
        "Playstate" => {
            let playing = shared
                .now
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .is_some_and(|now| now.playing);
            match text("Command") {
                "Pause" => vec![Command::Pause],
                "Unpause" => vec![Command::Play],
                "PlayPause" => vec![match playing {
                    true => Command::Pause,
                    false => Command::Play,
                }],
                "Stop" => vec![Command::Pause],
                "NextTrack" => vec![Command::Next],
                "PreviousTrack" => vec![Command::Previous],
                "Seek" => data
                    .get("SeekPositionTicks")
                    .and_then(serde_json::Value::as_u64)
                    .map(|ticks| Command::Seek(wire::ticks(ticks)))
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        }
        "GeneralCommand" => general(&data, shared),
        "Play" => play(client, &data).await,
        "UserDataChanged" => return liked(client, &data).await,
        "LibraryChanged" => return vec![Event::LibraryChanged],
        _ => Vec::new(),
    };
    commands.into_iter().map(Event::Command).collect()
}

/// A general command: the volume, shuffle and repeat.
fn general(data: &serde_json::Value, shared: &Shared) -> Vec<Command> {
    let argument = |key: &str| {
        data.get("Arguments")
            .and_then(|arguments| arguments.get(key))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let volume = shared
        .now
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map_or(1., |now| now.volume);
    let command = match data.get("Name").and_then(|name| name.as_str()) {
        Some("SetVolume") => argument("Volume")
            .parse::<f32>()
            .ok()
            .map(|level| Command::Volume((level / 100.).clamp(0., 1.))),
        Some("VolumeUp") => Some(Command::Volume((volume + VOLUME_STEP).min(1.))),
        Some("VolumeDown") => Some(Command::Volume((volume - VOLUME_STEP).max(0.))),
        Some("Mute") => Some(mute(shared, volume)),
        Some("Unmute") => Some(unmute(shared)),
        Some("ToggleMute") => Some(match volume > 0. {
            true => mute(shared, volume),
            false => unmute(shared),
        }),
        Some("SetShuffleQueue") => Some(Command::Shuffle(argument("ShuffleMode") == "Shuffle")),
        Some("SetRepeatMode") => Some(Command::Repeat(repeat(&argument("RepeatMode")))),
        _ => None,
    };
    command.into_iter().collect()
}

/// Silences playback and keeps `volume`, which an unmute gives back. A second mute keeps the
/// volume from before the first.
fn mute(shared: &Shared, volume: f32) -> Command {
    let mut kept = shared
        .muted_from
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if volume > 0. {
        *kept = Some(volume);
    }
    Command::Volume(0.)
}

/// Gives back the volume the last mute took away, or full volume when nothing was kept.
fn unmute(shared: &Shared) -> Command {
    let kept = shared
        .muted_from
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    Command::Volume(kept.unwrap_or(1.))
}

/// A request to play items: now, next or last, or shuffled, or as an instant mix.
async fn play(client: &JellyfinClient, data: &serde_json::Value) -> Vec<Command> {
    let mut ids: Vec<String> = data
        .get("ItemIds")
        .and_then(|ids| ids.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let how = data
        .get("PlayCommand")
        .and_then(|how| how.as_str())
        .unwrap_or("PlayNow");
    // an album, a playlist or an artist is played as the songs in it
    ids = match client.songs_of(&ids).await {
        Ok(songs) if !songs.is_empty() => songs,
        Ok(_) => ids,
        Err(error) => {
            log::warn!("jellyfin: cannot read what a remote asked to play: {error:#}");
            ids
        }
    };
    if how == "PlayNext" || how == "PlayLast" {
        return ids
            .into_iter()
            .map(|id| Command::Enqueue { id, added_by: None })
            .collect();
    }
    if how == "PlayInstantMix"
        && let Some(seed) = ids.first()
    {
        match client.instant_mix(seed).await {
            Ok(mix) => ids = mix,
            Err(error) => log::warn!("jellyfin: cannot read the instant mix: {error:#}"),
        }
    }
    let start = data
        .get("StartIndex")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let Some(track) = ids.get(start).cloned() else {
        return Vec::new();
    };
    let position = data
        .get("StartPositionTicks")
        .and_then(serde_json::Value::as_u64)
        .map_or(Duration::ZERO, wire::ticks);
    let queued = |ids: &[String]| -> Vec<QueuedTrack> {
        ids.iter()
            .map(|id| QueuedTrack::new(id.clone(), false))
            .collect()
    };
    let state = Snapshot {
        track: Some(QueuedTrack::new(track, false)),
        previous: queued(&ids[..start]),
        upcoming: queued(&ids[start + 1..]),
        index: Some(start),
        playing: true,
        position,
        volume: 1.,
        // shuffle rides with the start, since a separate command would change the queue under
        // the load the start is waiting on and the start would then give up
        shuffle: (how == "PlayShuffle").then_some(true),
        ..Snapshot::default()
    };
    vec![Command::Start(Start {
        state: Arc::new(state),
        preserve_queue: true,
        restart: true,
    })]
}

/// Favorites another app changed. A song's change is a like the app shows at once, and any
/// other item's is a library change the app reads again.
async fn liked(client: &JellyfinClient, data: &serde_json::Value) -> Vec<Event> {
    let changes: Vec<(String, bool)> = data
        .get("UserDataList")
        .and_then(|list| list.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|change| {
                    Some((
                        change.get("ItemId")?.as_str()?.to_owned(),
                        change.get("IsFavorite")?.as_bool()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if changes.is_empty() {
        return Vec::new();
    }
    let ids: Vec<String> = changes.iter().map(|(id, _)| id.clone()).collect();
    let songs = client.audio_ids(&ids).await.unwrap_or_default();
    let mut events: Vec<Event> = changes
        .into_iter()
        .filter(|(id, _)| songs.contains(id))
        .map(|(track, liked)| Event::Liked { track, liked })
        .collect();
    if events.len() < ids.len() {
        events.push(Event::LibraryChanged);
    }
    events
}

/// Reads the other sessions every few seconds and reports them as the device list.
async fn poll(client: JellyfinClient, out: UnboundedSender<Event>, shared: Arc<Shared>) {
    loop {
        match sessions(&client).await {
            Ok(sessions) => {
                let roster = roster(&sessions);
                *shared
                    .sessions
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = sessions;
                out.send(Event::Roster(roster)).ok();
            }
            Err(error) => log::info!("jellyfin: cannot read the other sessions: {error:#}"),
        }
        tokio::time::sleep(ROSTER_EVERY).await;
    }
}

/// The account's other sessions that are still about.
async fn sessions(client: &JellyfinClient) -> Result<Vec<Session>> {
    let path = format!(
        "/Sessions?controllableByUserId={}&activeWithinSeconds={ACTIVE_WITHIN}",
        client.user_id()
    );
    let sessions: Vec<Session> = client.read(&path).await?;
    let seen = SystemTime::now();
    Ok(sessions
        .into_iter()
        .filter(|session| session.device_id.as_deref() != Some(client.device_id()))
        .map(|session| Session {
            seen: Some(seen),
            ..session
        })
        .collect())
}

/// The controllable sessions as devices, and the one of them that plays a song, if any. A
/// session that plays wins over one that is paused, and one this app cannot control is left
/// out, since its playback could not be steered from here.
fn roster(sessions: &[Session]) -> Roster {
    let devices = sessions
        .iter()
        .filter(|session| session.supports_remote_control)
        .map(Session::device)
        .collect();
    let elsewhere = playing(sessions).map(|(session, state)| Elsewhere {
        device: session.device(),
        state: Arc::new(state),
    });
    Roster { devices, elsewhere }
}

/// The controllable session whose song a transfer takes over, with what it plays. One that
/// plays wins over one that is paused, so the device list and a transfer pick the same one.
fn playing(sessions: &[Session]) -> Option<(&Session, Snapshot)> {
    sessions
        .iter()
        .filter(|session| session.supports_remote_control)
        .filter_map(|session| Some((session, session.snapshot()?)))
        .max_by_key(|(_, state)| state.playing)
}

/// Asks another session to do `command`.
async fn control(client: &JellyfinClient, session: &str, command: Command) -> Result<()> {
    let playing = |what: &str| format!("/Sessions/{session}/Playing/{what}");
    let general = |name: &str, arguments: serde_json::Value| {
        (
            format!("/Sessions/{session}/Command"),
            serde_json::json!({ "Name": name, "Arguments": arguments }),
        )
    };
    let (path, body) = match command {
        Command::Play => (playing("Unpause"), None),
        Command::Pause | Command::Released => (playing("Pause"), None),
        Command::Next => (playing("NextTrack"), None),
        Command::Previous => (playing("PreviousTrack"), None),
        Command::Seek(at) => (
            format!(
                "{}?seekPositionTicks={}",
                playing("Seek"),
                wire::to_ticks(at)
            ),
            None,
        ),
        Command::Volume(level) => {
            let (path, body) = general(
                "SetVolume",
                serde_json::json!({ "Volume": ((level * 100.).round() as u32).to_string() }),
            );
            (path, Some(body))
        }
        Command::Shuffle(on) => {
            let mode = match on {
                true => "Shuffle",
                false => "Sorted",
            };
            let (path, body) = general(
                "SetShuffleQueue",
                serde_json::json!({ "ShuffleMode": mode }),
            );
            (path, Some(body))
        }
        Command::Repeat(mode) => {
            let mode = match mode {
                Repeat::Off => "RepeatNone",
                Repeat::All => "RepeatAll",
                Repeat::One => "RepeatOne",
            };
            let (path, body) = general("SetRepeatMode", serde_json::json!({ "RepeatMode": mode }));
            (path, Some(body))
        }
        Command::Enqueue { id, .. } => (
            format!("/Sessions/{session}/Playing?playCommand=PlayLast&itemIds={id}"),
            None,
        ),
        Command::Start(start) => {
            return play_on(client, session, &start.state, start.state.position).await;
        }
        Command::SetQueue { .. } => return Ok(()),
    };
    client
        .request(Method::POST, &path, body)
        .await
        .with_context(|| format!("cannot control the jellyfin session {session}"))
}

/// Asks another session to play the snapshot's track and what follows it, from `at`.
async fn play_on(
    client: &JellyfinClient,
    session: &str,
    state: &Snapshot,
    at: Duration,
) -> Result<()> {
    let ids: Vec<&str> = state
        .track
        .iter()
        .chain(state.upcoming.iter().take(CARRIED))
        .map(|track| track.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let path = format!(
        "/Sessions/{session}/Playing?playCommand=PlayNow&itemIds={}&startPositionTicks={}",
        ids.join(","),
        wire::to_ticks(at)
    );
    client
        .request(Method::POST, &path, None)
        .await
        .with_context(|| format!("cannot hand playback to the jellyfin session {session}"))
}

/// What kind of device a Jellyfin app most likely runs on, from the app's name.
fn kind(client: &str) -> DeviceKind {
    let client = client.to_lowercase();
    match () {
        () if [
            "android",
            "ios",
            "finamp",
            "mobile",
            "swiftfin",
            "streamyfin",
        ]
        .iter()
        .any(|name| client.contains(name)) =>
        {
            DeviceKind::Phone
        }
        () if ["tv", "roku", "kodi", "kore", "fire"]
            .iter()
            .any(|name| client.contains(name)) =>
        {
            DeviceKind::Tv
        }
        () if [
            "web",
            "media player",
            "desktop",
            "jellyfin mpv",
            "feishin",
            "sonora",
        ]
        .iter()
        .any(|name| client.contains(name)) =>
        {
            DeviceKind::Computer
        }
        () => DeviceKind::Other,
    }
}

fn repeat(mode: &str) -> Repeat {
    match mode {
        "RepeatAll" => Repeat::All,
        "RepeatOne" => Repeat::One,
        _ => Repeat::Off,
    }
}
