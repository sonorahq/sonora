use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Entity, Task};
use music::lyrics::LOCAL;
use music::{LyricsHit, LyricsProvider, LyricsQuery, Track, TrackKey};
use tokio::task::JoinSet;

use crate::choices::{Choice, Choices};
use crate::sheets::{Sheets, Unsaved};
use crate::{AppSettings, Io, Outcome, Playback, Queue, QueuePosition, Session, Toasts, join};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsState {
    Idle,
    Loading,
    Ready,
    Instrumental,
    Missing,
    Failed(String),
}

/// Where a search the listener typed by hand stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SearchState {
    #[default]
    Idle,
    Searching,
    Ready,
    /// No enabled source can search by title and artist, or the track's metadata may not go
    /// online.
    Unsupported,
    Offline,
    Failed(String),
}

/// A search the listener typed by hand: what it asked, what it found, and the task asking.
#[derive(Default)]
struct ManualSearch {
    state: SearchState,
    title: String,
    artist: String,
    hits: Vec<LyricsHit>,
    task: Option<Task<()>>,
}

const SAVE_DELAY: Duration = Duration::from_millis(800);

/// How many tracks keep their answer in memory. The rest are read back from the cache database,
/// so this only has to cover the track playing, the next one and a few just played.
const RECENT: usize = 16;

pub struct Lyrics {
    state: LyricsState,
    hits: Vec<LyricsHit>,
    chosen: usize,
    picked: bool,
    settled: bool,
    revision: u64,
    following: Option<String>,
    /// The answers looked at most recently, newest first.
    recent: VecDeque<(String, Found)>,
    store: Sheets,
    /// Answers found since the last save, written together once lookups go quiet.
    unsaved: Vec<Unsaved>,
    providers: Vec<Arc<dyn LyricsProvider>>,
    choices: Choices,
    /// The manual matches on file, by track key, read once at start and kept current here.
    bindings: HashMap<String, Choice>,
    /// The matches bound or dropped while the on-disk read is still in flight. They win when
    /// it lands, so a match made the moment the app started is not erased by the read begun
    /// before it, nor a dropped one brought back.
    dirty: HashMap<String, Option<Choice>>,
    /// The queued write to the matches on file. Binds and unbinds write one after another, in
    /// the order they were made, so a restart shows the last choice made, never an earlier
    /// one that was still on its way to disk.
    persist: Option<Task<()>>,
    /// The match bound to the track on screen, if any.
    bound: Option<Choice>,
    manual: ManualSearch,
    /// Bumped every time a manual search is replaced or dropped, so a late answer to an old
    /// search writes nothing.
    manual_generation: u64,
    enabled_providers: Vec<String>,
    /// The order of the sources the answer on show was ranked by. A new order looks the track
    /// up again, from what is held, so the source now ranked first is the one shown.
    priority: Vec<String>,
    playback: Entity<Playback>,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
    io: Io,
    task: Option<Task<()>>,
    ahead: Option<Task<()>>,
    ahead_of: Option<String>,
    save: Option<Task<()>>,
    /// Moves the old lyrics.json into the cache database, once.
    _migration: Task<()>,
}

