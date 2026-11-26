//! Casting to the YouTube app on a Cast device, the way YouTube Music's own apps do it: the
//! device's YouTube receiver is launched, and playback is handed to it through the Lounge API. The
//! device then streams from YouTube itself, so it plays on after Sonora closes or the computer
//! sleeps. The Cast devices that run YouTube are the device list, and the one that plays is
//! steered from the player like any other device.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use mdns_sd::ServiceDaemon;
use music::connect::{
    Command, Connect, Device, Elsewhere, Event, Invite, QueuedTrack, Request, Roster, Snapshot,
    Start,
};
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use crate::googlecast::{self, Channel, Found, Seen};
use crate::lounge::{Account, Ended, Lounge, Screen};

/// The YouTube receiver of a device with a screen, then the YouTube Music receiver a speaker
/// runs instead, in the order a device is asked about them. Each names the theme a remote binds
/// with, since a screen turns away a remote of the other app.
const RECEIVERS: [(&str, &str); 2] = [("233637DE", "cl"), ("2DB7CC49", "m")];
/// The namespace the YouTube receiver answers its screen id on.
const MDX: &str = "urn:x-cast:com.google.youtube.mdx";
/// This app's id among the devices, which a transfer back here names.
const HERE: &str = "sonora";
/// What the screen calls this remote.
const REMOTE_NAME: &str = "Sonora";
/// How many upcoming tracks a handoff carries. Each is a request of its own.
const CARRIED: usize = 50;
/// How long to wait before polling a screen again after the poll failed.
const RETRY: Duration = Duration::from_secs(5);
/// How often a bind is tried on a screen whose app was only just launched, and how long apart.
const BIND_TRIES: usize = 3;
const BIND_WAIT: Duration = Duration::from_secs(2);
/// How often a device is asked whether it runs YouTube, and how long apart. A device that is
/// still starting up turns the connection away or says no before its apps are ready.
const ASK_TRIES: usize = 4;
const ASK_WAIT: Duration = Duration::from_secs(30);
const CONNECT: Duration = Duration::from_secs(10);
/// How long the screen's long poll may stay silent. The server sends a no-op well within it, so
/// silence this long means the connection died, as it does while the computer sleeps.
const SILENCE: Duration = Duration::from_secs(90);

/// Starts the worker and answers its handle, which ends the worker once the last copy drops.
/// Screens are bound signed in to `account`, without which a speaker plays nothing. The network
/// has no group sessions, so the handle offers none. `remote` is the id the screens know this
/// app by.
pub(crate) fn start(remote: String, account: Option<Account>) -> Arc<Connect> {
    let (connect, requests, out) = Connect::new(HERE.to_owned());
    let http = reqwest::Client::builder()
        .connect_timeout(CONNECT)
        .read_timeout(SILENCE)
        .build()
        .unwrap_or_default();
    let shared = Arc::new(Shared {
        out,
        http,
        account,
        remote,
        now: Mutex::new(None),
        casts: Mutex::new(HashMap::new()),
        asked: Mutex::new(HashMap::new()),
        probes: AtomicU64::new(0),
    });
    tokio::spawn(
        Worker {
            shared,
            daemon: None,
            discovery: None,
        }
        .run(requests),
    );
    Arc::new(connect.without_jams())
}

/// What the worker and its tasks share.
struct Shared {
    out: UnboundedSender<Event>,
    http: reqwest::Client,
    account: Option<Account>,
    /// This remote's id on every screen.
    remote: String,
    /// The last snapshot of this app's playback, which a handoff carries.
    now: Mutex<Option<Arc<Snapshot>>>,
    /// The Cast devices that run YouTube, by device id.
    casts: Mutex<HashMap<String, Cast>>,
    /// The devices asked whether they run YouTube, so an announcement repeated is not asked
    /// again, each with the probe that asks. A probe whose entry is gone or replaced, because
    /// the device left or discovery stopped, drops its answer.
    asked: Mutex<HashMap<String, u64>>,
    /// Numbers the probes, so one from before discovery stopped is told from its successor.
    probes: AtomicU64,
}

