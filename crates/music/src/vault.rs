//! Tracks kept on disk, so the next play needs no download: the ones that played, in a cache
//! with a size limit, and the ones the listener downloaded, which stay until they are removed
//! and play with no network at all.
//!
//! A kept track is never a file another program can play. Each one is sealed with
//! ChaCha20-Poly1305 under a key derived from a master key, so the folder read without that key
//! is noise. File names are keyed hashes, so the folder does not say which tracks are in it
//! either. The master key lives in the OS keychain, or on a desktop with no keychain in a key
//! file readable by the owner alone in the data folder the downloads share. There, copying the
//! whole data folder carries the key along with the downloads.
//!
//! What goes in is what the provider sent, before its [`Body`](crate::stream::Body) touched it.
//! A Deezer track stays striped with Blowfish inside the vault, so its decrypted audio still
//! never reaches the disk. An Apple Music track stays under Widevine and is licensed again on
//! every play, so peeling off the vault's layer leaves nothing easier to play than the stream
//! and nothing that plays once Apple stops handing out the key.
//!
//! Spotify is the exception. librespot keeps its own played files, as the CDN sent them and
//! still under the key Spotify hands out on every play, in a folder inside the vault that
//! [`Vault::foreign`] names. A Spotify download is sealed in the vault with that key beside it,
//! so peeling off the vault's layer leaves all it takes to play the file.
//!
//! The owner of the machine can always read the master key out of their keychain or key file,
//! or the audio out of the process. The vault keeps kept tracks from playing outside Sonora for
//! anyone without the master key, and nothing more.
//!
//! A provider whose bytes carry no protection of their own, such as the listener's own Subsonic
//! server, keeps its downloads as plain audio files instead, named after the track and playable
//! in any other player. [`Shelf::plain`] turns that on. A plain download says which file holds
//! it in a small index file under [`INDEX`], so it is found without the key. The cache is sealed
//! whatever the provider.
//!
//! An entry is the file's random salt, then the track in sealed chunks of [`CHUNK`] bytes, then
//! the provider's facts about the track sealed on their own, then the length of that last block.
//! Every chunk's nonce carries its index and whether it is the last, so a truncated, reordered or
//! swapped entry fails to open rather than playing wrong.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use bytes::Bytes;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom as _, SystemRandom};
use ring::{digest, hkdf, hmac};
use tokio::sync::watch;

use crate::stream::Source;

/// How much of a track one sealed chunk holds. Big enough that the tags cost nothing, small
/// enough that a damaged chunk is found long before the decoder needs it.
const CHUNK: usize = 64 * 1024;
/// The bytes of a ChaCha20-Poly1305 tag.
const TAG: usize = 16;
/// The bytes of the random salt every entry starts with.
const SALT: usize = 32;
/// The bytes of the master key.
pub const MASTER: usize = 32;
/// Mixed into every key the vault derives, so a change of layout cannot read an older one.
const VERSION: &[u8] = b"sonora vault 1";
/// What one track may take of the whole vault at most, as a fraction of the limit.
const SHARE: u64 = 4;
/// The folder inside the vault that players keeping their own files share.
const FOREIGN: &str = "players";
/// How long a half-written entry left by a crash stays before a sweep removes it.
const ABANDONED: Duration = Duration::from_secs(24 * 60 * 60);
/// The hidden folder beside a provider's plain downloads holding one index file per track,
/// which names the audio file and carries the provider's facts.
const INDEX: &str = ".index";
/// The most bytes of a plain download's file name before its extension.
const NAME_BYTES: usize = 120;
/// How much of a plain download's start is held to tell its format by.
const HEAD: usize = 64;
/// How long a lookup made before the vault has its key waits for it. Reading the keychain takes
/// milliseconds, so this only runs out while the keychain is prompting or stuck.
const UNLOCK_WAIT: Duration = Duration::from_secs(5);

