use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use futures::StreamExt as _;
use gpui::{App, Context, Entity, Task};
use logic::stats::RecordedHistory;
use music::{
    Album, CoverSize, Descriptors, ListeningStats, Milestone, MusicApi, Overview, SavedArtist,
    StatsPeriod, Track, TrackBreakdown, TrackTotals, album_key,
};
use scrobble::Service;
use storage::Database;

use crate::history::Store;
use crate::{History, Io, Scrobbling, Session, SessionEvent, join};

/// Resolve these top entries eagerly and match the rest when opened.
const MATCHED: usize = 12;
/// How many recorded artists get a portrait, and how many are requested at once.
const PORTRAITS: usize = 100;
const PORTRAIT_BATCH: usize = 25;
const CONCURRENT: usize = 6;
/// How long the dates sit still before a scrobbler is asked about them.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// How long recorded plays wait for the dates to settle, which is only a pass over the log.
const SETTLE: Duration = Duration::from_millis(120);
/// How long a load counts as current when the page is opened again.
const FRESH: Duration = Duration::from_secs(300);
/// How many of the most played recorded tracks are asked for genres and moods, and how many at once.
const DESCRIBED: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatsSource {
    /// The signed-in provider's own rankings, over its fixed windows.
    Provider,
    /// The timestamped plays Sonora recorded, over any range of dates.
    Recorded,
    /// A linked scrobbling service, by its id.
    Scrobbler(&'static str),
}

impl StatsSource {
    /// Whether the source accepts an inclusive range of calendar dates.
    pub fn supports_dates(self) -> bool {
        self != Self::Provider
    }
}

/// A remote ranking window that can be reused until it expires.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Window {
    Provider(StatsPeriod),
    Scrobbler(&'static str, Option<(i64, i64)>),
}

/// A scrobbler chart entry that names something in the active catalogue without an id of its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    Artist { name: String },
    Track { title: String, artist: String },
    Album { title: String, artist: String },
}

/// The catalogue item an [`Entry`] matched.
#[derive(Clone, Debug)]
pub enum Found {
    Artist(SavedArtist),
    Track(Track),
    Album(Album),
}

impl From<&SavedArtist> for Entry {
    fn from(item: &SavedArtist) -> Self {
        Self::Artist {
            name: item.name.clone(),
        }
    }
}

impl From<&Track> for Entry {
    fn from(item: &Track) -> Self {
        Self::Track {
            title: item.name.clone(),
            artist: item.artists.clone(),
        }
    }
}

impl From<&Album> for Entry {
    fn from(item: &Album) -> Self {
        Self::Album {
            title: item.name.clone(),
            artist: item.artists.clone(),
        }
    }
}

impl Entry {
    /// Searches the catalogue and keeps a result only when its names match exactly.
    async fn find(&self, client: &dyn MusicApi) -> Result<Option<Found>> {
        Ok(match self {
            Self::Artist { name } => client
                .search_artists(name)
                .await?
                .into_iter()
                .find(|found| !found.id.is_empty() && same(&found.name, name))
                .map(Found::Artist),
            Self::Track { title, artist } => client
                .search(&format!("{title} {artist}"))
                .await?
                .into_iter()
                .find(|found| {
                    found.id.is_some() && named(&found.name, &found.artists, title, artist)
                })
                .map(Found::Track),
            Self::Album { title, artist } => client
                .search_albums(&format!("{title} {artist}"))
                .await?
                .into_iter()
                .find(|found| {
                    !found.id.is_empty() && named(&found.name, &found.artists, title, artist)
                })
                .map(Found::Album),
        })
    }

    /// Applies a catalogue match to all matching rows, preserving chart artwork when missing.
    fn adopt(&self, found: &Found, data: &mut ListeningStats) {
        match (self, found) {
            (Self::Artist { name }, Found::Artist(found)) => {
                for row in &mut data.artists {
                    if row.item.id.is_empty() && same(&row.item.name, name) {
                        let cover = found.cover.clone().or_else(|| row.item.cover.take());
                        row.item = SavedArtist {
                            cover,
                            ..found.clone()
                        };
                    }
                }
            }
            (Self::Track { title, artist }, Found::Track(found)) => {
                for row in &mut data.tracks {
                    let item = &mut row.item;
                    if item.id.is_none() && named(&item.name, &item.artists, title, artist) {
                        let cover = found.cover.clone().or_else(|| item.cover.take());
                        *item = Track {
                            cover,
                            ..found.clone()
                        };
                    }
                }
            }
            (Self::Album { title, artist }, Found::Album(found)) => {
                for row in &mut data.albums {
                    let item = &mut row.item;
                    if item.id.is_empty() && named(&item.name, &item.artists, title, artist) {
                        let chart = item.cover.take();
                        *item = Album {
                            cover: found.cover.clone().or_else(|| chart.clone()),
                            cover_large: found.cover_large.clone().or(chart),
                            ..found.clone()
                        };
                    }
                }
            }
            _ => {}
        }
    }
}

