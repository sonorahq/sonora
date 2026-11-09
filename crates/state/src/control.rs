use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cli::commands::{
    self, Item, Kind, Level, PlaybackCommand, ProviderCommand, QueueCommand, Request, SearchQuery,
    SessionCommand, Switch, Time, Watch,
};
use cli::wire::{self, Current, Failure, Reply, SessionInfo, State, Value};
use futures::future;
use music::link::Linked;
use music::{Album, MediaKind, MusicApi, MusicProvider, Playlist, SavedArtist, Track};
use tokio::sync::{mpsc, oneshot};

use crate::search::{self, Catalog};
use crate::{
    Change, Cx, Hit, Job, Library, Origin, PlaybackState, QueuePlacement, Repeat, Session,
    SessionState, Shelf, Store, Topic, Whence, join, noted,
};

/// What `sonora current` calls the provider of a local file, the slug `sonora providers` gives it.
const LOCAL: &str = "local";

/// How often a watch looks again on its own, for the position it reports and for a change made
/// somewhere that does not go through `Playback`, such as the volume of a steered device.
const WATCH_TICK: Duration = Duration::from_secs(1);

/// What the four searches of a query answered: songs, albums, playlists and artists.
type Found = (
    anyhow::Result<Vec<Track>>,
    anyhow::Result<Vec<Album>>,
    anyhow::Result<Vec<Playlist>>,
    anyhow::Result<Vec<SavedArtist>>,
);

/// Carries out what the command line asks of the running Sonora. Playback goes through
/// `Playback`, so a device steered over Connect is the one that obeys. A command that fetches
/// tracks first answers once they are queued, so a failure reaches the terminal. The host hands
/// it each request with [`Store::answer_control`] and [`Store::watch_control`].
pub struct Control {
    /// Queue commands and searches still fetching, each removing itself once it answers.
    pending: HashMap<u64, Job>,
    issued: u64,
    /// A provider switch waiting for the session to settle before it answers.
    switching: Option<Switching>,
    /// Command lines following what plays, as `sonora watch`.
    watchers: Vec<Watcher>,
    /// What keeps the watchers told on its own, held only while there are any. Playback's
    /// changes reach them through [`Store::react_control`].
    watching: Option<Job>,
}

/// A `sonora watch` and what it was last told.
struct Watcher {
    sink: mpsc::UnboundedSender<Reply>,
    /// Whether a new position alone is worth a line.
    position: bool,
    /// The state last sent, with the position left out unless `position`; `None` before the
    /// first line.
    told: Option<Option<Current>>,
}

/// A switch to the provider `slug`, answered once the session lands somewhere.
struct Switching {
    slug: String,
    name: &'static str,
    reply: oneshot::Sender<Reply>,
}

/// Where fetched tracks go: playing at once right after the current track, so the rest of
/// the queue follows them, or into the queue.
#[derive(Clone, Copy)]
enum Place {
    Now,
    Queue(QueuePlacement),
}

/// An item to queue, checked on the main thread and fetched on tokio.
enum Source {
    Path(PathBuf),
    /// The link as typed, for the failure that names it, and what it points at.
    Link(String, Linked),
}

/// The tracks one item stands for, and the collection they came from.
struct Batch {
    tracks: Vec<Track>,
    origin: Option<Origin>,
}

/// Why an item could not be fetched, with the network error behind it for `Network` to hear.
struct Missed {
    failure: Failure,
    error: Option<anyhow::Error>,
}

/// What a search hands back to the command that asked for it, on the UI thread.
type Then = Box<dyn FnOnce(&mut Store, Result<Vec<wire::Hit>, Failure>) + Send>;

