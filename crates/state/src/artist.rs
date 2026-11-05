use std::sync::Arc;

use gpui::{Context, Entity, EventEmitter, Task};
use music::{Album, Artist, ArtistCatalogue, Track};
use tokio::task::AbortHandle;

use crate::{Io, Library, LibraryEvent, Session, SessionEvent, join};

/// Signals that a local artist no longer has a page after the library changes.
pub enum ArtistDetailEvent {
    Gone(String),
}

pub struct ArtistDetail {
    id: Option<String>,
    artist: Option<Arc<Artist>>,
    appears_on: Vec<Album>,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
    request: Option<AbortHandle>,
    filling: bool,
    fill: Option<Task<()>>,
    filling_request: Option<AbortHandle>,
}

impl EventEmitter<ArtistDetailEvent> for ArtistDetail {}

impl ArtistDetail {
    pub fn new(
        session: Entity<Session>,
        library: Entity<Library>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone().filter(|id| !music::is_local_id(id)) {
                    this.clear();
                    this.open(&id, cx);
                }
            }
            SessionEvent::SignedOut => {
                if !this.id.as_deref().is_some_and(music::is_local_id) {
                    this.clear();
                    cx.notify();
                }
            }
            SessionEvent::Reconnected | SessionEvent::WentOffline => {}
            SessionEvent::LocalChanged => {
                if let Some(id) = this.id.clone().filter(|id| music::is_local_id(id)) {
                    this.clear();
                    this.open(&id, cx);
                }
            }
        })
        .detach();

        cx.subscribe(&library, |this, _, event, cx| {
            let LibraryEvent::TracksHidden(ids) = event else {
                return;
            };
            let Some(artist) = this.artist.as_mut() else {
                return;
            };
            let before = artist.top_tracks.len();
            Arc::make_mut(artist)
                .top_tracks
                .retain(|track| !track.id.as_ref().is_some_and(|id| ids.contains(id)));
            if artist.top_tracks.len() != before {
                cx.notify();
            }
        })
        .detach();

        Self {
            id: None,
            artist: None,
            appears_on: Vec::new(),
            loading: false,
            error: None,
            session,
            io,
            task: None,
            request: None,
            filling: false,
            fill: None,
            filling_request: None,
        }
    }

    pub fn artist(&self) -> Option<&Artist> {
        self.artist.as_deref()
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn tracks(&self) -> &[Track] {
        self.artist
            .as_ref()
            .map(|artist| artist.top_tracks.as_slice())
            .unwrap_or_default()
    }

    pub fn albums(&self) -> &[Album] {
        self.artist
            .as_ref()
            .map(|artist| artist.albums.as_slice())
            .unwrap_or_default()
    }

    /// What the artist guests on, filled in behind the overview.
    pub fn appears_on(&self) -> &[Album] {
        &self.appears_on
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Whether the discography and the deeper popular tracks are still on their way, after
    /// the overview has already put the page up.
    pub fn is_filling(&self) -> bool {
        self.filling
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.id.as_deref() == Some(id) && (self.loading || self.artist.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());

        let Some(catalog) = self.session.read(cx).catalog(id) else {
            cx.notify();
            return;
        };
        if let Some(artist) = catalog.peek_artist(id) {
            self.artist = Some(artist);
            let id = id.to_owned();
            self.fill(&id, cx);
            cx.notify();
            return;
        }
        self.load(id.to_owned(), false, cx);
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new overview lands.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.load(id, true, cx);
    }

    /// Fetches the artist's overview and fills the rest of the page once it lands. A `fresh`
    /// load drops the cached artist first, so the provider is asked again.
    fn load(&mut self, id: String, fresh: bool, cx: &mut Context<Self>) {
        let Some(catalog) = self.session.read(cx).catalog(&id) else {
            cx.notify();
            return;
        };

        self.loading = true;
        cx.notify();

        let local = music::is_local_id(&id)
            .then(|| self.session.read(cx).local_client())
            .flatten();
        let request = self.io.spawn({
            let id = id.clone();
            async move {
                if let Some(client) = local
                    && !client
                        .all_artists()
                        .await?
                        .iter()
                        .any(|artist| artist.id == id)
                {
                    return Ok(None);
                }
                if fresh {
                    catalog.invalidate_artist(&id).await;
                }
                catalog.artist(&id).await.map(Some)
            }
        });
        self.request = Some(request.abort_handle());
        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;

            this.update(cx, |this, cx| {
                if this.id.as_deref() != Some(id.as_str()) {
                    return;
                }

                this.loading = false;
                this.request = None;
                match crate::settled(loaded, cx) {
                    Ok(Some(artist)) => {
                        this.artist = Some(artist);
                        this.fill(&id, cx);
                    }
                    Ok(None) => {
                        this.clear();
                        cx.emit(ArtistDetailEvent::Gone(id));
                    }
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Asks the provider for the rest of the page once its overview is up: the whole
    /// discography and the popular tracks only the discography can rank. A provider that
    /// answers everything in `artist` has nothing to add here and the page stays as it is.
    fn fill(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(artist) = self.artist.clone() else {
            return;
        };
        let Some(catalog) = self.session.read(cx).catalog(id) else {
            return;
        };
        if let Some(catalogue) = catalog.peek_artist_catalogue(id) {
            self.absorb(&catalogue);
            return;
        }

        self.filling = true;
        let id = id.to_owned();
        let request = self.io.spawn({
            let id = id.clone();
            let known = artist.top_tracks.clone();
            async move { catalog.artist_catalogue(&id, &known).await }
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
                    Err(error) => log::warn!("artist: cannot fill the page: {error:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Puts the catalogue over the overview. Every list replaces what the overview carried,
    /// and an empty one leaves that part of the page alone.
    fn absorb(&mut self, catalogue: &ArtistCatalogue) {
        let Some(artist) = self.artist.as_mut() else {
            return;
        };
        if catalogue.is_empty() {
            return;
        }
        let artist = Arc::make_mut(artist);
        if !catalogue.albums.is_empty() {
            artist.albums = catalogue.albums.clone();
        }
        if !catalogue.top_tracks.is_empty() {
            artist.top_tracks = catalogue.top_tracks.clone();
        }
        if !catalogue.appears_on.is_empty() {
            self.appears_on = catalogue.appears_on.clone();
        }
    }

    fn clear(&mut self) {
        self.task = None;
        self.fill = None;
        for request in [self.request.take(), self.filling_request.take()]
            .into_iter()
            .flatten()
        {
            request.abort();
        }
        self.id = None;
        self.artist = None;
        self.appears_on.clear();
        self.loading = false;
        self.filling = false;
        self.error = None;
    }
}