impl Lyrics {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        providers: Vec<Arc<dyn LyricsProvider>>,
        io: Io,
        database: storage::Database,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&playback, |this, _, cx| {
            this.follow(cx);
            this.prefetch(cx);
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.prefetch(cx)).detach();
        cx.observe(&settings, |this, settings, cx| {
            let settings = settings.read(cx);
            let enabled = settings.lyrics_providers();
            let priority = settings.lyrics_provider_priority();
            if enabled == this.enabled_providers && priority == this.priority {
                return;
            }
            this.enabled_providers = enabled.to_vec();
            this.priority = priority;
            this.task = None;
            this.ahead = None;
            this.ahead_of = None;
            this.forget(cx);
            this.follow(cx);
        })
        .detach();
        let enabled_providers = settings.read(cx).lyrics_providers().to_vec();
        let priority = settings.read(cx).lyrics_provider_priority();
        let store = Sheets::new(storage::Cache::standard());
        let moving = store.clone();
        let migration = cx
            .background_executor()
            .spawn(async move { moving.migrate() });
        let choices = Choices::new(database);
        let loading = choices.clone();
        // The matches picked by hand are read once, then kept current as they are made. A
        // track that started playing before they were in is followed again so its match
        // shows without a restart of the song.
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { loading.load() })
                .await;
            this.update(cx, |this, cx| {
                merge_choices(loaded, &mut this.bindings, &this.dirty);
                this.dirty.clear();
                this.following = None;
                this.follow(cx);
            })
            .ok();
        })
        .detach();
        Self {
            state: LyricsState::Idle,
            hits: Vec::new(),
            chosen: 0,
            picked: false,
            settled: false,
            revision: 0,
            following: None,
            recent: VecDeque::new(),
            store,
            unsaved: Vec::new(),
            providers,
            choices,
            bindings: HashMap::new(),
            dirty: HashMap::new(),
            persist: None,
            bound: None,
            manual: ManualSearch::default(),
            manual_generation: 0,
            enabled_providers,
            priority,
            playback,
            session,
            settings,
            io,
            task: None,
            ahead: None,
            ahead_of: None,
            save: None,
            _migration: migration,
        }
    }

    pub fn state(&self) -> &LyricsState {
        &self.state
    }

    pub fn following(&self) -> Option<&str> {
        self.following.as_deref()
    }

    pub fn hits(&self) -> &[LyricsHit] {
        &self.hits
    }

    pub fn current(&self) -> Option<&LyricsHit> {
        self.hits.get(self.chosen)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.hits.len() || index == self.chosen {
            return;
        }
        self.chosen = index;
        self.picked = true;
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    /// Where the manual search for the track on screen stands.
    pub fn search_state(&self) -> &SearchState {
        &self.manual.state
    }

    /// What the manual search found, best first as the listener typed it.
    pub fn search_hits(&self) -> &[LyricsHit] {
        &self.manual.hits
    }

    /// The match bound to the track on screen: the searched title and artist, and the sheet.
    pub fn manual_match(&self) -> Option<(String, String, LyricsHit)> {
        let bound = self.bound.as_ref()?;
        let (title, artist) = bound.searched();
        Some((title.to_owned(), artist.to_owned(), self.bound_hit()?))
    }

    /// Searches the enabled sources for lyrics to `title` and `artist`, typed by the listener,
    /// for the track on screen. This replaces any earlier manual search, and nothing it finds
    /// changes what is shown until the listener picks a match with [`Self::bind`]. Sources are
    /// asked without the track's identity: a provider keyed to the song id would answer for
    /// the recording the listener already knows is wrong, and one that cannot search by title
    /// and artist is not asked at all.
    pub fn search_manually(&mut self, title: String, artist: String, cx: &mut Context<Self>) {
        self.manual_generation = self.manual_generation.wrapping_add(1);
        let generation = self.manual_generation;
        self.manual.task = None;
        self.manual.hits.clear();
        self.manual.title = title.trim().to_owned();
        self.manual.artist = artist.trim().to_owned();
        self.manual.state = SearchState::Idle;

        let Some(id) = self.following.clone() else {
            cx.notify();
            return;
        };
        if self.manual.title.is_empty() && self.manual.artist.is_empty() {
            cx.notify();
            return;
        }
        // A local file's metadata goes online only with the listener's leave.
        if !self.online(&id, cx) {
            self.manual.state = SearchState::Unsupported;
            cx.notify();
            return;
        }
        if crate::Network::offline(cx) {
            self.manual.state = SearchState::Offline;
            cx.notify();
            return;
        }
        let providers: Vec<Arc<dyn LyricsProvider>> = self
            .providers
            .iter()
            .filter(|provider| provider.text_search())
            .filter(|provider| {
                self.settings
                    .read(cx)
                    .lyrics_provider_enabled(provider.name())
            })
            .cloned()
            .collect();
        if providers.is_empty() {
            self.manual.state = SearchState::Unsupported;
            cx.notify();
            return;
        }
        let duration = self
            .playback
            .read(cx)
            .shown_track(cx)
            .map(|track| track.duration)
            .unwrap_or_default();
        let query = LyricsQuery {
            title: self.manual.title.clone(),
            artist: self.manual.artist.clone(),
            album: None,
            duration,
            track: None,
        };
        self.manual.state = SearchState::Searching;
        cx.notify();

        let io = self.io.clone();
        self.manual.task = Some(cx.spawn(async move |this, cx| {
            let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel();
            let worker = io.spawn(gather_text(providers, query.clone(), sender));
            while let Some(mut found) = incoming.recv().await {
                let ranking = query.clone();
                this.update(cx, |this, cx| {
                    if this.manual_generation != generation {
                        return;
                    }
                    this.manual.hits.append(&mut found);
                    this.manual.hits =
                        music::lyrics::candidates(&ranking, std::mem::take(&mut this.manual.hits));
                    cx.notify();
                })
                .ok();
            }
            let answered = join(worker).await;
            this.update(cx, |this, cx| {
                if this.manual_generation != generation {
                    return;
                }
                this.manual.task = None;
                this.manual.state = match answered {
                    Ok(answered) => {
                        crate::Network::reached(cx);
                        match answered.is_empty() {
                            true => SearchState::Failed("no lyrics source answered".to_owned()),
                            false => SearchState::Ready,
                        }
                    }
                    Err(error) => SearchState::Failed(crate::blamed(&error, cx)),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    /// Drops the manual search in flight, if any, and what it found.
    pub fn cancel_search(&mut self, cx: &mut Context<Self>) {
        self.manual = ManualSearch::default();
        self.manual_generation = self.manual_generation.wrapping_add(1);
        cx.notify();
    }

    /// Binds the manual-search `hit` the listener picked to the track on screen. The hit
    /// itself is passed, not its position in the results: the list keeps reranking as late
    /// answers come in, and the pick must stay the sheet the listener previewed. It shows at
    /// once and stays the track's lyrics until [`Self::unbind`], through restarts, cache
    /// eviction and changes to the sources and their order.
    pub fn bind(&mut self, hit: LyricsHit, cx: &mut Context<Self>) {
        let Some(id) = self.following.clone() else {
            return;
        };
        let key = self.choice_key(&id, cx);
        let choice = Choice::of(&self.manual.title, &self.manual.artist, &hit);
        self.bound = Some(choice.clone());
        self.bindings.insert(key.clone(), choice.clone());
        self.dirty.insert(key.clone(), Some(choice.clone()));
        self.hits.retain(|held| !same(held, &hit));
        self.hits.insert(0, hit);
        self.chosen = 0;
        self.picked = true;
        self.settled = true;
        self.state = LyricsState::Ready;
        self.revision = self.revision.wrapping_add(1);
        cx.notify();

        let queued = self.persist.take();
        let choices = self.choices.clone();
        self.persist = Some(cx.spawn(async move |this, cx| {
            if let Some(queued) = queued {
                queued.await;
            }
            let saved = cx
                .background_executor()
                .spawn(async move { choices.save(&key, &choice) })
                .await;
            if let Err(error) = saved {
                log::warn!("lyrics: cannot save the picked match: {error:#}");
                this.update(cx, |_, cx| {
                    Toasts::show(Outcome::Failed, "lyrics-choice-save-failed", cx);
                })
                .ok();
            }
        }));
    }

    /// Drops the match bound to the track on screen and puts the automatic answer back.
    pub fn unbind(&mut self, cx: &mut Context<Self>) {
        if self.bound.is_none() {
            return;
        }
        let Some(id) = self.following.clone() else {
            return;
        };
        let key = self.choice_key(&id, cx);
        self.bound = None;
        self.bindings.remove(&key);
        self.dirty.insert(key.clone(), None);
        self.picked = false;
        self.chosen = 0;

        let queued = self.persist.take();
        let choices = self.choices.clone();
        self.persist = Some(cx.spawn(async move |this, cx| {
            if let Some(queued) = queued {
                queued.await;
            }
            let dropped = cx
                .background_executor()
                .spawn(async move { choices.drop(&key) })
                .await;
            if let Err(error) = dropped {
                log::warn!("lyrics: cannot drop the picked match: {error:#}");
                this.update(cx, |_, cx| {
                    Toasts::show(Outcome::Failed, "lyrics-choice-drop-failed", cx);
                })
                .ok();
            }
        }));

        // The automatic answer is held in memory or in the cache; following again shows it.
        self.following = None;
        self.follow(cx);
    }

    /// The hit the listener picked by hand for the track on screen, if its source is still
    /// known to this build.
    fn bound_hit(&self) -> Option<LyricsHit> {
        let known: Vec<&'static str> = self.providers.iter().map(|p| p.name()).collect();
        self.bound.as_ref().and_then(|choice| choice.hit(&known))
    }

    /// Puts the sheet the listener picked by hand ahead of every automatic answer. It is kept
    /// with the app's state, not in the evictable cache, so a source being switched off or
    /// reordered, or the cache losing the track, never hides it.
    fn bound_first(&self, hits: &mut Vec<LyricsHit>) {
        let Some(bound) = self.bound_hit() else {
            return;
        };
        hits.retain(|hit| !same(hit, &bound));
        hits.insert(0, bound);
    }

    pub fn active_line(&self, cx: &Context<Self>) -> Option<usize> {
        let music::Lyrics::Synced { lines } = &self.current()?.lyrics else {
            return None;
        };
        let playback = self.playback.read(cx);
        let position = playback.shown_time(cx).0;
        music::lyrics::active(lines, position)
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let shown = self.playback.read(cx).shown_track(cx);
        let Some(track) = shown.as_ref() else {
            return self.forget(cx);
        };
        let Some(id) = track.id.as_deref() else {
            return self.forget(cx);
        };
        if self.following.as_deref() == Some(id) {
            return;
        }
        let (track, id) = (track.clone(), id.to_owned());
        self.following = Some(id.clone());
        self.chosen = 0;
        self.bound = self.bindings.get(&self.choice_key(&id, cx)).cloned();
        // A match picked by hand is the shown pick from the first frame, before any source
        // has answered.
        self.picked = self.bound.is_some();
        self.settled = false;
        self.manual = ManualSearch::default();
        self.manual_generation = self.manual_generation.wrapping_add(1);
        self.revision = self.revision.wrapping_add(1);

        // A file's own lyrics are never cached, so an edit to its tags shows on the next play.
        if !self.reads_file(&id, cx)
            && let Some(found) = self.recall(&id)
            && self
                .asked(&id, cx)
                .iter()
                .all(|provider| found.covers(provider))
        {
            let query = self.query(&track, &id, cx);
            self.show(&found, &query, cx);
            return;
        }
        self.load(id, track, cx);
    }

    /// Puts what `found` holds from the enabled sources on screen as the settled answer.
    fn show(&mut self, found: &Found, query: &LyricsQuery, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx);
        let hits: Vec<LyricsHit> = found
            .hits
            .iter()
            .filter(|hit| settings.lyrics_provider_enabled(hit.source))
            .cloned()
            .collect();
        let priority = settings.lyrics_provider_priority();
        let instrumental = music::lyrics::instrumental(query, &hits);
        self.task = None;
        self.settled = true;
        let mut hits = ordered(query, hits, &priority);
        self.bound_first(&mut hits);
        if self.bound.is_some() {
            self.chosen = 0;
            self.picked = true;
        }
        self.hits = hits;
        self.state = state_for(&self.hits, instrumental);
        cx.notify();
        self.prefetch(cx);
    }

    /// The answer held in memory for `id`, which becomes the most recently used.
    fn recall(&mut self, id: &str) -> Option<Found> {
        let index = self.recent.iter().position(|(held, _)| held == id)?;
        let entry = self.recent.remove(index)?;
        let found = entry.1.clone();
        self.recent.push_front(entry);
        Some(found)
    }

    fn recalls(&self, id: &str) -> bool {
        self.recent.iter().any(|(held, _)| held == id)
    }

    /// Holds `found` in memory as the newest answer, letting the oldest go past `RECENT`.
    fn hold(&mut self, id: String, found: Found) {
        self.recent.retain(|(held, _)| *held != id);
        self.recent.push_front((id, found));
        self.recent.truncate(RECENT);
    }

    /// Reads the answer stored for `id` off the main thread.
    fn stored(&self, id: &str, cx: &Context<Self>) -> Task<Option<Found>> {
        let key = self.key(id, cx);
        let known: Vec<&'static str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .collect();
        let store = self.store.clone();
        cx.background_executor().spawn(async move {
            store
                .get(&key, &known)
                .map(|(hits, answered)| Found { hits, answered })
        })
    }

    /// The durable key a manual match is filed under. Unlike the cache key above, it names
    /// the account or server the track id means something under: two Subsonic servers can both
    /// have a track `123`, and a match picked on one is not the other's. Providers whose ids
    /// are global (the streaming services) or carry their own identity (a local file's path)
    /// keep the provider-only shape.
    fn choice_key(&self, id: &str, cx: &Context<Self>) -> String {
        let session = self.session.read(cx);
        match session.provider_for(id) {
            Some(provider) => choice_key(provider.slug(), provider.identity().as_deref(), id),
            None => format!("{id}:lyrics"),
        }
    }

    /// One key per track. The stored answer covers every service that has replied, whichever
    /// are enabled now, and the selection is applied when it is shown. Services that replied
    /// empty are asked again while the stored answer holds nothing usable.
    fn key(&self, id: &str, cx: &Context<Self>) -> String {
        match self.session.read(cx).slug_for(id) {
            Some(slug) => format!("{slug}:{id}:lyrics"),
            None => format!("{id}:lyrics"),
        }
    }

    /// The providers a lookup for `id` may ask. The file's own lyrics are read only for a local
    /// file, and the services only when its metadata may go online.
    fn asked(&self, id: &str, cx: &Context<Self>) -> Vec<Arc<dyn LyricsProvider>> {
        let online = self.online(id, cx);
        let reads_file = self.reads_file(id, cx);
        let settings = self.settings.read(cx);
        self.providers
            .iter()
            .filter(|provider| settings.lyrics_provider_enabled(provider.name()))
            .filter(|provider| match provider.name() == LOCAL {
                true => reads_file,
                false => online,
            })
            .cloned()
            .collect()
    }

    fn query(&self, track: &Track, id: &str, cx: &Context<Self>) -> LyricsQuery {
        let key = self.session.read(cx).slug_for(id).map(|provider| TrackKey {
            provider,
            id: id.to_owned(),
        });
        query_for(track, key)
    }

    /// Whether the services may be asked about this track, which a local file only allows when
    /// the user lets its metadata go online.
    fn online(&self, id: &str, cx: &Context<Self>) -> bool {
        !music::is_local_id(id) || self.settings.read(cx).lyrics_for_local_files()
    }

    fn reads_file(&self, id: &str, cx: &Context<Self>) -> bool {
        music::is_local_id(id) && self.settings.read(cx).lyrics_provider_enabled(LOCAL)
    }

    /// Ends a lookup's bookkeeping; true when it was for the track on screen.
    fn finished(&mut self, id: &str) -> bool {
        let current = self.following.as_deref() == Some(id);
        if current {
            self.task = None;
        }
        if self.ahead_of.as_deref() == Some(id) {
            self.ahead_of = None;
        }
        current
    }

    fn forget(&mut self, cx: &mut Context<Self>) {
        if self.following.is_none() {
            return;
        }
        self.task = None;
        self.following = None;
        self.hits.clear();
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.bound = None;
        self.manual = ManualSearch::default();
        self.manual_generation = self.manual_generation.wrapping_add(1);
        self.revision = self.revision.wrapping_add(1);
        self.state = LyricsState::Idle;
        cx.notify();
    }

    fn load(&mut self, id: String, track: Track, cx: &mut Context<Self>) {
        self.hits.clear();
        self.state = LyricsState::Loading;
        cx.notify();

        if self.ahead_of.as_deref() == Some(id.as_str()) && self.ahead.is_some() {
            self.task = self.ahead.take();
            self.ahead_of = None;
            return;
        }
        self.task = Some(self.fetch(id, track, cx));
    }

    fn prefetch(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let queue = self.playback.read(cx).shown_queue(cx);
        let Some(track) = queue.track(QueuePosition::Upcoming(0), cx) else {
            return;
        };
        let Some(id) = track.id.as_deref() else {
            return;
        };
        if self.ahead_of.as_deref() == Some(id) || self.recalls(id) {
            return;
        }
        let id = id.to_owned();
        self.ahead_of = Some(id.clone());
        self.ahead = Some(self.fetch(id, track, cx));
    }

    /// Looks a track up: in memory, then in the cache database, then with the providers. Only
    /// the services that have not answered for this track are asked, unless nothing usable is
    /// stored, and a file's own lyrics are read again on every play. What the cache holds is
    /// shown alongside the new answers.
    fn fetch(&mut self, id: String, track: Track, cx: &mut Context<Self>) -> Task<()> {
        let settings = self.settings.read(cx);
        let priority = settings.lyrics_provider_priority();
        let enabled: Vec<&'static str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .filter(|name| settings.lyrics_provider_enabled(name))
            .collect();
        let mut asked = self.asked(&id, cx);
        // Offline mode reads a file's own lyrics and what the cache kept, and asks no service.
        if crate::Network::offline(cx) {
            asked.retain(|provider| provider.name() == LOCAL);
        }
        let remembered = self.recall(&id);
        let stored = remembered.is_none().then(|| self.stored(&id, cx));
        let query = self.query(&track, &id, cx);
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let cached = match stored {
                Some(stored) => stored.await,
                None => remembered,
            };
            if let Some(found) = &cached {
                let held = found.clone();
                let id = id.clone();
                this.update(cx, |this, _| this.hold(id, held)).ok();
            }
            let cached = cached.unwrap_or_default();
            // Services that replied empty are asked again while nothing usable is stored:
            // catalogues grow, and the one source that had anything may since have been
            // switched off, leaving a stored answer with no enabled hits.
            let stored = cached.hits.iter().any(|hit| enabled.contains(&hit.source));
            let providers: Vec<Arc<dyn LyricsProvider>> = asked
                .into_iter()
                .filter(|provider| !stored || !cached.covers(provider))
                .collect();
            if providers.is_empty() {
                this.update(cx, |this, cx| {
                    if this.finished(&id) {
                        log::debug!("lyrics: showing the stored answer");
                        this.show(&cached, &query, cx);
                    }
                })
                .ok();
                return;
            }

            // The cached hits from enabled sources go in first, as if they had just arrived.
            let held: Vec<LyricsHit> = cached
                .hits
                .iter()
                .filter(|hit| enabled.contains(&hit.source))
                .cloned()
                .collect();
            let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel();
            if !held.is_empty() {
                sender.send((None, held)).ok();
            }
            let mut pending: Vec<&'static str> =
                providers.iter().map(|provider| provider.name()).collect();
            let ranking = query.clone();
            let worker = io.spawn(async move { gather(providers, query, sender).await });
            let mut hits = Vec::new();
            let mut displayed: Option<LyricsHit> = None;
            let mut synced = false;

            while let Some((from, mut found)) = incoming.recv().await {
                pending.retain(|name| Some(*name) != from);
                hits.append(&mut found);
                let ranked = ordered(&ranking, hits.clone(), &priority);
                let Some(best) = ranked.first().cloned() else {
                    continue;
                };
                if !synced || best.lyrics.synced() {
                    synced = best.lyrics.synced();
                    displayed = Some(best);
                }
                // Full synced lyrics can only lose to a source ranked higher, anything else to
                // whichever source answers next.
                let waiting = displayed.as_ref().is_some_and(|displayed| {
                    let place = match music::lyrics::fully_synced(&ranking, &displayed.lyrics) {
                        true => rank_in(&priority, displayed.source),
                        false => usize::MAX,
                    };
                    pending.iter().any(|name| rank_in(&priority, name) < place)
                });
                let anchor = displayed.clone();
                this.update(cx, |this, cx| {
                    if this.following.as_deref() != Some(id.as_str()) {
                        return;
                    }
                    this.paint(ranked, anchor.as_ref(), waiting, cx);
                })
                .ok();
            }

            let answered = join(worker).await;

            this.update(cx, |this, cx| {
                let current = this.finished(&id);
                match answered {
                    Ok(answered) => {
                        crate::Network::reached(cx);
                        this.keep(id, cached, &hits, answered, cx);
                        if current {
                            let instrumental = music::lyrics::instrumental(&ranking, &hits);
                            let ranked = ordered(&ranking, hits, &priority);
                            this.settle(ranked, displayed.as_ref(), instrumental, cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("lyrics: cannot look up {}: {error:#}", track.name);
                        let reason = crate::blamed(&error, cx);
                        if current {
                            this.state = LyricsState::Failed(reason);
                            cx.notify();
                        }
                    }
                }
            })
            .ok();
        })
    }

    /// Shows an answer that arrived while others are still being looked for.
    ///
    /// A synced sheet settles the question the moment it turns up, unless `waiting` says a
    /// source that could still beat it has not answered yet. Short of that, the first synced
    /// answer is put up and nothing replaces it until the question settles, and an untimed one
    /// is not shown at all while something better could still arrive.
    fn paint(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        waiting: bool,
        cx: &mut Context<Self>,
    ) {
        if self.settled {
            return;
        }
        let synced = self
            .prospect(&ranked, displayed)
            .is_some_and(|hit| hit.lyrics.synced());
        if !synced || (waiting && !self.hits.is_empty()) {
            return;
        }
        match waiting {
            false => {
                log::debug!("lyrics: settling on a synced answer as it arrives");
                self.settled = true;
            }
            true => log::debug!("lyrics: showing the first synced answer while the rest arrive"),
        }
        self.apply(ranked, displayed, cx);
    }

    fn apply(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        cx: &mut Context<Self>,
    ) {
        self.pin(ranked, displayed);
        if !self.hits.is_empty() {
            self.state = LyricsState::Ready;
        }
        cx.notify();
    }

    /// The hit a ranking would put on screen.
    fn prospect<'a>(
        &'a self,
        ranked: &'a [LyricsHit],
        displayed: Option<&'a LyricsHit>,
    ) -> Option<&'a LyricsHit> {
        let anchor = match self.picked {
            true => self.hits.get(self.chosen),
            false => displayed,
        };
        anchor
            .and_then(|anchor| ranked.iter().find(|hit| same(hit, anchor)))
            .or_else(|| ranked.first())
    }

    /// Adds what the services said in this lookup to the track's answer and saves it when one of
    /// them replied for the first time. A service that failed stays out of `answered`, so the
    /// next lookup asks it again.
    fn keep(
        &mut self,
        id: String,
        mut found: Found,
        hits: &[LyricsHit],
        answered: Vec<&'static str>,
        cx: &mut Context<Self>,
    ) {
        // A file's own lyrics are read again on every play, so they are never kept.
        if !self.online(&id, cx) {
            return;
        }
        let fresh: Vec<&'static str> = answered
            .into_iter()
            .filter(|name| *name != LOCAL && !found.answered.contains(name))
            .collect();
        if !fresh.is_empty() {
            found.hits.extend(
                hits.iter()
                    .filter(|hit| fresh.contains(&hit.source))
                    .cloned(),
            );
            found.answered.extend(fresh);
            if !found.hits.is_empty() {
                let key = self.key(&id, cx);
                self.unsaved
                    .push(Sheets::unsaved(key, &found.hits, &found.answered));
                self.schedule_save(cx);
            }
        }
        self.hold(id, found);
    }

    /// Puts up the answer once every source has replied. Unless a word-by-word sheet already
    /// settled the question, this is the best there is and nothing may replace it afterwards.
    fn settle(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        instrumental: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.settled {
            log::debug!("lyrics: settling on the best answer");
            self.settled = true;
            self.pin(ranked, displayed);
            self.state = state_for(&self.hits, instrumental);
        }
        cx.notify();
        self.prefetch(cx);
    }

    fn pin(&mut self, ranked: Vec<LyricsHit>, displayed: Option<&LyricsHit>) {
        let anchor = match self.picked {
            true => self.hits.get(self.chosen).cloned(),
            false => displayed.cloned(),
        };
        let anchor = anchor.or_else(|| self.bound_hit());
        let before = self.current().map(|hit| hit.lyrics.clone());
        let mut hits = ranked;
        self.bound_first(&mut hits);
        let held = keep_displayed_first(&mut hits, anchor.as_ref());
        self.hits = hits;
        self.chosen = 0;
        self.picked &= held;
        self.picked |= self.bound.is_some();
        if before.as_ref() != self.current().map(|hit| &hit.lyrics) {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Writes the unsaved answers once lookups have gone quiet for `SAVE_DELAY`. A write that
    /// has begun always finishes, even when another answer reschedules the save.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok((store, unsaved)) = this.update(cx, |this, _| {
                (this.store.clone(), std::mem::take(&mut this.unsaved))
            }) else {
                return;
            };
            cx.background_executor()
                .spawn(async move { store.save(unsaved) })
                .detach();
        }));
    }
}