/// Tells apart the temporary files of two recordings running at once in one process.
static WRITES: AtomicU64 = AtomicU64::new(0);
/// Whether keeping played tracks is on, for a player that keeps its own files and is set up
/// before any engine starts. Spotify reads it when its session opens.
static KEEPING: AtomicBool = AtomicBool::new(false);
/// The most the standard cache may hold, in bytes, for the same players.
static LIMIT: AtomicU64 = AtomicU64::new(0);
/// The players that keep their own files and are running, each with what empties its folder
/// through the player itself.
static PLAYERS: Mutex<Vec<(&'static str, Clear)>> = Mutex::new(Vec::new());

/// Answers the facts a provider keeps beside a track once the whole track has arrived, or
/// `None` when the provider cannot vouch that it has.
type Describe = Box<dyn FnOnce() -> Option<Vec<u8>> + Send>;
/// Empties a player's own folder through the player, or answers false once the player is gone.
type Clear = Box<dyn Fn() -> bool + Send + Sync>;

/// What a sealed block is, which its nonce carries so one cannot stand in for another.
#[derive(Clone, Copy)]
enum Block {
    Chunk = 0,
    Last = 1,
    Facts = 2,
}

/// Every kept track on this machine, under one master key.
///
/// Tracks live in one of two places. The cache holds what has played, within one size limit,
/// and gives up the least recently played tracks to stay under it. Downloads are tracks the
/// listener asked to keep, in Sonora's data folder rather than its cache, and stay until they
/// are discarded.
///
/// The vault exists before its key does. Until [`unlock`](Self::unlock) or
/// [`stay_locked`](Self::stay_locked) runs, nothing is kept, and a lookup from an engine waits up
/// to [`UNLOCK_WAIT`] for the key rather than missing a track that is there.
pub struct Vault {
    root: PathBuf,
    /// Where downloads live.
    pinned: PathBuf,
    /// The most the cache may hold, in bytes.
    limit: AtomicU64,
    /// Whether tracks that only played are kept in the cache.
    caching: AtomicBool,
    keys: OnceLock<hkdf::Prk>,
    /// Turns true once the vault has its key or will not get one this session.
    settled: watch::Sender<bool>,
    /// What the cache takes on disk, once a sweep has counted it.
    used: Mutex<Option<u64>>,
    /// The tracks being downloaded, by shelf and id, with what to call a plain download's file.
    /// Whatever arrives for one of them goes to the downloads rather than the cache.
    wanted: Mutex<HashMap<(&'static str, String), Option<String>>>,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("root", &self.root)
            .field("pinned", &self.pinned)
            .field("limit", &self.limit())
            .field("unlocked", &self.keys.get().is_some())
            .finish()
    }
}

impl Vault {
    /// A vault that caches at most `limit` bytes in `root` and keeps downloads in `pinned`.
    pub fn new(root: PathBuf, pinned: PathBuf, limit: u64) -> Self {
        Self {
            root,
            pinned,
            limit: AtomicU64::new(limit),
            caching: AtomicBool::new(true),
            keys: OnceLock::new(),
            settled: watch::Sender::new(false),
            used: Mutex::new(None),
            wanted: Mutex::new(HashMap::new()),
        }
    }

    /// A vault in Sonora's folders: the cache in `$XDG_CACHE_HOME/sonora/vault` and downloads in
    /// `$XDG_DATA_HOME/sonora/downloads`.
    pub fn standard(limit: u64) -> Self {
        Self::new(standard_root(), pinned_root(), limit)
    }

    /// Tells a player that keeps its own files whether to keep them and how much the whole
    /// cache may hold. Its session reads this when it opens, so a change reaches it at the next
    /// sign-in.
    pub fn set_keeping(on: bool, limit: u64) {
        KEEPING.store(on, Ordering::Relaxed);
        LIMIT.store(limit, Ordering::Relaxed);
    }

    /// Changes how much the cache may hold. A lower limit evicts the least recently played
    /// tracks at once, which walks the cache on disk, so call it off the main thread.
    pub fn set_limit(&self, limit: u64) {
        let before = self.limit.swap(limit, Ordering::Relaxed);
        if limit < before {
            self.sweep();
        }
    }

    fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Whether tracks that only played are cached. Downloads are kept either way.
    pub fn set_caching(&self, on: bool) {
        self.caching.store(on, Ordering::Relaxed);
    }

    /// Where a player that keeps its own files keeps them when keeping is on, and how many bytes
    /// it may keep there. The folder is made private before the player can create it.
    ///
    /// The listener's limit covers the whole cache, the player's folder included. The player
    /// gets what Sonora's own entries leave of it, and at least half, so the oldest of those
    /// entries are evicted here when they take more. The share is fixed for the session, so a
    /// new limit reaches the player at its next sign-in.
    pub(crate) fn foreign(player: &str) -> Option<(PathBuf, u64)> {
        if !KEEPING.load(Ordering::Relaxed) {
            return None;
        }
        let root = standard_root();
        let folder = root.join(FOREIGN).join(player);
        if let Err(error) = create_private(&root)
            .and_then(|()| create_private(&root.join(FOREIGN)))
            .and_then(|()| create_private(&folder))
        {
            log::warn!("vault: {player} keeps no files: {error:#}");
            return None;
        }
        let limit = LIMIT.load(Ordering::Relaxed);
        let own = evict(&root, limit - limit / 2);
        Some((folder, limit.saturating_sub(own)))
    }

    /// Registers a running player that keeps its own files, with `clear` emptying its folder
    /// through the player so its own count of what it keeps stays true. `clear` answers false
    /// once the player is gone. A later registration for the same player replaces this one.
    pub(crate) fn adopt(player: &'static str, clear: impl Fn() -> bool + Send + Sync + 'static) {
        let mut players = PLAYERS.lock().unwrap_or_else(|held| held.into_inner());
        players.retain(|(name, _)| *name != player);
        players.push((player, Box::new(clear)));
    }

    /// A fresh random master key, for a machine whose keychain has none yet.
    pub fn new_master() -> Result<[u8; MASTER]> {
        let mut master = [0; MASTER];
        SystemRandom::new()
            .fill(&mut master)
            .map_err(|_| anyhow!("cannot draw a random master key"))?;
        Ok(master)
    }

    /// Lets the vault find and keep tracks under `master`. Only the first call counts.
    pub fn unlock(&self, master: &[u8]) {
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, VERSION).extract(master);
        if self.keys.set(prk).is_err() {
            log::debug!("vault: already unlocked");
        }
        self.settled.send_replace(true);
    }

    /// Gives up on the key for this session, so lookups stop waiting for it and miss at once.
    pub fn stay_locked(&self) {
        self.settled.send_replace(true);
    }

    /// Whether the vault has its key yet.
    pub fn unlocked(&self) -> bool {
        self.keys.get().is_some()
    }

    /// Whether the key is still being looked for, so which tracks are downloaded cannot be told
    /// yet.
    pub fn unlocking(&self) -> bool {
        !*self.settled.borrow()
    }

    /// The key, waiting up to [`UNLOCK_WAIT`] for it while it is still being looked for.
    async fn ready(&self) -> Option<&hkdf::Prk> {
        if let Some(prk) = self.keys.get() {
            return Some(prk);
        }
        let mut settled = self.settled.subscribe();
        if tokio::time::timeout(UNLOCK_WAIT, settled.wait_for(|done| *done))
            .await
            .is_err()
        {
            log::warn!("vault: still locked after {UNLOCK_WAIT:?}, looking past it");
        }
        self.keys.get()
    }

    /// The part of the vault one provider keeps its tracks in. `provider` names its folder and
    /// goes into every key, so no provider can open another's entries.
    pub(crate) fn shelf(self: &Arc<Self>, provider: &'static str) -> Shelf {
        Shelf {
            vault: self.clone(),
            provider,
            scope: String::new(),
            plain: false,
        }
    }

    /// Removes every cached track. Downloads stay. A running player's own files are removed
    /// through the player, which keeps its folder and its count of what it holds.
    pub fn forget(&self) {
        let players = PLAYERS.lock().unwrap_or_else(|held| held.into_inner());
        let mut failed = false;
        for item in fs::read_dir(&self.root).into_iter().flatten().flatten() {
            let path = item.path();
            let removed = match item.file_name() == FOREIGN {
                true => forget_players(&path, &players),
                false => remove(&path),
            };
            if let Err(error) = removed {
                failed = true;
                log::warn!("vault: cannot forget the cached tracks: {error}");
            }
        }
        drop(players);
        if !failed {
            log::info!("vault: forgot every cached track");
        }
        *self.used.lock().unwrap_or_else(|held| held.into_inner()) = None;
    }

    /// The folder `provider` keeps its downloads in, made readable by the owner alone when it
    /// does not exist yet, for opening it in a file manager.
    pub fn downloads_of(&self, provider: &str) -> Result<PathBuf> {
        let folder = self.pinned.join(provider);
        create_private(&self.pinned)?;
        create_private(&folder)?;
        Ok(folder)
    }

    /// What the cache and the downloads take on disk, in bytes, the cache counting the files
    /// players keep for themselves. It walks both folders, so call it off the main thread.
    pub fn usage(&self) -> (u64, u64) {
        (footprint(&self.root), footprint(&self.pinned))
    }

    /// Whether any sealed download is on disk, whatever key it was sealed under. Plain downloads
    /// need no key, so they do not count. It walks the downloads folder, so call it off the main
    /// thread.
    pub fn holds_downloads(&self) -> bool {
        fs::read_dir(&self.pinned)
            .into_iter()
            .flatten()
            .flatten()
            .any(|shelf| {
                fs::read_dir(shelf.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|item| sealed_name(&item.file_name().to_string_lossy()))
            })
    }

    /// Removes every download.
    pub fn forget_downloads(&self) {
        match fs::remove_dir_all(&self.pinned) {
            Ok(()) => log::info!("vault: removed every download"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("vault: cannot remove the downloads: {error}"),
        }
    }

    fn base(&self, place: Place) -> &Path {
        match place {
            Place::Cache => &self.root,
            Place::Pinned => &self.pinned,
        }
    }

    fn wanted(&self) -> std::sync::MutexGuard<'_, HashMap<(&'static str, String), Option<String>>> {
        self.wanted.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// Counts a new cache entry of `size` bytes and evicts the least recently played ones once
    /// the cache, players' own files included, holds more than its limit.
    fn settle(&self, size: u64) {
        let over = {
            let mut used = self.used.lock().unwrap_or_else(|held| held.into_inner());
            *used = used.map(|used| used.saturating_add(size));
            used.is_none_or(|used| {
                used.saturating_add(footprint(&self.root.join(FOREIGN))) > self.limit()
            })
        };
        if over {
            self.sweep();
        }
    }

    /// Drops what a crash left half-written in either place and the least recently played cache
    /// entries until they and the players' own files fit the limit. Only a player removes its
    /// own files, so those are counted and never evicted here.
    fn sweep(&self) {
        let limit = self.limit();
        abandoned(&self.pinned);
        let room = limit.saturating_sub(footprint(&self.root.join(FOREIGN)));
        let own = evict(&self.root, room);
        *self.used.lock().unwrap_or_else(|held| held.into_inner()) = Some(own);
    }
}

/// Where in the vault an entry lives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Cache,
    Pinned,
}

