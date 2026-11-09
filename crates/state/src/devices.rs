use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use futures::{StreamExt as _, stream};
use music::connect::{
    Collection, Command, Connect, Device, Elsewhere, Event as ConnectEvent, Invite, Jam, Naming,
    QueuedTrack, Request, Roster, Snapshot, Start,
};
use music::{Contributor, MusicApi, Track};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::queue::Reach;
use crate::{
    AppSettings, Change, ConnectName, Cx, Event, Job, Origin, Outcome, Playback, Queue, QueueKind,
    QueuePosition, QueueSections, Session, SessionEvent, Store, Toasts, Topic, Whence,
};

/// Librespot's 80-track report window avoids overloading Spotify's web player.
const UPCOMING: usize = 80;
/// Match librespot’s recent-history window; the local queue retains its longer history.
const PREVIOUS: usize = 10;
/// Resolving a queue should not fan out into a request for every track at once.
const QUEUE_READS: usize = 8;
const REMOTE_TRACKS: usize = 25;
/// How often the views are told that another device's playback moved on while it plays.
const TICK: Duration = Duration::from_millis(500);
/// The actual history and upcoming order supplied by another device.
type RemoteWindow = (Vec<(Track, bool)>, bool, Vec<(Track, bool)>);

/// The provider's device network as the app sees it: this app listed as a device, playback
/// reported to the account's other apps, their commands carried out, and the account's other
/// devices to hand playback to. Only a provider that has such a network gives it anything to do.
pub struct Devices {
    link: Option<Arc<Connect>>,
    /// Whether this app is on the device list, as far as the link was told.
    shown: bool,
    /// The name last handed to the link.
    named: Option<Naming>,
    roster: Roster,
    /// The group session the listener joined, if any.
    jam: Option<Jam>,
    /// The group session a link named, which the listener is being asked to join.
    invite: Option<Invite>,
    /// A Jam link asked about before there was a device network to look it up on.
    asked: Option<String>,
    /// Whether the listener started a Jam that is not up yet, so its code shows once it is.
    awaiting_code: bool,
    /// Whether the code that opens the Jam's link on a phone is on show.
    code: bool,
    /// Whether the dialog that takes a Jam link to join is open.
    joining: bool,
    /// Whether a leave is on its way. The Jam is off screen already, so updates to it that come
    /// in before the leave is through are dropped.
    leaving: bool,
    /// The tracks on that device's queue that are read in, by id.
    listed: HashMap<String, Track>,
    listing: Option<Job>,
    /// People who queued songs and are not in the Jam's member list, read from their profiles,
    /// by the id the queue names them by.
    people: HashMap<String, Arc<Contributor>>,
    /// Queued tracks and people whose read failed, by id. They are not asked for again until the
    /// other device moves to another track, so one that never reads cannot keep `list` going.
    unreadable: HashSet<String>,
    /// Whether this app took playback, which only the listener pressing play does. A track
    /// restored paused at launch must not pull playback off another device.
    claimed: bool,
    sent: Option<Arc<Snapshot>>,
    events: Option<Job>,
    /// The latest transfer or queue edit; a newer request cancels its predecessor.
    changing: Option<Job>,
    queuing: HashMap<String, Job>,
    liking: HashMap<String, Job>,
    /// Notifies twice a second while another device plays, so its progress is redrawn.
    ticking: Option<Job>,
}

/// The models the device network reads: the account it belongs to, the setting that lists this
/// app, and the playback and queue it reports.
#[derive(Clone, Copy)]
struct Around<'a> {
    session: &'a Session,
    settings: &'a AppSettings,
    playback: &'a Playback,
    queue: &'a Queue,
}