/// The durable key for a manual match: provider, its account or server where it has one, and
/// the track id. The owner sits between the provider and the id, so the same id on another
/// server never collides.
fn choice_key(slug: &str, owner: Option<&str>, id: &str) -> String {
    match owner {
        Some(owner) => format!("{slug}:{owner}:{id}:lyrics"),
        None => format!("{slug}:{id}:lyrics"),
    }
}

/// Folds the matches read from disk into the ones kept in memory. A bind or unbind made while
/// the read was in flight is recorded in `dirty` and wins: the read began before it, so it
/// must not erase the new match nor bring the dropped one back.
fn merge_choices(
    loaded: HashMap<String, Choice>,
    bindings: &mut HashMap<String, Choice>,
    dirty: &HashMap<String, Option<Choice>>,
) {
    for (key, choice) in loaded {
        if !dirty.contains_key(&key) {
            bindings.insert(key, choice);
        }
    }
}

fn keep_displayed_first(hits: &mut Vec<LyricsHit>, displayed: Option<&LyricsHit>) -> bool {
    let Some(index) =
        displayed.and_then(|displayed| hits.iter().position(|hit| same(hit, displayed)))
    else {
        return false;
    };
    let displayed = hits.remove(index);
    hits.insert(0, displayed);
    true
}

