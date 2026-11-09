use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use music::{Genre, GenreDetail, GenreSection, MusicApi};
use serde::Deserialize;

use crate::{Change, Cx, Event, Job, Session, SessionEvent, Store, join, trim_heap};

/// How many bare genres get a picture fetched for them, which covers the first screens of tiles.
const DRESSED: usize = 48;
/// How many of those pictures are asked for at once.
const AT_ONCE: usize = 4;
/// How long the pictures found for a provider's genres are reused before they are looked up again.
const KEEP_COVERS: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The pictures found for a provider's genres by genre id, with `None` for a genre known to have
/// none.
type Covers = HashMap<String, Option<String>>;

/// The genres the search page browses. Nothing is fetched until a screen calls `load`, and a
/// sign-in fetches them again only after that.
pub struct Genres {
    genres: Arc<Vec<Genre>>,
    /// Whether a screen has asked for the genres, so a sign-in knows to fetch them.
    wanted: bool,
    loading: bool,
    error: Option<String>,
    cache: storage::Cache,
    task: Option<Job>,
    /// Fetches the pictures of genres the provider listed without one.
    dressing: Option<Job>,
}

/// The pictures a provider's genres were last dressed with, as `cache.sqlite` keeps them.
#[derive(Deserialize)]
struct Stored {
    saved: u64,
    covers: Covers,
}

impl Genres {
    pub fn new() -> Self {
        Self {
            genres: Arc::new(Vec::new()),
            wanted: false,
            loading: false,
            error: None,
            cache: storage::Cache::standard(),
            task: None,
            dressing: None,
        }
    }

    pub fn genres(&self) -> Arc<Vec<Genre>> {
        self.genres.clone()
    }

    pub fn adopt(&mut self, id: &str, cover: Option<String>) {
        let Some(cover) = cover else {
            return;
        };
        let Some(genre) = Arc::make_mut(&mut self.genres)
            .iter_mut()
            .find(|genre| genre.id == id && genre.cover.is_none())
        else {
            return;
        };

        genre.cover = Some(cover);
    }

    pub fn forget(&mut self, id: &str) {
        Arc::make_mut(&mut self.genres).retain(|genre| genre.id != id);
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Fetches the genres unless they are loaded or loading. Before a sign-in it only marks them
    /// wanted, and the sign-in fetches them.
    pub fn load(&mut self, session: &Session, cx: &mut Cx) {
        self.wanted = true;
        if self.loading || !self.genres.is_empty() {
            return;
        }
        let Some(client) = session.client() else {
            return;
        };

        self.loading = true;
        self.error = None;

        self.task = Some(cx.spawn(move |link| async move {
            let loaded = client.genres().await;

            link.post(move |store| {
                let Store {
                    genres,
                    session,
                    cx,
                    ..
                } = store;
                genres.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(listed) => {
                        genres.genres = Arc::new(listed);
                        genres.dress(client, session, cx);
                    }
                    Err(reason) => genres.error = Some(reason),
                }
            });
        }));
    }

