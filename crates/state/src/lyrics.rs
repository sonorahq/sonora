use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use music::lyrics::LOCAL;
use music::{LyricsHit, LyricsProvider, LyricsQuery, Track, TrackKey};
use tokio::task::JoinSet;

use crate::sheets::{Sheets, Unsaved};
use crate::{AppSettings, Change, Cx, Event, Job, QueuePosition, Session, Store, Topic, join};

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

/// The lyrics of the shown track: every answer the sources gave, the one on show, and a lookup
/// of the next track ahead of time. It follows playback, the queue and the settings through
/// [`Store::react_lyrics`].
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
    sheets: Sheets,
    /// Answers found since the last save, written together once lookups go quiet.
    unsaved: Vec<Unsaved>,
    providers: Vec<Arc<dyn LyricsProvider>>,
    /// Whether lyrics are on at all. While off, nothing is looked up and the sheet stays empty.
    enabled: bool,
    enabled_providers: Vec<String>,
    /// The order of the sources the answer on show was ranked by. A new order looks the track
    /// up again, from what is held, so the source now ranked first is the one shown.
    priority: Vec<String>,
    task: Option<Job>,
    ahead: Option<Job>,
    ahead_of: Option<String>,
    save: Option<Job>,
    /// Moves the old lyrics.json into the cache database, once.
    _migration: Job,
}

/// What a lookup reads from the rest of the store: the session that names an id's provider,
/// the settings that pick the sources, and whether offline mode is on.
struct Around<'a> {
    session: &'a Session,
    settings: &'a AppSettings,
    offline: bool,
}

impl Lyrics {
    pub fn new(
        settings: &AppSettings,
        providers: Vec<Arc<dyn LyricsProvider>>,
        cx: &mut Cx,
    ) -> Self {
        let enabled = settings.lyrics_enabled();
        let enabled_providers = settings.lyrics_providers().to_vec();
        let priority = settings.lyrics_provider_priority();
        let sheets = Sheets::new(storage::Cache::standard());
        let moving = sheets.clone();
        let migration = cx.spawn(|_| async move {
            tokio::task::spawn_blocking(move || moving.migrate())
                .await
                .ok();
        });
        Self {
            state: LyricsState::Idle,
            hits: Vec::new(),
            chosen: 0,
            picked: false,
            settled: false,
            revision: 0,
            following: None,
            recent: VecDeque::new(),
            sheets,
            unsaved: Vec::new(),
            providers,
            enabled,
            enabled_providers,
            priority,
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

    pub fn choose(&mut self, index: usize) {
        if index >= self.hits.len() || index == self.chosen {
            return;
        }
        self.chosen = index;
        self.picked = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// The synced line sung at `position`, how far into the shown track playback is, as
    /// `Playback::shown_time` tells it.
    pub fn active_line(&self, position: Duration) -> Option<usize> {
        let music::Lyrics::Synced { lines } = &self.current()?.lyrics else {
            return None;
        };
        music::lyrics::active(lines, position)
    }

    /// Follows the shown track. Returns whether a held answer went on screen at once, after
    /// which the next track is looked up ahead.
    fn follow(&mut self, shown: Option<Track>, around: &Around, cx: &mut Cx) -> bool {
        if !self.enabled {
            self.forget();
            return false;
        }
        let Some(track) = shown.as_ref() else {
            self.forget();
            return false;
        };
        let Some(id) = track.id.as_deref() else {
            self.forget();
            return false;
        };
        if self.following.as_deref() == Some(id) {
            return false;
        }
        let (track, id) = (track.clone(), id.to_owned());
        self.following = Some(id.clone());
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.revision = self.revision.wrapping_add(1);

        let query = self.query(&track, &id, around);
        // A file's own lyrics are never cached, so an edit to its tags shows on the next play.
        if !self.reads_file(&id, around)
            && let Some(found) = self.recall(&id)
            && found.usable(&query, &self.enabled_providers)
            && self
                .asked(&id, around)
                .iter()
                .all(|provider| found.covers(provider))
        {
            self.show(&found, &query, around.settings);
            return true;
        }
        self.load(id, track, around, cx);
        false
    }

    /// Puts what `found` holds from the enabled sources on screen as the settled answer. The
    /// caller looks the next track up ahead afterwards.
    fn show(&mut self, found: &Found, query: &LyricsQuery, settings: &AppSettings) {
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
        self.hits = ordered(query, hits, &priority);
        self.state = state_for(&self.hits, instrumental);
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
    fn stored(
        &self,
        id: &str,
        around: &Around,
    ) -> impl Future<Output = Option<Found>> + Send + 'static {
        let key = self.key(id, around);
        let known: Vec<&'static str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .collect();
        let sheets = self.sheets.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                sheets
                    .get(&key, &known)
                    .map(|(hits, answered)| Found { hits, answered })
            })
            .await
            .ok()
            .flatten()
        }
    }

