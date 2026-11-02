use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, Context, Entity, SharedString, Task};
use music::{
    Album, Catalog, ImportedPlaylist, MediaKind, MusicApi, Page, Pages, Playlist, SavedArtist,
    Shape, Track,
};
use tokio::sync::Semaphore;

use crate::snapshot::{Kind, Remembered, Snapshots};
use crate::{
    Io, Network, Outcome, Session, SessionEvent, SessionState, Target, Toast, Toasts, join, mosaic,
};

const FATAL: [LibraryPart; 3] = [
    LibraryPart::Tracks,
    LibraryPart::Playlists,
    LibraryPart::Albums,
];
const FATAL_LOCAL: [LibraryPart; 2] = [LibraryPart::Tracks, LibraryPart::Albums];
/// How often the pins are read again from a provider that does not say when they change.
const PIN_POLL: Duration = Duration::from_secs(60);

/// Lets a few playlists have their contents or mosaic covers read at a time. A library asks for
/// all of its playlists at once. Each read holds every track of its playlist until it is cut down,
/// and unchecked reads would spend the provider's request budget before the liked songs load.
static READS: Semaphore = Semaphore::const_new(4);

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
    /// provider. The part's real rows replace them once they have all arrived.
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
            Shape::Catalog(_) => Some(Favorites {
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
            Shape::Catalog(_) => Some(FavoritesMut {
                tracks: &mut self.starred.tracks,
                albums: &mut self.starred.albums,
                artists: &mut self.starred.artists,
            }),
        }
    }
}

#[derive(Default)]
struct Starred {
    tracks: Rows<Track>,
    albums: Rows<Album>,
    artists: Rows<SavedArtist>,
}

struct Favorites<'a> {
    tracks: &'a [Track],
    albums: &'a [Album],
    artists: &'a [SavedArtist],
}

struct FavoritesMut<'a> {
    tracks: &'a mut Rows<Track>,
    albums: &'a mut Rows<Album>,
    artists: &'a mut Rows<SavedArtist>,
}

/// The rows of one part on their way to a snapshot, shared with the shelf so the write can
/// happen off the main thread without copying the library.
enum Kept {
    Tracks(Arc<Vec<Track>>),
    Playlists(Arc<Vec<Playlist>>),
    Albums(Arc<Vec<Album>>),
    Artists(Arc<Vec<SavedArtist>>),
}

impl Kept {
    fn listed(ready: &Ready, part: LibraryPart) -> Self {
        match part {
            LibraryPart::Tracks => Self::Tracks(ready.tracks.share()),
            LibraryPart::Playlists => Self::Playlists(ready.playlists.share()),
            LibraryPart::Albums => Self::Albums(ready.albums.share()),
            LibraryPart::Artists => Self::Artists(ready.artists.share()),
        }
    }

    /// The favorites of one part. Playlists have none, so they answer an empty list that the
    /// caller never asks for.
    fn starred(starred: &Starred, part: LibraryPart) -> Self {
        match part {
            LibraryPart::Tracks => Self::Tracks(starred.tracks.share()),
            LibraryPart::Albums => Self::Albums(starred.albums.share()),
            LibraryPart::Artists => Self::Artists(starred.artists.share()),
            LibraryPart::Playlists => Self::Playlists(Arc::default()),
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

/// What a playlist change puts in: tracks already named, or every track of an album, which
/// the provider is asked for only once the change runs.
#[derive(Clone, Debug)]
pub enum Addition {
    Tracks(Vec<String>),
    Album(String),
}

impl Addition {
    /// Whether there is nothing to add. An album always counts as something, since its
    /// tracks are not known until it is read.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Tracks(ids) if ids.is_empty())
    }

    /// The track ids to add, reading an album's tracks from `client` first.
    async fn resolve(self, client: &Arc<dyn MusicApi>) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Tracks(ids) => Ok(ids),
            Self::Album(id) => Ok(client
                .album_tracks(&id)
                .await?
                .into_iter()
                .filter_map(|track| track.id)
                .collect()),
        }
    }
}

/// Where a track moved into a playlist is taken out of once the playlist has it.
#[derive(Clone, Debug)]
pub enum Leaving {
    Liked,
    Playlist(String),
}

/// What a move took its track out of, kept for the Undo. A favorite keeps the whole track,
/// since favoriting it again needs it.
#[derive(Clone)]
enum Left {
    Liked(Box<Track>),
    Playlist(String),
}

struct PlaylistMutation {
    action: &'static str,
    /// The toast shown once the provider agrees, or `None` when `on_done` reports it itself.
    done: Option<&'static str>,
    name: Option<String>,
    target: Option<Target>,
    /// The playlists whose cached pages the change makes stale. All of them sit on `shelf`.
    invalidated: Vec<String>,
    shelf: Shelf,
}

/// What the edit dialog does to a playlist's cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverEdit {
    Keep,
    /// Makes the picture at this path the cover.
    Replace(PathBuf),
    /// Puts the provider's own cover back.
    Remove,
}

/// The message that says what is wrong with a picked cover file.
pub fn cover_problem(unusable: music::cover::Unusable) -> &'static str {
    match unusable {
        music::cover::Unusable::Unopenable => "playlist-cover-unopenable",
        music::cover::Unusable::TooLarge => "playlist-cover-too-large",
        music::cover::Unusable::NotAPicture => "playlist-cover-not-a-picture",
    }
}

/// One thing a removal took out of the library, with what an Undo needs to put it back.
#[derive(Clone)]
enum Gone {
    SavedTrack(Track),
    SavedAlbum(Album),
    SavedArtist(SavedArtist),
    LibraryTrack(Track),
    LibraryAlbum(Album),
    Playlist(Playlist),
}

impl Gone {
    fn id(&self) -> Option<&str> {
        match self {
            Self::SavedTrack(track) | Self::LibraryTrack(track) => track.id.as_deref(),
            Self::SavedAlbum(album) | Self::LibraryAlbum(album) => Some(&album.id),
            Self::SavedArtist(artist) => Some(&artist.id),
            Self::Playlist(playlist) => Some(&playlist.id),
        }
    }

    fn title(&self) -> &str {
        match self {
            Self::SavedTrack(track) | Self::LibraryTrack(track) => &track.name,
            Self::SavedAlbum(album) | Self::LibraryAlbum(album) => &album.name,
            Self::SavedArtist(artist) => &artist.name,
            Self::Playlist(playlist) => &playlist.name,
        }
    }

    fn target(&self) -> Option<Target> {
        let id = SharedString::from(self.id()?.to_owned());
        Some(match self {
            Self::SavedTrack(_) | Self::LibraryTrack(_) => Target::Song(id),
            Self::SavedAlbum(_) | Self::LibraryAlbum(_) => Target::Album(id),
            Self::SavedArtist(_) => Target::Artist(id),
            Self::Playlist(_) => Target::Playlist(id),
        })
    }

    /// Which family's toast reports this removal.
    fn family(&self) -> Family {
        match self {
            Self::SavedTrack(_) => Family::SavedTracks,
            Self::SavedAlbum(_) => Family::SavedAlbums,
            Self::SavedArtist(_) => Family::SavedArtists,
            Self::LibraryTrack(_) => Family::LibraryTracks,
            Self::LibraryAlbum(_) => Family::LibraryAlbums,
            Self::Playlist(_) => Family::Playlists,
        }
    }
}

/// A removal and the epoch its shelf was in when the provider agreed to it.
#[derive(Clone)]
struct Removed {
    gone: Gone,
    epoch: u64,
}

/// The kinds of removal that share a toast. Removals of one family started in the same turn,
/// or while that family's toast is still up, are reported together with one Undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Family {
    SavedTracks,
    SavedAlbums,
    SavedArtists,
    LibraryTracks,
    LibraryAlbums,
    Playlists,
}