fn same(left: &LyricsHit, right: &LyricsHit) -> bool {
    left.source == right.source
        && left.title == right.title
        && left.artist == right.artist
        && left.album == right.album
        && left.duration == right.duration
        && left.lyrics.worded() == right.lyrics.worded()
}

fn ordered(query: &LyricsQuery, hits: Vec<LyricsHit>, priority: &[String]) -> Vec<LyricsHit> {
    let mut ranked = music::lyrics::rank_with_priority(query, hits, priority);
    music::lyrics::reshape(&mut ranked);
    ranked
}

/// Hits sent to a lookup, with the provider that has now answered. Hits held from an earlier
/// lookup come with no provider.
type Reply = (Option<&'static str>, Vec<LyricsHit>);

/// Where `source` stands in the user's priority. Sources missing from it come last.
fn rank_in(priority: &[String], source: &str) -> usize {
    priority
        .iter()
        .position(|name| name == source)
        .unwrap_or(priority.len())
}

/// What the services have said about one track: every hit matched to it, unranked and from every
/// source, and the services that replied. The selection is applied when it is shown.
#[derive(Clone, Default)]
struct Found {
    hits: Vec<LyricsHit>,
    answered: Vec<&'static str>,
}

impl Found {
    /// Whether `provider` has nothing left to say. A service that replied is covered, but one
    /// that replied empty is asked again while the stored answer holds nothing usable from
    /// an enabled source. A file's own lyrics are never covered, since they are read again
    /// on every play.
    fn covers(&self, provider: &Arc<dyn LyricsProvider>) -> bool {
        provider.name() != LOCAL && self.answered.contains(&provider.name())
    }
}

fn state_for(hits: &[LyricsHit], instrumental: bool) -> LyricsState {
    match (hits.is_empty(), instrumental) {
        (false, _) => LyricsState::Ready,
        (true, true) => LyricsState::Instrumental,
        (true, false) => LyricsState::Missing,
    }
}

fn query_for(track: &Track, key: Option<TrackKey>) -> LyricsQuery {
    LyricsQuery {
        title: track.name.clone(),
        artist: track.artists.clone(),
        album: (!track.album.is_empty()).then(|| track.album.clone()),
        duration: track.duration,
        track: key,
    }
}

/// Asks every provider at once and sends each one's name and hits for the queried track as
/// they arrive. A provider that fails sends its name with no hits, so the receiver knows it is
/// done. Returns the providers that replied, leaving out any that failed.
async fn gather(
    providers: Vec<Arc<dyn LyricsProvider>>,
    query: LyricsQuery,
    sender: tokio::sync::mpsc::UnboundedSender<Reply>,
) -> anyhow::Result<Vec<&'static str>> {
    let mut tasks = JoinSet::new();
    for provider in providers {
        let query = query.clone();
        tasks.spawn(async move {
            let name = provider.name();
            let found = provider
                .search(&query)
                .await
                .inspect_err(|error| log::warn!("lyrics: {name} did not answer: {error:#}"))
                .ok()
                .map(|mut found| {
                    // Hits for another track are dropped here, before the lookup holds and
                    // copies them.
                    found.retain(|hit| music::lyrics::matched(&query, hit));
                    found
                });
            (name, found)
        });
    }
    let mut answered = Vec::new();
    while !tasks.is_empty() {
        tokio::select! {
            // Dropping the foreground lookup (including on a provider change) must also
            // stop its network requests. Dropping this JoinSet aborts the remaining tasks.
            _ = sender.closed() => break,
            found = tasks.join_next() => {
                if let Some(Ok((name, found))) = found {
                    if found.is_some() {
                        answered.push(name);
                    }
                    sender.send((Some(name), found.unwrap_or_default())).ok();
                }
            }
        }
    }
    Ok(answered)
}

