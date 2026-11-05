//! A track downloaded while it plays. The decoder reads from the front of one spool while the
//! response keeps filling the back, so playback starts after a short preroll instead of after
//! the whole file.
//!
//! The spool is an unlinked file in the cache directory, so a track sits in the page cache
//! rather than on the heap, and the kernel can drop it under pressure where it could never drop
//! a buffer. Memory is the fallback for a system where no such file can be made.
//!
//! Every provider that streams a file over HTTP uses this. What differs between them is where
//! the bytes come from, which is [`Source`], and what happens to them, which is [`Body`].
//! Subsonic and YouTube Music take them as they come, Deezer decrypts each Blowfish stripe as
//! it lands, and Apple Music indexes CENC fragments and hands each sample to a CDM the moment a
//! reader asks for it. Everything else, the waiting and the seeking and what a broken
//! connection does, is the same for all of them and lives here.

use std::borrow::Cow;
use std::fs::File;
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use bytes::Bytes;
use tokio::sync::watch;

/// How much has to be in before the decoder is let loose on the spool. At any ordinary bitrate
/// this is several seconds of audio, and the download outruns playback many times over, so the
/// lead only grows from here.
pub const PREROLL: usize = 256 * 1024;

/// How long a read waits for bytes that have not arrived. rodio pulls the decoder on the audio
/// thread, so a wait there is silence: bounded, because a dead connection must not hold the
/// output for the rest of the session.
const PATIENCE: Duration = Duration::from_secs(20);

/// The most one track may spool unless its body allows more. A long album side is tens of
/// megabytes, so this is only a ceiling on what a wrong `Content-Length` or an endless body can
/// cost. It also bounds the room a spool in memory reserves up front.
const CEILING: usize = 256 * 1024 * 1024;

/// What a provider does to the body of one track: as it arrives, and before it is served.
///
/// Every method has an answer that suits a plain file, so a provider overrides only what it
/// actually does differently. `feed` and `flush` run on the tokio task pulling the response;
/// `limit`, `tail` and `ready` run on whichever thread is reading, under the spool's lock.
pub trait Body: Send + 'static {
    /// What to append for one chunk of the response. `out` starts empty on every call, and
    /// whatever is left in it goes onto the end of the spool. The default passes the chunk on
    /// unchanged.
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(chunk);
    }

    /// The response ended: append anything that was held back waiting for more.
    fn flush(&mut self, _out: &mut Vec<u8>) {}

    /// How far into the spool a read may be served. The default serves everything that has
    /// arrived; a provider that must account for bytes before handing them over answers less.
    ///
    /// The spool is mutable because accounting for bytes can mean preparing them: an fMP4 has
    /// its sample entry relabelled once its samples can be decrypted.
    fn limit(&mut self, spool: &mut Spool, _complete: bool) -> usize {
        spool.len()
    }

    /// How much of the end of the track a read may have without waiting for it.
    ///
    /// Zero, for a plain file: a read waits for what it asked for. A decoder that probes the
    /// end of a file before playing needs the last few kilobytes answered rather than waited
    /// on, or it holds playback until the whole track has arrived.
    fn tail(&self) -> u64 {
        0
    }

    /// Makes `range` readable, in place, before it is copied out. The default has nothing to do.
    fn ready(&mut self, _spool: &mut Spool, _range: Range<usize>) -> io::Result<()> {
        Ok(())
    }

    /// Whether the spool will hold decrypted media, which has to stay in memory and never
    /// reach a file, even an unlinked one whose pages get written back.
    fn confidential(&self) -> bool {
        false
    }

    /// The most this track may spool. A body whose tracks run longer than any ordinary file
    /// answers more, and its track may then reach the heap at that size if the disk fails it.
    fn ceiling(&self) -> usize {
        CEILING
    }
}

/// A body that is already what it should be.
pub struct Plain;

impl Body for Plain {}

