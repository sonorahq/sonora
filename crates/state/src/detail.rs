use std::collections::HashSet;
use std::sync::Arc;

use gpui::{App, Context, Entity, Task};
use music::{
    Album, AlbumCatalogue, AlbumDetail, ArtistRef, Contributor, Playlist, PlaylistDetail,
    ReleaseType, SavedArtist, Track,
};
use tokio::task::AbortHandle;

use crate::{
    Io, Library, LibraryEvent, Network, Session, SessionEvent, SessionState, Sonora, join, mosaic,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// What kind of release an album is, `None` for a playlist.
    pub release: Option<ReleaseType>,
    pub title: String,
    pub artist: Option<String>,
    pub artist_refs: Vec<ArtistRef>,
    pub owner: Option<Contributor>,
    pub release_date: Option<String>,
    /// The owner's name when the provider gives no id to link it to.
    pub owner_name: Option<String>,
    /// Zero when the provider does not report a count.
    pub track_count: u32,
    pub cover: Option<String>,
}

pub struct Detail {
    id: Option<String>,
    header: Option<Header>,
    kind: Option<Collection>,
    album: Option<Album>,
    playlist: Option<Playlist>,
    tracks: Vec<Track>,
    continuation: Option<String>,
    also_like: Vec<Album>,
    similar: Vec<SavedArtist>,
    loading: bool,
    loading_more: bool,
    loaded: bool,
    filling: bool,
    error: Option<String>,
    session: Entity<Session>,
    library: Entity<Library>,
    io: Io,
    task: Option<Task<()>>,
    request: Option<AbortHandle>,
    fill: Option<Task<()>>,
    filling_request: Option<AbortHandle>,
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
            SessionEvent::Reconnected | SessionEvent::WentOffline => {}
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

        // A page that opened while the account was still signing back in waits for it. Once the
        // session settles anywhere but signed in, there is nothing coming to wait for.
        cx.observe(&session, |this, session, cx| {
            let restoring = matches!(session.read(cx).state(), SessionState::Restoring);
            if this.loading && this.task.is_none() && !restoring {
                this.loading = false;
                cx.notify();
            }
        })
        .detach();

