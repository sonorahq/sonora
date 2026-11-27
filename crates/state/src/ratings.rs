use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, Context, Entity, Task};
use i18n::t;
use music::Track;
use tokio::sync::Semaphore;

use crate::{
    AppSettings, Io, Library, LibraryEvent, Outcome, Session, SessionEvent, StarRatings, Toast,
    Toasts, join,
};

const REFRESH_WAIT: Duration = Duration::from_millis(300);
/// How long library events are ignored after a save, as they echo our own write.
const ECHO_WAIT: Duration = Duration::from_secs(3);

/// The account's track ratings, in half-stars.
pub struct Ratings {
    session: Entity<Session>,
    library: Entity<Library>,
    /// Serialises reads and writes.
    turn: Arc<Semaphore>,
    values: HashMap<String, u8>,
    /// The rating playlists by id.
    playlists: HashMap<String, u8>,
    /// Tracks whose last save failed, to verify on the next one.
    dirty: HashSet<String>,
    /// When the last save finished.
    saved_at: Option<Instant>,
    loaded: bool,
    failed: bool,
    /// The settings at the last notify, to skip unrelated changes.
    shown: StarRatings,
    load: Option<Task<()>>,
    refresh: Option<Task<()>>,
    refresh_due: bool,
    /// In-flight saves by track id.
    saves: HashMap<String, (Option<u8>, Task<()>)>,
}

