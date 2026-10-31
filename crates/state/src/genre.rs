use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, Entity, Task};
use music::{Genre, GenreDetail, GenreSection, MusicApi};
use tokio::task::AbortHandle;

use crate::{Io, Session, SessionEvent, join};

/// How many bare genres get a picture fetched for them, which covers the first screens of tiles.
const DRESSED: usize = 48;
/// How many of those pictures are asked for at once.
const AT_ONCE: usize = 4;

pub struct Genres {
    genres: Rc<Vec<Genre>>,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
    /// Fetches the pictures of genres the provider listed without one.
    dressing: Option<Task<()>>,
}

impl Genres {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => this.load(cx),
            SessionEvent::SignedOut => {
                this.task = None;
                this.dressing = None;
                this.genres = Rc::new(Vec::new());
                this.loading = false;
                this.error = None;
                cx.notify();
            }
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            genres: Rc::new(Vec::new()),
            loading: false,
            error: None,
            session,
            io,
            task: None,
            dressing: None,
        }
    }

    pub fn genres(&self) -> Rc<Vec<Genre>> {
        self.genres.clone()
    }

    pub fn adopt(&mut self, id: &str, cover: Option<String>, cx: &mut Context<Self>) {
        let Some(cover) = cover else {
            return;
        };
        let Some(genre) = Rc::make_mut(&mut self.genres)
            .iter_mut()
            .find(|genre| genre.id == id && genre.cover.is_none())
        else {
            return;
        };

        genre.cover = Some(cover);
        cx.notify();
    }

    pub fn forget(&mut self, id: &str, cx: &mut Context<Self>) {
        let kept = self.genres.len();
        Rc::make_mut(&mut self.genres).retain(|genre| genre.id != id);
        if self.genres.len() != kept {
            cx.notify();
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.genres.is_empty() {
            return;
        }
        let Some(client) = self.session.read(cx).client() else {
            return;
        };

        self.loading = true;
        self.error = None;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let lister = client.clone();
            let loaded = join(io.spawn(async move { lister.genres().await })).await;

            this.update(cx, |this, cx| {
                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(genres) => {
                        this.genres = Rc::new(genres);
                        this.dress(client, cx);
                    }
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Fetches pictures for the first genres the provider listed without one, a few at a time,
    /// and puts each on its tile as it lands. A genre whose picture cannot be found keeps the
    /// placeholder.
    fn dress(&mut self, client: Arc<dyn MusicApi>, cx: &mut Context<Self>) {
        let bare: Vec<String> = self
            .genres
            .iter()
            .filter(|genre| genre.cover.is_none())
            .take(DRESSED)
            .map(|genre| genre.id.clone())
            .collect();
        if bare.is_empty() {
            return;
        }

        let io = self.io.clone();
        self.dressing = Some(cx.spawn(async move |this, cx| {
            for batch in bare.chunks(AT_ONCE) {
                let asked = batch.iter().map(|id| {
                    let client = client.clone();
                    let id = id.clone();
                    join(io.spawn(async move { client.genre_cover(&id).await }))
                });
                let found = futures::future::join_all(asked).await;
                let kept = this.update(cx, |this, cx| {
                    for (id, cover) in batch.iter().zip(found) {
                        match cover {
                            Ok(cover) => this.adopt(id, cover, cx),
                            Err(error) => log::debug!("genres: no picture for {id}: {error:#}"),
                        }
                    }
                });
                if kept.is_err() {
                    return;
                }
            }
        }));
    }
}

pub struct GenreDetails {
    id: Option<String>,
    detail: Option<Arc<GenreDetail>>,
    sections: Rc<Vec<GenreSection>>,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    genres: Entity<Genres>,
    io: Io,
    task: Option<Task<()>>,
    request: Option<AbortHandle>,
}

impl GenreDetails {
    pub fn new(
        session: Entity<Session>,
        genres: Entity<Genres>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone() {
                    this.clear();
                    this.open(&id, cx);
                }
            }
            SessionEvent::SignedOut => {
                this.clear();
                cx.notify();
            }
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            id: None,
            detail: None,
            sections: Rc::new(Vec::new()),
            loading: false,
            error: None,
            session,
            genres,
            io,
            task: None,
            request: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn name(&self) -> Option<&str> {
        self.detail.as_ref().map(|detail| detail.name.as_str())
    }

    pub fn sections(&self) -> Rc<Vec<GenreSection>> {
        self.sections.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.id.as_deref() == Some(id) && (self.loading || self.detail.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());
        let Some(catalog) = self.session.read(cx).catalog(id) else {
            cx.notify();
            return;
        };
        if let Some(detail) = catalog.peek_genre(id) {
            self.adopt(id, detail, cx);
            cx.notify();
            return;
        }
        self.load(id.to_owned(), false, cx);
    }

    /// Drops the page and fetches it again from the provider, past the catalog cache, so it
    /// shows its loading state until the new sections land.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.id = Some(id.clone());
        self.load(id, true, cx);
    }

    /// Fetches the genre's sections. A `fresh` load drops the cached genre first, so the
    /// provider is asked again.
    fn load(&mut self, id: String, fresh: bool, cx: &mut Context<Self>) {
        let Some(catalog) = self.session.read(cx).catalog(&id) else {
            cx.notify();
            return;
        };

        self.loading = true;
        cx.notify();

        let request = self.io.spawn({
            let id = id.clone();
            async move {
                if fresh {
                    catalog.invalidate_genre(&id).await;
                }
                catalog.genre(&id).await
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
                    Ok(detail) => this.adopt(&id, detail, cx),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn adopt(&mut self, id: &str, detail: Arc<GenreDetail>, cx: &mut Context<Self>) {
        let cover = detail.cover();
        self.genres
            .update(cx, |genres, cx| match detail.sections.is_empty() {
                true => genres.forget(id, cx),
                false => genres.adopt(id, cover, cx),
            });
        self.sections = Rc::new(detail.sections.clone());
        self.detail = Some(detail);
    }

    fn clear(&mut self) {
        self.task = None;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.id = None;
        self.detail = None;
        self.sections = Rc::new(Vec::new());
        self.loading = false;
        self.error = None;
    }
}