/// Where the bytes of one track come from, in file order. A single response is one, and so is
/// a provider that asks for the file a range at a time.
pub trait Source: Send + 'static {
    /// The next part of the body, or `None` once it has all arrived.
    fn chunk(&mut self) -> impl Future<Output = Result<Option<Bytes>>> + Send;
}

/// A failed read drops the url from its error, since a media url is often signed and the error
/// ends up in the log.
impl Source for reqwest::Response {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        Ok(reqwest::Response::chunk(self)
            .await
            .map_err(reqwest::Error::without_url)?)
    }
}

/// The bytes of one track that have arrived so far, as a reader would see them. They live in
/// an unlinked file in the cache directory, which goes away with the last handle to it, or in
/// memory when no such file can be made or written or the body is [`Body::confidential`].
pub struct Spool {
    len: usize,
    /// The body length the server announced, which a body can size its own bookkeeping by.
    announced: Option<usize>,
    store: Store,
}

enum Store {
    File(File),
    Memory(Vec<u8>),
}

impl Spool {
    /// A spool on disk, or in memory sized for `total` when the disk will not have one. The
    /// track may spool up to `ceiling` bytes.
    fn new(total: Option<u64>, ceiling: usize) -> Self {
        match spool_file() {
            Ok(file) => Self {
                len: 0,
                announced: announced(total, ceiling),
                store: Store::File(file),
            },
            Err(error) => {
                log::warn!("playback: cannot spool to disk, buffering in memory: {error}");
                Self::held(total, ceiling)
            }
        }
    }

    /// A spool in memory for a track under the default ceiling, for a test to fill by hand.
    #[cfg(test)]
    pub(crate) fn in_memory(total: Option<u64>) -> Self {
        Self::held(total, CEILING)
    }

    /// A spool in memory, with room reserved for `total` bytes when that is known and for the
    /// preroll when it is not. A body of unknown length then starts out big enough to be mapped
    /// on its own, rather than growing through a run of heap blocks that each leave a hole. The
    /// reservation never passes the default ceiling, whatever `ceiling` the track may grow to.
    fn held(total: Option<u64>, ceiling: usize) -> Self {
        let room = total.map_or(PREROLL, |total| usize::try_from(total).unwrap_or(CEILING));
        Self {
            len: 0,
            announced: announced(total, ceiling),
            store: Store::Memory(Vec::with_capacity(room.min(CEILING))),
        }
    }

    /// The body length the server announced, if it did.
    pub fn announced(&self) -> Option<usize> {
        self.announced
    }

    /// How many bytes have arrived.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Fills `out` from position `at`. Asking for anything past what has arrived is an error.
    pub fn read_at(&self, at: usize, out: &mut [u8]) -> io::Result<()> {
        let end = self.check(at, out.len())?;
        match &self.store {
            Store::File(file) => read_exact_at(file, out, at as u64),
            Store::Memory(held) => {
                out.copy_from_slice(&held[at..end]);
                Ok(())
            }
        }
    }

    /// A copy of `range`, which must lie within what has arrived.
    pub fn bytes(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        let mut out = vec![0; range.end.saturating_sub(range.start)];
        self.read_at(range.start, &mut out)?;
        Ok(out)
    }

