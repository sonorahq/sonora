use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Entity, Task};
use music::lyrics::LOCAL;
use music::{Lyrics as Sheet, LyricsHit, LyricsProvider, LyricsQuery, Track, TrackKey};
use tokio::task::JoinSet;

use crate::sheets::Sheets;
use crate::{AppSettings, Io, Playback, Queue, Session, join};

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

pub struct Lyrics {
    state: LyricsState,
    hits: Vec<LyricsHit>,
    chosen: usize,
    picked: bool,
    settled: bool,
    revision: u64,
    following: Option<String>,
    cache: HashMap<String, Found>,
    store: Sheets,
    providers: Vec<Arc<dyn LyricsProvider>>,
    enabled_providers: Vec<String>,
    prefer_local: bool,
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
    io: Io,
    task: Option<Task<()>>,
    ahead: Option<Task<()>>,
    ahead_of: Option<String>,
    save: Option<Task<()>>,
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
        cx.observe(&playback, |this, _, cx| this.follow(cx))
            .detach();
        cx.observe(&queue, |this, _, cx| this.prefetch(cx)).detach();
        cx.observe(&settings, |this, settings, cx| {
            let settings = settings.read(cx);
            let enabled = settings.lyrics_providers();
            let prefer_local = settings.prefer_local_lyrics();
            if enabled == this.enabled_providers && prefer_local == this.prefer_local {
                return;
            }
            this.enabled_providers = enabled.to_vec();
            this.prefer_local = prefer_local;
            this.task = None;
            this.ahead = None;
            this.ahead_of = None;
            this.cache.clear();
            this.forget(cx);
            this.follow(cx);
        })
        .detach();
        let (enabled_providers, prefer_local) = {
            let settings = settings.read(cx);
            (
                settings.lyrics_providers().to_vec(),
                settings.prefer_local_lyrics(),
            )
        };
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { Sheets::read() })
                .await;
            this.update(cx, |this, _| this.store.absorb(loaded)).ok();
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
            cache: HashMap::new(),
            store: Sheets::new(),
            providers,
            enabled_providers,
            prefer_local,
            playback,
            queue,
            session,
            settings,
            io,
            task: None,
            ahead: None,
            ahead_of: None,
            save: None,
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

    pub fn active_line(&self, cx: &Context<Self>) -> Option<usize> {
        let music::Lyrics::Synced { lines } = &self.current()?.lyrics else {
            return None;
        };
        music::lyrics::active(lines, self.playback.read(cx).position())
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let track = self.playback.read(cx).track().cloned();
        let Some(track) = track else {
            return self.forget(cx);
        };
        let Some(id) = track.id.clone() else {
            return self.forget(cx);
        };
        if self.following.as_deref() == Some(id.as_str()) {
            return;
        }
        self.following = Some(id.clone());
        self.chosen = 0;
        self.picked = false;
        self.settled = false;
        self.revision = self.revision.wrapping_add(1);

        // A file's own lyrics are never cached, so an edit to its tags shows on the next play.
        if !self.reads_file(&id, cx)
            && let Some(found) = self.remembered(&id, cx)
        {
            self.show(found, cx);
            return;
        }
        self.load(id, track, cx);
    }

    fn show(&mut self, found: Found, cx: &mut Context<Self>) {
        self.task = None;
        self.settled = true;
        self.hits = found.hits;
        self.state = state_for(&self.hits, found.instrumental);
        cx.notify();
        self.prefetch(cx);
    }

    fn remembered(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Found> {
        if let Some(found) = self.cache.get(id) {
            return Some(found.clone());
        }
        let (hits, instrumental) = self.store.get(&self.key(id, cx), &self.known(cx))?;
        let found = Found { hits, instrumental };
        self.cache.insert(id.to_owned(), found.clone());
        Some(found)
    }

    fn key(&self, id: &str, cx: &Context<Self>) -> String {
        let track = match self.session.read(cx).slug_for(id) {
            Some(slug) => format!("{slug}:{id}"),
            None => id.to_owned(),
        };
        // A result is complete only for the sources queried. Changing the selection must
        // not reuse a sheet that includes disabled sources or omits newly enabled ones.
        // The file's own lyrics are never stored, so turning Local on keeps the services' sheets.
        let services: Vec<&str> = self
            .known(cx)
            .into_iter()
            .filter(|name| *name != LOCAL)
            .collect();
        format!("{track}:lyrics:{}", services.join(","))
    }

    fn known(&self, cx: &Context<Self>) -> Vec<&'static str> {
        self.providers
            .iter()
            .filter(|provider| {
                self.settings
                    .read(cx)
                    .lyrics_provider_enabled(provider.name())
            })
            .map(|provider| provider.name())
            .collect()
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
        let next = self.queue.read(cx).upcoming().next().cloned();
        let Some((track, id)) = next.and_then(|track| Some((track.clone(), track.id?))) else {
            return;
        };
        if self.ahead_of.as_deref() == Some(id.as_str())
            || self.cache.contains_key(&id)
            || self.store.holds(&self.key(&id, cx))
        {
            return;
        }
        self.ahead_of = Some(id.clone());
        self.ahead = Some(self.fetch(id, track, cx));
    }

    fn fetch(&mut self, id: String, track: Track, cx: &mut Context<Self>) -> Task<()> {
        let online = self.online(&id, cx);
        let settings = self.settings.read(cx);
        let prefer_local = settings.prefer_local_lyrics();
        // A file's own lyrics never leave the computer, so only the services are held back.
        let mut providers: Vec<Arc<dyn LyricsProvider>> = self
            .providers
            .iter()
            .filter(|provider| settings.lyrics_provider_enabled(provider.name()))
            .filter(|provider| online || provider.name() == LOCAL)
            .cloned()
            .collect();
        if !online && providers.is_empty() {
            log::info!(
                "lyrics: local files are disabled, skipping {:?}",
                track.name
            );
            self.state = LyricsState::Missing;
            return Task::ready(());
        }
        let cached = match self.reads_file(&id, cx) {
            true => self.remembered(&id, cx),
            false => None,
        };
        if cached.is_some() {
            // The services already answered for this track; only the file is read again.
            providers.retain(|provider| provider.name() == LOCAL);
        }

        let key = self
            .session
            .read(cx)
            .slug_for(&id)
            .map(|provider| TrackKey {
                provider,
                id: id.clone(),
            });
        let query = query_for(&track, key);
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            if prefer_local
                && let Some(index) = providers
                    .iter()
                    .position(|provider| provider.name() == LOCAL)
            {
                let local = providers.remove(index);
                let asked = query.clone();
                let read = io.spawn(async move { local.search(&asked).await });
                let hits = match join(read).await {
                    Ok(found) => ordered(&query, found),
                    Err(error) => {
                        log::warn!(
                            "lyrics: cannot read the lyrics in {}: {error:#}",
                            track.name
                        );
                        Vec::new()
                    }
                };
                if !hits.is_empty() {
                    let found = Found {
                        hits,
                        instrumental: false,
                    };
                    this.update(cx, |this, cx| {
                        if this.finished(&id) {
                            this.show(found, cx);
                        }
                    })
                    .ok();
                    return;
                }
            }

            let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel();
            let ranking = query.clone();
            let worker = io.spawn(async move { gather(providers, query, sender).await });
            let Found {
                mut hits,
                instrumental: cached_instrumental,
            } = cached.unwrap_or_default();
            let mut displayed: Option<LyricsHit> = None;
            let mut shown: Option<u8> = None;

            while let Some(mut found) = incoming.recv().await {
                hits.append(&mut found);
                let ranked = ordered(&ranking, hits.clone());
                let Some(best) = ranked.first().cloned() else {
                    continue;
                };
                let step = depth(&best.lyrics);
                if shown.is_none_or(|shown| step >= shown) {
                    shown = Some(step);
                    displayed = Some(best);
                }
                let anchor = displayed.clone();
                this.update(cx, |this, cx| {
                    if this.following.as_deref() != Some(id.as_str()) {
                        return;
                    }
                    this.paint(ranked, anchor.as_ref(), cx);
                })
                .ok();
            }

            let found = join(worker).await;

            this.update(cx, |this, cx| {
                let current = this.finished(&id);
                match found {
                    Ok(()) => {
                        let instrumental =
                            cached_instrumental || music::lyrics::instrumental(&ranking, &hits);
                        let ranked = ordered(&ranking, hits);
                        this.remember(id, ranked, displayed.as_ref(), instrumental, current, cx);
                    }
                    Err(error) => {
                        log::warn!("lyrics: cannot look up {}: {error:#}", track.name);
                        if current {
                            this.state = LyricsState::Failed(format!("{error:#}"));
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
    /// A word-by-word sheet is the best there is, so it goes up the moment it
    /// turns up and settles the question rather than waiting for the slowest
    /// source to answer. Short of that, the first timed answer is put up and
    /// nothing replaces it until the search is over: the reader is never walked
    /// through a series of ever better sheets, and an untimed one is not shown at
    /// all while something better could still arrive.
    fn paint(
        &mut self,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        cx: &mut Context<Self>,
    ) {
        if self.settled {
            return;
        }
        let kind = self
            .prospect(&ranked, displayed)
            .map(|hit| depth(&hit.lyrics));
        let best = kind == Some(WORDED);
        if !best && !self.hits.is_empty() {
            return;
        }
        if !best && kind != Some(TIMED) {
            return;
        }
        match best {
            true => {
                log::debug!("lyrics: settling on a word-by-word answer as it arrives");
                self.settled = true;
            }
            false => log::debug!("lyrics: showing the first timed answer while the rest arrive"),
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

    fn remember(
        &mut self,
        id: String,
        ranked: Vec<LyricsHit>,
        displayed: Option<&LyricsHit>,
        instrumental: bool,
        current: bool,
        cx: &mut Context<Self>,
    ) {
        let mut hits = ranked;
        keep_displayed_first(&mut hits, displayed);
        // A file's own lyrics are read again on every play. Services kept out of this lookup must
        // still be asked once allowed, and an answer already cached needs no second write.
        if self.online(&id, cx) && !self.cache.contains_key(&id) {
            let kept: Vec<LyricsHit> = hits
                .iter()
                .filter(|hit| hit.source != LOCAL)
                .cloned()
                .collect();
            if !kept.is_empty() || instrumental {
                self.store.put(self.key(&id, cx), &kept, instrumental);
                self.schedule_save(cx);
            }
            self.cache.insert(
                id,
                Found {
                    hits: kept,
                    instrumental,
                },
            );
        }
        if current {
            // Every source has answered. Unless a word-by-word sheet already
            // settled the question, this is the best there is and nothing may
            // replace it afterwards.
            if !self.settled {
                log::debug!("lyrics: settling on the best answer");
                self.settled = true;
                self.pin(hits, displayed);
                self.state = state_for(&self.hits, instrumental);
            }
            cx.notify();
            self.prefetch(cx);
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

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let chore = this.update(cx, |this, _| this.store.chore()).ok().flatten();
            if let Some(chore) = chore {
                cx.background_executor()
                    .spawn(async move { chore.write() })
                    .await;
            }
        }));
    }
}

/// What a sheet is worth: word-by-word beats timed, timed beats untimed.
const WORDED: u8 = 3;
const TIMED: u8 = 2;

fn depth(lyrics: &Sheet) -> u8 {
    match (lyrics.worded(), lyrics.synced()) {
        (true, _) => WORDED,
        (false, true) => TIMED,
        (false, false) => 1,
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

fn ordered(query: &LyricsQuery, hits: Vec<LyricsHit>) -> Vec<LyricsHit> {
    let mut ranked = music::lyrics::rank(query, hits);
    music::lyrics::reshape(&mut ranked);
    ranked
}

#[derive(Clone, Default)]
struct Found {
    hits: Vec<LyricsHit>,
    instrumental: bool,
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

async fn gather(
    providers: Vec<Arc<dyn LyricsProvider>>,
    query: LyricsQuery,
    sender: tokio::sync::mpsc::UnboundedSender<Vec<LyricsHit>>,
) -> anyhow::Result<()> {
    let mut tasks = JoinSet::new();
    for provider in providers {
        let query = query.clone();
        tasks.spawn(async move {
            provider
                .search(&query)
                .await
                .inspect_err(|error| {
                    log::warn!("lyrics: {} did not answer: {error:#}", provider.name())
                })
                .unwrap_or_default()
        });
    }
    while !tasks.is_empty() {
        tokio::select! {
            // Dropping the foreground lookup (including on a provider change) must also
            // stop its network requests. Dropping this JoinSet aborts the remaining tasks.
            _ = sender.closed() => break,
            found = tasks.join_next() => {
                if let Some(found) = found {
                    sender.send(found.unwrap_or_default()).ok();
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use music::{Lyrics as Sheet, LyricsHit, LyricsLine, LyricsWord, Voice};

    use super::keep_displayed_first;

    fn hit(source: &'static str, lyrics: Sheet) -> LyricsHit {
        LyricsHit {
            source,
            trust: 0,
            lyrics,
            instrumental: false,
            title: "title".to_owned(),
            artist: "artist".to_owned(),
            album: None,
            duration: None,
            writers: Vec::new(),
        }
    }

    #[test]
    fn a_displayed_karaoke_result_stays_selected_after_final_ranking() {
        let plain = hit("plain", Sheet::plain("line"));
        let displayed = hit(
            "karaoke",
            Sheet::Synced {
                lines: vec![LyricsLine {
                    start: Duration::ZERO,
                    end: Some(Duration::from_secs(1)),
                    text: "line".to_owned(),
                    romanized: None,
                    words: Some(vec![LyricsWord {
                        start: Duration::ZERO,
                        end: Duration::from_secs(1),
                        text: "line".to_owned(),
                    }]),
                    secondary: Vec::new(),
                    voice: Voice::Lead,
                }]
                .into(),
            },
        );
        let mut final_ranking = vec![plain, displayed.clone()];

        keep_displayed_first(&mut final_ranking, Some(&displayed));

        assert_eq!(final_ranking.first(), Some(&displayed));
    }
}
