use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use gpui::{App, Context, Entity, Task};
use music::{SavedArtist, Track};
use rusqlite::{Connection, params};
use storage::Database;

use crate::{Io, Session, join};

/// What a block covers: one song, or every song an artist is credited on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Song,
    Artist,
}

/// One blocked song or artist, as the settings list shows it. The names are kept with it, so
/// the list needs no network call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocked {
    /// The slug of the provider the id belongs to.
    pub provider: String,
    pub kind: BlockKind,
    /// The track id or the artist id.
    pub id: String,
    pub name: String,
    /// The song's artists as a track row shows them, and empty for an artist.
    pub artists: String,
    /// When it was blocked, in milliseconds.
    pub added_at: i64,
}

impl Blocked {
    fn is(&self, provider: &str, kind: BlockKind, id: &str) -> bool {
        self.kind == kind && self.id == id && self.provider == provider
    }
}

/// The ids blocked on the providers signed in now. It is cheap to clone and can be sent, so a
/// radio fetch can filter on tokio. One flat set is enough because at most one streaming
/// provider is up at a time and local ids carry a prefix of their own.
#[derive(Clone, Default)]
pub(crate) struct Blocklist {
    songs: Arc<HashSet<String>>,
    artists: Arc<HashSet<String>>,
}

impl Blocklist {
    fn of(all: &[Blocked], slugs: &[&str]) -> Self {
        let live = all
            .iter()
            .filter(|entry| slugs.contains(&entry.provider.as_str()));
        let mut songs = HashSet::new();
        let mut artists = HashSet::new();
        for entry in live {
            match entry.kind {
                BlockKind::Song => songs.insert(entry.id.clone()),
                BlockKind::Artist => artists.insert(entry.id.clone()),
            };
        }
        Self {
            songs: Arc::new(songs),
            artists: Arc::new(artists),
        }
    }

    /// Whether `track` is blocked itself or credits a blocked artist.
    pub(crate) fn holds(&self, track: &Track) -> bool {
        track
            .id
            .as_deref()
            .is_some_and(|id| self.songs.contains(id))
            || track
                .artist_refs
                .iter()
                .filter_map(|artist| artist.id.as_deref())
                .any(|id| self.artists.contains(id))
    }
}

/// A write to the blocklist. Edits run in batches, one batch at a time and in the order they
/// were made, so an undo never lands before the block it reverses.
enum Edit {
    Keep(Blocked),
    Forget {
        provider: String,
        kind: BlockKind,
        id: String,
    },
}

/// The songs and artists the listener blocked, kept on this machine whatever the provider. A
/// blocked song still plays when it is picked, but the queue, radio and shuffle pass over it,
/// and over every song by a blocked artist. The blocks of a provider that is signed out stay on
/// disk and apply again at its next sign-in.
pub struct Blocks {
    session: Entity<Session>,
    store: Store,
    io: Io,
    /// Every block of every provider, newest first.
    all: Vec<Blocked>,
    /// The ids of `all` that apply to `slugs`.
    live: Blocklist,
    /// The providers `live` was built for.
    slugs: Vec<&'static str>,
    /// Reads the list from disk. Edits made meanwhile wait for it, so the read never misses
    /// them.
    load: Option<Task<()>>,
    /// Edits waiting for the batch under way, or the read, to finish.
    edits: Vec<Edit>,
    writing: Option<Task<()>>,
}

impl Blocks {
    pub fn new(
        session: Entity<Session>,
        database: Database,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |this, _, cx| this.relive(cx)).detach();

