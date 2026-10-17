use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use futures::future::join_all;
use gpui::{App, Context, Entity, Task};
use music::connect::{
    Collection, Command, Connect, Device, Elsewhere, Event, Naming, NowPlaying, RepeatMode, Roster,
    Start,
};
use music::{MusicApi, Track};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    AppSettings, ConnectName, Io, Library, Origin, Playback, Queue, Repeat, Session, SessionEvent,
    Whence, join,
};

/// How far the position may stray from where steady playback would have put it before it is
/// reported again as a seek.
const SEEK_SLACK: Duration = Duration::from_secs(2);
/// How many upcoming tracks the other devices are told about.
const UPCOMING: usize = 10;
/// How often the views are told that another device's playback moved on while it plays.
const TICK: Duration = Duration::from_millis(500);

/// Where a start handed over by another device begins, with the tracks it plays read in.
struct Loaded {
    tracks: Vec<Track>,
    index: usize,
    origin: Option<Origin>,
    position: Duration,
    paused: bool,
}

/// Another device of the account that has playback, as the player shows it while this app steers
/// that device instead of playing itself.
#[derive(Clone, Debug)]
pub struct Steered {
    pub device: Device,
    /// The track it plays, once it is read in.
    pub track: Option<Track>,
    pub playing: bool,
    /// How far into the track it is now.
    pub position: Duration,
    pub duration: Duration,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// The provider's device network as the app sees it: this app listed as a device, playback
/// reported to the account's other apps, their commands carried out, and the account's other
/// devices to hand playback to. Only a provider that has such a network gives it anything to do.
pub struct Devices {
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    library: Entity<Library>,
    settings: Entity<AppSettings>,
    session: Entity<Session>,
    io: Io,
    link: Option<Arc<dyn Connect>>,
    /// Whether this app is on the device list, as far as the link was told.
    shown: bool,
    /// The name last handed to the link.
    named: Option<Naming>,
    roster: Roster,
    /// The track playing on another device, once it is read in.
    remote: Option<Track>,
    /// Whether this app took playback, which only the listener pressing play does. A track
    /// restored paused at launch must not pull playback off another device.
    claimed: bool,
    sent: Option<NowPlaying>,
    stamp: Instant,
    events: Option<Task<()>>,
    starting: Option<Task<()>>,
    fetching: Option<Task<()>>,
    queuing: HashMap<String, Task<()>>,
    liking: HashMap<String, Task<()>>,
    /// Notifies twice a second while another device plays, so its progress is redrawn.
    ticking: Option<Task<()>>,
}

impl Devices {
    pub fn new(
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        library: Entity<Library>,
        settings: Entity<AppSettings>,
        session: Entity<Session>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::LocalChanged => {}
            SessionEvent::SignedIn | SessionEvent::SignedOut | SessionEvent::Reconnected => {
                this.relink(cx)
            }
        })
        .detach();
        cx.observe(&playback, |this, _, cx| this.publish(cx))
            .detach();
        cx.observe(&queue, |this, _, cx| this.publish(cx)).detach();
        cx.observe(&settings, |this, _, cx| this.apply(cx)).detach();

