use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use gpui::{App, Context, Entity, EventEmitter, SharedString, Task};
use music::{Album, AlbumDetail, Fraction, Playlist, PlaylistDetail, Track};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use storage::Database;

use crate::catalog::CatalogSource;
use crate::{
    Collection, Io, Network, Outcome, Playback, Reconnected, Session, SessionEvent, SessionState,
    Target, Toast, Toasts, join,
};

/// How many tracks download at once. Two keep the line busy without starving a track that is
/// playing.
const AT_ONCE: usize = 2;
/// How long the queue first rests after a provider refuses a download for asking too much. Each
/// refusal in a row doubles it, up to `LONGEST_REST`.
const FIRST_REST: Duration = Duration::from_secs(15);
const LONGEST_REST: Duration = Duration::from_secs(60);
/// How many downloads in a row have to work before the pause between them halves.
const EASE_AFTER: u32 = 5;
/// How often the progress of running downloads is redrawn.
/// How many finished downloads the pace is read from.
const PACE_WINDOW: usize = 12;
/// How long after its last download the pace still counts, since a rest in between would
/// otherwise promise a speed that is gone.
const PACE_FRESH: Duration = Duration::from_secs(5 * 60);
const TICK: Duration = Duration::from_millis(250);

/// Where one track stands on its way to being downloaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Asked for and waiting its turn, or waiting for the network.
    Waiting,
    Running,
    Done,
    /// The last try did not work. The next sign-in tries again.
    Failed,
}

/// What a download in the list of running ones is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Album,
    Playlist,
    Song,
}

/// An album, playlist or song that is downloading, as the list of running downloads shows it.
#[derive(Clone, Debug)]
pub struct Active {
    pub subject: Subject,
    pub id: String,
    pub cover: Option<String>,
    pub name: SharedString,
    /// How many of its tracks are in, out of how many can be downloaded.
    pub done: usize,
    pub total: usize,
    /// How much of it is in, from 0 to 1, counting the tracks that are part-way.
    pub share: f32,
    /// How long the rest of it should take at the pace of the latest downloads.
    pub eta: Option<Duration>,
    /// Tracks finished per minute lately, when enough have to say.
    pub per_minute: Option<f32>,
    /// The tracks running now, with how much of each has arrived when the provider can tell.
    pub current: Vec<(SharedString, Option<f32>)>,
}

/// Raised when what a Downloaded filter shows may have changed, which is when a track finished
/// or went, or an album or playlist was added or removed. Progress alone never raises it, so a
/// filtered list rebuilds on this rather than on every redraw of a running download.
pub struct DownloadsChanged;

/// An album or playlist that was downloaded whole, kept with its tracks so its page opens with
/// no network.
#[derive(Clone, Serialize, Deserialize)]
enum Head {
    Album(Album),
    Playlist(Playlist),
}

#[derive(Clone, Serialize, Deserialize)]
struct Set {
    head: Head,
    tracks: Vec<Track>,
}

impl Set {
    /// The ids of the tracks in the set that can be downloaded, which are the only ones its
    /// progress counts. The others are kept so the page still lists them.
    fn ids(&self) -> impl Iterator<Item = &str> + Clone {
        self.tracks
            .iter()
            .filter(|track| downloadable(track))
            .filter_map(|track| track.id.as_deref())
    }
}

/// A downloaded track, or one on its way.
struct Entry {
    /// Whether the listener asked for this track on its own rather than as part of a set, which
    /// keeps it when the set goes.
    single: bool,
    done: bool,
}

/// A toast that follows a download the listener asked for until none of its tracks is left to
/// wait for, then says how it went.
struct Watched {
    toast: usize,
    /// The tracks it counts. One taken off the downloads leaves the count.
    ids: Vec<String>,
    about: About,
}

/// What a download toast names.
#[derive(Clone)]
enum About {
    /// An album or a playlist, by name and the page it opens.
    Set { name: SharedString, target: Target },
    /// One song picked on its own.
    Song { name: SharedString, target: Target },
    /// Several songs picked one by one.
    Songs(usize),
}

impl About {
    fn of(head: &Head) -> Self {
        match head {
            Head::Album(album) => Self::Set {
                name: album.name.clone().into(),
                target: Target::Album(album.id.clone().into()),
            },
            Head::Playlist(playlist) => Self::Set {
                name: playlist.name.clone().into(),
                target: Target::Playlist(playlist.id.clone().into()),
            },
        }
    }

    /// The toast for where the download stands: `Loading` while it runs, `Done` once every
    /// track came in, and anything else once some could not.
    fn toast(&self, outcome: Outcome) -> Toast {
        let key = match (self, outcome) {
            (Self::Set { .. } | Self::Song { .. }, Outcome::Loading) => "toast-downloading",
            (Self::Set { .. } | Self::Song { .. }, Outcome::Done) => "toast-downloaded",
            (Self::Set { .. }, _) => "toast-download-set-failed",
            (Self::Song { .. }, _) => "toast-download-song-failed",
            (Self::Songs(_), Outcome::Loading) => "toast-downloading-songs",
            (Self::Songs(_), Outcome::Done) => "toast-downloaded-songs",
            (Self::Songs(_), _) => "toast-download-songs-failed",
        };
        // A failure goes on past the name, so only it wraps rather than lose its end.
        let toast = match outcome {
            Outcome::Loading | Outcome::Done => Toast::new(outcome, key).single_line(),
            _ => Toast::new(outcome, key),
        };
        match self {
            Self::Set { name, target } | Self::Song { name, target } => {
                toast.name(name.clone()).target(Some(target.clone()))
            }
            Self::Songs(count) => toast.count(*count),
        }
    }