impl Devices {
    pub fn new(
        session: &Session,
        settings: &AppSettings,
        playback: &Playback,
        queue: &Queue,
        cx: &mut Cx,
    ) -> Self {
        let mut devices = Self {
            link: None,
            shown: false,
            named: None,
            roster: Roster::default(),
            jam: None,
            invite: None,
            asked: None,
            awaiting_code: false,
            code: false,
            joining: false,
            leaving: false,
            listed: HashMap::new(),
            listing: None,
            people: HashMap::new(),
            unreadable: HashSet::new(),
            claimed: false,
            sent: None,
            events: None,
            changing: None,
            queuing: HashMap::new(),
            liking: HashMap::new(),
            ticking: None,
        };
        let around = Around {
            session,
            settings,
            playback,
            queue,
        };
        devices.relink(around, cx);
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

    /// The other device that has playback, if one does. `None` once this app took playback, even
    /// before the network's next roster stops naming the device it came from.
    pub fn elsewhere(&self) -> Option<&Elsewhere> {
        self.roster.elsewhere.as_ref().filter(|_| !self.claimed)
    }

    /// The group session the listener is in, if any.
    pub fn jam(&self) -> Option<&Jam> {
        self.jam.as_ref()
    }

    /// The group session a link named, while the listener is asked whether to join it.
    pub fn invite(&self) -> Option<&Invite> {
        self.invite.as_ref()
    }

    /// Looks up the group session a shared link names. It shows up on `invite()` to be accepted
    /// or dismissed. A link asked about before the device network is up waits for it, and one
    /// asked about while the listener keeps the app off the device list only gets a toast.
    pub fn peek(&mut self, link: &str, cx: &mut Cx) {
        let Some(link_to) = &self.link else {
            self.asked = Some(link.to_owned());
            return;
        };
        if !self.shown {
            return Toasts::show(Outcome::Info, "toast-jam-needs-connect", cx);
        }
        link_to.send(Request::Peek(link.to_owned()));
    }

    /// Joins the group session the invite names.
    pub fn accept(&mut self, cx: &mut Cx) {
        if let (Some(invite), Some(link)) = (self.invite.take(), &self.link) {
            link.send(Request::Join(invite.link));
            self.leaving = false;
        }
        cx.changed(Topic::Devices);
    }

    /// Drops the invite without joining.
    pub fn dismiss(&mut self, cx: &mut Cx) {
        self.invite = None;
        cx.changed(Topic::Devices);
    }

    /// Removes a member, by `JamMember::id`, from the group session the listener owns.
    pub fn kick(&self, member: &str) {
        if let Some(link) = &self.link {
            link.send(Request::Kick(member.to_owned()));
        }
    }

    /// Starts a group session on this app. Its link to share is on `jam()` once it is up, and its
    /// code is shown then.
    pub fn start_jam(&mut self) {
        if let Some(link) = &self.link {
            link.send(Request::Start);
            self.awaiting_code = true;
            self.leaving = false;
        }
    }

    /// Whether the code of the Jam's link is on show, which only a Jam with a link can have.
    pub fn code_shown(&self) -> bool {
        self.code && self.jam.as_ref().is_some_and(|jam| jam.link.is_some())
    }

    /// Shows or hides the code of the Jam's link.
    pub fn show_code(&mut self, shown: bool, cx: &mut Cx) {
        self.code = shown;
        cx.changed(Topic::Devices);
    }

    /// Whether the dialog that takes a Jam link to join is open.
    pub fn join_shown(&self) -> bool {
        self.joining
    }

    /// Opens or closes the dialog that takes a Jam link to join.
    pub fn show_join(&mut self, shown: bool, cx: &mut Cx) {
        self.joining = shown;
        cx.changed(Topic::Devices);
    }

    /// Leaves the group session, or ends it when the listener started it.
    /// Takes the Jam off screen at once, says it is over, and asks the provider to follow. A
    /// refusal brings it back with a toast.
    pub fn leave(&mut self, cx: &mut Cx) {
        let Some(link) = &self.link else {
            return;
        };
        link.send(Request::Leave);
        let owned = self.jam.as_ref().is_some_and(|jam| jam.owner);
        let said = match owned {
            true => "toast-jam-ended",
            false => "toast-jam-left",
        };
        Toasts::show(Outcome::Done, said, cx);
        self.leaving = true;
        self.jam = None;
        self.code = false;
        cx.changed(Topic::Devices);
    }

    /// The other device playback is on, which the player shows and steers instead of this app's
    /// own engine. `None` while playback is here or nowhere, including the moment between this app
    /// taking playback and the network saying so.
    pub fn steered(&self) -> Option<Elsewhere> {
        if !self.available() || self.claimed {
            return None;
        }
        self.elsewhere().cloned()
    }

    /// Whether the metadata for `id` is read in.
    fn readable(&self, id: &str) -> bool {
        self.listed.contains_key(id)
    }

    /// Addresses the provider's queue through its metadata cache without copying the play order.
    pub(crate) fn queue_tracks(&self, position: QueuePosition) -> Option<Track> {
        let remote = &self.elsewhere()?.state;
        let listed = match position {
            QueuePosition::Past(at) => remote
                .previous
                .iter()
                .filter(|row| self.readable(&row.id))
                .nth(at)?,
            QueuePosition::Current => remote.track.as_ref()?,
            QueuePosition::Upcoming(at) => remote.upcoming.get(at)?,
        };
        let mut track = self.listed.get(&listed.id)?.clone();
        track.added_by = listed.added_by.as_deref().and_then(|id| self.known(id));
        Some(track)
    }

    /// Counts readable rows, stopping upcoming at the first gap so Next never names a later song.
    pub(crate) fn queue_sections(&self) -> QueueSections {
        let remote = self
            .elsewhere()
            .map(|remote| &remote.state)
            .expect("remote queue requires remote playback");
        QueueSections::new(
            remote
                .previous
                .iter()
                .filter(|row| self.readable(&row.id))
                .count(),
            remote
                .track
                .as_ref()
                .is_some_and(|track| self.readable(&track.id)),
            remote
                .upcoming
                .iter()
                .take_while(|row| self.readable(&row.id))
                .map(|row| match row.queued {
                    true => QueueKind::Queued,
                    false => QueueKind::Context,
                }),
        )
    }

    /// The listener as the Jam's members list them, so a song they queue names them.
    pub(crate) fn you(&self) -> Option<Arc<Contributor>> {
        let member = self
            .jam
            .as_ref()?
            .members
            .iter()
            .find(|member| member.you)?;
        Some(Arc::new(Contributor {
            id: member.id.clone(),
            name: member.name.clone(),
            avatar: member.image.clone(),
        }))
    }

    /// The person a provider id names, by the Jam's members or a profile already read, when
    /// either knows them.
    fn known(&self, id: &str) -> Option<Arc<Contributor>> {
        let member = self
            .jam
            .iter()
            .flat_map(|jam| &jam.members)
            .find(|member| member.id == id || member.username == id);
        match member {
            Some(member) => Some(Arc::new(Contributor {
                id: member.id.clone(),
                name: member.name.clone(),
                avatar: member.image.clone(),
            })),
            None => self.people.get(id).cloned(),
        }
    }

    /// Moves playback onto this app from whichever device has it.
    pub fn transfer_here(&self) {
        if let Some(link) = &self.link {
            link.send(Request::Transfer(link.device().to_owned()));
        }
    }

    /// Moves playback onto another device.
    pub fn transfer(&self, device: &str) {
        if let Some(link) = &self.link {
            link.send(Request::Transfer(device.to_owned()));
        }
    }

    /// Sends a command to another device.
    pub fn control(&self, device: &str, command: Command) {
        if let Some(link) = &self.link {
            link.send(Request::Control(device.to_owned(), command));
        }
    }

    /// Changes the device's volume and shows the requested level until its next report arrives.
    /// The caller raises [`Topic::Playback`], since what playback shows moved with it.
    pub(crate) fn set_volume(&mut self, device: &str, level: f32, cx: &mut Cx) {
        self.control(device, Command::Volume(level));
        if let Some(listed) = self
            .roster
            .devices
            .iter_mut()
            .find(|listed| listed.id == device)
        {
            listed.volume = level;
        }
        if let Some(elsewhere) = self
            .roster
            .elsewhere
            .as_mut()
            .filter(|e| e.device.id == device)
        {
            elsewhere.device.volume = level;
        }
        cx.changed(Topic::Devices);
    }

    /// Takes the device network of the signed-in provider, or lets go of the last one when
    /// there is none or the session was replaced.
    fn relink(&mut self, around: Around<'_>, cx: &mut Cx) {
        let link = around.session.client().and_then(|client| client.connect());
        let same = match (&self.link, &link) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }

        self.reset(link);
        if let Some(events) = self.link.as_ref().and_then(|link| link.events()) {
            self.events = Some(listen(events, cx));
        }
        self.apply(around, cx);
        if let Some(asked) = self.asked.take()
            && self.link.is_some()
        {
            self.peek(&asked, cx);
        }
        cx.changed(Topic::Devices);
    }

