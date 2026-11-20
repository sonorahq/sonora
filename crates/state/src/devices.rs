use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use futures::{StreamExt as _, stream};
use gpui::{App, Context, Entity, Task};
use music::connect::{
    Collection, Command, Connect, Device, Event, Invite, Jam, Naming, PREVIOUS, Published,
    QueuedTrack, Request, Roster, Snapshot, Start, Transport, UPCOMING,
};
use music::{Contributor, MusicApi, Track};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::queue::Reach;
use crate::{
    AppSettings, ConnectName, Io, Library, Origin, Outcome, Playback, Queue, QueueKind, Session,
    SessionEvent, Toasts, Whence,
};

/// Resolving a queue should not fan out into a request for every track at once.
const QUEUE_READS: usize = 8;
/// How often the views are told that another device's playback moved on while it plays.
const TICK: Duration = Duration::from_millis(500);
/// The actual history and upcoming order supplied by another device.
type RemoteWindow = (Vec<(Track, QueueKind)>, QueueKind, Vec<(Track, QueueKind)>);

/// A seek another device asked for while a song was loading, kept until that song plays.
struct LateSeek {
    /// The id of the song it was asked for.
    track: Option<String>,
    at: Duration,
    /// When it was asked, since a song that plays has moved on by the time it loads.
    asked: Instant,
}

