use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use futures::StreamExt as _;
use gpui::{App, Context, Entity, Task};
use music::scrobble::{self, Service};
use music::{
    Album, Board, CoverSize, Descriptors, ListeningStats, Milestone, MusicApi, Overview, Ranked,
    SavedArtist, StatsPeriod, Track, album_key, album_of, compare, credits, milestone_numbers,
    tag_key,
};
use storage::Database;

use crate::history::Store;
use crate::{AppSettings, History, Io, Session, SessionEvent, join};

/// How many ranked artists, songs and albums of a scrobbler chart are matched to the catalogue up
/// front. Further entries are matched when opened.
const MATCHED: usize = 10;
/// How many recorded artists get a portrait, and how many are requested at once.
const PORTRAITS: usize = 100;
const PORTRAIT_BATCH: usize = 25;
/// How many catalogue requests run at once.
const CONCURRENT: usize = 6;
/// How long the dates sit still before a scrobbler is asked about them.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// How long recorded plays wait for the dates to settle, which is only a pass over the log.
const SETTLE: Duration = Duration::from_millis(120);
/// How long a load counts as current when the page is opened again.
const FRESH: Duration = Duration::from_secs(300);
/// How many of the most played recorded tracks are asked for genres and moods, and how many at once.
const DESCRIBED: usize = 300;
/// How many slices the recorded history is cut into for the timeline's activity shape.
const DENSITY_BUCKETS: usize = 64;

/// Where the rankings come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatsSource {
    /// The signed-in provider's own rankings, over its fixed windows.
    Provider,
    /// The timestamped plays Sonora recorded, over any range of dates.
    Recorded,
    /// A linked scrobbling service, by its id.
    Scrobbler(&'static str),
}

/// A scrobbler chart entry that names something in the active catalogue without an id of its own.
#[derive(Clone, Debug)]
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

impl Entry {
    /// What names the entry, for remembering what the catalogue made of it.
    fn key(&self) -> String {
        match self {
            Self::Artist { name } => format!("artist\u{1}{name}"),
            Self::Track { title, artist } => format!("track\u{1}{title}\u{1}{artist}"),
            Self::Album { title, artist } => format!("album\u{1}{title}\u{1}{artist}"),
        }
    }

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

    /// Swaps every unmatched ranking row this entry names for the catalogue item, keeping the
    /// chart's artwork when the catalogue has none.
    fn adopt(&self, found: &Found, data: &mut ListeningStats) {
        match (self, found) {
            (Self::Artist { name }, Found::Artist(found)) => {
                let rows = data.artists.iter_mut();
                for row in rows.filter(|row| row.item.id.is_empty() && same(&row.item.name, name)) {
                    let cover = found.cover.clone().or(row.item.cover.take());
                    row.item = SavedArtist {
                        cover,
                        ..found.clone()
                    };
                }
            }
            (Self::Track { title, artist }, Found::Track(found)) => {
                let rows = data.tracks.iter_mut();
                for row in rows.filter(|row| {
                    row.item.id.is_none() && named(&row.item.name, &row.item.artists, title, artist)
                }) {
                    let cover = found.cover.clone().or(row.item.cover.take());
                    row.item = Track {
                        cover,
                        ..found.clone()
                    };
                }
            }
            (Self::Album { title, artist }, Found::Album(found)) => {
                let rows = data.albums.iter_mut();
                for row in rows.filter(|row| {
                    row.item.id.is_empty()
                        && named(&row.item.name, &row.item.artists, title, artist)
                }) {
                    let chart = row.item.cover.take();
                    row.item = Album {
                        cover: found.cover.clone().or(chart.clone()),
                        cover_large: found.cover_large.clone().or(chart),
                        ..found.clone()
                    };
                }
            }
            _ => {}
        }
    }
}