    /// Drops everything owned by the previous provider connection.
    fn reset(&mut self, link: Option<Arc<Connect>>) {
        self.link = link;
        self.shown = false;
        self.named = None;
        self.roster = Roster::default();
        self.jam = None;
        self.invite = None;
        self.awaiting_code = false;
        self.code = false;
        self.joining = false;
        self.leaving = false;
        self.listed.clear();
        self.listing = None;
        self.people.clear();
        self.unreadable.clear();
        self.claimed = false;
        self.sent = None;
        self.events = None;
        self.changing = None;
        self.queuing.clear();
        self.liking.clear();
        self.ticking = None;
    }

    /// Lists the app on the device network, or takes it off, as the setting says.
    fn apply(&mut self, around: Around<'_>, cx: &mut Cx) {
        let Some(link) = self.link.clone() else {
            return;
        };
        let settings = around.settings;
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
            link.send(Request::Rename(naming.clone()));
            self.named = Some(naming);
        }
        if wanted == self.shown {
            return;
        }
        self.shown = wanted;
        link.send(Request::Enable(wanted));
        if wanted {
            self.publish(around.playback, around.queue, cx);
        } else {
            let named = self.named.take();
            let events = self.events.take();
            self.reset(Some(link));
            self.named = named;
            self.events = events;
        }
        cx.changed(Topic::Devices);
    }

    /// Takes in a new roster: what plays elsewhere, where, and who is listening. Says whether
    /// this app lets go of playback it took for a local file, which the caller then pauses.
    fn set_roster(&mut self, roster: Roster, session: &Session, cx: &mut Cx) -> bool {
        let wanted = roster
            .elsewhere
            .as_ref()
            .and_then(|elsewhere| elsewhere.state.track.as_ref())
            .map(|track| &track.id);
        let had = self
            .roster
            .elsewhere
            .as_ref()
            .and_then(|elsewhere| elsewhere.state.track.as_ref())
            .map(|track| &track.id);
        let moved = wanted != had;
        let started = roster.elsewhere.as_ref().is_some_and(|e| e.state.playing)
            && !self
                .roster
                .elsewhere
                .as_ref()
                .is_some_and(|e| e.state.playing);
        self.roster = roster;
        // playback taken for a local file is never reported, so the network never releases it,
        // and another device starting to play is what takes it back
        let release = started && self.claimed && self.sent.is_none();
        if release {
            self.claimed = false;
        }
        // what failed to read is asked for again once the other device plays something else
        if moved {
            self.listing = None;
            self.unreadable.clear();
        }
        if self.roster.elsewhere.as_ref().is_none() {
            self.listing = None;
        }
        let retained: HashSet<_> = self
            .roster
            .elsewhere
            .as_ref()
            .into_iter()
            .flat_map(|remote| remote.state.tracks())
            .map(|track| &track.id)
            .collect();
        self.listed.retain(|id, _| retained.contains(id));
        self.list(session, cx);
        self.tick(cx);
        redraw(cx);
        release
    }

    /// Resolves the active song, history and upcoming tracks through one cache and task.
    /// Missing metadata is retried on the next song, with bounded batches for queue reads.
    fn list(&mut self, session: &Session, cx: &mut Cx) {
        if self.listing.is_some() {
            return;
        }
        let Some(client) = session.client() else {
            return;
        };
        let Some(remote) = self.roster.elsewhere.as_ref() else {
            return;
        };
        let state = &remote.state;
        // The playing song comes first, so it is read in the same concurrent batch.
        let window = || {
            state
                .track
                .iter()
                .chain(state.upcoming.iter().take(REMOTE_TRACKS))
                .chain(&state.previous)
        };
        let mut seen = HashSet::new();
        let missing = window()
            .map(|listed| &listed.id)
            .filter(|id| !self.readable(id) && !self.unreadable.contains(*id) && seen.insert(*id))
            .take(QUEUE_READS)
            .cloned()
            .collect::<Vec<_>>();
        let strangers = window()
            .filter_map(|listed| listed.added_by.clone())
            .filter(|adder| self.known(adder).is_none() && !self.unreadable.contains(adder))
            .collect::<HashSet<_>>();
        if missing.is_empty() && strangers.is_empty() {
            return;
        }
        let asked = missing
            .iter()
            .chain(&strangers)
            .cloned()
            .collect::<Vec<_>>();
        self.listing = Some(cx.load(
            async move {
                let read: Vec<_> = resolve_queue(&*client, &mut HashMap::new(), missing)
                    .await
                    .into_iter()
                    .flatten()
                    .filter_map(|track| Some((track.id.clone()?, track)))
                    .collect();
                let mut people = Vec::new();
                for adder in strangers {
                    people.extend(person(&*client, adder).await);
                }
                Ok((read, people))
            },
            move |store, read| {
                let Store {
                    devices: this,
                    session,
                    cx,
                    ..
                } = store;
                this.listing = None;
                // a read that never finished counts as failed, so it is not asked for at once
                let (read, people) = read.unwrap_or_default();
                this.listed.extend(read);
                this.people.extend(people);
                let unreadable: Vec<_> = asked
                    .into_iter()
                    .filter(|id| !this.readable(id) && !this.people.contains_key(id))
                    .collect();
                this.unreadable.extend(unreadable);
                this.list(session, cx);
                redraw(cx);
            },
        ));
    }

    /// Whether another device has playback that is moving on.
    fn moving(&self) -> bool {
        self.available() && self.elsewhere().is_some_and(|e| e.state.playing)
    }

    /// Redraws a remote device's advancing clock until playback stops or moves here.
    fn tick(&mut self, cx: &mut Cx) {
        if !self.moving() {
            self.ticking = None;
        } else if self.ticking.is_none() {
            self.ticking = Some(cx.spawn(|link| async move {
                while link
                    .read(|store| {
                        let this = &mut store.devices;
                        if this.moving() {
                            redraw(&mut store.cx);
                        } else {
                            this.ticking = None;
                        }
                        this.moving()
                    })
                    .await
                    .unwrap_or(false)
                {
                    tokio::time::sleep(TICK).await;
                }
            }));
        }
    }

    /// Reads in a track another device queued here, with who queued it when the Jam does not
    /// list them, and queues it on this app's own engine.
    fn enqueue(&mut self, id: String, added_by: Option<String>, session: &Session, cx: &mut Cx) {
        let Some(client) = session.client() else {
            return;
        };
        let key = id.clone();
        let slot = id.clone();
        let stranger = added_by.clone().filter(|adder| self.known(adder).is_none());
        let task = cx.load(read_track(client, id, stranger), move |store, found| {
            let this = &mut store.devices;
            this.queuing.remove(&key);
            match found {
                Ok((mut track, person)) => {
                    this.people.extend(person);
                    track.added_by = added_by.as_deref().and_then(|id| this.known(id));
                    store.locally(|store| store.enqueue_track(track))
                }
                Err(error) => log::warn!("connect: cannot queue {key}: {error:#}"),
            }
        });
        self.queuing.insert(slot, task);
    }

    /// Tells the account's other apps what plays, when something does that this app took.
    fn publish(&mut self, playback: &Playback, queue: &Queue, cx: &mut Cx) {
        if self.changing.is_some() {
            return;
        }
        let Some(link) = self.link.clone() else {
            return;
        };
        if !self.shown {
            return;
        }

        let Some(now) = self.now_playing(playback, queue) else {
            if self.sent.take().is_some() {
                link.send(Request::Publish(None));
                cx.changed(Topic::Devices);
            }
            return;
        };
        let Some(reason) = self
            .sent
            .as_ref()
            .map_or(Some("first report"), |sent| now.change(sent))
        else {
            return;
        };
        log::debug!(
            "connect: reporting {} at {:?} because of the {reason}",
            now.track.as_ref().map_or("", |track| &track.id),
            now.position
        );
        self.sent = Some(now.clone());
        link.send(Request::Publish(Some(now)));
        cx.changed(Topic::Devices);
    }

    /// What this app plays, for the report to the other devices, once it took playback. Playing
    /// anything here takes it, a local file too, though a local file is never reported.
    fn now_playing(&mut self, playback: &Playback, queue: &Queue) -> Option<Arc<Snapshot>> {
        let track = playback.track()?;
        let id = track.id.clone()?;
        let current = queue.current().and_then(|current| current.id.clone());
        if current.as_deref() != Some(&id) {
            return None;
        }
        let local = music::is_local_id(&id);
        let playing = playback.wants_playing();
        if playing && !self.claimed {
            self.claimed = true;
            if local {
                self.pause_elsewhere();
            }
        }
        if !self.claimed || local {
            return None;
        }
        Some(Arc::new(snapshot(playback, queue)?))
    }

    /// Pauses the other device that has playback while it plays. A report takes playback off it
    /// on its own, but a local file sends none and would otherwise play over it.
    fn pause_elsewhere(&self) {
        let playing = self
            .roster
            .elsewhere
            .as_ref()
            .filter(|elsewhere| elsewhere.state.playing);
        if let (Some(link), Some(elsewhere)) = (&self.link, playing) {
            link.send(Request::Control(
                elsewhere.device.id.clone(),
                Command::Pause,
            ));
        }
    }
}