    /// The bytes of `range`, which must lie within what has arrived. A spool in memory lends
    /// them where they are, and only one on disk pays for a copy.
    pub fn view(&self, range: Range<usize>) -> io::Result<Cow<'_, [u8]>> {
        match &self.store {
            Store::Memory(held) => {
                let end = self.check(range.start, range.end.saturating_sub(range.start))?;
                Ok(Cow::Borrowed(&held[range.start..end]))
            }
            Store::File(_) => self.bytes(range).map(Cow::Owned),
        }
    }

    /// Overwrites the bytes at `at` with `bytes`, which must lie within what has arrived.
    pub fn write_at(&mut self, at: usize, bytes: &[u8]) -> io::Result<()> {
        let end = self.check(at, bytes.len())?;
        match &mut self.store {
            Store::File(file) => write_all_at(file, bytes, at as u64),
            Store::Memory(held) => {
                held[at..end].copy_from_slice(bytes);
                Ok(())
            }
        }
    }

    /// Adds `bytes` to the end. A disk that stops taking them moves the spool into memory
    /// rather than breaking the track.
    pub(crate) fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Store::File(file) = &self.store {
            match write_all_at(file, bytes, self.len as u64) {
                Ok(()) => {
                    self.len += bytes.len();
                    return Ok(());
                }
                Err(error) => {
                    log::warn!("playback: cannot spool to disk, buffering in memory: {error}");
                    let mut held = vec![0; self.len];
                    read_exact_at(file, &mut held, 0)?;
                    self.store = Store::Memory(held);
                }
            }
        }
        if let Store::Memory(held) = &mut self.store {
            held.extend_from_slice(bytes);
            self.len = held.len();
        }
        Ok(())
    }

    /// The end of `len` bytes from `at`, when all of them have arrived.
    fn check(&self, at: usize, len: usize) -> io::Result<usize> {
        at.checked_add(len)
            .filter(|end| *end <= self.len)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "a read runs past what has arrived",
                )
            })
    }
}

struct Buffered<B> {
    /// The track so far, as a reader would see it.
    spool: Spool,
    /// The body length the server announced, if it did.
    total: Option<u64>,
    /// The response finished, one way or the other.
    complete: bool,
    failed: Option<String>,
    body: B,
}

struct Shared<B> {
    state: Mutex<Buffered<B>>,
    filled: Condvar,
}

impl<B: Body> Shared<B> {
    fn held(&self) -> io::Result<MutexGuard<'_, Buffered<B>>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("the track buffer is poisoned"))
    }

    fn finish(&self, failed: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            let mut tail = Vec::new();
            state.body.flush(&mut tail);
            let spooled = state.spool.append(&tail);
            if state.failed.is_none() {
                state.failed = failed.or_else(|| spooled.err().map(|error| error.to_string()));
            }
            state.complete = true;
        }
        self.filled.notify_all();
    }
}

/// One download in progress. Clones share the bytes, and `reader` hands out an independent
/// cursor over them, so a preloaded track can be decoded more than once.
pub struct Stream<B = Plain> {
    shared: Arc<Shared<B>>,
    arrived: watch::Receiver<usize>,
}

/// A handle on a [`Stream`] that does not keep it alive, for background work that should stop
/// once nothing else wants the track. The download stops when the last strong handle goes,
/// whatever weak ones are left.
pub struct WeakStream<B = Plain> {
    shared: Weak<Shared<B>>,
    arrived: watch::Receiver<usize>,
}

impl<B> Clone for Stream<B> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            arrived: self.arrived.clone(),
        }
    }
}

impl<B: Body> Stream<B> {
    /// Starts pulling `response` into a spool and returns at once. The download carries on in
    /// the background for as long as any reader or clone is alive; it stops on its own once all
    /// of them are gone.
    pub fn new(response: reqwest::Response, body: B) -> Self {
        let total = response.content_length();
        Self::pulling(response, total, body)
    }