    /// The toast that says the download was cancelled.
    fn cancelled(&self) -> Toast {
        match self {
            Self::Set { name, target } | Self::Song { name, target } => {
                Toast::new(Outcome::Plain, "toast-download-cancelled")
                    .single_line()
                    .name(name.clone())
                    .target(Some(target.clone()))
            }
            Self::Songs(count) => Toast::new(Outcome::Plain, "toast-downloads-cancelled")
                .single_line()
                .count(*count),
        }
    }
}

/// One download under way.
struct Running {
    /// Tells this run apart from an earlier one of the same track that was removed while it ran.
    serial: u64,
    progress: Fraction,
    task: Task<()>,
}

/// The tracks, albums and playlists the listener downloaded from the active provider, and the
/// line of tracks still to fetch.
///
/// What is downloaded is listed in `state.sqlite`, while the audio itself lives in the vault
/// that [`Playback`] shares with its engines. A download goes through the same engine a play
/// would, so it needs the provider signed in, and plays back with no network once it is done.
pub struct Downloads {
    session: Entity<Session>,
    playback: Entity<Playback>,
    store: Store,
    io: Io,
    /// The provider the lists below belong to.
    provider: Option<&'static str>,
    tracks: HashMap<String, Entry>,
    sets: HashMap<(Collection, String), Set>,
    /// The songs picked one by one this run, so the list of running downloads can name them.
    songs: HashMap<String, Track>,
    queue: VecDeque<String>,
    /// The downloads under way, each with how far it has got.
    running: HashMap<String, Running>,
    /// Downloads removed while they ran, by serial. An engine cannot stop a fetch it started, so
    /// each runs to its end and whatever it kept is removed then.
    abandoned: HashMap<u64, Task<()>>,
    /// The serial the next download gets.
    serial: u64,
    /// Redraws whoever shows downloads while any is under way, so their progress moves.
    ticker: Option<Task<()>>,
    /// When the next download may start, after a provider refused the last ones or asks for
    /// room between them.
    resting: Option<Instant>,
    /// How many downloads in a row the provider refused for asking too much, which sets how
    /// long the queue rests.
    refusals: u32,
    /// The pause between downloads a provider that refuses bursts is held to. It starts at
    /// nothing, opens up with each refusal and closes again over a run of downloads that work.
    gap: Duration,
    /// Downloads that worked since the gap last changed.
    streak: u32,
    /// When the latest downloads finished, for the pace the list of running ones reports.
    finished_at: VecDeque<Instant>,
    /// Starts the queue again once it has rested.
    wake: Option<Task<()>>,
    failed: HashSet<String>,
    /// Downloads that stopped because the connection went. They count as waiting and go back in
    /// line when the network returns.
    stalled: HashSet<String>,
    /// Fetches the tracks of an album or playlist asked for from a menu, which has none to hand.
    fetching: HashMap<String, (usize, Task<()>)>,
    /// The toasts following the downloads the listener asked for.
    watched: Vec<Watched>,
    restore: Option<Task<()>>,
}

impl EventEmitter<DownloadsChanged> for Downloads {}