        cx.subscribe(&library, |this, _, event, cx| match event {
            LibraryEvent::PlaylistGone(id) if this.id.as_deref() == Some(id.as_str()) => {
                this.clear();
                cx.notify();
            }
            LibraryEvent::TrackAdded { playlist } | LibraryEvent::PlaylistChanged(playlist)
                if this.id.as_deref() == Some(playlist.as_str()) =>
            {
                this.load(Collection::Playlist, playlist.clone(), false, cx);
            }
            LibraryEvent::TrackDropped { playlist, track }
                if this.id.as_deref() == Some(playlist.as_str()) =>
            {
                this.tracks
                    .retain(|shown| shown.id.as_deref() != Some(track.as_str()));
                cx.notify();
            }
            LibraryEvent::TracksHidden(ids) => {
                let before = this.tracks.len();
                this.tracks
                    .retain(|track| !track.id.as_ref().is_some_and(|id| ids.contains(id)));
                if this.tracks.len() != before {
                    cx.notify();
                }
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
            continuation: None,
            also_like: Vec::new(),
            similar: Vec::new(),
            loading: false,
            loading_more: false,
            loaded: false,
            filling: false,
            error: None,
            session,
            library,
            io,
            task: None,
            request: None,
            fill: None,
            filling_request: None,
            mosaic: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Whether the page is an album or a playlist, known from the moment it opens, before its
    /// header arrives.
    pub fn kind(&self) -> Option<Collection> {
        self.kind
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

    /// What the provider lists as related to the page's album, with more from the same
    /// artist first and similar artists' releases topping the rail up, filled in behind
    /// the tracks.
    pub fn also_like(&self) -> &[Album] {
        &self.also_like
    }

    /// The artists the provider lists as similar to the page's artist, filling the
    /// rail's artists tab.
    pub fn similar(&self) -> &[SavedArtist] {
        &self.similar
    }

    /// Whether the recommendations are still on their way, after the tracks have already
    /// put the page up.
    pub fn is_filling(&self) -> bool {
        self.filling
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Appends provider pages in sequence while keeping the first page immediately usable.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more {
            return;
        }
        let Some(continuation) = self.continuation.clone() else {
            return;
        };
        let Some(id) = self.id.as_deref() else {
            return;
        };
        let Some(catalog) = self.session.read(cx).catalog(id) else {
            return;
        };

        self.loading_more = true;
        cx.notify();

        let request = self
            .io
            .spawn(async move { catalog.playlist_continuation(&continuation).await });
        self.request = Some(request.abort_handle());
        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;
            this.update(cx, |this, cx| {
                this.loading_more = false;
                this.request = None;
                match loaded {
                    Ok((tracks, continuation)) => {
                        let offset = this.tracks.len() as u32;
                        this.tracks.extend(tracks.into_iter().enumerate().map(
                            |(index, mut track)| {
                                track.track_number = offset + index as u32 + 1;
                                track
                            },
                        ));
                        this.continuation = continuation;
                        if let Some(playlist) = this.playlist.as_mut()
                            && playlist.track_count < this.tracks.len() as u32
                        {
                            playlist.track_count = this.tracks.len() as u32;
                            this.header = Some(playlist_header(playlist));
                        }
                        this.load_more(cx);
                    }
                    Err(error) => log::warn!("detail: cannot load more playlist tracks: {error:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn remove_from_playlist(&mut self, track_id: String, cx: &mut Context<Self>) {
        self.remove_tracks_from_playlist(vec![track_id], cx);
    }

    /// Takes every copy of the named tracks out of the playlist on the page. The library is
    /// told about each row the page shows of them, so its count and Undo cover every copy.
    pub fn remove_tracks_from_playlist(&mut self, track_ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(playlist_id) = self.id.clone() else {
            log::warn!("detail: cannot remove a track without a playlist");
            return;
        };
        let named: HashSet<&str> = track_ids.iter().map(String::as_str).collect();
        let mut rows: Vec<String> = self
            .tracks
            .iter()
            .filter_map(|track| track.id.clone())
            .filter(|id| named.contains(id.as_str()))
            .collect();
        let mut counted: HashSet<String> = rows.iter().cloned().collect();
        let unseen: Vec<String> = track_ids
            .iter()
            .filter(|id| counted.insert((*id).clone()))
            .cloned()
            .collect();
        rows.extend(unseen);
        self.library.update(cx, |library, cx| {
            library.remove_tracks_from_playlist(playlist_id, rows, cx);
        });
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Loads what the page already shows again, which is how a screen retries after a failure.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (Some(kind), Some(id)) = (self.kind, self.id.clone()) else {
            return;
        };
        match kind {
            Collection::Album => self.open_album(&id, cx),
            Collection::Playlist => self.open_playlist(&id, cx),
        }
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new header and rows land.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let (Some(kind), Some(id)) = (self.kind, self.id.clone()) else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.kind = Some(kind);
        self.load(kind, id, true, cx);
    }

    pub fn open_album(&mut self, id: &str, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        let known = library.album(id).cloned();
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

        // With no network, or none allowed, a downloaded page opens from its kept copy.
        let offline = Network::lost(cx) && !music::is_local_id(id);
        let catalog = self.session.read(cx).catalog(id).filter(|_| !offline);
        let Some(catalog) = catalog else {
            if let Some(kept) = kept(kind, id, cx) {
                self.adopt(kept, cx);
            }
            // Signing back in reopens the page, so until then it reads as loading.
            let restoring = matches!(self.session.read(cx).state(), SessionState::Restoring);
            self.loading = restoring && !offline && self.tracks.is_empty();
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

        self.load(kind, id.to_owned(), false, cx);
        true
    }

    /// Fetches the page from the catalog. A `fresh` load drops the cached page first, and an
    /// album's rails with it, so the provider is asked again.
    fn load(&mut self, kind: Collection, id: String, fresh: bool, cx: &mut Context<Self>) {
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
                    Collection::Album => {
                        if fresh {
                            catalog.invalidate_album(&id).await;
                            catalog.invalidate_album_catalogue(&id).await;
                        }
                        catalog.album(&id).await.map(Loaded::Album)
                    }
                    Collection::Playlist => {
                        if fresh {
                            catalog.invalidate_playlist(&id).await;
                        }
                        catalog.playlist(&id).await.map(Loaded::Playlist)
                    }
                }
            }
        });
        self.request = Some(request.abort_handle());

        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;

            this.update(cx, |this, cx| {
                this.loading = false;
                this.request = None;
                match crate::settled(loaded, cx) {
                    Ok(detail) => this.adopt(detail, cx),
                    Err(reason) => match kept(kind, &id, cx) {
                        Some(kept) => this.adopt(kept, cx),
                        None => this.error = Some(reason),
                    },
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
                self.tracks = detail
                    .tracks
                    .iter()
                    .filter(|track| {
                        !track
                            .id
                            .as_deref()
                            .is_some_and(|id| self.library.read(cx).local_track_hidden(id))
                    })
                    .cloned()
                    .collect();
                self.continuation = None;
                self.fill(cx);
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
                self.continuation = detail.continuation.clone();
            }
        }
        self.loaded = true;
        self.load_more(cx);
    }

    fn clear(&mut self) {
        self.task = None;
        self.fill = None;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        if let Some(request) = self.filling_request.take() {
            request.abort();
        }
        self.mosaic = None;
        self.id = None;
        self.header = None;
        self.kind = None;
        self.album = None;
        self.playlist = None;
        self.tracks.clear();
        self.continuation = None;
        self.also_like.clear();
        self.similar.clear();
        self.loading = false;
        self.loading_more = false;
        self.loaded = false;
        self.filling = false;
        self.error = None;
    }

    /// Asks the provider for the rest of an album page once its tracks are up: related
    /// releases, with more from the same artist and similar artists beside them. A provider
    /// that answers
    /// everything in `album` has nothing to add here and the page stays as it is.
    fn fill(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        let artist = self.album.as_ref().and_then(|album| {
            album
                .artist_refs
                .iter()
                .find_map(|artist| artist.id.clone())
        });
        let Some(catalog) = self.session.read(cx).catalog(&id) else {
            return;
        };
        if let Some(catalogue) = catalog.peek_album_catalogue(&id) {
            self.absorb(&catalogue);
            return;
        }

        self.filling = true;
        let request = self.io.spawn({
            let id = id.clone();
            async move { catalog.album_catalogue(&id, artist.as_deref()).await }
        });
        self.filling_request = Some(request.abort_handle());
        self.fill = Some(cx.spawn(async move |this, cx| {
            let filled = join(request).await;

            this.update(cx, |this, cx| {
                if this.id.as_deref() != Some(id.as_str()) {
                    return;
                }

                this.filling = false;
                this.filling_request = None;
                match filled {
                    Ok(catalogue) => this.absorb(&catalogue),
                    Err(error) => log::warn!("detail: cannot fill the page: {error:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Puts the catalogue over the tracks. Every list replaces what the page held, and
    /// an empty one leaves that part of the page alone.
    fn absorb(&mut self, catalogue: &AlbumCatalogue) {
        if catalogue.is_empty() {
            return;
        }
        if !catalogue.also_like.is_empty() {
            self.also_like = catalogue.also_like.clone();
        }
        if !catalogue.similar.is_empty() {
            self.similar = catalogue.similar.clone();
        }
    }
}

fn album_header(album: &Album) -> Header {
    Header {
        kind: Collection::Album,
        release: Some(album.release_type),
        title: album.name.clone(),
        artist: Some(album.artists.clone()),
        artist_refs: album.artist_refs.clone(),
        owner: None,
        release_date: match album.release_date.is_empty() {
            true => (album.year > 0).then(|| album.year.to_string()),
            false => Some(album.release_date.clone()),
        },
        owner_name: None,
        track_count: album.track_count,
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
    let owner_name = match owner.is_some() {
        true => None,
        false => Some(playlist.owner.clone()),
    };

    Header {
        kind: Collection::Playlist,
        release: None,
        title: playlist.name.clone(),
        artist: None,
        artist_refs: Vec::new(),
        owner,
        release_date: None,
        owner_name,
        track_count: playlist.track_count,
        cover: playlist.cover.clone(),
    }
}

/// The downloaded copy of an album or playlist, which stands in for its page while the provider
/// is out of reach.
fn kept(kind: Collection, id: &str, cx: &App) -> Option<Loaded> {
    let downloads = cx.try_global::<Sonora>()?.downloads.read(cx);
    match kind {
        Collection::Album => downloads
            .album_detail(id)
            .map(|detail| Loaded::Album(Arc::new(detail))),
        Collection::Playlist => downloads
            .playlist_detail(id)
            .map(|detail| Loaded::Playlist(Arc::new(detail))),
    }
}