    /// Starts pulling `source` into a spool and returns at once, like [`new`](Self::new).
    /// `total` is the body length when it is known up front.
    pub fn pulling(source: impl Source, total: Option<u64>, body: B) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(Buffered {
                spool: match body.confidential() {
                    true => Spool::held(total, body.ceiling()),
                    false => Spool::new(total, body.ceiling()),
                },
                total,
                complete: false,
                failed: None,
                body,
            }),
            filled: Condvar::new(),
        });
        let (progress, arrived) = watch::channel(0usize);
        tokio::spawn(pump(source, Arc::downgrade(&shared), progress));
        Self { shared, arrived }
    }
    /// Starts the download and waits for the preroll, or for the whole body of a track shorter
    /// than that.
    pub async fn open(response: reqwest::Response, body: B) -> Result<Self> {
        Self::new(response, body).primed().await
    }

    /// Waits for the preroll, or for the whole body of a track shorter than that.
    pub async fn primed(mut self) -> Result<Self> {
        let wanted = self.total().map_or(PREROLL, |total| {
            usize::try_from(total).unwrap_or(usize::MAX).min(PREROLL)
        });
        self.wait_for(wanted).await?;
        Ok(self)
    }

    /// Waits until the download has ended, whether it finished or broke.
    pub async fn finished(&self) {
        let mut arrived = self.arrived.clone();
        while !self.done() {
            if arrived.changed().await.is_err() {
                break;
            }
        }
    }

    /// Waits until `wanted` bytes have arrived, or the download ends. Nothing here blocks a
    /// thread: this is the async side of the same spool the readers wait on.
    pub async fn wait_for(&mut self, wanted: usize) -> Result<()> {
        loop {
            let arrived = *self.arrived.borrow_and_update();
            if arrived >= wanted || self.done() {
                break;
            }
            if self.arrived.changed().await.is_err() {
                break;
            }
        }
        match self.failed() {
            Some(failed) => bail!("the stream broke before playback could start: {failed}"),
            None => Ok(()),
        }
    }

    /// An independent cursor over the track.
    pub fn reader(&self) -> Reader<B> {
        Reader {
            shared: self.shared.clone(),
            head: 0,
            from: 0,
            at: 0,
        }
    }

    /// A cursor that serves the first `head` bytes and then carries on from `from`, so a
    /// decoder can be opened part way into a file that describes itself up front.
    pub fn spliced(&self, head: usize, from: usize) -> Reader<B> {
        Reader {
            shared: self.shared.clone(),
            head,
            from,
            at: 0,
        }
    }

    /// The body length the server announced, or the length it turned out to be once the
    /// download ended.
    pub fn total(&self) -> Option<u64> {
        let state = self.shared.state.lock().ok()?;
        state
            .total
            .or_else(|| state.complete.then_some(state.spool.len() as u64))
    }

    /// How much has arrived so far.
    pub fn arrived(&self) -> usize {
        self.shared
            .state
            .lock()
            .map(|state| state.spool.len())
            .unwrap_or(0)
    }

    /// Reaches the provider's own half of the spool, and again on every chunk that lands,
    /// until it answers something or the download ends. The third argument is whether the body
    /// is complete, so a question with no answer left can say so rather than wait.
    ///
    /// This blocks whichever thread asks, bounded by the same patience a read has. It is for a
    /// question a reader would otherwise have to guess at, where guessing is worse than
    /// waiting: where in the file a position is, when the download has not reached it yet.
    pub fn awaiting<T>(
        &self,
        mut read: impl FnMut(&mut B, &mut Spool, bool) -> Option<T>,
    ) -> Option<T> {
        let mut state = self.shared.state.lock().ok()?;
        let deadline = Instant::now() + PATIENCE;
        loop {
            let Buffered {
                spool,
                body,
                complete,
                failed,
                ..
            } = &mut *state;
            let (complete, broken) = (*complete, failed.is_some());
            if let Some(found) = read(body, spool, complete) {
                return Some(found);
            }
            if complete || broken {
                return None;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            let (held, timed_out) = self.shared.filled.wait_timeout(state, left).ok()?;
            if timed_out.timed_out() {
                return None;
            }
            state = held;
        }
    }

    /// Reaches the provider's own half of the spool, for anything it keeps there.
    pub fn with<T>(&self, read: impl FnOnce(&mut B, &mut Spool) -> T) -> Option<T> {
        let mut state = self.shared.state.lock().ok()?;
        let Buffered { spool, body, .. } = &mut *state;
        Some(read(body, spool))
    }

    pub fn done(&self) -> bool {
        self.shared
            .state
            .lock()
            .map(|state| state.complete)
            .unwrap_or(true)
    }

    pub fn failed(&self) -> Option<String> {
        self.shared.state.lock().ok()?.failed.clone()
    }

    pub fn downgrade(&self) -> WeakStream<B> {
        WeakStream {
            shared: Arc::downgrade(&self.shared),
            arrived: self.arrived.clone(),
        }
    }
}

