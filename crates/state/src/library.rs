use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, Context, Entity, SharedString, Task};
use music::{Album, MediaKind, MusicApi, Page, Pages, Playlist, SavedArtist, Shape, Track};

use crate::snapshot::{Kind, Remembered, Snapshots};
use crate::{Io, Network, Outcome, Session, SessionEvent, Target, Toasts, join, mosaic};

const FATAL: [LibraryPart; 3] = [
    LibraryPart::Tracks,
    LibraryPart::Playlists,
    LibraryPart::Albums,
];
const FATAL_LOCAL: [LibraryPart; 2] = [LibraryPart::Tracks, LibraryPart::Albums];

/// Which provider a library page, an id or a playlist belongs to. The streaming shelf follows
/// the signed-in provider; the local shelf follows the imported folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    Streaming,
    Local,
}

impl Shelf {
    /// The shelf an id belongs to, by its prefix.
    pub fn of(id: &str) -> Self {
        match music::is_local_id(id) {
            true => Self::Local,
            false => Self::Streaming,
        }
    }

    pub fn local(self) -> bool {
        self == Self::Local
    }

    /// What a log line calls the shelf.
    fn name(self) -> &'static str {
        match self {
            Self::Streaming => "streaming",
            Self::Local => "local",
        }
    }

    fn slot(self) -> usize {
        match self {
            Self::Streaming => 0,
            Self::Local => 1,
        }
    }

    /// The parts whose joint failure marks the whole shelf failed, rather than one problem.
    fn fatal(self) -> &'static [LibraryPart] {
        match self {
            Self::Streaming => &FATAL,
            Self::Local => &FATAL_LOCAL,
        }
    }
}

/// Everything the library holds for one shelf. On a `Shape::Saved` shelf the lists in `state`
/// are the favorites; on a `Shape::Catalog` shelf they are the whole catalog and `starred`
/// carries the favorites beside them.
struct Held {
    shape: Shape,
    state: LibraryState,
    awaited: Vec<LibraryPart>,
    /// How many rows each part will have once it has all arrived, for the parts whose
    /// provider said so on the first page. Absent, the rows so far are the count.
    expected: HashMap<LibraryPart, usize>,
    /// The parts whose rows come from the last run's snapshot rather than from this run's
    /// provider. The first real rows of such a part replace them instead of joining them.
    stale: HashSet<LibraryPart>,
    starred: Starred,
    tasks: Vec<Task<()>>,
}

impl Held {
    fn empty() -> Self {
        Self {
            shape: Shape::Saved,
            state: LibraryState::Empty,
            awaited: Vec::new(),
            expected: HashMap::new(),
            stale: HashSet::new(),
            starred: Starred::default(),
            tasks: Vec::new(),
        }
    }

    fn clear(&mut self) {
        *self = Self::empty();
    }

    fn ready(&self) -> Option<&Ready> {
        match &self.state {
            LibraryState::Ready(ready) => Some(ready),
            _ => None,
        }
    }

    fn ready_mut(&mut self) -> Option<&mut Ready> {
        match &mut self.state {
            LibraryState::Ready(ready) => Some(ready),
            _ => None,
        }
    }

    /// The favorites of each kind, wherever the shape keeps them.
    fn favorites(&self) -> Option<Favorites<'_>> {
        match self.shape {
            Shape::Saved => {
                let ready = self.ready()?;
                Some(Favorites {
                    tracks: &ready.tracks,
                    albums: &ready.albums,
                    artists: &ready.artists,
                })
            }
            Shape::Catalog => Some(Favorites {
                tracks: &self.starred.tracks,
                albums: &self.starred.albums,
                artists: &self.starred.artists,
            }),
        }
    }

    fn favorites_mut(&mut self) -> Option<FavoritesMut<'_>> {
        match self.shape {
            Shape::Saved => {
                let ready = self.ready_mut()?;
                Some(FavoritesMut {
                    tracks: &mut ready.tracks,
                    albums: &mut ready.albums,
                    artists: &mut ready.artists,
                })
            }
            Shape::Catalog => Some(FavoritesMut {
                tracks: &mut self.starred.tracks,
                albums: &mut self.starred.albums,
                artists: &mut self.starred.artists,
            }),
        }
    }
}

#[derive(Default)]
struct Starred {
    tracks: Vec<Track>,
    albums: Vec<Album>,
    artists: Vec<SavedArtist>,
}

struct Favorites<'a> {
    tracks: &'a [Track],
    albums: &'a [Album],
    artists: &'a [SavedArtist],
}

struct FavoritesMut<'a> {
    tracks: &'a mut Vec<Track>,
    albums: &'a mut Vec<Album>,
    artists: &'a mut Vec<SavedArtist>,
}

/// The rows of one part on their way to a snapshot, owned so the write can happen off the main
/// thread.
enum Kept {
    Tracks(Vec<Track>),
    Playlists(Vec<Playlist>),
    Albums(Vec<Album>),
    Artists(Vec<SavedArtist>),
}

impl Kept {
    fn listed(ready: &Ready, part: LibraryPart) -> Self {
        match part {
            LibraryPart::Tracks => Self::Tracks(ready.tracks.clone()),
            LibraryPart::Playlists => Self::Playlists(ready.playlists.clone()),
            LibraryPart::Albums => Self::Albums(ready.albums.clone()),
            LibraryPart::Artists => Self::Artists(ready.artists.clone()),
        }
    }

    /// The favorites of one part. Playlists have none, so they answer an empty list that the
    /// caller never asks for.
    fn starred(starred: &Starred, part: LibraryPart) -> Self {
        match part {
            LibraryPart::Tracks => Self::Tracks(starred.tracks.clone()),
            LibraryPart::Albums => Self::Albums(starred.albums.clone()),
            LibraryPart::Artists => Self::Artists(starred.artists.clone()),
            LibraryPart::Playlists => Self::Playlists(Vec::new()),
        }
    }
}

enum Landed {
    Tracks(anyhow::Result<Vec<Track>>),
    Playlists(anyhow::Result<Vec<Playlist>>),
    Albums(anyhow::Result<Vec<Album>>),
    Artists(anyhow::Result<Vec<SavedArtist>>),
}

impl Landed {
    /// Whether the provider answered at all. A part that failed keeps whatever the snapshot
    /// put there, so a launch without a network still shows the library it saw last time.
    fn arrived(&self) -> bool {
        match self {
            Self::Tracks(result) => result.is_ok(),
            Self::Playlists(result) => result.is_ok(),
            Self::Albums(result) => result.is_ok(),
            Self::Artists(result) => result.is_ok(),
        }
    }

    fn part(&self) -> LibraryPart {
        match self {
            Self::Tracks(_) => LibraryPart::Tracks,
            Self::Playlists(_) => LibraryPart::Playlists,
            Self::Albums(_) => LibraryPart::Albums,
            Self::Artists(_) => LibraryPart::Artists,
        }
    }

    /// Why the part failed, flattened, when it did.
    fn reason(&self) -> Option<String> {
        let error = match self {
            Self::Tracks(result) => result.as_ref().err()?,
            Self::Playlists(result) => result.as_ref().err()?,
            Self::Albums(result) => result.as_ref().err()?,
            Self::Artists(result) => result.as_ref().err()?,
        };
        Some(format!("{error:#}"))
    }
}

struct PlaylistMutation {
    action: &'static str,
    done: &'static str,
    name: Option<String>,
    target: Option<Target>,
    invalidated: Option<String>,
    shelf: Shelf,
}

/// Puts one part onto the shelf whole: its rows, then the word that it is complete.
fn place(
    state: &mut LibraryState,
    awaited: &mut Vec<LibraryPart>,
    landed: Landed,
    fatal: &[LibraryPart],
) {
    let part = landed.part();
    extend(state, landed);
    settle(state, awaited, part, fatal);
}