    /// Puts pictures on the first genres the provider listed without one. Pictures stored for this
    /// provider within `KEEP_COVERS` go on at once, and the rest are fetched a few at a time and
    /// stored for the next launch. A genre whose picture cannot be found keeps the placeholder.
    fn dress(&mut self, client: Arc<dyn MusicApi>, session: &Session, cx: &mut Cx) {
        let listed: Vec<String> = self
            .genres
            .iter()
            .filter(|genre| genre.cover.is_none())
            .take(DRESSED)
            .map(|genre| genre.id.clone())
            .collect();
        if listed.is_empty() {
            return;
        }

        let slug = session.provider_slug();
        let cache = self.cache.clone();
        let io = cx.io.clone();
        self.dressing = Some(cx.spawn(move |link| async move {
            let stored = match slug {
                Some(slug) => {
                    let cache = cache.clone();
                    tokio::task::spawn_blocking(move || remembered(&cache, slug))
                        .await
                        .unwrap_or_default()
                }
                None => Covers::new(),
            };
            let mut known: Covers = listed
                .iter()
                .filter_map(|id| Some((id.clone(), stored.get(id)?.clone())))
                .collect();
            let bare: Vec<String> = listed
                .into_iter()
                .filter(|id| !known.contains_key(id))
                .collect();
            let shown = known.clone();
            let kept = link
                .read(move |store| {
                    for (id, cover) in shown {
                        store.genres.adopt(&id, cover);
                    }
                })
                .await;
            if kept.is_none() || bare.is_empty() {
                return;
            }

            for batch in bare.chunks(AT_ONCE) {
                let asked = batch.iter().map(|id| {
                    let client = client.clone();
                    let id = id.clone();
                    join(io.spawn(async move { client.genre_cover(&id).await }))
                });
                let found = futures::future::join_all(asked).await;
                let mut landed = Vec::new();
                for (id, cover) in batch.iter().zip(found) {
                    match cover {
                        Ok(cover) => {
                            landed.push((id.clone(), cover.clone()));
                            known.insert(id.clone(), cover);
                        }
                        Err(error) => log::debug!("genres: no picture for {id}: {error:#}"),
                    }
                }
                let kept = link
                    .read(move |store| {
                        for (id, cover) in landed {
                            store.genres.adopt(&id, cover);
                        }
                    })
                    .await;
                if kept.is_none() {
                    return;
                }
            }

            // A provider may answer with whole pages to find one picture each, and what parsing
            // them freed would otherwise stay resident.
            tokio::task::spawn_blocking(move || {
                if let Some(slug) = slug {
                    remember(&cache, slug, &known);
                }
                trim_heap();
            })
            .await
            .ok();
        }));
    }

    fn session_event(&mut self, event: &SessionEvent, session: &Session, cx: &mut Cx) {
        match event {
            SessionEvent::SignedIn if self.wanted => self.load(session, cx),
            SessionEvent::SignedIn => {}
            SessionEvent::SignedOut => {
                self.task = None;
                self.dressing = None;
                self.genres = Arc::new(Vec::new());
                self.loading = false;
                self.error = None;
            }
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        }
    }
}

impl Default for Genres {
    fn default() -> Self {
        Self::new()
    }
}

/// The genre page: what [`Store::pages`](crate::Pages) shows as `genre`.
pub struct GenreDetails {
    id: Option<String>,
    detail: Option<Arc<GenreDetail>>,
    sections: Arc<Vec<GenreSection>>,
    loading: bool,
    error: Option<String>,
    task: Option<Job>,
}

impl GenreDetails {
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            detail: None,
            sections: Arc::new(Vec::new()),
            loading: false,
            error: None,
            task: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn name(&self) -> Option<&str> {
        self.detail.as_ref().map(|detail| detail.name.as_str())
    }

    pub fn sections(&self) -> Arc<Vec<GenreSection>> {
        self.sections.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn open(&mut self, id: &str, session: &Session, genres: &mut Genres, cx: &mut Cx) {
        if self.id.as_deref() == Some(id) && (self.loading || self.detail.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());
        let Some(catalog) = session.catalog(id) else {
            return;
        };
        if let Some(detail) = catalog.peek_genre(id) {
            self.adopt(id, detail, genres);
            return;
        }
        self.load(id.to_owned(), false, session, cx);
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new sections land.
    fn refresh(&mut self, session: &Session, cx: &mut Cx) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.load(id, true, session, cx);
    }

    /// Fetches the genre's sections. A `fresh` load drops the cached genre first, so the
    /// provider is asked again.
    fn load(&mut self, id: String, fresh: bool, session: &Session, cx: &mut Cx) {
        let Some(catalog) = session.catalog(&id) else {
            return;
        };

        self.loading = true;

        self.task = Some(cx.spawn(move |link| async move {
            if fresh {
                catalog.invalidate_genre(&id).await;
            }
            let loaded = catalog.genre(&id).await;

            link.post(move |store| {
                let Store {
                    pages, genres, cx, ..
                } = store;
                let Some(this) = pages
                    .genre
                    .as_mut()
                    .filter(|this| this.id.as_deref() == Some(id.as_str()))
                else {
                    return;
                };

                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(detail) => this.adopt(&id, detail, genres),
                    Err(reason) => this.error = Some(reason),
                }
            });
        }));
    }