        let this = cx.weak_entity();
        playback.update(cx, |playback, _| playback.set_devices(this));
        let mut devices = Self {
            playback,
            queue,
            library,
            settings,
            session,
            io,
            link: None,
            shown: false,
            named: None,
            roster: Roster::default(),
            remote: None,
            claimed: false,
            sent: None,
            stamp: Instant::now(),
            events: None,
            starting: None,
            fetching: None,
            queuing: HashMap::new(),
            liking: HashMap::new(),
            ticking: None,
        };
        devices.relink(cx);
        devices
    }

    /// Whether the signed-in provider has a device network, wanted or not.
    pub fn supported(&self) -> bool {
        self.link.is_some()
    }

    /// Whether the signed-in provider has a device network and the listener wants it.
    pub fn available(&self) -> bool {
        self.link.is_some() && self.shown
    }

    /// Whether this app's playback is what the account's other devices are being told about.
    pub fn publishing(&self) -> bool {
        self.available() && self.sent.is_some()
    }

    /// The account's other devices.
    pub fn devices(&self) -> &[Device] {
        &self.roster.devices
    }

    /// The other device that has playback, if one does.
    pub fn elsewhere(&self) -> Option<&Elsewhere> {
        self.roster.elsewhere.as_ref()
    }

    /// The track playing on that device, once it is read in.
    pub fn remote_track(&self) -> Option<&Track> {
        self.remote.as_ref()
    }

    /// The other device playback is on, which the player shows and steers instead of this app's
    /// own engine. `None` while playback is here or nowhere, including the moment between this app
    /// taking playback and the network saying so.
    pub fn steered(&self) -> Option<Steered> {
        if !self.available() || self.claimed {
            return None;
        }
        let elsewhere = self.elsewhere()?;
        let duration = match elsewhere.duration.is_zero() {
            true => self
                .remote
                .as_ref()
                .map_or(Duration::ZERO, |track| track.duration),
            false => elsewhere.duration,
        };
        Some(Steered {
            device: elsewhere.device.clone(),
            track: self.remote.clone(),
            playing: elsewhere.playing,
            position: elsewhere.live_position(),
            duration,
            shuffle: elsewhere.shuffle,
            repeat: repeat_of(elsewhere.repeat),
        })
    }

    /// Moves playback onto this app from whichever device has it.
    pub fn transfer_here(&self) {
        if let Some(link) = &self.link {
            link.transfer(&link.device());
        }
    }

    /// Moves playback onto another device.
    pub fn transfer(&self, device: &str) {
        if let Some(link) = &self.link {
            link.transfer(device);
        }
    }

    /// Sends a command to another device.
    pub fn control(&self, device: &str, command: Command) {
        if let Some(link) = &self.link {
            link.control(device, command);
        }
    }

    /// Has playback act on this app's own engine, as another device asked, rather than steer
    /// whichever device has playback.
    fn here(
        &self,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Playback, &mut Context<Playback>),
    ) {
        self.playback
            .update(cx, |playback, cx| playback.locally(cx, act));
    }

    /// Takes the device network of the signed-in provider, or lets go of the last one when
    /// there is none or the session was replaced.
    fn relink(&mut self, cx: &mut Context<Self>) {
        let link = self
            .session
            .read(cx)
            .client()
            .and_then(|client| client.connect());
        let same = match (&self.link, &link) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }

        self.events = None;
        self.starting = None;
        self.fetching = None;
        self.queuing.clear();
        self.liking.clear();
        self.roster = Roster::default();
        self.remote = None;
        self.claimed = false;
        self.sent = None;
        self.shown = false;
        self.named = None;
        self.link = link;

        if let Some(events) = self.link.as_ref().and_then(|link| link.events()) {
            self.events = Some(self.listen(events, cx));
        }
        self.apply(cx);
        cx.notify();
    }

    /// Lists the app on the device network, or takes it off, as the setting says.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.link.clone() else {
            return;
        };
        let settings = self.settings.read(cx);
        let wanted = settings.spotify_connect();
        let naming = match settings.spotify_connect_name() {
            ConnectName::Sonora => Naming::App,
            ConnectName::Both => Naming::AppOnComputer,
            ConnectName::Computer => Naming::Computer,
            ConnectName::Custom => {
                Naming::Custom(settings.spotify_connect_custom_name().to_owned())
            }
        };
        if self.named.as_ref() != Some(&naming) {
            link.rename(naming.clone());
            self.named = Some(naming);
        }
        if wanted == self.shown {
            return;
        }
        self.shown = wanted;
        link.enable(wanted);
        if wanted {
            self.publish(cx);
        } else {
            self.claimed = false;
            self.sent = None;
            self.roster = Roster::default();
            self.remote = None;
        }
        cx.notify();
    }

    fn listen(&mut self, mut events: UnboundedReceiver<Event>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Roster(roster) => self.set_roster(roster, cx),
            Event::Command(command) => self.run(command, cx),
            Event::Liked { track, liked } => self.liked(track, liked, cx),
        }
    }

    fn set_roster(&mut self, roster: Roster, cx: &mut Context<Self>) {
        self.roster = roster;
        let wanted = self
            .elsewhere()
            .and_then(|elsewhere| elsewhere.track.clone());
        if self.remote.as_ref().and_then(|track| track.id.clone()) != wanted {
            self.remote = None;
            self.fetching = None;
            if let (Some(id), Some(client)) = (wanted, self.session.read(cx).client()) {
                self.fetching = Some(self.read_remote(client, id, cx));
            }
        }
        self.tick(cx);
        self.redraw(cx);
    }

    /// Tells the views, and everything that follows playback such as the lyrics, that what the
    /// other device plays has changed or moved on.
    fn redraw(&self, cx: &mut Context<Self>) {
        cx.notify();
        self.playback.update(cx, |_, cx| cx.notify());
    }

    /// Whether another device has playback that plays, which is when its position moves on.
    fn moving(&self) -> bool {
        self.available() && self.elsewhere().is_some_and(|elsewhere| elsewhere.playing)
    }

    /// Starts the timer that redraws another device's progress while it plays, or drops it once
    /// nothing plays there.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.moving() {
            self.ticking = None;
        } else if self.ticking.is_none() {
            self.ticking = Some(cx.spawn(async move |this, cx| {
                while this.update(cx, |this, cx| this.pulse(cx)).unwrap_or(false) {
                    cx.background_executor().timer(TICK).await;
                }
            }));
        }
    }

    /// Tells the views that time has passed, and says whether the timer is to go on.
    fn pulse(&mut self, cx: &mut Context<Self>) -> bool {
        let moving = self.moving();
        match moving {
            true => self.redraw(cx),
            false => self.ticking = None,
        }
        moving
    }

    fn read_remote(
        &self,
        client: Arc<dyn MusicApi>,
        id: String,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let wanted = id.clone();
            let found = join(io.spawn(async move { client.track(&wanted).await })).await;
            this.update(cx, |this, cx| {
                this.fetching = None;
                match found {
                    Ok(track) if track.id.as_deref() == Some(id.as_str()) => {
                        let current = this.elsewhere().and_then(|e| e.track.as_deref());
                        if current == Some(id.as_str()) {
                            this.remote = Some(track);
                            this.redraw(cx);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!("connect: cannot read the remote track: {error:#}"),
                }
            })
            .ok();
        })
    }

    /// Carries out what another device asked of this one.
    fn run(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Play => self.here(cx, |playback, cx| playback.resume(cx)),
            Command::Pause => self.here(cx, |playback, cx| playback.pause(cx)),
            Command::Next => self.here(cx, |playback, cx| playback.next(cx)),
            Command::Previous => self.here(cx, |playback, cx| playback.previous(cx)),
            Command::Seek(at) => self.here(cx, |playback, cx| playback.seek(at, cx)),
            Command::Volume(level) => self.here(cx, |playback, cx| playback.set_volume(level, cx)),
            Command::Shuffle(on) => self.queue.update(cx, |queue, cx| queue.set_shuffle(on, cx)),
            Command::Repeat(mode) => {
                self.here(cx, |playback, cx| playback.set_repeat(repeat_of(mode), cx))
            }
            Command::Enqueue(id) => self.enqueue(id, cx),
            Command::Start(start) => self.start(start, cx),
            Command::Released => {
                // another device took over, so this one stops without calling playback back
                self.claimed = false;
                self.here(cx, |playback, cx| playback.pause(cx));
                self.publish(cx);
            }
        }
    }

    fn enqueue(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        let io = self.io.clone();
        let key = id.clone();
        let slot = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let found = join(io.spawn(async move { client.track(&id).await })).await;
            this.update(cx, |this, cx| {
                this.queuing.remove(&key);
                match found {
                    Ok(track) => this.here(cx, |playback, cx| playback.enqueue(track, cx)),
                    Err(error) => log::warn!("connect: cannot queue {key}: {error:#}"),
                }
            })
            .ok();
        });
        self.queuing.insert(slot, task);
    }

    /// Shows a like made in another of the account's apps, reading the track in first when it
    /// was liked rather than unliked.
    fn liked(&mut self, id: String, liked: bool, cx: &mut Context<Self>) {
        // an unlike also calls off a like of the same track still being read in
        self.liking.remove(&id);
        if !liked {
            return self
                .library
                .update(cx, |library, cx| library.liked_elsewhere(&id, None, cx));
        }
        // a like this app made comes back the same way, with nothing left to show
        if self.library.read(cx).saved(&id) {
            return;
        }
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        let io = self.io.clone();
        let slot = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let wanted = id.clone();
            let found = join(io.spawn(async move { client.track(&wanted).await })).await;
            this.update(cx, |this, cx| {
                this.liking.remove(&id);
                match found {
                    Ok(track) => this.library.update(cx, |library, cx| {
                        library.liked_elsewhere(&id, Some(track), cx)
                    }),
                    Err(error) => {
                        log::warn!("connect: cannot read the liked track {id}: {error:#}")
                    }
                }
            })
            .ok();
        });
        self.liking.insert(slot, task);
    }

    /// Plays what another device handed over, from where it left off.
    fn start(&mut self, start: Start, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
            log::warn!("connect: cannot start what was handed over, no client");
            return;
        };
        self.claimed = true;
        let io = self.io.clone();
        self.starting = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move { load(client, start).await })).await;
            this.update(cx, |this, cx| {
                this.starting = None;
                match loaded {
                    Ok(loaded) => this.here(cx, |playback, cx| {
                        playback.start_at(
                            loaded.tracks,
                            loaded.index,
                            loaded.origin,
                            loaded.position,
                            cx,
                        );
                        if loaded.paused {
                            playback.pause(cx);
                        }
                    }),
                    Err(error) => {
                        log::warn!("connect: cannot start what was handed over: {error:#}");
                        this.claimed = false;
                    }
                }
            })
            .ok();
        }));
    }

    /// Tells the account's other apps what plays, when something does that this app took.
    fn publish(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.link.clone() else {
            return;
        };
        if !self.shown {
            return;
        }

        let Some(now) = self.now_playing(cx) else {
            if self.sent.take().is_some() {
                link.publish(None);
                cx.notify();
            }
            return;
        };
        let Some(reason) = self.changed(&now) else {
            return;
        };
        log::debug!(
            "connect: reporting {} at {:?} because of the {reason}",
            now.track,
            now.position
        );
        self.stamp = Instant::now();
        self.sent = Some(now.clone());
        link.publish(Some(now));
        cx.notify();
    }

    /// What `now` says beyond the last report, or `None` when it says nothing new. The position
    /// moving on its own is not news, a jump is.
    fn changed(&self, now: &NowPlaying) -> Option<&'static str> {
        let Some(sent) = &self.sent else {
            return Some("first report");
        };
        let expected = match sent.playing {
            true => sent.position + self.stamp.elapsed(),
            false => sent.position,
        };
        if now.position.abs_diff(expected) > SEEK_SLACK {
            return Some("position");
        }
        let mut same = now.clone();
        same.position = sent.position;
        (same != *sent).then(|| differs(&same, sent))
    }

    fn now_playing(&mut self, cx: &App) -> Option<NowPlaying> {
        let playback = self.playback.read(cx);
        let track = playback.track()?;
        let id = track.id.clone().filter(|id| !music::is_local_id(id))?;
        let playing = playback.wants_playing();
        self.claimed |= playing;
        if !self.claimed {
            return None;
        }

        let queue = self.queue.read(cx);
        let upcoming = queue
            .upcoming()
            .filter_map(|track| track.id.clone())
            .filter(|id| !music::is_local_id(id))
            .take(UPCOMING)
            .collect();
        let context = playback.origin(cx).and_then(collection_of);
        Some(NowPlaying {
            track: id,
            upcoming,
            index: context.is_some().then(|| queue.place()).flatten(),
            context,
            playing,
            position: playback.live_position(),
            duration: track.duration,
            volume: playback.volume(),
            shuffle: queue.shuffle(),
            repeat: mode_of(playback.repeat()),
        })
    }
}

