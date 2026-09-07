use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{Context, Entity, SharedString, Task};
use music::{Album, MusicApi, Playlist, PlaylistEntry, SavedArtist, Track};

use crate::outline::{self, PlaylistRow};
use crate::{Io, Outcome, Session, SessionEvent, Target, Toasts, join, mosaic};

const PAGE_LIMIT: u32 = 10000;
const FATAL: [LibraryPart; 3] = [
    LibraryPart::Tracks,
    LibraryPart::Playlists,
    LibraryPart::Albums,
];
const FATAL_LOCAL: [LibraryPart; 2] = [LibraryPart::Tracks, LibraryPart::Albums];

enum Landed {
    Tracks(anyhow::Result<Vec<Track>>),
    Playlists(anyhow::Result<Vec<PlaylistEntry>>),
    Albums(anyhow::Result<Vec<Album>>),
    Artists(anyhow::Result<Vec<SavedArtist>>),
}

impl Landed {
    fn part(&self) -> LibraryPart {
        match self {
            Self::Tracks(_) => LibraryPart::Tracks,
            Self::Playlists(_) => LibraryPart::Playlists,
            Self::Albums(_) => LibraryPart::Albums,
            Self::Artists(_) => LibraryPart::Artists,
        }
    }
}

struct PlaylistMutation {
    action: &'static str,
    done: &'static str,
    name: Option<String>,
    target: Option<Target>,
    invalidated: Option<String>,
    local: bool,
}

