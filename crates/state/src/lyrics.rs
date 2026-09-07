use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Entity, Task};
use music::{Lyrics as Sheet, LyricsHit, LyricsProvider, LyricsQuery, MusicApi, Track, TrackKey};
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

const OWN_TRUST: u32 = 25;
const SAVE_DELAY: Duration = Duration::from_millis(800);

struct Native {
    api: Arc<dyn MusicApi>,
    source: &'static str,
    id: String,
}

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

        if let Some(found) = self.remembered(&id, cx) {
            self.task = None;
            self.settled = true;
            self.hits = found.hits;
            self.state = state_for(&self.hits, found.instrumental);
            cx.notify();
            self.prefetch(cx);
            return;
        }
        self.load(id, track, cx);
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
        match self.session.read(cx).slug_for(id) {
            Some(slug) => format!("{slug}:{id}"),
            None => id.to_owned(),
        }
    }

    fn known(&self, cx: &Context<Self>) -> Vec<&'static str> {
        self.providers
            .iter()
            .map(|provider| provider.name())
            .chain(self.session.read(cx).provider_name())
            .collect()
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

    fn native(&self, id: &str, cx: &mut Context<Self>) -> Option<Native> {
        if music::is_local_id(id) {
            return None;
        }
        let session = self.session.read(cx);
        Some(Native {
            api: session.client()?,
            source: session.provider_name()?,
            id: id.to_owned(),
        })
    }

    fn load(&mut self, id: String, track: Track, cx: &mut Context<Self>) {
        if self.providers.is_empty() {
            self.state = LyricsState::Missing;
            cx.notify();
            return;
        }
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
        if self.providers.is_empty() || self.task.is_some() {
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
        if !self.settings.read(cx).lyrics_for_local_files() && music::is_local_id(&id) {
            log::info!(
                "lyrics: local files are disabled, skipping {:?}",
                track.name
            );
            self.state = LyricsState::Missing;
            return Task::ready(());
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
        let providers = self.providers.clone();
        let native = self.native(&id, cx);
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let (sender, mut incoming) = tokio::sync::mpsc::unbounded_channel();
            let ranking = query.clone();
            let worker = io.spawn(async move { gather(providers, native, query, sender).await });
            let mut hits = Vec::new();
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
                let current = this.following.as_deref() == Some(id.as_str());
                if current {
                    this.task = None;
                }
                if this.ahead_of.as_deref() == Some(id.as_str()) {
                    this.ahead_of = None;
                }
                match found {
                    Ok(()) => {
                        let instrumental = music::lyrics::instrumental(&ranking, &hits);
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
        if !hits.is_empty() || instrumental {
            self.store.put(self.key(&id, cx), &hits, instrumental);
            self.schedule_save(cx);
        }
        self.cache.insert(
            id,
            Found {
                hits: hits.clone(),
                instrumental,
            },
        );
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

#[derive(Clone)]
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
    native: Option<Native>,
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
    if let Some(native) = native {
        let query = query.clone();
        tasks.spawn(async move { own(native, query).await });
    }
    while let Some(found) = tasks.join_next().await {
        sender.send(found.unwrap_or_default()).ok();
    }
    Ok(())
}

async fn own(native: Native, query: LyricsQuery) -> Vec<LyricsHit> {
    let found = native
        .api
        .track_lyrics(&native.id)
        .await
        .inspect_err(|error| log::warn!("lyrics: {} did not answer: {error:#}", native.source))
        .unwrap_or_default();
    let Some(lyrics) = found.filter(|lyrics| !lyrics.is_empty()) else {
        return Vec::new();
    };
    vec![LyricsHit {
        source: native.source,
        trust: OWN_TRUST,
        lyrics,
        instrumental: false,
        title: query.title,
        artist: query.artist,
        album: query.album,
        duration: (!query.duration.is_zero()).then_some(query.duration),
        writers: Vec::new(),
    }]
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
