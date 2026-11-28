use std::collections::VecDeque;
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use logic::lyrics::{awaits_better, gather, ordered};
use lyrics::LOCAL;
use music::{LyricsHit, LyricsProvider, LyricsQuery, Track, TrackKey};

use crate::sheets::{Sheets, Unsaved};
use crate::{AppSettings, Io, Playback, Queue, QueuePosition, Session, SessionEvent, join};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LyricsState {
    Idle,
    Loading,
    Ready,
    Instrumental,
    Missing,
    Failed(String),
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
    following_query: Option<LyricsQuery>,
    /// The answers looked at most recently, newest first.
    recent: VecDeque<(String, Found)>,
    store: Sheets,
    /// Answers found since the last save, written together once lookups go quiet.
    unsaved: Vec<Unsaved>,
    providers: Vec<Arc<dyn LyricsProvider>>,
    /// Whether lyrics are on at all. While off, nothing is looked up and the sheet stays empty.
    enabled: bool,
    enabled_providers: Vec<String>,
    /// The lookup permissions last applied, so changing either restarts the current sheet.
    local_online: bool,
    offline: bool,
    /// The order of the sources the answer on show was ranked by. A new order looks the track
    /// up again, from what is held, so the source now ranked first is the one shown.
    priority: Vec<String>,
    /// Whether the answer on show was ranked with word-by-word lyrics ahead of the source order.
    worded_first: bool,
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
    pub fn new(
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        providers: Vec<Arc<dyn LyricsProvider>>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&playback, |this, _, cx| {
            this.follow(cx);
            this.prefetch(cx);
        })
        .detach();
        // a lookup ahead that waited on the current track's is started on a later tick
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |this, _, cx| {
            this.follow(cx);
            this.prefetch(cx);
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.prefetch(cx)).detach();
        cx.subscribe(&session, |this, _, event, cx| {
            if !matches!(event, SessionEvent::LocalChanged) {
                return;
            }
            this.recent.retain(|(id, _)| !music::is_local_id(id));
            this.ahead = None;
            this.ahead_of = None;
            if this.following.as_deref().is_some_and(music::is_local_id) {
                this.forget(cx);
                this.follow(cx);
            }
        })
        .detach();
        cx.observe(&settings, |this, settings, cx| {
            let settings = settings.read(cx);
            let on = settings.lyrics_enabled();
            let enabled = settings.lyrics_providers();
            let priority = settings.lyrics_provider_priority();
            let worded_first = settings.lyrics_word_synced_first();
            let local_online = settings.lyrics_for_local_files();
            let offline = settings.offline_mode();
            if on == this.enabled
                && enabled == this.enabled_providers
                && priority == this.priority
                && worded_first == this.worded_first
                && local_online == this.local_online
                && offline == this.offline
            {
                return;
            }
            this.enabled = on;
            this.enabled_providers = enabled.to_vec();
            this.priority = priority;
            this.worded_first = worded_first;
            this.local_online = local_online;
            this.offline = offline;
            this.task = None;
            this.ahead = None;
            this.ahead_of = None;
            this.forget(cx);
            this.follow(cx);
        })
        .detach();
        let enabled = settings.read(cx).lyrics_enabled();
        let enabled_providers = settings.read(cx).lyrics_providers().to_vec();
        let priority = settings.read(cx).lyrics_provider_priority();
        let worded_first = settings.read(cx).lyrics_word_synced_first();
        let store = Sheets::new(storage::Cache::standard());
        let moving = store.clone();
        let migration = cx
            .background_executor()
            .spawn(async move { moving.migrate() });
        Self {
            state: LyricsState::Idle,
            hits: Vec::new(),
            chosen: 0,
            picked: false,
            settled: false,
            revision: 0,
            following: None,
            following_query: None,
            recent: VecDeque::new(),
            store,
            unsaved: Vec::new(),
            providers,
            enabled,
            enabled_providers,
            local_online: settings.read(cx).lyrics_for_local_files(),
            offline: settings.read(cx).offline_mode(),
            priority,
            worded_first,
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

    /// Looks up the lyrics for a song without changing the sheet the player follows.
    pub(crate) fn for_song(
        &self,
        id: &str,
        track: &Track,
        cx: &App,
    ) -> Task<Result<music::Lyrics, LyricsState>> {
        let query = self.query(track, id, cx);
        if self.following.as_deref() == Some(id)
            && self.following_query.as_ref() == Some(&query)
            && self.state == LyricsState::Ready
            && let Some(hit) = self.current()
        {
            return Task::ready(Ok(hit.lyrics.clone()));
        }

        let settings = self.settings.read(cx);
        let priority = settings.lyrics_provider_priority();
        let worded_first = settings.lyrics_word_synced_first();
        let (enabled, asked) = self.sources(id, cx);
        let cached = match self.recent.iter().find(|(held, _)| held == id) {
            Some((_, found)) => Task::ready(Some(found.clone())),
            None => self.stored(id, track, cx),
        };
        let io = self.io.clone();

        cx.background_executor().spawn(async move {
            let cached = cached.await.unwrap_or_default();
            let providers = cached.pending(asked, &query, &enabled);
            let mut hits = cached.held(&enabled, &providers);
            let all_failed = match providers.is_empty() {
                true => false,
                false => {
                    let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel();
                    let search = query.clone();
                    let worker = io.spawn(async move { gather(providers, search, sender).await });
                    while let Some((_, mut found)) = incoming.recv().await {
                        hits.append(&mut found);
                    }
                    join(worker)
                        .await
                        .map_err(|error| LyricsState::Failed(error.to_string()))?
                        .is_empty()
                }
            };
            if all_failed {
                hits = cached.held(&enabled, &[]);
            }

            let instrumental = lyrics::instrumental(&query, &hits);
            let ranked = ordered(&query, hits, &priority, worded_first);
            let state = state_for(&ranked, instrumental);
            if all_failed && state == LyricsState::Missing {
                return Err(LyricsState::Failed(
                    "all lyrics providers failed".to_owned(),
                ));
            }
            ranked.into_iter().next().map(|hit| hit.lyrics).ok_or(state)
        })
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

    pub fn active_line(&self, cx: &App) -> Option<usize> {
        let music::Lyrics::Synced { lines } = &self.current()?.lyrics else {
            return None;
        };
        let playback = self.playback.read(cx);
        let position = playback.shown_time(cx).0;
        lyrics::active(lines, position)
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        if !self.enabled {
            return self.forget(cx);
        }
        let shown = self.playback.read(cx).shown_track(cx);
        let Some(track) = shown.as_ref() else {
            return self.forget(cx);
        };
        let Some(id) = track.id.as_deref() else {
            return self.forget(cx);
        };
        let query = self.query(track, id, cx);
        if self.following.as_deref() == Some(id) {
            if self.following_query.as_ref() == Some(&query) {
                return;
            }
            self.task = None;
            self.ahead = None;
            self.ahead_of = None;
            self.recent.retain(|(held, _)| held != id);
        }
        self.following_query = Some(query);
        let (track, id) = (track.clone(), id.to_owned());
        self.following = Some(id.clone());
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.revision = self.revision.wrapping_add(1);

        let query = self.query(&track, &id, cx);
        // A file's own lyrics are never cached, so an edit to its tags shows on the next play.
        if !self.reads_file(&id, cx)
            && let Some(found) = self.recall(&id)
            && found.usable(&query, &self.enabled_providers)
            && self
                .asked(&id, cx)
                .iter()
                .all(|provider| found.covers(provider))
        {
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
        let worded_first = settings.lyrics_word_synced_first();
        let instrumental = lyrics::instrumental(query, &hits);
        self.task = None;
        self.settled = true;
        self.hits = ordered(query, hits, &priority, worded_first);
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
    fn stored(&self, id: &str, track: &Track, cx: &App) -> Task<Option<Found>> {
        let key = self.key(id, track, cx);
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

    /// One key per track. The stored answer covers every service that has replied, whichever
    /// are enabled now, and the selection is applied when it is shown. Services that replied
    /// empty are asked again while the stored answer holds nothing usable.
    fn key(&self, id: &str, track: &Track, cx: &App) -> String {
        let key = match self.session.read(cx).slug_for(id) {
            Some(slug) => format!("{slug}:{id}:lyrics"),
            None => format!("{id}:lyrics"),
        };
        if !music::is_local_id(id) {
            return key;
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (&track.name, &track.artists, &track.album, track.duration).hash(&mut hash);
        format!("{key}:{:x}", hash.finish())
    }

    /// The providers a lookup for `id` may ask. The file's own lyrics are read only for a local
    /// file, and the services only when its metadata may go online.
    fn asked(&self, id: &str, cx: &App) -> Vec<Arc<dyn LyricsProvider>> {
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

    /// Sources whose cached sheets may be shown and sources this lookup may contact.
    fn sources(&self, id: &str, cx: &App) -> (Vec<&'static str>, Vec<Arc<dyn LyricsProvider>>) {
        let settings = self.settings.read(cx);
        let enabled = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .filter(|name| settings.lyrics_provider_enabled(name))
            .collect();
        let mut asked = self.asked(id, cx);
        if crate::Network::offline(cx) {
            asked.retain(|provider| provider.name() == LOCAL);
        }
        (enabled, asked)
    }

    fn query(&self, track: &Track, id: &str, cx: &App) -> LyricsQuery {
        let key = self.session.read(cx).slug_for(id).map(|provider| TrackKey {
            provider,
            id: id.to_owned(),
        });
        query_for(track, key)
    }

    /// Whether the services may be asked about this track, which a local file only allows when
    /// the user lets its metadata go online.
    fn online(&self, id: &str, cx: &App) -> bool {
        !music::is_local_id(id) || self.settings.read(cx).lyrics_for_local_files()
    }

    fn reads_file(&self, id: &str, cx: &App) -> bool {
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
        self.following_query = None;
        self.hits.clear();
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
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
        if !self.enabled || self.task.is_some() {
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
        let worded_first = settings.lyrics_word_synced_first();
        let (enabled, asked) = self.sources(&id, cx);
        let key = self.key(&id, &track, cx);
        let remembered = self.recall(&id);
        let stored = remembered.is_none().then(|| self.stored(&id, &track, cx));
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
            let providers = cached.pending(asked, &query, &enabled);
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
            let held = cached.held(&enabled, &providers);
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
                let ranked = ordered(&ranking, hits.clone(), &priority, worded_first);
                let Some(best) = ranked.first().cloned() else {
                    continue;
                };
                if !synced || best.lyrics.synced() {
                    synced = best.lyrics.synced();
                    displayed = Some(best);
                }
                let waiting = displayed.as_ref().is_some_and(|displayed| {
                    awaits_better(&ranking, displayed, &pending, &priority, worded_first)
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
                        this.keep(id, key, cached, &hits, answered, cx);
                        if current {
                            let instrumental = lyrics::instrumental(&ranking, &hits);
                            let ranked = ordered(&ranking, hits, &priority, worded_first);
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

    /// Replaces each successful source's stored answer with what it said in this lookup, including
    /// a source retried after an empty answer. Failed sources keep their previous answers.
    fn keep(
        &mut self,
        id: String,
        key: String,
        mut found: Found,
        hits: &[LyricsHit],
        answered: Vec<&'static str>,
        cx: &mut Context<Self>,
    ) {
        // A file's own lyrics are read again on every play, so they are never kept.
        if !self.online(&id, cx) {
            return;
        }
        let answered: Vec<&'static str> =
            answered.into_iter().filter(|name| *name != LOCAL).collect();
        if !answered.is_empty() {
            found.hits.retain(|hit| !answered.contains(&hit.source));
            found.hits.extend(
                hits.iter()
                    .filter(|hit| answered.contains(&hit.source))
                    .cloned(),
            );
            for name in answered {
                if !found.answered.contains(&name) {
                    found.answered.push(name);
                }
            }
            if !found.hits.is_empty() {
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
        let before = self.current().map(|hit| hit.lyrics.clone());
        let mut hits = ranked;
        let held = keep_displayed_first(&mut hits, anchor.as_ref());
        self.hits = hits;
        self.chosen = 0;
        self.picked &= held;
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

/// What the services have said about one track: every hit matched to it, unranked and from every
/// source, and the services that replied. The selection is applied when it is shown.
#[derive(Clone, Default)]
struct Found {
    hits: Vec<LyricsHit>,
    answered: Vec<&'static str>,
}

impl Found {
    /// Sources that still need asking. Retry empty answers when none can be shown.
    fn pending(
        &self,
        mut asked: Vec<Arc<dyn LyricsProvider>>,
        query: &LyricsQuery,
        enabled: &[&str],
    ) -> Vec<Arc<dyn LyricsProvider>> {
        if self.usable(query, enabled) {
            asked.retain(|provider| !self.covers(provider));
        }
        asked
    }

    /// Enabled sheets whose sources will not be asked for a fresh answer.
    fn held(&self, enabled: &[&str], pending: &[Arc<dyn LyricsProvider>]) -> Vec<LyricsHit> {
        self.hits
            .iter()
            .filter(|hit| enabled.contains(&hit.source))
            .filter(|hit| !pending.iter().any(|provider| provider.name() == hit.source))
            .cloned()
            .collect()
    }

    /// Whether an enabled source has lyrics that can be shown, or confirms an instrumental.
    fn usable(&self, query: &LyricsQuery, enabled: &[impl AsRef<str>]) -> bool {
        self.hits.iter().any(|hit| {
            enabled.iter().any(|name| name.as_ref() == hit.source)
                && (lyrics::eligible(query, hit)
                    || (hit.instrumental && lyrics::matched(query, hit)))
        })
    }

    /// Whether `provider` has replied. Callers only reuse that answer while an enabled source
    /// holds something usable. A file's own lyrics are never covered, since they are read again
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use music::{Lyrics as Sheet, LyricsHit, LyricsLine, LyricsWord};

    use super::keep_displayed_first;

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
        let plain = hit("plain", lyrics::plain("line"));
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
}