impl Ratings {
    /// Follows the session, library and settings, loading ratings when enabled.
    pub fn new(
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        library: Entity<Library>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| {
            match event {
                SessionEvent::SignedIn | SessionEvent::SignedOut => {
                    this.values.clear();
                    this.playlists.clear();
                    this.dirty.clear();
                    this.saves.clear();
                    this.load = None;
                    this.refresh = None;
                    this.refresh_due = false;
                    this.loaded = false;
                    this.retry(cx);
                }
                SessionEvent::WentOffline => this.refresh = None,
                _ => {}
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(&library, |this, _, event, cx| {
            let changed = match event {
                // Emitted on reconnects and manual refreshes.
                LibraryEvent::Refreshing => return this.schedule_refresh(cx),
                LibraryEvent::PlaylistChanged(id)
                | LibraryEvent::PlaylistGone(id)
                | LibraryEvent::PlaylistBack(id)
                | LibraryEvent::TrackAdded { playlist: id }
                | LibraryEvent::TrackDropped { playlist: id, .. } => {
                    this.playlists.contains_key(id)
                }
                _ => false,
            };
            if changed
                && this.saves.is_empty()
                && this.saved_at.is_none_or(|at| at.elapsed() >= ECHO_WAIT)
            {
                this.schedule_refresh(cx);
            }
        })
        .detach();
        cx.observe(&settings, |this, settings, cx| {
            let shown = settings.read(cx).star_ratings();
            if this.shown == shown {
                return;
            }
            this.shown = shown;
            if !this.loaded && !this.failed {
                this.reload(cx);
            }
            cx.notify();
        })
        .detach();
        Self {
            shown: settings.read(cx).star_ratings(),
            session,
            library,
            turn: Arc::new(Semaphore::new(1)),
            values: HashMap::new(),
            playlists: HashMap::new(),
            dirty: HashSet::new(),
            saved_at: None,
            loaded: false,
            failed: false,
            load: None,
            refresh: None,
            refresh_due: false,
            saves: HashMap::new(),
        }
    }

    /// Whether the provider can store ratings.
    pub fn supported(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        session.authenticated() && session.client().is_some()
    }

    /// Whether the provider takes half stars.
    pub fn half_stars(&self, cx: &App) -> bool {
        self.session
            .read(cx)
            .client()
            .is_none_or(|api| api.half_star_ratings())
    }

    /// Whether half stars are on and supported.
    pub fn halves(&self, cx: &App) -> bool {
        self.shown.half_ratings && self.half_stars(cx)
    }

    /// Whether `rating` likes the track, or `None` with auto-like off.
    fn likes(&self, rating: u8) -> Option<bool> {
        self.shown.auto_like_at().map(|at| rating >= at)
    }

    /// Whether ratings are supported and turned on.
    pub fn enabled(&self, cx: &App) -> bool {
        self.shown.enabled && self.supported(cx)
    }

    /// Whether `track` can be rated.
    pub fn can_rate(&self, track: &Track, cx: &App) -> bool {
        self.enabled(cx) && rating_id(track).is_some()
    }

    /// The rating of `id`, including one being saved.
    pub fn for_track(&self, id: &str) -> Option<u8> {
        match self.saves.get(id) {
            Some((rating, _)) => *rating,
            None => self.values.get(id).copied(),
        }
    }

    /// Whether a save for `id` is in flight.
    pub fn pending(&self, id: &str) -> bool {
        self.saves.contains_key(id)
    }

    /// The like state a hovered rating previews.
    pub fn preview_like(&self, id: &str, hover: Option<u8>) -> Option<bool> {
        hover
            .filter(|_| !self.loading() && !self.failed && !self.pending(id))
            .and_then(|value| self.likes(value))
    }

    /// Whether the playlist stores ratings.
    pub fn is_rating_playlist(&self, id: &str) -> bool {
        self.playlists.contains_key(id)
    }

    /// Whether the first load is still running.
    pub fn loading(&self) -> bool {
        !self.loaded && self.load.is_some()
    }

    /// Whether the last load failed.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Loads again after a failure.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.failed = false;
        self.reload(cx);
    }

    /// Rates a track in half-stars, or clears it when `rating` is the current one.
    pub fn rate(&mut self, track: Track, rating: u8, cx: &mut Context<Self>) {
        let Some(id) = rating_id(&track) else {
            return;
        };
        let wanted = (self.values.get(id).copied() != Some(rating)).then_some(rating);
        self.set(track, wanted, None, cx);
    }

    /// Writes a rating and offers Undo, which also restores the like.
    fn set(
        &mut self,
        track: Track,
        wanted: Option<u8>,
        restore_like: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = track.id.clone().filter(|id| !self.pending(id)) else {
            return;
        };
        let Some(api) = self.session.read(cx).client() else {
            return;
        };
        // Undo still works with ratings off.
        if !self.supported(cx) || !self.loaded || (restore_like.is_none() && !self.enabled(cx)) {
            return;
        }
        let previous = self.values.get(&id).copied();
        if previous == wanted {
            return;
        }
        let previous_like = self.library.read(cx).saved(&id);
        let turn = self.turn.clone();
        let pending_id = id.clone();
        let task = cx.spawn(async move |this, cx| {
            // Waiting our turn stops two saves creating the same playlist.
            let Ok(_turn) = turn.acquire_owned().await else {
                return;
            };
            let Ok(request) = this.update(cx, |this, cx| {
                let playlists = this.playlists.clone();
                let verify = this.dirty.contains(&id);
                let id = id.clone();
                Io::global(cx).spawn(async move {
                    api.set_track_rating(&id, wanted, previous, &playlists, verify)
                        .await
                })
            }) else {
                return;
            };
            let result = join(request).await;
            // Wait for any auto-like write to settle.
            if result.is_ok() {
                while let Ok(Some(settled)) = this.update(cx, |this, cx| {
                    Library::pending_favorite(&this.library, &id, cx)
                }) {
                    settled.await;
                }
            }
            this.update(cx, |this, cx| {
                this.saves.remove(&id);
                this.saved_at = Some(Instant::now());
                this.playlists = match result {
                    Ok(playlists) => {
                        this.dirty.remove(&id);
                        playlists
                    }
                    Err(error) => {
                        log::warn!("ratings: cannot save a rating: {error:#}");
                        Toasts::post(Toast::new(Outcome::Failed, "toast-rating-save-failed"), cx);
                        // It may have half applied, so verify next time.
                        this.dirty.insert(id.clone());
                        this.reload(cx);
                        cx.notify();
                        return;
                    }
                };
                match wanted {
                    Some(rating) => this.values.insert(id.clone(), rating),
                    None => this.values.remove(&id),
                };
                let like = restore_like
                    .or_else(|| wanted.and_then(|rating| this.likes(rating)))
                    .filter(|want| this.library.read(cx).saved(&id) != *want);
                if like.is_some() {
                    this.library
                        .update(cx, |library, cx| library.toggle(track.clone(), cx));
                }
                if restore_like.is_none() {
                    let key = match wanted {
                        Some(_) => "toast-rating-updated",
                        None => "toast-rating-cleared",
                    };
                    let stars = |rating: u8| t!("unit-stars", value = f64::from(rating) / 2.);
                    let current = wanted.map(stars).unwrap_or_else(|| t!("common-off"));
                    let previous_stars = previous.map(stars).unwrap_or_default();
                    let detail = t!(
                        "toast-rating-detail",
                        previous = &previous_stars,
                        current = &current,
                        changed = if previous.is_some() { "yes" } else { "no" },
                        favorite = match like {
                            Some(true) => "liked",
                            Some(false) => "unliked",
                            None => "unchanged",
                        }
                    );
                    let toast = Toast::new(Outcome::Done, key)
                        .name(track.name.clone())
                        .detail(detail);
                    let ratings = cx.weak_entity();
                    let toast = toast.undo(move |cx| {
                        ratings
                            .update(cx, |ratings, cx| {
                                // Skip if a newer rating replaced this one.
                                if ratings.for_track(&id) == wanted {
                                    ratings.set(track.clone(), previous, Some(previous_like), cx);
                                }
                            })
                            .ok();
                    });
                    Toasts::post(toast, cx);
                }
                this.finish_operation(cx);
            })
            .ok();
        });
        self.saves.insert(pending_id, (wanted, task));
        cx.notify();
    }

    /// Debounces provider events into one reload.
    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.enabled(cx) {
            return;
        }
        self.refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_WAIT).await;
            this.update(cx, |this, cx| {
                this.refresh = None;
                this.retry(cx);
            })
            .ok();
        }));
    }

    /// Loads every rating, deferred while a load or save runs.
    fn reload(&mut self, cx: &mut Context<Self>) {
        if !self.enabled(cx) {
            return;
        }
        if self.load.is_some() || !self.saves.is_empty() {
            self.refresh_due = true;
            return;
        }
        self.refresh_due = false;
        let Some(api) = self.session.read(cx).client() else {
            return;
        };
        let turn = self.turn.clone();
        let request = Io::global(cx).spawn(async move {
            let _turn = turn.acquire().await?;
            api.track_ratings().await
        });
        self.load = Some(cx.spawn(async move |this, cx| {
            let result = join(request).await;
            this.update(cx, |this, cx| {
                this.load = None;
                this.failed = result.is_err();
                match result {
                    Ok((values, playlists)) => {
                        this.values = values;
                        this.playlists = playlists;
                        this.loaded = true;
                    }
                    Err(error) => log::warn!("ratings: cannot load ratings: {error:#}"),
                }
                this.finish_operation(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    /// Runs any deferred reload, then notifies.
    fn finish_operation(&mut self, cx: &mut Context<Self>) {
        if self.refresh_due {
            self.retry(cx);
        }
        cx.notify();
    }
}

fn rating_id(track: &Track) -> Option<&str> {
    track
        .id
        .as_deref()
        .filter(|id| !id.is_empty() && !music::is_local_id(id))
}