impl Family {
    /// The toast reporting `gone`: one thing by name, several by count.
    fn notice(self, gone: &[Removed]) -> Toast {
        if let [removed] = gone {
            let key = match self {
                Self::SavedTracks | Self::SavedAlbums | Self::SavedArtists => {
                    "toast-favorite-removed"
                }
                Self::LibraryTracks | Self::LibraryAlbums | Self::Playlists => {
                    "toast-library-removed-one"
                }
            };
            return Toast::new(Outcome::Done, key)
                .name(removed.gone.title().to_owned())
                .target(removed.gone.target());
        }
        let key = match self {
            Self::SavedTracks => "toast-favorite-songs-removed",
            Self::SavedAlbums => "toast-favorite-albums-removed",
            Self::SavedArtists => "toast-favorite-artists-removed",
            Self::LibraryTracks => "toast-library-songs-removed",
            Self::LibraryAlbums => "toast-library-albums-removed",
            Self::Playlists => "toast-playlists-removed",
        };
        Toast::new(Outcome::Done, key).count(gone.len())
    }
}

/// Removals of one family that one turn of the app started. They are reported together once
/// each has been answered.
struct Sweep {
    family: Family,
    /// Answers still to come, counting the end of the turn as one.
    left: usize,
    gone: Vec<Removed>,
}

/// The toast a family last showed and everything its Undo puts back.
struct Raised {
    toast: usize,
    gone: Vec<Removed>,
}

/// The toast a playlist's last row removals showed, and the tracks its Undo adds back.
struct RowsRaised {
    toast: usize,
    tracks: Vec<String>,
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

fn hold<T>(what: &str, result: anyhow::Result<Vec<T>>, into: &mut Rows<T>) -> bool {
    match result {
        Ok(rows) => {
            *into = rows.into();
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

/// Whether the provider lists `uri` as pinned. A provider may leave unpinned items out
/// entirely, so a missing item reads as unpinned.
fn holds_pin(items: &[music::PinTarget], uri: &str) -> bool {
    items.iter().any(|item| item.uri == uri && item.pinned)
}

impl Library {
    /// Favorites `item` or takes it out of the favorites, whichever it is not now. With
    /// `announce`, a removal is reported in a toast with an Undo once the provider agrees,
    /// which a deliberate removal wants and a heart that can simply be clicked again does not.
    fn toggle_saved<S: Savable>(&mut self, mut item: S, announce: bool, cx: &mut Context<Self>) {
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
        let sweep = (announce && !saved).then(|| self.gather(S::FAMILY, cx));

        let asked = id.clone();
        let answered = id.clone();
        let target = S::target(&id);
        let io = self.io.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = join(io.spawn(S::ask(client, asked, saved))).await;
            this.update(cx, |this, cx| {
                S::requests(this).remove(&answered);
                let gone = match result {
                    Ok(()) if saved => {
                        S::admit(this, item, cx);
                        None
                    }
                    Ok(()) => previous.map(S::gone),
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
                        None
                    }
                };
                if let Some(sweep) = sweep {
                    this.answer(sweep, gone, cx);
                }
                cx.notify();
            })
            .ok();
        });
        S::requests(self).insert(id, task);
        cx.notify();
    }

    /// Favorites `item` again for an Undo, through the same path a click takes. An item that
    /// is a favorite already or on its way there is left alone.
    fn resave<S: Savable>(&mut self, item: S, cx: &mut Context<Self>) {
        let Some(id) = item.id() else {
            return;
        };
        if S::saved_now(self, id).is_some() || S::requests(self).contains_key(id) {
            return;
        }
        self.toggle_saved(item, false, cx);
    }

    /// The sweep that removals of `family` started during this turn join, opened by the first
    /// of them. The end of the turn counts as one more answer, given once the turn is over, so
    /// a loop of removals is reported once rather than once per item.
    fn gather(&mut self, family: Family, cx: &mut Context<Self>) -> usize {
        if let Some(id) = self.gathering.get(&family)
            && let Some(sweep) = self.sweeps.get_mut(id)
        {
            sweep.left += 1;
            return *id;
        }
        let id = self.next_sweep;
        self.next_sweep += 1;
        self.sweeps.insert(
            id,
            Sweep {
                family,
                left: 2,
                gone: Vec::new(),
            },
        );
        self.gathering.insert(family, id);
        let library = cx.weak_entity();
        cx.defer(move |cx| {
            library
                .update(cx, |this, cx| {
                    if this.gathering.get(&family) == Some(&id) {
                        this.gathering.remove(&family);
                    }
                    this.answer(id, None, cx);
                })
                .ok();
        });
        id
    }

    /// Counts one answer toward sweep `id`, keeping what it removed, and reports the sweep once
    /// every answer is in.
    fn answer(&mut self, id: usize, gone: Option<Gone>, cx: &mut Context<Self>) {
        let Some(sweep) = self.sweeps.get_mut(&id) else {
            return;
        };
        if let Some(gone) = gone {
            let shelf = gone.id().map_or(Shelf::Streaming, Shelf::of);
            let epoch = self.epochs[shelf.slot()];
            sweep.gone.push(Removed { gone, epoch });
        }
        sweep.left = sweep.left.saturating_sub(1);
        if sweep.left > 0 {
            return;
        }
        if let Some(sweep) = self.sweeps.remove(&id) {
            self.raise(sweep.family, sweep.gone, cx);
        }
    }

    /// Reports what a family's removals took, with an Undo that puts all of it back. A toast
    /// the family showed before and is still up takes these in too, rather than a second one
    /// the stack would drop as a duplicate along with its Undo.
    fn raise(&mut self, family: Family, gone: Vec<Removed>, cx: &mut Context<Self>) {
        if gone.is_empty() {
            return;
        }
        let up = self
            .raised
            .remove(&family)
            .filter(|raised| showing(raised.toast, cx));
        let (shown, mut all) = match up {
            Some(raised) => (Some(raised.toast), raised.gone),
            None => (None, Vec::new()),
        };
        for removed in gone {
            all.retain(|known| known.gone.id() != removed.gone.id());
            all.push(removed);
        }

        let library = cx.weak_entity();
        let back = all.clone();
        let toast = family.notice(&all).undo(move |cx| {
            library
                .update(cx, |this, cx| this.put_back(back.clone(), cx))
                .ok();
        });
        let toast = match shown {
            Some(id) => {
                Toasts::settle(id, toast, cx);
                id
            }
            None => Toasts::post(toast, cx),
        };
        self.raised.insert(family, Raised { toast, gone: all });
    }

    /// Puts back what an Undo reports removed, each through the path a plain add takes.
    /// Anything back already, on its way back, or removed under an account or folder set that
    /// has since changed is left alone.
    fn put_back(&mut self, gone: Vec<Removed>, cx: &mut Context<Self>) {
        let mut playlists = Vec::new();
        for Removed { gone, epoch } in gone {
            let Some(id) = gone.id().map(str::to_owned) else {
                continue;
            };
            if self.epochs[Shelf::of(&id).slot()] != epoch {
                continue;
            }
            match gone {
                Gone::SavedTrack(track) => self.resave(track, cx),
                Gone::SavedAlbum(album) => self.resave(album, cx),
                Gone::SavedArtist(artist) => self.resave(artist, cx),
                Gone::LibraryTrack(track) if !self.in_library(&id) => {
                    self.set_track_in_library(track, true, cx)
                }
                Gone::LibraryAlbum(album) if !self.in_library(&id) => {
                    self.set_album_in_library(album, true, cx)
                }
                Gone::Playlist(playlist) if self.playlist(&id).is_none() => {
                    playlists.push(playlist)
                }
                _ => {}
            }
        }
        self.follow_again(playlists, cx);
    }
}

trait Savable: Clone + Send + Sized + 'static {
    const TROUBLE: &'static str;
    /// The family whose toast reports this kind's removals.
    const FAMILY: Family;

    fn id(&self) -> Option<&str>;
    fn title(&self) -> &str;
    /// What an Undo needs to favorite this again.
    fn gone(self) -> Gone;
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
    const FAMILY: Family = Family::SavedTracks;