impl Control {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            issued: 0,
            switching: None,
            watchers: Vec::new(),
            watching: None,
        }
    }

    /// Checks an item before anything is fetched: a link has to be a track, album, playlist or
    /// artist of the provider signed in now.
    fn source(&self, item: Item, session: &Session) -> Result<Source, Failure> {
        let link = match item {
            Item::Path(path) => return Ok(Source::Path(path)),
            Item::Link(link) => link,
            // `queue` finds a search before anything reaches here
            Item::Search { .. } => return Err(Failure::Malformed),
        };
        let Some((slug, linked)) = session.read_link(&link) else {
            return Err(Failure::Unqueueable(link));
        };
        let current = session.provider_slug().ok_or(Failure::SignedOut)?;
        if slug != current {
            let name = |slug: &'static str| session.account(slug).map_or(slug, |(name, _)| name);
            return Err(Failure::OtherProvider {
                link: name(slug).to_owned(),
                current: name(current).to_owned(),
            });
        }
        if session.client().is_none() {
            return Err(Failure::SignedOut);
        }
        match linked {
            Linked::Track(_) | Linked::Album(_) | Linked::Playlist(_) | Linked::Artist(_) => {
                Ok(Source::Link(link, linked))
            }
            Linked::User(_) | Linked::Jam(_) | Linked::Short => Err(Failure::Unqueueable(link)),
        }
    }

    /// The key the next fetch is kept under in `pending`.
    fn issue(&mut self) -> u64 {
        let key = self.issued;
        self.issued += 1;
        key
    }
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// Follows what plays for a `sonora watch`: what plays now at once, then again on every
    /// change, until the command line hangs up and `sink` closes.
    pub fn watch_control(&mut self, watch: Watch, sink: mpsc::UnboundedSender<Reply>) {
        let control = &mut self.control;
        control.watchers.push(Watcher {
            sink,
            position: watch.position,
            told: None,
        });
        if control.watching.is_none() {
            control.watching = Some(self.cx.spawn(|link| async move {
                loop {
                    tokio::time::sleep(WATCH_TICK).await;
                    link.post(|store| store.control_tell());
                }
            }));
        }
        self.control_tell();
    }

    /// Sends what plays to every watcher it changed for since its last line, and lets go of the
    /// ones that hung up. With none left, nothing follows playback any more.
    fn control_tell(&mut self) {
        let now = current(self);
        let control = &mut self.control;
        control.watchers.retain_mut(|watcher| {
            let seen = match watcher.position {
                true => now.clone(),
                false => now.clone().map(|current| Current {
                    position: 0,
                    ..current
                }),
            };
            if watcher.told.as_ref() == Some(&seen) {
                return !watcher.sink.is_closed();
            }
            watcher.told = Some(seen);
            watcher
                .sink
                .send(Reply::Value(Value::Current(now.clone())))
                .is_ok()
        });
        if control.watchers.is_empty() {
            control.watching = None;
        }
    }

    /// Carries out `request` and sends what to answer it into `reply`, at once or, for a
    /// command that fetches, once it is done.
    pub fn answer_control(&mut self, request: Request, reply: oneshot::Sender<Reply>) {
        let answer = match request {
            Request::Playback(command) => play(self, command),
            Request::Queue(command) => return self.control_queue(command, reply),
            Request::Provider(command) => return self.control_provider(command, reply),
            Request::Search(query) => return self.control_search(query, reply),
            Request::Session(command) => session(self, command),
            // a watch is answered as a stream, by `watch_control`, and never reaches here
            Request::Watch(_) => Reply::Failed(Failure::Malformed),
        };
        reply.send(answer).ok();
    }

    /// Answers a waiting provider switch once the session settles, and tells the watchers what
    /// plays whenever playback changes.
    pub(crate) fn react_control(&mut self, change: &Change) {
        match change {
            Change::Topic(Topic::Session) => self.control_settle(),
            Change::Topic(Topic::Playback) if self.control.watching.is_some() => {
                self.control_tell()
            }
            _ => {}
        }
    }

    /// Queues the items of `command`, or the best hit of the search it carries, which first has
    /// to be found.
    fn control_queue(&mut self, command: QueueCommand, reply: oneshot::Sender<Reply>) {
        let (items, placement) = match command {
            QueueCommand::Add(items) => (items, Place::Queue(QueuePlacement::End)),
            QueueCommand::PlayNext(items) => (items, Place::Queue(QueuePlacement::Next)),
            QueueCommand::Play(items) => (items, Place::Now),
        };
        if !items.iter().any(|item| matches!(item, Item::Search { .. })) {
            return self.control_enqueue(items, placement, reply);
        }
        // a search is queued alone, as the command line holds it to
        let Ok([Item::Search { text, kind }]) = <[Item; 1]>::try_from(items) else {
            reply.send(Reply::Failed(Failure::Malformed)).ok();
            return;
        };
        let query = SearchQuery {
            text: text.clone(),
            kind: Some(kind),
            limit: 1,
        };
        self.control_find(
            query,
            Box::new(move |store, found| {
                let failure = match found.map(|hits| hits.into_iter().next()) {
                    Ok(Some(hit)) => {
                        return store.control_enqueue(vec![Item::Link(hit.link)], placement, reply);
                    }
                    Ok(None) => Failure::NothingFound(text),
                    Err(failure) => failure,
                };
                reply.send(Reply::Failed(failure)).ok();
            }),
        );
    }

    /// Fetches every item at once and queues them together, in the order they were given. One
    /// that fails refuses the whole command, so nothing is queued half.
    fn control_enqueue(
        &mut self,
        items: Vec<Item>,
        placement: Place,
        reply: oneshot::Sender<Reply>,
    ) {
        let Store {
            control,
            session,
            cx,
            ..
        } = self;
        let sources = items
            .into_iter()
            .map(|item| control.source(item, session))
            .collect::<Result<Vec<_>, _>>();
        let sources = match sources {
            Ok(sources) => sources,
            Err(failure) => {
                reply.send(Reply::Failed(failure)).ok();
                return;
            }
        };

        let local = session.local_provider();
        let client = session.client();
        let key = control.issue();
        let io = cx.io.clone();
        let task = cx.spawn(|link| async move {
            let fetch = io.spawn(async move {
                let fetches = sources
                    .into_iter()
                    .map(|source| fetch(source, &local, client.as_ref()));
                anyhow::Ok(future::try_join_all(fetches).await)
            });
            let fetched = join(fetch).await;
            link.post(move |store| {
                let answer = match fetched {
                    Ok(Ok(batches)) => {
                        store.control_place(batches, placement);
                        Reply::Done
                    }
                    Ok(Err(missed)) => {
                        if let Some(error) = &missed.error {
                            noted(error, &mut store.cx);
                        }
                        Reply::Failed(missed.failure)
                    }
                    Err(error) => {
                        log::warn!("control: cannot queue: {error:#}");
                        Reply::Failed(Failure::Malformed)
                    }
                };
                reply.send(answer).ok();
                store.control.pending.remove(&key);
            });
        });
        control.pending.insert(key, task);
    }

    /// Searches the current provider and answers with the hits.
    fn control_search(&mut self, query: SearchQuery, reply: oneshot::Sender<Reply>) {
        self.control_find(
            query,
            Box::new(move |_, found| {
                let answer = match found {
                    Ok(hits) => Reply::Value(Value::Hits(hits)),
                    Err(failure) => Reply::Failed(failure),
                };
                reply.send(answer).ok();
            }),
        );
    }

    /// Searches the current provider the way the search page does, ranked against the library
    /// alike, and hands `then` the hits that have a link to act on.
    fn control_find(&mut self, query: SearchQuery, then: Then) {
        let Some(client) = self.session.client() else {
            return then(self, Err(Failure::SignedOut));
        };
        let fetch = {
            let client = client.clone();
            let asked = query.text.clone();
            async move {
                anyhow::Ok(
                    future::join4(
                        client.search(&asked),
                        client.search_albums(&asked),
                        client.search_playlists(&asked),
                        client.search_artists(&asked),
                    )
                    .await,
                )
            }
        };
        let key = self.control.issue();
        let io = self.cx.io.clone();
        let task = self.cx.spawn(|link| async move {
            let fetched = join(io.spawn(fetch)).await;
            link.post(move |store| {
                store.control.pending.remove(&key);
                let found = match fetched {
                    Ok(found) => hits(
                        &query,
                        found,
                        client.as_ref(),
                        &store.library,
                        &mut store.cx,
                    ),
                    Err(error) => {
                        log::warn!("control: cannot search: {error:#}");
                        Err(Failure::Unfetched(query.text.clone()))
                    }
                };
                then(store, found);
            });
        });
        self.control.pending.insert(key, task);
    }

    /// Lists the providers or tells the current one at once. A switch answers once the session
    /// has signed in, or failed to, so a script can go on to use the provider it asked for.
    fn control_provider(&mut self, command: ProviderCommand, reply: oneshot::Sender<Reply>) {
        let session = &self.session;
        let value = match command {
            ProviderCommand::List => Value::Providers(
                session
                    .providers()
                    .map(|info| wire::Provider {
                        slug: info.slug.to_owned(),
                        stored: info.stored,
                        current: info.active,
                    })
                    .collect(),
            ),
            ProviderCommand::Current(None) => {
                Value::Provider(session.provider_slug().map(str::to_owned))
            }
            ProviderCommand::Current(Some(slug)) => return self.control_switch(slug, reply),
        };
        reply.send(Reply::Value(value)).ok();
    }

    fn control_switch(&mut self, slug: String, reply: oneshot::Sender<Reply>) {
        let session = &self.session;
        let refusal = match session.account(&slug) {
            None => Some(Failure::UnknownProvider(slug.clone())),
            Some((name, false)) => Some(Failure::NoAccount(name.to_owned())),
            Some(_) if self.control.switching.is_some() || session.is_pending() => {
                Some(Failure::Busy)
            }
            Some(_) => None,
        };
        if let Some(failure) = refusal {
            reply.send(Reply::Failed(failure)).ok();
            return;
        }
        if settled_on(session, &slug) {
            reply.send(Reply::Done).ok();
            return;
        }
        let name = session.account(&slug).map_or("", |(name, _)| name);
        self.control.switching = Some(Switching {
            slug: slug.clone(),
            name,
            reply,
        });
        self.session.switch(&slug, &self.settings, &mut self.cx);
    }

    /// Answers a waiting switch once the session is no longer signing in. A provider that could
    /// not sign in hands over to another, so landing elsewhere is a failure.
    fn control_settle(&mut self) {
        let session = &self.session;
        if session.is_pending() {
            return;
        }
        let Some(switching) = self.control.switching.take() else {
            return;
        };
        let answer = match settled_on(session, &switching.slug) {
            true => Reply::Done,
            false => Reply::Failed(Failure::SignInFailed(switching.name.to_owned())),
        };
        switching.reply.send(answer).ok();
    }

    /// Plays or queues what was fetched in one go, so the items keep their order and one toast
    /// tells of them. A lone collection keeps its origin, several mixed ones have none.
    fn control_place(&mut self, batches: Vec<Batch>, placement: Place) {
        let (tracks, origin) = match <[Batch; 1]>::try_from(batches) {
            Ok([batch]) => (batch.tracks, batch.origin),
            Err(batches) => (
                batches.into_iter().flat_map(|batch| batch.tracks).collect(),
                None,
            ),
        };
        if tracks.is_empty() {
            return;
        }
        // with nothing playing the tracks start by themselves, so only a queue that already
        // plays is skipped forward into them
        let playing =
            self.playback.steered(&self.devices).is_some() || self.queue.current().is_some();
        match placement {
            Place::Now => {
                self.insert_tracks(tracks, 0, origin);
                if playing {
                    self.next_track();
                }
            }
            Place::Queue(placement) => self.enqueue_tracks(tracks, origin, placement),
        }
    }
}