/// Recorded history and its enrichment, retained across date changes.
#[derive(Default)]
struct Log {
    recorded: RecordedHistory,
    /// Genres and moods the provider gave the most played recorded tracks.
    descriptors: HashMap<String, Descriptors>,
    portraits: HashMap<String, String>,
    /// Plays per slice of the scrobbler account's history, which a date range never changes.
    scrobbled_density: Vec<usize>,
    /// The scrobbler account's round-number plays, read once with the all-time view.
    scrobbled_milestones: Vec<Milestone>,
    /// The selected range of recorded plays, in Unix seconds, or all of them.
    dates: Option<(i64, i64)>,
}

/// Owns stats requests, the retained play log and date filtering, independently of any view.
pub struct Stats {
    session: Entity<Session>,
    history: Entity<History>,
    scrobbling: Entity<Scrobbling>,
    store: Store,
    source: StatsSource,
    period: StatsPeriod,
    log: Log,
    /// What was last loaded and when, so that opening the page again need not load it again.
    loaded: Option<(StatsSource, StatsPeriod, Instant)>,
    /// When the scrobbler account began, in Unix seconds.
    since: Option<i64>,
    data: ListeningStats,
    opened: bool,
    loading: bool,
    failed: bool,
    task: Option<Task<()>>,
    /// The wait for dragged dates to settle, kept apart from `task` so that it never cancels a load.
    settling: Option<Task<()>>,
    matching: Option<Task<()>>,
    opening: Option<Task<()>>,
    /// The read of what only a whole scrobbler account says, which runs beside the page.
    history_task: Option<Task<()>>,
    /// Catalogue matches and misses for the current provider.
    matched: HashMap<Entry, Option<Found>>,
    matched_for: Option<String>,
    windows: HashMap<Window, (Instant, ListeningStats)>,
}