impl Store {
    /// The device network with the models it reads, split off the store so both can be
    /// borrowed.
    fn devices_parts(&mut self) -> (&mut Devices, Around<'_>, &mut Cx) {
        let Store {
            devices,
            session,
            settings,
            playback,
            queue,
            cx,
            ..
        } = self;
        let around = Around {
            session,
            settings,
            playback,
            queue,
        };
        (devices, around, cx)
    }

    fn publish_devices(&mut self) {
        let (devices, around, cx) = self.devices_parts();
        devices.publish(around.playback, around.queue, cx);
    }

    /// Applies already-delivered events together without delaying commands or scheduling a timer.
    fn on_device_events(&mut self, events: Vec<ConnectEvent>) {
        let mut changed = false;
        let mut listed = false;
        let mut playlists = HashSet::new();
        for event in events {
            match event {
                ConnectEvent::Roster(roster) => {
                    let (devices, around, cx) = self.devices_parts();
                    if devices.set_roster(roster, around.session, cx) {
                        self.locally(|store| store.pause_playback());
                    }
                }
                ConnectEvent::Command(command) => self.run_device_command(command),
                ConnectEvent::Liked { track, liked } => self.liked_elsewhere(track, liked),
                ConnectEvent::LibraryChanged => {
                    changed = true;
                    listed = true;
                }
                ConnectEvent::PlaylistChanged(id) => {
                    changed = true;
                    playlists.insert(id);
                }
                ConnectEvent::Jam(mut jam) => {
                    let this = &mut self.devices;
                    if this.leaving {
                        if jam.is_some() {
                            continue;
                        }
                        this.leaving = false;
                    }
                    // an update to a Jam the listener owns can come without its link or its owner,
                    // which would take the code off screen and bring it back with its entrance played
                    // again, so both carry over from what was known
                    if let (Some(jam), Some(known)) = (jam.as_mut(), this.jam.as_ref())
                        && known.owner
                        && jam.link.is_none()
                    {
                        jam.owner = true;
                        jam.link = known.link.clone();
                    }
                    // a Jam the listener started shows its code once, as it comes up
                    if this.awaiting_code && jam.as_ref().is_some_and(|jam| jam.link.is_some()) {
                        this.awaiting_code = false;
                        this.code = true;
                    }
                    if jam.is_none() {
                        this.code = false;
                    }
                    this.jam = jam;
                    self.cx.changed(Topic::Devices);
                }
                ConnectEvent::Invite(invite) => {
                    if invite.is_none() {
                        Toasts::show(Outcome::Failed, "toast-jam-unreadable", &mut self.cx);
                    }
                    self.devices.invite = invite;
                    self.cx.changed(Topic::Devices);
                }
                ConnectEvent::LeaveFailed => {
                    self.devices.leaving = false;
                    Toasts::show(Outcome::Failed, "toast-jam-leave-failed", &mut self.cx);
                }
                ConnectEvent::JamFailed => {
                    self.devices.awaiting_code = false;
                    Toasts::show(Outcome::Failed, "toast-jam-failed", &mut self.cx);
                }
            }
        }
        if changed {
            self.library
                .changed_elsewhere(playlists, listed, &self.session, &mut self.cx);
        }
    }