/// Puts one part's rows, or its failure, onto the shelf. A part arriving in pages passes here
/// once per page, so the rows already there stay and the new ones go behind them. A track the
/// provider cannot play is left out, so the songs list and its count hold only what plays.
fn extend(state: &mut LibraryState, landed: Landed) {
    if !matches!(state, LibraryState::Ready(_)) {
        *state = LibraryState::Ready(Ready::default());
    }
    let LibraryState::Ready(ready) = state else {
        return;
    };
    let Ready {
        tracks,
        playlists,
        albums,
        artists,
        problems,
    } = ready;
    let part = landed.part();
    match landed {
        Landed::Tracks(result) => tracks.extend(
            take(part, result, problems)
                .into_iter()
                .filter(|track| track.playable),
        ),
        Landed::Playlists(result) => playlists.extend(take(part, result, problems)),
        Landed::Albums(result) => albums.extend(take(part, result, problems)),
        Landed::Artists(result) => artists.extend(take(part, result, problems)),
    }
}

/// Drops the rows a snapshot put on one part, the first time that part's own rows arrive. A
/// part that was never primed passes through untouched.
fn shed(state: &mut LibraryState, stale: &mut HashSet<LibraryPart>, part: LibraryPart) {
    if stale.remove(&part) {
        empty(state, part);
    }
}

/// Takes one part's rows off the shelf, leaving the others where they are.
fn empty(state: &mut LibraryState, part: LibraryPart) {
    let LibraryState::Ready(ready) = state else {
        return;
    };
    match part {
        LibraryPart::Tracks => ready.tracks.clear(),
        LibraryPart::Playlists => ready.playlists.clear(),
        LibraryPart::Albums => ready.albums.clear(),
        LibraryPart::Artists => ready.artists.clear(),
    }
}

/// Marks one part as fully arrived. Once every part has, a shelf whose fatal parts all failed
/// is failed as a whole.
fn settle(
    state: &mut LibraryState,
    awaited: &mut Vec<LibraryPart>,
    part: LibraryPart,
    fatal: &[LibraryPart],
) {
    awaited.retain(|waiting| *waiting != part);
    if !awaited.is_empty() {
        return;
    }
    let failure = {
        let LibraryState::Ready(ready) = state else {
            return;
        };
        let reasons: Vec<&str> = fatal
            .iter()
            .filter_map(|part| ready.problems.iter().find(|problem| problem.part == *part))
            .map(|problem| problem.reason.as_str())
            .collect();
        (reasons.len() == fatal.len()).then(|| reasons.join("\n"))
    };
    if let Some(reason) = failure {
        *state = LibraryState::Failed(reason);
    }
}

/// Puts one loaded favorites set onto a catalog shelf and says whether it arrived. A failure
/// only logs and leaves the set alone, since the catalog is still there to show and a snapshot
/// may have filled it.
///
/// Playlists never arrive here. The favorites sets exist because a catalog shelf lists
/// everything the provider has and needs the starred ones beside it; a shelf's playlists are
/// the listener's own whatever its shape, so there is no second set to load and no
/// `saved_playlists` on `MusicApi` to load it from.
fn star(held: &mut Held, landed: Landed) -> bool {
    match landed {
        Landed::Tracks(result) => hold("tracks", result, &mut held.starred.tracks),
        Landed::Albums(result) => hold("albums", result, &mut held.starred.albums),
        Landed::Artists(result) => hold("artists", result, &mut held.starred.artists),
        Landed::Playlists(_) => false,
    }
}

fn hold<T>(what: &str, result: anyhow::Result<Vec<T>>, into: &mut Vec<T>) -> bool {
    match result {
        Ok(rows) => {
            *into = rows;
            true
        }
        Err(error) => {
            log::warn!("library: cannot load the starred {what}: {error:#}");
            false
        }
    }
}

fn stamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn take<T>(
    part: LibraryPart,
    result: anyhow::Result<Vec<T>>,
    problems: &mut Vec<Problem>,
) -> Vec<T> {
    result.unwrap_or_else(|error| {
        log::warn!("library: cannot load {}: {error:#}", part.label());
        problems.push(Problem {
            part,
            reason: format!("{error:#}"),
        });
        Vec::new()
    })
}