impl<B: Body> WeakStream<B> {
    pub fn upgrade(&self) -> Option<Stream<B>> {
        Some(Stream {
            shared: self.shared.upgrade()?,
            arrived: self.arrived.clone(),
        })
    }

    /// Waits until the download has ended, whether it finished or broke, without keeping it
    /// alive in the meantime. False when every strong handle went first.
    pub async fn finished(&mut self) -> bool {
        loop {
            match self.upgrade() {
                Some(stream) if stream.done() => return true,
                Some(_) => {}
                None => return false,
            }
            if self.arrived.changed().await.is_err() {
                return self.upgrade().is_some_and(|stream| stream.done());
            }
        }
    }
}

/// Pulls the body into the spool, one chunk at a time, and stops as soon as nothing is left
/// that could read it.
async fn pump<S: Source, B: Body>(
    mut source: S,
    weak: Weak<Shared<B>>,
    progress: watch::Sender<usize>,
) {
    let mut staged = Vec::new();
    loop {
        let chunk = match source.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                if let Some(shared) = weak.upgrade() {
                    shared.finish(None);
                }
                progress.send_modify(|_| {});
                return;
            }
            Err(error) => {
                log::warn!("playback: the stream broke: {error:#}");
                if let Some(shared) = weak.upgrade() {
                    shared.finish(Some(format!("{error:#}")));
                }
                progress.send_modify(|_| {});
                return;
            }
        };
        let Some(shared) = weak.upgrade() else {
            return;
        };
        let filled = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            let Buffered { spool, body, .. } = &mut *state;
            let ceiling = body.ceiling();
            if spool
                .len()
                .checked_add(chunk.len())
                .is_none_or(|end| end > ceiling)
            {
                drop(state);
                shared.finish(Some(format!("the track is longer than {ceiling} bytes")));
                progress.send_modify(|_| {});
                return;
            }
            staged.clear();
            body.feed(&chunk, &mut staged);
            if let Err(error) = spool.append(&staged) {
                drop(state);
                log::warn!("playback: cannot keep the track: {error}");
                shared.finish(Some(format!("cannot keep the track: {error}")));
                progress.send_modify(|_| {});
                return;
            }
            spool.len()
        };
        shared.filled.notify_all();
        progress.send(filled).ok();
    }
}

/// A cursor over a `Stream`. Reading past what has arrived blocks the caller until more does,
/// so a decoder simply waits out a slow connection.
pub struct Reader<B = Plain> {
    shared: Arc<Shared<B>>,
    /// The end of the part served from the front of the file, when this cursor is spliced.
    head: usize,
    /// Where the rest is served from. Zero for a plain cursor.
    from: usize,
    /// Where the cursor is, in what it presents rather than in the file.
    at: u64,
}

impl<B: Body> Reader<B> {
    /// The byte in the file that `at` presents, which differs only past the head of a splice.
    fn place(&self, at: u64) -> u64 {
        match at < self.head as u64 {
            true => at,
            false => self.from as u64 + (at - self.head as u64),
        }
    }

    /// A length in the file, as this cursor presents it.
    fn presented(&self, length: usize) -> u64 {
        self.head as u64 + length.saturating_sub(self.from) as u64
    }

    /// The end of the track: the announced length, or the final length once the download ends.
    /// A server that announced nothing makes this wait for the last byte.
    fn end(&self) -> u64 {
        let Ok(mut state) = self.shared.state.lock() else {
            return 0;
        };
        loop {
            if let Some(total) = state.total {
                return self.presented(total as usize);
            }
            if state.complete {
                return self.presented(state.spool.len());
            }
            let Ok((held, timed_out)) = self.shared.filled.wait_timeout(state, PATIENCE) else {
                return 0;
            };
            if timed_out.timed_out() {
                return self.presented(held.spool.len());
            }
            state = held;
        }
    }
}