/// A Cast device that runs YouTube, and the lounge on it once playback was handed there.
struct Cast {
    found: Found,
    /// The YouTube receiver the device runs, and the theme a remote binds to it with.
    app: (&'static str, &'static str),
    lounge: Option<Arc<Lounge>>,
    reported: Screen,
    /// The videos handed to the screen, which tell the queue around the one it plays.
    handed: Vec<String>,
    /// When this app last handed playback to the screen. Of two screens that both play or are
    /// both paused, the later one is where playback is.
    handed_at: Option<Instant>,
    /// Polls the screen while the lounge is bound.
    watch: Option<JoinHandle<()>>,
    /// Queues the rest of a handoff on the screen, one track at a time.
    queuing: Option<JoinHandle<()>>,
    /// Feeds `queuing`, so a song queued by hand lands behind the handoff's own.
    queue: Option<UnboundedSender<String>>,
    /// The video of a handoff of a paused song. The screen starts whatever it is handed, so it
    /// is paused again once it says it plays this one.
    paused: Option<String>,
    /// That pause on its way to the screen, which a Play pressed meanwhile calls off.
    pausing: Option<JoinHandle<()>>,
    /// When the latest handoff began. A refusal the screen reports after it is news once, since
    /// a screen skips through a queue it may not play, which is one failure.
    armed: Option<Instant>,
    /// Whether the refusal of the latest handoff was already told.
    warned: bool,
    /// Whether the device stopped announcing itself while this app held its lounge. The lounge
    /// runs through YouTube rather than the network, so the device stays listed until its screen
    /// leaves. Its announcement lapses whenever the computer sleeps.
    lost: bool,
}

impl Cast {
    fn new(found: Found, app: (&'static str, &'static str)) -> Self {
        Self {
            found,
            app,
            lounge: None,
            reported: Screen::default(),
            handed: Vec::new(),
            handed_at: None,
            watch: None,
            queuing: None,
            queue: None,
            paused: None,
            pausing: None,
            armed: None,
            warned: false,
            lost: false,
        }
    }