    /// One key per track. The stored answer covers every service that has replied, whichever
    /// are enabled now, and the selection is applied when it is shown. Services that replied
    /// empty are asked again while the stored answer holds nothing usable.
    fn key(&self, id: &str, around: &Around) -> String {
        match around.session.slug_for(id) {
            Some(slug) => format!("{slug}:{id}:lyrics"),
            None => format!("{id}:lyrics"),
        }
    }

    /// The providers a lookup for `id` may ask. The file's own lyrics are read only for a local
    /// file, and the services only when its metadata may go online.
    fn asked(&self, id: &str, around: &Around) -> Vec<Arc<dyn LyricsProvider>> {
        let online = self.online(id, around.settings);
        let reads_file = self.reads_file(id, around);
        let settings = around.settings;
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

    fn query(&self, track: &Track, id: &str, around: &Around) -> LyricsQuery {
        let key = around.session.slug_for(id).map(|provider| TrackKey {
            provider,
            id: id.to_owned(),
        });
        query_for(track, key)
    }

    /// Whether the services may be asked about this track, which a local file only allows when
    /// the user lets its metadata go online.
    fn online(&self, id: &str, settings: &AppSettings) -> bool {
        !music::is_local_id(id) || settings.lyrics_for_local_files()
    }

    fn reads_file(&self, id: &str, around: &Around) -> bool {
        music::is_local_id(id) && around.settings.lyrics_provider_enabled(LOCAL)
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

    fn forget(&mut self) {
        if self.following.is_none() {
            return;
        }
        self.task = None;
        self.following = None;
        self.hits.clear();
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.revision = self.revision.wrapping_add(1);
        self.state = LyricsState::Idle;
    }

    fn load(&mut self, id: String, track: Track, around: &Around, cx: &mut Cx) {
        self.hits.clear();
        self.state = LyricsState::Loading;

        if self.ahead_of.as_deref() == Some(id.as_str()) && self.ahead.is_some() {
            self.task = self.ahead.take();
            self.ahead_of = None;
            return;
        }
        self.task = Some(self.fetch(id, track, around, cx));
    }

    /// Looks `next`, the track after the shown one, up ahead of time.
    fn prefetch(&mut self, next: Option<Track>, around: &Around, cx: &mut Cx) {
        if !self.enabled || self.task.is_some() {
            return;
        }
        let Some(track) = next else {
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
        self.ahead = Some(self.fetch(id, track, around, cx));
    }

    /// Looks a track up: in memory, then in the cache database, then with the providers. Only
    /// the services that have not answered for this track are asked, unless nothing usable is
    /// stored, and a file's own lyrics are read again on every play. What the cache holds is
    /// shown alongside the new answers.
    fn fetch(&mut self, id: String, track: Track, around: &Around, cx: &mut Cx) -> Job {
        let settings = around.settings;
        let priority = settings.lyrics_provider_priority();
        let enabled: Vec<&'static str> = self
            .providers
            .iter()
            .map(|provider| provider.name())
            .filter(|name| settings.lyrics_provider_enabled(name))
            .collect();
        let mut asked = self.asked(&id, around);
        // Offline mode reads a file's own lyrics and what the cache kept, and asks no service.
        if around.offline {
            asked.retain(|provider| provider.name() == LOCAL);
        }
        let remembered = self.recall(&id);
        let stored = remembered.is_none().then(|| self.stored(&id, around));
        let query = self.query(&track, &id, around);
        let io = cx.io.clone();
        cx.spawn(|link| async move {
            let cached = match stored {
                Some(stored) => stored.await,
                None => remembered,
            };
            if let Some(found) = &cached {
                let held = found.clone();
                let id = id.clone();
                link.post(move |store| store.lyrics.hold(id, held));
            }
            let cached = cached.unwrap_or_default();
            // Services that replied empty are asked again while nothing usable is stored:
            // catalogues grow, and the one source that had anything may since have been
            // switched off, leaving a stored answer with no enabled hits.
            let stored = cached.usable(&query, &enabled);
            let providers: Vec<Arc<dyn LyricsProvider>> = asked
                .into_iter()
                .filter(|provider| !stored || !cached.covers(provider))
                .collect();
            if providers.is_empty() {
                link.post(move |store| {
                    if store.lyrics.finished(&id) {
                        log::debug!("lyrics: showing the stored answer");
                        store.lyrics.show(&cached, &query, &store.settings);
                        store.prefetch_lyrics();
                    }
                });
                return;
            }

            // The cached hits from enabled sources go in first, as if they had just arrived.
            let held: Vec<LyricsHit> = cached
                .hits
                .iter()
                .filter(|hit| enabled.contains(&hit.source))
                .filter(|hit| {
                    !providers
                        .iter()
                        .any(|provider| provider.name() == hit.source)
                })
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
                let id = id.clone();
                link.post(move |store| {
                    let this = &mut store.lyrics;
                    if this.following.as_deref() != Some(id.as_str()) {
                        return;
                    }
                    this.paint(ranked, anchor.as_ref(), waiting);
                });
            }

            let answered = join(worker).await;

            link.post(move |store| {
                let Store {
                    lyrics: this,
                    session,
                    settings,
                    network,
                    cx,
                    ..
                } = store;
                let around = Around {
                    session,
                    settings,
                    offline: network.offline(),
                };
                let current = this.finished(&id);
                let mut ahead = false;
                match answered {
                    Ok(answered) => {
                        cx.emit(Event::NetworkReached);
                        this.keep(id, cached, &hits, answered, &around, cx);
                        if current {
                            let instrumental = music::lyrics::instrumental(&ranking, &hits);
                            let ranked = ordered(&ranking, hits, &priority);
                            this.settle(ranked, displayed.as_ref(), instrumental);
                            ahead = true;
                        }
                    }
                    Err(error) => {
                        log::warn!("lyrics: cannot look up {}: {error:#}", track.name);
                        let reason = crate::blamed(&error, cx);
                        if current {
                            this.state = LyricsState::Failed(reason);
                        }
                    }
                }
                if ahead {
                    store.prefetch_lyrics();
                }
            });
        })
    }

    /// Shows an answer that arrived while others are still being looked for.
    ///
    /// A synced sheet settles the question the moment it turns up, unless `waiting` says a
    /// source that could still beat it has not answered yet. Short of that, the first synced
    /// answer is put up and nothing replaces it until the question settles, and an untimed one
    /// is not shown at all while something better could still arrive.
    fn paint(&mut self, ranked: Vec<LyricsHit>, displayed: Option<&LyricsHit>, waiting: bool) {
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
        self.apply(ranked, displayed);
    }

    fn apply(&mut self, ranked: Vec<LyricsHit>, displayed: Option<&LyricsHit>) {
        self.pin(ranked, displayed);
        if !self.hits.is_empty() {
            self.state = LyricsState::Ready;
        }
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
        mut found: Found,
        hits: &[LyricsHit],
        answered: Vec<&'static str>,
        around: &Around,
        cx: &mut Cx,
    ) {
        // A file's own lyrics are read again on every play, so they are never kept.
        if !self.online(&id, around.settings) {
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
                let key = self.key(&id, around);
                self.unsaved
                    .push(Sheets::unsaved(key, &found.hits, &found.answered));
                self.schedule_save(cx);
            }
        }
        self.hold(id, found);
    }

    /// Puts up the answer once every source has replied. Unless a word-by-word sheet already
    /// settled the question, this is the best there is and nothing may replace it afterwards.
    /// The caller looks the next track up ahead afterwards.
    fn settle(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        instrumental: bool,
    ) {
        if !self.settled {
            log::debug!("lyrics: settling on the best answer");
            self.settled = true;
            self.pin(ranked, displayed);
            self.state = state_for(&self.hits, instrumental);
        }
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
    fn schedule_save(&mut self, cx: &mut Cx) {
        self.save = Some(cx.spawn(|link| async move {
            tokio::time::sleep(SAVE_DELAY).await;
            let Some((sheets, unsaved)) = link
                .read(|store| {
                    let this = &mut store.lyrics;
                    (this.sheets.clone(), std::mem::take(&mut this.unsaved))
                })
                .await
            else {
                return;
            };
            drop(tokio::task::spawn_blocking(move || sheets.save(unsaved)));
        }));
    }
}

impl Store {
    /// Follows the shown track as playback changes and looks the next one up ahead as the queue
    /// changes. A change to whether lyrics are on, which sources are asked or their order
    /// looks the shown track up again.
    pub(crate) fn react_lyrics(&mut self, change: &Change) {
        match change {
            Change::Topic(Topic::Playback) => {
                self.follow_lyrics();
                self.prefetch_lyrics();
            }
            Change::Topic(Topic::Queue) => self.prefetch_lyrics(),
            Change::Topic(Topic::Settings) => {
                let (lyrics, settings) = (&mut self.lyrics, &self.settings);
                let on = settings.lyrics_enabled();
                let enabled = settings.lyrics_providers();
                let priority = settings.lyrics_provider_priority();
                if on == lyrics.enabled
                    && enabled == lyrics.enabled_providers
                    && priority == lyrics.priority
                {
                    return;
                }
                lyrics.enabled = on;
                lyrics.enabled_providers = enabled.to_vec();
                lyrics.priority = priority;
                lyrics.task = None;
                lyrics.ahead = None;
                lyrics.ahead_of = None;
                lyrics.forget();
                if self.follow_lyrics() {
                    self.prefetch_lyrics();
                }
            }
            _ => {}
        }
    }

    /// Follows the track playback shows. Returns whether a held answer went on screen at once.
    fn follow_lyrics(&mut self) -> bool {
        let shown = self.playback.shown_track(&self.devices);
        let Store {
            lyrics,
            session,
            settings,
            network,
            cx,
            ..
        } = self;
        let around = Around {
            session,
            settings,
            offline: network.offline(),
        };
        lyrics.follow(shown, &around, cx)
    }

    /// Looks up the track after the shown one ahead of time, unless the shown one is still
    /// being looked up.
    fn prefetch_lyrics(&mut self) {
        if !self.lyrics.enabled || self.lyrics.task.is_some() {
            return;
        }
        let next = self.shown_queue().track(QueuePosition::Upcoming(0));
        let Store {
            lyrics,
            session,
            settings,
            network,
            cx,
            ..
        } = self;
        let around = Around {
            session,
            settings,
            offline: network.offline(),
        };
        lyrics.prefetch(next, &around, cx);
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
    /// Whether an enabled source has lyrics that can be shown, or confirms an instrumental.
    fn usable(&self, query: &LyricsQuery, enabled: &[impl AsRef<str>]) -> bool {
        self.hits.iter().any(|hit| {
            enabled.iter().any(|name| name.as_ref() == hit.source)
                && (music::lyrics::eligible(query, hit)
                    || (hit.instrumental && music::lyrics::matched(query, hit)))
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
                    log::debug!(
                        "lyrics: {name} returned {} matching answer(s) for {}",
                        found.len(),
                        query.title
                    );
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
}