/// Ranks what the provider found against the library and keeps `query.limit` hits of the
/// kind asked for. A part of the search that failed is left out, as the search page leaves
/// it, and only a search that failed whole is refused.
fn hits(
    query: &SearchQuery,
    (tracks, albums, playlists, artists): Found,
    client: &dyn MusicApi,
    library: &Library,
    cx: &mut Cx,
) -> Result<Vec<wire::Hit>, Failure> {
    if let (Err(error), Err(_), Err(_), Err(_)) = (&tracks, &albums, &playlists, &artists) {
        log::warn!("control: cannot search: {error:#}");
        noted(error, cx);
        return Err(Failure::Unfetched(query.text.clone()));
    }
    let catalog = Catalog {
        tracks: tracks.unwrap_or_default(),
        albums: albums.unwrap_or_default(),
        playlists: playlists.unwrap_or_default(),
        artists: artists.unwrap_or_default(),
    };
    let library = library.state(Shelf::Streaming);
    let ranked = search::rank(
        library.tracks(),
        library.albums(),
        library.playlists(),
        &catalog,
        &HashMap::new(),
        &query.text,
    );
    Ok(ranked
        .into_iter()
        .filter_map(|hit| wire_hit(hit, client))
        .filter(|hit| query.kind.is_none_or(|kind| hit.kind == kind))
        .take(query.limit.into())
        .collect())
}