impl Downloads {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        network: &Entity<Network>,
        database: Database,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        // The queue stops while the network is gone, and picks up again once it is back.
        cx.subscribe(network, |this, _, _: &Reconnected, cx| this.resume(cx))
            .detach();
        cx.subscribe(&session, |this, session, event, cx| match event {
            SessionEvent::SignedIn | SessionEvent::Reconnected => {
                let provider = session.read(cx).provider_slug();
                this.adopt(provider, cx);
                this.failed.clear();
                this.requeue();
                this.pump(cx);
            }
            SessionEvent::SignedOut => this.clear(cx),
            SessionEvent::WentOffline => this.halt(cx),
            SessionEvent::LocalChanged => {}
        })
        .detach();
        // A run that starts offline never signs in, but its downloads are still listed.
        cx.observe(&session, |this, session, cx| {
            let session = session.read(cx);
            if this.provider.is_none() && matches!(session.state(), SessionState::Offline(_)) {
                let provider = session.provider_slug();
                this.adopt(provider, cx);
            }
        })
        .detach();
        // The streaming engine starts a moment after sign-in, and the queue waits for it.
        cx.observe(&playback, |this, _, cx| this.pump(cx)).detach();
        Self {
            session,
            playback,
            store: Store { database },
            io,
            provider: None,
            tracks: HashMap::new(),
            sets: HashMap::new(),
            songs: HashMap::new(),
            queue: VecDeque::new(),
            running: HashMap::new(),
            abandoned: HashMap::new(),
            serial: 0,
            ticker: None,
            resting: None,
            refusals: 0,
            gap: Duration::ZERO,
            streak: 0,
            finished_at: VecDeque::new(),
            wake: None,
            failed: HashSet::new(),
            stalled: HashSet::new(),
            fetching: HashMap::new(),
            watched: Vec::new(),
            restore: None,
        }
    }

    /// Where a track stands, or `None` when it was never asked for.
    pub fn track(&self, id: &str) -> Option<Progress> {
        let entry = self.tracks.get(id)?;
        Some(match () {
            () if entry.done => Progress::Done,
            () if self.running.contains_key(id) => Progress::Running,
            () if self.failed.contains(id) => Progress::Failed,
            () => Progress::Waiting,
        })
    }

    /// How much of a track that is downloading has arrived, from 0 to 1, when the provider can
    /// tell.
    pub fn fraction(&self, id: &str) -> Option<f32> {
        self.running.get(id)?.progress.get()
    }

    /// Whether a track is downloaded and plays with no network.
    pub fn has(&self, id: &str) -> bool {
        self.tracks.get(id).is_some_and(|entry| entry.done)
    }

    /// Where a downloaded album or playlist stands, taken from the tracks in it that can be
    /// downloaded. `None` when it was never downloaded as a whole.
    pub fn set(&self, kind: Collection, id: &str) -> Option<Progress> {
        let set = self.sets.get(&(kind, id.to_owned()))?;
        let states: Vec<Progress> = set
            .ids()
            .map(|id| self.track(id).unwrap_or(Progress::Failed))
            .collect();
        Some(if states.iter().all(|state| *state == Progress::Done) {
            Progress::Done
        } else if states.contains(&Progress::Running) {
            Progress::Running
        } else if states.contains(&Progress::Waiting) {
            Progress::Waiting
        } else {
            Progress::Failed
        })
    }

    /// How many of a downloaded set's tracks are in, and how many of them can be downloaded.
    pub fn counts(&self, kind: Collection, id: &str) -> Option<(usize, usize)> {
        let set = self.sets.get(&(kind, id.to_owned()))?;
        let ids = set.ids();
        let done = ids.clone().filter(|id| self.has(id)).count();
        Some((done, ids.count()))
    }

    /// Tracks finished per minute over the latest downloads, or `None` while there are too few
    /// or the last one was long ago.
    fn pace(&self) -> Option<f32> {
        let (first, last) = (self.finished_at.front()?, self.finished_at.back()?);
        let span = last.duration_since(*first).as_secs_f32();
        if self.finished_at.len() < 3 || span <= 0. || last.elapsed() > PACE_FRESH {
            return None;
        }
        Some((self.finished_at.len() - 1) as f32 / span * 60.)
    }

    /// The albums, playlists and songs still downloading, by name, with how far each has come
    /// and the tracks of it running now.
    pub fn active(&self) -> Vec<Active> {
        let per_minute = self.pace();
        let mut active: Vec<Active> = self
            .sets
            .iter()
            .filter(|((kind, id), _)| {
                matches!(self.set(*kind, id), Some(Progress::Waiting | Progress::Running))
            })
            .map(|((kind, id), set)| {
                let (name, count, cover, subject) = match &set.head {
                    Head::Album(album) => (
                        album.name.clone(),
                        album.id.clone(),
                        album.cover.clone(),
                        Subject::Album,
                    ),
                    Head::Playlist(playlist) => (
                        playlist.name.clone(),
                        playlist.id.clone(),
                        playlist.cover.clone(),
                        Subject::Playlist,
                    ),
                };
                let (done, total) = self.counts(*kind, id).unwrap_or_default();
                Active {
                    subject,
                    id: count,
                    cover,
                    name: name.into(),
                    done,
                    total,
                    share: self.share(*kind, id),
                    eta: per_minute.map(|rate| {
                        Duration::from_secs_f32(total.saturating_sub(done) as f32 / rate * 60.)
                    }),
                    per_minute,
                    current: set
                        .tracks
                        .iter()
                        .filter(|track| {
                            track
                                .id
                                .as_deref()
                                .is_some_and(|id| self.running.contains_key(id))
                        })
                        .map(|track| {
                            let fraction = track.id.as_deref().and_then(|id| self.fraction(id));
                            (SharedString::from(track.name.clone()), fraction)
                        })
                        .collect(),
                }
            })
            .collect();
        active.extend(self.songs.iter().filter_map(|(id, track)| {
            let waiting = matches!(self.track(id)?, Progress::Waiting | Progress::Running);
            let arrived = self.fraction(id);
            waiting.then(|| Active {
                subject: Subject::Song,
                id: id.clone(),
                cover: track.cover.clone(),
                name: track.name.clone().into(),
                done: 0,
                total: 1,
                share: arrived.unwrap_or(0.),
                eta: per_minute.map(|rate| Duration::from_secs_f32(60. / rate)),
                per_minute,
                current: Vec::new(),
            })
        }));
        active.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        active
    }

    /// How much of a downloaded album or playlist is in, from 0 to 1, counting the part of each
    /// running track that has arrived.
    pub fn share(&self, kind: Collection, id: &str) -> f32 {
        let Some(set) = self.sets.get(&(kind, id.to_owned())) else {
            return 0.;
        };
        self.portion(set.ids())
    }

    /// How much of `ids` is in, from 0 to 1, counting the part of each running track that has
    /// arrived. Nothing at all reads as none.
    fn portion<'a>(&self, ids: impl Iterator<Item = &'a str>) -> f32 {
        let (arrived, count) = ids.fold((0., 0usize), |(arrived, count), id| {
            let part = match self.has(id) {
                true => 1.,
                false => self.fraction(id).unwrap_or(0.),
            };
            (arrived + part, count + 1)
        });
        match count {
            0 => 0.,
            count => arrived / count as f32,
        }
    }

    /// Whether anything at all is downloaded or on its way.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// How many tracks are downloaded or on their way.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// The ids of the artists credited on a downloaded album or playlist or on any of its tracks.
    pub fn credited(&self) -> impl Iterator<Item = &str> {
        self.sets
            .values()
            .flat_map(|set| {
                let head = match &set.head {
                    Head::Album(album) => album.artist_refs.as_slice(),
                    Head::Playlist(_) => &[],
                };
                head.iter()
                    .chain(set.tracks.iter().flat_map(|track| &track.artist_refs))
            })
            .filter_map(|artist| artist.id.as_deref())
    }

    /// The downloaded copy of an album page, for when the provider is out of reach.
    pub fn album_detail(&self, id: &str) -> Option<AlbumDetail> {
        let set = self.sets.get(&(Collection::Album, id.to_owned()))?;
        let Head::Album(album) = &set.head else {
            return None;
        };
        Some(AlbumDetail {
            album: album.clone(),
            tracks: set.tracks.clone(),
        })
    }

    /// The downloaded copy of a playlist page, for when the provider is out of reach.
    pub fn playlist_detail(&self, id: &str) -> Option<PlaylistDetail> {
        let set = self.sets.get(&(Collection::Playlist, id.to_owned()))?;
        let Head::Playlist(playlist) = &set.head else {
            return None;
        };
        Some(PlaylistDetail {
            playlist: playlist.clone(),
            tracks: set.tracks.clone(),
            continuation: None,
        })
    }

    /// Downloads tracks the listener picked one by one.
    pub fn download_tracks(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let Some(provider) = self.provider else {
            return;
        };
        let tracks: Vec<Track> = tracks.into_iter().filter(downloadable).collect();
        for track in &tracks {
            self.enlist(track, true);
            if let Some(id) = track.id.clone() {
                self.songs.insert(id, track.clone());
            }
        }
        let about = match tracks.as_slice() {
            [track] => track.id.as_ref().map(|id| About::Song {
                name: track.name.clone().into(),
                target: Target::Song(id.clone().into()),
            }),
            [] => None,
            tracks => Some(About::Songs(tracks.len())),
        };
        let ids: Vec<String> = tracks.iter().filter_map(|track| track.id.clone()).collect();
        self.persist(provider, None, tracks);
        self.pump(cx);
        if let Some(about) = about {
            self.watch(ids, about, None, cx);
        }
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// Downloads an album whose tracks are already loaded.
    pub fn download_album(&mut self, detail: AlbumDetail, cx: &mut Context<Self>) {
        let (id, set) = album_set(detail);
        self.download_set(Collection::Album, id, set, None, cx);
    }

    /// Downloads a playlist whose tracks are already loaded.
    pub fn download_playlist(&mut self, detail: PlaylistDetail, cx: &mut Context<Self>) {
        let (id, set) = playlist_set(detail);
        self.download_set(Collection::Playlist, id, set, None, cx);
    }

    /// Downloads an album named from a menu, fetching its tracks first. Its toast is up while
    /// they are fetched.
    pub fn download_album_by_id(&mut self, album: &Album, cx: &mut Context<Self>) {
        let Some(catalog) = self.session.read(cx).catalog(&album.id) else {
            return;
        };
        let id = album.id.clone();
        let about = About::Set {
            name: album.name.clone().into(),
            target: Target::Album(id.clone().into()),
        };
        let toast = Toasts::post(running(&about), cx);
        let request = self.io.spawn({
            let id = id.clone();
            async move { catalog.album(&id).await }
        });
        let task = cx.spawn(async move |this, cx| {
            let loaded = join(request).await;
            this.update(cx, |this, cx| {
                this.fetching.remove(&id);
                match loaded {
                    Ok(detail) => {
                        let (id, set) = album_set((*detail).clone());
                        this.download_set(Collection::Album, id, set, Some(toast), cx);
                    }
                    Err(error) => this.failed_to_fetch(&error, Some(toast), cx),
                }
            })
            .ok();
        });
        self.fetching.insert(album.id.clone(), (toast, task));
    }

    /// Downloads a playlist named from a menu, fetching all of its tracks first. Its toast is up
    /// while they are fetched.
    pub fn download_playlist_by_id(&mut self, playlist: &Playlist, cx: &mut Context<Self>) {
        let Some(catalog) = self.session.read(cx).catalog(&playlist.id) else {
            return;
        };
        let id = playlist.id.clone();
        let about = About::Set {
            name: playlist.name.clone().into(),
            target: Target::Playlist(id.clone().into()),
        };
        let toast = Toasts::post(running(&about), cx);
        let request = self.io.spawn({
            let id = id.clone();
            async move { whole_playlist(&catalog, &id).await }
        });
        let task = cx.spawn(async move |this, cx| {
            let loaded = join(request).await;
            this.update(cx, |this, cx| {
                this.fetching.remove(&id);
                match loaded {
                    Ok(detail) => {
                        let (id, set) = playlist_set(detail);
                        this.download_set(Collection::Playlist, id, set, Some(toast), cx);
                    }
                    Err(error) => this.failed_to_fetch(&error, Some(toast), cx),
                }
            })
            .ok();
        });
        self.fetching.insert(playlist.id.clone(), (toast, task));
    }

    /// Removes the downloads of tracks, and takes them out of any downloaded album or playlist
    /// that held them. A set left with no tracks stops being downloaded.
    pub fn remove_tracks(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let Some(provider) = self.provider else {
            return;
        };
        let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut kept = Vec::new();
        let mut emptied = Vec::new();
        for ((kind, id), set) in &mut self.sets {
            let before = set.tracks.len();
            set.tracks
                .retain(|track| !track.id.as_deref().is_some_and(|id| gone.contains(id)));
            match (set.tracks.len() == before, set.tracks.is_empty()) {
                (true, _) => {}
                (false, true) => emptied.push((*kind, id.clone())),
                (false, false) => kept.push((*kind, id.clone(), set.clone())),
            }
        }
        for key in &emptied {
            self.sets.remove(key);
        }
        self.drop_tracks(provider, ids, cx);
        self.write(move |store| {
            for (kind, id, set) in &kept {
                store.keep_set(provider, *kind, id, set)?;
            }
            for (kind, id) in &emptied {
                store.forget_set(provider, *kind, id)?;
            }
            Ok(())
        });
        self.report(cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// Removes a downloaded album or playlist, and every track of it nothing else keeps.
    pub fn remove_set(&mut self, kind: Collection, id: &str, cx: &mut Context<Self>) {
        let Some(provider) = self.provider else {
            return;
        };
        let key = (kind, id.to_owned());
        let Some(set) = self.sets.remove(&key) else {
            return;
        };
        let held = self.held_by_sets();
        let gone: Vec<String> = set
            .tracks
            .iter()
            .filter_map(|track| track.id.clone())
            .filter(|id| !held.contains(id))
            .filter(|id| !self.tracks.get(id).is_some_and(|entry| entry.single))
            .collect();
        self.drop_tracks(provider, &gone, cx);
        let id = id.to_owned();
        self.write(move |store| store.forget_set(provider, kind, &id));
        self.report(cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// Removes every download of every provider, files and lists alike. The ones under way are
    /// removed again as they end.
    pub fn remove_all(&mut self, cx: &mut Context<Self>) {
        self.tracks.clear();
        self.sets.clear();
        self.queue.clear();
        for (_, run) in self.running.drain() {
            self.abandoned.insert(run.serial, run.task);
        }
        self.failed.clear();
        self.stalled.clear();
        self.close_fetching(cx);
        self.playback
            .update(cx, |playback, cx| playback.remove_downloads(cx));
        self.write(|store| store.forget_everything());
        self.report(cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// Downloads an album or playlist. `toast` is the one already up while its tracks were
    /// fetched, which goes on to follow the download, or else a new one is put up.
    fn download_set(
        &mut self,
        kind: Collection,
        id: String,
        set: Set,
        toast: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.provider else {
            return;
        };
        let tracks: Vec<Track> = set
            .tracks
            .iter()
            .filter(|track| downloadable(track))
            .cloned()
            .collect();
        for track in &tracks {
            self.enlist(track, false);
        }
        let ids = tracks.iter().filter_map(|track| track.id.clone()).collect();
        let about = About::of(&set.head);
        self.sets.insert((kind, id.clone()), set.clone());
        self.persist(provider, Some((kind, id, set)), tracks);
        self.pump(cx);
        self.watch(ids, about, toast, cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// Puts a track on the list and in the line, unless it is there already.
    fn enlist(&mut self, track: &Track, single: bool) {
        let Some(id) = track.id.clone() else {
            return;
        };
        match self.tracks.get_mut(&id) {
            Some(entry) => {
                entry.single |= single;
                if !entry.done && self.failed.remove(&id) {
                    self.queue.push_back(id);
                }
            }
            None => {
                self.tracks.insert(
                    id.clone(),
                    Entry {
                        single,
                        done: false,
                    },
                );
                self.queue.push_back(id);
            }
        }
    }

    /// Writes new entries and, for a whole album or playlist, the set itself.
    fn persist(
        &mut self,
        provider: &'static str,
        set: Option<(Collection, String, Set)>,
        tracks: Vec<Track>,
    ) {
        let rows: Vec<(String, bool)> = tracks
            .into_iter()
            .filter_map(|track| {
                let id = track.id?;
                let entry = self.tracks.get(&id)?;
                Some((id, entry.single))
            })
            .collect();
        self.write(move |store| {
            if let Some((kind, id, set)) = &set {
                store.keep_set(provider, *kind, id, set)?;
            }
            store.enlist(provider, &rows)
        });
    }

    /// Starts the next downloads in line, as many as may run at once, while there is an engine
    /// to fetch through and a network to fetch from.
    fn pump(&mut self, cx: &mut Context<Self>) {
        if Network::lost(cx) || self.provider.is_none() {
            return;
        }
        if let Some(until) = self.resting.filter(|until| *until > Instant::now()) {
            return self.wake_at(until, cx);
        }
        // A provider that refuses bursts gets its downloads one at a time.
        let bursts = self.session.read(cx).capabilities().download_gap;
        let at_once = match bursts.is_zero() {
            true => AT_ONCE,
            false => 1,
        };
        while self.running.len() < at_once {
            let Some(id) = self.queue.pop_front() else {
                return;
            };
            if self.tracks.get(&id).is_none_or(|entry| entry.done) {
                continue;
            }
            let progress = Fraction::default();
            let Some(download) = self.playback.read(cx).download(&id, progress.clone()) else {
                self.queue.push_front(id);
                return;
            };
            let request = self.io.spawn(download);
            let serial = self.serial;
            self.serial += 1;
            let task = cx.spawn({
                let id = id.clone();
                async move |this, cx| {
                    let fetched = join(request).await;
                    this.update(cx, |this, cx| this.finished(id, serial, fetched, cx))
                        .ok();
                }
            });
            self.running.insert(
                id,
                Running {
                    serial,
                    progress,
                    task,
                },
            );
            if !self.gap.is_zero() {
                self.resting = Some(Instant::now() + self.gap);
            }
            self.tick(cx);
            cx.notify();
        }
    }

    /// Keeps the views that show progress redrawing while a download runs, and stops once none
    /// does.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this
                    .update(cx, |this, cx| {
                        this.report(cx);
                        cx.notify();
                        let going = !this.running.is_empty();
                        if !going {
                            this.ticker = None;
                        }
                        going
                    })
                    .unwrap_or(false);
                if !going {
                    return;
                }
            }
        }));
    }

    /// Settles download `serial` of track `id`. One that was removed while it ran has its file
    /// removed again, unless the track was asked for anew in the meantime.
    fn finished(&mut self, id: String, serial: u64, fetched: Result<()>, cx: &mut Context<Self>) {
        if self.abandoned.remove(&serial).is_some() {
            if fetched.is_ok() && !self.tracks.contains_key(&id) {
                self.playback.read(cx).discard(&id);
            }
            return;
        }
        if self.running.get(&id).is_none_or(|run| run.serial != serial) {
            return;
        }
        self.running.remove(&id);
        match fetched {
            Ok(()) => {
                self.refusals = 0;
                self.finished_at.push_back(Instant::now());
                if self.finished_at.len() > PACE_WINDOW {
                    self.finished_at.pop_front();
                }
                self.ease(cx);
                if let Some(entry) = self.tracks.get_mut(&id)
                    && let Some(provider) = self.provider
                {
                    entry.done = true;
                    let done = id.clone();
                    self.write(move |store| store.done(provider, &done));
                    cx.emit(DownloadsChanged);
                }
            }
            Err(error) if error.downcast_ref::<music::Throttled>().is_some() => {
                self.refusals = self.refusals.saturating_add(1);
                let wait = rest(self.refusals);
                let bursts = self.session.read(cx).capabilities().download_gap;
                // The pause stays at the provider's own, since the long rest is what lets its
                // limit clear. Widening the pause as well left a quick burst crawling for good.
                self.gap = bursts;
                self.streak = 0;
                log::warn!(
                    "downloads: the provider is refusing {id} for now, resting {}s, then {}s apart",
                    wait.as_secs(),
                    self.gap.as_secs()
                );
                self.resting = Some(Instant::now() + wait);
                // The track goes to the back of the line, so one the provider keeps refusing
                // cannot hold up every other download behind it.
                if self.tracks.contains_key(&id) {
                    self.queue.push_back(id);
                }
            }
            Err(error) => {
                let reason = format!("{error:#}");
                log::warn!("downloads: cannot download {id}: {reason}");
                crate::noted(&error, cx);
                if self.tracks.contains_key(&id) {
                    match music::trouble::offline(&reason) {
                        true => self.stalled.insert(id),
                        false => self.failed.insert(id),
                    };
                }
            }
        }
        self.report(cx);
        self.pump(cx);
        cx.notify();
    }

    /// Narrows the pause between downloads after a run of them worked, down to none once it is
    /// below a quarter of the one the provider starts from.
    fn ease(&mut self, cx: &mut Context<Self>) {
        if self.gap.is_zero() {
            return;
        }
        self.streak += 1;
        if self.streak < EASE_AFTER {
            return;
        }
        self.streak = 0;
        let bursts = self.session.read(cx).capabilities().download_gap;
        self.gap /= 2;
        if self.gap < bursts / 4 {
            self.gap = Duration::ZERO;
        }
        log::debug!("downloads: now {}s apart", self.gap.as_secs_f32());
    }

    /// Starts the queue again at `at`, unless something already will.
    fn wake_at(&mut self, at: Instant, cx: &mut Context<Self>) {
        if self.wake.is_some() {
            return;
        }
        self.wake = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(at.saturating_duration_since(Instant::now()))
                .await;
            this.update(cx, |this, cx| {
                this.wake = None;
                this.pump(cx);
            })
            .ok();
        }));
    }

    /// Says the tracks of an album or playlist could not be listed, on `toast` when one is up.
    fn failed_to_fetch(
        &mut self,
        error: &anyhow::Error,
        toast: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        log::warn!("downloads: cannot list the tracks to download: {error:#}");
        crate::noted(error, cx);
        let failed = Toast::new(Outcome::Failed, "toast-download-failed");
        match toast {
            Some(toast) => Toasts::settle(toast, failed, cx),
            None => {
                Toasts::post(failed, cx);
            }
        }
    }

    /// Puts up a toast that follows the download of `ids`, or has `toast` follow it when one is
    /// already up, and brings it up to date at once.
    fn watch(
        &mut self,
        ids: Vec<String>,
        about: About,
        toast: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let toast = match toast {
            Some(toast) => toast,
            None => Toasts::post(running(&about), cx),
        };
        self.watched.push(Watched { toast, ids, about });
        self.report(cx);
    }

    /// Moves every download toast on to where its tracks stand. One with none left to wait for
    /// says whether all of them came in, and one whose tracks were all removed goes.
    fn report(&mut self, cx: &mut Context<Self>) {
        let mut watched = std::mem::take(&mut self.watched);
        watched.retain_mut(|watch| {
            watch.ids.retain(|id| self.tracks.contains_key(id));
            if watch.ids.is_empty() {
                Toasts::close(watch.toast, cx);
                return false;
            }
            let ids = watch.ids.iter().map(String::as_str);
            let pending = ids
                .clone()
                .any(|id| matches!(self.track(id), Some(Progress::Waiting | Progress::Running)));
            if pending {
                Toasts::progress(watch.toast, self.portion(ids), cx);
                return true;
            }
            let outcome = match ids.clone().all(|id| self.has(id)) {
                true => Outcome::Done,
                false => Outcome::Failed,
            };
            Toasts::settle(watch.toast, watch.about.toast(outcome), cx);
            false
        });
        self.watched = watched;
    }

    /// Cancels the download of a song the listener stopped from the list of running downloads.
    pub fn cancel_song(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(track) = self.songs.remove(id) else {
            return;
        };
        self.remove_tracks(&[id.to_owned()], cx);
        Toasts::post(
            About::Song {
                name: track.name.into(),
                target: Target::Song(id.to_owned().into()),
            }
            .cancelled(),
            cx,
        );
    }

    /// Cancels the download of an album or playlist the listener stopped from the list of running
    /// downloads, removing its tracks, and puts up a toast that says so.
    pub fn cancel_set(&mut self, kind: Collection, id: &str, cx: &mut Context<Self>) {
        let Some(set) = self.sets.get(&(kind, id.to_owned())) else {
            return;
        };
        let about = About::of(&set.head);
        if let Some((toast, _)) = self.fetching.remove(id) {
            Toasts::close(toast, cx);
        }
        self.remove_set(kind, id, cx);
        Toasts::post(about.cancelled(), cx);
    }

    /// Takes down the toasts of albums and playlists whose tracks are still being listed, as their
    /// fetches are dropped.
    fn close_fetching(&mut self, cx: &mut Context<Self>) {
        for (_, (toast, _)) in self.fetching.drain() {
            Toasts::close(toast, cx);
        }
    }

    /// Takes tracks off the list and removes their audio. A download of one still under way is
    /// left to end, and its file is removed when it does. Its place in line goes to the next
    /// track at once.
    fn drop_tracks(&mut self, provider: &'static str, ids: &[String], cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let playback = self.playback.read(cx);
        for id in ids {
            self.tracks.remove(id);
            if let Some(run) = self.running.remove(id) {
                self.abandoned.insert(run.serial, run.task);
            }
            self.failed.remove(id);
            self.stalled.remove(id);
            playback.discard(id);
        }
        self.queue.retain(|id| !ids.contains(id));
        let ids = ids.to_vec();
        self.write(move |store| store.forget(provider, &ids));
        self.pump(cx);
    }

    /// Every track a downloaded set holds.
    fn held_by_sets(&self) -> HashSet<String> {
        self.sets
            .values()
            .flat_map(|set| set.tracks.iter().filter_map(|track| track.id.clone()))
            .collect()
    }

    /// Puts the downloads the lost connection stopped back at the front of the line, and starts
    /// it again.
    fn resume(&mut self, cx: &mut Context<Self>) {
        for id in self.stalled.drain() {
            self.queue.push_front(id);
        }
        self.pump(cx);
        cx.notify();
    }

    /// Stops the downloads under way and puts them back in the queue, which waits for the account
    /// to sign in again.
    fn halt(&mut self, cx: &mut Context<Self>) {
        self.running.clear();
        self.ticker = None;
        self.requeue();
        self.report(cx);
        cx.notify();
    }

    /// Puts every track not yet in back in line, as after a sign-in.
    fn requeue(&mut self) {
        self.stalled.clear();
        self.queue = self
            .tracks
            .iter()
            .filter(|(id, entry)| !entry.done && !self.running.contains_key(*id))
            .map(|(id, _)| id.clone())
            .collect();
    }

    /// Reads the lists of `provider` from disk, unless they are the ones already up.
    fn adopt(&mut self, provider: Option<&'static str>, cx: &mut Context<Self>) {
        if self.provider == provider {
            return;
        }
        self.clear(cx);
        let Some(provider) = provider else {
            return;
        };
        self.provider = Some(provider);
        let store = self.store.clone();
        let request = self.io.spawn_blocking(move || store.restore(provider));
        self.restore = Some(cx.spawn(async move |this, cx| {
            let restored = join(request).await;
            this.update(cx, |this, cx| {
                if this.provider != Some(provider) {
                    return;
                }
                match restored {
                    Ok(Restored { tracks, sets }) => {
                        for (id, single, done) in tracks {
                            if !done && !this.queue.contains(&id) {
                                this.queue.push_back(id.clone());
                            }
                            this.tracks.entry(id).or_insert(Entry { single, done });
                        }
                        for (kind, id, set) in sets {
                            this.sets.entry((kind, id)).or_insert(set);
                        }
                    }
                    Err(error) => log::warn!("downloads: cannot read the list: {error:#}"),
                }
                this.pump(cx);
                cx.emit(DownloadsChanged);
                cx.notify();
            })
            .ok();
        }));
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.provider = None;
        self.tracks.clear();
        self.sets.clear();
        self.queue.clear();
        self.running.clear();
        self.abandoned.clear();
        self.ticker = None;
        self.resting = None;
        self.refusals = 0;
        self.gap = Duration::ZERO;
        self.streak = 0;
        self.wake = None;
        self.failed.clear();
        self.stalled.clear();
        self.close_fetching(cx);
        self.restore = None;
        self.report(cx);
        cx.emit(DownloadsChanged);
        cx.notify();
    }

    /// Runs one write on a blocking thread and logs what went wrong. The lists in memory are
    /// already right, so nothing waits on it.
    fn write(&self, edit: impl FnOnce(&Store) -> Result<()> + Send + 'static) {
        let store = self.store.clone();
        self.io.spawn_blocking(move || {
            if let Err(error) = edit(&store) {
                log::warn!("downloads: cannot save the list: {error:#}");
            }
        });
    }
}