/// One provider's tracks in the vault.
#[derive(Clone)]
pub struct Shelf {
    vault: Arc<Vault>,
    provider: &'static str,
    /// What else tells two tracks with one id apart, such as the server a Subsonic id is from.
    scope: String,
    /// Whether downloads may be kept as plain files, see [`Shelf::plain`].
    plain: bool,
}

impl fmt::Debug for Shelf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shelf")
            .field("provider", &self.provider)
            .finish()
    }
}

impl Shelf {
    /// The same shelf for ids that are only unique within `scope`.
    pub(crate) fn within(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// The same shelf with downloads kept as plain audio files another player opens. Only for a
    /// provider whose bytes carry no protection of their own, since a plain download is exactly
    /// what the provider sent.
    pub(crate) fn plain(mut self) -> Self {
        self.plain = true;
        self
    }

    /// Whether downloads on this shelf are kept as plain files.
    pub(crate) fn keeps_plain(&self) -> bool {
        self.plain
    }

    /// The kept copy of track `id`, a download first and then a cached copy as `variant`
    /// describes it, when there is a whole one. An entry that fails to open is removed, so a
    /// damaged file costs one download and no more. A cached copy of a track being downloaded
    /// moves to the downloads on the way out.
    ///
    /// `variant` is whatever changes the bytes for the same id, such as the quality asked for.
    /// A download is found whatever the variant, since the listener asked for that copy. A
    /// lookup made while the vault is still unlocking waits for it, up to [`UNLOCK_WAIT`].
    pub(crate) async fn find(&self, id: &str, variant: &str) -> Option<Kept> {
        if let Some(kept) = self.find_plain(id) {
            return Some(kept);
        }
        let prk = self.vault.ready().await?;
        if let Some(kept) = self.find_in(prk, Place::Pinned, id, variant) {
            return Some(kept);
        }
        if !self.vault.caching.load(Ordering::Relaxed) {
            return None;
        }
        let kept = self.find_in(prk, Place::Cache, id, variant)?;
        if !self.is_wanted(id) {
            return Some(kept);
        }
        let shelf = self.clone();
        let owned = id.to_owned();
        let promoted = tokio::task::spawn_blocking(move || shelf.promote(&owned, kept))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|promoted| promoted);
        match promoted {
            Ok(kept) => Some(kept),
            Err(error) => {
                log::warn!("vault: cannot move a cached track to the downloads: {error:#}");
                self.find_in(prk, Place::Cache, id, variant)
            }
        }
    }

    /// Wraps `source` so that everything it yields is sealed into the vault as it passes, and
    /// the entry is kept once the source has yielded its last byte. `facts` answers whatever the
    /// provider needs to play the track again without asking the network, which comes back
    /// from [`Kept::facts`]. It runs when the entry is kept, so it can describe the whole
    /// download, and answering `None` there keeps nothing.
    ///
    /// A track being downloaded goes to the downloads, and any other to the cache while caching
    /// is on. The source passes through untouched whatever happens to the vault. A disk that
    /// refuses the entry only means the track is not kept.
    pub(crate) fn keep<S: Source>(
        &self,
        id: &str,
        variant: &str,
        facts: impl FnOnce() -> Option<Vec<u8>> + Send + 'static,
        source: S,
    ) -> Recording<S> {
        let place = match self.is_wanted(id) {
            true => Place::Pinned,
            false if self.vault.caching.load(Ordering::Relaxed) => Place::Cache,
            false => return Recording::through(source),
        };
        let pending = match self.begin(place, id, variant, Box::new(facts)) {
            Ok(pending) => pending,
            Err(error) => {
                log::debug!("vault: not keeping a {} track: {error:#}", self.provider);
                None
            }
        };
        Recording { source, pending }
    }

    /// Keeps a whole track that arrived in one piece as a download, with the provider's `facts`
    /// beside it.
    pub(crate) fn pin(&self, id: &str, facts: Vec<u8>, bytes: &[u8]) -> Result<()> {
        let mut pending = self
            .begin(Place::Pinned, id, "", Box::new(move || Some(facts)))?
            .context("the vault is locked")?;
        pending.write(bytes)?;
        pending.commit()
    }