    /// Carries out what another device asked of this one, on this app's own engine rather than
    /// steering whichever device has playback.
    fn run_device_command(&mut self, command: Command) {
        match command {
            Command::Play => self.locally(|store| store.resume_playback()),
            Command::Pause => self.locally(|store| store.pause_playback()),
            Command::Next => self.locally(|store| store.next_track()),
            Command::Previous => self.locally(|store| store.previous_track()),
            Command::Seek(at) => self.locally(|store| store.seek_to(at)),
            Command::Volume(level) => self.locally(|store| store.set_volume(level)),
            Command::Shuffle(on) => self.queue.set_shuffle(on, &mut self.cx),
            Command::Repeat(mode) => self.locally(|store| store.set_repeat(mode)),
            Command::Enqueue { id, added_by } => {
                let Store {
                    devices,
                    session,
                    cx,
                    ..
                } = self;
                devices.enqueue(id, added_by, session, cx);
            }
            Command::Start(start) => self.start_from_device(start),
            Command::SetQueue { previous, upcoming } => self.set_device_queue(previous, upcoming),
            Command::Released => {
                // another device took over, so this one stops without calling playback back
                self.devices.changing = None;
                self.devices.claimed = false;
                self.locally(|store| store.pause_playback());
                self.publish_devices();
            }
        }
    }

