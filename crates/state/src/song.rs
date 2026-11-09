use std::collections::HashMap;
use std::sync::Arc;

use music::{AlbumDetail, ArtistProfile, Track};

use crate::catalog::SongPage;
use crate::{Change, Cx, Event, Job, Session, SessionEvent, Store};

/// The song page: what [`Store::pages`](crate::Pages) shows as `song`.
pub struct SongDetail {
    id: Option<String>,
    page: Option<Arc<SongPage>>,
    loading: bool,
    error: Option<String>,
    task: Option<Job>,
}

impl SongDetail {
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            page: None,
            loading: false,
            error: None,
            task: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn track(&self) -> Option<&Track> {
        self.page.as_ref().map(|page| &page.track)
    }
    pub fn album(&self) -> Option<&AlbumDetail> {
        self.page.as_ref()?.album.as_deref()
    }
    pub fn artist(&self) -> Option<&ArtistProfile> {
        self.page.as_ref()?.artist.as_deref()
    }
    pub fn portraits(&self) -> &HashMap<String, String> {
        self.page
            .as_ref()
            .map(|page| &page.portraits)
            .unwrap_or_else(|| empty_portraits())
    }
    pub fn playcount(&self) -> Option<u64> {
        self.page.as_ref().and_then(|page| page.playcount)
    }
    pub fn is_loading(&self) -> bool {
        self.loading
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn open(&mut self, id: &str, session: &Session, cx: &mut Cx) {
        if self.id.as_deref() == Some(id) && (self.loading || self.page.is_some()) {
            return;
        }
        self.clear();
        self.id = Some(id.to_owned());
        let Some(catalog) = session.catalog(id) else {
            return;
        };
        if let Some(page) = catalog.peek_song(id) {
            self.page = Some(page);
            return;
        }
        self.load(id.to_owned(), false, session, cx);
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new one lands.
    fn refresh(&mut self, session: &Session, cx: &mut Cx) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.load(id, true, session, cx);
    }

    /// Fetches the song page. A `fresh` load drops the cached page first, so the provider is
    /// asked again.
    fn load(&mut self, id: String, fresh: bool, session: &Session, cx: &mut Cx) {
        let Some(catalog) = session.catalog(&id) else {
            return;
        };

        self.loading = true;
        self.task = Some(cx.spawn(move |link| async move {
            if fresh {
                catalog.invalidate_song(&id).await;
            }
            let loaded = catalog.song(&id).await;
            link.post(move |store| {
                let Store { pages, cx, .. } = store;
                let Some(this) = pages
                    .song
                    .as_mut()
                    .filter(|this| this.id.as_deref() == Some(id.as_str()))
                else {
                    return;
                };
                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(page) => this.page = Some(page),
                    Err(reason) => this.error = Some(reason),
                }
            });
        }));
    }

    fn clear(&mut self) {
        self.task = None;
        self.id = None;
        self.page = None;
        self.loading = false;
        self.error = None;
    }
}

impl Store {
    /// Opens the song page on `id`, from the catalog cache when it already holds the song.
    pub fn open_song(&mut self, id: &str) {
        let Store {
            pages, session, cx, ..
        } = self;
        pages
            .song
            .get_or_insert_with(SongDetail::new)
            .open(id, session, cx);
    }

    /// Drops the song page, and the load it has under way.
    pub fn close_song(&mut self) {
        self.pages.song = None;
    }

    /// Drops the song page and fetches it again from the provider, past the catalog cache.
    pub fn refresh_song(&mut self) {
        let Store {
            pages, session, cx, ..
        } = self;
        if let Some(song) = pages.song.as_mut() {
            song.refresh(session, cx);
        }
    }

    pub(crate) fn react_song(&mut self, change: &Change) {
        let Change::Event(Event::Session(event)) = change else {
            return;
        };
        let Store {
            pages, session, cx, ..
        } = self;
        let Some(this) = pages.song.as_mut() else {
            return;
        };
        match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone().filter(|id| !music::is_local_id(id)) {
                    this.clear();
                    this.open(&id, session, cx);
                }
            }
            SessionEvent::SignedOut => this.clear(),
            SessionEvent::Reconnected | SessionEvent::WentOffline => {}
            SessionEvent::LocalChanged => {
                if let Some(id) = this.id.clone().filter(|id| music::is_local_id(id)) {
                    this.clear();
                    this.open(&id, session, cx);
                }
            }
        }
    }
}

fn empty_portraits() -> &'static HashMap<String, String> {
    static EMPTY: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}