/// The retained play log with everything derived from it, which changing the dates leaves be.
#[derive(Default)]
struct Log {
    /// The retained play log, kept so that changing the dates needs no new read.
    recorded: Vec<Track>,
    /// Genres and moods the provider gave the most played recorded tracks.
    descriptors: HashMap<String, Descriptors>,
    portraits: HashMap<String, String>,
    /// When each artist was first played in the retained log, which a date range never changes.
    first_play: Firsts,
    /// The figures of every recorded play, which a date range never changes.
    history: Overview,
    /// Plays per slice of the scrobbler account's history, which a date range never changes.
    scrobbled_density: Vec<usize>,
    /// The scrobbler account's round-number plays, read once with the all-time view.
    scrobbled_milestones: Vec<Milestone>,
    /// The recorded plays that reached a round number, which a date range never changes.
    milestones: Vec<Milestone>,
    /// The first and last recorded play, in Unix seconds.
    bounds: Option<(i64, i64)>,
    /// The selected range of recorded plays, in Unix seconds, or all of them.
    dates: Option<(i64, i64)>,
    /// Recorded plays per slice of `bounds`.
    density: Vec<usize>,
}

/// Owns stats requests, the retained play log and date filtering, independently of any view.
pub struct Stats {
    session: Entity<Session>,
    history: Entity<History>,
    settings: Entity<AppSettings>,
    store: Store,
    source: StatsSource,
    period: StatsPeriod,
    log: Log,
    /// What was last loaded and when, so that opening the page again need not load it again.
    loaded: Option<(StatsSource, StatsPeriod, Instant)>,
    /// When the scrobbler account began, in Unix seconds.
    since: Option<i64>,
    data: ListeningStats,
    loading: bool,
    failed: bool,
    task: Option<Task<()>>,
    /// The wait for dragged dates to settle, kept apart from `task` so that it never cancels a load.
    settling: Option<Task<()>>,
    matching: Option<Task<()>>,
    opening: Option<Task<()>>,
    /// The read of what only a whole scrobbler account says, which runs beside the page.
    history_task: Option<Task<()>>,
    /// What searching the catalogue found for chart entries, misses included, so that a new range
    /// asks only about the entries it has not met. It belongs to the provider named beside it.
    matched: HashMap<String, Option<Found>>,
    matched_for: Option<String>,
    /// The latest answer for each window of a source, so that coming back to one is instant.
    windows: HashMap<(StatsSource, StatsPeriod), (Instant, ListeningStats)>,
}