        let slugs = session.read(cx).active_slugs();
        let mut blocks = Self {
            session,
            store: Store { database },
            io,
            all: Vec::new(),
            live: Blocklist::default(),
            slugs,
            load: None,
            edits: Vec::new(),
            writing: None,
        };
        blocks.restore(cx);
        blocks
    }

    /// Whether `track` is passed over, for itself or for one of its artists.
    pub fn blocks(&self, track: &Track) -> bool {
        self.live.holds(track)
    }

    /// Whether the song `id` is blocked on its own, whatever its artists.
    pub fn song_blocked(&self, id: &str) -> bool {
        self.live.songs.contains(id)
    }

    pub fn artist_blocked(&self, id: &str) -> bool {
        self.live.artists.contains(id)
    }

    /// The blocks of the providers signed in now, newest first.
    pub fn entries(&self, cx: &App) -> impl Iterator<Item = &Blocked> {
        let slugs = self.session.read(cx).active_slugs();
        self.all
            .iter()
            .filter(move |entry| slugs.contains(&entry.provider.as_str()))
    }

    /// Blocks `track`, or unblocks it when it is blocked on its own already. A track with no id
    /// is left alone.
    pub fn toggle_song(&mut self, track: &Track, cx: &mut Context<Self>) {
        let Some(id) = track.id.clone() else {
            return;
        };
        match self.song_blocked(&id) {
            true => self.unblock_id(BlockKind::Song, id, cx),
            false => self.block(
                BlockKind::Song,
                id,
                track.name.clone(),
                track.artists.clone(),
                cx,
            ),
        }
    }

    /// Blocks every song by `artist`, or unblocks them when the artist is blocked already.
    pub fn toggle_artist(&mut self, artist: &SavedArtist, cx: &mut Context<Self>) {
        let id = artist.id.clone();
        match self.artist_blocked(&id) {
            true => self.unblock_id(BlockKind::Artist, id, cx),
            false => self.block(
                BlockKind::Artist,
                id,
                artist.name.clone(),
                String::new(),
                cx,
            ),
        }
    }

    /// Lifts one block, as the settings list does.
    pub fn unblock(&mut self, entry: &Blocked, cx: &mut Context<Self>) {
        self.forget(entry.provider.clone(), entry.kind, entry.id.clone(), cx);
    }

    fn block(
        &mut self,
        kind: BlockKind,
        id: String,
        name: String,
        artists: String,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.session.read(cx).slug_for(&id) else {
            return;
        };
        let entry = Blocked {
            provider: provider.to_owned(),
            kind,
            id,
            name,
            artists,
            added_at: now(),
        };
        self.keep(entry, cx);
    }

    fn unblock_id(&mut self, kind: BlockKind, id: String, cx: &mut Context<Self>) {
        let Some(provider) = self.session.read(cx).slug_for(&id) else {
            return;
        };
        self.forget(provider.to_owned(), kind, id, cx);
    }

    fn keep(&mut self, entry: Blocked, cx: &mut Context<Self>) {
        self.all
            .retain(|held| !held.is(&entry.provider, entry.kind, &entry.id));
        self.all.insert(0, entry.clone());
        self.rebuild();
        cx.notify();
        self.write(Edit::Keep(entry), cx);
    }

    fn forget(&mut self, provider: String, kind: BlockKind, id: String, cx: &mut Context<Self>) {
        let before = self.all.len();
        self.all.retain(|held| !held.is(&provider, kind, &id));
        if self.all.len() == before {
            return;
        }
        self.rebuild();
        cx.notify();
        self.write(Edit::Forget { provider, kind, id }, cx);
    }

    /// Follows the session, so a provider that signs in or out brings its blocks into play or
    /// takes them out.
    fn relive(&mut self, cx: &mut Context<Self>) {
        let slugs = self.session.read(cx).active_slugs();
        if slugs == self.slugs {
            return;
        }
        self.slugs = slugs;
        self.rebuild();
        cx.notify();
    }

    fn rebuild(&mut self) {
        self.live = Blocklist::of(&self.all, &self.slugs);
    }

    /// Reads the list from disk once, off the main thread.
    fn restore(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let request = self.io.spawn_blocking(move || store.load());
        self.load = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;
            this.update(cx, |this, cx| {
                this.load = None;
                match loaded {
                    Ok(rows) => this.adopt(rows),
                    Err(error) => log::warn!("blocks: cannot read the blocklist: {error:#}"),
                }
                this.rebuild();
                cx.notify();
                this.flush(cx);
            })
            .ok();
        }));
    }

    /// Takes in the rows read from disk, with the edits made while they were read laid over
    /// them, since those reach the disk only afterwards.
    fn adopt(&mut self, mut rows: Vec<Blocked>) {
        for edit in &self.edits {
            match edit {
                Edit::Keep(entry) => {
                    rows.retain(|held| !held.is(&entry.provider, entry.kind, &entry.id));
                    rows.insert(0, entry.clone());
                }
                Edit::Forget { provider, kind, id } => {
                    rows.retain(|held| !held.is(provider, *kind, id));
                }
            }
        }
        self.all = rows;
    }

    /// Queues an edit, and starts a batch unless one, or the read, is under way.
    fn write(&mut self, edit: Edit, cx: &mut Context<Self>) {
        self.edits.push(edit);
        if self.writing.is_none() && self.load.is_none() {
            self.flush(cx);
        }
    }

    /// Runs every queued edit as one batch, then the edits queued meanwhile. The list in
    /// memory is already right, so a failed write is only logged.
    fn flush(&mut self, cx: &mut Context<Self>) {
        let edits = std::mem::take(&mut self.edits);
        if edits.is_empty() {
            self.writing = None;
            return;
        }
        let store = self.store.clone();
        let request = self.io.spawn_blocking(move || {
            for edit in edits {
                if let Err(error) = store.apply(edit) {
                    log::warn!("blocks: cannot save the blocklist: {error:#}");
                }
            }
        });
        self.writing = Some(cx.spawn(async move |this, cx| {
            if let Err(error) = request.await {
                log::warn!("blocks: write task failed: {error}");
            }
            this.update(cx, |this, cx| this.flush(cx)).ok();
        }));
    }
}

