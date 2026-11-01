use gpui::{App, Context, Entity, Task};
use i18n::t;
use music::Track;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::{
    AppSettings, Io, Library, Outcome, Session, SessionEvent, StarRatings, Toast, Toasts, join,
};

/// The signed-in account's track ratings, in half-stars.
pub struct Ratings {
    session: Entity<Session>,
    library: Entity<Library>,
    /// Runs this entity's reads and writes one at a time, in order.
    turn: Arc<Semaphore>,
    values: HashMap<String, u8>,
    /// The playlists that hold the ratings, which the Add to playlist menu leaves out.
    playlists: HashSet<String>,
    loaded: bool,
    failed: bool,
    /// The rating settings at the last notify, to ignore unrelated settings changes.
    shown: StarRatings,
    load: Option<Task<()>>,
    /// In-flight saves by track id, with the rating being written.
    saves: HashMap<String, (Option<u8>, Task<()>)>,
}

impl Ratings {
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
                    this.saves.clear();
                    this.load = None;
                    this.loaded = false;
                    this.retry(cx);
                }
                SessionEvent::Reconnected if this.failed => this.retry(cx),
                _ => {}
            }
            cx.notify();
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
            playlists: HashSet::new(),
            loaded: false,
            failed: false,
            load: None,
            saves: HashMap::new(),
        }
    }

    /// Whether the signed-in provider can store ratings.
    pub fn supported(&self, cx: &App) -> bool {
        let session = self.session.read(cx);
        session.authenticated() && session.client().is_some_and(|api| api.supports_ratings())
    }

    /// Whether ratings are supported and turned on.
    pub fn enabled(&self, cx: &App) -> bool {
        self.shown.enabled && self.supported(cx)
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

    /// Whether `id` names one of the playlists that hold the ratings.
    pub fn is_rating_playlist(&self, id: &str) -> bool {
        self.playlists.contains(id)
    }

    pub fn loading(&self) -> bool {
        !self.loaded && self.load.is_some()
    }

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
        let Some(id) = track.id.as_deref() else {
            return;
        };
        let wanted = (self.values.get(id).copied() != Some(rating)).then_some(rating);
        self.set(track, wanted, true, cx);
    }

    /// Writes `wanted` as the track's rating, or clears it. `announce` offers an Undo that puts
    /// the previous rating back, which the Undo itself does not repeat.
    fn set(&mut self, track: Track, wanted: Option<u8>, announce: bool, cx: &mut Context<Self>) {
        let Some(id) = track.id.clone().filter(|id| !self.pending(id)) else {
            return;
        };
        let Some(api) = self.session.read(cx).client() else {
            return;
        };
        if !self.enabled(cx) || !self.loaded {
            return;
        }
        let previous = self.values.get(&id).copied();
        if previous == wanted {
            return;
        }
        let request = Io::global(cx).spawn({
            let id = id.clone();
            let turn = self.turn.clone();
            async move {
                let _turn = turn.acquire().await?;
                api.set_rating(&id, wanted).await
            }
        });
        let task = cx.spawn({
            let id = id.clone();
            async move |this, cx| {
                let result = join(request).await;
                this.update(cx, |this, cx| {
                    this.saves.remove(&id);
                    if let Err(error) = result {
                        log::warn!("ratings: cannot save a rating: {error:#}");
                        Toasts::post(Toast::new(Outcome::Failed, "ratings-save-failed"), cx);
                        // The save may have half applied, so reload.
                        this.reload(cx);
                        cx.notify();
                        return;
                    }
                    match wanted {
                        Some(rating) => this.values.insert(id.clone(), rating),
                        None => this.values.remove(&id),
                    };
                    let like = wanted
                        .zip(this.shown.auto_like)
                        .map(|(rating, at)| rating >= at);
                    let like = like.filter(|want| this.library.read(cx).saved(&id) != *want);
                    if like.is_some() {
                        this.library
                            .update(cx, |library, cx| library.toggle(track.clone(), cx));
                    }
                    if announce {
                        let key = match wanted {
                            None => "toast-rating-cleared",
                            Some(_) => "toast-rating-updated",
                        };
                        let like = match like {
                            Some(true) => "liked",
                            Some(false) => "unliked",
                            None => "unchanged",
                        };
                        let stars =
                            |rating: u8| t!("ratings-threshold", rating = f64::from(rating) / 2.);
                        let current = wanted.map(stars).unwrap_or_else(|| t!("common-off"));
                        let detail = match previous {
                            Some(from) => {
                                let previous = stars(from);
                                t!("ratings-change", previous = &previous, current = &current)
                            }
                            None => current,
                        };
                        let toast = Toast::new(Outcome::Done, key)
                            .name(track.name.clone())
                            .arg("like", like)
                            .detail(detail);
                        let ratings = cx.weak_entity();
                        let toast = toast.undo(move |cx| {
                            ratings
                                .update(cx, |ratings, cx| {
                                    // A newer rating has its own toast, so this one leaves it alone.
                                    if ratings.for_track(&id) == wanted {
                                        ratings.set(track.clone(), previous, false, cx);
                                    }
                                })
                                .ok();
                        });
                        Toasts::post(toast, cx);
                    }
                    cx.notify();
                })
                .ok();
            }
        });
        self.saves.insert(id, (wanted, task));
        cx.notify();
    }

    /// Loads every rating, unless a load is already running.
    fn reload(&mut self, cx: &mut Context<Self>) {
        if !self.enabled(cx) || self.load.is_some() {
            return;
        }
        let Some(api) = self.session.read(cx).client() else {
            return;
        };
        let turn = self.turn.clone();
        let request = Io::global(cx).spawn(async move {
            let _turn = turn.acquire().await?;
            api.ratings().await
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
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}
