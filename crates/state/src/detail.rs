use std::sync::Arc;

use gpui::{Context, Entity, Task};
use i18n::t;
use music::{Album, AlbumDetail, ArtistRef, Contributor, Playlist, PlaylistDetail, Track};
use tokio::task::AbortHandle;

use crate::{Io, Library, LibraryEvent, Session, SessionEvent, join, mosaic};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    Album,
    Playlist,
}

enum Loaded {
    Album(Arc<AlbumDetail>),
    Playlist(Arc<PlaylistDetail>),
}

pub struct Header {
    pub kind: Collection,
    pub title: String,
    pub artist: Option<String>,
    pub artist_refs: Vec<ArtistRef>,
    pub owner: Option<Contributor>,
    pub release_date: Option<String>,
    pub meta: Vec<String>,
    pub cover: Option<String>,
}

pub struct Detail {
    id: Option<String>,
    header: Option<Header>,
    kind: Option<Collection>,
    album: Option<Album>,
    playlist: Option<Playlist>,
    tracks: Vec<Track>,
    loading: bool,
    loaded: bool,
    error: Option<String>,
    session: Entity<Session>,
    library: Entity<Library>,
    io: Io,
    task: Option<Task<()>>,
    request: Option<AbortHandle>,
    mosaic: Option<Task<()>>,
}

