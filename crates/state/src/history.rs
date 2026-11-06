use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use gpui::{App, Context, Entity, Task};
use music::{ArtistRef, Track};
use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};
use storage::Database;

use crate::playback::PlaybackEvent;
use crate::{Io, Outcome, Playback, Session, SessionEvent, SessionState, Toast, Toasts, join};

const LOCAL_LIMIT: usize = 500;
/// Every column of `plays`, in the order a [`Row`] holds them.
const COLUMNS: &str = "scope, provider, track_id, played_at, name, playable, artists, \
                       artist_refs, album, album_id, cover, duration_ms, explicit";
const COLUMN_COUNT: usize = 13;

/// One `plays` row as stored, column by column, so a removed play goes back exactly as it was.
type Row = Vec<Value>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryState {
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

    fn open(&self) -> Result<Connection> {
        self.database
            .open()
            .context("cannot open listening history")
    }

    fn save(&self, scope: &str, provider: &str, track: &Track, played_at: i64) -> Result<()> {
        let Some(track_id) = track.id.as_deref() else {
            return Ok(());
        };
        let artist_refs = serde_json::to_string(&track.artist_refs)
            .context("cannot encode listening history artists")?;
        self.open()?
            .execute(
                "INSERT OR IGNORE INTO plays (
                    scope, provider, track_id, played_at, name, playable, artists,
                    artist_refs, album, album_id, cover, duration_ms, explicit
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    scope,
                    provider,
                    track_id,
                    played_at,
                    track.name,
                    track.playable,
                    track.artists,
                    artist_refs,
                    track.album,
                    track.album_id,
                    track.cover,
                    track.duration.as_millis().min(i64::MAX as u128) as i64,
                    track.explicit,
                ],
            )
            .context("cannot save listening history")?;
        self.prune(scope)
    }

    fn prune(&self, scope: &str) -> Result<()> {
        self.open()?
            .execute(
                "DELETE FROM plays
                 WHERE scope = ?1
                   AND rowid NOT IN (
                       SELECT rowid FROM plays
                       WHERE scope = ?1
                       ORDER BY played_at DESC
                       LIMIT ?2
                   )",
                params![scope, LOCAL_LIMIT],
            )
            .context("cannot trim listening history")?;
        Ok(())
    }

    /// Runs one edit and says how it went.
    fn apply(&self, edit: Edit) -> Applied {
        match edit {
            Edit::Remove { scope, track } => {
                let rows = self.take(&scope, &track);
                Applied::Removed { scope, track, rows }
            }
            Edit::Restore(rows) => Applied::Restored(self.put(&rows)),
            Edit::Wipe(scope) => Applied::Wiped(self.wipe(&scope)),
        }
    }

    /// Deletes the plays of `track` in the second the list shows for it, and returns the rows
    /// it deleted.
    fn take(&self, scope: &str, track: &Track) -> Result<Vec<Row>> {
        let (Some(track_id), Some(played_at)) = (track.id.as_deref(), track.added_at) else {
            return Ok(Vec::new());
        };
        let played_at = played_at.saturating_mul(1_000);
        let window = params![scope, track_id, played_at, played_at.saturating_add(1_000)];
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .context("cannot start removing a play")?;
        let rows = {
            let mut query = transaction
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM plays
                     WHERE scope = ? AND track_id = ? AND played_at >= ? AND played_at < ?"
                ))
                .context("cannot prepare a play read")?;
            query
                .query_map(window, |row| {
                    (0..COLUMN_COUNT)
                        .map(|column| row.get::<_, Value>(column))
                        .collect::<rusqlite::Result<Row>>()
                })
                .context("cannot read a play")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("cannot decode a play")?
        };
        transaction
            .execute(
                "DELETE FROM plays
                 WHERE scope = ? AND track_id = ? AND played_at >= ? AND played_at < ?",
                window,
            )
            .context("cannot remove a play")?;
        transaction.commit().context("cannot remove a play")?;
        Ok(rows)
    }

    /// Writes back rows that [`Self::take`] returned. A row already there is kept as it is.
    fn put(&self, rows: &[Row]) -> Result<()> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .context("cannot start restoring plays")?;
        let insert = format!(
            "INSERT OR IGNORE INTO plays ({COLUMNS}) VALUES ({})",
            vec!["?"; COLUMN_COUNT].join(", ")
        );
        for row in rows {
            transaction
                .execute(&insert, params_from_iter(row))
                .context("cannot restore a play")?;
        }
        transaction.commit().context("cannot restore plays")?;
        Ok(())
    }

    fn wipe(&self, scope: &str) -> Result<()> {
        self.open()?
            .execute("DELETE FROM plays WHERE scope = ?", params![scope])
            .context("cannot clear listening history")?;
        Ok(())
    }

    fn load(&self, scope: &str) -> Result<Vec<Track>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(
                "SELECT track_id, played_at, name, playable, artists, artist_refs, album,
                        album_id, cover, duration_ms, explicit
                 FROM plays
                 WHERE scope = ?
                 ORDER BY played_at DESC
                 LIMIT ?",
            )
            .context("cannot prepare listening history read")?;
        let rows = query
            .query_map(params![scope, LOCAL_LIMIT], |row| {
                let refs = row.get_ref(5)?.as_str()?;
                let artist_refs: Vec<ArtistRef> = serde_json::from_str(refs).unwrap_or_default();
                let played_at: i64 = row.get(1)?;
                let duration: i64 = row.get(9)?;
                Ok(Track {
                    id: Some(row.get(0)?),
                    name: row.get(2)?,
                    playable: row.get(3)?,
                    artists: row.get(4)?,
                    artist_refs,
                    album: row.get(6)?,
                    album_id: row.get(7)?,
                    cover: row.get(8)?,
                    duration: std::time::Duration::from_millis(duration.max(0) as u64),
                    added_at: Some(played_at / 1_000),
                    explicit: row.get(10)?,
                    ..Default::default()
                })
            })
            .context("cannot read listening history")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot decode listening history")
    }
}

