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
use gpui::{App, Context, Entity, Subscription, Task};
use music::link::Linked;
use music::{Album, MediaKind, MusicApi, MusicProvider, Playlist, Track};
use tokio::sync::{mpsc, oneshot};

use crate::search::{self, Catalog};
use crate::{
    Hit, Io, Library, Origin, Playback, PlaybackState, QueuePlacement, Repeat, Session,
    SessionState, Shelf, Sonora, Whence, join, noted,
};

/// What `sonora current` calls the provider of a local file, the slug `sonora providers` gives it.
const LOCAL: &str = "local";

/// How often a watch looks again on its own, for the position it reports and for a change made
/// somewhere that does not go through `Playback`, such as the volume of a steered device.
const WATCH_TICK: Duration = Duration::from_secs(1);

/// What the three searches of a query answered: songs, albums and playlists.
type Found = (
    anyhow::Result<Vec<Track>>,
    anyhow::Result<Vec<Album>>,
    anyhow::Result<Vec<Playlist>>,
);

/// Carries out what the command line asks of the running Sonora. Playback goes through
/// `Playback`, so a device steered over Connect is the one that obeys. A command that fetches
/// tracks first answers once they are queued, so a failure reaches the terminal.
pub struct Control {
    session: Entity<Session>,
    playback: Entity<Playback>,
    library: Entity<Library>,
    /// Queue commands and searches still fetching, each removing itself once it answers.
    pending: HashMap<u64, Task<()>>,
    issued: u64,
    /// A provider switch waiting for the session to settle before it answers.
    switching: Option<Switching>,
    /// Command lines following what plays, as `sonora watch`.
    watchers: Vec<Watcher>,
    /// What keeps the watchers told, held only while there are any.
    watching: Option<(Subscription, Task<()>)>,
    _session: Subscription,
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

impl Control {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        library: Entity<Library>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _session = cx.observe(&session, |this, _, cx| this.settle(cx));
        Self {
            session,
            playback,
            library,
            pending: HashMap::new(),
            issued: 0,
            switching: None,
            watchers: Vec::new(),
            watching: None,
            _session,
        }
    }