/// Starts, joins or leaves a Sonora Session, or says which one Sonora is in. A start or join
/// answers before the server does, so `sonora session` tells whether it went through.
fn session(store: &mut Store, command: SessionCommand) -> Reply {
    match command {
        SessionCommand::Status => {
            let info = store.listening.room().map(|room| SessionInfo {
                code: room.code.clone(),
                provider: room.provider.clone(),
                hosting: room.hosting(),
                members: room
                    .host()
                    .into_iter()
                    .chain(room.members.iter().filter(|member| !member.host))
                    .map(|member| member.name.clone())
                    .collect(),
            });
            return Reply::Value(Value::Session(info));
        }
        SessionCommand::Start if !store.listening.available(&store.session) => {
            return Reply::Failed(Failure::SignedOut);
        }
        SessionCommand::Start => store.start_listening(),
        SessionCommand::Join(Some(code)) => store.join_listening(&code),
        SessionCommand::Join(None) => store.listening.show_join(true),
        SessionCommand::Leave => store.listening.leave(&mut store.cx),
    }
    Reply::Done
}

fn play(store: &mut Store, command: PlaybackCommand) -> Reply {
    match command {
        PlaybackCommand::Play => store.resume_playback(),
        PlaybackCommand::Pause => store.pause_playback(),
        PlaybackCommand::Toggle => store.toggle_play(),
        PlaybackCommand::Next => store.next_track(),
        PlaybackCommand::Previous => store.previous_track(),
        PlaybackCommand::Seek(percent) => store.seek_fraction(f32::from(percent) / 100.),
        PlaybackCommand::SeekTime(time) => {
            let target = moment(store, time);
            store.seek_to(target);
        }
        PlaybackCommand::CurrentTrack => {
            return Reply::Value(Value::Current(current(store)));
        }
        PlaybackCommand::Volume(None) => {
            return Reply::Value(Value::Volume(percent(shown_volume(store))));
        }
        PlaybackCommand::Volume(Some(level)) => {
            let target = match level {
                Level::To(level) => i16::from(level),
                Level::By(step) => i16::from(percent(shown_volume(store))) + i16::from(step),
            };
            store.set_volume(f32::from(target.clamp(0, 100)) / 100.);
        }
        PlaybackCommand::Shuffle(None) => {
            return Reply::Value(Value::Shuffle(shown_shuffle(store)));
        }
        PlaybackCommand::Shuffle(Some(switch)) => match switch {
            Switch::On => store.set_shuffle(true),
            Switch::Off => store.set_shuffle(false),
            Switch::Toggle => store.toggle_shuffle(),
        },
        PlaybackCommand::Repeat(None) => {
            return Reply::Value(Value::Repeat(wire_repeat(shown_repeat(store))));
        }
        PlaybackCommand::Repeat(Some(repeat)) => {
            let repeat = match repeat {
                commands::Repeat::Off => Repeat::Off,
                commands::Repeat::All => Repeat::All,
                commands::Repeat::One => Repeat::One,
            };
            store.set_repeat(repeat);
        }
    }
    Reply::Done
}

