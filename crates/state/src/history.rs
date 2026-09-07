use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use gpui::{Context, Entity, Task};
use music::{ArtistRef, Track};
use rusqlite::{Connection, params};
use storage::Database;

use crate::playback::PlaybackEvent;
use crate::{Io, Playback, Session, SessionEvent, SessionState, join};

const LOCAL_LIMIT: usize = 500;

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

    fn delete(&self, scope: &str, track_id: &str, played_at: i64) -> Result<()> {
        self.open()?
            .execute(
                "DELETE FROM plays
                 WHERE scope = ? AND track_id = ? AND played_at >= ? AND played_at < ?",
                params![scope, track_id, played_at, played_at + 1_000],
            )
            .context("cannot remove a play")?;
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
                let refs: String = row.get(5)?;
                let artist_refs: Vec<ArtistRef> = serde_json::from_str(&refs).unwrap_or_default();
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
                    added_by: None,
                    playcount: None,
                    popularity: 0,
                    explicit: row.get(10)?,
                    track_number: 0,
                    disc_number: 0,
                    tags: Vec::new(),
                    languages: Vec::new(),
                    credits: Vec::new(),
                })
            })
            .context("cannot read listening history")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot decode listening history")
    }
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
            SessionEvent::LocalChanged => {}
        })
        .detach();
        cx.subscribe(&playback, |this, _, event, cx| match event {
            PlaybackEvent::StartedPlayback => this.record(cx),
            PlaybackEvent::EndedPlayback => this.active = None,
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

    pub fn remove(&mut self, track: &Track, cx: &mut Context<Self>) {
        let (Some(track_id), Some(played_at)) = (track.id.clone(), track.added_at) else {
            return;
        };
        let Some(scope) = self.scope(cx) else {
            return;
        };
        self.tracks
            .retain(|held| held.id != track.id || held.added_at != track.added_at);
        self.pending
            .retain(|held| held.id != track.id || held.added_at != track.added_at);
        cx.notify();

        let store = self.store.clone();
        let played_at = played_at.saturating_mul(1_000);
        self.io.spawn(async move {
            let removed =
                tokio::task::spawn_blocking(move || store.delete(&scope, &track_id, played_at))
                    .await;
            match removed {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("history: cannot remove a play: {error:#}"),
                Err(error) => log::warn!("history: remove task failed: {error}"),
            }
        });
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.scope(cx) else {
            return;
        };
        self.reset(cx);

        let store = self.store.clone();
        self.io.spawn(async move {
            let cleared = tokio::task::spawn_blocking(move || store.wipe(&scope)).await;
            match cleared {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("history: cannot clear plays: {error:#}"),
                Err(error) => log::warn!("history: clear task failed: {error}"),
            }
        });
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(scope) = self.scope(cx) else {
            return self.reset(cx);
        };
        let store = self.store.clone();
        let io = self.io.clone();
        self.state = HistoryState::Loading;
        cx.notify();

        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move {
                tokio::task::spawn_blocking(move || store.load(&scope)).await?
            }))
            .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match loaded {
                    Ok(tracks) => {
                        let mut pending = std::mem::take(&mut this.pending);
                        pending.extend(tracks);
                        let mut seen = HashSet::new();
                        pending.retain(|track| seen.insert((track.id.clone(), track.added_at)));
                        this.tracks = pending;
                        this.tracks.truncate(LOCAL_LIMIT);
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
        let key = (provider.to_owned(), track_id);
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
        self.state = HistoryState::Ready;
        cx.notify();
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