    /// The download of track `id`, ready to be read anywhere in it, for a player that seeks in
    /// the file itself.
    pub(crate) fn open_pinned(&self, id: &str) -> Option<(Vec<u8>, StoredReader)> {
        let kept = self.find_download(id)?;
        Some((kept.facts, kept.source.into_reader(kept.length)))
    }

    /// Marks track `id` as being downloaded, so the next fetch of it is kept as a download.
    /// `label` names the file when the download is plain, and `None` names it after a hash.
    pub(crate) fn want(&self, id: &str, label: Option<String>) {
        self.vault
            .wanted()
            .insert((self.provider, id.to_owned()), label);
    }

    /// Ends what [`want`](Self::want) started.
    pub(crate) fn unwant(&self, id: &str) {
        self.vault.wanted().remove(&(self.provider, id.to_owned()));
    }

    /// Whether track `id` has been downloaded. It answers no while the vault is still unlocking,
    /// so an engine calls [`settle`](Self::settle) first.
    pub(crate) fn holds(&self, id: &str) -> bool {
        self.plain_file(id).is_some() || self.holds_sealed(id)
    }

    fn holds_sealed(&self, id: &str) -> bool {
        let Some(prk) = self.vault.keys.get() else {
            return false;
        };
        self.spot(prk, Place::Pinned, id, "")
            .is_some_and(|(_, path)| path.is_file())
    }

    /// Whether track `id` is still sealed on a shelf that keeps plain files, as a download made
    /// before the shelf did is, so [`unseal`](Self::unseal) has something to do. It answers no
    /// while the vault is locked.
    pub(crate) fn sealed_download(&self, id: &str) -> bool {
        self.plain && self.holds_sealed(id)
    }

    /// Rewrites a sealed download of track `id` as a plain file named after `label`, and removes
    /// the sealed copy once the plain one is in place. It reads and writes the whole track, so it
    /// runs on a blocking thread.
    pub(crate) fn unseal(&self, id: &str, label: Option<String>) -> Result<()> {
        if !self.plain {
            return Ok(());
        }
        let prk = self.vault.keys.get().context("the vault is locked")?;
        let Some(kept) = self.find_in(prk, Place::Pinned, id, "") else {
            return Ok(());
        };
        let old = kept.source.path.clone();
        let facts = kept.facts.clone();
        let mut pending = self
            .begin(Place::Pinned, id, "", Box::new(move || Some(facts)))?
            .context("the vault is locked")?;
        if let Some(label) = label {
            pending.label(label);
        }
        copy(kept, pending)?;
        fs::remove_file(old).ok();
        log::debug!(
            "vault: rewrote a sealed {} download as a file",
            self.provider
        );
        Ok(())
    }

    /// Whether track `id` has been downloaded, or may have been since the vault is still
    /// unlocking and cannot tell yet.
    pub(crate) fn may_hold(&self, id: &str) -> bool {
        self.vault.unlocking() || self.holds(id)
    }

    /// Waits up to [`UNLOCK_WAIT`] for the vault to have its key or give up on it.
    pub(crate) async fn settle(&self) {
        self.vault.ready().await;
    }

    /// Removes the download of track `id`.
    pub(crate) fn discard(&self, id: &str) {
        self.discard_plain(id);
        let Some(prk) = self.vault.keys.get() else {
            return;
        };
        let Some((_, path)) = self.spot(prk, Place::Pinned, id, "") else {
            return;
        };
        match fs::remove_file(path) {
            Ok(()) => log::debug!("vault: removed a {} download", self.provider),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("vault: cannot remove a {} download: {error}", self.provider),
        }
    }

    fn is_wanted(&self, id: &str) -> bool {
        self.vault
            .wanted()
            .contains_key(&(self.provider, id.to_owned()))
    }