    fn adopt(&mut self, id: &str, detail: Arc<GenreDetail>, genres: &mut Genres) {
        let cover = detail.cover();
        match detail.sections.is_empty() {
            true => genres.forget(id),
            false => genres.adopt(id, cover),
        }
        self.sections = Arc::new(detail.sections.clone());
        self.detail = Some(detail);
    }

    fn clear(&mut self) {
        self.task = None;
        self.id = None;
        self.detail = None;
        self.sections = Arc::new(Vec::new());
        self.loading = false;
        self.error = None;
    }
}

impl Store {
    /// Fetches the genres the search page browses, unless they are loaded or loading. Before a
    /// sign-in it only marks them wanted, and the sign-in fetches them.
    pub fn load_genres(&mut self) {
        self.genres.load(&self.session, &mut self.cx);
    }

    /// Opens the genre page on `id`, from the catalog cache when it already holds the genre.
    pub fn open_genre(&mut self, id: &str) {
        let Store {
            pages,
            session,
            genres,
            cx,
            ..
        } = self;
        pages
            .genre
            .get_or_insert_with(GenreDetails::new)
            .open(id, session, genres, cx);
    }

    /// Drops the genre page, and the load it has under way.
    pub fn close_genre(&mut self) {
        self.pages.genre = None;
    }

    /// Drops the genre page's sections and fetches them again from the provider, past the
    /// catalog cache.
    pub fn refresh_genre(&mut self) {
        let Store {
            pages, session, cx, ..
        } = self;
        if let Some(genre) = pages.genre.as_mut() {
            genre.refresh(session, cx);
        }
    }

    pub(crate) fn react_genre(&mut self, change: &Change) {
        let Change::Event(Event::Session(event)) = change else {
            return;
        };
        let Store {
            pages,
            session,
            genres,
            cx,
            ..
        } = self;
        genres.session_event(event, session, cx);
        let Some(this) = pages.genre.as_mut() else {
            return;
        };
        match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone() {
                    this.clear();
                    this.open(&id, session, genres, cx);
                }
            }
            SessionEvent::SignedOut => this.clear(),
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        }
    }
}

/// The genre pictures stored for the provider `slug`, or none when they are missing, unreadable
/// or older than `KEEP_COVERS`. It blocks on sqlite.
fn remembered(cache: &storage::Cache, slug: &str) -> Covers {
    let stored = cache.read(&covers_key(slug), |text| {
        serde_json::from_str::<Stored>(text).ok()
    });
    match stored {
        Ok(Some(Some(stored))) if now().saturating_sub(stored.saved) < KEEP_COVERS.as_secs() => {
            stored.covers
        }
        Ok(_) => Covers::new(),
        Err(error) => {
            log::warn!("genres: {error:#}");
            Covers::new()
        }
    }
}

/// Stores the genre pictures found for the provider `slug`, replacing what was kept before. It
/// blocks on sqlite.
fn remember(cache: &storage::Cache, slug: &str, covers: &Covers) {
    let saved = now();
    let value = serde_json::json!({ "saved": saved, "covers": covers }).to_string();
    if let Err(error) = cache.write(&covers_key(slug), &value, saved as i64) {
        log::warn!("genres: {error:#}");
    }
}

fn covers_key(slug: &str) -> String {
    format!("genre-covers/{slug}")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
