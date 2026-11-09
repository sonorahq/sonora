use std::sync::Arc;

use music::{Album, Artist, ArtistCatalogue, Track};

use crate::{Change, Cx, Event, Job, LibraryEvent, Session, SessionEvent, Store};

/// The artist page: what [`Store::pages`](crate::Pages) shows as `artist`. A local artist the
/// library no longer holds raises [`Event::ArtistGone`].
pub struct ArtistDetail {
    id: Option<String>,
    artist: Option<Arc<Artist>>,
    appears_on: Vec<Album>,
    loading: bool,
    error: Option<String>,
    task: Option<Job>,
    filling: bool,
    fill: Option<Job>,
}

impl ArtistDetail {
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            artist: None,
            appears_on: Vec::new(),
            loading: false,
            error: None,
            task: None,
            filling: false,
            fill: None,
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

    fn open(&mut self, id: &str, session: &Session, cx: &mut Cx) {
        if self.id.as_deref() == Some(id) && (self.loading || self.artist.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());

        let Some(catalog) = session.catalog(id) else {
            return;
        };
        if let Some(artist) = catalog.peek_artist(id) {
            self.artist = Some(artist);
            let id = id.to_owned();
            self.fill(&id, session, cx);
            return;
        }
        self.load(id.to_owned(), false, session, cx);
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new overview lands.
    fn refresh(&mut self, session: &Session, cx: &mut Cx) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.load(id, true, session, cx);
    }

    /// Fetches the artist's overview and fills the rest of the page once it lands. A `fresh`
    /// load drops the cached artist first, so the provider is asked again.
    fn load(&mut self, id: String, fresh: bool, session: &Session, cx: &mut Cx) {
        let Some(catalog) = session.catalog(&id) else {
            return;
        };

        self.loading = true;

        let local = music::is_local_id(&id)
            .then(|| session.local_client())
            .flatten();
        self.task = Some(cx.spawn(move |link| async move {
            let loaded: anyhow::Result<Option<Arc<Artist>>> = async {
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
            .await;

            link.post(move |store| {
                let Store {
                    pages, session, cx, ..
                } = store;
                let Some(this) = showing(&mut pages.artist, &id) else {
                    return;
                };

                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(Some(artist)) => {
                        this.artist = Some(artist);
                        this.fill(&id, session, cx);
                    }
                    Ok(None) => {
                        this.clear();
                        cx.emit(Event::ArtistGone(id));
                    }
                    Err(reason) => this.error = Some(reason),
                }
            });
        }));
    }

    /// Asks the provider for the rest of the page once its overview is up: the whole
    /// discography and the popular tracks only the discography can rank. A provider that
    /// answers everything in `artist` has nothing to add here and the page stays as it is.
    fn fill(&mut self, id: &str, session: &Session, cx: &mut Cx) {
        let Some(artist) = self.artist.clone() else {
            return;
        };
        let Some(catalog) = session.catalog(id) else {
            return;
        };
        if let Some(catalogue) = catalog.peek_artist_catalogue(id) {
            self.absorb(&catalogue);
            return;
        }

        self.filling = true;
        let id = id.to_owned();
        let known = artist.top_tracks.clone();
        self.fill = Some(cx.spawn(move |link| async move {
            let filled = catalog.artist_catalogue(&id, &known).await;

            link.post(move |store| {
                let Some(this) = showing(&mut store.pages.artist, &id) else {
                    return;
                };

                this.filling = false;
                match filled {
                    Ok(catalogue) => this.absorb(&catalogue),
                    Err(error) => log::warn!("artist: cannot fill the page: {error:#}"),
                }
            });
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
        self.id = None;
        self.artist = None;
        self.appears_on.clear();
        self.loading = false;
        self.filling = false;
        self.error = None;
    }
}

impl Store {
    /// Opens the artist page on `id`, from the catalog cache when it already holds the artist.
    pub fn open_artist(&mut self, id: &str) {
        let Store {
            pages, session, cx, ..
        } = self;
        pages
            .artist
            .get_or_insert_with(ArtistDetail::new)
            .open(id, session, cx);
    }

    /// Drops the artist page, and the loads it has under way.
    pub fn close_artist(&mut self) {
        self.pages.artist = None;
    }

    /// Drops the artist page's contents and fetches them again from the provider, past the
    /// catalog cache.
    pub fn refresh_artist(&mut self) {
        let Store {
            pages, session, cx, ..
        } = self;
        if let Some(artist) = pages.artist.as_mut() {
            artist.refresh(session, cx);
        }
    }

    pub(crate) fn react_artist(&mut self, change: &Change) {
        let Store {
            pages, session, cx, ..
        } = self;
        let Some(this) = pages.artist.as_mut() else {
            return;
        };
        match change {
            Change::Event(Event::Session(event)) => match event {
                SessionEvent::SignedIn => {
                    if let Some(id) = this.id.clone().filter(|id| !music::is_local_id(id)) {
                        this.clear();
                        this.open(&id, session, cx);
                    }
                }
                SessionEvent::SignedOut => {
                    if !this.id.as_deref().is_some_and(music::is_local_id) {
                        this.clear();
                    }
                }
                SessionEvent::Reconnected | SessionEvent::WentOffline => {}
                SessionEvent::LocalChanged => {
                    if let Some(id) = this.id.clone().filter(|id| music::is_local_id(id)) {
                        this.clear();
                        this.open(&id, session, cx);
                    }
                }
            },
            Change::Event(Event::Library(LibraryEvent::TracksHidden(ids))) => {
                let Some(artist) = this.artist.as_mut() else {
                    return;
                };
                Arc::make_mut(artist)
                    .top_tracks
                    .retain(|track| !track.id.as_ref().is_some_and(|id| ids.contains(id)));
            }
            _ => {}
        }
    }
}

/// The page in `slot` when it still shows `id`, which a load that lands checks first, since
/// another artist may have opened in its place.
fn showing<'a>(slot: &'a mut Option<ArtistDetail>, id: &str) -> Option<&'a mut ArtistDetail> {
    slot.as_mut()
        .filter(|artist| artist.id.as_deref() == Some(id))
}