/// Whether `track` is blocked, for code with no handle on the [`Blocks`] entity. Nothing is
/// before the app state is up.
pub fn blocked(track: &Track, cx: &App) -> bool {
    cx.try_global::<crate::Sonora>()
        .is_some_and(|sonora| sonora.blocks.read(cx).blocks(track))
}

/// The blocklist as it stands, for code with no handle on the [`Blocks`] entity, such as a
/// radio fetch about to leave for tokio. It is empty before the app state is up.
pub(crate) fn blocklist(cx: &App) -> Blocklist {
    cx.try_global::<crate::Sonora>()
        .map(|sonora| sonora.blocks.read(cx).live.clone())
        .unwrap_or_default()
}

#[derive(Clone)]
struct Store {
    database: Database,
}

impl Store {
    fn open(&self) -> Result<Connection> {
        self.database.open().context("cannot open the blocklist")
    }

    fn load(&self) -> Result<Vec<Blocked>> {
        let connection = self.open()?;
        let mut query = connection
            .prepare(
                "SELECT provider, kind, item_id, name, artists, added_at
                 FROM blocked
                 ORDER BY added_at DESC",
            )
            .context("cannot prepare a blocklist read")?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .context("cannot read the blocklist")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("cannot decode the blocklist")?;
        Ok(rows
            .into_iter()
            .filter_map(|(provider, kind, id, name, artists, added_at)| {
                Some(Blocked {
                    provider,
                    kind: kind_of(&kind)?,
                    id,
                    name,
                    artists,
                    added_at,
                })
            })
            .collect())
    }

    fn apply(&self, edit: Edit) -> Result<()> {
        match edit {
            Edit::Keep(entry) => self.keep(&entry),
            Edit::Forget { provider, kind, id } => self.forget(&provider, kind, &id),
        }
    }

    fn keep(&self, entry: &Blocked) -> Result<()> {
        self.open()?
            .execute(
                "INSERT OR REPLACE INTO blocked (provider, kind, item_id, name, artists, added_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    entry.provider,
                    kind_name(entry.kind),
                    entry.id,
                    entry.name,
                    entry.artists,
                    entry.added_at,
                ],
            )
            .context("cannot save a block")?;
        Ok(())
    }

    fn forget(&self, provider: &str, kind: BlockKind, id: &str) -> Result<()> {
        self.open()?
            .execute(
                "DELETE FROM blocked WHERE provider = ? AND kind = ? AND item_id = ?",
                params![provider, kind_name(kind), id],
            )
            .context("cannot remove a block")?;
        Ok(())
    }
}

fn kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Song => "song",
        BlockKind::Artist => "artist",
    }
}

fn kind_of(name: &str) -> Option<BlockKind> {
    match name {
        "song" => Some(BlockKind::Song),
        "artist" => Some(BlockKind::Artist),
        _ => None,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
