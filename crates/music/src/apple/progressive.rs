//! An Apple Music track that decrypts as it is read, over bytes that are still arriving.
//!
//! The waiting, the seeking and the splice are [`crate::stream`]'s, the same spool every other
//! provider streams through. What is Apple's own is the [`Cenc`] body: each `moof`/`mdat` pair
//! is indexed as it completes, and a sample is handed to the CDM the moment a reader asks for
//! the bytes it covers. CENC is size preserving and every sample carries its own IV, so
//! decryption happens in place and in any order, and playback starts on the first fragment
//! rather than the last.
//!
//! Decryption runs on whichever thread reads, which is the engine's audio thread and never a
//! tokio worker or the output callback. A sample costs about one and a half milliseconds, nearly
//! all of it inside the CDM rather than on the way to its host process, so one second of audio
//! costs about seventy, well inside what the output has queued.
//!
//! The track is spooled in memory rather than to a file, because samples are decrypted in place
//! and cleartext must never reach the disk.
//!
//! Once the download is complete, a background pass decrypts every sample no read has reached
//! and then gives the license back. The CDM lives in a host process that exits when no track
//! holds a license, so a track that is paused, queued or left behind by another provider does
//! not keep it running.

use std::io;
use std::ops::Range;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use widevine::Cdm;
use widevine::cenc::{self, Encrypted, Sample, Step};

use crate::stream::{Body, Reader, Source, Spool, Stream, WeakStream};

/// How much has to be in past the init segment before the decoder is let loose.
const PREROLL: usize = 256 * 1024;

/// The longest a box header can be: a size, a kind and a 64-bit size behind them.
const BOX_HEADER: usize = 16;

/// The pause between two samples of the background pass. It leaves the CDM and the track free
/// for a moment, so a read on the audio thread never queues behind a run of them.
const PACE: Duration = Duration::from_micros(500);

/// The fewest bytes a sample is assumed to hold when sizing the index, which caps what a
/// first fragment of implausibly small samples can make it reserve.
const MIN_SAMPLE: usize = 64;

/// Reads that start this far from the end never wait. A decoder probing for the end of the file
/// would otherwise hold playback until the whole track had downloaded.
const TAIL: u64 = 8 * 1024;

/// The content keys for one track, held until every sample in it is cleartext.
struct Keys {
    cdm: Cdm,
    key_id: Vec<u8>,
}

/// What one sample in the spool holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Seal {
    #[default]
    Encrypted,
    Clear,
    /// Its cleartext could not be written back, so the spool may hold a mix of the two that
    /// can be neither served nor decrypted again.
    Lost,
}

/// Where a track is with its license.
#[derive(Default)]
enum License {
    /// The exchange has not finished yet.
    #[default]
    Pending,
    Held(Keys),
    /// Every sample was decrypted and the keys went back. Nothing is left to decrypt, so a
    /// read that still finds ciphertext is a bug.
    Released,
}

/// What one step of the background pass did.
enum Pass {
    Cleared,
    /// Nothing was left to decrypt, and these are the keys that were let go. They are dropped
    /// outside the track's lock.
    Finished(Option<Keys>),
}

/// One `moof`/`mdat` pair: where it is in the file, and where it starts in the music.
#[derive(Clone, Copy, Debug)]
struct Fragment {
    at: usize,
    start: Duration,
}

/// Where to start reading a track, and what part of the first fragment to throw away to land
/// exactly on the position that was asked for.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The byte the fragments should start from, spliced behind the init segment.
    graft: usize,
    /// The end of the init segment, which is what gets spliced in front of it.
    head: usize,
    /// How far into that fragment the wanted position is.
    pub into: Duration,
}

/// The encrypted half of an Apple Music track: what has been accounted for, and the keys that
/// turn it into sound.
#[derive(Default)]
pub struct Cenc {
    /// One past the end of the init segment, once `moov` has arrived whole.
    init_end: Option<usize>,
    /// What the init segment said about each encrypted track, which every later fragment is
    /// read against.
    tracks: Vec<Encrypted>,
    /// Ticks per second of the media timeline, from the init segment.
    timescale: u32,
    /// One past the last byte accounted for: every byte below this is either cleartext framing
    /// or a sample whose IV is known, which is what makes it safe to serve.
    walked: usize,
    /// How much of the file has to be in before the walk can take another step. Every read
    /// asks for a walk, so this is what keeps the asking free while a fragment is arriving.
    awaited: usize,
    /// Where each fragment starts, in the file and on the media timeline. This is what turns a
    /// position into a place to start reading, so a seek needs no winding.
    fragments: Vec<Fragment>,
    samples: Vec<Sample>,
    /// Per sample, in `samples` order. A sample must be decrypted exactly once: running
    /// cleartext through the cipher again would corrupt it.
    seals: Vec<Seal>,
    license: License,
    /// Holds one sample on its way through the CDM. It is kept between samples, so decrypting
    /// a track allocates once rather than once a sample.
    scratch: Vec<u8>,
    /// How many samples have gone through the CDM, and how long that took. Playback pays this
    /// as it reads, so when a track is slow to start these two numbers say whether the CDM is
    /// the reason.
    spent: (usize, Duration),
}