/// The repeat setting the device network names `mode`.
fn repeat_of(mode: RepeatMode) -> Repeat {
    match mode {
        RepeatMode::Off => Repeat::Off,
        RepeatMode::Context => Repeat::All,
        RepeatMode::Track => Repeat::One,
    }
}

/// The device network's name for the repeat setting `repeat`.
pub(crate) fn mode_of(repeat: Repeat) -> RepeatMode {
    match repeat {
        Repeat::Off => RepeatMode::Off,
        Repeat::All => RepeatMode::Context,
        Repeat::One => RepeatMode::Track,
    }
}

/// The collection a track was queued from, as the device network names it. `None` for one it has
/// no name for, such as an artist or a radio.
pub(crate) fn collection_of(origin: &Origin) -> Option<Collection> {
    match origin.whence {
        Whence::Album => Some(Collection::Album(origin.id.clone())),
        Whence::Playlist => Some(Collection::Playlist(origin.id.clone())),
        Whence::Saved => Some(Collection::Saved),
        _ => None,
    }
}

/// Names the first thing two reports disagree on.
fn differs(a: &NowPlaying, b: &NowPlaying) -> &'static str {
    match () {
        () if a.track != b.track => "track",
        () if a.upcoming != b.upcoming => "queue",
        () if a.context != b.context => "context",
        () if a.playing != b.playing => "play state",
        () if a.duration != b.duration => "duration",
        () if a.volume != b.volume => "volume",
        () if a.shuffle != b.shuffle => "shuffle",
        _ => "repeat",
    }
}