    fn gone(self) -> Gone {
        Gone::SavedTrack(self)
    }

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
        if !library.session.read(cx).shape_of(shelf).editable() {
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
    const FAMILY: Family = Family::SavedAlbums;

    fn gone(self) -> Gone {
        Gone::SavedAlbum(self)
    }

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
    const FAMILY: Family = Family::SavedArtists;

    fn gone(self) -> Gone {
        Gone::SavedArtist(self)
    }

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
    /// A playlist an Undo brought back into the library after `PlaylistGone` took it out.
    PlaylistBack(String),
    TrackAdded {
        playlist: String,
    },
    TrackDropped {
        playlist: String,
        track: String,
    },
    TracksHidden(Vec<String>),
    /// A playlist a failed change may have altered in part, or whose cover changed, so a page
    /// showing it loads it again.
    PlaylistChanged(String),
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
    pub tracks: Rows<Track>,
    pub playlists: Rows<Playlist>,
    pub albums: Rows<Album>,
    pub artists: Rows<SavedArtist>,
    pub problems: Vec<Problem>,
}

/// One list a shelf holds. A snapshot write shares it instead of copying every row, and a
/// change made while such a write still holds the list copies it once first.
pub struct Rows<T>(Arc<Vec<T>>);

impl<T> Rows<T> {
    /// The rows as they stand, shared rather than copied.
    fn share(&self) -> Arc<Vec<T>> {
        Arc::clone(&self.0)
    }