impl<B: Body> Read for Reader<B> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut state = self.shared.held()?;
        loop {
            let Buffered {
                spool,
                body,
                complete,
                total,
                ..
            } = &mut *state;
            let limit = self.presented(body.limit(spool, *complete));
            if self.at < limit {
                // A read never crosses a splice: the bytes on either side of it are nowhere
                // near each other in the file.
                let wanted = match self.at < self.head as u64 {
                    true => out.len().min(self.head - self.at as usize),
                    false => out.len(),
                };
                let start = self.place(self.at) as usize;
                let end = (start + wanted).min(self.place(limit) as usize);
                body.ready(spool, start..end)?;
                spool.read_at(start, &mut out[..end - start])?;
                self.at += (end - start) as u64;
                return Ok(end - start);
            }
            let ending = total
                .map(|total| self.presented(total as usize))
                .is_some_and(|total| self.at + body.tail() >= total && self.at < total);
            if let Some(failed) = &state.failed {
                return Err(io::Error::other(failed.clone()));
            }
            if state.complete || ending {
                return Ok(0);
            }
            let (held, timed_out) = self
                .shared
                .filled
                .wait_timeout(state, PATIENCE)
                .map_err(|_| io::Error::other("the track buffer is poisoned"))?;
            if timed_out.timed_out() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the track stopped arriving",
                ));
            }
            state = held;
        }
    }
}

impl<B: Body> Seek for Reader<B> {
    /// Free: nothing is waited on or made ready until something is read from the new position.
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::Current(delta) => i128::from(self.at) + i128::from(delta),
            SeekFrom::End(delta) => i128::from(self.end()) + i128::from(delta),
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot seek before the start",
            ));
        }
        self.at = target as u64;
        Ok(self.at)
    }
}

/// An announced body length as a size, when it fits under `ceiling`.
fn announced(total: Option<u64>, ceiling: usize) -> Option<usize> {
    total
        .and_then(|total| usize::try_from(total).ok())
        .filter(|total| *total <= ceiling)
}

/// A file in Sonora's cache directory with no name, so nothing is left behind however the
/// process ends.
fn spool_file() -> io::Result<File> {
    let folder = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora");
    std::fs::create_dir_all(&folder)?;
    tempfile::tempfile_in(folder)
}

#[cfg(unix)]
fn read_exact_at(file: &File, out: &mut [u8], at: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, out, at)
}

#[cfg(unix)]
fn write_all_at(file: &File, bytes: &[u8], at: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, bytes, at)
}

/// Every caller holds the spool's lock, so moving the shared cursor cannot race another read.
#[cfg(not(unix))]
fn read_exact_at(mut file: &File, out: &mut [u8], at: u64) -> io::Result<()> {
    file.seek(SeekFrom::Start(at))?;
    file.read_exact(out)
}