/// The last report’s device and track, with a fingerprint of its queue order.
struct ActiveDevice {
    device: Device,
    track: Option<String>,
    playing: bool,
    order: Option<u64>,
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
    link: Option<Arc<Connect>>,
    /// Whether this app is on the device list, as far as the link was told.
    shown: bool,
    observing: bool,
    local_device: Option<String>,
    /// The name last handed to the link.
    named: Option<Naming>,
    devices: Vec<Device>,
    active: Option<ActiveDevice>,
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
    /// People who queued songs and are not in the Jam's member list, read from their profiles,
    /// by the id the queue names them by.
    people: HashMap<String, Arc<Contributor>>,
    /// Whether this app took playback, which only the listener pressing play does. A track
    /// restored paused at launch must not pull playback off another device.
    claimed: bool,
    sent: Option<Published>,
    events: Option<Task<()>>,
    /// A queue load and the latest playback report when it adopts a device.
    changing: Option<(Task<()>, Option<Transport>)>,
    /// A seek another device asked for while the song was loading here. It is made once the
    /// song plays, unless another song has taken its place by then.
    seek_due: Option<LateSeek>,
    queuing: HashMap<String, Task<()>>,
    liking: HashMap<String, Task<()>>,
    /// Notifies `Playback::ticks` twice a second while another device plays, so its progress is
    /// redrawn.
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
        cx.observe(&playback, |this, _, cx| {
            this.seek_late(cx);
            this.publish(cx);
        })
        .detach();
        // the report to the other devices also follows the position as it moves
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |this, _, cx| this.publish(cx)).detach();
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
            observing: false,
            local_device: None,
            named: None,
            devices: Vec::new(),
            active: None,
            jam: None,
            invite: None,
            asked: None,
            awaiting_code: false,
            code: false,
            joining: false,
            leaving: false,
            people: HashMap::new(),
            claimed: false,
            sent: None,
            events: None,
            changing: None,
            seek_due: None,
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

    /// Whether the device network has group sessions to start and join.
    pub fn jams(&self) -> bool {
        !self.observing && self.link.as_ref().is_some_and(|link| link.jams())
    }

    /// Whether this app's playback is what the account's other devices are being told about.
    pub fn publishing(&self) -> bool {
        self.available() && self.sent.is_some()
    }

    /// The account's other devices.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The other device that has playback, if one does. `None` once this app took playback, even
    /// before the network's next roster stops naming the device it came from.
    pub fn elsewhere(&self) -> Option<&Device> {
        self.steered_device().map(|remote| &remote.device)
    }

    fn is_native(&self, device: &Device) -> bool {
        Some(device.id.as_str()) == self.local_device.as_deref()
    }

    fn steered_device(&self) -> Option<&ActiveDevice> {
        self.active.as_ref().filter(|remote| {
            let local = self.is_native(&remote.device);
            (self.shown || local) && !self.claimed && (!local || self.adopting())
        })
    }

    /// Whether the current queue load projects an active device into Sonora.
    fn adopting(&self) -> bool {
        self.changing
            .as_ref()
            .is_some_and(|(_, report)| report.is_some())
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
        let Some(link) = &self.link else {
            return;
        };
        let target = match self.observing {
            true => self.local_device.as_deref(),
            false => Some(link.device()),
        };
        if let Some(target) = target {
            self.transfer(target);
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
        if let Some(listed) = self.devices.iter_mut().find(|listed| listed.id == device) {
            listed.volume = level;
        }
        if let Some(elsewhere) = self.active.as_mut().filter(|e| e.device.id == device) {
            elsewhere.device.volume = level;
        }
        cx.notify();
    }

    /// Seeks here as another device asked. A seek into a song that is still loading leaves
    /// playback showing it as loading for good, so that one waits for the song to play.
    fn seek(&mut self, at: Duration, cx: &mut Context<Self>) {
        match self.playback.read(cx).is_loading() {
            true => {
                self.seek_due = Some(LateSeek {
                    track: self.current_id(cx).map(str::to_owned),
                    at,
                    asked: Instant::now(),
                })
            }
            false => self.here(cx, |playback, cx| playback.seek(at, cx)),
        }
    }

    /// The id of the song that plays or loads here.
    fn current_id<'a>(&self, cx: &'a App) -> Option<&'a str> {
        self.queue
            .read(cx)
            .current()
            .and_then(|track| track.id.as_deref())
    }

    /// Makes the seek that waited for the song to load, from as far on as the song it follows
    /// has got since.
    fn seek_late(&mut self, cx: &mut Context<Self>) {
        let Some(due) = &self.seek_due else {
            return;
        };
        // the seek was meant for the song that was loading, not for one that came after it
        if due.track.as_deref() != self.current_id(cx) {
            self.seek_due = None;
            return;
        }
        let playback = self.playback.read(cx);
        if playback.is_loading() {
            return;
        }
        if let Some(due) = self.seek_due.take() {
            let at = if playback.wants_playing() {
                due.at + due.asked.elapsed()
            } else {
                due.at
            };
            self.here(cx, |playback, cx| playback.seek(at, cx));
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

        self.here(cx, |playback, cx| playback.release_external(cx));
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
        self.observing = false;
        self.local_device = None;
        self.named = None;
        self.devices.clear();
        self.active = None;
        self.jam = None;
        self.invite = None;
        self.awaiting_code = false;
        self.code = false;
        self.joining = false;
        self.leaving = false;
        self.people.clear();
        self.claimed = false;
        self.sent = None;
        self.events = None;
        self.changing = None;
        self.queuing.clear();
        self.liking.clear();
        self.ticking = None;
    }

    /// Keeps account discovery available while Soloist owns the visible Connect device.
    pub(crate) fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.link.clone() else {
            return;
        };
        let settings = self.settings.read(cx);
        let wanted = settings.spotify_connect();
        let observing = self.session.read(cx).offers_soloist() && music::soloist::ready();
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
        if wanted == self.shown && observing == self.observing {
            return;
        }
        self.shown = wanted;
        self.observing = observing;
        if observing {
            self.sent = None;
            self.claimed = false;
        }
        link.send(match observing {
            true => Request::Observe,
            false => Request::Enable(wanted),
        });
        if wanted || observing {
            self.publish(cx);
        } else {
            let named = self.named.take();
            let events = self.events.take();
            self.here(cx, |playback, cx| playback.release_external(cx));
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
        let mut listed = false;
        let mut playlists = HashSet::new();
        for event in events {
            match event {
                Event::Roster(roster) => self.set_roster(roster, cx),
                Event::Command(command) => self.run(command, cx),
                Event::Liked { track, liked } => self.liked(track, liked, cx),
                Event::LibraryChanged => listed = true,
                Event::PlaylistChanged(id) => {
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
        if listed || !playlists.is_empty() {
            self.library.update(cx, |library, cx| {
                library.changed_elsewhere(playlists, listed, cx)
            });
        }
    }

    fn set_roster(&mut self, mut roster: Roster, cx: &mut Context<Self>) {
        self.local_device = self
            .observing
            .then(|| self.playback.read(cx).connect_device().map(str::to_owned))
            .flatten()
            .filter(|id| {
                roster.devices.iter().any(|device| device.id == *id)
                    || roster
                        .active
                        .as_ref()
                        .is_some_and(|(device, _)| device.id == *id)
            });
        roster
            .devices
            .retain(|device| Some(device.id.as_str()) != self.local_device.as_deref());
        let before = self.active.as_ref();
        let incoming = roster.active.as_ref();
        let reported_volume = before.zip(incoming).and_then(|(had, (now, _))| {
            (had.device.id == now.id && had.device.volume != now.volume)
                .then(|| (now.id.clone(), now.volume))
        });
        let moved = before.and_then(|remote| remote.track.as_deref())
            != incoming.and_then(|(_, state)| state.track_id());
        let from_other =
            before.map(|remote| &remote.device.id) != incoming.map(|(device, _)| &device.id);
        let native = incoming.is_some_and(|(device, _)| self.is_native(device));
        let order = if native {
            before
                .filter(|_| !from_other && !moved)
                .and_then(|before| before.order)
        } else {
            incoming.map(|(_, state)| queue_order(state))
        };
        let selected = incoming
            .filter(|_| !native && self.shown && !self.claimed)
            .map(|(_, state)| state);
        let playback = self.playback.read(cx);
        let current = playback.track().and_then(|track| track.id.as_deref());
        let adopted = selected.and_then(|state| {
            let wanted = state.track_id()?;
            let restart = from_other || (moved && current != Some(wanted));
            let edit = current == Some(wanted)
                && !moved
                && !playback.is_loading()
                && before.is_some_and(|before| before.order != order)
                && self
                    .snapshot(cx)
                    .is_some_and(|local| Some(queue_order(&local)) != order);
            (restart || edit).then(|| (state.clone(), restart))
        });
        let refresh = native && (from_other || playback.is_external() && !self.adopting());
        if from_other {
            if self.observing {
                self.sent = None;
            }
            if self.adopting() {
                self.changing = None;
            }
        }
        let started = incoming.is_some_and(|(_, state)| state.transport.playing)
            && !before.is_some_and(|remote| remote.playing);
        let external = selected.map(|state| (state.transport, state.track_id() == current));
        if let Some((_, Some(report))) = &mut self.changing
            && let Some(state) = selected
        {
            *report = state.transport;
        }
        self.devices = roster.devices;
        self.active = roster.active.map(|(device, state)| ActiveDevice {
            track: state.track_id().map(str::to_owned),
            device,
            playing: state.transport.playing,
            order,
        });
        if let Some((report, current)) = external {
            self.here(cx, |playback, cx| {
                playback.set_external(true, cx);
                playback.sync_external(&report, current, cx);
            });
        } else if self.active.is_none() {
            self.here(cx, |playback, cx| playback.release_external(cx));
        }
        if refresh {
            self.playback.read(cx).refresh_device_queue();
        }
        if let Some((state, restart)) = adopted {
            self.adopt_report(state, restart, true, cx);
        } else if self.observing && !refresh {
            self.publish(cx);
        }
        if let Some((device, level)) = reported_volume {
            self.playback.update(cx, |playback, _| {
                playback.reported_steered_volume(&device, level)
            });
        }
        // playback taken for a local file is never reported, so the network never releases it,
        // and another device starting to play is what takes it back
        if started && !self.adopting() && self.claimed && self.sent.is_none() {
            self.claimed = false;
            self.here(cx, |playback, cx| playback.pause(cx));
        }
        self.tick(cx);
        self.redraw(cx);
    }

    /// Adopts the native queue once the account agrees that this device owns playback.
    pub(crate) fn device_queue(&mut self, mut state: Arc<Snapshot>, cx: &mut Context<Self>) {
        if !self.observing
            || self
                .active
                .as_ref()
                .is_some_and(|remote| !self.is_native(&remote.device))
        {
            return;
        }
        let playback = self.playback.read(cx);
        let Some(id) = state.track_id() else { return };
        let current = playback.track().and_then(|track| track.id.as_deref());
        if current.is_some_and(music::is_local_id) && !state.transport.playing {
            return;
        }
        let local = self.snapshot(cx);
        let taking_over = playback.is_external();
        let same_context = state.context.is_none()
            || local
                .as_ref()
                .is_some_and(|local| local.context == state.context);
        let locally_edited = local.is_some_and(|mut local| {
            self.limit_device_queue(&mut local, cx);
            self.sent
                .as_ref()
                .is_none_or(|sent| sent.queue != local.queue_fingerprint())
        });
        let order = queue_order(&state);
        let previous = self
            .active
            .as_mut()
            .filter(|remote| remote.track.as_deref() == Some(id))
            .and_then(|remote| remote.order.replace(order));
        if same_context {
            self.here(cx, |playback, cx| playback.follow_device_track(&state, cx));
        }
        let playback = self.playback.read(cx);
        let restart = taking_over
            || !same_context
            || playback.track().and_then(|track| track.id.as_deref()) != state.track_id();
        if !restart {
            let Some(local) = self.snapshot(cx) else {
                return;
            };
            if locally_edited || previous == Some(order) || queue_order(&local) == order {
                return;
            }
            Arc::make_mut(&mut state).track = local.track;
        }
        self.adopt_report(state, restart, false, cx);
    }

    /// Installs a device's queue, keeping the current playback when the report only edits it.
    fn adopt_report(
        &mut self,
        state: Arc<Snapshot>,
        restart: bool,
        external: bool,
        cx: &mut Context<Self>,
    ) {
        self.claimed = false;
        self.load_start(
            Start {
                state,
                preserve_queue: true,
            },
            restart,
            Some(external),
            cx,
        );
    }

    /// Tells the views, and everything that follows playback such as the lyrics, that what the
    /// other device plays has changed or moved on.
    fn redraw(&self, cx: &mut Context<Self>) {
        cx.notify();
        self.playback.update(cx, |_, cx| cx.notify());
    }

    /// Whether another device has playback that is moving on.
    fn moving(&self) -> bool {
        self.steered_device().is_some_and(|remote| remote.playing)
    }

    /// Redraws a remote device's advancing clock until playback stops or moves here. Only the
    /// observers of `Playback::ticks` hear each tick.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.moving() {
            self.ticking = None;
        } else if self.ticking.is_none() {
            self.ticking = Some(cx.spawn(async move |this, cx| {
                while this
                    .update(cx, |this, cx| {
                        let moving = this.moving();
                        if moving {
                            let ticks = this.playback.read(cx).ticks().clone();
                            ticks.update(cx, |_, cx| cx.notify());
                        } else {
                            this.ticking = None;
                        }
                        moving
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
            Command::Seek(at) => self.seek(at, cx),
            Command::Volume(level) => self.here(cx, |playback, cx| playback.set_volume(level, cx)),
            Command::Shuffle(on) => self.queue.update(cx, |queue, cx| queue.set_shuffle(on, cx)),
            Command::Repeat(mode) => self.here(cx, |playback, cx| playback.set_repeat(mode, cx)),
            Command::Enqueue { id, added_by } => self.enqueue(id, added_by, cx),
            Command::Start(start) => self.load_start(start, true, None, cx),
            Command::SetQueue { upcoming } => self.set_queue(upcoming, cx),
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
            async move {
                let track = client.track(&id).await?;
                let adder = match stranger {
                    Some(id) => person(client.as_ref(), id).await,
                    None => None,
                };
                Ok::<_, anyhow::Error>((track, adder))
            },
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
    /// The history stays as it is here, since an edit only ever changes what is still to play.
    fn set_queue(&mut self, upcoming: Vec<QueuedTrack>, cx: &mut Context<Self>) {
        let Some(state) = self.snapshot(cx) else {
            return;
        };
        // the edit was made on the last report, and one made on an older queue would undo what
        // changed here since, so it gives way to the report of what this queue holds now
        if !self
            .sent
            .as_ref()
            .is_some_and(|sent| sent.queue == state.queue_fingerprint())
        {
            log::debug!("connect: dropping a queue edit made on an older report");
            self.sent = None;
            self.publish(cx);
            return;
        }
        self.load_start(
            Start {
                state: Arc::new(Snapshot { upcoming, ..state }),
                preserve_queue: true,
            },
            false,
            None,
            cx,
        );
    }

    /// Resolves one Connect snapshot, either to start playback or to adopt the engine's playback.
    fn load_start(
        &mut self,
        start: Start,
        restart: bool,
        external: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let adopt = external.is_some();
        let native = external == Some(false);
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        let state = start.state.transport;
        let report = adopt.then_some(state);
        let reach = match restart {
            true => Reach::Handed,
            false => Reach::Reported {
                past: PREVIOUS,
                upcoming: UPCOMING,
            },
        };
        let revision = self.queue.read(cx).revision();
        let shown = (!restart)
            .then(|| {
                self.snapshot(cx)
                    .map(|snapshot| snapshot.queue_fingerprint())
            })
            .flatten();
        let queue = self.queue.read(cx);
        let known = queue
            .tracks()
            .filter_map(|track| Some((track.id.clone()?, track.clone())))
            .collect();
        let people = start
            .state
            .tracks()
            .filter_map(|track| track.added_by.as_ref())
            .filter_map(|id| self.known(id).map(|person| (id.clone(), person)))
            .collect();
        let task = self.io.load(
            load(client, start, known, people, adopt, restart),
            move |this, loaded, cx| {
                let reported = this.changing.take().and_then(|(_, report)| report);
                let moved = if restart {
                    this.queue.read(cx).revision() != revision
                } else {
                    shown.is_none_or(|shown| {
                        this.snapshot(cx)
                            .is_none_or(|now| now.queue_fingerprint() != shown)
                    })
                };
                if moved {
                    if adopt {
                        this.redraw(cx);
                    }
                    this.publish(cx);
                    return;
                }
                let (tracks, index, origin, window) = match loaded {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        log::warn!("connect: cannot apply playback change: {error:#}");
                        this.sent = adopt
                            .then(|| this.snapshot(cx).map(|state| state.published()))
                            .flatten();
                        if !adopt {
                            this.publish(cx);
                        }
                        cx.notify();
                        return;
                    }
                };
                let retained = if restart && native && !this.playback.read(cx).is_external() {
                    this.queue
                        .read(cx)
                        .upcoming()
                        .filter(|track| track.id.as_deref().is_some_and(music::is_local_id))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                if restart {
                    this.claimed = !adopt;
                    let queue = this.queue.clone();
                    let state = reported.unwrap_or(state);
                    this.here(cx, |playback, cx| {
                        let external = external.unwrap_or(false);
                        playback.set_external(external, cx);
                        if native {
                            playback.report_modes(state.shuffle, state.repeat, cx);
                        } else if !external {
                            if let Some(shuffle) = state.shuffle {
                                queue.update(cx, |queue, cx| queue.set_shuffle(shuffle, cx));
                            }
                            if let Some(repeat) = state.repeat {
                                playback.set_repeat(repeat, cx);
                            }
                        }
                        playback.start_at(
                            tracks,
                            index,
                            origin,
                            state.live_position(),
                            adopt.then_some(&state),
                            cx,
                        );
                        if !adopt && !state.playing {
                            playback.pause(cx);
                        }
                    });
                }
                if window.is_some() || !retained.is_empty() {
                    this.queue.update(cx, |queue, cx| {
                        if let Some((past, current, next)) = window {
                            queue.replace_remote(past, current, next, reach, cx);
                        }
                        if !retained.is_empty() {
                            queue.insert(retained, None, crate::QueuePlacement::Last, cx);
                        }
                    });
                }
                if adopt {
                    this.sent = native
                        .then(|| this.snapshot(cx))
                        .flatten()
                        .map(|mut state| {
                            this.limit_device_queue(&mut state, cx);
                            state.published()
                        });
                }
                cx.notify();
            },
            cx,
        );
        self.changing = Some((task, report));
    }

    /// Publishes one local snapshot through either the built-in player or Soloist's device.
    fn publish(&mut self, cx: &mut Context<Self>) {
        if self.changing.is_some() || !(self.shown || self.observing) {
            return;
        }
        let Some(link) = self.link.clone() else {
            return;
        };
        let Some(mut now) = self.now_playing(cx) else {
            if self.sent.take().is_some() && !self.observing {
                link.send(Request::Publish(None));
                cx.notify();
            }
            return;
        };
        let native = if self.observing {
            let Some(remote) = self
                .active
                .as_ref()
                .filter(|remote| self.is_native(&remote.device))
            else {
                self.sent = None;
                return;
            };
            if remote.track.as_deref() != now.track_id() {
                return;
            }
            self.limit_device_queue(&mut now, cx);
            Some(remote)
        } else {
            None
        };
        let published = now.published();
        if self.sent.as_ref().is_some_and(|sent| match native {
            Some(_) => sent.queue == published.queue,
            None => !published.changed(sent),
        }) {
            return;
        }
        self.sent = Some(published);
        match native {
            Some(remote) if Some(queue_order(&now)) != remote.order => {
                link.send(Request::PublishQueue {
                    device: remote.device.id.clone(),
                    state: Arc::new(now),
                })
            }
            Some(_) => {}
            None => {
                link.send(Request::Publish(Some(Arc::new(now))));
                cx.notify();
            }
        }
    }

    /// Stops the shared queue before the first playable local file.
    fn limit_device_queue(&self, state: &mut Snapshot, cx: &App) {
        // Soloist can advance itself, so stop its queue before the next local file.
        let contiguous = self
            .queue
            .read(cx)
            .upcoming()
            .filter(|track| track.playable)
            .take_while(|track| {
                track
                    .id
                    .as_deref()
                    .is_some_and(|id| !music::is_local_id(id))
            })
            .count();
        state.upcoming.truncate(contiguous);
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
            volume: playback.volume(),
            transport: Transport {
                playing: playback.wants_playing(),
                position: playback.live_position(),
                duration: track.duration,
                shuffle: Some(queue.shuffle()),
                repeat: Some(playback.repeat()),
                stamp: Some(SystemTime::now()),
            },
        })
    }

    /// Reports streaming playback only once the player and queue agree on the current track.
    fn now_playing(&mut self, cx: &App) -> Option<Snapshot> {
        let playback = self.playback.read(cx);
        let id = self.current_id(cx)?;
        if playback.is_external()
            || playback.track().and_then(|track| track.id.as_deref()) != Some(id)
        {
            return None;
        }
        let local = music::is_local_id(id);
        if !self.observing && !self.claimed && playback.wants_playing() {
            self.claimed = true;
            if local {
                self.pause_elsewhere();
            }
        }
        if local || !(self.observing || self.claimed) {
            return None;
        }
        self.snapshot(cx)
    }

    /// Pauses the other device that has playback while it plays. A report takes playback off it
    /// on its own, but a local file sends none and would otherwise play over it.
    fn pause_elsewhere(&self) {
        if let Some(remote) = self.active.as_ref().filter(|remote| remote.playing) {
            self.control(&remote.device.id, Command::Pause);
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
    people: HashMap<String, Arc<Contributor>>,
    adopt: bool,
    restart: bool,
) -> Result<(Vec<Track>, usize, Option<Origin>, Option<RemoteWindow>)> {
    let Start {
        state: start,
        preserve_queue,
    } = start;
    let context = start.context.as_ref().filter(|_| restart);
    let origin = context.map(|context| match context {
        Collection::Album(id) => Origin::album(id),
        Collection::Playlist(id) => Origin::playlist(id),
        Collection::Saved => Origin::saved(),
        Collection::Radio(id) => Origin::radio(id),
    });
    let tracks = match context.filter(|_| !adopt) {
        Some(Collection::Album(id)) => client.album_tracks(id).await?,
        Some(Collection::Playlist(id)) => client.playlist_tracks(id).await?,
        Some(Collection::Saved) => client.saved_tracks().await?,
        Some(Collection::Radio(id)) => {
            let (mut tracks, _) = client.track_radio(id, None).await.unwrap_or_default();
            tracks.retain(|track| track.playable && track.id.as_deref() != Some(id));
            tracks.insert(0, client.track(id).await?);
            tracks
        }
        None => Vec::new(),
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
        resolve_queue(&*client, &mut known, people, &start).await
    } else {
        Vec::new()
    };
    let mut rows = resolved.into_iter();
    let past: Vec<_> = rows.by_ref().take(start.previous.len()).flatten().collect();
    let current = start.track.as_ref().and_then(|_| rows.next().flatten());
    let upcoming: Vec<_> = rows.flatten().collect();
    let (tracks, index) = match (restart, index) {
        (false, _) => (Vec::new(), 0),
        (true, Some(index)) => (tracks, index),
        (true, None) => {
            anyhow::ensure!(
                current.is_some(),
                "cannot load the transferred current track"
            );
            let tracks = past
                .iter()
                .chain(&current)
                .chain(&upcoming)
                .map(|(track, _)| track.clone())
                .collect();
            (tracks, past.len())
        }
    };
    let kind = start.track.as_ref().map_or(QueueKind::Context, kind_of);
    let window = preserve_queue.then_some((past, kind, upcoming));

    Ok((tracks, index, origin, window))
}

/// Fetches each missing song once, concurrently, then reconstructs the requested order.
/// Repeated occurrences share metadata; an unavailable song remains an explicit gap.
async fn resolve_queue(
    client: &dyn MusicApi,
    known: &mut HashMap<String, Track>,
    mut people: HashMap<String, Arc<Contributor>>,
    snapshot: &Snapshot,
) -> Vec<Option<(Track, QueueKind)>> {
    let rows = snapshot.tracks();
    let mut requested = HashSet::new();
    let missing: Vec<_> = rows
        .clone()
        .map(|row| &row.id)
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
    let strangers: HashSet<_> = rows
        .clone()
        .filter_map(|row| row.added_by.as_ref())
        .filter(|id| !people.contains_key(*id))
        .cloned()
        .collect();
    let mut profiles = stream::iter(strangers)
        .map(|id| person(client, id))
        .buffer_unordered(QUEUE_READS);
    while let Some(profile) = profiles.next().await {
        people.extend(profile);
    }
    rows.map(|row| {
        let mut track = known.get(&row.id)?.clone();
        track.added_by = row.added_by.as_ref().and_then(|id| people.get(id)).cloned();
        Some((track, kind_of(row)))
    })
    .collect()
}

/// Why a track another device lists is in its play order: queued by hand, picked by the
/// provider to follow the collection, or part of the collection itself.
fn kind_of(row: &QueuedTrack) -> QueueKind {
    match (row.queued, row.suggested) {
        (true, _) => QueueKind::Queued,
        (false, true) => QueueKind::Suggested,
        (false, false) => QueueKind::Context,
    }
}

/// Fingerprints history and upcoming separately so a repeated track cannot hide a boundary change.
fn queue_order(state: &Snapshot) -> u64 {
    let mut hash = DefaultHasher::new();
    for rows in [&state.previous, &state.upcoming] {
        rows.len().hash(&mut hash);
        for row in rows {
            row.id.hash(&mut hash);
        }
    }
    hash.finish()
}