/// Whether a track is downloaded, for code with no handle on the [`Downloads`] entity.
pub fn downloaded(id: &str, cx: &App) -> bool {
    cx.try_global::<crate::Sonora>()
        .is_some_and(|sonora| sonora.downloads.read(cx).has(id))
}

/// Whether a track can be downloaded at all. It has to play, have an id and come from the
/// streaming provider rather than from disk.
fn downloadable(track: &Track) -> bool {
    track.playable
        && track
            .id
            .as_deref()
            .is_some_and(|id| !music::is_local_id(id))
}

/// A playlist with every page of its tracks, since a download of part of one is no use.
async fn whole_playlist(catalog: &Arc<CatalogSource>, id: &str) -> Result<PlaylistDetail> {
    let mut detail = (*catalog.playlist(id).await?).clone();
    while let Some(next) = detail.continuation.take() {
        let (tracks, more) = catalog.playlist_continuation(&next).await?;
        detail.tracks.extend(tracks);
        detail.continuation = more;
    }
    Ok(detail)
}

/// What was downloaded from one provider, as read back from disk.
struct Restored {
    tracks: Vec<(String, bool, bool)>,
    sets: Vec<(Collection, String, Set)>,
}

#[derive(Clone)]
struct Store {
    database: Database,
}

impl Store {
    fn open(&self) -> Result<Connection> {
        self.database
            .open()
            .context("cannot open the downloads list")
    }