/// The volume playback shows, the steered device's while there is one.
fn shown_volume(store: &Store) -> f32 {
    store.playback.shown_volume(&store.devices)
}

/// Whether the shown queue shuffles, the steered device's while there is one.
fn shown_shuffle(store: &Store) -> bool {
    store.playback.shown_shuffle(&store.queue, &store.devices)
}

/// How the shown queue repeats, the steered device's while there is one.
fn shown_repeat(store: &Store) -> Repeat {
    store.playback.shown_repeat(&store.devices)
}

/// A search hit as the command line lists it, or `None` for one with no link to act on, as an
/// artist drawn from a track that names no id.
fn wire_hit(hit: Hit, client: &dyn MusicApi) -> Option<wire::Hit> {
    let (kind, media, id, title, by, duration) = match hit {
        Hit::Song(track) => (
            Kind::Song,
            MediaKind::Track,
            track.id?,
            track.name,
            track.artists,
            Some(track.duration.as_secs()),
        ),
        Hit::Album(album) => (
            Kind::Album,
            MediaKind::Album,
            album.id,
            album.name,
            album.artists,
            None,
        ),
        Hit::Artist(artist) => (
            Kind::Artist,
            MediaKind::Artist,
            artist.id?,
            artist.name,
            String::new(),
            None,
        ),
        Hit::Playlist(playlist) => (
            Kind::Playlist,
            MediaKind::Playlist,
            playlist.id,
            playlist.name,
            playlist.owner,
            None,
        ),
    };
    let link = client.share_url(media, &id)?;
    Some(wire::Hit {
        kind,
        title,
        by,
        duration,
        link,
    })
}