impl Stats {
    pub fn new(
        session: Entity<Session>,
        history: Entity<History>,
        settings: Entity<AppSettings>,
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
                this.refresh(cx);
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
        cx.observe(&settings, |this, _, cx| {
            if matches!(this.source, StatsSource::Scrobbler(_)) {
                this.ensure(cx);
            }
        })
        .detach();
        let (source, period) = defaults(&session, cx);
        Self {
            session,
            history,
            settings,
            store: Store::new(database),
            source,
            period,
            log: Log::default(),
            loaded: None,
            since: None,
            data: ListeningStats::default(),
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

    /// The sources that can be shown right now, in tab order.
    pub fn sources(&self, cx: &App) -> Vec<StatsSource> {
        let mut sources = vec![StatsSource::Recorded];
        if !self.periods_of(StatsSource::Provider, cx).is_empty() {
            sources.push(StatsSource::Provider);
        }
        let accounts = self.settings.read(cx).scrobbling();
        sources.extend(
            scrobble::services()
                .into_iter()
                .filter(|service| {
                    service.stats_dated()
                        && accounts
                            .get(service.id())
                            .is_some_and(|account| account.linked())
                })
                .map(|service| StatsSource::Scrobbler(service.id())),
        );
        sources
    }

    /// The fixed windows of the selected source. Sources that read any dates have none.
    pub fn periods(&self, cx: &App) -> Vec<StatsPeriod> {
        self.periods_of(self.source, cx)
    }

    pub fn period(&self) -> StatsPeriod {
        self.period
    }

    pub fn data(&self) -> &ListeningStats {
        &self.data
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The stretch the dates can be picked from: the first to the last recorded play, or the
    /// scrobbler account's first day to now.
    pub fn bounds(&self) -> Option<(i64, i64)> {
        match self.source {
            StatsSource::Scrobbler(_) => self
                .since
                .map(|since| (since, jiff::Timestamp::now().as_second())),
            _ => self.log.bounds,
        }
    }

    /// The selected range of dates, or none for all time.
    pub fn dates(&self) -> Option<(i64, i64)> {
        self.log.dates
    }

    /// The figures of the whole recorded history, whatever dates are selected, where the source
    /// has every play. Only recorded plays do.
    pub fn history(&self) -> Option<&Overview> {
        (self.source == StatsSource::Recorded).then_some(&self.log.history)
    }

    /// How many plays fall in each equal slice of [`Self::bounds`].
    pub fn density(&self) -> &[usize] {
        match self.source {
            StatsSource::Recorded => &self.log.density,
            StatsSource::Scrobbler(_) => &self.log.scrobbled_density,
            StatsSource::Provider => &[],
        }
    }

    /// Switches source, moving to the new source's first window when it lacks `period`.
    pub fn select(&mut self, source: StatsSource, period: StatsPeriod, cx: &mut Context<Self>) {
        let periods = self.periods_of(source, cx);
        if source != self.source {
            self.log.dates = None;
        }
        self.settling = None;
        self.source = source;
        self.period = match periods.contains(&period) {
            true => period,
            false => periods.last().copied().unwrap_or(period),
        };
        let kept = self
            .windows
            .get(&(self.source, self.period))
            .filter(|(at, _)| at.elapsed() < FRESH && self.log.dates.is_none());
        if let (Some((at, data)), true) = (kept, self.source != StatsSource::Recorded) {
            self.task = None;
            self.matching = None;
            self.failed = false;
            self.loading = false;
            self.data = data.clone();
            self.loaded = Some((self.source, self.period, *at));
            cx.notify();
            return;
        }
        self.refresh(cx);
    }

    /// Restricts the rankings to an inclusive range of Unix seconds, or to none for all time.
    /// Recorded plays are regrouped at once, and a scrobbler is asked again once the range has
    /// stopped moving.
    pub fn set_dates(&mut self, dates: Option<(i64, i64)>, cx: &mut Context<Self>) {
        self.log.dates = dates;
        // A drag sends many dates, and each answer is a pass over the whole log or a request, so
        // the work waits for the dates to settle.
        let (wait, scrobbled) = match self.source {
            StatsSource::Scrobbler(_) => (DEBOUNCE, true),
            _ => (SETTLE, false),
        };
        self.settling = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                match scrobbled {
                    true => this.refresh(cx),
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
        let current = self.loaded.is_some_and(|(source, period, at)| {
            source == self.source && period == self.period && at.elapsed() < FRESH
        });
        if !(self.loading || current && !self.failed) {
            self.refresh(cx);
        }
    }

    /// Reloads whatever the selected source shows.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.windows.remove(&(self.source, self.period));
        self.loaded = None;
        self.task = None;
        self.matching = None;
        self.opening = None;
        self.failed = false;
        self.loading = false;
        self.data = ListeningStats::default();
        let client = self.session.read(cx).client();
        let ranks = client
            .as_ref()
            .is_some_and(|client| client.stats_periods().contains(&self.period));
        if self.source == StatsSource::Provider && !ranks {
            self.source = StatsSource::Recorded;
        }
        match (self.source, client) {
            (StatsSource::Provider, Some(client)) => self.load_provider(client, cx),
            (StatsSource::Scrobbler(id), client) => self.load_scrobbler(id, client, cx),
            _ => self.load_recorded(false, cx),
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
        self.opening = Some(cx.spawn(async move |this, cx| {
            let worker = entry.clone();
            let found = join(io.spawn(async move { worker.find(client.as_ref()).await }));
            let found = found.await.unwrap_or_else(|error| {
                log::debug!("stats: cannot match {entry:?} to the catalogue: {error:#}");
                None
            });
            this.update(cx, |this, cx| {
                this.opening = None;
                if let Some(found) = &found {
                    entry.adopt(found, &mut this.data);
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
                self.data = data;
                self.loaded = Some((self.source, self.period, Instant::now()));
            }
            Err(error) => {
                log::warn!("stats: cannot load {what}: {error:#}");
                self.failed = true;
            }
        }
        cx.notify();
    }

    /// Shows the rankings as soon as the provider answers. The portraits of credited artists, which
    /// it does not send, are looked up after, in batches that run together, and each card takes
    /// its picture as the batch holding it arrives.
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
            move |this, result, cx| {
                let missing: Vec<_> = result
                    .as_ref()
                    .map(|data| {
                        data.songwriters
                            .iter()
                            .chain(&data.producers)
                            .map(|ranked| ranked.item.id.clone())
                            .filter(|id| !this.log.portraits.contains_key(id))
                            .collect()
                    })
                    .unwrap_or_default();
                let result = result.map(|mut data| {
                    this.dress_credits(&mut data);
                    this.windows.insert(
                        (StatsSource::Provider, period),
                        (Instant::now(), data.clone()),
                    );
                    data
                });
                this.finish(result, "provider rankings", cx);
                if missing.is_empty() {
                    return;
                }
                let batches: Vec<_> = missing
                    .chunks(PORTRAIT_BATCH)
                    .map(|batch| {
                        let (client, ids) = (looked_up.clone(), batch.to_vec());
                        join(io.spawn(async move { client.artist_images(ids).await }))
                    })
                    .collect();
                this.matching = Some(cx.spawn(async move |this, cx| {
                    // Each batch is a request of its own, and the cards take what each brings as
                    // it arrives.
                    let mut pending: futures::stream::FuturesUnordered<_> =
                        batches.into_iter().collect();
                    while let Some(found) = pending.next().await {
                        let Ok(found) = found else { continue };
                        this.update(cx, |this, cx| {
                            this.log.portraits.extend(found);
                            let mut data = std::mem::take(&mut this.data);
                            this.dress_credits(&mut data);
                            this.data = data;
                            let key = (StatsSource::Provider, period);
                            if let Some((at, mut kept)) = this.windows.remove(&key) {
                                this.dress_credits(&mut kept);
                                this.windows.insert(key, (at, kept));
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

    /// Gives the credited artists the portraits that are known.
    fn dress_credits(&self, data: &mut ListeningStats) {
        for ranked in data.songwriters.iter_mut().chain(&mut data.producers) {
            if let Some(cover) = self.log.portraits.get(&ranked.item.id) {
                ranked.item.cover = Some(cover.clone());
            }
        }
    }

    /// Shows the chart as soon as the service answers, then matches its top entries to the active
    /// catalogue in the background so they open in Sonora.
    fn load_scrobbler(
        &mut self,
        id: &'static str,
        client: Option<Arc<dyn MusicApi>>,
        cx: &mut Context<Self>,
    ) {
        let account = self.settings.read(cx).account(id);
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
        let period = self.period;
        let history = (self.log.scrobbled_density.is_empty()
            && self.log.scrobbled_milestones.is_empty())
        .then(|| (service.clone(), account.clone()));
        // The history of the whole account is read beside the rankings, not after them.
        if let Some((service, account)) = history {
            self.history_task = Some(io.load(
                async move { service.history(&account).await },
                |this, result, cx| {
                    this.history_task = None;
                    match result {
                        Ok(history) => {
                            this.log.scrobbled_density = history.density;
                            this.log.scrobbled_milestones = history.milestones.clone();
                            this.data.milestones = history.milestones;
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
            move |this, mut result, cx| {
                if let Ok(data) = &mut result {
                    // The milestones and the history belong to the whole account, so a range
                    // keeps the ones read before it.
                    data.milestones = this.log.scrobbled_milestones.clone();
                    this.since = data.overview.since.or(data.lifetime.since).or(this.since);
                }
                if let (Ok(data), Some(client)) = (&result, client) {
                    let data = data.clone();
                    this.matching = Some(io.clone().load(
                        async move { Ok(match_catalogue(data, client.as_ref(), known).await) },
                        |this, result, cx| {
                            this.matching = None;
                            match result {
                                Ok((mut data, found)) => {
                                    this.matched.extend(found);
                                    data.milestones = this.log.scrobbled_milestones.clone();
                                    this.data = data;
                                }
                                Err(error) => log::debug!("stats: cannot match chart: {error:#}"),
                            }
                            cx.notify();
                        },
                        cx,
                    ));
                }
                if let (Ok(data), None) = (&result, dates) {
                    this.windows.insert(
                        (StatsSource::Scrobbler(id), period),
                        (Instant::now(), data.clone()),
                    );
                }
                this.finish(result, "scrobbler rankings", cx);
            },
            cx,
        ));
    }

    /// Reads the play log and shows it at once. Portraits and genres follow. A
    /// quiet read leaves the page alone, and changes it only when the log has changed.
    fn load_recorded(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let Some(scope) = self.history.read(cx).scope(cx) else {
            return;
        };
        let store = self.store.clone();
        let io = Io::global(cx);
        let dates = self.log.dates;
        let client = self.session.read(cx).client();
        // What the log held when last read, so that a quiet read of an unchanged log stops short.
        let known = (
            self.log.recorded.len(),
            self.log.recorded.first().map(|track| track.added_at),
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
                    let first_play = first_plays(&tracks);
                    let data = aggregate(&tracks, &first_play, dates, &HashMap::new());
                    let milestones = milestones_of(&tracks);
                    let times: Vec<_> = tracks.iter().filter_map(|track| track.added_at).collect();
                    let history = Overview::default().timed(&times);
                    Ok(Some((tracks, first_play, data, milestones, history)))
                })
                .await?
            },
            move |this, result, cx| match result {
                Ok(None) => this.task = None,
                Ok(Some((tracks, first_play, data, milestones, history))) => {
                    let times = tracks.iter().filter_map(|track| track.added_at);
                    this.log.bounds = times.clone().min().zip(times.max());
                    this.log.density = this
                        .log
                        .bounds
                        .map(|bounds| slice_plays(&tracks, bounds))
                        .unwrap_or_default();
                    this.log.recorded = tracks;
                    this.log.first_play = first_play;
                    this.log.milestones = milestones;
                    this.log.history = history;
                    this.finish(Ok(data), "recorded plays", cx);
                    this.enrich(cx);
                }
                Err(error) => this.finish(Err(error), "recorded plays", cx),
            },
            cx,
        ));
    }

    /// Fetches, all at once, the genres and moods of the most played recorded songs and the
    /// artists' portraits. The page is already on screen, and the answers are applied together.
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
                    this.data = this.aggregated();
                    cx.notify();
                }
            },
            cx,
        ));
    }

    /// The recorded plays inside the selected dates, with portraits applied.
    fn aggregated(&self) -> ListeningStats {
        let mut data = aggregate(
            &self.log.recorded,
            &self.log.first_play,
            self.log.dates,
            &self.log.descriptors,
        );
        if let Some((from, to)) = self.log.dates {
            let length = to - from + 1;
            let range = Some((from - length, from - 1));
            let before = aggregate(
                &self.log.recorded,
                &self.log.first_play,
                range,
                &self.log.descriptors,
            );
            compare_with(&mut data, &before);
        }
        data.milestones = self.log.milestones.clone();
        for artist in &mut data.artists {
            if let Some(cover) = self.log.portraits.get(&artist.item.id) {
                artist.item.cover = Some(cover.clone());
            }
        }
        data
    }
}

/// The source and window a fresh account starts on: its provider's broadest window, else recorded
/// plays.
fn defaults(session: &Entity<Session>, cx: &App) -> (StatsSource, StatsPeriod) {
    let first = session
        .read(cx)
        .client()
        .and_then(|client| client.stats_periods().last().copied());
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

/// Matches the top entries of a scrobbler chart to the catalogue, a few searches at a time, then
/// gives the matches large covers. Entries already met come from `known`, and what the searches
/// found is handed back to be kept.
async fn match_catalogue(
    mut data: ListeningStats,
    client: &dyn MusicApi,
    known: HashMap<String, Option<Found>>,
) -> (ListeningStats, HashMap<String, Option<Found>>) {
    let top = |len: usize| len.min(MATCHED);
    let entries: Vec<Entry> = data.artists[..top(data.artists.len())]
        .iter()
        .filter(|row| row.item.id.is_empty())
        .map(|row| Entry::Artist {
            name: row.item.name.clone(),
        })
        .chain(
            data.tracks[..top(data.tracks.len())]
                .iter()
                .filter(|row| row.item.id.is_none())
                .map(|row| Entry::Track {
                    title: row.item.name.clone(),
                    artist: row.item.artists.clone(),
                }),
        )
        .chain(
            data.albums[..top(data.albums.len())]
                .iter()
                .filter(|row| row.item.id.is_empty())
                .map(|row| Entry::Album {
                    title: row.item.name.clone(),
                    artist: row.item.artists.clone(),
                }),
        )
        .collect();
    let known = &known;
    let found: Vec<_> = futures::stream::iter(entries)
        .map(|entry| async move {
            let key = entry.key();
            match known.get(&key) {
                Some(found) => (entry, key, Ok(found.clone()), false),
                None => {
                    let found = entry.find(client).await;
                    (entry, key, found, true)
                }
            }
        })
        .buffer_unordered(CONCURRENT)
        .collect()
        .await;
    let mut fresh = HashMap::new();
    for (entry, key, found, searched) in found {
        match found {
            Ok(found) => {
                if let Some(found) = &found {
                    entry.adopt(found, &mut data);
                }
                if searched {
                    fresh.insert(key, found);
                }
            }
            Err(error) => log::debug!("stats: cannot match {entry:?} to the catalogue: {error:#}"),
        }
    }
    enlarge_ranked(&mut data, client);
    (data, fresh)
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

/// Gives a song with no cover the cover of another play of its album, and asks the provider for
/// large covers once, so that every later regrouping of the plays has them.
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
                    .unwrap_or(cover.clone())
            });
            *cover = url.clone();
        }
    }
}

/// Counts the plays that started within `dates`. Every credited artist, genre and mood of a play
/// gets one hit. A track the provider described takes its genres and moods from that, and any
/// other from the tags it was recorded with.
fn aggregate(
    tracks: &[Track],
    first_play: &Firsts,
    dates: Option<(i64, i64)>,
    descriptors: &HashMap<String, Descriptors>,
) -> ListeningStats {
    let (mut songs, mut artists, mut albums) =
        (Board::default(), Board::default(), Board::default());
    let (mut genres, mut moods) = (Board::default(), Board::default());
    let (mut time, mut times) = (0u64, Vec::new());
    let mut new_artists = HashSet::new();
    let (mut plays, mut new_tracks) = (0, 0);
    for track in tracks {
        let (Some(at), Some(id)) = (track.added_at, track.id.as_ref()) else {
            continue;
        };
        if dates.is_some_and(|(from, to)| at < from || at > to) {
            continue;
        }
        plays += 1;
        new_tracks += usize::from(first_play.tracks.get(id) == Some(&at));
        time += track.duration.as_secs();
        times.push(at);
        songs.add(id.clone(), 1, || track.clone());
        if let Some((key, album)) = album_of(track) {
            albums.add(key, 1, || album);
        }
        for (key, artist) in credits(track) {
            if first_play.artists.get(&key) == Some(&at) {
                new_artists.insert(key.clone());
            }
            artists.add(key, 1, || artist);
        }
        let (own_genres, own_moods) = match descriptors.get(id) {
            Some(found) => (&found.genres, &found.moods),
            None => (&track.tags, &Vec::new()),
        };
        for (board, names) in [(&mut genres, own_genres), (&mut moods, own_moods)] {
            let mut seen = HashSet::new();
            for name in names {
                if let Some(key) = tag_key(name).filter(|key| seen.insert(key.clone())) {
                    board.add(key, 1, || name.trim().to_owned());
                }
            }
        }
    }

    let overview = match plays {
        0 => Overview::default(),
        _ => Overview {
            plays: Some(plays),
            time: Some(time),
            tracks: Some(songs.len()),
            artists: Some(artists.len()),
            albums: Some(albums.len()),
            new_artists: Some(new_artists.len()),
            new_tracks: Some(new_tracks),
            replay_share: Some((plays.saturating_sub(songs.len()) * 100 / plays) as u8),
            avg_length: Some(time / plays as u64),
            ..Default::default()
        }
        .timed(&times),
    };
    ListeningStats {
        tracks: songs.ranked(),
        artists: artists.ranked(),
        albums: albums.ranked(),
        genres: genres.ranked(),
        moods: moods.ranked(),
        overview,
        ..Default::default()
    }
}

/// When each artist and each song was first played in the log.
#[derive(Default)]
struct Firsts {
    artists: HashMap<String, i64>,
    tracks: HashMap<String, i64>,
}

/// The recorded plays that reached a round number, oldest first.
fn milestones_of(tracks: &[Track]) -> Vec<Milestone> {
    let mut plays: Vec<_> = tracks
        .iter()
        .filter_map(|track| Some((track.added_at?, track)))
        .collect();
    plays.sort_by_key(|(at, _)| *at);
    milestone_numbers(plays.len())
        .into_iter()
        .filter_map(|number| {
            let (at, track) = plays.get(number - 1)?;
            Some(Milestone {
                number,
                title: track.name.clone(),
                artist: track.artists.clone(),
                at: *at,
            })
        })
        .collect()
}

fn first_plays(tracks: &[Track]) -> Firsts {
    let mut firsts = Firsts::default();
    let earliest = |map: &mut HashMap<String, i64>, key: String, at: i64| {
        map.entry(key)
            .and_modify(|first| *first = (*first).min(at))
            .or_insert(at);
    };
    for track in tracks {
        let Some(at) = track.added_at else { continue };
        for (key, _) in credits(track) {
            earliest(&mut firsts.artists, key, at);
        }
        if let Some(id) = &track.id {
            earliest(&mut firsts.tracks, id.clone(), at);
        }
    }
    firsts
}

/// Marks each ranked item as new, up or down against the period before, and sets how plays
/// changed. Nothing is marked when there was no period before to compare with.
fn compare_with(data: &mut ListeningStats, before: &ListeningStats) {
    let Some(was) = before.overview.plays.filter(|was| *was > 0) else {
        return;
    };
    let named = |id: &str, name: &str| if id.is_empty() { name } else { id }.to_owned();
    let keys = |rows: &[Ranked<SavedArtist>]| -> Vec<String> {
        rows.iter()
            .map(|row| named(&row.item.id, &row.item.name))
            .collect()
    };
    compare(&mut data.artists, &keys(&before.artists), |artist| {
        named(&artist.id, &artist.name)
    });
    let songs: Vec<_> = before
        .tracks
        .iter()
        .map(|row| row.item.id.clone().unwrap_or_default())
        .collect();
    compare(&mut data.tracks, &songs, |track| {
        track.id.clone().unwrap_or_default()
    });
    let albums: Vec<_> = before
        .albums
        .iter()
        .map(|row| named(&row.item.id, &row.item.name))
        .collect();
    compare(&mut data.albums, &albums, |album| {
        named(&album.id, &album.name)
    });
    let now = data.overview.plays.unwrap_or_default();
    data.overview.change = Some(((now as i64 - was as i64) * 100 / was as i64) as i32);
}

/// Counts plays in equal slices of `bounds`.
fn slice_plays(tracks: &[Track], (first, last): (i64, i64)) -> Vec<usize> {
    let span = (last - first).max(1) as f64;
    let mut slices = vec![0; DENSITY_BUCKETS];
    for at in tracks.iter().filter_map(|track| track.added_at) {
        let share = (at - first) as f64 / span;
        slices[((share * DENSITY_BUCKETS as f64) as usize).min(DENSITY_BUCKETS - 1)] += 1;
    }
    slices
}