impl Cenc {
    /// Indexes whatever has arrived since the last walk. Free when the box the walk stopped at
    /// is still short, and otherwise one box header read per box plus each whole fragment.
    fn index(&mut self, spool: &mut Spool) {
        let arrived = spool.len();
        if arrived < self.awaited {
            return;
        }
        if self.init_end.is_none() && !self.init(spool) {
            return;
        }
        while self.walked < arrived {
            let Some((kind, end)) = self.whole(spool, self.walked) else {
                break;
            };
            // A fragment is read with the box behind it, which is its mdat when the file is
            // well formed, so the parser sees the pair it expects.
            let end = match &kind {
                b"moof" => match self.whole(spool, end) {
                    Some((_, behind)) => behind,
                    None => break,
                },
                _ => end,
            };
            let Ok(window) = spool.view(self.walked..end) else {
                break;
            };
            match cenc::read_fragment(&window, self.walked, &self.tracks) {
                Step::Fragment {
                    samples,
                    next,
                    decode,
                } => {
                    if let Some(start) = self.moment(decode) {
                        self.fragments.push(Fragment {
                            at: self.walked,
                            start,
                        });
                    }
                    if self.samples.is_empty() {
                        self.reserve(spool.announced(), samples.len(), next - self.walked);
                    }
                    self.seals
                        .resize(self.samples.len() + samples.len(), Seal::Encrypted);
                    self.samples.extend(samples);
                    self.walked = next;
                }
                Step::Other { next } => self.walked = next,
                Step::Partial => break,
            }
        }
    }

    /// Makes room for every sample of the track, going by how densely the first fragment, of
    /// `span` bytes, packed its `found` samples. A track of unknown length is left to grow.
    ///
    /// Growing the index a fragment at a time would leave a trail of freed, ever larger copies
    /// in the heap. A guess on the high side costs only address space, since an index this big
    /// is mapped and the pages it never fills are never touched.
    fn reserve(&mut self, total: Option<usize>, found: usize, span: usize) {
        let (Some(total), Some(start)) = (total, self.init_end) else {
            return;
        };
        let fragments = total.saturating_sub(start).div_ceil(span.max(1));
        let expected = found.saturating_mul(fragments + 1).min(total / MIN_SAMPLE);
        self.samples.reserve_exact(expected);
        self.seals.reserve_exact(expected);
    }

    /// Reads the init segment once `moov` has arrived whole, relabels its sample entry in the
    /// spool, and starts the fragment walk behind it. False while `moov` is still arriving.
    fn init(&mut self, spool: &mut Spool) -> bool {
        let mut at = 0;
        let end = loop {
            let Some((kind, end)) = self.whole(spool, at) else {
                return false;
            };
            if &kind == b"moov" {
                break end;
            }
            at = end;
        };
        let Ok(mut front) = spool.bytes(0..end) else {
            return false;
        };
        let Some(init) = cenc::read_init(&front) else {
            return false;
        };
        log::debug!(
            "apple: init segment is {} bytes, {} encrypted track(s)",
            init.end,
            init.tracks.len()
        );
        // The relabel is applied once and never taken off: the fragment walk only ever
        // moves forward, so nothing reads the sample entry again.
        cenc::unlock(&mut front, &init);
        if let Err(error) = spool.write_at(0, &front) {
            log::warn!("apple: cannot relabel the sample entry: {error}");
            return false;
        }
        self.walked = init.end;
        self.init_end = Some(init.end);
        self.timescale = init.timescale;
        self.tracks = init.tracks;
        true
    }