    fn find_in(&self, prk: &hkdf::Prk, place: Place, id: &str, variant: &str) -> Option<Kept> {
        let (name, path) = self.spot(prk, place, id, variant)?;
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                log::warn!("vault: cannot open a kept track: {error}");
                return None;
            }
        };
        match self.open(prk, &name, &path, file) {
            Ok(kept) => {
                if place == Place::Cache {
                    let now = SystemTime::now();
                    if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                        file.set_times(FileTimes::new().set_accessed(now).set_modified(now))
                            .ok();
                    }
                }
                Some(kept)
            }
            Err(error) => {
                log::warn!(
                    "vault: dropping a damaged {} track: {error:#}",
                    self.provider
                );
                fs::remove_file(&path).ok();
                None
            }
        }
    }

    /// Writes a cached copy again as a download and removes the cached one. It reads and writes
    /// the whole track, so it runs on a blocking thread rather than an engine's.
    fn promote(&self, id: &str, kept: Kept) -> Result<Kept> {
        let facts = kept.facts.clone();
        let cached = kept.source.path.clone();
        let pending = self
            .begin(Place::Pinned, id, "", Box::new(move || Some(facts)))?
            .context("the vault is locked")?;
        copy(kept, pending)?;
        fs::remove_file(cached).ok();
        log::debug!(
            "vault: moved a cached {} track to the downloads",
            self.provider
        );
        self.find_download(id).context("the download does not open")
    }

    /// The download of track `id`, plain or sealed.
    fn find_download(&self, id: &str) -> Option<Kept> {
        if let Some(kept) = self.find_plain(id) {
            return Some(kept);
        }
        let prk = self.vault.keys.get()?;
        self.find_in(prk, Place::Pinned, id, "")
    }

    /// The index file of a plain download of track `id`. Its name is a plain hash of the id,
    /// since a plain download is found without the key.
    fn index_path(&self, id: &str) -> PathBuf {
        let mut context = digest::Context::new(&digest::SHA256);
        for part in [self.provider, id, &self.scope] {
            context.update(part.as_bytes());
            context.update(&[0]);
        }
        let name = hex(&context.finish().as_ref()[..16]);
        self.folder(Place::Pinned).join(INDEX).join(name)
    }

    /// The audio file of a plain download of track `id` and the facts its index carries, or
    /// `None` when there is none. An index whose file is gone is removed.
    fn plain_file(&self, id: &str) -> Option<(PathBuf, Vec<u8>)> {
        if !self.plain {
            return None;
        }
        let index = self.index_path(id);
        let read = fs::read(&index).ok()?;
        let at = read.iter().position(|byte| *byte == b'\n')?;
        let name = std::str::from_utf8(&read[..at]).ok()?;
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            log::warn!("vault: a {} index names no usable file", self.provider);
            return None;
        }
        let path = self.folder(Place::Pinned).join(name);
        if !path.is_file() {
            fs::remove_file(&index).ok();
            return None;
        }
        Some((path, read[at + 1..].to_vec()))
    }

    /// The plain download of track `id`, ready to play.
    fn find_plain(&self, id: &str) -> Option<Kept> {
        let (path, facts) = self.plain_file(id)?;
        let file = File::open(&path)
            .inspect_err(|error| log::warn!("vault: cannot open a plain download: {error}"))
            .ok()?;
        let length = file.metadata().ok()?.len();
        Some(Kept {
            facts,
            length,
            source: Stored {
                file,
                key: None,
                name: String::new(),
                path,
                index: 0,
                left: length,
                buffer: Vec::with_capacity(CHUNK),
            },
        })
    }

    /// Removes the plain download of track `id` and its index.
    fn discard_plain(&self, id: &str) {
        let Some((path, _)) = self.plain_file(id) else {
            return;
        };
        match fs::remove_file(&path) {
            Ok(()) => log::debug!("vault: removed a plain {} download", self.provider),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!(
                    "vault: cannot remove a plain {} download: {error}",
                    self.provider
                );
                return;
            }
        }
        fs::remove_file(self.index_path(id)).ok();
    }

    /// The folder this shelf keeps its entries in within `place`.
    fn folder(&self, place: Place) -> PathBuf {
        self.vault.base(place).join(self.provider)
    }

    /// The file name and path of a track in `place`. The name is a keyed hash that says nothing
    /// to anyone without the key. A download's ignores `variant`.
    fn spot(
        &self,
        prk: &hkdf::Prk,
        place: Place,
        id: &str,
        variant: &str,
    ) -> Option<(String, PathBuf)> {
        let key: hmac::Key = prk
            .expand(&[b"name", self.provider.as_bytes()], hmac::HMAC_SHA256)
            .ok()?
            .into();
        let mut context = hmac::Context::with_key(&key);
        context.update(id.as_bytes());
        context.update(&[0]);
        context.update(self.scope.as_bytes());
        context.update(&[0]);
        match place {
            Place::Cache => context.update(variant.as_bytes()),
            Place::Pinned => context.update(&[1]),
        }
        let name = hex(&context.sign().as_ref()[..16]);
        let path = self.folder(place).join(&name);
        Some((name, path))
    }

    /// The key that seals one entry, from the master key, this shelf and the entry's salt.
    fn key(&self, prk: &hkdf::Prk, salt: &[u8]) -> Result<LessSafeKey> {
        let info = [b"track".as_slice(), self.provider.as_bytes(), salt];
        let okm = prk
            .expand(&info, &CHACHA20_POLY1305)
            .map_err(|_| anyhow!("cannot derive a track key"))?;
        Ok(LessSafeKey::new(UnboundKey::from(okm)))
    }

    /// Opens a temporary file for a new entry in `place`, or answers `None` while the vault is
    /// locked and the entry would be sealed. A download on a plain shelf is a plain file, and
    /// anything cached is sealed.
    fn begin(
        &self,
        place: Place,
        id: &str,
        variant: &str,
        facts: Describe,
    ) -> Result<Option<Pending>> {
        if place == Place::Pinned && self.plain {
            return self.begin_plain(id, facts).map(Some);
        }
        let Some(prk) = self.vault.keys.get() else {
            return Ok(None);
        };
        let (name, path) = self
            .spot(prk, place, id, variant)
            .context("cannot name the entry")?;
        let base = self.vault.base(place);
        let folder = self.folder(place);
        create_private(base)?;
        create_private(&folder)?;
        let mut salt = [0; SALT];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_| anyhow!("cannot draw a salt"))?;
        let key = self.key(prk, &salt)?;
        let temporary = folder.join(format!(
            ".{name}.{}.{}",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = private_file(&temporary)?;
        file.write_all(&salt).context("cannot start the entry")?;
        let (cache, ceiling) = match place {
            Place::Cache => (Some(self.vault.clone()), self.vault.limit() / SHARE),
            Place::Pinned => (None, u64::MAX),
        };
        Ok(Some(Pending {
            cache,
            path,
            temporary,
            name,
            file,
            key: Some(key),
            plain: None,
            facts: Some(facts),
            buffer: Vec::with_capacity(CHUNK + TAG),
            index: 0,
            written: SALT as u64,
            ceiling,
        }))
    }

    /// Opens a temporary file for a plain download of track `id`. Its file name and extension
    /// are settled when it is kept, once the label and the format are known.
    fn begin_plain(&self, id: &str, facts: Describe) -> Result<Pending> {
        let index = self.index_path(id);
        let folder = self.folder(Place::Pinned);
        create_private(self.vault.base(Place::Pinned))?;
        create_private(&folder)?;
        create_private(&folder.join(INDEX))?;
        let name = index
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temporary = folder.join(format!(
            ".{name}.{}.{}",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        let file = private_file(&temporary)?;
        let label = self
            .vault
            .wanted()
            .get(&(self.provider, id.to_owned()))
            .cloned()
            .flatten();
        Ok(Pending {
            cache: None,
            path: folder,
            temporary,
            name,
            file,
            key: None,
            plain: Some(Plain {
                index,
                label,
                head: Vec::with_capacity(HEAD),
            }),
            facts: Some(facts),
            buffer: Vec::new(),
            index: 0,
            written: 0,
            ceiling: u64::MAX,
        })
    }

    /// Reads the salt and the facts of an entry, and leaves a source at its first chunk.
    fn open(&self, prk: &hkdf::Prk, name: &str, path: &Path, mut file: File) -> Result<Kept> {
        let size = file.metadata()?.len();
        let mut salt = [0; SALT];
        file.read_exact(&mut salt).context("cannot read the salt")?;
        let key = self.key(prk, &salt)?;
        if size < (SALT + TAG + 4) as u64 {
            bail!("the entry is too short");
        }
        let mut length = [0; 4];
        file.seek(SeekFrom::End(-4))?;
        file.read_exact(&mut length)?;
        let sealed = u64::from(u32::from_le_bytes(length));
        let facts_at = size
            .checked_sub(4 + sealed)
            .filter(|at| *at >= SALT as u64)
            .context("the entry's facts run past its start")?;
        let mut facts = vec![0; sealed as usize];
        file.seek(SeekFrom::Start(facts_at))?;
        file.read_exact(&mut facts)?;
        let opened = open(&key, name, Block::Facts, 0, &mut facts)?;
        let (length, facts) = opened
            .split_first_chunk::<8>()
            .context("the entry's facts have no length")?;
        let length = u64::from_le_bytes(*length);
        let facts = facts.to_vec();
        file.seek(SeekFrom::Start(SALT as u64))?;
        Ok(Kept {
            facts,
            length,
            source: Stored {
                file,
                key: Some(key),
                name: name.to_owned(),
                path: path.to_owned(),
                index: 0,
                left: facts_at - SALT as u64,
                buffer: Vec::with_capacity(CHUNK + TAG),
            },
        })
    }
}

/// A kept track, ready to be played again.
pub(crate) struct Kept {
    /// What the provider stored beside the track when it was kept.
    pub(crate) facts: Vec<u8>,
    /// The length of the track as the provider sent it.
    pub(crate) length: u64,
    /// The track, chunk by chunk, as the provider sent it.
    pub(crate) source: Stored,
}

/// The bytes of a kept track, read a chunk at a time as the stream asks for them. A sealed
/// chunk that fails to open removes the entry, so the next play downloads the track again. A
/// plain download is read as it lies and never removed here.
pub(crate) struct Stored {
    file: File,
    /// What opens each chunk, or `None` for a plain download.
    key: Option<LessSafeKey>,
    name: String,
    /// Where the entry lies, so a chunk that fails to open can take the entry with it.
    path: PathBuf,
    index: u64,
    /// The stored bytes still to read.
    left: u64,
    buffer: Vec<u8>,
}

impl Source for Stored {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        let chunk = self.open_next();
        if let Err(error) = &chunk {
            log::warn!("vault: cannot read a kept track: {error:#}");
            if self.key.is_some() {
                fs::remove_file(&self.path).ok();
            }
        }
        chunk
    }
}

impl Stored {
    /// The same bytes read anywhere in them. `length` is how long the track is.
    pub(crate) fn into_reader(self, length: u64) -> StoredReader {
        StoredReader {
            file: self.file,
            key: self.key,
            name: self.name,
            stored: self.left,
            length,
            position: 0,
            opened: None,
        }
    }

    /// Reads and opens the next chunk, or answers `None` once there are none left.
    fn open_next(&mut self) -> Result<Option<Bytes>> {
        if self.left == 0 {
            return Ok(None);
        }
        let tag = self.key.as_ref().map_or(0, |_| TAG);
        let take = self.left.min((CHUNK + tag) as u64) as usize;
        self.buffer.resize(take, 0);
        self.file
            .read_exact(&mut self.buffer)
            .context("cannot read a kept track")?;
        self.left -= take as u64;
        let Some(key) = &self.key else {
            return Ok(Some(Bytes::copy_from_slice(&self.buffer)));
        };
        let block = match self.left {
            0 => Block::Last,
            _ => Block::Chunk,
        };
        let opened = open(key, &self.name, block, self.index, &mut self.buffer)?;
        self.index += 1;
        Ok(Some(Bytes::copy_from_slice(opened)))
    }
}

/// A kept track read anywhere in it. Every chunk but the last holds [`CHUNK`] bytes, so a
/// position names its chunk directly, and the chunk last opened is held for the reads after it.
pub(crate) struct StoredReader {
    file: File,
    /// What opens each chunk, or `None` for a plain download.
    key: Option<LessSafeKey>,
    name: String,
    /// The bytes of stored chunks, after the salt of a sealed entry.
    stored: u64,
    /// The bytes of the track.
    length: u64,
    position: u64,
    opened: Option<(u64, Vec<u8>)>,
}

impl StoredReader {
    /// How long the track is.
    pub(crate) fn length(&self) -> u64 {
        self.length
    }

    /// The plain bytes of chunk `index`.
    fn chunk(&mut self, index: u64) -> io::Result<&[u8]> {
        if self.opened.as_ref().is_none_or(|(held, _)| *held != index) {
            let (tag, salt) = match self.key {
                Some(_) => (TAG, SALT),
                None => (0, 0),
            };
            let stride = (CHUNK + tag) as u64;
            let start = index * stride;
            let size = stride.min(self.stored.saturating_sub(start)) as usize;
            let mut buffer = vec![0; size];
            self.file.seek(SeekFrom::Start(salt as u64 + start))?;
            self.file.read_exact(&mut buffer)?;
            let plain = match &self.key {
                Some(key) => {
                    let block = match start + size as u64 >= self.stored {
                        true => Block::Last,
                        false => Block::Chunk,
                    };
                    open(key, &self.name, block, index, &mut buffer)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                        .to_vec()
                }
                None => buffer,
            };
            self.opened = Some((index, plain));
        }
        Ok(self
            .opened
            .as_ref()
            .map_or(&[], |(_, plain)| plain.as_slice()))
    }
}

impl Read for StoredReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || out.is_empty() {
            return Ok(0);
        }
        let index = self.position / CHUNK as u64;
        let within = (self.position % CHUNK as u64) as usize;
        let chunk = self.chunk(index)?;
        let take = chunk.len().saturating_sub(within).min(out.len());
        out[..take].copy_from_slice(&chunk[within..within + take]);
        self.position += take as u64;
        Ok(take)
    }
}