    /// Stops the tasks that follow and fill the screen.
    fn abort(&mut self) {
        for task in [self.watch.take(), self.queuing.take(), self.pausing.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    /// Browses for Cast devices while the device list is on. Dropping it does not stop it.
    daemon: Option<ServiceDaemon>,
    discovery: Option<JoinHandle<()>>,
}

impl Worker {
    async fn run(mut self, mut requests: UnboundedReceiver<Request>) {
        while let Some(request) = requests.recv().await {
            let mut pending = vec![request];
            while let Ok(request) = requests.try_recv() {
                pending.push(request);
            }
            for request in coalesce(pending) {
                if let Err(error) = self.handle(request).await {
                    log::warn!("youtube: casting failed: {error:#}");
                }
            }
        }
        self.stop();
    }

    async fn handle(&mut self, request: Request) -> Result<()> {
        match request {
            Request::Enable(on) => self.enable(on)?,
            Request::Publish(now) => *lock(&self.shared.now) = now,
            Request::Transfer(device) if device == HERE => self.take_back().await?,
            Request::Transfer(device) => {
                if let Err(error) = self.transfer(&device).await {
                    self.shared.out.send(Event::TransferFailed(device)).ok();
                    return Err(error);
                }
            }
            Request::Control(device, command) => control(&self.shared, &device, command).await?,
            Request::Peek(_) => {
                self.shared.out.send(Event::Invite(None::<Invite>)).ok();
            }
            Request::Join(_) | Request::Start => {
                self.shared.out.send(Event::JamFailed).ok();
            }
            Request::Leave => {
                self.shared.out.send(Event::LeaveFailed).ok();
            }
            Request::Rename(_) | Request::Kick(_) => {}
        }
        Ok(())
    }

    /// Starts or stops looking for Cast devices. Stopping lets go of every screen, which plays
    /// on regardless.
    fn enable(&mut self, on: bool) -> Result<()> {
        if on == self.daemon.is_some() {
            return Ok(());
        }
        self.stop();
        if !on {
            self.shared.out.send(Event::Roster(Roster::default())).ok();
            return Ok(());
        }
        let (seen, found) = unbounded_channel();
        self.daemon = Some(googlecast::browse(seen)?);
        self.discovery = Some(tokio::spawn(discover(self.shared.clone(), found)));
        log::info!("youtube: looking for cast devices");
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            daemon.shutdown().ok();
        }
        if let Some(discovery) = self.discovery.take() {
            discovery.abort();
        }
        for (_, mut cast) in lock(&self.shared.casts).drain() {
            cast.abort();
        }
        lock(&self.shared.asked).clear();
    }

    /// Hands playback to the screen of `device`: what this app plays, which then lets go of it,
    /// or else what another screen plays, which is then paused. With nothing to hand, the screen
    /// is made ready and becomes where playback is, so what is picked next starts there.
    async fn transfer(&self, device: &str) -> Result<()> {
        let now = lock(&self.shared.now).clone();
        let (state, source) = match now {
            Some(now) => (now, None),
            None => {
                let playing = {
                    let casts = lock(&self.shared.casts);
                    casts
                        .iter()
                        .filter(|(id, _)| *id != device)
                        .filter_map(|(_, cast)| {
                            Some((cast.lounge.clone()?, snapshot(cast)?, cast.handed_at))
                        })
                        .filter(|(_, state, _)| state.track.is_some())
                        .max_by_key(|(_, state, at)| (state.playing, *at))
                        .map(|(lounge, state, _)| (lounge, state))
                };
                let Some((lounge, state)) = playing else {
                    return ready(&self.shared, device).await;
                };
                (Arc::new(state), Some(lounge))
            }
        };
        if !hand(&self.shared, device, &state, state.live_position()).await? {
            anyhow::bail!("nothing queued can play on {device}");
        }
        match source {
            Some(source) => source.send("pause", Vec::new()).await,
            None => {
                self.shared.out.send(Event::Command(Command::Released)).ok();
                Ok(())
            }
        }
    }

    /// Moves playback from the screen that has it back to this app, with the queue that was
    /// handed to it, and pauses the screen.
    async fn take_back(&self) -> Result<()> {
        let taken = {
            let casts = lock(&self.shared.casts);
            casts
                .values()
                .filter_map(|cast| Some((cast, cast.lounge.clone()?, snapshot(cast)?)))
                .max_by_key(|(cast, _, state)| (state.playing, cast.handed_at))
                .map(|(cast, lounge, state)| (cast.found.id.clone(), lounge, state))
        };
        let Some((device, lounge, state)) = taken else {
            return Ok(());
        };
        // a screen made ready that was never handed anything has nothing to give back, and is
        // only let go of
        if state.track.is_none() {
            if let Some(cast) = lock(&self.shared.casts).get_mut(&device) {
                cast.handed_at = None;
            }
            report(&self.shared);
            return Ok(());
        }
        self.shared
            .out
            .send(Event::Command(Command::Start(Start {
                state: Arc::new(state),
                preserve_queue: true,
                restart: true,
            })))
            .ok();
        lounge.send("pause", Vec::new()).await
    }
}

/// Asks every Cast device that turns up whether it runs YouTube, lists the ones that do, and
/// takes off the ones that leave.
async fn discover(shared: Arc<Shared>, mut found: UnboundedReceiver<Seen>) {
    while let Some(seen) = found.recv().await {
        match seen {
            Seen::Found(device) => {
                if let Some(cast) = lock(&shared.casts).get_mut(&device.id) {
                    cast.found = device;
                    cast.lost = false;
                    continue;
                }
                let probe = shared.probes.fetch_add(1, Ordering::Relaxed);
                match lock(&shared.asked).entry(device.id.clone()) {
                    Entry::Occupied(_) => continue,
                    Entry::Vacant(entry) => entry.insert(probe),
                };
                tokio::spawn(ask(shared.clone(), device, probe));
            }
            Seen::Lost(id) => {
                {
                    let mut casts = lock(&shared.casts);
                    match casts.get_mut(&id) {
                        Some(cast) if cast.lounge.is_some() => cast.lost = true,
                        _ => {
                            lock(&shared.asked).remove(&id);
                            if let Some(mut cast) = casts.remove(&id) {
                                cast.abort();
                            }
                        }
                    }
                }
                report(&shared);
            }
        }
    }
}

/// Lists `device` when it runs one of YouTube's receivers. A device that cannot be reached or
/// says no is asked again a few times, and after that the next time it announces itself.
async fn ask(shared: Arc<Shared>, device: Found, probe: u64) {
    let current = |asked: &HashMap<String, u64>| asked.get(&device.id) == Some(&probe);
    let mut tries = 0;
    let found = loop {
        tries += 1;
        let answer = async {
            let mut channel = Channel::open(device.address).await?;
            for app in RECEIVERS {
                if channel.available(app.0).await? {
                    return Ok(Some(app));
                }
            }
            anyhow::Ok(None)
        };
        match answer.await {
            Ok(Some(app)) => break Some(app),
            Ok(None) => log::debug!("youtube: {} does not run youtube", device.name),
            Err(error) => log::info!(
                "youtube: cannot ask {} about youtube: {error:#}",
                device.name
            ),
        }
        if tries == ASK_TRIES {
            break None;
        }
        tokio::time::sleep(ASK_WAIT).await;
        // the device left, or discovery stopped, while this waited
        if !current(&lock(&shared.asked)) {
            return;
        }
    };
    let mut casts = lock(&shared.casts);
    let mut asked = lock(&shared.asked);
    if !current(&asked) {
        return;
    }
    match found {
        // a device listed already keeps its entry, which may hold the screen it plays on
        Some(app) if !casts.contains_key(&device.id) => {
            log::info!("youtube: found the cast device {}", device.name);
            casts.insert(device.id.clone(), Cast::new(device, app));
            drop(asked);
            drop(casts);
            report(&shared);
        }
        Some(_) => {}
        None => {
            asked.remove(&device.id);
        }
    }
}

/// Hands `state` to the screen of `device` from `at`: its track and what follows it, in place of
/// whatever the screen played. Local files cannot play there and are skipped, the playing one
/// included. Answers whether anything was handed.
async fn hand(shared: &Arc<Shared>, device: &str, state: &Snapshot, at: Duration) -> Result<bool> {
    let streamed = |track: &&QueuedTrack| !music::is_local_id(&track.id);
    let at = match state.track.iter().any(|track| streamed(&track)) {
        true => at,
        false => Duration::ZERO,
    };
    let videos: Vec<String> = state
        .track
        .iter()
        .chain(state.upcoming.iter())
        .filter(streamed)
        .take(CARRIED + 1)
        .map(|track| track.id.clone())
        .collect();
    let Some(first) = videos.first() else {
        return Ok(false);
    };
    let lounge = lounge(shared, device).await?;
    if let Some(cast) = lock(&shared.casts).get_mut(device) {
        // a handoff still queueing or pausing would carry on with the one this replaces
        for task in [cast.queuing.take(), cast.pausing.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
        cast.armed = Some(Instant::now());
        cast.warned = false;
        cast.paused = (!state.playing).then(|| first.clone());
    }
    lounge.play(first, at).await?;
    log::info!("youtube: handed playback to {device}");
    // the rest is queued one request at a time, which takes a while, so it runs on its own
    let (queue, mut rest) = unbounded_channel::<String>();
    for video in &videos[1..] {
        queue.send(video.clone()).ok();
    }
    // a handoff that failed before here is not where playback is, so only now is it the latest
    if let Some(cast) = lock(&shared.casts).get_mut(device) {
        cast.handed = videos;
        cast.queue = Some(queue);
        cast.handed_at = Some(Instant::now());
    }
    let (owner, screen) = (shared.clone(), device.to_owned());
    let queuing = tokio::spawn(async move {
        while let Some(video) = rest.recv().await {
            if let Err(error) = lounge.enqueue(&video).await {
                log::warn!("youtube: cannot queue on the screen: {error:#}");
                // what never reached the screen leaves the queue it reports, and a song queued
                // by hand from here on goes to the screen directly
                rest.close();
                let mut unsent = vec![video];
                while let Ok(video) = rest.try_recv() {
                    unsent.push(video);
                }
                if let Some(cast) = lock(&owner.casts).get_mut(&screen) {
                    for video in unsent {
                        if let Some(at) = cast.handed.iter().rposition(|handed| *handed == video) {
                            cast.handed.remove(at);
                        }
                    }
                }
                report(&owner);
                break;
            }
        }
    });
    if let Some(cast) = lock(&shared.casts).get_mut(device) {
        cast.queuing = Some(queuing);
    }
    Ok(true)
}

/// Binds to the screen of `device` with nothing to play yet and makes it where playback is, so
/// the next start goes there.
async fn ready(shared: &Arc<Shared>, device: &str) -> Result<()> {
    lounge(shared, device).await?;
    if let Some(cast) = lock(&shared.casts).get_mut(device) {
        for task in [cast.queuing.take(), cast.pausing.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
        cast.handed.clear();
        cast.queue = None;
        cast.paused = None;
        cast.handed_at = Some(Instant::now());
    }
    log::info!("youtube: connected to {device} with nothing to play yet");
    report(shared);
    Ok(())
}

/// The lounge on the YouTube app of `device`, launching the app and binding to it when this app
/// holds none yet.
async fn lounge(shared: &Arc<Shared>, device: &str) -> Result<Arc<Lounge>> {
    let (address, app) = {
        let casts = lock(&shared.casts);
        let cast = casts
            .get(device)
            .with_context(|| format!("no cast device {device}"))?;
        if let Some(lounge) = &cast.lounge {
            return Ok(lounge.clone());
        }
        (cast.found.address, cast.app)
    };
    let mut channel = Channel::open(address).await?;
    let (app, theme) = app;
    let running = channel.launch(app, MDX).await?;
    let status = channel
        .app_request(
            &running,
            MDX,
            json!({ "type": "getMdxSessionStatus" }),
            "mdxSessionStatus",
        )
        .await?;
    drop(channel);
    let screen = status["data"]["screenId"]
        .as_str()
        .context("the youtube app named no screen")?
        .to_owned();
    let mut tries = 0;
    let (lounge, reported) = loop {
        tries += 1;
        match Lounge::bind(
            shared.http.clone(),
            screen.clone(),
            shared.remote.clone(),
            REMOTE_NAME.to_owned(),
            theme,
            shared.account.clone(),
        )
        .await
        {
            Ok(bound) => break bound,
            Err(error) if tries < BIND_TRIES => {
                log::info!("youtube: the screen is not ready yet: {error:#}");
                tokio::time::sleep(BIND_WAIT).await;
            }
            Err(error) => return Err(error),
        }
    };
    let lounge = Arc::new(lounge);
    let mut casts = lock(&shared.casts);
    let cast = casts
        .get_mut(device)
        .with_context(|| format!("the cast device {device} left"))?;
    // another handoff may have bound while this one waited
    if let Some(bound) = &cast.lounge {
        return Ok(bound.clone());
    }
    cast.lounge = Some(lounge.clone());
    cast.reported = reported.clone();
    cast.watch = Some(tokio::spawn(watch(
        shared.clone(),
        device.to_owned(),
        lounge.clone(),
        reported,
    )));
    Ok(lounge)
}

/// Follows what the screen reports until it leaves, binding again whenever the server forgets
/// the session, as it does after the computer slept.
async fn watch(shared: Arc<Shared>, device: String, lounge: Arc<Lounge>, mut reported: Screen) {
    let update = |reported: &Screen| {
        let refused = match lock(&shared.casts).get_mut(&device) {
            Some(cast) => {
                let refused = !cast.warned
                    && reported
                        .refused
                        .zip(cast.armed)
                        .is_some_and(|(refused, armed)| refused > armed);
                cast.warned |= refused;
                if reported.playing && cast.paused.is_some() && reported.video == cast.paused {
                    cast.paused = None;
                    let lounge = lounge.clone();
                    cast.pausing = Some(tokio::spawn(async move {
                        lounge.send("pause", Vec::new()).await.ok();
                    }));
                }
                cast.reported = reported.clone();
                refused
            }
            None => false,
        };
        if refused {
            shared.out.send(Event::TransferFailed(device.clone())).ok();
        }
        report(&shared);
    };
    loop {
        match lounge.poll(&mut reported, update).await {
            Ok(Ended::Done) => {}
            Ok(Ended::Rebind) => match lounge.rebind(&mut reported).await {
                Ok(()) => update(&reported),
                Err(error) => {
                    log::info!("youtube: cannot bind to the screen again: {error:#}");
                    tokio::time::sleep(RETRY).await;
                }
            },
            Ok(Ended::Gone) => break,
            Err(error) => {
                log::info!("youtube: lost the screen for now: {error:#}");
                tokio::time::sleep(RETRY).await;
            }
        }
    }
    log::info!("youtube: the screen of {device} left");
    {
        let mut casts = lock(&shared.casts);
        if let Some(cast) = casts.get_mut(&device) {
            if let Some(queuing) = cast.queuing.take() {
                queuing.abort();
            }
            // this task is the watch, which ends here by itself
            cast.watch = None;
            match cast.lost {
                true => {
                    casts.remove(&device);
                    lock(&shared.asked).remove(&device);
                }
                false => *cast = Cast::new(cast.found.clone(), cast.app),
            }
        }
    }
    report(&shared);
}

/// Asks the screen of `device` to do `command`. A start on a screen this app holds no lounge on
/// hands playback there. A start the screen does not take comes back to play here, since the
/// caller left it to the screen, and the screen is paused so the two do not play at once.
async fn control(shared: &Arc<Shared>, device: &str, command: Command) -> Result<()> {
    let lounge = || {
        lock(&shared.casts)
            .get(device)
            .and_then(|cast| cast.lounge.clone())
    };
    if let Command::Start(start) = command {
        let handed = hand(shared, device, &start.state, start.state.position).await;
        if !matches!(handed, Ok(true)) {
            // read after the handoff, which may have bound the screen before it failed
            if let Some(lounge) = lounge() {
                lounge.send("pause", Vec::new()).await.ok();
            }
            shared.out.send(Event::Command(Command::Start(start))).ok();
        }
        handed?;
        return Ok(());
    }
    let Some(lounge) = lounge() else {
        return Ok(());
    };
    if let Command::Play = command
        && let Some(cast) = lock(&shared.casts).get_mut(device)
    {
        // a song handed over paused that is played before the screen started it, or before the
        // pause reached it, stays playing
        cast.paused = None;
        if let Some(pausing) = cast.pausing.take() {
            pausing.abort();
        }
    }
    match command {
        Command::Play => lounge.send("play", Vec::new()).await,
        Command::Pause | Command::Released => lounge.send("pause", Vec::new()).await,
        Command::Next => lounge.send("next", Vec::new()).await,
        Command::Previous => lounge.send("previous", Vec::new()).await,
        Command::Seek(at) => {
            let at = format!("{:.3}", at.as_secs_f64());
            lounge.send("seekTo", vec![("newTime", at)]).await
        }
        Command::Volume(level) => {
            let level = ((level * 100.).round() as u32).to_string();
            lounge.send("setVolume", vec![("volume", level)]).await
        }
        Command::Enqueue { id, .. } if !music::is_local_id(&id) => {
            let queued = lock(&shared.casts).get_mut(device).is_some_and(|cast| {
                let queued = cast
                    .queue
                    .as_ref()
                    .is_some_and(|queue| queue.send(id.clone()).is_ok());
                if queued {
                    cast.handed.push(id.clone());
                }
                queued
            });
            if queued {
                return Ok(());
            }
            lounge.enqueue(&id).await?;
            if let Some(cast) = lock(&shared.casts).get_mut(device) {
                cast.handed.push(id);
            }
            Ok(())
        }
        Command::Enqueue { .. }
        | Command::Shuffle(_)
        | Command::Repeat(_)
        | Command::SetQueue { .. }
        | Command::Start(_) => Ok(()),
    }
}

/// Sends the devices that run YouTube, and the one whose screen plays what this app handed it.
/// One that plays wins over one that is paused, and of two alike the one handed playback last.
fn report(shared: &Shared) {
    let roster = {
        let casts = lock(&shared.casts);
        let mut devices: Vec<Device> = casts.values().map(device).collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        let elsewhere = casts
            .values()
            .filter_map(|cast| Some((cast, snapshot(cast)?)))
            .max_by_key(|(cast, state)| (state.playing, cast.handed_at))
            .map(|(cast, state)| Elsewhere {
                device: device(cast),
                state: Arc::new(state),
            });
        Roster { devices, elsewhere }
    };
    shared.out.send(Event::Roster(roster)).ok();
}

fn device(cast: &Cast) -> Device {
    Device {
        id: cast.found.id.clone(),
        name: cast.found.name.clone(),
        kind: cast.found.kind,
        volume: cast.reported.volume.unwrap_or(1.),
    }
}

/// What the screen plays, with the queue around it from what was handed to it. A screen made
/// ready with nothing yet has an empty one, and any other screen without a video has none.
fn snapshot(cast: &Cast) -> Option<Snapshot> {
    let reported = &cast.reported;
    // a screen this app connected to with nothing on it yet still has playback, playing nothing
    let Some(video) = reported.video.clone() else {
        return cast.handed_at.map(|_| Snapshot {
            volume: reported.volume.unwrap_or(1.),
            ..Snapshot::default()
        });
    };
    let at = reported
        .index
        .filter(|index| cast.handed.get(*index) == Some(&video))
        .or_else(|| cast.handed.iter().position(|handed| *handed == video));
    let queued = |ids: &[String]| -> Vec<QueuedTrack> {
        ids.iter()
            .map(|id| QueuedTrack::new(id.clone(), false))
            .collect()
    };
    let (previous, upcoming) = match at {
        Some(at) => (queued(&cast.handed[..at]), queued(&cast.handed[at + 1..])),
        None => (Vec::new(), Vec::new()),
    };
    Some(Snapshot {
        track: Some(QueuedTrack::new(video, false)),
        previous,
        upcoming,
        playing: reported.playing,
        position: reported.position,
        duration: reported.duration,
        volume: reported.volume.unwrap_or(1.),
        stamp: reported.stamp,
        ..Snapshot::default()
    })
}

/// Drops each volume or seek that a later one for the same device makes moot, since every
/// command is a request of its own that the screen takes a while to answer.
fn coalesce(pending: Vec<Request>) -> Vec<Request> {
    let moot = |at: usize| {
        let Request::Control(device, command) = &pending[at] else {
            return false;
        };
        pending[at + 1..]
            .iter()
            .any(|later| match (command, later) {
                (Command::Volume(_), Request::Control(other, Command::Volume(_)))
                | (Command::Seek(_), Request::Control(other, Command::Seek(_))) => other == device,
                _ => false,
            })
    };
    let keep: Vec<bool> = (0..pending.len()).map(|at| !moot(at)).collect();
    pending
        .into_iter()
        .zip(keep)
        .filter_map(|(request, keep)| keep.then_some(request))
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