    /// The kind and end of the box at `at`, once all of it has arrived. Short of that it
    /// records how much has to arrive first, so the walk is not retried before then.
    fn whole(&mut self, spool: &Spool, at: usize) -> Option<([u8; 4], usize)> {
        let arrived = spool.len();
        let mut head = [0u8; BOX_HEADER];
        let read = BOX_HEADER.min(arrived.saturating_sub(at));
        spool.read_at(at, &mut head[..read]).ok()?;
        let Some((kind, total)) = cenc::extent(&head[..read]) else {
            if read < BOX_HEADER {
                self.awaited = at + BOX_HEADER;
            }
            return None;
        };
        let end = at.checked_add(total)?;
        if end > arrived {
            self.awaited = end;
            return None;
        }
        Some((kind, end))
    }

    /// Decrypts sample `index` in place in the spool, unless it is cleartext already. The
    /// returned time is what the CDM took, or zero when there was nothing to do.
    fn clear(&mut self, spool: &mut Spool, index: usize) -> io::Result<Duration> {
        match self.seals[index] {
            Seal::Encrypted => {}
            Seal::Clear => return Ok(Duration::ZERO),
            Seal::Lost => {
                return Err(io::Error::other(
                    "a sample was lost when its cleartext could not be kept",
                ));
            }
        }
        let keys = match &self.license {
            License::Held(keys) => keys,
            License::Pending => return Err(io::Error::other("the track has no license loaded")),
            License::Released => {
                log::error!(
                    "apple: sample {index} of {} is still encrypted after the license was released",
                    self.samples.len()
                );
                return Err(io::Error::other(
                    "a sample is still encrypted after the license was released",
                ));
            }
        };
        let sample = &self.samples[index];
        let span = sample.start..sample.end();
        let bytes = &mut self.scratch;
        bytes.clear();
        bytes.resize(span.len(), 0);
        if spool.read_at(span.start, bytes).is_err() {
            return Err(io::Error::other("a sample runs past the track"));
        }
        let began = Instant::now();
        keys.cdm
            .decrypt(bytes, &keys.key_id, &sample.iv, &sample.subs)
            .map_err(|error| io::Error::other(format!("{error:#}")))?;
        let took = began.elapsed();
        if let Err(error) = spool.write_at(span.start, bytes) {
            self.seals[index] = Seal::Lost;
            log::error!("apple: cannot keep the cleartext of sample {index}: {error}");
            return Err(error);
        }
        self.seals[index] = Seal::Clear;
        Ok(took)
    }

    /// One step of the background pass: decrypts the first sample at or after `next` that no
    /// read has reached, or releases the license when there is none. Only called once the
    /// download is complete, when the index can no longer grow.
    fn pass(&mut self, spool: &mut Spool, next: &mut usize) -> io::Result<Pass> {
        self.index(spool);
        let found = (*next..self.seals.len()).find(|&index| self.seals[index] == Seal::Encrypted);
        let Some(index) = found else {
            let keys = match std::mem::replace(&mut self.license, License::Released) {
                License::Held(keys) => Some(keys),
                License::Pending | License::Released => None,
            };
            return Ok(Pass::Finished(keys));
        };
        self.clear(spool, index)?;
        *next = index + 1;
        Ok(Pass::Cleared)
    }

    /// A fragment's decode time as a position, once the timescale is known.
    fn moment(&self, ticks: Option<u64>) -> Option<Duration> {
        let ticks = ticks?;
        (self.timescale > 0)
            .then(|| Duration::from_secs_f64(ticks as f64 / f64::from(self.timescale)))
    }

    /// Where to start reading to land on `at`. `None` when the fragment covering it has not
    /// been indexed yet, which leaves the caller to wind there the slow way.
    fn entry(&self, at: Duration, complete: bool) -> Option<Entry> {
        let last = self.fragments.last()?;
        // Past the end of what is indexed the answer would be the last fragment, which is not
        // where the listener asked to be.
        if at > last.start && !complete {
            return None;
        }
        let found = self
            .fragments
            .iter()
            .rev()
            .find(|fragment| fragment.start <= at)?;
        Some(Entry {
            graft: found.at,
            head: self.init_end?,
            into: at.saturating_sub(found.start),
        })
    }
}

impl Body for Cenc {
    /// Everything below the walk is either framing or a sample whose IV is known. Once the body
    /// is complete there is nothing more coming, so whatever is left is served as it is.
    fn limit(&mut self, spool: &mut Spool, complete: bool) -> usize {
        self.index(spool);
        match complete {
            true => spool.len(),
            false => self.walked,
        }
    }

    fn tail(&self) -> u64 {
        TAIL
    }