impl Stats {
    pub fn new(
        session: Entity<Session>,
        history: Entity<History>,
        scrobbling: Entity<Scrobbling>,
        database: Database,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| {
            if matches!(event, SessionEvent::SignedOut | SessionEvent::SignedIn) {
                (this.source, this.period) = defaults(&this.session, cx);
                this.log = Log::default();
                this.since = None;
                this.windows.clear();
                this.matched.clear();
                this.history_task = None;
                if this.opened {
                    this.refresh(cx);
                }
            }
        })
        .detach();
        cx.observe(&history, |this, _, cx| {
            // The history notifies whenever the window comes back to the front, so a change in
            // the log is checked for quietly rather than reloading a page that is being read.
            if this.source == StatsSource::Recorded && this.loaded.is_some() && this.task.is_none()
            {
                this.load_recorded(true, cx);
            }
        })
        .detach();
        cx.observe(&scrobbling, |this, _, cx| {
            if matches!(this.source, StatsSource::Scrobbler(_)) {
                this.ensure(cx);
            }
            cx.notify();
        })
        .detach();
        let (source, period) = defaults(&session, cx);
        Self {
            session,
            history,
            scrobbling,
            store: Store::new(database),
            source,
            period,
            log: Log::default(),
            loaded: None,
            since: None,
            data: ListeningStats::default(),
            opened: false,
            loading: false,
            failed: false,
            task: None,
            settling: None,
            matching: None,
            opening: None,
            history_task: None,
            matched: HashMap::new(),
            matched_for: None,
            windows: HashMap::new(),
        }
    }

    pub fn source(&self) -> StatsSource {
        self.source
    }

    pub fn sources(&self, cx: &App) -> Vec<StatsSource> {
        let mut sources = vec![StatsSource::Recorded];
        if !self.periods_of(StatsSource::Provider, cx).is_empty() {
            sources.push(StatsSource::Provider);
        }
        let accounts = self.scrobbling.read(cx);
        sources.extend(
            scrobble::services()
                .into_iter()
                .filter(|service| {
                    service.stats_dated()
                        && accounts
                            .rows()
                            .iter()
                            .any(|row| row.id() == service.id() && row.account().linked())
                })
                .map(|service| StatsSource::Scrobbler(service.id())),
        );
        sources
    }

    /// The fixed windows of the selected source. Sources that read any dates have none.
    pub fn periods(&self, cx: &App) -> Vec<StatsPeriod> {
        self.periods_of(self.source, cx)
    }

    pub fn milestones(&self) -> &[Milestone] {
        match self.source {
            StatsSource::Scrobbler(_) => &self.log.scrobbled_milestones,
            _ => &self.data().milestones,
        }
    }

    pub fn period(&self) -> StatsPeriod {
        self.period
    }

    pub fn data(&self) -> &ListeningStats {
        static EMPTY: std::sync::LazyLock<ListeningStats> =
            std::sync::LazyLock::new(ListeningStats::default);
        match self.window() {
            Some(key) => self.windows.get(&key).map_or(&EMPTY, |(_, stats)| stats),
            None => &self.data,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Known history limits for the activity scrubber, when the source reports them.
    pub fn history_bounds(&self) -> Option<(i64, i64)> {
        match self.source {
            StatsSource::Scrobbler(_) => self
                .since
                .map(|since| (since, jiff::Timestamp::now().as_second())),
            StatsSource::Recorded => self.log.recorded.bounds,
            StatsSource::Provider => None,
        }
    }

    /// The selected range of dates, or none for all time.
    pub fn dates(&self) -> Option<(i64, i64)> {
        self.log.dates
    }

    /// Whole-history overview, available for recorded plays.
    pub fn history(&self) -> Option<&Overview> {
        (self.source == StatsSource::Recorded).then_some(&self.log.recorded.overview)
    }

    /// How many plays fall in each equal slice of [`Self::history_bounds`].
    pub fn density(&self) -> &[usize] {
        match self.source {
            StatsSource::Recorded => &self.log.recorded.density,
            StatsSource::Scrobbler(_) => &self.log.scrobbled_density,
            StatsSource::Provider => &[],
        }
    }

    /// Switches source, moving to the new source's first window when it lacks `period`.
    pub fn select(&mut self, source: StatsSource, period: StatsPeriod, cx: &mut Context<Self>) {
        let periods = self.periods_of(source, cx);
        if source != self.source || source == StatsSource::Provider {
            self.log.dates = None;
        }
        self.settling = None;
        self.source = source;
        self.period = match periods.contains(&period) {
            true => period,
            false => periods.last().copied().unwrap_or(period),
        };
        self.reload(cx);
    }

    /// Selects inclusive Unix-second bounds and debounces history updates.
    pub fn set_dates(&mut self, dates: Option<(i64, i64)>, cx: &mut Context<Self>) {
        if !self.source.supports_dates() {
            return;
        }
        self.log.dates = dates;
        // A drag sends many dates, and each answer is a pass over the whole log or a request, so
        // the work waits for the dates to settle.
        let (wait, remote) = match self.source {
            StatsSource::Recorded => (SETTLE, false),
            _ => {
                self.task = None;
                self.matching = None;
                (DEBOUNCE, true)
            }
        };
        self.settling = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.settling = None;
                match remote {
                    true => this.reload(cx),
                    false => this.data = this.aggregated(),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Loads what the selected source shows, unless that is loading or was loaded recently.
    pub fn ensure(&mut self, cx: &mut Context<Self>) {
        self.opened = true;
        let current = self.loaded.is_some_and(|(source, period, at)| {
            source == self.source && period == self.period && at.elapsed() < FRESH
        });
        if !(self.loading || self.settling.is_some() || current || self.failed) {
            self.refresh(cx);
        }
    }

    /// Reloads whatever the selected source shows, bypassing any cached window.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(window) = self.window() {
            self.windows.remove(&window);
        }
        self.reload(cx);
    }

    /// The cache key for a provider preset or scrobbler date range.
    fn window(&self) -> Option<Window> {
        match self.source {
            StatsSource::Provider => Some(Window::Provider(self.period)),
            StatsSource::Scrobbler(id) => Some(Window::Scrobbler(id, self.log.dates)),
            StatsSource::Recorded => None,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.settling = None;
        self.loaded = None;
        self.task = None;
        self.matching = None;
        self.opening = None;
        self.failed = false;
        self.loading = false;
        let client = self.session.read(cx).client();
        if let Some((at, _)) = self
            .window()
            .and_then(|key| self.windows.get(&key))
            .filter(|(at, _)| at.elapsed() < FRESH)
        {
            self.loaded = Some((self.source, self.period, *at));
            cx.notify();
            return;
        }
        match (self.source, client) {
            (StatsSource::Provider, Some(client)) => {
                self.load_provider(client, cx);
            }
            (StatsSource::Scrobbler(id), client) => {
                self.load_scrobbler(id, client, cx);
            }
            (StatsSource::Provider, None) => self.failed = true,
            (StatsSource::Recorded, _) => {
                self.data = ListeningStats::default();
                self.load_recorded(false, cx);
            }
        }
        cx.notify();
    }

    /// Matches a scrobbler chart entry to the active catalogue, and tells `resolved` the result.
    pub fn resolve(
        &mut self,
        entry: Entry,
        resolved: impl FnOnce(Option<Found>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.session.read(cx).client() else {
            return resolved(None, cx);
        };
        let io = Io::global(cx);
        let window = self.window();
        self.opening = Some(cx.spawn(async move |this, cx| {
            let worker = entry.clone();
            let found = join(io.spawn(async move { worker.find(client.as_ref()).await }));
            let found = found.await.unwrap_or_else(|error| {
                log::debug!("stats: cannot match {entry:?} to the catalogue: {error:#}");
                None
            });
            this.update(cx, |this, cx| {
                this.opening = None;
                if let Some(found) = &found
                    && let Some((_, data)) = window.and_then(|key| this.windows.get_mut(&key))
                {
                    entry.adopt(found, data);
                    cx.notify();
                }
                resolved(found, cx);
            })
            .ok();
        }));
    }

    fn periods_of(&self, source: StatsSource, cx: &App) -> Vec<StatsPeriod> {
        match source {
            StatsSource::Provider => self
                .session
                .read(cx)
                .client()
                .map(|client| client.stats_periods().to_vec())
                .unwrap_or_default(),
            StatsSource::Recorded | StatsSource::Scrobbler(_) => Vec::new(),
        }
    }

    /// Stores a finished load, or the reason it failed.
    fn finish(&mut self, result: Result<ListeningStats>, what: &str, cx: &mut Context<Self>) {
        self.loading = false;
        self.task = None;
        match result {
            Ok(data) => {
                let now = Instant::now();
                self.publish(self.window(), data, now);
                self.loaded = Some((self.source, self.period, now));
            }
            Err(error) => {
                log::warn!("stats: cannot load {what}: {error:#}");
                self.failed = true;
            }
        }
        cx.notify();
    }

    /// Keeps the answer with the window that requested it and displays it if still selected.
    fn publish(&mut self, window: Option<Window>, data: ListeningStats, at: Instant) {
        if let Some(window) = window {
            self.windows.insert(window, (at, data));
        } else {
            self.data = data;
        }
    }

    /// Publishes rankings first, then fills credit portraits as batches finish.
    fn load_provider(&mut self, client: Arc<dyn MusicApi>, cx: &mut Context<Self>) {
        let period = self.period;
        let io = Io::global(cx);
        self.loading = true;
        let looked_up = client.clone();
        self.task = Some(io.clone().load(
            async move {
                let mut data = client.stats(period).await?;
                enlarge_ranked(&mut data, client.as_ref());
                Ok(data)
            },
            move |this, mut result, cx| {
                let mut seen = HashSet::new();
                let mut missing = Vec::new();
                if let Ok(data) = &mut result {
                    for row in data.songwriters.iter_mut().chain(&mut data.producers) {
                        let id = &row.item.id;
                        if let Some(cover) = this.log.portraits.get(id) {
                            row.item.cover = Some(cover.clone());
                        } else if seen.insert(id.clone()) {
                            missing.push(id.clone());
                        }
                    }
                }
                this.finish(result, "provider rankings", cx);
                if missing.is_empty() {
                    return;
                }
                let mut pending: futures::stream::FuturesUnordered<_> = missing
                    .chunks(PORTRAIT_BATCH)
                    .map(|batch| {
                        let (client, ids) = (looked_up.clone(), batch.to_vec());
                        join(io.spawn(async move { client.artist_images(ids).await }))
                    })
                    .collect();
                this.matching = Some(cx.spawn(async move |this, cx| {
                    while let Some(found) = pending.next().await {
                        let Ok(found) = found else { continue };
                        this.update(cx, |this, cx| {
                            this.log.portraits.extend(found);
                            if let Some((_, data)) = this.windows.get_mut(&Window::Provider(period))
                            {
                                for row in data.songwriters.iter_mut().chain(&mut data.producers) {
                                    if let Some(cover) = this.log.portraits.get(&row.item.id) {
                                        row.item.cover = Some(cover.clone());
                                    }
                                }
                            }
                            cx.notify();
                        })
                        .ok();
                    }
                }));
            },
            cx,
        ));
    }

    /// Publishes the scrobbler chart before resolving catalogue links.
    fn load_scrobbler(
        &mut self,
        id: &'static str,
        client: Option<Arc<dyn MusicApi>>,
        cx: &mut Context<Self>,
    ) {
        let account = self.scrobbling.read(cx).account(id);
        let Some(service) = service(id).filter(|service| account.linked() && service.stats_dated())
        else {
            self.failed = true;
            return;
        };
        let dates = self.log.dates;
        let io = Io::global(cx);
        // The matches belong to one catalogue, so another provider starts them afresh.
        let provider = self.session.read(cx).provider_slug().map(str::to_owned);
        if provider != self.matched_for {
            self.matched.clear();
            self.matched_for = provider;
        }
        let known = self.matched.clone();
        let history = (self.log.scrobbled_density.is_empty()
            && self.log.scrobbled_milestones.is_empty())
        .then(|| (service.clone(), account.clone()));
        if let Some((service, account)) = history {
            self.history_task = Some(io.load(
                async move { service.history(&account).await },
                move |this, result, cx| {
                    this.history_task = None;
                    match result {
                        Ok(history) => {
                            this.log.scrobbled_density = history.density;
                            this.log.scrobbled_milestones = history.milestones;
                        }
                        Err(error) => log::debug!("stats: cannot load history: {error:#}"),
                    }
                    cx.notify();
                },
                cx,
            ));
        }
        self.loading = true;
        self.task = Some(io.clone().load(
            async move { service.stats(&account, dates).await },
            move |this, result, cx| {
                if let Ok(data) = &result {
                    this.since = data.overview.since.or(this.since);
                }
                if let (Ok(data), Some(client)) = (&result, client) {
                    let data = data.clone();
                    let known_descriptors = this.log.descriptors.clone();
                    this.matching = Some(io.clone().load(
                        async move {
                            Ok(
                                match_catalogue(data, client.as_ref(), known, known_descriptors)
                                    .await,
                            )
                        },
                        move |this, result, cx| {
                            this.matching = None;
                            match result {
                                Ok((data, found, new_descriptors)) => {
                                    this.matched.extend(found);
                                    this.log.descriptors.extend(new_descriptors);
                                    this.publish(
                                        Some(Window::Scrobbler(id, dates)),
                                        data,
                                        Instant::now(),
                                    );
                                }
                                Err(error) => log::debug!("stats: cannot match chart: {error:#}"),
                            }
                            cx.notify();
                        },
                        cx,
                    ));
                }
                this.finish(result, "scrobbler rankings", cx);
            },
            cx,
        ));
    }

    /// Quiet reads only publish a changed log.
    fn load_recorded(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let Some(scope) = self.history.read(cx).scope(cx) else {
            return;
        };
        let store = self.store.clone();
        let io = Io::global(cx);
        let client = self.session.read(cx).client();
        let known = (
            self.log.recorded.tracks.len(),
            self.log.recorded.tracks.first().map(|track| track.added_at),
        );
        self.loading = !quiet;
        self.task = Some(io.clone().load(
            async move {
                io.spawn_blocking(move || {
                    let mut tracks = store.recorded(&scope)?;
                    if quiet && (tracks.len(), tracks.first().map(|track| track.added_at)) == known
                    {
                        return Ok(None);
                    }
                    prepare_covers(&mut tracks, client.as_deref());
                    Ok(Some(RecordedHistory::new(tracks)))
                })
                .await?
            },
            move |this, result, cx| match result {
                Ok(None) => this.task = None,
                Ok(Some(history)) => {
                    this.log.recorded = history;
                    let data = this.aggregated();
                    this.finish(Ok(data), "recorded plays", cx);
                    this.enrich(cx);
                }
                Err(error) => this.finish(Err(error), "recorded plays", cx),
            },
            cx,
        ));
    }

    /// Enriches the recorded chart with descriptors and portraits.
    fn enrich(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        let described: Vec<_> = self
            .data
            .tracks
            .iter()
            .map(|ranked| &ranked.item)
            .filter(|track| track.tags.is_empty())
            .filter_map(|track| track.id.clone())
            .filter(|id| !self.log.descriptors.contains_key(id))
            .take(DESCRIBED)
            .collect();
        let portraits: Vec<_> = self
            .data
            .artists
            .iter()
            .map(|ranked| &ranked.item.id)
            .filter(|id| !id.is_empty() && !self.log.portraits.contains_key(*id))
            .take(PORTRAITS)
            .cloned()
            .collect();
        if described.is_empty() && portraits.is_empty() {
            return;
        }
        self.matching = Some(Io::global(cx).load(
            async move {
                let client = client.as_ref();
                Ok(tokio::join!(
                    fetch_descriptors(client, described),
                    fetch_portraits(client, portraits),
                ))
            },
            |this, result, cx| {
                this.matching = None;
                if let Ok((descriptors, portraits)) = result {
                    this.log.descriptors.extend(descriptors);
                    this.log.portraits.extend(portraits);
                    this.publish(None, this.aggregated(), Instant::now());
                    cx.notify();
                }
            },
            cx,
        ));
    }

    /// The recorded plays inside the selected dates, with portraits applied.
    fn aggregated(&self) -> ListeningStats {
        let mut data = self
            .log
            .recorded
            .stats(self.log.dates, &self.log.descriptors);
        for artist in &mut data.artists {
            if let Some(cover) = self.log.portraits.get(&artist.item.id) {
                artist.item.cover = Some(cover.clone());
            }
        }
        data
    }
}

/// Starts on the broadest provider preset available, otherwise recorded history.
fn defaults(session: &Entity<Session>, cx: &App) -> (StatsSource, StatsPeriod) {
    let first = session.read(cx).client().and_then(|client| {
        let periods = client.stats_periods();
        periods
            .iter()
            .find(|&&period| period == StatsPeriod::Year)
            .or_else(|| periods.last())
            .copied()
    });
    match first {
        Some(period) => (StatsSource::Provider, period),
        None => (StatsSource::Recorded, StatsPeriod::Weeks),
    }
}

fn service(id: &str) -> Option<Arc<dyn Service>> {
    scrobble::services()
        .into_iter()
        .find(|service| service.id() == id)
}

/// Folds punctuation and case, so that a chart's spelling can match the catalogue's.
fn comparable(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether two names are the same once folded, which a blank never is.
fn same(a: &str, b: &str) -> bool {
    let a = comparable(a);
    !a.is_empty() && a == comparable(b)
}

/// Whether a title and its artist are the ones a chart entry names.
fn named(title: &str, artist: &str, wanted_title: &str, wanted_artist: &str) -> bool {
    same(title, wanted_title) && same(artist, wanted_artist)
}

/// Resolves uncached chart entries concurrently and returns new matches for reuse.
async fn match_catalogue(
    mut data: ListeningStats,
    client: &dyn MusicApi,
    known: HashMap<Entry, Option<Found>>,
    mut known_descriptors: HashMap<String, Descriptors>,
) -> (
    ListeningStats,
    HashMap<Entry, Option<Found>>,
    HashMap<String, Descriptors>,
) {
    let entries: Vec<Entry> = data
        .artists
        .iter()
        .take(MATCHED)
        .filter(|row| row.item.id.is_empty())
        .map(|row| Entry::from(&row.item))
        .chain(
            data.tracks
                .iter()
                .take(MATCHED)
                .filter(|row| row.item.id.is_none())
                .map(|row| Entry::from(&row.item)),
        )
        .chain(
            data.albums
                .iter()
                .take(MATCHED)
                .filter(|row| row.item.id.is_empty())
                .map(|row| Entry::from(&row.item)),
        )
        .collect();
    let known = &known;
    let found: Vec<_> = futures::stream::iter(entries)
        .map(|entry| async move {
            if let Some(found) = known.get(&entry) {
                return (entry, Ok(found.clone()), false);
            }
            let found = entry.find(client).await;
            (entry, found, true)
        })
        .buffer_unordered(CONCURRENT)
        .collect()
        .await;
    let mut fresh = HashMap::new();
    for (entry, found, searched) in found {
        match found {
            Ok(found) => {
                if let Some(found) = &found {
                    entry.adopt(found, &mut data);
                }
                if searched {
                    fresh.insert(entry, found);
                }
            }
            Err(error) => log::debug!("stats: cannot match {entry:?} to the catalogue: {error:#}"),
        }
    }
    enlarge_ranked(&mut data, client);
    let missing_ids: Vec<String> = data
        .tracks
        .iter()
        .filter_map(|row| row.item.id.as_ref())
        .filter(|id| !known_descriptors.contains_key(*id))
        .cloned()
        .collect();
    let fresh_descriptors = fetch_descriptors(client, missing_ids).await;
    known_descriptors.extend(fresh_descriptors.clone());
    apply_scrobbler_descriptors(&mut data, &known_descriptors);
    (data, fresh, fresh_descriptors)
}

/// Weights matched track metadata by scrobble count.
fn apply_scrobbler_descriptors(
    data: &mut ListeningStats,
    descriptors: &HashMap<String, Descriptors>,
) {
    let mut breakdown = TrackBreakdown::default();
    let mut totals = TrackTotals::default();
    for row in &data.tracks {
        let weight = row.count.unwrap_or(1).max(1);
        if !row.item.artist_refs.is_empty() {
            totals.add(&row.item, weight);
        }
        breakdown.add(
            &row.item,
            row.item.id.as_ref().and_then(|id| descriptors.get(id)),
            weight,
        );
    }
    breakdown.apply(data);
    data.overview.collab_share = data.overview.collab_share.or_else(|| totals.collab_share());
}

/// Gives the ranked songs and albums large covers.
fn enlarge_ranked(data: &mut ListeningStats, client: &dyn MusicApi) {
    let covers = data
        .tracks
        .iter_mut()
        .map(|row| &mut row.item.cover)
        .chain(data.albums.iter_mut().map(|row| &mut row.item.cover));
    for cover in covers {
        if let Some(large) = cover
            .as_deref()
            .and_then(|url| client.cover_at(url, CoverSize::Large))
        {
            *cover = Some(large);
        }
    }
}

/// Asks the provider for the genres and moods of songs, which it may not describe.
async fn fetch_descriptors(
    client: &dyn MusicApi,
    ids: Vec<String>,
) -> HashMap<String, Descriptors> {
    if ids.is_empty() {
        return HashMap::new();
    }
    client.descriptors(&ids).await.unwrap_or_else(|error| {
        log::debug!("stats: cannot load descriptors: {error:#}");
        HashMap::new()
    })
}

/// Fetches artist portraits in batches, all at once.
async fn fetch_portraits(client: &dyn MusicApi, ids: Vec<String>) -> HashMap<String, String> {
    let batches = ids
        .chunks(PORTRAIT_BATCH)
        .map(|batch| client.artist_images(batch.to_vec()));
    futures::future::join_all(batches)
        .await
        .into_iter()
        .filter_map(|found| {
            found
                .map_err(|error| log::debug!("stats: cannot load portraits: {error:#}"))
                .ok()
        })
        .flatten()
        .collect()
}

/// Reuses album artwork and resolves each large cover once per log.
fn prepare_covers(tracks: &mut [Track], client: Option<&dyn MusicApi>) {
    let covers: HashMap<_, _> = tracks
        .iter()
        .filter_map(|track| Some((album_key(track)?, track.cover.clone()?)))
        .collect();
    let mut large: HashMap<String, String> = HashMap::new();
    for track in tracks.iter_mut() {
        track.cover = track
            .cover
            .take()
            .or_else(|| covers.get(&album_key(track)?).cloned());
        if let (Some(cover), Some(client)) = (track.cover.as_mut(), client) {
            let url = large.entry(cover.clone()).or_insert_with(|| {
                client
                    .cover_at(cover, CoverSize::Large)
                    .unwrap_or_else(|| cover.clone())
            });
            *cover = url.clone();
        }
    }
}