impl Seek for StoredReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(by) => self.length.checked_add_signed(by),
            SeekFrom::Current(by) => self.position.checked_add_signed(by),
        };
        self.position = target.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a seek before the start")
        })?;
        Ok(self.position)
    }
}

/// A source whose bytes are sealed into the vault as they pass through.
pub(crate) struct Recording<S> {
    source: S,
    /// The entry being written, until it is kept or given up on.
    pending: Option<Pending>,
}

impl<S> Recording<S> {
    /// A source that passes through without keeping anything, for a provider with no shelf.
    pub(crate) fn through(source: S) -> Self {
        Self {
            source,
            pending: None,
        }
    }
}

impl<S: Source> Source for Recording<S> {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        let chunk = self.source.chunk().await?;
        if let Some(mut pending) = self.pending.take() {
            let written = match &chunk {
                Some(bytes) => pending.write(bytes).map(|()| Some(pending)),
                None => pending.commit().map(|()| None),
            };
            match written {
                Ok(pending) => self.pending = pending,
                Err(error) => log::warn!("vault: cannot keep the track: {error:#}"),
            }
        }
        Ok(chunk)
    }
}

/// An entry being written. Dropping it before [`commit`](Self::commit) removes the file, so
/// a skipped or broken track leaves nothing behind.
struct Pending {
    /// The vault whose cache the entry counts against, or `None` for a download.
    cache: Option<Arc<Vault>>,
    /// Where a sealed entry goes, or the folder a plain download goes into.
    path: PathBuf,
    temporary: PathBuf,
    name: String,
    file: File,
    /// Seals every chunk and the facts, or `None` for a plain download.
    key: Option<LessSafeKey>,
    /// What only a plain download needs.
    plain: Option<Plain>,
    /// Answers the provider's facts once the whole track is in. Taken when the entry is kept.
    facts: Option<Describe>,
    /// What has arrived since the last sealed chunk, and the room to seal it in.
    buffer: Vec<u8>,
    index: u64,
    /// The bytes of the track the provider has sent so far, salt included.
    written: u64,
    /// The most one entry may take before it is given up on.
    ceiling: u64,
}