    /// Samples are decrypted in place, so the whole track stays in memory.
    fn confidential(&self) -> bool {
        true
    }

    /// Decrypts every sample overlapping the range that is not cleartext yet. Bytes outside any
    /// sample are framing, and cost nothing.
    fn ready(&mut self, spool: &mut Spool, range: Range<usize>) -> io::Result<()> {
        // Samples are sorted and disjoint, so the first that can overlap is a search away.
        let mut index = self
            .samples
            .partition_point(|sample| sample.end() <= range.start);
        while index < self.samples.len() && self.samples[index].start < range.end {
            if self.seals[index] != Seal::Clear {
                let took = self.clear(spool, index)?;
                self.spent.0 += 1;
                self.spent.1 += took;
            }
            index += 1;
        }
        Ok(())
    }
}

/// One Apple Music track being downloaded and decrypted. Clones share the download, the license
/// and the index, so a track can be lined up behind the one playing and read twice without
/// paying for any of it twice.
#[derive(Clone)]
pub struct Media(Stream<Cenc>);

impl Media {
    /// Starts pulling the encrypted file from `source`, which is `total` bytes long when that is
    /// known. Nothing can be decrypted until [`license`](Self::license) supplies the keys, so
    /// the license exchange can run while the body is arriving.
    pub(crate) fn pulling(source: impl Source, total: Option<u64>) -> Self {
        Self(Stream::pulling(source, total, Cenc::default()))
    }

    /// How much of the encrypted file has arrived, from 0 to 1, when its length is known.
    pub(crate) fn progress(&self) -> Option<f32> {
        let total = self.0.total().filter(|total| *total > 0)?;
        Some(self.0.arrived() as f32 / total as f32)
    }

    /// Hands the track its content keys. Until this lands, a read that needs a sample fails
    /// rather than waiting, so it is called before any reader exists.
    ///
    /// A read decrypts what it lands on, and a seek splices straight to its own fragment rather
    /// than reading its way there. Once the download is complete a background pass decrypts
    /// the rest and gives the keys back, see [`Self::release_when_downloaded`].
    pub fn license(&self, cdm: Cdm, key_id: Vec<u8>) -> Result<()> {
        let installed = self.0.with(|cenc, _| {
            cenc.license = License::Held(Keys { cdm, key_id });
        });
        match installed {
            Some(()) => Ok(()),
            None => bail!("the track buffer is poisoned"),
        }
    }

    /// Once the download is complete, decrypts every sample no read has reached on a blocking
    /// thread and then drops the track's license, which lets the CDM host exit. Call it once
    /// per download, after [`Self::license`]. It holds the track only weakly, so a track
    /// dropped meanwhile stops the pass and is not kept downloading.
    pub fn release_when_downloaded(&self) {
        let mut weak = self.0.downgrade();
        tokio::spawn(async move {
            if !weak.finished().await {
                return;
            }
            let passed = tokio::task::spawn_blocking(move || clear_rest(&weak)).await;
            if let Err(error) = passed {
                log::warn!("apple: the decrypt pass stopped: {error}");
            }
        });
    }

    /// Waits for the init segment and the preroll behind it, so the decoder opens against a
    /// cushion rather than an empty buffer.
    pub async fn prime(&mut self) -> Result<()> {
        loop {
            let (wanted, arrived) = self
                .0
                .with(|cenc, spool| {
                    cenc.index(spool);
                    (cenc.init_end.map(|end| end + PREROLL), spool.len())
                })
                .unwrap_or((None, 0));
            match wanted {
                Some(wanted) => {
                    self.0.wait_for(wanted).await?;
                    break;
                }
                // Nothing to measure the preroll from yet; take another chunk and look again.
                None if self.0.done() => bail!("the track carries no moov"),
                None => self.0.wait_for(arrived + 1).await?,
            }
        }
        let (samples, indexed) = self
            .0
            .with(|cenc, spool| (cenc.samples.len(), spool.len()))
            .unwrap_or_default();
        log::info!(
            "apple: prebuffer ready, {} KiB in, {samples} samples indexed",
            indexed / 1024
        );
        Ok(())
    }

    /// Waits until the download has ended, whether it finished or broke.
    pub async fn finished(&self) {
        self.0.finished().await;
    }

    /// A cursor over the whole track.
    pub fn reader(&self) -> Reader<Cenc> {
        self.0.reader()
    }

    /// A cursor that starts at `entry`: the init segment, then the fragment covering the
    /// position. Nothing before it is read at all.
    pub fn reader_at(&self, entry: Entry) -> Reader<Cenc> {
        self.0.spliced(entry.head, entry.graft)
    }