/// A write to the play log. Edits run in batches, one batch at a time and in the order they
/// were made, so an undo never lands before the removal it reverses.
enum Edit {
    /// Deletes one play and keeps its rows so it can be put back.
    Remove {
        scope: String,
        track: Box<Track>,
    },
    Restore(Vec<Row>),
    Wipe(String),
}

/// How an [`Edit`] went.
enum Applied {
    Removed {
        scope: String,
        track: Box<Track>,
        rows: Result<Vec<Row>>,
    },
    Restored(Result<()>),
    Wiped(Result<()>),
}

/// A play removed while the Removed toast is up, kept so its Undo can put the play back.
struct Removal {
    scope: String,
    track: Box<Track>,
    rows: Vec<Row>,
}

pub struct History {
    session: Entity<Session>,
    playback: Entity<Playback>,
    store: Store,
    io: Io,
    state: HistoryState,
    tracks: Vec<Track>,
    pending: Vec<Track>,
    active: Option<(String, String)>,
    task: Option<Task<()>>,
    /// Edits waiting for the batch under way to finish.
    edits: Vec<Edit>,
    writing: Option<Task<()>>,
    /// Plays removed since the Removed toast went up, which its Undo puts back.
    removals: Vec<Removal>,
    /// The Removed toast, while it may still be up.
    removal_toast: Option<usize>,
    /// Bumped whenever the list is reset, so a removal that lands afterwards offers no undo
    /// and is not put back into a list that was cleared.
    era: u64,
}

