use std::collections::HashSet;
use std::sync::Arc;

use music::{
    Album, AlbumCatalogue, AlbumDetail, ArtistRef, Contributor, Playlist, PlaylistDetail,
    ReleaseType, SavedArtist, Track,
};

use crate::{
    Change, Cx, Downloads, Event, Job, Library, LibraryEvent, Network, Session, SessionEvent,
    SessionState, Store, Topic, mosaic,
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

/// The album or playlist page: what [`Store::pages`](crate::Pages) shows as `album` or
/// `playlist`. Each page kind keeps one, which the next item opened replaces the contents of.
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
    /// Fetches the mosaic's covers.
    http: reqwest::Client,
    task: Option<Job>,
    fill: Option<Job>,
    mosaic: Option<Job>,
}

/// What a page reads of the other models while it opens or takes in a load.
pub(crate) struct Around<'a> {
    session: &'a Session,
    library: &'a Library,
    network: &'a Network,
    downloads: &'a Downloads,
}

impl Detail {
    pub(crate) fn new() -> Self {
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
            http: reqwest::Client::new(),
            task: None,
            fill: None,
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

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Follows the session: a sign-out drops a streamed page, and a sign-in or a change to the
    /// local files opens the page again from its provider.
    fn session_event(&mut self, event: &SessionEvent, around: &Around, cx: &mut Cx) {
        match event {
            SessionEvent::SignedOut => {
                if !self.id.as_deref().is_some_and(music::is_local_id) {
                    self.clear();
                }
            }
            SessionEvent::SignedIn => {
                if let (Some(kind), Some(id)) = (
                    self.kind,
                    self.id.clone().filter(|id| !music::is_local_id(id)),
                ) {
                    self.clear();
                    match kind {
                        Collection::Album => self.open_album(&id, around, cx),
                        Collection::Playlist => self.open_playlist(&id, around, cx),
                    }
                }
            }
            SessionEvent::Reconnected | SessionEvent::WentOffline => {}
            SessionEvent::LocalChanged => {
                if let (Some(kind), Some(id)) = (
                    self.kind,
                    self.id.clone().filter(|id| music::is_local_id(id)),
                ) {
                    self.clear();
                    match kind {
                        Collection::Album => self.open_album(&id, around, cx),
                        Collection::Playlist => self.open_playlist(&id, around, cx),
                    }
                }
            }
        }
    }

    /// A page that opened while the account was still signing back in waits for it. Once the
    /// session settles anywhere but signed in, there is nothing coming to wait for.
    fn session_changed(&mut self, session: &Session) {
        let restoring = matches!(session.state(), SessionState::Restoring);
        if self.loading && self.task.is_none() && !restoring {
            self.loading = false;
        }
    }

    /// Follows changes the library made to the playlist on the page and to hidden tracks.
    fn library_event(&mut self, event: &LibraryEvent, around: &Around, cx: &mut Cx) {
        match event {
            LibraryEvent::PlaylistGone(id) if self.id.as_deref() == Some(id.as_str()) => {
                self.clear();
            }
            LibraryEvent::TrackAdded { playlist } | LibraryEvent::PlaylistChanged(playlist)
                if self.id.as_deref() == Some(playlist.as_str()) =>
            {
                self.load(Collection::Playlist, playlist.clone(), false, around, cx);
            }
            LibraryEvent::TrackDropped { playlist, track }
                if self.id.as_deref() == Some(playlist.as_str()) =>
            {
                self.tracks
                    .retain(|shown| shown.id.as_deref() != Some(track.as_str()));
            }
            LibraryEvent::TracksHidden(ids) => {
                self.tracks
                    .retain(|track| !track.id.as_ref().is_some_and(|id| ids.contains(id)));
            }
            _ => {}
        }
    }

    /// Takes in what the library now knows of the playlist on the page, such as a rename or a
    /// new cover.
    fn library_changed(&mut self, library: &Library) {
        let Some(id) = self.id.clone() else {
            return;
        };
        let Some(mut playlist) = library.playlist(&id).cloned() else {
            return;
        };
        if playlist.cover.is_none() {
            playlist.cover = self.playlist.as_ref().and_then(|shown| shown.cover.clone());
        }
        if self.playlist.as_ref() == Some(&playlist) {
            return;
        }
        self.header = Some(playlist_header(&playlist));
        self.playlist = Some(playlist);
    }

    /// Appends provider pages in sequence while keeping the first page immediately usable.
    fn load_more(&mut self, around: &Around, cx: &mut Cx) {
        if self.loading_more {
            return;
        }
        let Some(continuation) = self.continuation.clone() else {
            return;
        };
        let Some(id) = self.id.clone() else {
            return;
        };
        let Some(kind) = self.kind else {
            return;
        };
        let Some(catalog) = around.session.catalog(&id) else {
            return;
        };

        self.loading_more = true;

        self.task = Some(cx.spawn(move |link| async move {
            let loaded = catalog.playlist_continuation(&continuation).await;
            link.post(move |store| {
                let (slot, around, cx) = store.detail_parts(kind);
                let Some(this) = showing(slot, &id) else {
                    return;
                };
                this.loading_more = false;
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
                        this.load_more(&around, cx);
                    }
                    Err(error) => log::warn!("detail: cannot load more playlist tracks: {error:#}"),
                }
            });
        }));
    }

    /// The rows the library is told to take out of the playlist on the page for every copy of
    /// the named tracks: each row the page shows of them, then any named track it does not
    /// show. `None` when the page shows no playlist.
    fn removal(&self, track_ids: &[String]) -> Option<(String, Vec<String>)> {
        let Some(playlist_id) = self.id.clone() else {
            log::warn!("detail: cannot remove a track without a playlist");
            return None;
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
        Some((playlist_id, rows))
    }

    /// Loads what the page already shows again, which is how a screen retries after a failure.
    fn reload(&mut self, around: &Around, cx: &mut Cx) {
        let (Some(kind), Some(id)) = (self.kind, self.id.clone()) else {
            return;
        };
        match kind {
            Collection::Album => self.open_album(&id, around, cx),
            Collection::Playlist => self.open_playlist(&id, around, cx),
        }
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new header and rows land.
    fn refresh(&mut self, around: &Around, cx: &mut Cx) {
        let (Some(kind), Some(id)) = (self.kind, self.id.clone()) else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.kind = Some(kind);
        self.load(kind, id, true, around, cx);
    }

    fn open_album(&mut self, id: &str, around: &Around, cx: &mut Cx) {
        let known = around.library.album(id).cloned();
        let header = known.as_ref().map(album_header);
        if self.open(Collection::Album, id, header, around, cx) && !self.loaded {
            self.album = known;
        }
    }

    fn open_playlist(&mut self, id: &str, around: &Around, cx: &mut Cx) {
        let mut known = around.library.playlist(id).cloned();
        let header = known.as_ref().map(playlist_header);
        if !self.open(Collection::Playlist, id, header, around, cx) {
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
        around: &Around,
        cx: &mut Cx,
    ) -> bool {
        if self.shows(kind, id) {
            return false;
        }

        self.clear();
        self.id = Some(id.to_owned());
        self.kind = Some(kind);
        self.header = known;

        // With no network, or none allowed, a downloaded page opens from its kept copy.
        let offline = around.network.lost() && !music::is_local_id(id);
        let catalog = around.session.catalog(id).filter(|_| !offline);
        let Some(catalog) = catalog else {
            if let Some(kept) = kept(kind, id, around.downloads) {
                self.adopt(kept, around, cx);
            }
            // Signing back in reopens the page, so until then it reads as loading.
            let restoring = matches!(around.session.state(), SessionState::Restoring);
            self.loading = restoring && !offline && self.tracks.is_empty();
            return true;
        };
        let cached = match kind {
            Collection::Album => catalog.peek_album(id).map(Loaded::Album),
            Collection::Playlist => catalog.peek_playlist(id).map(Loaded::Playlist),
        };
        if let Some(cached) = cached {
            self.adopt(cached, around, cx);
            return true;
        }

        self.load(kind, id.to_owned(), false, around, cx);
        true
    }

    /// Fetches the page from the catalog. A `fresh` load drops the cached page first, and an
    /// album's rails with it, so the provider is asked again.
    fn load(&mut self, kind: Collection, id: String, fresh: bool, around: &Around, cx: &mut Cx) {
        let Some(catalog) = around.session.catalog(&id) else {
            return;
        };

        // The slot the page sits in, which is the kind it opened as.
        let page = self.kind.unwrap_or(kind);
        self.loading = true;
        self.error = None;

        self.task = Some(cx.spawn(move |link| async move {
            let loaded = match kind {
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
            };

            link.post(move |store| {
                let (slot, around, cx) = store.detail_parts(page);
                let Some(this) = showing(slot, &id) else {
                    return;
                };
                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(detail) => this.adopt(detail, &around, cx),
                    Err(reason) => match kept(kind, &id, around.downloads) {
                        Some(kept) => this.adopt(kept, &around, cx),
                        None => this.error = Some(reason),
                    },
                }
            });
        }));
    }

    fn known_mosaic(&self, playlist: &Playlist, library: &Library) -> Option<String> {
        library
            .playlist(&playlist.id)
            .and_then(|known| known.cover.clone())
            .or_else(|| mosaic::cached(&playlist.id, playlist.track_count))
    }

    fn paint_mosaic(&mut self, playlist: &Playlist, tracks: &[Track], cx: &mut Cx) {
        let covers = music::distinct_covers(tracks, mosaic::TILES);
        if covers.len() < mosaic::TILES {
            return;
        }
        let Some(kind) = self.kind else {
            return;
        };
        let Some(shown) = self.id.clone() else {
            return;
        };

        let id = playlist.id.clone();
        let stamp = playlist.track_count;
        let http = self.http.clone();
        self.mosaic = Some(cx.spawn(move |link| async move {
            let built = mosaic::build(http, &id, stamp, covers).await;

            link.post(move |store| {
                let (slot, _, _) = store.detail_parts(kind);
                let Some(this) = showing(slot, &shown) else {
                    return;
                };
                match built {
                    Ok(cover) => {
                        if let Some(header) = this.header.as_mut() {
                            header.cover = Some(cover.clone());
                        }
                        if let Some(playlist) = this.playlist.as_mut() {
                            playlist.cover = Some(cover);
                        }
                    }
                    Err(error) => log::warn!("detail: cannot build a mosaic: {error:#}"),
                }
            });
        }));
    }

    fn shows(&self, kind: Collection, id: &str) -> bool {
        let same = self.kind == Some(kind) && self.id.as_deref() == Some(id);
        same && (self.loading || self.loaded)
    }

    fn adopt(&mut self, loaded: Loaded, around: &Around, cx: &mut Cx) {
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
                            .is_some_and(|id| around.library.local_track_hidden(id))
                    })
                    .cloned()
                    .collect();
                self.continuation = None;
                self.fill(around, cx);
            }
            Loaded::Playlist(detail) => {
                let mut playlist = detail.playlist.clone();
                if playlist.cover.is_none() {
                    playlist.cover = self.known_mosaic(&playlist, around.library);
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
        self.load_more(around, cx);
    }

    fn clear(&mut self) {
        self.task = None;
        self.fill = None;
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
    fn fill(&mut self, around: &Around, cx: &mut Cx) {
        let Some(id) = self.id.clone() else {
            return;
        };
        let Some(kind) = self.kind else {
            return;
        };
        let artist = self.album.as_ref().and_then(|album| {
            album
                .artist_refs
                .iter()
                .find_map(|artist| artist.id.clone())
        });
        let Some(catalog) = around.session.catalog(&id) else {
            return;
        };
        if let Some(catalogue) = catalog.peek_album_catalogue(&id) {
            self.absorb(&catalogue);
            return;
        }

        self.filling = true;
        self.fill = Some(cx.spawn(move |link| async move {
            let filled = catalog.album_catalogue(&id, artist.as_deref()).await;

            link.post(move |store| {
                let (slot, _, _) = store.detail_parts(kind);
                let Some(this) = showing(slot, &id) else {
                    return;
                };

                this.filling = false;
                match filled {
                    Ok(catalogue) => this.absorb(&catalogue),
                    Err(error) => log::warn!("detail: cannot fill the page: {error:#}"),
                }
            });
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

impl Store {
    /// Opens the album page on `id`, from what the library and the catalog cache already know
    /// when they know it.
    pub fn open_album(&mut self, id: &str) {
        let (slot, around, cx) = self.detail_parts(Collection::Album);
        slot.get_or_insert_with(Detail::new)
            .open_album(id, &around, cx);
    }

    /// Opens the playlist page on `id`, from what the library and the catalog cache already
    /// know when they know it.
    pub fn open_playlist(&mut self, id: &str) {
        let (slot, around, cx) = self.detail_parts(Collection::Playlist);
        slot.get_or_insert_with(Detail::new)
            .open_playlist(id, &around, cx);
    }

    /// Drops the album or playlist page, and the loads it has under way.
    pub fn close_detail(&mut self, kind: Collection) {
        *self.detail_parts(kind).0 = None;
    }

    /// Loads what the album or playlist page already shows again, which is how a screen
    /// retries after a failure.
    pub fn reload_detail(&mut self, kind: Collection) {
        let (slot, around, cx) = self.detail_parts(kind);
        if let Some(detail) = slot.as_mut() {
            detail.reload(&around, cx);
        }
    }

    /// Drops the album or playlist page and fetches it again from the provider, past the
    /// catalog cache, so it shows its loading state until the new header and rows land.
    pub fn refresh_detail(&mut self, kind: Collection) {
        let (slot, around, cx) = self.detail_parts(kind);
        if let Some(detail) = slot.as_mut() {
            detail.refresh(&around, cx);
        }
    }

    /// Takes one track out of the playlist on the playlist page.
    pub fn remove_from_playlist_page(&mut self, track_id: String) {
        self.remove_tracks_from_playlist_page(vec![track_id]);
    }

    /// Takes every copy of the named tracks out of the playlist on the playlist page. The
    /// library is told about each row the page shows of them, so its count and Undo cover
    /// every copy.
    pub fn remove_tracks_from_playlist_page(&mut self, track_ids: Vec<String>) {
        let Some(detail) = self.pages.playlist.as_ref() else {
            log::warn!("detail: cannot remove a track without a playlist");
            return;
        };
        let Some((playlist_id, rows)) = detail.removal(&track_ids) else {
            return;
        };
        self.library
            .remove_tracks_from_playlist(playlist_id, rows, &self.session, &mut self.cx);
    }

    /// The page slot of `kind`, what it reads of the other models, and the store's context,
    /// borrowed apart so a page can change while it reads them.
    pub(crate) fn detail_parts(
        &mut self,
        kind: Collection,
    ) -> (&mut Option<Detail>, Around<'_>, &mut Cx) {
        let Store {
            pages,
            session,
            library,
            network,
            downloads,
            cx,
            ..
        } = self;
        let slot = match kind {
            Collection::Album => &mut pages.album,
            Collection::Playlist => &mut pages.playlist,
        };
        let around = Around {
            session,
            library,
            network,
            downloads,
        };
        (slot, around, cx)
    }

    pub(crate) fn react_detail(&mut self, change: &Change) {
        for kind in [Collection::Album, Collection::Playlist] {
            let (slot, around, cx) = self.detail_parts(kind);
            let Some(detail) = slot.as_mut() else {
                continue;
            };
            match change {
                Change::Event(Event::Session(event)) => detail.session_event(event, &around, cx),
                Change::Topic(Topic::Session) => detail.session_changed(around.session),
                Change::Event(Event::Library(event)) => detail.library_event(event, &around, cx),
                Change::Topic(Topic::Library) => detail.library_changed(around.library),
                _ => {}
            }
        }
    }
}

/// The page in `slot` when it still shows `id`, which a load that lands checks first, since
/// another item may have opened in its place.
fn showing<'a>(slot: &'a mut Option<Detail>, id: &str) -> Option<&'a mut Detail> {
    slot.as_mut()
        .filter(|detail| detail.id.as_deref() == Some(id))
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
fn kept(kind: Collection, id: &str, downloads: &Downloads) -> Option<Loaded> {
    match kind {
        Collection::Album => downloads
            .album_detail(id)
            .map(|detail| Loaded::Album(Arc::new(detail))),
        Collection::Playlist => downloads
            .playlist_detail(id)
            .map(|detail| Loaded::Playlist(Arc::new(detail))),
    }
}