    /// Where to start reading to land on `at`, waiting for the fragment that covers it when
    /// the download has not reached it yet.
    ///
    /// The wait is the point. Seeking past what has arrived is ordinary, and the alternative
    /// to waiting is answering nothing, which starts the track from the beginning instead of
    /// where the listener asked to be. A plain file waits for the same bytes inside the read
    /// that follows its seek; a fragmented one has to wait for the index first.
    pub fn entry(&self, at: Duration) -> Option<Entry> {
        self.0.awaiting(|cenc, spool, complete| {
            cenc.index(spool);
            cenc.entry(at, complete)
        })
    }

    /// The init segment, once it has arrived: `ftyp` and `moov`, with the sample entry already
    /// relabelled. It carries the edit list, which is what says where the music starts.
    pub fn header(&self) -> Option<Vec<u8>> {
        self.0.with(|cenc, spool| {
            let end = cenc.init_end?;
            spool.bytes(0..end).ok()
        })?
    }

    /// How many samples the CDM has decrypted for this track so far, and how long it spent.
    /// Read after the decoder opens, this says how much of the wait was decryption.
    pub fn spent(&self) -> (usize, Duration) {
        self.0.with(|cenc, _| cenc.spent).unwrap_or_default()
    }

    /// How many samples the track has indexed so far, decrypted or not.
    pub fn samples(&self) -> usize {
        self.0
            .with(|cenc, _| cenc.samples.len())
            .unwrap_or_default()
    }
}

/// Decrypts what is left of a complete track one sample at a time, letting go of the track
/// between samples, then releases its license. Stops early when every other handle on the
/// track is gone, or on a failure, which leaves the keys in place for the reads to use.
fn clear_rest(weak: &WeakStream<Cenc>) {
    let began = Instant::now();
    let mut next = 0;
    let mut count = 0usize;
    loop {
        let Some(stream) = weak.upgrade() else {
            return;
        };
        let step = stream.with(|cenc, spool| cenc.pass(spool, &mut next));
        drop(stream);
        match step {
            Some(Ok(Pass::Cleared)) => count += 1,
            Some(Ok(Pass::Finished(keys))) => {
                drop(keys);
                log::info!(
                    "apple: decrypted the {count} samples left in {:.1}s, license released",
                    began.elapsed().as_secs_f32()
                );
                return;
            }
            Some(Err(error)) => {
                log::warn!("apple: cannot decrypt ahead, keeping the license: {error}");
                return;
            }
            None => return,
        }
        std::thread::sleep(PACE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An encrypted sample cannot be served without keys, and the failure must be an error
    /// rather than ciphertext reaching the decoder.
    #[test]
    fn a_sample_without_a_license_is_refused() {
        let mut cenc = Cenc {
            walked: 64,
            samples: vec![Sample {
                start: 16,
                len: 16,
                iv: [0; 16],
                subs: Default::default(),
            }],
            seals: vec![Seal::Encrypted],
            ..Cenc::default()
        };
        let mut spool = Spool::in_memory(None);
        spool.append(&[7u8; 64]).unwrap();
        assert!(
            cenc.ready(&mut spool, 0..8).is_ok(),
            "framing needs no keys"
        );
        assert!(cenc.ready(&mut spool, 0..32).is_err(), "a sample does");
    }

    /// A position is answered with the last fragment that starts at or before it, and with
    /// nothing at all while the track past it is still arriving.
    #[test]
    fn a_position_finds_the_fragment_that_covers_it() {
        let cenc = Cenc {
            init_end: Some(1000),
            fragments: vec![
                Fragment {
                    at: 1000,
                    start: Duration::ZERO,
                },
                Fragment {
                    at: 5000,
                    start: Duration::from_secs(10),
                },
                Fragment {
                    at: 9000,
                    start: Duration::from_secs(20),
                },
            ],
            ..Cenc::default()
        };
        let entry = cenc.entry(Duration::from_secs(12), false).unwrap();
        assert_eq!(entry.graft, 5000);
        assert_eq!(entry.head, 1000);
        assert_eq!(entry.into, Duration::from_secs(2));
        // Past the indexed end, only a finished download can answer.
        assert!(cenc.entry(Duration::from_secs(30), false).is_none());
        assert_eq!(
            cenc.entry(Duration::from_secs(30), true).unwrap().graft,
            9000
        );
    }
}