impl History {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        database: Database,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn | SessionEvent::Reconnected => this.refresh(cx),
            SessionEvent::SignedOut => this.reset(cx),
            SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        })
        .detach();
        cx.subscribe(&playback, |this, _, event, cx| match event {
            PlaybackEvent::StartedPlayback => this.record(cx),
            PlaybackEvent::EndedPlayback => this.active = None,
            PlaybackEvent::Paused | PlaybackEvent::Seeked => {}
        })
        .detach();

        let mut history = Self {
            session,
            playback,
            store: Store::new(database),
            io,
            state: HistoryState::Ready,
            tracks: Vec::new(),
            pending: Vec::new(),
            active: None,
            task: None,
            edits: Vec::new(),
            writing: None,
            removals: Vec::new(),
            removal_toast: None,
            era: 0,
        };
        history.refresh(cx);
        history
    }

    pub fn state(&self) -> &HistoryState {
        &self.state
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Removes one play and offers to put it back. A play no longer in the list, such as one
    /// removed a moment ago, is left alone.
    pub fn remove(&mut self, track: &Track, cx: &mut Context<Self>) {
        if track.id.is_none() || track.added_at.is_none() {
            return;
        }
        let Some(scope) = self.scope(cx) else {
            return;
        };
        let same = |held: &Track| held.id == track.id && held.added_at == track.added_at;
        let Some(at) = self.tracks.iter().position(same) else {
            return;
        };
        let track = self.tracks.remove(at);
        self.pending.retain(|held| !same(held));
        cx.notify();
        self.write(
            Edit::Remove {
                scope,
                track: Box::new(track),
            },
            cx,
        );
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.scope(cx) else {
            return;
        };
        self.reset(cx);
        self.write(Edit::Wipe(scope), cx);
    }

    /// Queues an edit, and starts a batch unless one is under way.
    fn write(&mut self, edit: Edit, cx: &mut Context<Self>) {
        self.edits.push(edit);
        if self.writing.is_none() {
            self.flush(cx);
        }
    }

    /// Runs every queued edit as one batch, then the edits queued meanwhile.
    fn flush(&mut self, cx: &mut Context<Self>) {
        let edits = std::mem::take(&mut self.edits);
        if edits.is_empty() {
            self.writing = None;
            return;
        }
        let store = self.store.clone();
        let io = self.io.clone();
        let era = self.era;
        self.writing = Some(cx.spawn(async move |this, cx| {
            let applied = io
                .spawn_blocking(move || {
                    edits
                        .into_iter()
                        .map(|edit| store.apply(edit))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                match applied {
                    Ok(applied) => this.applied(applied, era, cx),
                    Err(error) => log::warn!("history: write task failed: {error}"),
                }
                this.flush(cx);
            })
            .ok();
        }));
    }

    /// Takes in a finished batch. Removals offer an undo, and a removal that failed puts its
    /// play back, unless the list was reset since the batch started.
    fn applied(&mut self, applied: Vec<Applied>, era: u64, cx: &mut Context<Self>) {
        let current = era == self.era;
        let mut removed = Vec::new();
        for done in applied {
            match done {
                Applied::Removed {
                    scope,
                    track,
                    rows: Ok(rows),
                } => {
                    if current {
                        removed.push(Removal { scope, track, rows });
                    }
                }
                Applied::Removed {
                    scope,
                    track,
                    rows: Err(error),
                } => {
                    log::warn!("history: cannot remove a play: {error:#}");
                    if current && self.scope(cx).as_deref() == Some(scope.as_str()) {
                        self.put_back(*track);
                        cx.notify();
                    }
                    failed("toast-history-remove-failed", &error, cx);
                }
                Applied::Restored(Err(error)) => {
                    log::warn!("history: cannot restore plays: {error:#}");
                    failed("toast-history-restore-failed", &error, cx);
                    self.refresh(cx);
                }
                Applied::Wiped(Err(error)) => {
                    log::warn!("history: cannot clear plays: {error:#}");
                    failed("toast-history-clear-failed", &error, cx);
                    self.refresh(cx);
                }
                Applied::Restored(Ok(())) | Applied::Wiped(Ok(())) => {}
            }
        }
        if !removed.is_empty() {
            self.offer_undo(removed, cx);
        }
    }

    /// Shows the Removed toast, or keeps the one already up and restarts its clock, so one
    /// Undo puts back everything removed while it was up.
    fn offer_undo(&mut self, removed: Vec<Removal>, cx: &mut Context<Self>) {
        let toasts = Toasts::entity(cx);
        let up = self
            .removal_toast
            .filter(|id| toasts.read(cx).shown().iter().any(|toast| toast.id == *id));
        if up.is_none() {
            self.removals.clear();
        }
        self.removals.extend(removed);

        let history = cx.entity().downgrade();
        let toast = Toast::new(Outcome::Plain, "toast-history-removed")
            .count(self.removals.len())
            .undo(move |cx| {
                history.update(cx, |history, cx| history.undo(cx)).ok();
            });
        match up {
            Some(id) => Toasts::settle(id, toast, cx),
            None => self.removal_toast = Some(Toasts::post(toast, cx)),
        }
    }

    /// Puts back every play the Removed toast reports. Plays of an account no longer signed in
    /// go back into its log without touching the list on screen.
    fn undo(&mut self, cx: &mut Context<Self>) {
        self.removal_toast = None;
        let scope = self.scope(cx);
        let mut rows = Vec::new();
        for removal in std::mem::take(&mut self.removals) {
            if scope.as_deref() == Some(removal.scope.as_str()) {
                self.put_back(*removal.track);
            }
            rows.extend(removal.rows);
        }
        cx.notify();
        if !rows.is_empty() {
            self.write(Edit::Restore(rows), cx);
        }
    }

    /// Puts a play back into the list where its time places it. A play the list already holds
    /// is left alone, and one put back during a refresh survives it.
    fn put_back(&mut self, track: Track) {
        if self
            .tracks
            .iter()
            .any(|held| held.id == track.id && held.added_at == track.added_at)
        {
            return;
        }
        let at = self
            .tracks
            .iter()
            .position(|held| held.added_at < track.added_at)
            .unwrap_or(self.tracks.len());
        if self.task.is_some() {
            self.pending.push(track.clone());
        }
        self.tracks.insert(at, track);
        self.state = HistoryState::Ready;
    }

    /// Drops the list and loads it again, so the page shows its skeleton until the plays land.
    /// A load already under way is left to finish and fills the list in its place.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.tracks.clear();
        self.state = HistoryState::Loading;
        cx.notify();
        self.refresh(cx);
    }

    /// Loads the plays again and keeps the list on screen until they land, which is how the
    /// history picks up plays from other devices in the background.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(scope) = self.scope(cx) else {
            return self.reset(cx);
        };
        let store = self.store.clone();
        let io = self.io.clone();
        // The provider's own cross-device recently played folds in here.
        let client = self.session.read(cx).client();
        // A refresh with something to show must not blank the list back to a skeleton.
        if self.tracks.is_empty() {
            self.state = HistoryState::Loading;
            cx.notify();
        }

        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move {
                tokio::task::spawn_blocking(move || store.load(&scope)).await?
            }))
            .await;
            let recent = match client {
                Some(client) => join(io.spawn(async move { client.recently_played().await })).await,
                None => Ok(Vec::new()),
            };
            this.update(cx, |this, cx| {
                this.task = None;
                match loaded {
                    Ok(tracks) => {
                        let mut plays = std::mem::take(&mut this.pending);
                        match plays.is_empty() {
                            true => plays = tracks,
                            false => plays.extend(tracks),
                        }
                        dedupe(&mut plays);
                        this.tracks = merge_recent(recent, plays, LOCAL_LIMIT);
                        this.state = HistoryState::Ready;
                    }
                    Err(error) => {
                        log::warn!("history: cannot load plays: {error:#}");
                        this.state = HistoryState::Failed;
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn record(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.scope(cx) else {
            return;
        };
        let Some(mut track) = self.playback.read(cx).track().cloned() else {
            return;
        };
        let Some(track_id) = track.id.clone() else {
            return;
        };
        let Some(provider) = self.session.read(cx).slug_for(&track_id) else {
            return;
        };
        let key = (provider.to_owned(), track_id.clone());
        if self.active.as_ref() == Some(&key) {
            return;
        }
        self.active = Some(key);

        let played_at = now();
        track.added_at = Some(played_at / 1_000);
        self.tracks.insert(0, track.clone());
        if self.task.is_some() {
            self.pending.push(track.clone());
        }
        self.tracks.truncate(LOCAL_LIMIT);
        self.state = HistoryState::Ready;
        cx.notify();

        self.report(&track_id, cx);

        let store = self.store.clone();
        let provider = provider.to_owned();
        self.io.spawn(async move {
            let saved = tokio::task::spawn_blocking(move || {
                store.save(&scope, &provider, &track, played_at)
            })
            .await;
            match saved {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("history: cannot save a play: {error:#}"),
                Err(error) => log::warn!("history: save task failed: {error}"),
            }
        });
    }

    /// Reports the play to the provider that owns the track, fire-and-forget. A failure is
    /// logged and dropped, never touching what the listener hears.
    fn report(&self, track_id: &str, cx: &Context<Self>) {
        if music::is_local_id(track_id) {
            return;
        }
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        let track_id = track_id.to_owned();
        self.io.spawn(async move {
            if let Err(error) = client.report_play(&track_id).await {
                log::warn!("history: cannot report the play to the provider: {error:#}");
            }
        });
    }

    fn scope(&self, cx: &Context<Self>) -> Option<String> {
        let session = self.session.read(cx);
        let SessionState::SignedIn(profile) = session.state() else {
            return None;
        };
        let provider = session.provider_slug()?;
        Some(format!("{provider}:{}", profile.id))
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.active = None;
        self.tracks.clear();
        self.pending.clear();
        self.removals.clear();
        self.era += 1;
        if let Some(id) = self.removal_toast.take() {
            Toasts::close(id, cx);
        }
        self.state = HistoryState::Ready;
        cx.notify();
    }
}

/// Shows that a change to the history failed, with the error below it.
fn failed(key: &'static str, error: &anyhow::Error, cx: &mut App) {
    Toasts::with_detail(Outcome::Failed, key, None, format!("{error:#}"), cx);
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

/// Folds the provider's cross-device recently played into the local play log. The timestamped
/// local log leads, so whatever just played here stays on top. Played-elsewhere tracks fold in
/// after, deduped, in the provider's newest-first order. A failed fetch folds in nothing, and
/// no more than `limit` plays come back.
fn merge_recent(recent: Result<Vec<Track>>, mut local: Vec<Track>, limit: usize) -> Vec<Track> {
    let mut recent = recent.unwrap_or_else(|error| {
        log::warn!("history: cannot load the provider's recently played: {error:#}");
        Vec::new()
    });
    local.truncate(limit);
    let fresh: Vec<bool> = {
        let mut seen: HashSet<&str> = local
            .iter()
            .filter_map(|track| track.id.as_deref())
            .collect();
        recent
            .iter()
            .map(|track| track.id.as_deref().is_some_and(|id| seen.insert(id)))
            .collect()
    };
    let mut fresh = fresh.into_iter();
    recent.retain(|_| fresh.next().unwrap_or(false));
    recent.truncate(limit - local.len());
    local.reserve_exact(recent.len());
    local.extend(recent);
    local
}

/// Drops every play after the first of the same track at the same time, keeping the order.
fn dedupe(plays: &mut Vec<Track>) {
    let first: Vec<bool> = {
        let mut seen = HashSet::new();
        plays
            .iter()
            .map(|track| seen.insert((track.id.as_deref(), track.added_at)))
            .collect()
    };
    let mut first = first.into_iter();
    plays.retain(|_| first.next().unwrap_or(true));
}
