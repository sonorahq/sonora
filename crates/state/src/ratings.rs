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
/// How long after a save the library events that echo our own writes are ignored.
const ECHO_WAIT: Duration = Duration::from_secs(3);

/// The signed-in account's track ratings, in half-stars.
pub struct Ratings {
    session: Entity<Session>,
    library: Entity<Library>,
    /// Runs this entity's reads and writes one at a time, in order.
    turn: Arc<Semaphore>,
    values: HashMap<String, u8>,
    /// The rating playlists by id, with their value in half-stars.
    playlists: HashMap<String, u8>,
    /// Tracks whose last save failed, which may be half applied and are checked in full next.
    dirty: HashSet<String>,
    /// When the last save finished, to tell our own library events from another client's.
    saved_at: Option<Instant>,
    loaded: bool,
    failed: bool,
    /// The rating settings at the last notify, to ignore unrelated settings changes.
    shown: StarRatings,
    load: Option<Task<()>>,
    refresh: Option<Task<()>>,
    refresh_due: bool,
    /// In-flight saves by track id, with the rating being written.
    saves: HashMap<String, (Option<u8>, Task<()>)>,
}

impl Ratings {
    /// Follows the session, library and settings, loading the ratings once they are on and the
    /// provider can hold them.
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
                // The library emits this for both reconnects and manual refreshes.
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

    /// Whether the signed-in provider can store ratings.
    pub fn supported(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        session.authenticated() && session.client().is_some()
    }

    /// Whether the provider takes ratings between whole stars.
    pub fn half_stars(&self, cx: &App) -> bool {
        self.session
            .read(cx)
            .client()
            .is_none_or(|api| api.half_star_ratings())
    }

    /// Whether a rating can be half a star: the listener allows it and the provider takes it.
    pub fn halves(&self, cx: &App) -> bool {
        self.shown.half_ratings && self.half_stars(cx)
    }

    /// Whether rating a track `rating` half-stars likes it, or `None` with auto-like off.
    fn likes(&self, rating: u8) -> Option<bool> {
        self.shown.auto_like_at().map(|at| rating >= at)
    }

    /// Whether ratings are supported and turned on.
    pub fn enabled(&self, cx: &App) -> bool {
        self.shown.enabled && self.supported(cx)
    }

    /// Whether this track can be rated on the current streaming account.
    pub fn can_rate(&self, track: &Track, cx: &App) -> bool {
        self.enabled(cx) && rating_id(track).is_some()
    }

    /// The rating to show for `id`, including one still being saved.
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

    /// The like state a hovered rating previews, unless ratings are loading or saving.
    pub fn preview_like(&self, id: &str, hover: Option<u8>) -> Option<bool> {
        hover
            .filter(|_| !self.loading() && !self.failed && !self.pending(id))
            .and_then(|value| self.likes(value))
    }

    /// Whether a playlist holds ratings for the signed-in account.
    pub fn is_rating_playlist(&self, id: &str) -> bool {
        self.playlists.contains_key(id)
    }

    /// Whether the first read of the ratings is still running.
    pub fn loading(&self) -> bool {
        !self.loaded && self.load.is_some()
    }

    /// Whether the last read of the ratings failed, until `retry` loads them again.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Clears a failed load and loads again.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.failed = false;
        self.reload(cx);
    }

    /// Rates a streaming track in half-stars, or clears it when `rating` is its current rating.
    /// With auto-like on, a rating at or above the threshold likes the track and one below unlikes
    /// it. Clearing a rating leaves the like alone.
    pub fn rate(&mut self, track: Track, rating: u8, cx: &mut Context<Self>) {
        let Some(id) = rating_id(&track) else {
            return;
        };
        let wanted = (self.values.get(id).copied() != Some(rating)).then_some(rating);
        self.set(track, wanted, None, cx);
    }

    /// Writes a rating and offers Undo. An Undo restores the previous like state too,
    /// bypasses auto-like, and does not offer another Undo.
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
        // An Undo still works after ratings are turned off, since it puts back what was changed.
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
            // The playlists are read once the saves before this one have published, so two
            // saves never both create the same rating playlist.
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
            // Undo may finish before the original auto-like write; wait without polling the network.
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
                        // The save may have half applied, so check it in full and reload.
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
                                // A newer rating has its own toast, so this one leaves it alone.
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

    /// Coalesces provider events into one read of the ratings, once a short wait has passed.
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

    /// Loads every rating. A load or a save already running defers it, and it runs when that
    /// one ends.
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

    /// Runs a refresh deferred by a load or save and publishes the completed operation.
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