#[cfg(not(unix))]
fn write_all_at(mut file: &File, bytes: &[u8], at: u64) -> io::Result<()> {
    use std::io::Write as _;
    file.seek(SeekFrom::Start(at))?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream without a response behind it, so the spool can be driven by hand.
    fn stream<B: Body>(body: B, total: Option<u64>) -> Stream<B> {
        let shared = Arc::new(Shared {
            state: Mutex::new(Buffered {
                spool: Spool::in_memory(total),
                total,
                complete: false,
                failed: None,
                body,
            }),
            filled: Condvar::new(),
        });
        let (_, arrived) = watch::channel(0usize);
        Stream { shared, arrived }
    }

    fn feed<B: Body>(stream: &Stream<B>, chunk: &[u8]) {
        let mut state = stream.shared.state.lock().unwrap();
        let Buffered { spool, body, .. } = &mut *state;
        let mut staged = Vec::new();
        body.feed(chunk, &mut staged);
        spool.append(&staged).unwrap();
    }

    fn finish<B: Body>(stream: &Stream<B>, failed: Option<String>) {
        stream.shared.finish(failed);
    }

    #[test]
    fn a_read_serves_what_arrived_and_then_ends() {
        let stream = stream(Plain, Some(8));
        let mut reader = stream.reader();
        feed(&stream, &[1, 2, 3, 4]);
        finish(&stream, None);

        let mut out = [0u8; 8];
        assert_eq!(reader.read(&mut out).unwrap(), 4);
        assert_eq!(&out[..4], &[1, 2, 3, 4]);
        assert_eq!(reader.read(&mut out).unwrap(), 0, "the body is complete");
    }

    #[test]
    fn seeking_is_absolute_and_clamped_at_zero() {
        let stream = stream(Plain, Some(100));
        finish(&stream, None);
        let mut reader = stream.reader();
        assert_eq!(reader.seek(SeekFrom::Start(5)).unwrap(), 5);
        assert_eq!(reader.seek(SeekFrom::Current(3)).unwrap(), 8);
        assert_eq!(reader.seek(SeekFrom::End(-10)).unwrap(), 90);
        assert!(reader.seek(SeekFrom::Current(-200)).is_err());
    }

    #[test]
    fn a_broken_download_surfaces_as_an_error() {
        let stream = stream(Plain, Some(8));
        let mut reader = stream.reader();
        finish(&stream, Some("the connection dropped".to_owned()));
        assert!(reader.read(&mut [0u8; 8]).is_err());
    }

    /// A body that accounts for bytes before serving them, the way a CENC index does.
    struct Half;

    impl Body for Half {
        fn limit(&mut self, spool: &mut Spool, _complete: bool) -> usize {
            spool.len() / 2
        }
    }

    #[test]
    fn a_body_can_hold_back_what_it_has_not_accounted_for() {
        let stream = stream(Half, None);
        let mut reader = stream.reader();
        feed(&stream, &[1, 2, 3, 4, 5, 6]);
        let mut out = [0u8; 6];
        assert_eq!(reader.read(&mut out).unwrap(), 3);
        assert_eq!(&out[..3], &[1, 2, 3]);
    }

    /// A body that transforms what arrives, the way a stripe cipher does.
    struct Doubling;

    impl Body for Doubling {
        fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
            out.extend(chunk.iter().map(|byte| byte * 2));
        }

        fn flush(&mut self, out: &mut Vec<u8>) {
            out.push(255);
        }
    }

    #[test]
    fn a_body_shapes_the_bytes_as_they_arrive() {
        let stream = stream(Doubling, None);
        let mut reader = stream.reader();
        feed(&stream, &[1, 2, 3]);
        finish(&stream, None);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, vec![2, 4, 6, 255], "fed, then flushed");
    }

    /// Without this a decoder probing the end of the file would hold playback until the whole
    /// track had arrived. A plain body waits instead, which is why it is not the default.
    #[test]
    fn a_tail_read_does_not_wait_when_the_body_allows_it() {
        struct Probing;
        impl Body for Probing {
            fn tail(&self) -> u64 {
                8
            }
        }

        let stream = stream(Probing, Some(64));
        let mut reader = stream.reader();
        reader.seek(SeekFrom::End(-4)).unwrap();
        assert_eq!(reader.read(&mut [0u8; 4]).unwrap(), 0);
    }

    /// A spliced cursor presents the head of the file followed by a later part of it, which is
    /// what lets a decoder open at a position instead of reading its way there.
    #[test]
    fn a_spliced_cursor_joins_the_head_to_a_later_part() {
        let stream = stream(Plain, Some(10));
        feed(&stream, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        finish(&stream, None);

        let mut reader = stream.spliced(3, 7);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, vec![0, 1, 2, 7, 8, 9]);
        // And it reports its own length, not the file's.
        assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), 6);
    }
}