/// What a plain download needs beyond what every entry has.
struct Plain {
    /// The index file that will name the audio file.
    index: PathBuf,
    /// What to call the audio file, or `None` to name it after a hash of the id.
    label: Option<String>,
    /// The first bytes of the track, which tell its format.
    head: Vec<u8>,
}

impl Pending {
    /// Sets what a plain download's file is called. A sealed entry has no name to give.
    fn label(&mut self, label: String) {
        if let Some(plain) = &mut self.plain {
            plain.label = Some(label);
        }
    }

    /// Seals every whole chunk that has built up. One is always held back, so the last chunk
    /// sealed is never empty unless the whole track is. A plain download is written as it comes.
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.written += bytes.len() as u64;
        if self.written > self.ceiling {
            bail!("the track is larger than one entry may be");
        }
        if let Some(plain) = &mut self.plain {
            let room = HEAD.saturating_sub(plain.head.len()).min(bytes.len());
            plain.head.extend_from_slice(&bytes[..room]);
            return self
                .file
                .write_all(bytes)
                .context("cannot write a download");
        }
        let mut bytes = bytes;
        while !bytes.is_empty() {
            let room = CHUNK + 1 - self.buffer.len().min(CHUNK + 1);
            let (head, rest) = bytes.split_at(room.min(bytes.len()));
            self.buffer.extend_from_slice(head);
            bytes = rest;
            if self.buffer.len() > CHUNK {
                let spill = self.buffer.split_off(CHUNK);
                self.seal(Block::Chunk)?;
                self.buffer = spill;
            }
        }
        Ok(())
    }

    /// Seals what is held as the last chunk, then the facts, and moves the entry into place.
    fn commit(mut self) -> Result<()> {
        let told = self
            .facts
            .take()
            .and_then(|facts| facts())
            .context("the provider cannot vouch for the whole track")?;
        if let Some(plain) = self.plain.take() {
            return self.commit_plain(plain, told);
        }
        let length = self.written - SALT as u64;
        self.seal(Block::Last)?;
        let mut facts = Vec::with_capacity(8 + told.len() + TAG);
        facts.extend_from_slice(&length.to_le_bytes());
        facts.extend_from_slice(&told);
        self.key
            .as_ref()
            .context("a plain download has no key")?
            .seal_in_place_append_tag(
                nonce(Block::Facts, 0),
                Aad::from(self.name.as_bytes()),
                &mut facts,
            )
            .map_err(|_| anyhow!("cannot seal the facts"))?;
        let sealed = u32::try_from(facts.len()).context("the facts are too long")?;
        self.file.write_all(&facts)?;
        self.file.write_all(&sealed.to_le_bytes())?;
        self.file.flush()?;
        let size = self.file.metadata()?.len();
        fs::rename(&self.temporary, &self.path).context("cannot move the entry into place")?;
        log::debug!(
            "vault: kept a track, {:.1} MiB",
            size as f64 / (1024.0 * 1024.0)
        );
        if let Some(vault) = &self.cache {
            vault.settle(size);
        }
        Ok(())
    }

    /// Moves a plain download into place under a free name after its label, then writes the
    /// index that finds it, and removes any older file the index named.
    fn commit_plain(mut self, plain: Plain, told: Vec<u8>) -> Result<()> {
        self.file.flush()?;
        let stem = plain
            .label
            .map(|label| file_stem(&label))
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| self.name.clone());
        let extension = extension(&plain.head);
        let older = fs::read(&plain.index).ok().and_then(|read| {
            let at = read.iter().position(|byte| *byte == b'\n')?;
            String::from_utf8(read[..at].to_vec()).ok()
        });
        let path = (1..)
            .map(|count| match count {
                1 => self.path.join(format!("{stem}.{extension}")),
                _ => self.path.join(format!("{stem} ({count}).{extension}")),
            })
            .find(|path| !path.exists())
            .context("there is no free name for the download")?;
        fs::rename(&self.temporary, &path).context("cannot move the download into place")?;
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut index = Vec::with_capacity(file_name.len() + 1 + told.len());
        index.extend_from_slice(file_name.as_bytes());
        index.push(b'\n');
        index.extend_from_slice(&told);
        let temporary = plain.index.with_extension(format!(
            "{}.{}",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        let written = private_file(&temporary)
            .and_then(|mut file| file.write_all(&index).context("cannot write the index"))
            .and_then(|()| {
                fs::rename(&temporary, &plain.index).context("cannot move the index into place")
            });
        if let Err(error) = written {
            fs::remove_file(&temporary).ok();
            fs::remove_file(&path).ok();
            return Err(error);
        }
        if let Some(older) = older.filter(|older| *older != file_name && !older.contains('/')) {
            fs::remove_file(self.path.join(older)).ok();
        }
        log::debug!(
            "vault: kept a plain download, {:.1} MiB",
            self.written as f64 / (1024.0 * 1024.0)
        );
        Ok(())
    }

    fn seal(&mut self, block: Block) -> Result<()> {
        self.key
            .as_ref()
            .context("a plain download has no key")?
            .seal_in_place_append_tag(
                nonce(block, self.index),
                Aad::from(self.name.as_bytes()),
                &mut self.buffer,
            )
            .map_err(|_| anyhow!("cannot seal a chunk"))?;
        self.file
            .write_all(&self.buffer)
            .context("cannot write a chunk")?;
        self.buffer.clear();
        self.index += 1;
        Ok(())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        // Once the entry is in place the temporary name is gone and this finds nothing.
        fs::remove_file(&self.temporary).ok();
    }
}