/// Asks every provider at once for hits to a typed search, sending each one's hits as they
/// arrive. Unlike [`gather`], the hits are not matched against the query here: the search
/// exists because the track's identity matched nothing, so the listener judges what fits when
/// they pick a match. Returns the providers that replied, leaving out any that failed.
async fn gather_text(
    providers: Vec<Arc<dyn LyricsProvider>>,
    query: LyricsQuery,
    sender: tokio::sync::mpsc::UnboundedSender<Vec<LyricsHit>>,
) -> anyhow::Result<Vec<&'static str>> {
    let mut tasks = JoinSet::new();
    for provider in providers {
        let query = query.clone();
        tasks.spawn(async move {
            let name = provider.name();
            let found = provider
                .search(&query)
                .await
                .inspect_err(|error| log::warn!("lyrics: {name} did not answer: {error:#}"))
                .ok();
            (name, found)
        });
    }
    let mut answered = Vec::new();
    while !tasks.is_empty() {
        tokio::select! {
            // Dropping the receiver (a new search, another track, a closed dialog) must also
            // stop the requests. Dropping this JoinSet aborts the remaining tasks.
            _ = sender.closed() => break,
            found = tasks.join_next() => {
                if let Some(Ok((name, Some(found)))) = found {
                    answered.push(name);
                    sender.send(found).ok();
                }
            }
        }
    }
    Ok(answered)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use music::{Lyrics as Sheet, LyricsHit, LyricsLine, LyricsWord};

    use super::{Choice, choice_key, keep_displayed_first, merge_choices};

    fn hit(source: &'static str, lyrics: Sheet) -> LyricsHit {
        LyricsHit {
            source,
            lyrics,
            title: "title".to_owned(),
            artist: "artist".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_displayed_karaoke_result_stays_selected_after_final_ranking() {
        let plain = hit("plain", Sheet::plain("line"));
        let displayed = hit(
            "karaoke",
            Sheet::Synced {
                lines: vec![LyricsLine {
                    end: Some(Duration::from_secs(1)),
                    text: "line".to_owned(),
                    words: Some(vec![LyricsWord {
                        start: Duration::ZERO,
                        end: Duration::from_secs(1),
                        text: "line".to_owned(),
                    }]),
                    ..Default::default()
                }]
                .into(),
            },
        );
        let mut final_ranking = vec![plain, displayed.clone()];

        keep_displayed_first(&mut final_ranking, Some(&displayed));

        assert_eq!(final_ranking.first(), Some(&displayed));
    }

    fn choice(key_id: &str) -> (String, Choice) {
        (
            key_id.to_owned(),
            Choice::of(
                "夜に駆ける",
                "YOASOBI",
                &hit("LrcLib", Sheet::plain("a line")),
            ),
        )
    }

    #[test]
    fn a_match_key_names_the_server_the_track_id_means_something_under() {
        // Two Subsonic servers can both have a track `123`; a match picked on one is not the
        // other's.
        let first = choice_key("subsonic", Some("alice@https://one.example"), "123");
        let second = choice_key("subsonic", Some("bob@https://two.example"), "123");
        assert_ne!(first, second);
        assert!(first.contains("123"));

        // A provider without an account namespace, and a local file whose id is its path,
        // keep the provider-only shape.
        assert_eq!(choice_key("spotify", None, "abc"), "spotify:abc:lyrics");
        assert_eq!(
            choice_key("local", None, "/music/song.mp3"),
            "local:/music/song.mp3:lyrics"
        );
    }

    #[test]
    fn a_bind_made_while_the_disk_read_is_in_flight_wins_over_it() {
        let (key, on_disk) = choice("subsonic:alice@srv:123:lyrics");
        let (_, made_now) = (
            key.clone(),
            Choice::of("other", "search", &hit("NetEase", Sheet::plain("new"))),
        );
        let loaded = HashMap::from([(key.clone(), on_disk)]);
        let mut bindings = HashMap::from([(key.clone(), made_now)]);
        let dirty = HashMap::from([(key.clone(), Some(bindings[&key].clone()))]);

        merge_choices(loaded, &mut bindings, &dirty);

        let (title, _) = bindings[&key].searched();
        assert_eq!(title, "other");
    }

    #[test]
    fn an_unbind_made_while_the_disk_read_is_in_flight_stays_gone() {
        let (key, on_disk) = choice("subsonic:alice@srv:123:lyrics");
        let loaded = HashMap::from([(key.clone(), on_disk)]);
        let mut bindings = HashMap::new();
        let dirty = HashMap::from([(key.clone(), None)]);

        merge_choices(loaded, &mut bindings, &dirty);

        assert!(!bindings.contains_key(&key));
    }

    #[test]
    fn matches_untouched_since_the_read_began_merge_in() {
        let (old_key, on_disk) = choice("subsonic:alice@srv:123:lyrics");
        let (new_key, made_now) = choice("subsonic:alice@srv:456:lyrics");
        let loaded = HashMap::from([(old_key.clone(), on_disk)]);
        let mut bindings = HashMap::from([(new_key.clone(), made_now)]);
        let dirty = HashMap::from([(new_key.clone(), Some(bindings[&new_key].clone()))]);

        merge_choices(loaded, &mut bindings, &dirty);

        assert!(bindings.contains_key(&old_key));
        assert!(bindings.contains_key(&new_key));
    }
}
