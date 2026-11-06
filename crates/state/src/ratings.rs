use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, ensure};
use futures::{TryStreamExt as _, stream};
use gpui::{App, Context, Entity, Task};
use i18n::t;
use music::{MusicApi, Track};
use tokio::sync::Semaphore;

use crate::{
    AppSettings, Io, Library, LibraryEvent, Outcome, Session, SessionEvent, StarRatings, Toast,
    Toasts, join,
};

/// Caps concurrent playlist reads so large rating libraries do not flood the provider.
const READS: usize = 4;
const REFRESH_WAIT: Duration = Duration::from_millis(300);
const LIKE_WAIT: Duration = Duration::from_millis(100);

/// The signed-in account's track ratings, in half-stars.
pub struct Ratings {
    session: Entity<Session>,
    library: Entity<Library>,
    /// Runs this entity's reads and writes one at a time, in order.
    turn: Arc<Semaphore>,
    values: HashMap<String, u8>,
    playlists: HashSet<String>,
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
                    this.refresh = None;
                    this.refresh_due = false;
                    this.loaded = false;
                    this.retry(cx);
                }
                SessionEvent::Reconnected => this.schedule_refresh(cx),
                SessionEvent::WentOffline => this.refresh = None,
                _ => {}
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(&library, |this, _, event, cx| {
            let changed = match event {
                LibraryEvent::Refreshing => true,
                LibraryEvent::PlaylistChanged(id)
                | LibraryEvent::PlaylistGone(id)
                | LibraryEvent::PlaylistBack(id)
                | LibraryEvent::TrackAdded { playlist: id }
                | LibraryEvent::TrackDropped { playlist: id, .. } => this.playlists.contains(id),
                _ => false,
            };
            if changed {
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
            playlists: HashSet::new(),
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

    /// The like state a hovered rating previews, unless ratings are loading or saving.
    pub fn preview_like(&self, id: &str, hover: Option<u8>) -> Option<bool> {
        hover
            .filter(|_| !self.loading() && !self.failed && !self.pending(id))
            .zip(self.shown.auto_like)
            .map(|(value, at)| value >= at)
    }

    /// Whether a playlist holds ratings for the signed-in account.
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
        if !self.enabled(cx) || !self.loaded {
            return;
        }
        let previous = self.values.get(&id).copied();
        if previous == wanted {
            return;
        }
        let previous_like = self.library.read(cx).saved(&id);
        let request = Io::global(cx).spawn({
            let id = id.clone();
            let turn = self.turn.clone();
            async move {
                let _turn = turn.acquire().await?;
                save_rating(api.as_ref(), &id, wanted).await
            }
        });
        let pending_id = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = join(request).await;
            // Undo may finish before the original auto-like write; wait without polling the network.
            while result.is_ok()
                && this
                    .update(cx, |this, cx| this.library.read(cx).pending(&id))
                    .unwrap_or(false)
            {
                cx.background_executor().timer(LIKE_WAIT).await;
            }
            this.update(cx, |this, cx| {
                this.saves.remove(&id);
                this.playlists = match result {
                    Ok(playlists) => playlists,
                    Err(error) => {
                        log::warn!("ratings: cannot save a rating: {error:#}");
                        Toasts::post(Toast::new(Outcome::Failed, "ratings-save-failed"), cx);
                        // The save may have half applied, so reload.
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
                    .or_else(|| {
                        wanted
                            .zip(this.shown.auto_like)
                            .map(|(rating, at)| rating >= at)
                    })
                    .filter(|want| this.library.read(cx).saved(&id) != *want);
                if like.is_some() {
                    this.library
                        .update(cx, |library, cx| library.toggle(track.clone(), cx));
                }
                if restore_like.is_none() {
                    let key = match (wanted, like) {
                        (None, _) => "toast-rating-cleared",
                        (_, Some(true)) => "toast-rating-liked",
                        (_, Some(false)) => "toast-rating-unliked",
                        _ => "toast-rating-updated",
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
                if this.refresh_due {
                    this.retry(cx);
                }
                cx.notify();
            })
            .ok();
        });
        self.saves.insert(pending_id, (wanted, task));
        cx.notify();
    }

    /// Coalesces provider events and waits for active work before reading ratings again.
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

    /// Loads every rating, unless a load is already running.
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
            load_ratings(api.as_ref()).await
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
                if this.refresh_due {
                    this.retry(cx);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

/// Reads ratings through the provider's playlist hooks. Newer additions win when available;
/// otherwise the higher rating breaks ties consistently.
async fn load_ratings(api: &dyn MusicApi) -> Result<(HashMap<String, u8>, HashSet<String>)> {
    let playlists = api.rating_playlists().await?;
    let ids = playlists.iter().map(|(id, _)| id.clone()).collect();
    let mut values = HashMap::new();
    for (id, rating) in playlists {
        for (id, added_at) in api.playlist_entries(&id).await? {
            let value = (added_at.unwrap_or_default(), rating);
            let entry = values.entry(id).or_insert(value);
            *entry = (*entry).max(value);
        }
    }
    Ok((
        values
            .into_iter()
            .map(|(id, (_, rating))| (id, rating))
            .collect(),
        ids,
    ))
}

/// Adds the new rating before removing old ones, so a failed write can be retried safely.
async fn save_rating(
    api: &dyn MusicApi,
    track_id: &str,
    rating: Option<u8>,
) -> Result<HashSet<String>> {
    ensure!(
        rating.is_none_or(|value| (1..=10).contains(&value)),
        "invalid half-star rating"
    );
    let playlists = api.rating_playlists().await?;
    let mut ids: HashSet<String> = playlists.iter().map(|(id, _)| id.clone()).collect();
    if let Some(rating) = rating {
        let id = match playlists.iter().find(|(_, value)| *value == rating) {
            Some((id, _)) => id.clone(),
            None => api.create_rating_playlist(rating).await?,
        };
        ids.insert(id.clone());
        if !contains(api, &id, track_id).await? {
            api.add_track_to_playlist(&id, track_id).await?;
        }
    }
    stream::iter(playlists.into_iter().map(Ok::<_, anyhow::Error>))
        .try_for_each_concurrent(READS, |(id, value)| async move {
            if Some(value) != rating && contains(api, &id, track_id).await? {
                api.remove_track_from_playlist(&id, track_id).await?;
            }
            Ok(())
        })
        .await?;
    Ok(ids)
}

/// Checks membership before a write, so retries neither duplicate tracks nor remove absent ones.
async fn contains(api: &dyn MusicApi, playlist: &str, track: &str) -> Result<bool> {
    Ok(api
        .playlist_entries(playlist)
        .await?
        .iter()
        .any(|(id, _)| id == track))
}