    /// Empties the list. Rows a snapshot write still holds are left to it rather than copied.
    fn clear(&mut self) {
        match Arc::get_mut(&mut self.0) {
            Some(rows) => rows.clear(),
            None => self.0 = Arc::default(),
        }
    }
}

impl<T> Default for Rows<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<T> From<Vec<T>> for Rows<T> {
    fn from(rows: Vec<T>) -> Self {
        Self(Arc::new(rows))
    }
}

impl<T> Deref for Rows<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T: Clone> DerefMut for Rows<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        Arc::make_mut(&mut self.0)
    }
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
    /// Reads the streaming playlists again after one changed elsewhere, without the rest of
    /// the shelf.
    relisting: Option<Task<()>>,
    pin_targets_task: Option<Task<()>>,
    pin_task: Option<Task<()>>,
    pin_targets: Option<Vec<music::PinTarget>>,
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
    /// Whether each shelf is done reading the last run's snapshot, found or not.
    primed: [bool; 2],
    /// Counts the accounts and folder sets each shelf has had, so an Undo made under one never
    /// acts on the next.
    epochs: [u64; 2],
    sweeps: HashMap<usize, Sweep>,
    /// The sweep each family is gathering this turn's removals into.
    gathering: HashMap<Family, usize>,
    next_sweep: usize,
    raised: HashMap<Family, Raised>,
    /// The row removals whose toast is up, by playlist id.
    raised_rows: HashMap<String, RowsRaised>,
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
                this.epochs[Shelf::Streaming.slot()] += 1;
                this.pin_task = None;
                if !session.read(cx).authenticated() {
                    this.pin_targets_task = None;
                    this.pin_targets = None;
                    this.held_mut(Shelf::Streaming).clear();
                    cx.notify();
                    return;
                }
                this.playlist_task = None;
                this.relisting = None;
                this.pending.clear();
                this.pending_albums.clear();
                this.pending_artists.clear();
                let favorite = |family: &Family| {
                    matches!(
                        family,
                        Family::SavedTracks | Family::SavedAlbums | Family::SavedArtists
                    )
                };
                this.sweeps.retain(|_, sweep| !favorite(&sweep.family));
                this.gathering.retain(|family, _| !favorite(family));
                this.pin_targets = None;
                this.sync_pin_targets(cx);
                this.prime(Shelf::Streaming, cx);
                this.load(Shelf::Streaming, cx);
            }
            SessionEvent::SignedOut => {
                this.epochs[Shelf::Streaming.slot()] += 1;
                this.sweeps.clear();
                this.gathering.clear();
                this.raised.clear();
                this.raised_rows.clear();
                this.forget(Shelf::Streaming, cx);
                this.pin_task = None;
                this.pin_targets_task = None;
                this.pin_targets = None;
                this.contents.clear();
                this.reading.clear();
                this.mosaics.clear();
                this.playlist_task = None;
                this.relisting = None;
                this.pending.clear();
                this.pending_albums.clear();
                this.pending_artists.clear();
                this.pending_library.clear();
                this.held_mut(Shelf::Streaming).clear();
                cx.notify();
            }
            // What was loaded stays up, read through the downloads filter.
            SessionEvent::WentOffline => {}
            SessionEvent::Reconnected => this.refresh(Shelf::Streaming, cx),
            SessionEvent::LocalChanged => {
                this.epochs[Shelf::Local.slot()] += 1;
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
        // An account kept offline never signs in, whether at launch or after a switch, so its shelf
        // starts from what the last run kept the moment the session settles there.
        cx.observe(&session, |this, session, cx| {
            let offline = matches!(session.read(cx).state(), SessionState::Offline(_));
            if offline && matches!(this.held(Shelf::Streaming).state, LibraryState::Empty) {
                this.prime(Shelf::Streaming, cx);
            }
        })
        .detach();

        let mut library = Self {
            shelves: [Held::empty(), Held::empty()],
            session,
            io,
            playlist_task: None,
            relisting: None,
            pin_targets_task: None,
            pin_task: None,
            pin_targets: None,
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
            primed: [true, true],
            epochs: [0, 0],
            sweeps: HashMap::new(),
            gathering: HashMap::new(),
            next_sweep: 0,
            raised: HashMap::new(),
            raised_rows: HashMap::new(),
        };
        library.held_mut(Shelf::Streaming).state = LibraryState::Loading;
        library.held_mut(Shelf::Local).shape = Shape::Catalog(Catalog::Fixed);
        library.prime(Shelf::Streaming, cx);
        match library.session.read(cx).client_of(Shelf::Local).is_some() {
            true => library.load(Shelf::Local, cx),
            false => library.prime(Shelf::Local, cx),
        }
        library
    }

    /// Puts the last run's rows on a shelf while this run's are still on their way, so a launch
    /// shows a library before the provider has answered. Every part stays awaited, so the page
    /// still reads as loading, and each part's primed rows stay until the provider has sent all
    /// of that part.
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
        self.primed[shelf.slot()] = false;
        self.priming[shelf.slot()] = Some(cx.spawn(async move |this, cx| {
            let remembered = io
                .spawn_blocking(move || snapshots.restore(provider))
                .await
                .ok()
                .flatten();
            this.update(cx, |this, cx| {
                this.primed[shelf.slot()] = true;
                cx.notify();
                let Some(remembered) = remembered else {
                    return;
                };
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
                    tracks: starred_tracks.into(),
                    albums: starred_albums.into(),
                    artists: starred_artists.into(),
                };
                held.state = LibraryState::Ready(Ready {
                    tracks: tracks.into(),
                    playlists: playlists.into(),
                    albums: albums.into(),
                    artists: artists.into(),
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
        self.primed[shelf.slot()] = true;
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

    pub fn pin_targets(&self) -> Option<&[music::PinTarget]> {
        self.pin_targets.as_deref()
    }

    pub fn pin_pending(&self) -> bool {
        self.pin_task.is_some()
    }

    /// Changes the provider's own pin for `uri` and calls `done` with whether it stuck. A second
    /// change replaces the one in flight, so the last one wins and only its `done` runs. A pin
    /// the provider turns away for being past its limit or outside the library counts as stuck
    /// too, since Sonora keeps the pin itself and the sidebar has no limit of its own.
    pub fn set_pinned(
        &mut self,
        uri: String,
        pinned: bool,
        done: impl FnOnce(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self
            .pin_targets
            .as_ref()
            .is_some_and(|items| holds_pin(items, &uri) == pinned)
        {
            return;
        }
        let Some(client) = self.session.read(cx).client_of(Shelf::Streaming) else {
            return;
        };
        // Keep an older refresh from overwriting the result of this mutation.
        self.pin_targets_task = None;
        let io = self.io.clone();
        self.pin_task = Some(cx.spawn(async move |this, cx| {
            let result = join(io.spawn(async move {
                let result = client.set_pinned(&uri, pinned).await?;
                if result != music::PinOutcome::Updated {
                    return Ok((result, None));
                }
                let items = client.pin_targets().await?;
                anyhow::ensure!(
                    items
                        .as_ref()
                        .is_some_and(|items| holds_pin(items, &uri) == pinned),
                    "the provider did not confirm the updated library pin"
                );
                Ok((result, items))
            }))
            .await;
            this.update(cx, |this, cx| {
                this.pin_task = None;
                match result {
                    Ok((music::PinOutcome::Updated, items)) => {
                        this.pin_targets = items;
                        done(true, cx);
                    }
                    Ok((music::PinOutcome::LimitReached, _)) => {
                        log::debug!(
                            "library: the provider's pin limit is reached, pinning locally"
                        );
                        done(true, cx);
                    }
                    Ok((music::PinOutcome::Outside, _)) => {
                        log::debug!("library: the item is not in the library, pinning locally");
                        done(true, cx);
                    }
                    Err(error) => {
                        log::warn!("library: cannot update the library pin: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-library-pin-failed", cx);
                        done(false, cx);
                    }
                }
                // the change stopped the pin reads, which pick up again from here
                this.sync_pin_targets(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Reads the provider's pins. A provider that does not say when they change elsewhere is
    /// asked again every `PIN_POLL` for as long as the task lives.
    fn sync_pin_targets(&mut self, cx: &mut Context<Self>) {
        if self.pin_pending() {
            return;
        }
        self.pin_targets_task = None;
        let Some(client) = self.session.read(cx).client_of(Shelf::Streaming) else {
            return;
        };
        let live = client.connect().is_some();
        let io = self.io.clone();
        self.pin_targets_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let client = client.clone();
                let loaded = join(io.spawn(async move { client.pin_targets().await })).await;
                if this
                    .update(cx, |this, cx| match loaded {
                        Ok(items) if items != this.pin_targets => {
                            this.pin_targets = items;
                            cx.notify();
                        }
                        Ok(_) => {}
                        Err(error) => {
                            log::warn!("library: cannot read the provider's pins: {error:#}");
                            crate::noted(&error, cx);
                        }
                    })
                    .is_err()
                    || live
                {
                    break;
                }
                cx.background_executor().timer(PIN_POLL).await;
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

    /// Whether a part of `shelf` still has rows on their way. While offline mode keeps the shelf
    /// to its downloads the provider sends nothing, so only the last run's snapshot is waited on.
    pub fn waiting(&self, shelf: Shelf, part: LibraryPart, cx: &App) -> bool {
        match Network::downloads_only(shelf, cx) {
            true => !self.primed[shelf.slot()],
            false => self.loading(shelf, part),
        }
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

    /// Flips the heart on a track. Nothing is announced, since the heart is the way back.
    pub fn toggle(&mut self, track: Track, cx: &mut Context<Self>) {
        self.toggle_saved(track, false, cx);
    }

    /// Favorites every track not yet a favorite, or takes every favorite out. Taking them out
    /// is reported in one toast with an Undo.
    pub fn save_tracks(&mut self, tracks: Vec<Track>, saved: bool, cx: &mut Context<Self>) {
        for track in tracks {
            let Some(id) = track.id.as_deref() else {
                continue;
            };
            if self.saved(id) == saved || self.pending(id) {
                continue;
            }
            self.toggle_saved(track, !saved, cx);
        }
    }

    /// Whether a track or album is in the shelf's own library, apart from its favorites. Only
    /// a `Catalog::Editable` provider draws the distinction, and there the pages list the
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
        let gone = Gone::LibraryTrack(track.clone());
        track.stamp_added();
        self.set_in_library(MediaKind::Track, present, gone, cx, move |ready| {
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
        let gone = Gone::LibraryAlbum(album.clone());
        album.stamp_added();
        self.set_in_library(MediaKind::Album, present, gone, cx, move |ready| {
            ready.albums.retain(|known| known.id != album.id);
            if present {
                ready.albums.insert(0, album);
            }
        });
    }

    /// Asks the provider to put one item into its library or take it out, and applies
    /// `place` to the shelf's pages once it agrees, so they follow without a reload. A second
    /// ask about the same item while the first is out is dropped. A removal is reported with
    /// an Undo that puts `gone` back, and removals in the same turn share one toast.
    fn set_in_library(
        &mut self,
        kind: MediaKind,
        present: bool,
        gone: Gone,
        cx: &mut Context<Self>,
        place: impl FnOnce(&mut Ready) + 'static,
    ) {
        let Some(id) = gone.id().map(str::to_owned) else {
            return;
        };
        let name = gone.title().to_owned();
        if self.pending_library.contains_key(&id) {
            return;
        }
        let shelf = Shelf::of(&id);
        let Some(client) = self.session.read(cx).client_of(shelf) else {
            return;
        };
        let sweep = (!present).then(|| self.gather(gone.family(), cx));
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
                        match sweep {
                            Some(sweep) => this.answer(sweep, Some(gone), cx),
                            None => Toasts::show(Outcome::Done, "toast-library-added", cx),
                        }
                    }
                    Err(error) => {
                        if let Some(sweep) = sweep {
                            this.answer(sweep, None, cx);
                        }
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

    /// Favorites an album or takes it out of the favorites. A removal is reported with an Undo,
    /// and removals in the same turn share one toast.
    pub fn toggle_album(&mut self, album: Album, cx: &mut Context<Self>) {
        self.toggle_saved(album, true, cx);
    }

    /// Flips the heart on an album. Nothing is announced, since the heart is the way back.
    pub fn heart_album(&mut self, album: Album, cx: &mut Context<Self>) {
        self.toggle_saved(album, false, cx);
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
            .chain(held.starred.artists.iter())
            .find(|artist| artist.id == id)
    }

    /// Follows an artist or unfollows them. An unfollow is reported with an Undo, and
    /// unfollows in the same turn share one toast.
    pub fn toggle_artist(&mut self, artist: SavedArtist, cx: &mut Context<Self>) {
        self.toggle_saved(artist, true, cx);
    }

    /// Flips the heart on an artist. Nothing is announced, since the heart is the way back.
    pub fn heart_artist(&mut self, artist: SavedArtist, cx: &mut Context<Self>) {
        self.toggle_saved(artist, false, cx);
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

    /// Creates a playlist called `name` and fills it with `addition`, which may be empty.
    pub fn create_playlist(
        &mut self,
        name: String,
        addition: Addition,
        shelf: Shelf,
        cx: &mut Context<Self>,
    ) {
        self.mutate_playlist(
            PlaylistMutation {
                action: "create playlist",
                done: Some("toast-playlist-created"),
                name: None,
                target: None,
                invalidated: Vec::new(),
                shelf,
            },
            move |client| async move {
                let tracks = addition.resolve(&client).await?;
                let id = client.create_playlist(&name).await?;
                client.add_tracks_to_playlist(&id, &tracks).await?;
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

    /// Reads the M3U playlist at `path` into a new local playlist named after the file. The
    /// toast says how many of its songs were left out for not being in the library, and nothing
    /// is made when none of them is.
    pub fn import_playlist(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.mutate_playlist(
            PlaylistMutation {
                action: "import playlist",
                done: None,
                name: None,
                target: None,
                invalidated: Vec::new(),
                shelf: Shelf::Local,
            },
            move |client| async move { client.import_playlist(&path).await },
            |this, imported, cx| {
                let ImportedPlaylist {
                    playlist,
                    name,
                    skipped,
                } = imported;
                let Some(playlist) = playlist else {
                    Toasts::about(Outcome::Failed, "toast-playlist-import-empty", name, cx);
                    return;
                };
                let target = Some(Target::Playlist(SharedString::from(playlist.id.clone())));
                let key = match skipped {
                    0 => "toast-playlist-imported",
                    _ => "toast-playlist-imported-partly",
                };
                this.insert_playlist(playlist, cx);
                Toasts::post(
                    Toast::new(Outcome::Done, key)
                        .name(name)
                        .count(skipped)
                        .target(target),
                    cx,
                );
            },
            cx,
        );
    }

    /// Writes a local playlist to `path` as M3U8.
    pub fn export_playlist(&mut self, id: String, path: PathBuf, cx: &mut Context<Self>) {
        self.mutate_playlist(
            PlaylistMutation {
                action: "export playlist",
                done: Some("toast-playlist-exported"),
                name: None,
                target: None,
                invalidated: Vec::new(),
                shelf: Shelf::of(&id),
            },
            move |client| async move { client.export_playlist(&id, &path).await },
            |_, (), _| {},
            cx,
        );
    }

    pub fn rename_playlist(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        let renamed = (id.clone(), name.clone());
        self.mutate_playlist(
            PlaylistMutation {
                action: "rename playlist",
                done: Some("toast-playlist-renamed"),
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: vec![id.clone()],
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
                done: Some("toast-playlist-visibility"),
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: vec![id.clone()],
            },
            move |client| async move { client.set_playlist_public(&id, public).await },
            move |this, _, cx| {
                this.amend_playlist(&changed, |playlist| playlist.public = public, cx);
            },
            cx,
        );
    }

    /// Saves what the edit dialog changed about a playlist: a new name, a new cover or both, in
    /// one write. The playlist then shows the cover the provider reports, or the picked file
    /// while the provider still reports the old one.
    pub fn edit_playlist(
        &mut self,
        id: String,
        name: Option<String>,
        cover: CoverEdit,
        cx: &mut Context<Self>,
    ) {
        let picture = match cover {
            CoverEdit::Keep => {
                if let Some(name) = name {
                    self.rename_playlist(id, name, cx);
                }
                return;
            }
            CoverEdit::Replace(path) => Some(path),
            CoverEdit::Remove => None,
        };
        let before = self
            .playlist(&id)
            .and_then(|playlist| playlist.cover.clone());
        let changed = id.clone();
        let renamed = name.clone();
        let done = match (name.is_some(), picture.is_some()) {
            (true, _) => "toast-playlist-updated",
            (false, true) => "toast-playlist-cover-changed",
            (false, false) => "toast-playlist-cover-removed",
        };
        self.mutate_playlist(
            PlaylistMutation {
                action: "edit playlist",
                done: Some(done),
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: vec![id.clone()],
            },
            move |client| async move {
                let image = match &picture {
                    Some(path) => Some(tokio::fs::read(path).await.map_err(|error| {
                        anyhow::Error::new(music::cover::Unusable::Unopenable)
                            .context(format!("cannot read {}: {error}", path.display()))
                    })?),
                    None => None,
                };
                client.set_playlist_cover(&id, image.as_deref()).await?;
                if let Some(name) = &name {
                    client.rename_playlist(&id, name).await?;
                }
                let reported = match client.playlist(&id).await {
                    Ok(detail) => detail.playlist.cover,
                    Err(error) => {
                        log::warn!("library: cannot read a playlist after its cover: {error:#}");
                        None
                    }
                };
                Ok(match picture {
                    Some(path) if reported.is_none() || reported == before => {
                        Some(format!("file://{}", path.display()))
                    }
                    _ => reported,
                })
            },
            move |this, cover, cx| {
                let bare = cover.is_none();
                this.amend_playlist(
                    &changed,
                    |playlist| {
                        playlist.cover = cover;
                        if let Some(name) = renamed {
                            playlist.name = name;
                        }
                    },
                    cx,
                );
                if bare {
                    this.build_mosaics(cx);
                }
                cx.emit(LibraryEvent::PlaylistChanged(changed));
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
        self.add_tracks_to_playlist(playlist_id, Addition::Tracks(vec![track_id]), cx);
    }

    /// Adds `addition` to a playlist. An album leaves out the tracks the playlist is known to
    /// hold already, so adding it a second time adds only what it was missing. Named tracks
    /// always go in, since adding one again is how a listener asks for a second copy.
    pub fn add_tracks_to_playlist(
        &mut self,
        playlist_id: String,
        addition: Addition,
        cx: &mut Context<Self>,
    ) {
        if addition.is_empty() {
            return;
        }
        let added = playlist_id.clone();
        let known = match addition {
            Addition::Album(_) => self.contents.get(&playlist_id).cloned().unwrap_or_default(),
            Addition::Tracks(_) => HashSet::new(),
        };
        let name = self
            .playlist(&playlist_id)
            .map(|playlist| playlist.name.clone());
        let target = Some(Target::Playlist(SharedString::from(playlist_id.clone())));
        self.mutate_playlist(
            PlaylistMutation {
                action: "add track to playlist",
                done: Some("toast-track-added"),
                name,
                target,
                shelf: Shelf::of(&playlist_id),
                invalidated: vec![playlist_id.clone()],
            },
            move |client| async move {
                let mut track_ids = addition.resolve(&client).await?;
                track_ids.retain(|id| !known.contains(id));
                client
                    .add_tracks_to_playlist(&playlist_id, &track_ids)
                    .await?;
                Ok(track_ids)
            },
            move |this, track_ids, cx| this.note_added(&added, track_ids, cx),
            cx,
        );
    }

    /// Adds a track to a playlist and takes it out of `leaving`, reported in one toast whose
    /// Undo reverses both. Both go in the same change, so nothing leaves when the add fails.
    pub fn move_to_playlist(
        &mut self,
        playlist_id: String,
        track_id: String,
        leaving: Leaving,
        cx: &mut Context<Self>,
    ) {
        // The provider takes out every copy, so an Undo must not touch a track already there.
        let held = self.holds(&playlist_id, &track_id) == Some(true);
        let (origin, liked) = match leaving {
            Leaving::Playlist(origin) => (Some(origin), None),
            Leaving::Liked => (None, self.favorite(&track_id)),
        };
        let mut invalidated = vec![playlist_id.clone()];
        invalidated.extend(origin.clone());
        let (target, track, from, unlike) = (
            playlist_id.clone(),
            track_id.clone(),
            origin.clone(),
            liked.is_some(),
        );
        self.mutate_playlist(
            PlaylistMutation {
                action: "move track to playlist",
                done: None,
                name: None,
                target: None,
                shelf: Shelf::of(&playlist_id),
                invalidated,
            },
            move |client| async move {
                client
                    .add_tracks_to_playlist(&target, std::slice::from_ref(&track))
                    .await?;
                if let Some(from) = &from {
                    client.remove_track_from_playlist(from, &track).await?;
                }
                if unlike {
                    client.set_track_saved(&track, false).await?;
                }
                Ok(())
            },
            move |this, (), cx| {
                this.note_added(&playlist_id, vec![track_id.clone()], cx);
                let left = match (origin, liked) {
                    (Some(origin), _) => {
                        this.note_removed(&origin, vec![track_id.clone()], 1, cx);
                        Some(Left::Playlist(origin))
                    }
                    (None, Some(track)) => {
                        this.set_saved(track.clone(), false);
                        cx.notify();
                        Some(Left::Liked(Box::new(track)))
                    }
                    (None, None) => None,
                };
                this.raise_move(playlist_id, track_id, left, held, cx);
            },
            cx,
        );
    }

    /// The favorite a moved track leaves, kept whole so the Undo can favorite it again. Only a
    /// favorite the shelf lists and no request is changing can leave.
    fn favorite(&self, track_id: &str) -> Option<Track> {
        let track = self
            .state(Shelf::Streaming)
            .tracks()
            .iter()
            .find(|track| track.id.as_deref() == Some(track_id))
            .filter(|_| self.saved(track_id) && !self.pending(track_id))
            .cloned();
        if track.is_none() {
            log::warn!("library: cannot unlike a moved track the favorites do not list");
        }
        track
    }

    /// Reports a move in one toast. Its Undo takes the track back out of the playlist, unless
    /// the playlist `held` it already, and puts it back where it came from. A track that left
    /// nothing was only added, and is reported that way.
    fn raise_move(
        &mut self,
        playlist_id: String,
        track_id: String,
        left: Option<Left>,
        held: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .playlist(&playlist_id)
            .map(|playlist| playlist.name.clone())
        else {
            return;
        };
        let target = Some(Target::Playlist(SharedString::from(playlist_id.clone())));
        let Some(left) = left else {
            Toasts::linked(Outcome::Done, "toast-track-added", name, target, cx);
            return;
        };
        let library = cx.weak_entity();
        let epoch = self.epochs[Shelf::of(&playlist_id).slot()];
        let toast = Toast::new(Outcome::Done, "toast-track-moved")
            .name(name)
            .target(target)
            .undo(move |cx| {
                let (playlist_id, track_id, left) =
                    (playlist_id.clone(), track_id.clone(), left.clone());
                library
                    .update(cx, |this, cx| {
                        if this.epochs[Shelf::of(&playlist_id).slot()] == epoch {
                            this.unmove(playlist_id, track_id, left, held, cx);
                        }
                    })
                    .ok();
            });
        Toasts::post(toast, cx);
    }

    /// Reverses a move for its Undo in one change, since a second playlist change would be
    /// refused as busy. An origin that holds the track again is left alone.
    fn unmove(
        &mut self,
        playlist_id: String,
        track_id: String,
        left: Left,
        held: bool,
        cx: &mut Context<Self>,
    ) {
        let (origin, liked) = match left {
            Left::Playlist(origin) => {
                let again = self.holds(&origin, &track_id) == Some(true);
                ((!again).then_some(origin), None)
            }
            Left::Liked(track) => (None, Some(*track)),
        };
        if held && origin.is_none() && liked.is_none() {
            return;
        }
        let mut invalidated = vec![playlist_id.clone()];
        invalidated.extend(origin.clone());
        let (target, track, back, relike) = (
            playlist_id.clone(),
            track_id.clone(),
            origin.clone(),
            liked.is_some(),
        );
        self.mutate_playlist(
            PlaylistMutation {
                action: "undo a move",
                done: None,
                name: None,
                target: None,
                shelf: Shelf::of(&playlist_id),
                invalidated,
            },
            move |client| async move {
                if let Some(back) = &back {
                    client
                        .add_tracks_to_playlist(back, std::slice::from_ref(&track))
                        .await?;
                }
                if relike {
                    client.set_track_saved(&track, true).await?;
                }
                if !held {
                    client.remove_track_from_playlist(&target, &track).await?;
                }
                Ok(())
            },
            move |this, (), cx| {
                if let Some(origin) = origin {
                    this.note_added(&origin, vec![track_id.clone()], cx);
                }
                if let Some(track) = liked {
                    this.set_saved(track, true);
                    cx.notify();
                }
                if !held {
                    this.note_removed(&playlist_id, vec![track_id], 1, cx);
                }
            },
            cx,
        );
    }

    /// Records tracks a provider added to a playlist.
    fn note_added(&mut self, playlist_id: &str, track_ids: Vec<String>, cx: &mut Context<Self>) {
        let added_count = track_ids.len() as u32;
        self.amend_playlist(
            playlist_id,
            |playlist| playlist.track_count += added_count,
            cx,
        );
        if let Some(ids) = self.contents.get_mut(playlist_id) {
            ids.extend(track_ids);
        }
        cx.emit(LibraryEvent::TrackAdded {
            playlist: playlist_id.to_owned(),
        });
    }

    /// Records tracks a provider took out of a playlist, `dropped_count` rows in all.
    fn note_removed(
        &mut self,
        playlist_id: &str,
        track_ids: Vec<String>,
        dropped_count: u32,
        cx: &mut Context<Self>,
    ) {
        self.amend_playlist(
            playlist_id,
            |playlist| playlist.track_count = playlist.track_count.saturating_sub(dropped_count),
            cx,
        );
        if let Some(ids) = self.contents.get_mut(playlist_id) {
            for track in &track_ids {
                ids.remove(track);
            }
        }
        for track in track_ids {
            cx.emit(LibraryEvent::TrackDropped {
                playlist: playlist_id.to_owned(),
                track,
            });
        }
    }

    pub fn remove_from_playlist(
        &mut self,
        playlist_id: String,
        track_id: String,
        cx: &mut Context<Self>,
    ) {
        self.remove_tracks_from_playlist(playlist_id, vec![track_id], cx);
    }

    /// Takes tracks out of a playlist and reports it once the provider agrees, with an Undo that
    /// adds them back at the end, the way every provider adds. Every copy of a named track goes,
    /// the way the page drops every row of it, so `rows` names a track once for each copy the
    /// playlist holds and the count and the Undo follow it.
    pub fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: String,
        rows: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if rows.is_empty() {
            return;
        }
        let mut seen = HashSet::new();
        let track_ids: Vec<String> = rows
            .iter()
            .filter(|track| seen.insert(track.as_str()))
            .cloned()
            .collect();
        let emptied = playlist_id.clone();
        let gone = track_ids.clone();
        let dropped_count = rows.len() as u32;
        let name = self
            .playlist(&playlist_id)
            .map(|playlist| playlist.name.clone());
        let shelf = Shelf::of(&playlist_id);
        self.mutate_playlist(
            PlaylistMutation {
                action: "remove track from playlist",
                done: None,
                name: None,
                target: None,
                shelf,
                invalidated: vec![playlist_id.clone()],
            },
            move |client| async move {
                for track_id in &track_ids {
                    client
                        .remove_track_from_playlist(&playlist_id, track_id)
                        .await?;
                }
                Ok(())
            },
            move |this, (), cx| {
                this.raise_rows(&emptied, name, rows, cx);
                this.note_removed(&emptied, gone, dropped_count, cx);
            },
            cx,
        );
    }

    pub fn delete_playlist(&mut self, id: String, cx: &mut Context<Self>) {
        let deleted = id.clone();
        self.mutate_playlist(
            PlaylistMutation {
                action: "delete playlist",
                done: Some("toast-playlist-deleted"),
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: vec![id.clone()],
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
                done: Some("toast-playlist-added"),
                name: None,
                target: None,
                shelf: Shelf::of(&id),
                invalidated: vec![id.clone()],
            },
            move |client| async move { client.add_playlist_to_library(&id).await },
            move |this, _, cx| this.insert_playlist(playlist, cx),
            cx,
        );
    }

    pub fn remove_playlist_from_library(&mut self, id: String, cx: &mut Context<Self>) {
        self.remove_playlists_from_library(vec![id], cx);
    }

    /// Takes playlists out of the library in one change, and reports it with an Undo that puts
    /// them back. They all have to sit on the shelf of the first. A failure after some went
    /// through stops the rest, and only the ones that went are reported.
    pub fn remove_playlists_from_library(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(first) = ids.first() else {
            return;
        };
        let shelf = Shelf::of(first);
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|id| Shelf::of(id) == shelf)
            .collect();
        let known: Vec<Playlist> = ids
            .iter()
            .filter_map(|id| self.playlist(id).cloned())
            .collect();
        self.mutate_playlist(
            PlaylistMutation {
                action: "remove playlist from library",
                done: None,
                name: None,
                target: None,
                shelf,
                invalidated: ids.clone(),
            },
            move |client| async move {
                each(ids, |id| {
                    let client = client.clone();
                    async move { client.remove_playlist_from_library(&id).await }
                })
                .await
            },
            move |this, removed, cx| {
                for id in &removed {
                    this.forget_playlist(id, cx);
                }
                let epoch = this.epochs[shelf.slot()];
                let gone: Vec<Removed> = known
                    .into_iter()
                    .filter(|playlist| removed.contains(&playlist.id))
                    .map(|playlist| Removed {
                        gone: Gone::Playlist(playlist),
                        epoch,
                    })
                    .collect();
                match gone.is_empty() {
                    true => Toasts::show(Outcome::Done, "toast-playlist-removed", cx),
                    false => this.raise(Family::Playlists, gone, cx),
                }
            },
            cx,
        );
    }

    /// Follows playlists again for an Undo, in one change, and brings back the pins that left
    /// with them.
    fn follow_again(&mut self, playlists: Vec<Playlist>, cx: &mut Context<Self>) {
        let Some(first) = playlists.first() else {
            return;
        };
        let shelf = Shelf::of(&first.id);
        let ids: Vec<String> = playlists
            .iter()
            .map(|playlist| playlist.id.clone())
            .collect();
        self.mutate_playlist(
            PlaylistMutation {
                action: "add playlist to library",
                done: Some("toast-playlist-added"),
                name: None,
                target: None,
                shelf,
                invalidated: ids.clone(),
            },
            move |client| async move {
                each(ids, |id| {
                    let client = client.clone();
                    async move { client.add_playlist_to_library(&id).await }
                })
                .await
            },
            move |this, added, cx| {
                for playlist in playlists {
                    if !added.contains(&playlist.id) {
                        continue;
                    }
                    let id = playlist.id.clone();
                    this.insert_playlist(playlist, cx);
                    cx.emit(LibraryEvent::PlaylistBack(id));
                }
            },
            cx,
        );
    }

    /// Reports tracks taken out of a playlist. The toast still up for the same playlist is
    /// replaced with one counting both removals, and its Undo adds all of them back.
    fn raise_rows(
        &mut self,
        playlist_id: &str,
        name: Option<String>,
        removed: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let up = self
            .raised_rows
            .remove(playlist_id)
            .filter(|raised| showing(raised.toast, cx));
        let (shown, mut tracks) = match up {
            Some(raised) => (Some(raised.toast), raised.tracks),
            None => (None, Vec::new()),
        };
        tracks.extend(removed);
        let target = Some(Target::Playlist(SharedString::from(playlist_id.to_owned())));
        // A playlist no longer in the library has no name to fill `$name`, so it gets the
        // message that does without one.
        let mut toast = match (tracks.len(), name.is_some()) {
            (1, true) => Toast::new(Outcome::Done, "toast-track-removed"),
            (1, false) => Toast::new(Outcome::Done, "toast-track-removed-unnamed"),
            (count, true) => Toast::new(Outcome::Done, "toast-tracks-removed").count(count),
            (count, false) => {
                Toast::new(Outcome::Done, "toast-tracks-removed-unnamed").count(count)
            }
        }
        .target(target);
        if let Some(name) = name {
            toast = toast.name(name);
        }
        let library = cx.weak_entity();
        let playlist = playlist_id.to_owned();
        let epoch = self.epochs[Shelf::of(playlist_id).slot()];
        let back = tracks.clone();
        let toast = toast.undo(move |cx| {
            library
                .update(cx, |this, cx| {
                    this.put_tracks_back(playlist.clone(), back.clone(), epoch, cx)
                })
                .ok();
        });
        let toast = match shown {
            Some(id) => {
                Toasts::settle(id, toast, cx);
                id
            }
            None => Toasts::post(toast, cx),
        };
        self.raised_rows
            .insert(playlist_id.to_owned(), RowsRaised { toast, tracks });
    }

    /// Adds removed tracks back to a playlist for an Undo, skipping any the playlist holds
    /// again. Nothing happens once the account or folders behind the playlist have changed.
    fn put_tracks_back(
        &mut self,
        playlist_id: String,
        tracks: Vec<String>,
        epoch: u64,
        cx: &mut Context<Self>,
    ) {
        if self.epochs[Shelf::of(&playlist_id).slot()] != epoch {
            return;
        }
        self.raised_rows.remove(&playlist_id);
        let held = self.contents.get(&playlist_id);
        let tracks: Vec<String> = tracks
            .into_iter()
            .filter(|track| !held.is_some_and(|held| held.contains(track)))
            .collect();
        self.add_tracks_to_playlist(playlist_id, Addition::Tracks(tracks), cx);
    }

    /// A listed album, from the shelf's pages or its favorites.
    pub fn album(&self, id: &str) -> Option<&Album> {
        let held = self.held(Shelf::of(id));
        held.state
            .albums()
            .iter()
            .chain(held.starred.albums.iter())
            .find(|album| album.id == id)
    }

    pub fn holds(&self, playlist_id: &str, track_id: &str) -> Option<bool> {
        Some(self.contents.get(playlist_id)?.contains(track_id))
    }

    /// Gives each coverless streaming playlist the mosaic already on disk for it, and answers
    /// the ones still to build with their stamps. The list is written only when a mosaic is
    /// found, so a snapshot write holding it is not copied for nothing.
    fn adopt_mosaics(&mut self) -> Vec<(String, u32)> {
        let Some(ready) = self.held_mut(Shelf::Streaming).ready_mut() else {
            return Vec::new();
        };
        let playlists = &mut ready.playlists;

        let mut wanted = Vec::new();
        let mut adopted = Vec::new();
        for (index, playlist) in playlists.iter().enumerate() {
            if playlist.cover.is_some() || (playlist.track_count as usize) < mosaic::TILES {
                continue;
            }
            match mosaic::cached(&playlist.id, playlist.track_count) {
                Some(cover) => adopted.push((index, cover)),
                None => wanted.push((playlist.id.clone(), playlist.track_count)),
            }
        }
        for (index, cover) in adopted {
            playlists[index].cover = Some(cover);
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
                let covers = join(io.spawn(async move {
                    let _turn = READS.acquire().await;
                    client.playlist_covers(&asked, mosaic::TILES).await
                }))
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

    /// Whether the listener may change the playlist `id`: one they own or collaborate on.
    pub fn is_editable(&self, id: &str) -> bool {
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
                let listed = join(io.spawn(async move {
                    let _turn = READS.acquire().await;
                    let tracks = client.playlist_tracks(&asked).await?;
                    let covers = music::distinct_covers(&tracks, mosaic::TILES);
                    let ids: HashSet<String> =
                        tracks.into_iter().filter_map(|track| track.id).collect();
                    anyhow::Ok((ids, covers))
                }))
                .await;

                this.update(cx, |this, cx| {
                    this.reading.remove(&key);
                    match listed {
                        Ok((ids, covers)) => {
                            if let Some(stamp) = this.mosaic_stamp(&key) {
                                this.paint_mosaic(key.clone(), stamp, covers, cx);
                            }
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

    fn playlists_mut(&mut self, id: &str) -> Option<&mut Rows<Playlist>> {
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
            .first()
            .and_then(|id| self.session.read(cx).catalog(id));
        let io = self.io.clone();
        self.playlist_task = Some(cx.spawn(async move |this, cx| {
            let result = join(io.spawn(async move { mutation(client).await })).await;
            // A change that failed part way may still have landed in part.
            if let Some(catalog) = catalog {
                for id in &invalidated {
                    catalog.invalidate_playlist(id).await;
                }
            }
            this.update(cx, |this, cx| {
                this.playlist_task = None;
                match result {
                    Ok(outcome) => {
                        on_done(this, outcome, cx);
                        match (done, name) {
                            (Some(done), Some(name)) => {
                                Toasts::linked(Outcome::Done, done, name, target, cx)
                            }
                            (Some(done), None) => Toasts::show(Outcome::Done, done, cx),
                            (None, _) => {}
                        }
                    }
                    Err(error) => {
                        log::warn!("library: cannot {action}: {error:#}");
                        let key = match music::cover::unusable(&error) {
                            Some(unusable) => cover_problem(unusable),
                            None => "toast-playlist-failed",
                        };
                        Toasts::show(Outcome::Failed, key, cx);
                        for id in invalidated {
                            cx.emit(LibraryEvent::PlaylistChanged(id));
                        }
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

    /// Forgets a playlist on the shelf and in the pin snapshot, notifying listeners of its removal.
    fn forget_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(targets) = self.pin_targets.as_mut() {
            targets.retain(|target| {
                target.kind != music::PinTargetKind::Playlist
                    || target.uri.rsplit_once(':').map(|(_, id)| id) != Some(id)
            });
        }
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

    /// Shows a like, or the taking back of one, made in another of the account's apps, without
    /// asking the provider again. `track` comes with a like; an unlike needs only `id`. A change
    /// still on its way from this app wins.
    pub fn liked_elsewhere(&mut self, id: &str, track: Option<Track>, cx: &mut Context<Self>) {
        if self.pending.contains_key(id) {
            return;
        }
        match track {
            Some(_) if self.saved(id) => return,
            Some(mut track) => {
                track.added_at = Some(stamp());
                self.set_saved(track, true);
            }
            None => {
                let Some(track) = <Track as Savable>::saved_now(self, id) else {
                    return;
                };
                self.set_saved(track, false);
            }
        }
        cx.notify();
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

    /// Drops what is cached for the playlists another client changed, so their pages read
    /// again. A change to the library's own lists reloads the streaming shelf, while a change
    /// inside a playlist only reads the playlist list again for its name, cover and count.
    pub(crate) fn changed_elsewhere(
        &mut self,
        playlists: HashSet<String>,
        listed: bool,
        cx: &mut Context<Self>,
    ) {
        for id in playlists {
            self.reading.remove(&id);
            self.contents.remove(&id);
            cx.emit(LibraryEvent::PlaylistChanged(id));
        }
        match listed {
            true => self.refresh(Shelf::Streaming, cx),
            false => {
                // a change that only names a playlist may still have moved the pins
                self.sync_pin_targets(cx);
                self.relist(cx);
            }
        }
    }

    /// Reads the streaming shelf's playlists again and leaves its other parts as they are. A
    /// shelf still loading brings its playlists anyway, so nothing more is asked of it.
    fn relist(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client_of(Shelf::Streaming) else {
            return;
        };
        let held = self.held_mut(Shelf::Streaming);
        if !matches!(held.state, LibraryState::Ready(_))
            || held.awaited.contains(&LibraryPart::Playlists)
        {
            return;
        }
        held.stale.insert(LibraryPart::Playlists);
        self.relisting = Some(self.io.load(
            async move { client.playlists().await },
            |this, loaded, cx| {
                this.relisting = None;
                // a failed read leaves the playlists on screen as they were, with no problem
                // noted that only a full refresh would clear
                match loaded {
                    Ok(playlists) => {
                        this.land(Shelf::Streaming, Landed::Playlists(Ok(playlists)), cx)
                    }
                    Err(error) => {
                        log::warn!("library: cannot read the changed playlists: {error:#}");
                        this.held_mut(Shelf::Streaming)
                            .stale
                            .remove(&LibraryPart::Playlists);
                    }
                }
            },
            cx,
        ));
    }

    /// Loads a shelf again from its provider. A shelf that is showing rows keeps them, each part
    /// until all of its new rows have arrived, and its old problems are dropped so the fresh rows
    /// can be snapshotted.
    pub fn refresh(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        if shelf == Shelf::Streaming {
            self.sync_pin_targets(cx);
        }
        let held = self.held_mut(shelf);
        if let LibraryState::Ready(ready) = &mut held.state {
            ready.problems.clear();
            held.stale = LibraryPart::ALL.into_iter().collect();
        }
        self.load(shelf, cx);
    }

    /// Drops what a shelf shows and loads it again from its provider, so the page goes back to
    /// its loading state until the new rows land. This is the Refresh a user asks for, while
    /// `refresh` is the quiet one that keeps rows on screen. The local shelf is only as fresh as
    /// the last scan, so its folders are scanned again first and the shelf loads once that lands.
    pub fn reload(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        self.held_mut(shelf).stale.clear();
        match shelf {
            Shelf::Streaming => self.sync_pin_targets(cx),
            Shelf::Local if !self.session.read(cx).local_paths().is_empty() => {
                self.rescan_local(false, cx);
                return;
            }
            Shelf::Local => {}
        }
        self.load(shelf, cx);
    }

    /// Loads a shelf from its provider. A `Saved` shape reads the `saved_*` lists; a `Catalog`
    /// shape reads the `all_*` lists and the `saved_*` ones beside them for hearts and filters.
    /// Rows a snapshot primed stay on screen while this runs, each part until all of its own rows
    /// arrive, and so do the rows a refresh finds on screen; a shelf with nothing on it goes back
    /// to loading. The favorites stay up either way until the new ones land, so a Favorites only
    /// filter shows what it showed before rather than nothing. Edits in flight carry on through a
    /// load; only a new sign-in drops them.
    fn load(&mut self, shelf: Shelf, cx: &mut Context<Self>) {
        // a full load lands the playlists too, and two of them would list each one twice
        if shelf == Shelf::Streaming {
            self.relisting = None;
        }
        let session = self.session.read(cx);
        let Some(client) = session.client_of(shelf) else {
            return;
        };
        let shape = session.shape_of(shelf);

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
                        Shape::Catalog(_) => tracks.all_tracks_paged().await,
                    }
                },
                Landed::Tracks,
                cx,
            ),
            self.io.load(
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
                        Shape::Catalog(_) => albums.all_albums_paged().await,
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
                        Shape::Catalog(_) => artists.all_artists_paged().await,
                    }
                },
                Landed::Artists,
                cx,
            ),
        ];
        if shape.is_catalog() {
            let tracks = client.clone();
            let albums = client.clone();
            tasks.extend([
                self.io.load(
                    async move { tracks.saved_tracks().await },
                    move |this, loaded, cx| this.land_starred(shelf, Landed::Tracks(loaded), cx),
                    cx,
                ),
                self.io.load(
                    async move { albums.saved_albums().await },
                    move |this, loaded, cx| this.land_starred(shelf, Landed::Albums(loaded), cx),
                    cx,
                ),
                self.io.load(
                    async move { client.saved_artists().await },
                    move |this, loaded, cx| this.land_starred(shelf, Landed::Artists(loaded), cx),
                    cx,
                ),
            ]);
        }
        self.held_mut(shelf).tasks = tasks;
    }

    /// Loads one part a page at a time. A part with nothing primed shows each page as it lands
    /// and learns how many rows to expect from the first, so a long library reads from its first
    /// hundred rows rather than its last. A part a snapshot primed holds its pages back and swaps
    /// them in together once the last one lands, so the snapshot never shrinks to one page. A
    /// page that fails ends the part with that failure, and a primed part keeps its snapshot.
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
            let mut held_back = Vec::new();
            while let Some(page) = pages.recv().await {
                let landed = match page {
                    Ok(Page { total, items }) => {
                        let placed = this.update(cx, |this, cx| {
                            let held = this.held_mut(shelf);
                            if held.stale.contains(&part) {
                                return Some(items);
                            }
                            if let Some(total) = total {
                                held.expected.insert(part, total);
                            }
                            extend(&mut held.state, wrap(Ok(items)));
                            cx.notify();
                            None
                        });
                        match placed {
                            Ok(Some(items)) => held_back.extend(items),
                            Ok(None) => {}
                            Err(_) => return,
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
                extend(&mut held.state, wrap(Ok(held_back)));
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
        let counts = |this: &Self| -> HashMap<String, u32> {
            this.state(shelf)
                .playlists()
                .iter()
                .map(|playlist| (playlist.id.clone(), playlist.track_count))
                .collect()
        };
        let before =
            (arrived && part == LibraryPart::Playlists && !shelf.local()).then(|| counts(self));
        let held = self.held_mut(shelf);
        held.expected.remove(&part);
        if arrived {
            shed(&mut held.state, &mut held.stale, part);
        }
        place(&mut held.state, &mut held.awaited, landed, shelf.fatal());
        if arrived {
            self.keep(shelf, Kind::Listed, part, cx);
        }
        // A streaming playlist's contents are never read again once known, so one whose count
        // moved, or that is gone, drops them here and the read below fetches them afresh.
        if let Some(before) = before {
            let fresh = counts(self);
            self.contents.retain(|id, _| {
                Shelf::of(id) != shelf
                    || (fresh.contains_key(id) && fresh.get(id) == before.get(id))
            });
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

/// Whether toast `id` is still on screen.
fn showing(id: usize, cx: &mut App) -> bool {
    let toasts = Toasts::entity(cx);
    toasts.read(cx).shown().iter().any(|toast| toast.id == id)
}

/// Runs `step` for each id in turn and answers the ids it went through for. A failure fails
/// the whole while nothing has gone through yet, and afterwards only stops the rest.
async fn each<F, R>(ids: Vec<String>, mut step: F) -> anyhow::Result<Vec<String>>
where
    F: FnMut(String) -> R,
    R: Future<Output = anyhow::Result<()>>,
{
    let mut done = Vec::new();
    for id in ids {
        match step(id.clone()).await {
            Ok(()) => done.push(id),
            Err(error) if done.is_empty() => return Err(error),
            Err(error) => {
                log::warn!("library: cannot finish a playlist change: {error:#}");
                break;
            }
        }
    }
    Ok(done)
}