/// Whether the session is on `slug` and usable, offline counting since the account is kept.
fn settled_on(session: &Session, slug: &str) -> bool {
    session.provider_slug() == Some(slug)
        && matches!(
            session.state(),
            SessionState::SignedIn(_) | SessionState::Offline(_)
        )
}

/// Where a seek in time lands: the time asked for, or a step from where the track plays now,
/// held inside the track on the steered device when there is one.
fn moment(store: &Store, time: Time) -> Duration {
    let playback = &store.playback;
    let (now, length) = match playback.steered(&store.devices) {
        Some(steered) => (steered.state.live_position(), steered.state.duration),
        None => (
            playback.live_position(),
            playback
                .track()
                .map_or(Duration::ZERO, |track| track.duration),
        ),
    };
    let target = match time {
        Time::To(at) => Duration::from_secs(at),
        Time::By(step) => match step < 0 {
            true => now.saturating_sub(Duration::from_secs(step.unsigned_abs())),
            false => now.saturating_add(Duration::from_secs(step.unsigned_abs())),
        },
    };
    match length.is_zero() {
        true => target,
        false => target.min(length),
    }
}

/// What plays now, on the steered device when there is one, or `None` when nothing is loaded.
fn current(store: &Store) -> Option<Current> {
    let playback = &store.playback;
    let track = playback.shown_track(&store.devices)?;
    let (state, position) = match playback.steered(&store.devices) {
        Some(steered) => {
            let state = match steered.state.playing {
                true => State::Playing,
                false => State::Paused,
            };
            (state, steered.state.live_position())
        }
        None => {
            let state = match playback.state() {
                PlaybackState::Playing => State::Playing,
                PlaybackState::Paused => State::Paused,
                PlaybackState::Loading => State::Loading,
                PlaybackState::Idle | PlaybackState::Failed(_) => State::Idle,
            };
            (state, playback.live_position())
        }
    };
    let id = track.id.as_deref();
    let provider = match id.is_some_and(|id| Shelf::of(id).local()) {
        true => Some(LOCAL.to_owned()),
        false => store.session.provider_slug().map(str::to_owned),
    };
    Some(Current {
        state,
        track: wire_track(&track),
        position: position.as_secs(),
        volume: percent(shown_volume(store)),
        shuffle: shown_shuffle(store),
        repeat: wire_repeat(shown_repeat(store)),
        provider,
        liked: id.is_some_and(|id| store.library.saved(id)),
        origin: playback.origin(&store.queue).map(wire_origin),
    })
}