impl Detail {
    pub fn new(
        session: Entity<Session>,
        library: Entity<Library>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedOut => {
                if !this.id.as_deref().is_some_and(music::is_local_id) {
                    this.clear();
                    cx.notify();
                }
            }
            SessionEvent::SignedIn => {
                if let (Some(kind), Some(id)) = (
                    this.kind,
                    this.id.clone().filter(|id| !music::is_local_id(id)),
                ) {
                    this.clear();
                    match kind {
                        Collection::Album => this.open_album(&id, cx),
                        Collection::Playlist => this.open_playlist(&id, cx),
                    }
                }
            }
            SessionEvent::Reconnected => {}
            SessionEvent::LocalChanged => {
                if let (Some(kind), Some(id)) = (
                    this.kind,
                    this.id.clone().filter(|id| music::is_local_id(id)),
                ) {
                    this.clear();
                    match kind {
                        Collection::Album => this.open_album(&id, cx),
                        Collection::Playlist => this.open_playlist(&id, cx),
                    }
                }
            }
        })
        .detach();

        cx.subscribe(&library, |this, _, event, cx| match event {
            LibraryEvent::TrackAdded { playlist }
                if this.id.as_deref() == Some(playlist.as_str()) =>
            {
                this.load(Collection::Playlist, playlist.clone(), cx);
            }
            LibraryEvent::TrackDropped { playlist, track }
                if this.id.as_deref() == Some(playlist.as_str()) =>
            {
                this.tracks
                    .retain(|shown| shown.id.as_deref() != Some(track.as_str()));
                cx.notify();
            }
            _ => {}
        })
        .detach();

        cx.observe(&library, |this, library, cx| {
            let Some(id) = this.id.clone() else {
                return;
            };
            let Some(mut playlist) = library.read(cx).playlist(&id).cloned() else {
                return;
            };
            if playlist.cover.is_none() {
                playlist.cover = this.playlist.as_ref().and_then(|shown| shown.cover.clone());
            }
            if this.playlist.as_ref() == Some(&playlist) {
                return;
            }
            this.header = Some(playlist_header(&playlist));
            this.playlist = Some(playlist);
            cx.notify();
        })
        .detach();

        Self {
            id: None,
            header: None,
            kind: None,
            album: None,
            playlist: None,
            tracks: Vec::new(),
            loading: false,
            loaded: false,
            error: None,
            session,
            library,
            io,
            task: None,
            request: None,
            mosaic: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    pub fn album(&self) -> Option<&Album> {
        self.album.as_ref()
    }

    pub fn playlist(&self) -> Option<&Playlist> {
        self.playlist.as_ref()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn remove_from_playlist(&mut self, track_id: String, cx: &mut Context<Self>) {
        self.remove_tracks_from_playlist(vec![track_id], cx);
    }

    pub fn remove_tracks_from_playlist(&mut self, track_ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(playlist_id) = self.id.clone() else {
            log::warn!("detail: cannot remove a track without a playlist");
            return;
        };
        self.library.update(cx, |library, cx| {
            library.remove_tracks_from_playlist(playlist_id, track_ids, cx);
        });
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn open_album(&mut self, id: &str, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        let known = library
            .album(id)
            .or_else(|| library.local_album(id))
            .cloned();
        let header = known.as_ref().map(album_header);
        if self.open(Collection::Album, id, header, cx) && !self.loaded {
            self.album = known;
        }
    }

    pub fn open_playlist(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut known = self.library.read(cx).playlist(id).cloned();
        let header = known.as_ref().map(playlist_header);
        if !self.open(Collection::Playlist, id, header, cx) {
            return;
        }
        if self.loaded
            && let Some(known) = known.as_mut()
        {
            if known.cover.is_none() {
                known.cover = self
                    .playlist
                    .as_ref()
                    .and_then(|playlist| playlist.cover.clone());
            }
            self.header = Some(playlist_header(known));
        }
        self.playlist = known.or_else(|| self.playlist.take());
    }

    fn open(
        &mut self,
        kind: Collection,
        id: &str,
        known: Option<Header>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.shows(kind, id) {
            return false;
        }

        self.clear();
        self.id = Some(id.to_owned());
        self.kind = Some(kind);
        self.header = known;

        let Some(catalog) = self.session.read(cx).catalog(id) else {
            cx.notify();
            return true;
        };
        let cached = match kind {
            Collection::Album => catalog.peek_album(id).map(Loaded::Album),
            Collection::Playlist => catalog.peek_playlist(id).map(Loaded::Playlist),
        };
        if let Some(cached) = cached {
            self.adopt(cached, cx);
            cx.notify();
            return true;
        }

        self.load(kind, id.to_owned(), cx);
        true
    }

    fn load(&mut self, kind: Collection, id: String, cx: &mut Context<Self>) {
        let Some(catalog) = self.session.read(cx).catalog(&id) else {
            cx.notify();
            return;
        };

        self.loading = true;
        self.error = None;
        cx.notify();

        let request = self.io.spawn({
            let id = id.clone();
            async move {
                match kind {
                    Collection::Album => catalog.album(&id).await.map(Loaded::Album),
                    Collection::Playlist => catalog.playlist(&id).await.map(Loaded::Playlist),
                }
            }
        });
        self.request = Some(request.abort_handle());

        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;

            this.update(cx, |this, cx| {
                this.loading = false;
                this.request = None;
                match loaded {
                    Ok(detail) => this.adopt(detail, cx),
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn known_mosaic(&self, playlist: &Playlist, cx: &Context<Self>) -> Option<String> {
        self.library
            .read(cx)
            .playlist(&playlist.id)
            .and_then(|known| known.cover.clone())
            .or_else(|| mosaic::cached(&playlist.id, playlist.track_count))
    }

    fn paint_mosaic(&mut self, playlist: &Playlist, tracks: &[Track], cx: &mut Context<Self>) {
        let covers = music::distinct_covers(tracks, mosaic::TILES);
        if covers.len() < mosaic::TILES {
            return;
        }

        let id = playlist.id.clone();
        let stamp = playlist.track_count;
        let io = self.io.clone();
        let http = cx.http_client();
        self.mosaic = Some(cx.spawn(async move |this, cx| {
            let built =
                join(io.spawn(async move { mosaic::build(http, &id, stamp, covers).await })).await;

            this.update(cx, |this, cx| match built {
                Ok(cover) => {
                    if let Some(header) = this.header.as_mut() {
                        header.cover = Some(cover.clone());
                    }
                    if let Some(playlist) = this.playlist.as_mut() {
                        playlist.cover = Some(cover);
                    }
                    cx.notify();
                }
                Err(error) => log::warn!("detail: cannot build a mosaic: {error:#}"),
            })
            .ok();
        }));
    }

    fn shows(&self, kind: Collection, id: &str) -> bool {
        let same = self.kind == Some(kind) && self.id.as_deref() == Some(id);
        same && (self.loading || self.loaded)
    }

    fn adopt(&mut self, loaded: Loaded, cx: &mut Context<Self>) {
        match loaded {
            Loaded::Album(detail) => {
                self.header = Some(album_header(&detail.album));
                self.album = Some(detail.album.clone());
                self.tracks = detail.tracks.clone();
            }
            Loaded::Playlist(detail) => {
                let mut playlist = detail.playlist.clone();
                if playlist.cover.is_none() {
                    playlist.cover = self.known_mosaic(&playlist, cx);
                }
                if playlist.cover.is_none() {
                    self.paint_mosaic(&playlist, &detail.tracks, cx);
                }
                self.header = Some(playlist_header(&playlist));
                self.playlist = Some(playlist);
                self.tracks = detail.tracks.clone();
            }
        }
        self.loaded = true;
    }

    fn clear(&mut self) {
        self.task = None;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.mosaic = None;
        self.id = None;
        self.header = None;
        self.kind = None;
        self.album = None;
        self.playlist = None;
        self.tracks.clear();
        self.loading = false;
        self.loaded = false;
        self.error = None;
    }
}

fn album_header(album: &Album) -> Header {
    let mut parts = Vec::new();
    if album.track_count > 0 {
        parts.push(t!("count-songs", count = album.track_count).to_string());
    }

    Header {
        kind: Collection::Album,
        title: album.name.clone(),
        artist: Some(album.artists.clone()),
        artist_refs: album.artist_refs.clone(),
        owner: None,
        release_date: match album.release_date.is_empty() {
            true => (album.year > 0).then(|| album.year.to_string()),
            false => Some(album.release_date.clone()),
        },
        meta: parts,
        cover: album.cover_large.clone(),
    }
}

fn playlist_header(playlist: &Playlist) -> Header {
    let owner = match playlist.owner_id.is_empty() {
        true => None,
        false => Some(Contributor {
            id: playlist.owner_id.clone(),
            name: playlist.owner.clone(),
            avatar: None,
        }),
    };
    let mut parts = match owner.is_some() {
        true => Vec::new(),
        false => vec![playlist.owner.clone()],
    };
    if playlist.track_count > 0 {
        parts.push(t!("count-songs", count = playlist.track_count).to_string());
    }

    Header {
        kind: Collection::Playlist,
        title: playlist.name.clone(),
        artist: None,
        artist_refs: Vec::new(),
        owner,
        release_date: None,
        meta: parts,
        cover: playlist.cover.clone(),
    }
}