/// The nonce of one sealed block. Every entry has its own key, so the index alone keeps a
/// nonce from repeating.
fn nonce(block: Block, index: u64) -> Nonce {
    let mut nonce = [0; NONCE_LEN];
    nonce[..8].copy_from_slice(&index.to_be_bytes());
    nonce[8] = block as u8;
    Nonce::assume_unique_for_key(nonce)
}

/// Opens one sealed block in place and answers the plain bytes inside it.
fn open<'a>(
    key: &LessSafeKey,
    name: &str,
    block: Block,
    index: u64,
    sealed: &'a mut [u8],
) -> Result<&'a mut [u8]> {
    key.open_in_place(nonce(block, index), Aad::from(name.as_bytes()), sealed)
        .map_err(|_| anyhow!("a sealed block does not open"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether a name in a shelf folder is a sealed entry, whose name is 32 hex digits, rather than
/// a plain download, an index or a half-written file.
fn sealed_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Copies a kept track into a new entry and keeps it.
fn copy(mut kept: Kept, mut pending: Pending) -> Result<()> {
    while let Some(bytes) = kept.source.open_next()? {
        pending.write(&bytes)?;
    }
    pending.commit()
}

/// A label as a file name every desktop accepts: no path separators, no characters Windows
/// refuses, no leading dot and at most [`NAME_BYTES`] bytes.
fn file_stem(label: &str) -> String {
    let cleaned = label
        .chars()
        .map(|letter| match letter {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            letter if letter.is_control() => ' ',
            letter => letter,
        })
        .collect::<String>();
    let mut stem = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_owned();
    if stem.len() > NAME_BYTES {
        let mut cut = NAME_BYTES;
        while !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        stem.truncate(cut);
        stem = stem.trim_end_matches(['.', ' ']).to_owned();
    }
    stem
}

/// The file extension the start of an audio file calls for, by the signature it opens with.
fn extension(head: &[u8]) -> &'static str {
    let at = |offset: usize, signature: &[u8]| {
        head.get(offset..offset + signature.len()) == Some(signature)
    };
    let sync = |mask: u8, value: u8| head.len() >= 2 && head[0] == 0xff && head[1] & mask == value;
    if at(0, b"fLaC") {
        "flac"
    } else if at(0, b"OggS") && at(28, b"OpusHead") {
        "opus"
    } else if at(0, b"OggS") {
        "ogg"
    } else if at(4, b"ftyp") {
        "m4a"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "wav"
    } else if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        "aiff"
    } else if at(0, &[0x1a, 0x45, 0xdf, 0xa3]) {
        "mka"
    } else if at(0, b"MAC ") {
        "ape"
    } else if at(0, b"wvpk") {
        "wv"
    } else if at(0, b"ID3") {
        "mp3"
    } else if sync(0xf6, 0xf0) {
        // An ADTS header has layer bits of zero, where MPEG audio has one, two or three.
        "aac"
    } else if sync(0xe0, 0xe0) {
        "mp3"
    } else {
        "audio"
    }
}

/// Creates `path` readable by the owner alone, or makes it so when another program created it
/// open to everyone.
fn create_private(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(path)
            .with_context(|| format!("cannot read {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .with_context(|| format!("cannot make {} private", path.display()))?;
        }
    }
    Ok(())
}

/// Removes a file or a whole folder.
fn remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path)?.is_dir() {
        true => fs::remove_dir_all(path),
        false => fs::remove_file(path),
    }
}

/// Empties the folder of players keeping their own files. A running player clears its own
/// folder through `players`, and the folder of any other is removed.
fn forget_players(folder: &Path, players: &[(&'static str, Clear)]) -> io::Result<()> {
    for item in fs::read_dir(folder)?.flatten() {
        let name = item.file_name();
        let running = players
            .iter()
            .find(|(player, _)| name == *player)
            .is_some_and(|(_, clear)| clear());
        if !running {
            remove(&item.path())?;
        }
    }
    Ok(())
}

/// Evicts the least recently played entries under `root` until they take `room` bytes or
/// fewer, and answers what they take after. Half-written entries a crash left are dropped on
/// the way, and the folder of players keeping their own files is left alone.
fn evict(root: &Path, room: u64) -> u64 {
    let mut entries = abandoned(root);
    entries.sort_unstable_by_key(|(_, used, _)| *used);
    let mut total = entries.iter().map(|(_, _, size)| size).sum::<u64>();
    for (path, _, size) in entries {
        if total <= room {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    total
}

/// A new file readable and writable by the owner alone.
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))
}

/// Sonora's vault folder, `$XDG_CACHE_HOME/sonora/vault`.
fn standard_root() -> PathBuf {
    crate::credentials::root().join("vault")
}

/// What everything under `path` takes on disk, in bytes. A folder that cannot be read counts as
/// empty.
fn footprint(path: &Path) -> u64 {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|item| match item.file_type() {
            Ok(kind) if kind.is_dir() => footprint(&item.path()),
            Ok(_) => item.metadata().map_or(0, |metadata| metadata.len()),
            Err(_) => 0,
        })
        .sum()
}

/// Sonora's downloads folder, `$XDG_DATA_HOME/sonora/downloads`.
fn pinned_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join("downloads")
}

/// Removes what a crash left half-written under `root` a while ago, and answers every whole
/// entry there with when it last played and its size. The folder of players keeping their own
/// files is left alone.
fn abandoned(root: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let now = SystemTime::now();
    let mut entries = Vec::new();
    for shelf in fs::read_dir(root).into_iter().flatten().flatten() {
        if shelf.file_name() == FOREIGN {
            continue;
        }
        for item in fs::read_dir(shelf.path()).into_iter().flatten().flatten() {
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                continue;
            }
            let used = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let partial = item.file_name().to_string_lossy().starts_with('.');
            if partial {
                if now.duration_since(used).is_ok_and(|age| age > ABANDONED) {
                    fs::remove_file(item.path()).ok();
                }
                continue;
            }
            entries.push((item.path(), used, metadata.len()));
        }
    }
    entries
}