fn place(
    state: &mut LibraryState,
    awaited: &mut Vec<LibraryPart>,
    landed: Landed,
    fatal: &[LibraryPart],
) {
    awaited.retain(|part| *part != landed.part());
    if !matches!(state, LibraryState::Ready { .. }) {
        *state = LibraryState::Ready {
            tracks: Vec::new(),
            playlists: Vec::new(),
            outline: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            problems: Vec::new(),
        };
    }

    let failure = {
        let LibraryState::Ready {
            tracks,
            playlists,
            outline,
            albums,
            artists,
            problems,
        } = state
        else {
            return;
        };
        let part = landed.part();
        match landed {
            Landed::Tracks(result) => *tracks = take(part, result, problems),
            Landed::Playlists(result) => {
                (*playlists, *outline) = outline::split(take(part, result, problems));
            }
            Landed::Albums(result) => *albums = take(part, result, problems),
            Landed::Artists(result) => *artists = take(part, result, problems),
        }
        if !awaited.is_empty() {
            return;
        }
        let reasons: Vec<&str> = fatal
            .iter()
            .filter_map(|part| problems.iter().find(|problem| problem.part == *part))
            .map(|problem| problem.reason.as_str())
            .collect();
        (reasons.len() == fatal.len()).then(|| reasons.join("\n"))
    };
    if let Some(reason) = failure {
        *state = LibraryState::Failed(reason);
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
        let session = self.session.read(cx);
        let picked = match music::is_local_id(&id) {
            true => session.local_client(),
            false => session.client(),
        };
        let Some(client) = picked else {
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
                if let Err(error) = result {
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
        library
            .favorites(id)
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

    fn saved_now(library: &Library, id: &str) -> Option<Self> {
        library.album(id).cloned()
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
    const TROUBLE: &'static str = "followed artist";

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
        library.artist(id).cloned()
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryPart {
    Tracks,
    Playlists,
    Albums,
    Artists,
}

impl LibraryPart {
    const ALL: [Self; 4] = [Self::Tracks, Self::Playlists, Self::Albums, Self::Artists];

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

pub enum LibraryState {
    Empty,
    Loading,
    Ready {
        tracks: Vec<Track>,
        playlists: Vec<Playlist>,
        outline: Vec<PlaylistRow>,
        albums: Vec<Album>,
        artists: Vec<SavedArtist>,
        problems: Vec<Problem>,
    },
    Failed(String),
}

impl gpui::EventEmitter<LibraryEvent> for Library {}

pub struct Library {
    state: LibraryState,
    local: LibraryState,
    local_favorites: Vec<Track>,
    local_favorites_loading: bool,
    awaited: Vec<LibraryPart>,
    local_awaited: Vec<LibraryPart>,
    session: Entity<Session>,
    io: Io,
    tasks: Vec<Task<()>>,
    local_tasks: Vec<Task<()>>,
    playlist_task: Option<Task<()>>,
    pending: HashMap<String, Task<()>>,
    pending_albums: HashMap<String, Task<()>>,
    pending_artists: HashMap<String, Task<()>>,
    contents: HashMap<String, HashSet<String>>,
    reading: HashMap<String, Task<()>>,
    mosaics: HashMap<String, Task<()>>,
}

impl Library {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, session, event, cx| match event {
            SessionEvent::SignedIn => {
                if !session.read(cx).authenticated() {
                    this.state = LibraryState::Empty;
                    cx.notify();
                    return;
                }
                let client = session.read(cx).client();
                if let Some(client) = client {
                    this.load(client, cx);
                }
            }
            SessionEvent::SignedOut => {
                this.contents.clear();
                this.reading.clear();
                this.mosaics.clear();
                this.tasks.clear();
                this.awaited.clear();
                this.playlist_task = None;
                this.pending.clear();
                this.pending_albums.clear();
                this.pending_artists.clear();
                this.state = LibraryState::Empty;
                cx.notify();
            }
            SessionEvent::Reconnected => {
                if matches!(this.state, LibraryState::Failed(_))
                    && let Some(client) = session.read(cx).client()
                {
                    this.load(client, cx);
                }
            }
            SessionEvent::LocalChanged => {
                let client = session.read(cx).local_client();
                match client {
                    Some(client) => this.load_local(client, cx),
                    None => {
                        this.local_tasks.clear();
                        this.local_awaited.clear();
                        this.local = LibraryState::Empty;
                        this.local_favorites.clear();
                        this.local_favorites_loading = false;
                        cx.notify();
                    }
                }
            }
        })
        .detach();

        let local_client = session.read(cx).local_client();

        let mut library = Self {
            state: LibraryState::Loading,
            local: LibraryState::Empty,
            local_favorites: Vec::new(),
            local_favorites_loading: false,
            awaited: Vec::new(),
            local_awaited: Vec::new(),
            session,
            io,
            tasks: Vec::new(),
            local_tasks: Vec::new(),
            playlist_task: None,
            pending: HashMap::new(),
            pending_albums: HashMap::new(),
            pending_artists: HashMap::new(),
            contents: HashMap::new(),
            reading: HashMap::new(),
            mosaics: HashMap::new(),
        };
        if let Some(client) = local_client {
            library.load_local(client, cx);
        }
        library
    }

    pub fn state(&self) -> &LibraryState {
        &self.state
    }

    pub fn local_state(&self) -> &LibraryState {
        &self.local
    }

    pub fn part_failed(&self, part: LibraryPart) -> bool {
        Self::failed_parts(&self.state).any(|failed| failed == part)
    }

    pub fn local_part_failed(&self, part: LibraryPart) -> bool {
        Self::failed_parts(&self.local).any(|failed| failed == part)
    }

    fn failed_parts(state: &LibraryState) -> impl Iterator<Item = LibraryPart> + '_ {
        let problems = match state {
            LibraryState::Ready { problems, .. } => problems.as_slice(),
            _ => &[],
        };
        problems.iter().map(|problem| problem.part)
    }

    pub fn rescan_local(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.choose_local_folder(path, cx));
    }

    pub fn forget_local(&mut self, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.clear_local_folder(cx));
    }

    pub fn loading(&self, part: LibraryPart) -> bool {
        matches!(self.state, LibraryState::Loading) || self.awaited.contains(&part)
    }

    pub fn local_loading(&self, part: LibraryPart) -> bool {
        matches!(self.local, LibraryState::Loading) || self.local_awaited.contains(&part)
    }

    pub fn local_favorites_loading(&self) -> bool {
        self.local_favorites_loading
    }

    pub fn saved(&self, track_id: &str) -> bool {
        self.favorites(track_id)
            .iter()
            .any(|track| track.id.as_deref() == Some(track_id))
    }

    pub fn local_favorites(&self) -> &[Track] {
        &self.local_favorites
    }

    fn favorites(&self, track_id: &str) -> &[Track] {
        if music::is_local_id(track_id) {
            return &self.local_favorites;
        }
        match &self.state {
            LibraryState::Ready { tracks, .. } => tracks.as_slice(),
            _ => &[],
        }
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

    pub fn saved_album(&self, album_id: &str) -> bool {
        self.album(album_id).is_some()
    }

    pub fn pending_album(&self, album_id: &str) -> bool {
        self.pending_albums.contains_key(album_id)
    }

    pub fn toggle_album(&mut self, album: Album, cx: &mut Context<Self>) {
        self.toggle_saved(album, cx);
    }

    fn set_album_saved(&mut self, album: Album, saved: bool) {
        let LibraryState::Ready { albums, .. } = &mut self.state else {
            return;
        };
        match saved {
            true if !albums.iter().any(|known| known.id == album.id) => albums.push(album),
            false => albums.retain(|known| known.id != album.id),
            _ => {}
        }
    }

    pub fn saved_artist(&self, artist_id: &str) -> bool {
        self.artist(artist_id).is_some()
    }

    pub fn pending_artist(&self, artist_id: &str) -> bool {
        self.pending_artists.contains_key(artist_id)
    }

    pub fn artist(&self, id: &str) -> Option<&SavedArtist> {
        let LibraryState::Ready { artists, .. } = &self.state else {
            return None;
        };
        artists.iter().find(|artist| artist.id == id)
    }

    pub fn toggle_artist(&mut self, artist: SavedArtist, cx: &mut Context<Self>) {
        self.toggle_saved(artist, cx);
    }

    fn set_artist_saved(&mut self, artist: SavedArtist, saved: bool) {
        let LibraryState::Ready { artists, .. } = &mut self.state else {
            return;
        };
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
        local: bool,
        cx: &mut Context<Self>,
    ) {
        self.mutate_playlist(
            PlaylistMutation {
                action: "create playlist",
                done: "toast-playlist-created",
                name: None,
                target: None,
                invalidated: None,
                local,
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
                local: music::is_local_id(&id),
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
                local: music::is_local_id(&id),
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
                local: music::is_local_id(&playlist_id),
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
                local: music::is_local_id(&playlist_id),
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
                local: music::is_local_id(&id),
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
                local: music::is_local_id(&id),
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
                local: music::is_local_id(&id),
                invalidated: Some(id.clone()),
            },
            move |client| async move { client.remove_playlist_from_library(&id).await },
            move |this, _, cx| this.forget_playlist(&removed, cx),
            cx,
        );
    }

    pub fn album(&self, id: &str) -> Option<&Album> {
        let LibraryState::Ready { albums, .. } = &self.state else {
            return None;
        };
        albums.iter().find(|album| album.id == id)
    }

    pub fn local_album(&self, id: &str) -> Option<&Album> {
        let LibraryState::Ready { albums, .. } = &self.local else {
            return None;
        };
        albums.iter().find(|album| album.id == id)
    }

    pub fn holds(&self, playlist_id: &str, track_id: &str) -> Option<bool> {
        Some(self.contents.get(playlist_id)?.contains(track_id))
    }

    fn adopt_mosaics(&mut self) -> Vec<(String, u32)> {
        let LibraryState::Ready { playlists, .. } = &mut self.state else {
            return Vec::new();
        };

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
        let LibraryState::Ready { playlists, .. } = &self.state else {
            return None;
        };
        let playlist = playlists.iter().find(|playlist| playlist.id == id)?;

        (playlist.cover.is_none() && playlist.track_count as usize >= mosaic::TILES)
            .then_some(playlist.track_count)
    }

    fn is_editable(&self, id: &str) -> bool {
        let LibraryState::Ready { playlists, .. } = &self.state else {
            return false;
        };

        playlists
            .iter()
            .find(|playlist| playlist.id == id)
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
        let LibraryState::Ready { playlists, .. } = &mut self.state else {
            return;
        };
        if let Some(playlist) = playlists.iter_mut().find(|playlist| playlist.id == id) {
            playlist.cover = Some(cover);
        }
    }

    pub fn read_local_playlists(&mut self, cx: &mut Context<Self>) {
        let LibraryState::Ready { playlists, .. } = &self.local else {
            return;
        };
        let wanted: Vec<String> = playlists
            .iter()
            .map(|playlist| playlist.id.clone())
            .filter(|id| !self.reading.contains_key(id))
            .collect();
        let Some(client) = self.session.read(cx).local_client() else {
            return;
        };
        self.read_contents(wanted, client, cx);
    }

    pub fn read_playlists(&mut self, cx: &mut Context<Self>) {
        let LibraryState::Ready { playlists, .. } = &self.state else {
            return;
        };
        let wanted: Vec<String> = playlists
            .iter()
            .filter(|playlist| playlist.owned || playlist.collaborative)
            .map(|playlist| playlist.id.clone())
            .filter(|id| !self.contents.contains_key(id) && !self.reading.contains_key(id))
            .collect();
        let Some(client) = self.session.read(cx).client() else {
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
        self.shelf(id).iter().find(|playlist| playlist.id == id)
    }

    fn shelf(&self, id: &str) -> &[Playlist] {
        let state = match music::is_local_id(id) {
            true => &self.local,
            false => &self.state,
        };
        match state {
            LibraryState::Ready { playlists, .. } => playlists.as_slice(),
            _ => &[],
        }
    }

    fn shelf_mut(&mut self, id: &str) -> Option<&mut Vec<Playlist>> {
        let state = match music::is_local_id(id) {
            true => &mut self.local,
            false => &mut self.state,
        };
        match state {
            LibraryState::Ready { playlists, .. } => Some(playlists),
            _ => None,
        }
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
            local,
        } = mutation_info;
        if self.playlist_task.is_some() {
            log::warn!("library: cannot {action} while another change is running");
            Toasts::show(Outcome::Failed, "toast-playlist-busy", cx);
            return;
        }
        let session = self.session.read(cx);
        let picked = match local {
            true => session.local_client(),
            false => session.client(),
        };
        let Some(client) = picked else {
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
        let Some(playlists) = self.shelf_mut(&playlist.id) else {
            return;
        };
        let id = playlist.id.clone();
        playlists.retain(|known| known.id != id);
        playlists.push(playlist);
        self.relist(&id);
        cx.notify();
    }

    fn forget_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.emit(LibraryEvent::PlaylistGone(id.to_owned()));
        self.drop_playlist(id, cx);
    }

    fn drop_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(playlists) = self.shelf_mut(id) else {
            return;
        };
        playlists.retain(|playlist| playlist.id != id);
        self.relist(id);
        cx.notify();
    }

    fn relist(&mut self, id: &str) {
        let state = match music::is_local_id(id) {
            true => &mut self.local,
            false => &mut self.state,
        };
        if let LibraryState::Ready {
            playlists, outline, ..
        } = state
        {
            outline::relist(playlists, outline);
        }
    }

    fn amend_playlist(
        &mut self,
        id: &str,
        amend: impl FnOnce(&mut Playlist),
        cx: &mut Context<Self>,
    ) {
        let Some(playlists) = self.shelf_mut(id) else {
            return;
        };
        let Some(playlist) = playlists.iter_mut().find(|playlist| playlist.id == id) else {
            return;
        };
        amend(playlist);
        cx.notify();
    }

    fn set_saved(&mut self, track: Track, saved: bool) {
        let local = track.id.as_deref().is_some_and(music::is_local_id);
        if local {
            let id = track.id.clone();
            match saved {
                true if !self.local_favorites.iter().any(|held| held.id == id) => {
                    self.local_favorites.insert(0, track)
                }
                false => self.local_favorites.retain(|held| held.id != id),
                _ => {}
            }
            return;
        }

        let LibraryState::Ready { tracks, .. } = &mut self.state else {
            return;
        };
        let id = track.id.as_deref();
        match saved {
            true if !tracks.iter().any(|saved| saved.id.as_deref() == id) => tracks.push(track),
            false => tracks.retain(|saved| saved.id.as_deref() != id),
            _ => {}
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let client = self.session.read(cx).client();
        if let Some(client) = client {
            self.load(client, cx);
        }
    }

    fn load(&mut self, client: Arc<dyn MusicApi>, cx: &mut Context<Self>) {
        self.playlist_task = None;
        self.pending.clear();
        self.pending_albums.clear();
        self.pending_artists.clear();
        self.state = LibraryState::Loading;
        self.awaited = LibraryPart::ALL.to_vec();
        cx.notify();

        let tracks = client.clone();
        let playlists = client.clone();
        let albums = client.clone();
        self.tasks = vec![
            self.fetch(
                async move { tracks.saved_tracks(PAGE_LIMIT).await },
                |this, loaded, cx| this.land(Landed::Tracks(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { playlists.playlists(PAGE_LIMIT).await },
                |this, loaded, cx| this.land(Landed::Playlists(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { albums.saved_albums(PAGE_LIMIT).await },
                |this, loaded, cx| this.land(Landed::Albums(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { client.saved_artists(PAGE_LIMIT).await },
                |this, loaded, cx| this.land(Landed::Artists(loaded), cx),
                cx,
            ),
        ];
    }

    fn load_local(&mut self, client: Arc<dyn MusicApi>, cx: &mut Context<Self>) {
        self.local = LibraryState::Loading;
        self.local_awaited = LibraryPart::ALL.to_vec();
        self.local_favorites_loading = true;
        cx.notify();

        let tracks = client.clone();
        let playlists = client.clone();
        let albums = client.clone();
        let artists = client.clone();
        self.local_tasks = vec![
            self.fetch(
                async move { tracks.all_tracks(PAGE_LIMIT).await },
                |this, loaded, cx| this.land_local(Landed::Tracks(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { playlists.playlists(PAGE_LIMIT).await },
                |this, loaded, cx| this.land_local(Landed::Playlists(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { albums.saved_albums(PAGE_LIMIT).await },
                |this, loaded, cx| this.land_local(Landed::Albums(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { artists.saved_artists(PAGE_LIMIT).await },
                |this, loaded, cx| this.land_local(Landed::Artists(loaded), cx),
                cx,
            ),
            self.fetch(
                async move { client.saved_tracks(PAGE_LIMIT).await },
                |this, loaded, cx| {
                    this.local_favorites = loaded.unwrap_or_else(|error| {
                        log::warn!("library: cannot load the local favorites: {error:#}");
                        Vec::new()
                    });
                    this.local_favorites_loading = false;
                    cx.notify();
                },
                cx,
            ),
        ];
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

    fn land(&mut self, landed: Landed, cx: &mut Context<Self>) {
        let part = landed.part();
        place(&mut self.state, &mut self.awaited, landed, &FATAL);
        if part == LibraryPart::Playlists {
            self.read_playlists(cx);
            self.build_mosaics(cx);
        }
        cx.notify();
    }

    fn land_local(&mut self, landed: Landed, cx: &mut Context<Self>) {
        let part = landed.part();
        place(
            &mut self.local,
            &mut self.local_awaited,
            landed,
            &FATAL_LOCAL,
        );
        if part == LibraryPart::Playlists {
            self.read_local_playlists(cx);
        }
        cx.notify();
    }
}
