//! A track downloaded while it plays. The decoder reads from the front of one buffer while the
//! response keeps filling the back, so playback starts after a short preroll instead of after
//! the whole file.
//!
//! Every provider that streams a file over HTTP uses this. What differs between them is only
//! what happens to the bytes, which is [`Body`]: Subsonic takes them as they come, Deezer
//! decrypts each Blowfish stripe as it lands, and Apple Music indexes CENC fragments and hands
//! each sample to a CDM the moment a reader asks for it. Everything else, the waiting and the
//! seeking and what a broken connection does, is the same for all three and lives here.

use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Duration;

use anyhow::{Result, bail};
use tokio::sync::watch;

/// How much has to be in before the decoder is let loose on the buffer. At any ordinary bitrate
/// this is several seconds of audio, and the download outruns playback many times over, so the
/// lead only grows from here.
pub const PREROLL: usize = 256 * 1024;

/// How long a read waits for bytes that have not arrived. rodio pulls the decoder on the audio
/// thread, so a wait there is silence: bounded, because a dead connection must not hold the
/// output for the rest of the session.
const PATIENCE: Duration = Duration::from_secs(20);

/// The most one track may buffer. A long album side is tens of megabytes, so this is only a
/// ceiling on what a wrong `Content-Length` or an endless body can cost.
const CEILING: usize = 256 * 1024 * 1024;

/// What a provider does to the body of one track: as it arrives, and before it is served.
///
/// Every method has an answer that suits a plain file, so a provider overrides only what it
/// actually does differently. `feed` and `flush` run on the tokio task pulling the response;
/// `limit`, `tail` and `ready` run on whichever thread is reading, under the buffer's lock.
pub trait Body: Send + 'static {
    /// What to append for one chunk of the response. The default appends it unchanged.
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(chunk);
    }

    /// The response ended: append anything that was held back waiting for more.
    fn flush(&mut self, _out: &mut Vec<u8>) {}

    /// How far into the buffer a read may be served. The default serves everything that has
    /// arrived; a provider that must account for bytes before handing them over answers less.
    ///
    /// The buffer is mutable because accounting for bytes can mean preparing them: an fMP4 has
    /// its sample entry relabelled once its samples can be decrypted.
    fn limit(&mut self, buf: &mut [u8], _complete: bool) -> usize {
        buf.len()
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
    fn ready(&mut self, _buf: &mut [u8], _range: Range<usize>) -> io::Result<()> {
        Ok(())
    }
}

/// A body that is already what it should be.
pub struct Plain;

impl Body for Plain {}

struct Buffered<B> {
    /// The track so far, as a reader would see it.
    buf: Vec<u8>,
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
            let Buffered { buf, body, .. } = &mut *state;
            body.flush(buf);
            if state.failed.is_none() {
                state.failed = failed;
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

impl<B> Clone for Stream<B> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            arrived: self.arrived.clone(),
        }
    }
}

impl<B: Body> Stream<B> {
    /// Starts pulling `response` into a buffer and returns at once. The download carries on in
    /// the background for as long as any reader or clone is alive; it stops on its own once all
    /// of them are gone.
    pub fn new(response: reqwest::Response, body: B) -> Self {
        let total = response.content_length();
        let shared = Arc::new(Shared {
            state: Mutex::new(Buffered {
                buf: Vec::new(),
                total,
                complete: false,
                failed: None,
                body,
            }),
            filled: Condvar::new(),
        });
        let (progress, arrived) = watch::channel(0usize);
        tokio::spawn(pump(response, Arc::downgrade(&shared), progress));
        Self { shared, arrived }
    }

    /// Starts the download and waits for the preroll, or for the whole body of a track shorter
    /// than that.
    pub async fn open(response: reqwest::Response, body: B) -> Result<Self> {
        let mut stream = Self::new(response, body);
        let wanted = stream.total().map_or(PREROLL, |total| {
            usize::try_from(total).unwrap_or(usize::MAX).min(PREROLL)
        });
        stream.wait_for(wanted).await?;
        Ok(stream)
    }

    /// Waits until `wanted` bytes have arrived, or the download ends. Nothing here blocks a
    /// thread: this is the async side of the same buffer the readers wait on.
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
            .or_else(|| state.complete.then_some(state.buf.len() as u64))
    }

    /// How much has arrived so far.
    pub fn arrived(&self) -> usize {
        self.shared
            .state
            .lock()
            .map(|state| state.buf.len())
            .unwrap_or(0)
    }

    /// Reaches the provider's own half of the buffer, for anything it keeps there.
    pub fn with<T>(&self, read: impl FnOnce(&mut B, &mut Vec<u8>) -> T) -> Option<T> {
        let mut state = self.shared.state.lock().ok()?;
        let Buffered { buf, body, .. } = &mut *state;
        Some(read(body, buf))
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
}

/// Pulls the response body into the buffer, one chunk at a time, and stops as soon as nothing
/// is left that could read it.
async fn pump<B: Body>(
    mut response: reqwest::Response,
    weak: Weak<Shared<B>>,
    progress: watch::Sender<usize>,
) {
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                if let Some(shared) = weak.upgrade() {
                    shared.finish(None);
                }
                progress.send_modify(|_| {});
                return;
            }
            Err(error) => {
                log::warn!("playback: the stream broke: {error}");
                if let Some(shared) = weak.upgrade() {
                    shared.finish(Some(error.to_string()));
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
            let Buffered { buf, body, .. } = &mut *state;
            if buf.len() + chunk.len() > CEILING {
                drop(state);
                shared.finish(Some(format!("the track is longer than {CEILING} bytes")));
                return;
            }
            body.feed(&chunk, buf);
            buf.len()
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
                return self.presented(state.buf.len());
            }
            let Ok((held, timed_out)) = self.shared.filled.wait_timeout(state, PATIENCE) else {
                return 0;
            };
            if timed_out.timed_out() {
                return self.presented(held.buf.len());
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
                buf,
                body,
                complete,
                total,
                ..
            } = &mut *state;
            let limit = self.presented(body.limit(buf, *complete));
            if self.at < limit {
                // A read never crosses a splice: the bytes on either side of it are nowhere
                // near each other in the file.
                let wanted = match self.at < self.head as u64 {
                    true => out.len().min(self.head - self.at as usize),
                    false => out.len(),
                };
                let start = self.place(self.at) as usize;
                let end = (start + wanted).min(self.place(limit) as usize);
                body.ready(buf, start..end)?;
                out[..end - start].copy_from_slice(&buf[start..end]);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream without a response behind it, so the buffer can be driven by hand.
    fn stream<B: Body>(body: B, total: Option<u64>) -> Stream<B> {
        let shared = Arc::new(Shared {
            state: Mutex::new(Buffered {
                buf: Vec::new(),
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
        let Buffered { buf, body, .. } = &mut *state;
        body.feed(chunk, buf);
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
        fn limit(&mut self, buf: &mut [u8], _complete: bool) -> usize {
            buf.len() / 2
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