impl Library {
    fn toggle_saved<S: Savable>(&mut self, mut item: S, cx: &mut Context<Self>) {
        let Some(id) = item.id().map(str::to_owned) else {
            return;
        };
        if S::requests(self).contains_key(&id) {
            return;
        }
        let Some(client) = self.session.read(cx).client_of(Shelf::of(&id)) else {
            return;
        };

        let previous = S::saved_now(self, &id);
        let saved = previous.is_none();
        if saved {
            item.stamp_added();
        }
        S::hold(self, item.clone(), saved);

        let asked = id.clone();
        let answered = id.clone();
        let target = S::target(&id);
        let io = self.io.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = join(io.spawn(S::ask(client, asked, saved))).await;
            this.update(cx, |this, cx| {
                S::requests(this).remove(&answered);
                match result {
                    Ok(()) if saved => S::admit(this, item, cx),
                    Ok(()) => {}
                    Err(error) => {
                        let name = match &previous {
                            Some(previous) => previous.title().to_owned(),
                            None => item.title().to_owned(),
                        };
                        match previous {
                            Some(previous) => S::hold(this, previous, true),
                            None => S::hold(this, item, false),
                        }
                        log::warn!("library: cannot update the {}: {error:#}", S::TROUBLE);
                        let key = match saved {
                            true => "toast-library-add-failed",
                            false => "toast-library-remove-failed",
                        };
                        Toasts::linked(Outcome::Failed, key, name, target, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        S::requests(self).insert(id, task);
        cx.notify();
    }
}

trait Savable: Clone + Send + Sized + 'static {
    const TROUBLE: &'static str;

    fn id(&self) -> Option<&str>;
    fn title(&self) -> &str;
    fn target(id: &str) -> Option<Target>;
    fn stamp_added(&mut self) {}
    fn saved_now(library: &Library, id: &str) -> Option<Self>;
    fn requests(library: &mut Library) -> &mut HashMap<String, Task<()>>;
    fn hold(library: &mut Library, item: Self, saved: bool);
    /// What else the shelf does once the provider has agreed to the favorite. Nothing for
    /// most kinds.
    fn admit(_library: &mut Library, _item: Self, _cx: &mut Context<Library>) {}
    fn ask(
        client: Arc<dyn MusicApi>,
        id: String,
        saved: bool,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl Savable for Track {
    const TROUBLE: &'static str = "saved track";

    fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    fn title(&self) -> &str {
        &self.name
    }

    fn target(id: &str) -> Option<Target> {
        Some(Target::Song(SharedString::from(id.to_owned())))
    }

    fn stamp_added(&mut self) {
        self.added_at = Some(stamp());
    }

    fn saved_now(library: &Library, id: &str) -> Option<Self> {
        let favorites = library.favorites(id)?;
        favorites
            .tracks
            .iter()
            .find(|track| track.id.as_deref() == Some(id))
            .cloned()
    }

    fn requests(library: &mut Library) -> &mut HashMap<String, Task<()>> {
        &mut library.pending
    }

    fn hold(library: &mut Library, item: Self, saved: bool) {
        library.set_saved(item, saved);
    }

    /// Puts the track onto the shelf's own pages where the library is a set apart from the
    /// favorites. Apple Music, the one such provider, adds a favorited song to the library
    /// itself, so the Songs page lists it without a reload.
    fn admit(library: &mut Library, track: Self, cx: &mut Context<Library>) {
        let Some(id) = track.id.clone() else {
            return;
        };
        let shelf = Shelf::of(&id);
        if !library.session.read(cx).capabilities_of(shelf).library {
            return;
        }
        let Some(ready) = library.held_mut(shelf).ready_mut() else {
            return;
        };
        if ready.tracks.iter().any(|known| known.id == track.id) {
            return;
        }
        ready.tracks.insert(0, track);
    }

    async fn ask(client: Arc<dyn MusicApi>, id: String, saved: bool) -> anyhow::Result<()> {
        client.set_track_saved(&id, saved).await
    }
}

impl Savable for Album {
    const TROUBLE: &'static str = "saved album";

    fn id(&self) -> Option<&str> {
        Some(&self.id)
    }

    fn title(&self) -> &str {
        &self.name
    }

    fn target(id: &str) -> Option<Target> {
        Some(Target::Album(SharedString::from(id.to_owned())))
    }

    fn stamp_added(&mut self) {
        self.added_at = Some(stamp());
    }

    fn saved_now(library: &Library, id: &str) -> Option<Self> {
        let favorites = library.favorites(id)?;
        favorites
            .albums
            .iter()
            .find(|album| album.id == id)
            .cloned()
    }

    fn requests(library: &mut Library) -> &mut HashMap<String, Task<()>> {
        &mut library.pending_albums
    }

    fn hold(library: &mut Library, item: Self, saved: bool) {
        library.set_album_saved(item, saved);
    }

    async fn ask(client: Arc<dyn MusicApi>, id: String, saved: bool) -> anyhow::Result<()> {
        client.set_album_saved(&id, saved).await
    }
}

impl Savable for SavedArtist {
    const TROUBLE: &'static str = "favorite artist";

    fn id(&self) -> Option<&str> {
        Some(&self.id)
    }

    fn title(&self) -> &str {
        &self.name
    }

    fn target(id: &str) -> Option<Target> {
        Some(Target::Artist(SharedString::from(id.to_owned())))
    }

    fn stamp_added(&mut self) {
        self.added_at = Some(stamp());
    }

    fn saved_now(library: &Library, id: &str) -> Option<Self> {
        let favorites = library.favorites(id)?;
        favorites
            .artists
            .iter()
            .find(|artist| artist.id == id)
            .cloned()
    }

    fn requests(library: &mut Library) -> &mut HashMap<String, Task<()>> {
        &mut library.pending_artists
    }

    fn hold(library: &mut Library, item: Self, saved: bool) {
        library.set_artist_saved(item, saved);
    }

    async fn ask(client: Arc<dyn MusicApi>, id: String, saved: bool) -> anyhow::Result<()> {
        client.set_artist_saved(&id, saved).await
    }
}

pub enum LibraryEvent {
    PlaylistGone(String),
    TrackAdded { playlist: String },
    TrackDropped { playlist: String, track: String },
    TracksHidden(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibraryPart {
    Tracks,
    Playlists,
    Albums,
    Artists,
}

impl LibraryPart {
    const ALL: [Self; 4] = [Self::Tracks, Self::Playlists, Self::Albums, Self::Artists];

    /// How the part names itself in a snapshot key, which is also how a log line calls it.
    pub(crate) fn key(self) -> &'static str {
        self.label()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Tracks => "songs",
            Self::Playlists => "playlists",
            Self::Albums => "albums",
            Self::Artists => "artists",
        }
    }
}

pub struct Problem {
    pub part: LibraryPart,
    pub reason: String,
}

/// What a shelf shows once loaded. A part that failed is empty and named in `problems`.
#[derive(Default)]
pub struct Ready {
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub albums: Vec<Album>,
    pub artists: Vec<SavedArtist>,
    pub problems: Vec<Problem>,
}

pub enum LibraryState {
    Empty,
    Loading,
    Ready(Ready),
    Failed(String),
}

impl LibraryState {
    pub fn ready(&self) -> Option<&Ready> {
        match self {
            Self::Ready(ready) => Some(ready),
            _ => None,
        }
    }

    pub fn tracks(&self) -> &[Track] {
        self.ready().map_or(&[], |ready| ready.tracks.as_slice())
    }

    pub fn playlists(&self) -> &[Playlist] {
        self.ready().map_or(&[], |ready| ready.playlists.as_slice())
    }

    pub fn albums(&self) -> &[Album] {
        self.ready().map_or(&[], |ready| ready.albums.as_slice())
    }

    pub fn artists(&self) -> &[SavedArtist] {
        self.ready().map_or(&[], |ready| ready.artists.as_slice())
    }
}

impl gpui::EventEmitter<LibraryEvent> for Library {}

pub struct Library {
    shelves: [Held; 2],
    session: Entity<Session>,
    io: Io,
    playlist_task: Option<Task<()>>,
    sidebar_task: Option<Task<()>>,
    sidebar_pin_task: Option<Task<()>>,
    sidebar_items: Option<Vec<music::LibraryItem>>,
    pending: HashMap<String, Task<()>>,
    pending_albums: HashMap<String, Task<()>>,
    pending_artists: HashMap<String, Task<()>>,
    /// Tracks and albums on their way into or out of the provider's own library, by id.
    pending_library: HashMap<String, Task<()>>,
    contents: HashMap<String, HashSet<String>>,
    hidden_local_tracks: HashSet<String>,
    reading: HashMap<String, Task<()>>,
    mosaics: HashMap<String, Task<()>>,
    snapshots: Snapshots,
    priming: [Option<Task<()>>; 2],
}

impl Library {
    pub fn new(
        session: Entity<Session>,
        io: Io,
        cache: storage::Cache,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, session, event, cx| match event {
            SessionEvent::SignedIn => {
                this.sidebar_pin_task = None;
                if !session.read(cx).authenticated() {
                    this.sidebar_task = None;
                    this.sidebar_items = None;
                    this.held_mut(Shelf::Streaming).clear();
                    cx.notify();
                    return;
                }
                this.sidebar_items = None;
                this.sync_sidebar(cx);
                this.prime(Shelf::Streaming, cx);
                this.load(Shelf::Streaming, cx);
            }
            SessionEvent::SignedOut => {
                this.forget(Shelf::Streaming, cx);
                this.sidebar_pin_task = None;
                this.sidebar_task = None;
                this.sidebar_items = None;
                this.contents.clear();
                this.reading.clear();
                this.mosaics.clear();
                this.playlist_task = None;
                this.pending.clear();
                this.pending_albums.clear();
                this.pending_artists.clear();
                this.pending_library.clear();
                this.held_mut(Shelf::Streaming).clear();
                cx.notify();
            }
            SessionEvent::Reconnected => {
                if matches!(this.held(Shelf::Streaming).state, LibraryState::Failed(_)) {
                    this.load(Shelf::Streaming, cx);
                }
            }
            SessionEvent::LocalChanged => {
                this.hidden_local_tracks.clear();
                match session.read(cx).client_of(Shelf::Local) {
                    Some(_) => this.load(Shelf::Local, cx),
                    None => {
                        this.held_mut(Shelf::Local).clear();
                        cx.notify();
                    }
                }
            }
        })
        .detach();

        let mut library = Self {
            shelves: [Held::empty(), Held::empty()],
            session,
            io,
            playlist_task: None,
            sidebar_task: None,
            sidebar_pin_task: None,
            sidebar_items: None,
            pending: HashMap::new(),
            pending_albums: HashMap::new(),
            pending_artists: HashMap::new(),
            pending_library: HashMap::new(),
            contents: HashMap::new(),
            hidden_local_tracks: HashSet::new(),
            reading: HashMap::new(),
            mosaics: HashMap::new(),
            snapshots: Snapshots::new(cache),
            priming: [None, None],
        };
        library.held_mut(Shelf::Streaming).state = LibraryState::Loading;
        library.prime(Shelf::Streaming, cx);
        match library.session.read(cx).client_of(Shelf::Local).is_some() {
            true => library.load(Shelf::Local, cx),
            false => library.prime(Shelf::Local, cx),
        }
        library
    }

    /// Puts the last run's rows on a shelf while this run's are still on their way, so a launch
    /// shows a library before the provider has answered. Every part stays awaited, so the page
    /// still reads as loading and the first rows the provider sends replace what was primed.
    /// A shelf that has already heard from its provider is left alone.
    fn prime(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let provider = match shelf {
            Shelf::Streaming => session.provider_slug(),
            Shelf::Local => (!session.local_paths().is_empty()).then(|| session.local_slug()),
        };
        let Some(provider) = provider else {
            return;
        };

        let snapshots = self.snapshots.clone();
        let io = self.io.clone();
        self.priming[shelf.slot()] = Some(cx.spawn(async move |this, cx| {
            let Ok(Some(remembered)) = io.spawn_blocking(move || snapshots.restore(provider)).await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                let held = this.held_mut(shelf);
                if !matches!(held.state, LibraryState::Empty | LibraryState::Loading) {
                    return;
                }
                let Remembered {
                    shape,
                    totals,
                    parts,
                    tracks,
                    playlists,
                    albums,
                    artists,
                    starred_tracks,
                    starred_albums,
                    starred_artists,
                } = remembered;
                log::debug!(
                    "library: the {} shelf starts from a snapshot of {} songs, {} playlists, \
                     {} albums and {} artists",
                    shelf.name(),
                    tracks.len(),
                    playlists.len(),
                    albums.len(),
                    artists.len()
                );
                held.shape = shape;
                held.expected = totals;
                held.stale = parts.into_iter().collect();
                held.awaited = LibraryPart::ALL.to_vec();
                held.starred = Starred {
                    tracks: starred_tracks,
                    albums: starred_albums,
                    artists: starred_artists,
                };
                held.state = LibraryState::Ready(Ready {
                    tracks,
                    playlists,
                    albums,
                    artists,
                    problems: Vec::new(),
                });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Remembers a list that has just arrived in full, for the next launch to show. A part that
    /// failed keeps whatever was remembered before it.
    fn keep(&self, shelf: Shelf, kind: Kind, part: LibraryPart, cx: &App) {
        let Some(provider) = self.provider_of(shelf, cx) else {
            return;
        };
        let held = self.held(shelf);
        let rows = match kind {
            Kind::Listed => {
                let LibraryState::Ready(ready) = &held.state else {
                    return;
                };
                if ready.problems.iter().any(|problem| problem.part == part) {
                    return;
                }
                Kept::listed(ready, part)
            }
            Kind::Starred => Kept::starred(&held.starred, part),
        };

        let snapshots = self.snapshots.clone();
        let shape = held.shape;
        drop(self.io.spawn_blocking(move || match rows {
            Kept::Tracks(rows) => snapshots.keep(provider, kind, part, shape, &rows),
            Kept::Playlists(rows) => snapshots.keep(provider, kind, part, shape, &rows),
            Kept::Albums(rows) => snapshots.keep(provider, kind, part, shape, &rows),
            Kept::Artists(rows) => snapshots.keep(provider, kind, part, shape, &rows),
        }));
    }

    /// Drops what a shelf remembered, so signing out of a provider takes its library with it.
    fn forget(&mut self, shelf: Shelf, cx: &App) {
        let Some(provider) = self.provider_of(shelf, cx) else {
            return;
        };
        self.priming[shelf.slot()] = None;
        let snapshots = self.snapshots.clone();
        drop(self.io.spawn_blocking(move || snapshots.forget(provider)));
    }

    /// The provider whose name a shelf's snapshots are filed under.
    fn provider_of(&self, shelf: Shelf, cx: &App) -> Option<&'static str> {
        let session = self.session.read(cx);
        match shelf {
            Shelf::Streaming => session.provider_slug(),
            Shelf::Local => Some(session.local_slug()),
        }
    }

    pub fn sidebar_items(&self) -> Option<&[music::LibraryItem]> {
        self.sidebar_items.as_deref()
    }

    pub fn sidebar_pin_pending(&self) -> bool {
        self.sidebar_pin_task.is_some()
    }

    /// Changes the provider's own pin for `uri` and calls `done` with whether it stuck. A second
    /// change replaces the one in flight, so the last one wins and only its `done` runs. A pin
    /// the provider turns away for being past its limit counts as stuck too, since Sonora keeps
    /// the pin itself and the sidebar has no limit of its own.
    pub fn set_sidebar_pinned(
        &mut self,
        uri: String,
        pinned: bool,
        done: impl FnOnce(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self
            .sidebar_items
            .as_ref()
            .and_then(|items| items.iter().find(|item| item.uri == uri))
            .is_some_and(|item| item.pinned == pinned)
        {
            return;
        }
        let Some(client) = self.session.read(cx).client_of(Shelf::Streaming) else {
            return;
        };
        // Keep a polling response from overwriting the result of this mutation.
        self.sidebar_task = None;
        let io = self.io.clone();
        let order = music::LibraryOrder::default();
        self.sidebar_pin_task = Some(cx.spawn(async move |this, cx| {
            let result = join(io.spawn(async move {
                let result = client.set_library_item_pinned(&uri, pinned).await?;
                if result == music::LibraryPinResult::LimitReached {
                    return Ok((result, None));
                }
                let items = client.library_items(order).await?;
                anyhow::ensure!(
                    items.as_ref().is_some_and(|items| items
                        .iter()
                        .any(|item| item.uri == uri && item.pinned == pinned)),
                    "the provider did not confirm the updated library pin"
                );
                Ok((result, items))
            }))
            .await;
            this.update(cx, |this, cx| {
                this.sidebar_pin_task = None;
                match result {
                    Ok((music::LibraryPinResult::Updated, items)) => {
                        this.sidebar_items = items;
                        done(true, cx);
                    }
                    Ok((music::LibraryPinResult::LimitReached, _)) => {
                        log::debug!(
                            "library: the provider's pin limit is reached, pinning locally"
                        );
                        done(true, cx);
                    }
                    Err(error) => {
                        log::warn!("library: cannot update the library pin: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-library-pin-failed", cx);
                        done(false, cx);
                    }
                }
                this.sync_sidebar(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn sync_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_pin_pending() {
            return;
        }
        self.sidebar_task = None;
        let Some(client) = self.session.read(cx).client_of(Shelf::Streaming) else {
            return;
        };
        let io = self.io.clone();
        let order = music::LibraryOrder::default();
        self.sidebar_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let client = client.clone();
                let loaded = join(io.spawn(async move { client.library_items(order).await })).await;
                if this
                    .update(cx, |this, cx| match loaded {
                        Ok(items) if items != this.sidebar_items => {
                            this.sidebar_items = items;
                            cx.notify();
                        }
                        Ok(_) => {}
                        Err(error) => {
                            log::warn!("library: cannot synchronize sidebar library: {error:#}");
                            crate::noted(&error, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
            }
        }));
    }

    fn held(&self, shelf: Shelf) -> &Held {
        &self.shelves[shelf.slot()]
    }

    fn held_mut(&mut self, shelf: Shelf) -> &mut Held {
        &mut self.shelves[shelf.slot()]
    }

    pub fn state(&self, shelf: Shelf) -> &LibraryState {
        &self.held(shelf).state
    }

    /// Whether the shelf has rows to show, this run's or a snapshot's. The sidebar asks, so
    /// Your Library is there from the first frame a snapshot is read rather than from the
    /// moment the session finishes restoring.
    pub fn stocked(&self, shelf: Shelf) -> bool {
        self.held(shelf).ready().is_some()
    }

    /// What the shelf's pages list: favorites on a `Saved` shelf, everything on a `Catalog` one.
    pub fn shape(&self, shelf: Shelf) -> Shape {
        self.held(shelf).shape
    }

    /// How many rows a part will have once it has all arrived, when the provider said on the
    /// first page. Nothing while nothing is known and nothing once the part has arrived, since
    /// the rows kept are then the count and the provider's total may have counted unplayable ones.
    pub fn expected(&self, shelf: Shelf, part: LibraryPart) -> Option<usize> {
        self.held(shelf).expected.get(&part).copied()
    }

    pub fn part_failed(&self, shelf: Shelf, part: LibraryPart) -> bool {
        self.part_problem(shelf, part).is_some()
    }

    /// Why a part of a shelf is empty, when it failed rather than arrived empty.
    pub fn part_problem(&self, shelf: Shelf, part: LibraryPart) -> Option<&str> {
        self.held(shelf)
            .ready()?
            .problems
            .iter()
            .find(|problem| problem.part == part)
            .map(|problem| problem.reason.as_str())
    }

    pub fn add_local_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.add_local_folder(path, cx));
    }

    pub fn add_local_folders(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.add_local_folders(paths, cx));
    }

    pub fn remove_local_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.remove_local_folder(&path, cx));
    }

    /// Reads the local folders again. `thorough` is the Rescan button: it distrusts the scan
    /// index, so nothing is taken on the word of a folder's modification time.
    pub fn rescan_local(&mut self, thorough: bool, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.rescan_local(thorough, cx));
    }

    pub fn hide_local_tracks(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let removed: bool = {
            let held = self.held_mut(Shelf::Local);
            let mut removed = false;
            if let Some(ready) = held.ready_mut() {
                let before = ready.tracks.len();
                ready
                    .tracks
                    .retain(|track| !track.id.as_ref().is_some_and(|id| ids.contains(id)));
                removed |= ready.tracks.len() != before;
            }
            let before = held.starred.tracks.len();
            held.starred
                .tracks
                .retain(|track| !track.id.as_ref().is_some_and(|id| ids.contains(id)));
            removed || held.starred.tracks.len() != before
        };
        if removed {
            self.hidden_local_tracks.extend(ids.iter().cloned());
            cx.emit(LibraryEvent::TracksHidden(ids.to_vec()));
            cx.notify();
        }
    }

    pub fn local_track_hidden(&self, id: &str) -> bool {
        self.hidden_local_tracks.contains(id)
    }

    pub fn loading(&self, shelf: Shelf, part: LibraryPart) -> bool {
        let held = self.held(shelf);
        matches!(held.state, LibraryState::Loading) || held.awaited.contains(&part)
    }

    pub fn saved(&self, track_id: &str) -> bool {
        self.favorites(track_id).is_some_and(|favorites| {
            favorites
                .tracks
                .iter()
                .any(|track| track.id.as_deref() == Some(track_id))
        })
    }

    fn favorites(&self, id: &str) -> Option<Favorites<'_>> {
        self.held(Shelf::of(id)).favorites()
    }

    pub fn pending(&self, track_id: &str) -> bool {
        self.pending.contains_key(track_id)
    }

    pub fn toggle(&mut self, track: Track, cx: &mut Context<Self>) {
        self.toggle_saved(track, cx);
    }

    pub fn save_tracks(&mut self, tracks: Vec<Track>, saved: bool, cx: &mut Context<Self>) {
        for track in tracks {
            let Some(id) = track.id.as_deref() else {
                continue;
            };
            if self.saved(id) == saved || self.pending(id) {
                continue;
            }
            self.toggle_saved(track, cx);
        }
    }

    /// Whether a track or album is in the shelf's own library, apart from its favorites. Only
    /// a `Capabilities::library` provider draws the distinction, and there the pages list the
    /// library, so this is what they list.
    pub fn in_library(&self, id: &str) -> bool {
        let state = &self.held(Shelf::of(id)).state;
        state
            .tracks()
            .iter()
            .any(|track| track.id.as_deref() == Some(id))
            || state.albums().iter().any(|album| album.id == id)
    }

    pub fn pending_library(&self, id: &str) -> bool {
        self.pending_library.contains_key(id)
    }

    pub fn set_track_in_library(
        &mut self,
        mut track: Track,
        present: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = track.id.clone() else {
            return;
        };
        let name = track.name.clone();
        track.stamp_added();
        self.set_in_library(id, MediaKind::Track, name, present, cx, move |ready| {
            ready.tracks.retain(|known| known.id != track.id);
            if present {
                ready.tracks.insert(0, track);
            }
        });
    }

    pub fn set_album_in_library(
        &mut self,
        mut album: Album,
        present: bool,
        cx: &mut Context<Self>,
    ) {
        let id = album.id.clone();
        let name = album.name.clone();
        album.stamp_added();
        self.set_in_library(id, MediaKind::Album, name, present, cx, move |ready| {
            ready.albums.retain(|known| known.id != album.id);
            if present {
                ready.albums.insert(0, album);
            }
        });
    }

    /// Asks the provider to put one item into its library or take it out, and applies
    /// `place` to the shelf's pages once it agrees, so they follow without a reload. A second
    /// ask about the same item while the first is out is dropped.
    fn set_in_library(
        &mut self,
        id: String,
        kind: MediaKind,
        name: String,
        present: bool,
        cx: &mut Context<Self>,
        place: impl FnOnce(&mut Ready) + 'static,
    ) {
        if self.pending_library.contains_key(&id) {
            return;
        }
        let shelf = Shelf::of(&id);
        let Some(client) = self.session.read(cx).client_of(shelf) else {
            return;
        };
        let asked = id.clone();
        let answered = id.clone();
        let io = self.io.clone();
        let task = cx.spawn(async move |this, cx| {
            let result =
                join(io.spawn(async move { client.set_in_library(kind, &asked, present).await }))
                    .await;
            this.update(cx, |this, cx| {
                this.pending_library.remove(&answered);
                match result {
                    Ok(()) => {
                        if let Some(ready) = this.held_mut(shelf).ready_mut() {
                            place(ready);
                        }
                        let done = match present {
                            true => "toast-library-added",
                            false => "toast-library-removed",
                        };
                        Toasts::show(Outcome::Done, done, cx);
                    }
                    Err(error) => {
                        log::warn!("library: cannot change the library: {error:#}");
                        let failed = match present {
                            true => "toast-library-add-failed",
                            false => "toast-library-remove-failed",
                        };
                        Toasts::linked(Outcome::Failed, failed, name, None, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.pending_library.insert(id, task);
    }

    pub fn saved_album(&self, album_id: &str) -> bool {
        self.favorites(album_id)
            .is_some_and(|favorites| favorites.albums.iter().any(|album| album.id == album_id))
    }

    pub fn pending_album(&self, album_id: &str) -> bool {
        self.pending_albums.contains_key(album_id)
    }

    pub fn toggle_album(&mut self, album: Album, cx: &mut Context<Self>) {
        self.toggle_saved(album, cx);
    }

    fn set_album_saved(&mut self, album: Album, saved: bool) {
        let Some(favorites) = self.held_mut(Shelf::of(&album.id)).favorites_mut() else {
            return;
        };
        let albums = favorites.albums;
        match saved {
            true if !albums.iter().any(|known| known.id == album.id) => albums.push(album),
            false => albums.retain(|known| known.id != album.id),
            _ => {}
        }
    }

    pub fn saved_artist(&self, artist_id: &str) -> bool {
        self.favorites(artist_id).is_some_and(|favorites| {
            favorites
                .artists
                .iter()
                .any(|artist| artist.id == artist_id)
        })
    }

    pub fn pending_artist(&self, artist_id: &str) -> bool {
        self.pending_artists.contains_key(artist_id)
    }

    /// A listed artist, from the shelf's pages or its favorites.
    pub fn artist(&self, id: &str) -> Option<&SavedArtist> {
        let held = self.held(Shelf::of(id));
        held.state
            .artists()
            .iter()
            .chain(&held.starred.artists)
            .find(|artist| artist.id == id)
    }

    pub fn toggle_artist(&mut self, artist: SavedArtist, cx: &mut Context<Self>) {
        self.toggle_saved(artist, cx);
    }

    fn set_artist_saved(&mut self, artist: SavedArtist, saved: bool) {
        let Some(favorites) = self.held_mut(Shelf::of(&artist.id)).favorites_mut() else {
            return;
        };
        let artists = favorites.artists;
        match saved {
            true if !artists.iter().any(|known| known.id == artist.id) => artists.push(artist),
            false => artists.retain(|known| known.id != artist.id),
            _ => {}
        }
    }

    pub fn create_playlist(
        &mut self,
        name: String,
        tracks: Vec<String>,
        shelf: Shelf,
        cx: &mut Context<Self>,
    ) {
        self.mutate_playlist(
            PlaylistMutation {
                action: "create playlist",
                done: "toast-playlist-created",
                name: None,
                target: None,
                invalidated: None,
                shelf,
            },
            move |client| async move {
                let id = client.create_playlist(&name).await?;
                for track in &tracks {
                    client.add_track_to_playlist(&id, track).await?;
                }
                let fetched = client.playlist(&id).await.map(|detail| detail.playlist);
                Ok(fetched.unwrap_or_else(|error| {
                    log::warn!("library: a new playlist is not readable yet: {error:#}");
                    Playlist {
                        id,
                        name,
                        owner: String::new(),
                        owner_id: String::new(),
                        owned: true,
                        collaborative: false,
                        blend: false,
                        public: false,
                        cover: None,
                        track_count: 0,
                        modified_at: None,
                    }
                }))
            },
            Self::insert_playlist,
            cx,
        );
    }

    pub fn rename_playlist(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        let renamed = (id.clone(), name.clone());
        self.mutate_playlist(
            PlaylistMutation {
                action: "rename playlist",
                done: "toast-playlist-renamed",
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: Some(id.clone()),
            },
            move |client| async move { client.rename_playlist(&id, &name).await },
            move |this, _, cx| {
                let (id, name) = renamed;
                this.amend_playlist(&id, |playlist| playlist.name = name, cx);
            },
            cx,
        );
    }

    pub fn set_playlist_public(&mut self, id: String, public: bool, cx: &mut Context<Self>) {
        let changed = id.clone();
        self.mutate_playlist(
            PlaylistMutation {
                action: "change playlist visibility",
                done: "toast-playlist-visibility",
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: Some(id.clone()),
            },
            move |client| async move { client.set_playlist_public(&id, public).await },
            move |this, _, cx| {
                this.amend_playlist(&changed, |playlist| playlist.public = public, cx);
            },
            cx,
        );
    }

    pub fn add_to_playlist(
        &mut self,
        playlist_id: String,
        track_id: String,
        cx: &mut Context<Self>,
    ) {
        self.add_tracks_to_playlist(playlist_id, vec![track_id], cx);
    }

    pub fn add_tracks_to_playlist(
        &mut self,
        playlist_id: String,
        track_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if track_ids.is_empty() {
            return;
        }
        let added = playlist_id.clone();
        let held = track_ids.clone();
        let added_count = track_ids.len() as u32;
        let name = self
            .playlist(&playlist_id)
            .map(|playlist| playlist.name.clone());
        let target = Some(Target::Playlist(SharedString::from(playlist_id.clone())));
        self.mutate_playlist(
            PlaylistMutation {
                action: "add track to playlist",
                done: "toast-track-added",
                name,
                target,
                shelf: Shelf::of(&playlist_id),
                invalidated: Some(playlist_id.clone()),
            },
            move |client| async move {
                for track_id in &track_ids {
                    client.add_track_to_playlist(&playlist_id, track_id).await?;
                }
                Ok(())
            },
            move |this, _, cx| {
                this.amend_playlist(&added, |playlist| playlist.track_count += added_count, cx);
                if let Some(ids) = this.contents.get_mut(&added) {
                    ids.extend(held);
                }
                cx.emit(LibraryEvent::TrackAdded { playlist: added });
            },
            cx,
        );
    }

    pub fn remove_from_playlist(
        &mut self,
        playlist_id: String,
        track_id: String,
        cx: &mut Context<Self>,
    ) {
        self.remove_tracks_from_playlist(playlist_id, vec![track_id], cx);
    }

    pub fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: String,
        track_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if track_ids.is_empty() {
            return;
        }
        let emptied = playlist_id.clone();
        let dropped = track_ids.clone();
        let dropped_count = track_ids.len() as u32;
        let name = self
            .playlist(&playlist_id)
            .map(|playlist| playlist.name.clone());
        let target = Some(Target::Playlist(SharedString::from(playlist_id.clone())));
        self.mutate_playlist(
            PlaylistMutation {
                action: "remove track from playlist",
                done: "toast-track-removed",
                name,
                target,
                shelf: Shelf::of(&playlist_id),
                invalidated: Some(playlist_id.clone()),
            },
            move |client| async move {
                for track_id in &track_ids {
                    client
                        .remove_track_from_playlist(&playlist_id, track_id)
                        .await?;
                }
                Ok(())
            },
            move |this, _, cx| {
                this.amend_playlist(
                    &emptied,
                    |playlist| {
                        playlist.track_count = playlist.track_count.saturating_sub(dropped_count)
                    },
                    cx,
                );
                if let Some(ids) = this.contents.get_mut(&emptied) {
                    for track in &dropped {
                        ids.remove(track);
                    }
                }
                for track in dropped {
                    cx.emit(LibraryEvent::TrackDropped {
                        playlist: emptied.clone(),
                        track,
                    });
                }
            },
            cx,
        );
    }

    pub fn delete_playlist(&mut self, id: String, cx: &mut Context<Self>) {
        let deleted = id.clone();
        self.mutate_playlist(
            PlaylistMutation {
                action: "delete playlist",
                done: "toast-playlist-deleted",
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: Some(id.clone()),
            },
            move |client| async move { client.delete_playlist(&id).await },
            move |this, _, cx| this.forget_playlist(&deleted, cx),
            cx,
        );
    }

    pub fn add_playlist_to_library(&mut self, playlist: Playlist, cx: &mut Context<Self>) {
        let id = playlist.id.clone();
        self.mutate_playlist(
            PlaylistMutation {
                action: "add playlist to library",
                done: "toast-playlist-added",
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: Some(id.clone()),
            },
            move |client| async move { client.add_playlist_to_library(&id).await },
            move |this, _, cx| this.insert_playlist(playlist, cx),
            cx,
        );
    }

    pub fn remove_playlist_from_library(&mut self, id: String, cx: &mut Context<Self>) {
        let removed = id.clone();
        self.mutate_playlist(
            PlaylistMutation {
                action: "remove playlist from library",
                done: "toast-playlist-removed",
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: Some(id.clone()),
            },
            move |client| async move { client.remove_playlist_from_library(&id).await },
            move |this, _, cx| this.forget_playlist(&removed, cx),
            cx,
        );
    }

    /// A listed album, from the shelf's pages or its favorites.
    pub fn album(&self, id: &str) -> Option<&Album> {
        let held = self.held(Shelf::of(id));
        held.state
            .albums()
            .iter()
            .chain(&held.starred.albums)
            .find(|album| album.id == id)
    }

    pub fn holds(&self, playlist_id: &str, track_id: &str) -> Option<bool> {
        Some(self.contents.get(playlist_id)?.contains(track_id))
    }

    fn adopt_mosaics(&mut self) -> Vec<(String, u32)> {
        let Some(ready) = self.held_mut(Shelf::Streaming).ready_mut() else {
            return Vec::new();
        };
        let playlists = &mut ready.playlists;

        let mut wanted = Vec::new();
        for playlist in playlists.iter_mut() {
            if playlist.cover.is_some() || (playlist.track_count as usize) < mosaic::TILES {
                continue;
            }
            match mosaic::cached(&playlist.id, playlist.track_count) {
                Some(cover) => playlist.cover = Some(cover),
                None => wanted.push((playlist.id.clone(), playlist.track_count)),
            }
        }

        wanted
    }

    fn build_mosaics(&mut self, cx: &mut Context<Self>) {
        let wanted = self.adopt_mosaics();
        let Some(client) = self.session.read(cx).client() else {
            return;
        };

        for (id, stamp) in wanted {
            if self.mosaics.contains_key(&id) {
                continue;
            }
            if self.is_editable(&id) {
                continue;
            }

            let io = self.io.clone();
            let client = client.clone();
            let asked = id.clone();
            let key = id.clone();
            let task = cx.spawn(async move |this, cx| {
                let covers = join(
                    io.spawn(async move { client.playlist_covers(&asked, mosaic::TILES).await }),
                )
                .await;
                match covers {
                    Ok(covers) => {
                        this.update(cx, |this, cx| this.paint_mosaic(id, stamp, covers, cx))
                            .ok();
                    }
                    Err(error) => log::warn!("library: cannot read playlist covers: {error:#}"),
                }
            });
            self.mosaics.insert(key, task);
        }
    }

    fn mosaic_stamp(&self, id: &str) -> Option<u32> {
        let playlist = self.playlist(id)?;

        (playlist.cover.is_none() && playlist.track_count as usize >= mosaic::TILES)
            .then_some(playlist.track_count)
    }

    fn is_editable(&self, id: &str) -> bool {
        self.playlist(id)
            .is_some_and(|playlist| playlist.owned || playlist.collaborative)
    }

    fn paint_mosaic(
        &mut self,
        id: String,
        stamp: u32,
        covers: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if covers.len() < mosaic::TILES {
            self.mosaics.remove(&id);
            return;
        }

        let io = self.io.clone();
        let http = cx.http_client();
        let built_for = id.clone();
        let key = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let built =
                join(io.spawn(async move { mosaic::build(http, &built_for, stamp, covers).await }))
                    .await;

            this.update(cx, |this, cx| {
                this.mosaics.remove(&id);
                match built {
                    Ok(cover) => {
                        this.set_playlist_cover(&id, cover);
                        cx.notify();
                    }
                    Err(error) => log::warn!("library: cannot build a mosaic: {error:#}"),
                }
            })
            .ok();
        });
        self.mosaics.insert(key, task);
    }

    fn set_playlist_cover(&mut self, id: &str, cover: String) {
        self.amend_playlist_quietly(id, |playlist| playlist.cover = Some(cover));
    }

    /// Reads the contents of every editable playlist on a shelf that is not known yet. A local
    /// playlist is always re-read, since its contents change without a mutation here.
    pub fn read_playlists(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        let wanted: Vec<String> = self
            .state(shelf)
            .playlists()
            .iter()
            .filter(|playlist| playlist.owned || playlist.collaborative)
            .map(|playlist| playlist.id.clone())
            .filter(|id| {
                (shelf.local() || !self.contents.contains_key(id)) && !self.reading.contains_key(id)
            })
            .collect();
        let Some(client) = self.session.read(cx).client_of(shelf) else {
            return;
        };
        self.read_contents(wanted, client, cx);
    }

    fn read_contents(
        &mut self,
        wanted: Vec<String>,
        client: Arc<dyn MusicApi>,
        cx: &mut Context<Self>,
    ) {
        for id in wanted {
            let io = self.io.clone();
            let client = client.clone();
            let key = id.clone();
            let asked = id.clone();
            let task = cx.spawn(async move |this, cx| {
                let listed =
                    join(io.spawn(async move { client.playlist_tracks(&asked).await })).await;

                this.update(cx, |this, cx| {
                    this.reading.remove(&key);
                    match listed {
                        Ok(tracks) => {
                            if let Some(stamp) = this.mosaic_stamp(&key) {
                                let covers = music::distinct_covers(&tracks, mosaic::TILES);
                                this.paint_mosaic(key.clone(), stamp, covers, cx);
                            }
                            let ids = tracks.into_iter().filter_map(|track| track.id).collect();
                            this.contents.insert(key, ids);
                            cx.notify();
                        }
                        Err(error) => {
                            log::warn!("library: cannot read a playlist: {error:#}")
                        }
                    }
                })
                .ok();
            });
            self.reading.insert(id.clone(), task);
        }
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.state(Shelf::of(id))
            .playlists()
            .iter()
            .find(|playlist| playlist.id == id)
    }

    fn playlists_mut(&mut self, id: &str) -> Option<&mut Vec<Playlist>> {
        self.held_mut(Shelf::of(id))
            .ready_mut()
            .map(|ready| &mut ready.playlists)
    }

    fn mutate_playlist<F, R, T, A>(
        &mut self,
        mutation_info: PlaylistMutation,
        mutation: F,
        on_done: A,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(Arc<dyn MusicApi>) -> R + Send + 'static,
        R: Future<Output = anyhow::Result<T>> + Send + 'static,
        T: Send + 'static,
        A: FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    {
        let PlaylistMutation {
            action,
            done,
            name,
            target,
            invalidated,
            shelf,
        } = mutation_info;
        if self.playlist_task.is_some() {
            log::warn!("library: cannot {action} while another change is running");
            Toasts::show(Outcome::Failed, "toast-playlist-busy", cx);
            return;
        }
        let Some(client) = self.session.read(cx).client_of(shelf) else {
            log::warn!("library: cannot {action} while signed out");
            Toasts::show(Outcome::Failed, "toast-playlist-signed-out", cx);
            return;
        };
        let catalog = invalidated
            .as_deref()
            .and_then(|id| self.session.read(cx).catalog(id));
        let io = self.io.clone();
        self.playlist_task = Some(cx.spawn(async move |this, cx| {
            let result = join(io.spawn(async move { mutation(client).await })).await;
            if result.is_ok()
                && let (Some(catalog), Some(id)) = (catalog, invalidated)
            {
                catalog.invalidate_playlist(&id).await;
            }
            this.update(cx, |this, cx| {
                this.playlist_task = None;
                match result {
                    Ok(outcome) => {
                        on_done(this, outcome, cx);
                        match name {
                            Some(name) => Toasts::linked(Outcome::Done, done, name, target, cx),
                            None => Toasts::show(Outcome::Done, done, cx),
                        }
                    }
                    Err(error) => {
                        log::warn!("library: cannot {action}: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-playlist-failed", cx);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn insert_playlist(&mut self, playlist: Playlist, cx: &mut Context<Self>) {
        let Some(playlists) = self.playlists_mut(&playlist.id) else {
            return;
        };
        playlists.retain(|known| known.id != playlist.id);
        playlists.push(playlist);
        cx.notify();
    }

    fn forget_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.emit(LibraryEvent::PlaylistGone(id.to_owned()));
        self.drop_playlist(id, cx);
    }

    fn drop_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(playlists) = self.playlists_mut(id) else {
            return;
        };
        playlists.retain(|playlist| playlist.id != id);
        cx.notify();
    }

    fn amend_playlist(
        &mut self,
        id: &str,
        amend: impl FnOnce(&mut Playlist),
        cx: &mut Context<Self>,
    ) {
        if self.amend_playlist_quietly(id, amend) {
            cx.notify();
        }
    }

    fn amend_playlist_quietly(&mut self, id: &str, amend: impl FnOnce(&mut Playlist)) -> bool {
        let Some(playlists) = self.playlists_mut(id) else {
            return false;
        };
        let Some(playlist) = playlists.iter_mut().find(|playlist| playlist.id == id) else {
            return false;
        };
        amend(playlist);
        true
    }

    fn set_saved(&mut self, track: Track, saved: bool) {
        let Some(id) = track.id.clone() else {
            return;
        };
        let Some(favorites) = self.held_mut(Shelf::of(&id)).favorites_mut() else {
            return;
        };
        let tracks = favorites.tracks;
        let same = |held: &Track| held.id.as_deref() == Some(id.as_str());
        match saved {
            true if !tracks.iter().any(same) => tracks.push(track),
            false => tracks.retain(|held| !same(held)),
            _ => {}
        }
    }

    pub fn refresh(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        if shelf == Shelf::Streaming {
            self.sync_sidebar(cx);
        }
        self.load(shelf, cx);
    }

    /// Loads a shelf from its provider. A `Saved` shape reads the `saved_*` lists; a `Catalog`
    /// shape reads the `all_*` lists and the `saved_*` ones beside them for hearts and filters.
    /// Rows a snapshot primed stay on screen while this runs, each part until its own rows
    /// arrive; a shelf with nothing primed goes back to loading, as a refresh does. The
    /// favorites stay up either way until the new ones land, so a Favorites only filter shows
    /// what it showed before rather than nothing.
    fn load(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let Some(client) = session.client_of(shelf) else {
            return;
        };
        let shape = session.shape_of(shelf);
        if shelf == Shelf::Streaming {
            self.playlist_task = None;
            self.pending.clear();
            self.pending_albums.clear();
            self.pending_artists.clear();
        }

        let held = self.held_mut(shelf);
        held.shape = shape;
        held.awaited = LibraryPart::ALL.to_vec();
        match held.stale.is_empty() {
            true => {
                held.state = LibraryState::Loading;
                held.expected.clear();
            }
            false => {
                for part in LibraryPart::ALL {
                    if !held.stale.contains(&part) {
                        held.expected.remove(&part);
                        empty(&mut held.state, part);
                    }
                }
            }
        }
        cx.notify();

        let tracks = client.clone();
        let playlists = client.clone();
        let albums = client.clone();
        let artists = client.clone();
        let mut tasks = vec![
            self.stream(
                shelf,
                LibraryPart::Tracks,
                async move {
                    match shape {
                        Shape::Saved => tracks.saved_tracks_paged().await,
                        Shape::Catalog => tracks.all_tracks_paged().await,
                    }
                },
                Landed::Tracks,
                cx,
            ),
            self.fetch(
                async move { playlists.playlists().await },
                move |this, loaded, cx| this.land(shelf, Landed::Playlists(loaded), cx),
                cx,
            ),
            self.stream(
                shelf,
                LibraryPart::Albums,
                async move {
                    match shape {
                        Shape::Saved => albums.saved_albums_paged().await,
                        Shape::Catalog => albums.all_albums_paged().await,
                    }
                },
                Landed::Albums,
                cx,
            ),
            self.stream(
                shelf,
                LibraryPart::Artists,
                async move {
                    match shape {
                        Shape::Saved => artists.saved_artists_paged().await,
                        Shape::Catalog => artists.all_artists_paged().await,
                    }
                },
                Landed::Artists,
                cx,
            ),
        ];
        if shape == Shape::Catalog {
            let tracks = client.clone();
            let albums = client.clone();
            tasks.extend([
                self.fetch(
                    async move { tracks.saved_tracks().await },
                    move |this, loaded, cx| this.land_starred(shelf, Landed::Tracks(loaded), cx),
                    cx,
                ),
                self.fetch(
                    async move { albums.saved_albums().await },
                    move |this, loaded, cx| this.land_starred(shelf, Landed::Albums(loaded), cx),
                    cx,
                ),
                self.fetch(
                    async move { client.saved_artists().await },
                    move |this, loaded, cx| this.land_starred(shelf, Landed::Artists(loaded), cx),
                    cx,
                ),
            ]);
        }
        self.held_mut(shelf).tasks = tasks;
    }

    fn fetch<T, R, A>(&self, work: R, apply: A, cx: &mut Context<Self>) -> Task<()>
    where
        R: Future<Output = anyhow::Result<T>> + Send + 'static,
        T: Send + 'static,
        A: FnOnce(&mut Self, anyhow::Result<T>, &mut Context<Self>) + 'static,
    {
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(work)).await;
            this.update(cx, |this, cx| apply(this, loaded, cx)).ok();
        })
    }

    /// Loads one part a page at a time. The shelf shows each page as it lands and learns how
    /// many rows to expect from the first, so a long library reads from its first hundred rows
    /// rather than its last. A page that fails ends the part with that failure.
    fn stream<T, R>(
        &self,
        shelf: Shelf,
        part: LibraryPart,
        work: R,
        wrap: fn(anyhow::Result<Vec<T>>) -> Landed,
        cx: &mut Context<Self>,
    ) -> Task<()>
    where
        R: Future<Output = anyhow::Result<Pages<T>>> + Send + 'static,
        T: Send + 'static,
    {
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let mut pages = match join(io.spawn(work)).await {
                Ok(pages) => pages,
                Err(error) => {
                    this.update(cx, |this, cx| this.land(shelf, wrap(Err(error)), cx))
                        .ok();
                    return;
                }
            };
            while let Some(page) = pages.recv().await {
                let landed = match page {
                    Ok(Page { total, items }) => {
                        let placed = this.update(cx, |this, cx| {
                            let held = this.held_mut(shelf);
                            if let Some(total) = total {
                                held.expected.insert(part, total);
                            }
                            shed(&mut held.state, &mut held.stale, part);
                            extend(&mut held.state, wrap(Ok(items)));
                            cx.notify();
                        });
                        if placed.is_err() {
                            return;
                        }
                        continue;
                    }
                    Err(error) => wrap(Err(error)),
                };
                this.update(cx, |this, cx| this.land(shelf, landed, cx))
                    .ok();
                return;
            }
            this.update(cx, |this, cx| {
                let held = this.held_mut(shelf);
                held.expected.remove(&part);
                shed(&mut held.state, &mut held.stale, part);
                settle(&mut held.state, &mut held.awaited, part, shelf.fatal());
                this.keep(shelf, Kind::Listed, part, cx);
                cx.notify();
            })
            .ok();
        })
    }

    fn land(&mut self, shelf: Shelf, landed: Landed, cx: &mut Context<Self>) {
        if !shelf.local() {
            match landed.reason() {
                Some(reason) => Network::failed(&reason, cx),
                None => Network::reached(cx),
            }
        }
        let part = landed.part();
        let arrived = landed.arrived();
        let held = self.held_mut(shelf);
        held.expected.remove(&part);
        if arrived {
            shed(&mut held.state, &mut held.stale, part);
        }
        place(&mut held.state, &mut held.awaited, landed, shelf.fatal());
        if arrived {
            self.keep(shelf, Kind::Listed, part, cx);
        }
        if part == LibraryPart::Playlists {
            self.read_playlists(shelf, cx);
            if shelf == Shelf::Streaming {
                self.build_mosaics(cx);
            }
        }
        cx.notify();
    }

    fn land_starred(&mut self, shelf: Shelf, landed: Landed, cx: &mut Context<Self>) {
        let part = landed.part();
        if star(self.held_mut(shelf), landed) {
            self.keep(shelf, Kind::Starred, part, cx);
        }
        cx.notify();
    }
}
