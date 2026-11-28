use anyhow::{Context as _, Result};
use gpui::{Context, Entity, Task};
use logic::recap::{self, Play, Year};
use music::ArtistRef;
use rusqlite::params;
use storage::Database;

use crate::history::store_scope;
use crate::{Io, Session, SessionEvent, join};

/// Whether the recap has figures to show, is working them out, or could not read the play log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecapState {
    Loading,
    Ready,
    Failed,
}

#[derive(Clone)]
struct Store {
    database: Database,
}

impl Store {
    fn new(database: Database) -> Self {
        Self { database }
    }

    fn open(&self) -> Result<rusqlite::Connection> {
        self.database.open().context("cannot open the yearly recap")
    }

    /// Every stored play of one account, newest first. The history keeps at most a few thousand,
    /// so the whole log is read in one go and the year is picked off it without another query.
    fn load(&self, scope: &str) -> Result<Vec<Play>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(
                "SELECT track_id, name, artists, artist_refs, album, album_id, cover,
                        duration_ms, played_at, genres
                 FROM plays
                 WHERE scope = ?
                 ORDER BY played_at DESC",
            )
            .context("cannot prepare the yearly recap read")?;
        let rows = query
            .query_map(params![scope], |row| {
                let refs = row.get_ref(3)?.as_str()?;
                let artist_refs: Vec<ArtistRef> = serde_json::from_str(refs).unwrap_or_default();
                let genres = row.get_ref(9)?.as_str()?;
                let genres: Vec<String> = serde_json::from_str(genres).unwrap_or_default();
                let duration_ms: i64 = row.get(7)?;
                Ok(Play {
                    track_id: row.get(0)?,
                    name: row.get(1)?,
                    artists: row.get(2)?,
                    artist_refs,
                    album: row.get(4)?,
                    album_id: row.get(5)?,
                    cover: row.get(6)?,
                    duration_ms: duration_ms.max(0) as u64,
                    played_at: row.get(8)?,
                    genres,
                })
            })
            .context("cannot read the yearly recap")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot decode the yearly recap")
    }
}

/// The yearly listening recap. It reads the plays Sonora already keeps for the signed-in
/// account, works the selected year out off the UI thread, and holds the result until another
/// year is picked or the log changes.
pub struct Recap {
    session: Entity<Session>,
    store: Store,
    io: Io,
    state: RecapState,
    years: Vec<i32>,
    selected: Option<i32>,
    report: Option<Year>,
    plays: Vec<Play>,
    /// Whether a screen has asked for the recap, so a sign-in reloads it rather than fetching it
    /// for someone who never opened it.
    wanted: bool,
    task: Option<Task<()>>,
    error: Option<String>,
}

impl Recap {
    pub fn new(
        session: Entity<Session>,
        database: Database,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn | SessionEvent::Reconnected | SessionEvent::LocalChanged => {
                if this.wanted {
                    this.refresh(cx);
                }
            }
            SessionEvent::SignedOut => this.reset(cx),
            SessionEvent::WentOffline => {}
        })
        .detach();

        Self {
            session,
            store: Store::new(database),
            io,
            state: RecapState::Ready,
            years: Vec::new(),
            selected: None,
            report: None,
            plays: Vec::new(),
            wanted: false,
            task: None,
            error: None,
        }
    }

    pub fn state(&self) -> RecapState {
        self.state
    }

    /// Every year with a recorded play, oldest first.
    pub fn years(&self) -> &[i32] {
        &self.years
    }

    pub fn selected(&self) -> Option<i32> {
        self.selected
    }

    pub fn report(&self) -> Option<&Year> {
        self.report.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the play log again and keeps the selected year when it still has plays, so opening
    /// the recap or refreshing it never drops the listener back to another year.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.wanted = true;
        let Some(scope) = store_scope(self.session.read(cx)) else {
            self.reset(cx);
            return;
        };

        let store = self.store.clone();
        let io = self.io.clone();
        let keep = self.selected;
        self.state = RecapState::Loading;
        self.error = None;
        cx.notify();

        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move {
                tokio::task::spawn_blocking(move || {
                    let plays = store.load(&scope)?;
                    let years = recap::years(&plays);
                    let selected = keep
                        .filter(|year| years.contains(year))
                        .or_else(|| years.last().copied());
                    let report = selected.map(|year| recap::year(&plays, year));
                    Ok((plays, years, selected, report))
                })
                .await?
            }))
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match loaded {
                    Ok((plays, years, selected, report)) => {
                        this.apply(plays, years, selected, report);
                        this.state = RecapState::Ready;
                    }
                    Err(error) => {
                        log::warn!("recap: cannot load plays: {error:#}");
                        this.state = RecapState::Failed;
                        this.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Shows another year. It does nothing for a year with no plays, so the picker can only
    /// offer the years the log holds.
    pub fn select(&mut self, year: i32, cx: &mut Context<Self>) {
        if self.selected == Some(year) && self.report.is_some() {
            return;
        }
        if !self.years.contains(&year) {
            return;
        }

        self.selected = Some(year);
        self.state = RecapState::Loading;
        self.error = None;
        cx.notify();

        let plays = self.plays.clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let report = io.spawn_blocking(move || recap::year(&plays, year)).await;
            this.update(cx, |this, cx| {
                this.task = None;
                match report {
                    Ok(report) => {
                        this.report = Some(report);
                        this.state = RecapState::Ready;
                    }
                    Err(error) => {
                        log::warn!("recap: cannot work out {year}: {error}");
                        this.state = RecapState::Failed;
                        this.error = Some(format!("{error}"));
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Keeps a log already worked out off the UI thread and the year it was folded for.
    fn apply(
        &mut self,
        plays: Vec<Play>,
        years: Vec<i32>,
        selected: Option<i32>,
        report: Option<Year>,
    ) {
        self.plays = plays;
        self.years = years;
        self.selected = selected;
        self.report = report;
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.years.clear();
        self.selected = None;
        self.report = None;
        self.plays.clear();
        self.state = RecapState::Ready;
        self.error = None;
        cx.notify();
    }
}