    /// Shows a like made in another of the account's apps, reading the track in first when it
    /// was liked rather than unliked.
    fn liked_elsewhere(&mut self, id: String, liked: bool) {
        // an unlike also calls off a like of the same track still being read in
        self.devices.liking.remove(&id);
        if !liked {
            return self.library.liked_elsewhere(&id, None, &mut self.cx);
        }
        // a like this app made comes back the same way, with nothing left to show
        if self.library.saved(&id) {
            return;
        }
        let Some(client) = self.session.client() else {
            return;
        };
        let slot = id.clone();
        let wanted = id.clone();
        let task = self.cx.load(
            async move { client.track(&wanted).await },
            move |store, found| {
                store.devices.liking.remove(&id);
                match found {
                    Ok(track) => store
                        .library
                        .liked_elsewhere(&id, Some(track), &mut store.cx),
                    Err(error) => {
                        log::warn!("connect: cannot read the liked track {id}: {error:#}")
                    }
                }
            },
        );
        self.devices.liking.insert(slot, task);
    }

    /// Accepts queue edits made in another Spotify client without resetting the current song.
    fn set_device_queue(&mut self, previous: Vec<QueuedTrack>, upcoming: Vec<QueuedTrack>) {
        let Some(state) = snapshot(&self.playback, &self.queue) else {
            return;
        };
        // the edit was made on the last report, and one made on an older queue would undo what
        // changed here since, so it gives way to the report of what this queue holds now
        if !self
            .devices
            .sent
            .as_deref()
            .is_some_and(|sent| same_queue(sent, &state))
        {
            log::debug!("connect: dropping a queue edit made on an older report");
            self.devices.sent = None;
            self.publish_devices();
            return;
        }
        self.start_from_device(Start {
            state: Arc::new(Snapshot {
                previous,
                upcoming,
                ..state
            }),
            preserve_queue: true,
            restart: false,
        });
    }