/// Reads in what a start names: the collection if it has one the track is in, otherwise the
/// track and the tracks that follow it.
async fn load(client: Arc<dyn MusicApi>, start: Start) -> Result<Loaded> {
    let (tracks, origin) = match &start.collection {
        Some(Collection::Album(id)) => (client.album_tracks(id).await?, Some(Origin::album(id))),
        Some(Collection::Playlist(id)) => (
            client.playlist_tracks(id).await?,
            Some(Origin::playlist(id)),
        ),
        Some(Collection::Saved) => (client.saved_tracks().await?, Some(Origin::saved())),
        // a radio that was refused elsewhere plays here: its seed, then what the station brings
        Some(Collection::Radio(id)) => {
            let mut tracks = client
                .track_radio(id, None)
                .await
                .map(|(tracks, _)| tracks)
                .unwrap_or_default();
            tracks.retain(|track| track.playable && track.id.as_deref() != Some(id));
            tracks.insert(0, client.track(id).await?);
            (tracks, Some(Origin::radio(id)))
        }
        None => (Vec::new(), None),
    };

    let at = |tracks: &[Track]| {
        tracks
            .iter()
            .position(|track| track.id.is_some() && track.id == start.track)
    };
    let (tracks, origin, index) = match at(&tracks) {
        Some(index) => (tracks, origin, index),
        None if start.collection.is_some() && start.track.is_none() => {
            let index = start
                .index
                .filter(|index| *index < tracks.len())
                .unwrap_or(0);
            (tracks, origin, index)
        }
        None => {
            let ids = start.track.iter().chain(&start.upcoming);
            let found = join_all(ids.map(|id| client.track(id))).await;
            let tracks = found.into_iter().filter_map(Result::ok).collect::<Vec<_>>();
            (tracks, None, 0)
        }
    };
    anyhow::ensure!(!tracks.is_empty(), "the start has no track to play");

    Ok(Loaded {
        tracks,
        index,
        origin,
        position: start.position,
        paused: start.paused,
    })
}