    /// Follows what plays for a `sonora watch`: what plays now at once, then again on every
    /// change, until the command line hangs up and `sink` closes.
    pub fn watch(
        &mut self,
        watch: Watch,
        sink: mpsc::UnboundedSender<Reply>,
        cx: &mut Context<Self>,
    ) {
        self.watchers.push(Watcher {
            sink,
            position: watch.position,
            told: None,
        });
        if self.watching.is_none() {
            let observed = cx.observe(&self.playback, |this, _, cx| this.tell(cx));
            let ticking = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(WATCH_TICK).await;
                    if this.update(cx, |this, cx| this.tell(cx)).is_err() {
                        break;
                    }
                }
            });
            self.watching = Some((observed, ticking));
        }
        self.tell(cx);
    }

    /// Sends what plays to every watcher it changed for since its last line, and lets go of the
    /// ones that hung up. With none left, nothing follows playback any more.
    fn tell(&mut self, cx: &mut Context<Self>) {
        let now = current(self.playback.read(cx), cx);
        self.watchers.retain_mut(|watcher| {
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
        if self.watchers.is_empty() {
            self.watching = None;
        }
    }

    /// Carries out `request` and sends what to answer it into `reply`, at once or, for a
    /// command that fetches, once it is done.
    pub fn answer(
        &mut self,
        request: Request,
        reply: oneshot::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let answer = match request {
            Request::Playback(command) => self
                .playback
                .update(cx, |playback, cx| play(playback, command, cx)),
            Request::Queue(command) => return self.queue(command, reply, cx),
            Request::Provider(command) => return self.provider(command, reply, cx),
            Request::Search(query) => return self.search(query, reply, cx),
            Request::Session(command) => session(command, cx),
            // a watch is answered as a stream, by `watch`, and never reaches here
            Request::Watch(_) => Reply::Failed(Failure::Malformed),
        };
        reply.send(answer).ok();
    }

    /// Queues the items of `command`, or the best hit of the search it carries, which first has
    /// to be found.
    fn queue(
        &mut self,
        command: QueueCommand,
        reply: oneshot::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let (items, placement) = match command {
            QueueCommand::Add(items) => (items, Place::Queue(QueuePlacement::End)),
            QueueCommand::PlayNext(items) => (items, Place::Queue(QueuePlacement::Next)),
            QueueCommand::Play(items) => (items, Place::Now),
        };
        if !items.iter().any(|item| matches!(item, Item::Search { .. })) {
            return self.enqueue(items, placement, reply, cx);
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
        self.find(query, cx, move |this, found, cx| {
            let failure = match found.map(|hits| hits.into_iter().next()) {
                Ok(Some(hit)) => {
                    return this.enqueue(vec![Item::Link(hit.link)], placement, reply, cx);
                }
                Ok(None) => Failure::NothingFound(text),
                Err(failure) => failure,
            };
            reply.send(Reply::Failed(failure)).ok();
        });
    }

    /// Fetches every item at once and queues them together, in the order they were given. One
    /// that fails refuses the whole command, so nothing is queued half.
    fn enqueue(
        &mut self,
        items: Vec<Item>,
        placement: Place,
        reply: oneshot::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let sources = items
            .into_iter()
            .map(|item| self.source(item, cx))
            .collect::<Result<Vec<_>, _>>();
        let sources = match sources {
            Ok(sources) => sources,
            Err(failure) => {
                reply.send(Reply::Failed(failure)).ok();
                return;
            }
        };

        let session = self.session.read(cx);
        let local = session.local_provider();
        let client = session.client();
        let fetch = Io::global(cx).spawn(async move {
            let fetches = sources
                .into_iter()
                .map(|source| fetch(source, &local, client.as_ref()));
            anyhow::Ok(future::try_join_all(fetches).await)
        });
        let key = self.issued;
        self.issued += 1;
        let task = cx.spawn(async move |this, cx| {
            let fetched = join(fetch).await;
            this.update(cx, |this, cx| {
                let answer = match fetched {
                    Ok(Ok(batches)) => {
                        this.place(batches, placement, cx);
                        Reply::Done
                    }
                    Ok(Err(missed)) => {
                        if let Some(error) = &missed.error {
                            noted(error, cx);
                        }
                        Reply::Failed(missed.failure)
                    }
                    Err(error) => {
                        log::warn!("control: cannot queue: {error:#}");
                        Reply::Failed(Failure::Malformed)
                    }
                };
                reply.send(answer).ok();
                this.pending.remove(&key);
            })
            .ok();
        });
        self.pending.insert(key, task);
    }

    /// Searches the current provider and answers with the hits.
    fn search(
        &mut self,
        query: SearchQuery,
        reply: oneshot::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        self.find(query, cx, move |_, found, _| {
            let answer = match found {
                Ok(hits) => Reply::Value(Value::Hits(hits)),
                Err(failure) => Reply::Failed(failure),
            };
            reply.send(answer).ok();
        });
    }

    /// Searches the current provider the way the search page does, ranked against the library
    /// alike, and hands `then` the hits that have a link to act on.
    fn find(
        &mut self,
        query: SearchQuery,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Result<Vec<wire::Hit>, Failure>, &mut Context<Self>) + 'static,
    ) {
        let Some(client) = self.session.read(cx).client() else {
            return then(self, Err(Failure::SignedOut), cx);
        };
        let fetch = {
            let client = client.clone();
            let asked = query.text.clone();
            Io::global(cx).spawn(async move {
                anyhow::Ok(
                    future::join3(
                        client.search(&asked),
                        client.search_albums(&asked),
                        client.search_playlists(&asked),
                    )
                    .await,
                )
            })
        };
        let key = self.issued;
        self.issued += 1;
        let task = cx.spawn(async move |this, cx| {
            let fetched = join(fetch).await;
            this.update(cx, |this, cx| {
                this.pending.remove(&key);
                let found = match fetched {
                    Ok(found) => this.hits(&query, found, client.as_ref(), cx),
                    Err(error) => {
                        log::warn!("control: cannot search: {error:#}");
                        Err(Failure::Unfetched(query.text.clone()))
                    }
                };
                then(this, found, cx);
            })
            .ok();
        });
        self.pending.insert(key, task);
    }

    /// Ranks what the provider found against the library and keeps `query.limit` hits of the
    /// kind asked for. A part of the search that failed is left out, as the search page leaves
    /// it, and only a search that failed whole is refused.
    fn hits(
        &self,
        query: &SearchQuery,
        (tracks, albums, playlists): Found,
        client: &dyn MusicApi,
        cx: &mut App,
    ) -> Result<Vec<wire::Hit>, Failure> {
        if let (Err(error), Err(_), Err(_)) = (&tracks, &albums, &playlists) {
            log::warn!("control: cannot search: {error:#}");
            noted(error, cx);
            return Err(Failure::Unfetched(query.text.clone()));
        }
        let catalog = Catalog {
            tracks: tracks.unwrap_or_default(),
            albums: albums.unwrap_or_default(),
            playlists: playlists.unwrap_or_default(),
        };
        let library = self.library.read(cx).state(Shelf::Streaming);
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

    /// Lists the providers or tells the current one at once. A switch answers once the session
    /// has signed in, or failed to, so a script can go on to use the provider it asked for.
    fn provider(
        &mut self,
        command: ProviderCommand,
        reply: oneshot::Sender<Reply>,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
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
            ProviderCommand::Current(Some(slug)) => return self.switch(slug, reply, cx),
        };
        reply.send(Reply::Value(value)).ok();
    }

    fn switch(&mut self, slug: String, reply: oneshot::Sender<Reply>, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let refusal = match session.account(&slug) {
            None => Some(Failure::UnknownProvider(slug.clone())),
            Some((name, false)) => Some(Failure::NoAccount(name.to_owned())),
            Some(_) if self.switching.is_some() || session.is_pending() => Some(Failure::Busy),
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
        self.switching = Some(Switching {
            slug: slug.clone(),
            name,
            reply,
        });
        self.session
            .update(cx, |session, cx| session.switch(&slug, cx));
    }

    /// Answers a waiting switch once the session is no longer signing in. A provider that could
    /// not sign in hands over to another, so landing elsewhere is a failure.
    fn settle(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if session.is_pending() {
            return;
        }
        let Some(switching) = self.switching.take() else {
            return;
        };
        let answer = match settled_on(session, &switching.slug) {
            true => Reply::Done,
            false => Reply::Failed(Failure::SignInFailed(switching.name.to_owned())),
        };
        switching.reply.send(answer).ok();
    }

    /// Checks an item before anything is fetched: a link has to be a track, album, playlist or
    /// artist of the provider signed in now.
    fn source(&self, item: Item, cx: &App) -> Result<Source, Failure> {
        let link = match item {
            Item::Path(path) => return Ok(Source::Path(path)),
            Item::Link(link) => link,
            // `queue` finds a search before anything reaches here
            Item::Search { .. } => return Err(Failure::Malformed),
        };
        let session = self.session.read(cx);
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

    /// Plays or queues what was fetched in one go, so the items keep their order and one toast
    /// tells of them. A lone collection keeps its origin, several mixed ones have none.
    fn place(&self, batches: Vec<Batch>, placement: Place, cx: &mut App) {
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
        let playing = self.playback.read(cx).steered(cx).is_some()
            || Sonora::global(cx).queue.read(cx).current().is_some();
        self.playback.update(cx, |playback, cx| match placement {
            Place::Now => {
                playback.insert_all(tracks, 0, origin, cx);
                if playing {
                    playback.next(cx);
                }
            }
            Place::Queue(placement) => playback.enqueue_tracks(tracks, origin, placement, cx),
        });
    }
}

/// Starts, joins or leaves a Sonora Session, or says which one Sonora is in. A start or join
/// answers before the server does, so `sonora session` tells whether it went through.
fn session(command: SessionCommand, cx: &mut App) -> Reply {
    let listening = Sonora::global(cx).listening.clone();
    match command {
        SessionCommand::Status => {
            let listening = listening.read(cx);
            let info = listening.room().map(|room| SessionInfo {
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
        SessionCommand::Start if !listening.read(cx).available(cx) => {
            return Reply::Failed(Failure::SignedOut);
        }
        SessionCommand::Start => listening.update(cx, |listening, cx| listening.start(cx)),
        SessionCommand::Join(Some(code)) => {
            listening.update(cx, |listening, cx| listening.join(&code, cx))
        }
        SessionCommand::Join(None) => {
            listening.update(cx, |listening, cx| listening.show_join(true, cx))
        }
        SessionCommand::Leave => listening.update(cx, |listening, cx| listening.leave(cx)),
    }
    Reply::Done
}

fn play(playback: &mut Playback, command: PlaybackCommand, cx: &mut Context<Playback>) -> Reply {
    match command {
        PlaybackCommand::Play => playback.resume(cx),
        PlaybackCommand::Pause => playback.pause(cx),
        PlaybackCommand::Toggle => playback.toggle_play(cx),
        PlaybackCommand::Next => playback.next(cx),
        PlaybackCommand::Previous => playback.previous(cx),
        PlaybackCommand::Seek(percent) => playback.seek_fraction(f32::from(percent) / 100., cx),
        PlaybackCommand::SeekTime(time) => {
            let target = moment(playback, time, cx);
            playback.seek(target, cx);
        }
        PlaybackCommand::CurrentTrack => {
            return Reply::Value(Value::Current(current(playback, cx)));
        }
        PlaybackCommand::Volume(None) => {
            return Reply::Value(Value::Volume(percent(playback.shown_volume(cx))));
        }
        PlaybackCommand::Volume(Some(level)) => {
            let target = match level {
                Level::To(level) => i16::from(level),
                Level::By(step) => i16::from(percent(playback.shown_volume(cx))) + i16::from(step),
            };
            playback.set_volume(f32::from(target.clamp(0, 100)) / 100., cx);
        }
        PlaybackCommand::Shuffle(None) => {
            return Reply::Value(Value::Shuffle(playback.shown_shuffle(cx)));
        }
        PlaybackCommand::Shuffle(Some(switch)) => match switch {
            Switch::On => playback.set_shuffle(true, cx),
            Switch::Off => playback.set_shuffle(false, cx),
            Switch::Toggle => playback.toggle_shuffle(cx),
        },
        PlaybackCommand::Repeat(None) => {
            return Reply::Value(Value::Repeat(wire_repeat(playback.shown_repeat(cx))));
        }
        PlaybackCommand::Repeat(Some(repeat)) => {
            let repeat = match repeat {
                commands::Repeat::Off => Repeat::Off,
                commands::Repeat::All => Repeat::All,
                commands::Repeat::One => Repeat::One,
            };
            playback.set_repeat(repeat, cx);
        }
    }
    Reply::Done
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
fn moment(playback: &Playback, time: Time, cx: &App) -> Duration {
    let (now, length) = match playback.steered(cx) {
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
fn current(playback: &Playback, cx: &App) -> Option<Current> {
    let track = playback.shown_track(cx)?;
    let (state, position) = match playback.steered(cx) {
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
    let sonora = Sonora::global(cx);
    let id = track.id.as_deref();
    let provider = match id.is_some_and(|id| Shelf::of(id).local()) {
        true => Some(LOCAL.to_owned()),
        false => sonora.session.read(cx).provider_slug().map(str::to_owned),
    };
    Some(Current {
        state,
        track: wire_track(&track),
        position: position.as_secs(),
        volume: percent(playback.shown_volume(cx)),
        shuffle: playback.shown_shuffle(cx),
        repeat: wire_repeat(playback.shown_repeat(cx)),
        provider,
        liked: id.is_some_and(|id| sonora.library.read(cx).saved(id)),
        origin: playback.origin(cx).map(wire_origin),
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
        Linked::Artist(id) => client.artist(&id).await.map(|artist| Batch {
            tracks: artist.top_tracks,
            origin: Some(Origin::artist(id).named(artist.name)),
        }),
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
