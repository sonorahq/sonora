use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use futures::{StreamExt as _, stream};
use gpui::{App, Context, Entity, Task};
use music::connect::{
    Collection, Command, Connect, Device, Elsewhere, Event, Invite, Jam, Naming, QueuedTrack,
    Request, Roster, Snapshot, Start,
};
use music::{Contributor, MusicApi, Track};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::queue::Reach;
use crate::{
    AppSettings, ConnectName, Io, Library, Origin, Outcome, Playback, Queue, QueueKind,
    QueuePosition, QueueSections, Session, SessionEvent, Toasts, Whence,
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
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    library: Entity<Library>,
    settings: Entity<AppSettings>,
    session: Entity<Session>,
    io: Io,
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
    listing: Option<Task<()>>,
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
    events: Option<Task<()>>,
    /// The latest transfer or queue edit; a newer request cancels its predecessor.
    changing: Option<Task<()>>,
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
            SessionEvent::SignedIn
            | SessionEvent::SignedOut
            | SessionEvent::Reconnected
            | SessionEvent::WentOffline => this.relink(cx),
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
    pub fn peek(&mut self, link: &str, cx: &mut Context<Self>) {
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
    pub fn accept(&mut self, cx: &mut Context<Self>) {
        if let (Some(invite), Some(link)) = (self.invite.take(), &self.link) {
            link.send(Request::Join(invite.link));
            self.leaving = false;
        }
        cx.notify();
    }

    /// Drops the invite without joining.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.invite = None;
        cx.notify();
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
    pub fn show_code(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.code = shown;
        cx.notify();
    }

    /// Whether the dialog that takes a Jam link to join is open.
    pub fn join_shown(&self) -> bool {
        self.joining
    }

    /// Opens or closes the dialog that takes a Jam link to join.
    pub fn show_join(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.joining = shown;
        cx.notify();
    }

    /// Leaves the group session, or ends it when the listener started it.
    /// Takes the Jam off screen at once, says it is over, and asks the provider to follow. A
    /// refusal brings it back with a toast.
    pub fn leave(&mut self, cx: &mut Context<Self>) {
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
        cx.notify();
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
    /// The caller notifies playback, since playback may already be updating when this runs.
    pub(crate) fn set_volume(&mut self, device: &str, level: f32, cx: &mut Context<Self>) {
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
        cx.notify();
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

        self.reset(link);
        if let Some(events) = self.link.as_ref().and_then(|link| link.events()) {
            self.events = Some(self.listen(events, cx));
        }
        self.apply(cx);
        if let Some(asked) = self.asked.take()
            && self.link.is_some()
        {
            self.peek(&asked, cx);
        }
        cx.notify();
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
            link.send(Request::Rename(naming.clone()));
            self.named = Some(naming);
        }
        if wanted == self.shown {
            return;
        }
        self.shown = wanted;
        link.send(Request::Enable(wanted));
        if wanted {
            self.publish(cx);
        } else {
            let named = self.named.take();
            let events = self.events.take();
            self.reset(Some(link));
            self.named = named;
            self.events = events;
        }
        cx.notify();
    }

    fn listen(&mut self, mut events: UnboundedReceiver<Event>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let queued = std::iter::once(event)
                            .chain(std::iter::from_fn(|| events.try_recv().ok()));
                        this.on_events(queued, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    /// Applies already-delivered events together without delaying commands or scheduling a timer.
    fn on_events(&mut self, events: impl Iterator<Item = Event>, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut listed = false;
        let mut playlists = HashSet::new();
        for event in events {
            match event {
                Event::Roster(roster) => self.set_roster(roster, cx),
                Event::Command(command) => self.run(command, cx),
                Event::Liked { track, liked } => self.liked(track, liked, cx),
                Event::LibraryChanged => {
                    changed = true;
                    listed = true;
                }
                Event::PlaylistChanged(id) => {
                    changed = true;
                    playlists.insert(id);
                }
                Event::Jam(mut jam) => {
                    if self.leaving {
                        if jam.is_some() {
                            continue;
                        }
                        self.leaving = false;
                    }
                    // an update to a Jam the listener owns can come without its link or its owner,
                    // which would take the code off screen and bring it back with its entrance played
                    // again, so both carry over from what was known
                    if let (Some(jam), Some(known)) = (jam.as_mut(), self.jam.as_ref())
                        && known.owner
                        && jam.link.is_none()
                    {
                        jam.owner = true;
                        jam.link = known.link.clone();
                    }
                    // a Jam the listener started shows its code once, as it comes up
                    if self.awaiting_code && jam.as_ref().is_some_and(|jam| jam.link.is_some()) {
                        self.awaiting_code = false;
                        self.code = true;
                    }
                    if jam.is_none() {
                        self.code = false;
                    }
                    self.jam = jam;
                    cx.notify();
                }
                Event::Invite(invite) => {
                    if invite.is_none() {
                        Toasts::show(Outcome::Failed, "toast-jam-unreadable", cx);
                    }
                    self.invite = invite;
                    cx.notify();
                }
                Event::LeaveFailed => {
                    self.leaving = false;
                    Toasts::show(Outcome::Failed, "toast-jam-leave-failed", cx);
                }
                Event::JamFailed => {
                    self.awaiting_code = false;
                    Toasts::show(Outcome::Failed, "toast-jam-failed", cx);
                }
            }
        }
        if changed {
            self.library.update(cx, |library, cx| {
                library.changed_elsewhere(playlists, listed, cx)
            });
        }
    }

    fn set_roster(&mut self, roster: Roster, cx: &mut Context<Self>) {
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
        if started && self.claimed && self.sent.is_none() {
            self.claimed = false;
            self.here(cx, |playback, cx| playback.pause(cx));
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
        self.list(cx);
        self.tick(cx);
        self.redraw(cx);
    }

    /// Resolves the active song, history and upcoming tracks through one cache and task.
    /// Missing metadata is retried on the next song, with bounded batches for queue reads.
    fn list(&mut self, cx: &mut Context<Self>) {
        if self.listing.is_some() {
            return;
        }
        let Some(client) = self.session.read(cx).client() else {
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
        self.listing = Some(self.io.load(
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
            move |this, read, cx| {
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
                this.list(cx);
                this.redraw(cx);
            },
            cx,
        ));
    }

    /// Tells the views, and everything that follows playback such as the lyrics, that what the
    /// other device plays has changed or moved on.
    fn redraw(&self, cx: &mut Context<Self>) {
        cx.notify();
        self.playback.update(cx, |_, cx| cx.notify());
    }

    /// Whether another device has playback that is moving on.
    fn moving(&self) -> bool {
        self.available() && self.elsewhere().is_some_and(|e| e.state.playing)
    }

    /// Redraws a remote device's advancing clock until playback stops or moves here.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.moving() {
            self.ticking = None;
        } else if self.ticking.is_none() {
            self.ticking = Some(cx.spawn(async move |this, cx| {
                while this
                    .update(cx, |this, cx| {
                        if this.moving() {
                            this.redraw(cx);
                        } else {
                            this.ticking = None;
                        }
                        this.moving()
                    })
                    .unwrap_or(false)
                {
                    cx.background_executor().timer(TICK).await;
                }
            }));
        }
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
            Command::Repeat(mode) => self.here(cx, |playback, cx| playback.set_repeat(mode, cx)),
            Command::Enqueue { id, added_by } => self.enqueue(id, added_by, cx),
            Command::Start(start) => self.start(start, cx),
            Command::SetQueue { previous, upcoming } => self.set_queue(previous, upcoming, cx),
            Command::Released => {
                // another device took over, so this one stops without calling playback back
                self.changing = None;
                self.claimed = false;
                self.here(cx, |playback, cx| playback.pause(cx));
                self.publish(cx);
            }
        }
    }

    fn enqueue(&mut self, id: String, added_by: Option<String>, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        let key = id.clone();
        let slot = id.clone();
        let stranger = added_by.clone().filter(|adder| self.known(adder).is_none());
        let task = self.io.load(
            read_track(client, id, stranger),
            move |this, found, cx| {
                this.queuing.remove(&key);
                match found {
                    Ok((mut track, person)) => {
                        this.people.extend(person);
                        track.added_by = added_by.as_deref().and_then(|id| this.known(id));
                        this.here(cx, |playback, cx| playback.enqueue(track, cx))
                    }
                    Err(error) => log::warn!("connect: cannot queue {key}: {error:#}"),
                }
            },
            cx,
        );
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
        let slot = id.clone();
        let wanted = id.clone();
        let task = self.io.load(
            async move { client.track(&wanted).await },
            move |this, found, cx| {
                this.liking.remove(&id);
                match found {
                    Ok(track) => this.library.update(cx, |library, cx| {
                        library.liked_elsewhere(&id, Some(track), cx)
                    }),
                    Err(error) => {
                        log::warn!("connect: cannot read the liked track {id}: {error:#}")
                    }
                }
            },
            cx,
        );
        self.liking.insert(slot, task);
    }

    /// Accepts queue edits made in another Spotify client without resetting the current song.
    fn set_queue(
        &mut self,
        previous: Vec<QueuedTrack>,
        upcoming: Vec<QueuedTrack>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.snapshot(cx) else {
            return;
        };
        // the edit was made on the last report, and one made on an older queue would undo what
        // changed here since, so it gives way to the report of what this queue holds now
        if !self
            .sent
            .as_deref()
            .is_some_and(|sent| same_queue(sent, &state))
        {
            log::debug!("connect: dropping a queue edit made on an older report");
            self.sent = None;
            self.publish(cx);
            return;
        }
        self.start(
            Start {
                state: Arc::new(Snapshot {
                    previous,
                    upcoming,
                    ..state
                }),
                preserve_queue: true,
                restart: false,
            },
            cx,
        );
    }

    /// Applies a resolved remote queue window without restarting playback.
    fn apply_window(
        &mut self,
        (past, current, next): RemoteWindow,
        reach: Reach,
        cx: &mut Context<Self>,
    ) {
        self.queue.update(cx, |queue, cx| {
            queue.replace_remote(past, current, next, reach, cx)
        });
    }

    /// Plays what another device handed over from where it left off, or applies its edit to the
    /// queue, once what it names has loaded. Nothing is applied if the listener changed the queue
    /// here while it loaded: a handover gives way to any change, and an edit to one that moved the
    /// songs the report showed. Songs already in the queue are not read from the provider again.
    fn start(&mut self, start: Start, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
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
        let revision = self.queue.read(cx).revision();
        let shown = self.snapshot(cx);
        let queue = self.queue.read(cx);
        let mut known = self.listed.clone();
        known.extend(
            queue
                .tracks()
                .filter_map(|track| Some((track.id.clone()?, track.clone()))),
        );
        self.changing = Some(self.io.load(
            load(client, start, known),
            move |this, loaded, cx| {
                this.changing = None;
                let moved = match restart {
                    true => this.queue.read(cx).revision() != revision,
                    false => match (this.snapshot(cx), &shown) {
                        (Some(now), Some(shown)) => !same_queue(&now, shown),
                        _ => true,
                    },
                };
                if moved {
                    this.publish(cx);
                    return;
                }
                match loaded {
                    Ok((tracks, index, origin, window)) => {
                        if restart {
                            this.claimed = true;
                            let queue = this.queue.clone();
                            this.here(cx, |playback, cx| {
                                if let Some(shuffle) = state.shuffle {
                                    queue.update(cx, |queue, cx| queue.set_shuffle(shuffle, cx));
                                }
                                if let Some(repeat) = state.repeat {
                                    playback.set_repeat(repeat, cx);
                                }
                                playback.start_at(tracks, index, origin, state.live_position(), cx);
                                if !state.playing {
                                    playback.pause(cx);
                                }
                            });
                        }
                        if let Some(window) = window {
                            this.apply_window(window, reach, cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("connect: cannot apply playback change: {error:#}");
                        this.sent = None;
                        this.publish(cx);
                    }
                }
                cx.notify();
            },
            cx,
        ));
    }

    /// Tells the account's other apps what plays, when something does that this app took.
    fn publish(&mut self, cx: &mut Context<Self>) {
        if self.changing.is_some() {
            return;
        }
        let Some(link) = self.link.clone() else {
            return;
        };
        if !self.shown {
            return;
        }

        let Some(now) = self.now_playing(cx) else {
            if self.sent.take().is_some() {
                link.send(Request::Publish(None));
                cx.notify();
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
        cx.notify();
    }

    /// The local queue and playback as a snapshot, regardless of who has playback.
    fn snapshot(&self, cx: &App) -> Option<Snapshot> {
        let playback = self.playback.read(cx);
        let queue = self.queue.read(cx);
        let track = queue.current()?;
        let context = playback
            .origin(cx)
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

    /// What this app plays, for the report to the other devices, once it took playback. Playing
    /// anything here takes it, a local file too, though a local file is never reported.
    fn now_playing(&mut self, cx: &App) -> Option<Arc<Snapshot>> {
        let playback = self.playback.read(cx);
        let track = playback.track()?;
        let id = track.id.clone()?;
        let current = self
            .queue
            .read(cx)
            .current()
            .and_then(|current| current.id.clone());
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
        Some(Arc::new(self.snapshot(cx)?))
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