fn wire_origin(origin: &Origin) -> wire::Origin {
    let kind = match origin.whence {
        Whence::Album => wire::OriginKind::Album,
        Whence::Playlist => wire::OriginKind::Playlist,
        Whence::Artist => wire::OriginKind::Artist,
        Whence::Radio => wire::OriginKind::Radio,
        Whence::Saved => wire::OriginKind::Liked,
        Whence::Local => wire::OriginKind::Local,
    };
    wire::Origin {
        kind,
        id: origin.id.clone(),
        name: origin.name.as_ref().map(ToString::to_string),
    }
}

fn wire_track(track: &Track) -> wire::Track {
    let artists = match track.artist_refs.is_empty() {
        true => vec![track.artists.clone()],
        false => track
            .artist_refs
            .iter()
            .map(|artist| artist.name.clone())
            .collect(),
    };
    wire::Track {
        id: track.id.clone(),
        title: track.name.clone(),
        artists,
        album: track.album.clone(),
        cover: track.cover.clone(),
        duration: track.duration.as_secs(),
    }
}

fn wire_repeat(repeat: Repeat) -> commands::Repeat {
    match repeat {
        Repeat::Off => commands::Repeat::Off,
        Repeat::All => commands::Repeat::All,
        Repeat::One => commands::Repeat::One,
    }
}

/// A volume level of 0 to 1 as the whole percent the command line speaks.
fn percent(level: f32) -> u8 {
    (level.clamp(0., 1.) * 100.).round() as u8
}

/// The tracks a checked item stands for. A file is read by the local provider, a link
/// fetched from the provider it belongs to.
async fn fetch(
    source: Source,
    local: &Arc<dyn MusicProvider>,
    client: Option<&Arc<dyn MusicApi>>,
) -> Result<Batch, Missed> {
    let (link, linked) = match source {
        Source::Path(path) => {
            let track = local.track_from_path(&path).ok_or_else(|| Missed {
                failure: Failure::Unreadable(path.display().to_string()),
                error: None,
            })?;
            return Ok(Batch {
                tracks: vec![track],
                origin: None,
            });
        }
        Source::Link(link, linked) => (link, linked),
    };
    let Some(client) = client else {
        return Err(Missed {
            failure: Failure::SignedOut,
            error: None,
        });
    };
    let fetched = match linked {
        Linked::Track(id) => client.track(&id).await.map(|track| Batch {
            tracks: vec![track],
            origin: None,
        }),
        Linked::Album(id) => client.album_tracks(&id).await.map(|tracks| {
            let name = tracks.first().map(|track| track.album.clone());
            let origin = Origin::album(id);
            Batch {
                origin: Some(match name {
                    Some(name) => origin.named(name),
                    None => origin,
                }),
                tracks,
            }
        }),
        Linked::Playlist(id) => client.playlist_tracks(&id).await.map(|tracks| Batch {
            tracks,
            origin: Some(Origin::playlist(id)),
        }),
        Linked::Artist(id) => {
            let (profile, tracks) =
                tokio::join!(client.artist_profile(&id), client.artist_tracks(&id));
            tracks.map(|tracks| {
                let origin = Origin::artist(id.clone());
                Batch {
                    tracks,
                    origin: Some(match profile {
                        Ok(profile) => origin.named(profile.name),
                        Err(_) => origin,
                    }),
                }
            })
        }
        Linked::User(_) | Linked::Jam(_) | Linked::Short => {
            return Err(Missed {
                failure: Failure::Unqueueable(link),
                error: None,
            });
        }
    };
    fetched.map_err(|error| {
        log::warn!("control: cannot fetch {link}: {error:#}");
        Missed {
            failure: Failure::Unfetched(link),
            error: Some(error),
        }
    })
}