    fn restore(&self, provider: &str) -> Result<Restored> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT track_id, single, done FROM downloads WHERE provider = ? ORDER BY added_at",
        )?;
        let tracks = statement
            .query_map(params![provider], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })?
            .filter_map(Result::ok)
            .collect();
        let mut statement = connection.prepare(
            "SELECT kind, set_id, value FROM downloaded_sets WHERE provider = ? ORDER BY added_at",
        )?;
        let sets = statement
            .query_map(params![provider], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(|row| {
                let (kind, id, value) = row.ok()?;
                let kind = match kind.as_str() {
                    "album" => Collection::Album,
                    "playlist" => Collection::Playlist,
                    _ => return None,
                };
                let set = serde_json::from_str::<Set>(&value).ok()?;
                Some((kind, id, set))
            })
            .collect();
        Ok(Restored { tracks, sets })
    }

    fn enlist(&self, provider: &str, rows: &[(String, bool)]) -> Result<()> {
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        let now = now();
        for (id, single) in rows {
            transaction.execute(
                "INSERT INTO downloads (provider, track_id, single, done, added_at)
                 VALUES (?1, ?2, ?3, 0, ?4)
                 ON CONFLICT (provider, track_id)
                 DO UPDATE SET single = single OR excluded.single",
                params![provider, id, single, now],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn done(&self, provider: &str, id: &str) -> Result<()> {
        self.open()?.execute(
            "UPDATE downloads SET done = 1 WHERE provider = ? AND track_id = ?",
            params![provider, id],
        )?;
        Ok(())
    }

    fn forget(&self, provider: &str, ids: &[String]) -> Result<()> {
        let connection = self.open()?;
        for id in ids {
            connection.execute(
                "DELETE FROM downloads WHERE provider = ? AND track_id = ?",
                params![provider, id],
            )?;
        }
        Ok(())
    }

    fn forget_everything(&self) -> Result<()> {
        self.open()?
            .execute_batch("DELETE FROM downloads; DELETE FROM downloaded_sets;")?;
        Ok(())
    }

    fn keep_set(&self, provider: &str, kind: Collection, id: &str, set: &Set) -> Result<()> {
        let value = serde_json::to_string(set)?;
        self.open()?.execute(
            "INSERT OR REPLACE INTO downloaded_sets (provider, kind, set_id, value, added_at)
             VALUES (?, ?, ?, ?, ?)",
            params![provider, kind_name(kind), id, value, now()],
        )?;
        Ok(())
    }

    fn forget_set(&self, provider: &str, kind: Collection, id: &str) -> Result<()> {
        self.open()?.execute(
            "DELETE FROM downloaded_sets WHERE provider = ? AND kind = ? AND set_id = ?",
            params![provider, kind_name(kind), id],
        )?;
        Ok(())
    }
}

fn kind_name(kind: Collection) -> &'static str {
    match kind {
        Collection::Album => "album",
        Collection::Playlist => "playlist",
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// How long the queue rests after `refusals` refusals in a row.
fn rest(refusals: u32) -> Duration {
    FIRST_REST
        .saturating_mul(1 << refusals.saturating_sub(1).min(8))
        .min(LONGEST_REST)
}

/// The toast a download shows while it runs. Dismissing it only hides it; the download goes on
/// and is cancelled from the list of running downloads.
fn running(about: &About) -> Toast {
    about.toast(Outcome::Loading).progress(0.)
}

fn album_set(detail: AlbumDetail) -> (String, Set) {
    let id = detail.album.id.clone();
    let set = Set {
        head: Head::Album(detail.album),
        tracks: detail.tracks,
    };
    (id, set)
}

fn playlist_set(detail: PlaylistDetail) -> (String, Set) {
    let id = detail.playlist.id.clone();
    let set = Set {
        head: Head::Playlist(detail.playlist),
        tracks: detail.tracks,
    };
    (id, set)
}