    /// Plays what another device handed over from where it left off, or applies its edit to the
    /// queue, once what it names has loaded. Nothing is applied if the listener changed the queue
    /// here while it loaded: a handover gives way to any change, and an edit to one that moved the
    /// songs the report showed. Songs already in the queue are not read from the provider again.
    fn start_from_device(&mut self, start: Start) {
        let Some(client) = self.session.client() else {
            return;
        };
        let state = start.state.clone();
        let restart = start.restart;
        let reach = match restart {
            true => Reach::Handed,
            false => Reach::Reported {
                past: PREVIOUS,
                upcoming: UPCOMING,
            },
        };
        let revision = self.queue.revision();
        let shown = snapshot(&self.playback, &self.queue);
        let mut known = self.devices.listed.clone();
        known.extend(
            self.queue
                .tracks()
                .filter_map(|track| Some((track.id.clone()?, track.clone()))),
        );
        self.devices.changing = Some(self.cx.load(
            load(client, start, known),
            move |store, loaded| {
                store.devices.changing = None;
                let moved = match restart {
                    true => store.queue.revision() != revision,
                    false => match (snapshot(&store.playback, &store.queue), &shown) {
                        (Some(now), Some(shown)) => !same_queue(&now, shown),
                        _ => true,
                    },
                };
                if moved {
                    store.publish_devices();
                    return;
                }
                match loaded {
                    Ok((tracks, index, origin, window)) => {
                        if restart {
                            store.devices.claimed = true;
                            store.locally(|store| {
                                if let Some(shuffle) = state.shuffle {
                                    store.queue.set_shuffle(shuffle, &mut store.cx);
                                }
                                if let Some(repeat) = state.repeat {
                                    store.set_repeat(repeat);
                                }
                                store.play_tracks_at(tracks, index, origin, state.live_position());
                                if !state.playing {
                                    store.pause_playback();
                                }
                            });
                        }
                        // a resolved remote queue window applies without restarting playback
                        if let Some((past, current, next)) = window {
                            store
                                .queue
                                .replace_remote(past, current, next, reach, &mut store.cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("connect: cannot apply playback change: {error:#}");
                        store.devices.sent = None;
                        store.publish_devices();
                    }
                }
                store.cx.changed(Topic::Devices);
            },
        ));
    }

    /// Follows the account, playback, the queue and the settings: the device network is taken
    /// again on every sign-in, sign-out, reconnect or going offline, what plays is reported as it
    /// changes, and the device list follows its setting.
    pub(crate) fn react_devices(&mut self, change: &Change) {
        match change {
            Change::Event(Event::Session(event)) => match event {
                SessionEvent::LocalChanged => {}
                SessionEvent::SignedIn
                | SessionEvent::SignedOut
                | SessionEvent::Reconnected
                | SessionEvent::WentOffline => {
                    let (devices, around, cx) = self.devices_parts();
                    devices.relink(around, cx);
                }
            },
            Change::Topic(Topic::Playback | Topic::Queue) => self.publish_devices(),
            Change::Topic(Topic::Settings) => {
                let (devices, around, cx) = self.devices_parts();
                devices.apply(around, cx);
            }
            _ => {}
        }
    }
}

/// Hears the device network, a batch of already-delivered events at a time.
fn listen(mut events: UnboundedReceiver<ConnectEvent>, cx: &mut Cx) -> Job {
    cx.spawn(|link| async move {
        while let Some(event) = events.recv().await {
            let queued: Vec<ConnectEvent> = std::iter::once(event)
                .chain(std::iter::from_fn(|| events.try_recv().ok()))
                .collect();
            if link
                .read(move |store| store.on_device_events(queued))
                .await
                .is_none()
            {
                break;
            }
        }
    })
}

/// Tells the views, and everything that follows playback such as the lyrics, that what the
/// other device plays has changed or moved on.
fn redraw(cx: &mut Cx) {
    cx.changed(Topic::Devices);
    cx.changed(Topic::Playback);
}

/// The local queue and playback as a snapshot, regardless of who has playback.
fn snapshot(playback: &Playback, queue: &Queue) -> Option<Snapshot> {
    let track = queue.current()?;
    let context = queue
        .origin()
        .or_else(|| queue.context_origin())
        .and_then(collection_of);
    let mut previous: Vec<_> = queue.connect_previous().rev().take(PREVIOUS).collect();
    previous.reverse();
    Some(Snapshot {
        track: queue.connect_current(),
        previous,
        upcoming: queue.connect_upcoming().take(UPCOMING).collect(),
        index: context.is_some().then(|| queue.place()).flatten(),
        context,
        playing: playback.wants_playing(),
        position: playback.live_position(),
        duration: track.duration,
        volume: playback.volume(),
        shuffle: Some(queue.shuffle()),
        repeat: Some(playback.repeat()),
        stamp: Some(SystemTime::now()),
    })
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

/// Reads a track and, when `adder` is someone the Jam does not list, their profile.
async fn read_track(
    client: Arc<dyn MusicApi>,
    id: String,
    adder: Option<String>,
) -> Result<(Track, Option<(String, Arc<Contributor>)>)> {
    let track = client.track(&id).await?;
    let person = match adder {
        Some(adder) => person(&*client, adder).await,
        None => None,
    };
    Ok((track, person))
}

/// The profile of someone a queue names by an id the Jam's members do not list, keyed by that id.
async fn person(client: &dyn MusicApi, id: String) -> Option<(String, Arc<Contributor>)> {
    match client.user(&id).await {
        Ok(user) => Some((
            id,
            Arc::new(Contributor {
                id: user.id,
                name: user.name,
                avatar: user.avatar,
            }),
        )),
        Err(error) => {
            log::warn!("connect: cannot read who queued a track: {error:#}");
            None
        }
    }
}

/// Reads in what a start names: the collection if it has one the track is in, otherwise the
/// track and the tracks that follow it.
async fn load(
    client: Arc<dyn MusicApi>,
    start: Start,
    mut known: HashMap<String, Track>,
) -> Result<(Vec<Track>, usize, Option<Origin>, Option<RemoteWindow>)> {
    let Start {
        state: start,
        preserve_queue,
        restart,
    } = start;
    let (tracks, origin) = if restart {
        match &start.context {
            Some(Collection::Album(id)) => {
                (client.album_tracks(id).await?, Some(Origin::album(id)))
            }
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
        }
    } else {
        (Vec::new(), None)
    };

    known.extend(
        tracks
            .iter()
            .filter_map(|track| Some((track.id.clone()?, track.clone()))),
    );
    let accepts = |track: &Track| {
        start
            .track
            .as_ref()
            .is_none_or(|listed| track.id.as_ref() == Some(&listed.id))
    };
    let index = start
        .index
        .filter(|at| tracks.get(*at).is_some_and(accepts))
        .or_else(|| tracks.iter().position(accepts));
    let resolved = if preserve_queue || index.is_none() {
        let ids = start
            .previous
            .iter()
            .chain(&start.track)
            .chain(&start.upcoming)
            .map(|track| &track.id);
        resolve_queue(&*client, &mut known, ids.cloned().collect()).await
    } else {
        Vec::new()
    };
    let (tracks, index) = if restart {
        match index {
            Some(index) => (tracks, index),
            None => {
                anyhow::ensure!(
                    start.track.is_some()
                        && resolved
                            .get(start.previous.len())
                            .is_some_and(Option::is_some),
                    "cannot load the transferred current track"
                );
                let index = resolved.iter().take(start.previous.len()).flatten().count();
                let tracks: Vec<_> = resolved.iter().flatten().cloned().collect();
                (tracks, index)
            }
        }
    } else {
        (Vec::new(), 0)
    };
    let window = preserve_queue.then(|| {
        let mut resolved = resolved.into_iter();
        let past = resolved
            .by_ref()
            .take(start.previous.len())
            .zip(&start.previous)
            .filter_map(|(track, listed)| Some((track?, listed.queued)))
            .collect();
        // The current song has its own slot in the resolver, outside history and upcoming.
        if start.track.is_some() {
            resolved.next();
        }
        let next = resolved
            .zip(&start.upcoming)
            .filter_map(|(track, listed)| Some((track?, listed.queued)))
            .collect();
        (
            past,
            start.track.as_ref().is_some_and(|track| track.queued),
            next,
        )
    });

    Ok((tracks, index, origin, window))
}

/// Fetches each missing song once, concurrently, then reconstructs the requested order.
/// Repeated occurrences share metadata; an unavailable song remains an explicit gap.
async fn resolve_queue(
    client: &dyn MusicApi,
    known: &mut HashMap<String, Track>,
    ids: Vec<String>,
) -> Vec<Option<Track>> {
    let mut requested = HashSet::new();
    let missing: Vec<_> = ids
        .iter()
        .filter(|id| !known.contains_key(*id) && requested.insert(*id))
        .cloned()
        .collect();
    let mut reads = stream::iter(missing)
        .map(|id| async move { (id.clone(), client.track(&id).await) })
        .buffer_unordered(QUEUE_READS);
    while let Some((id, result)) = reads.next().await {
        match result {
            Ok(track) if track.id.as_ref() == Some(&id) => {
                known.insert(id, track);
            }
            result => log::warn!("connect: cannot resolve queued track {id}: {result:?}"),
        }
    }
    ids.iter().map(|id| known.get(id).cloned()).collect()
}

/// Whether two reports show the same queue around the same song, whatever their clocks say.
fn same_queue(a: &Snapshot, b: &Snapshot) -> bool {
    a.track == b.track && a.previous == b.previous && a.upcoming == b.upcoming
}
