//! An Apple Music track that decrypts as it is read, over bytes that are still arriving.
//!
//! The waiting, the seeking and the splice are [`crate::stream`]'s, the same buffer every other
//! provider streams through. What is Apple's own is the [`Cenc`] body: each `moof`/`mdat` pair
//! is indexed as it completes, and a sample is handed to the CDM the moment a reader asks for
//! the bytes it covers. CENC is size preserving and every sample carries its own IV, so
//! decryption happens in place and in any order, and playback starts on the first fragment
//! rather than the last.
//!
//! Decryption runs on whichever thread reads, which is the engine's audio thread and never a
//! tokio worker or the output callback. A sample costs about a millisecond, so one second of
//! audio costs fifty, well inside what the output has queued.

use std::io;
use std::ops::Range;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use widevine::Cdm;
use widevine::cenc::{self, Encrypted, Sample, Step};

use crate::stream::{Body, Reader, Stream};

/// How much has to be in past the init segment before the decoder is let loose.
const PREROLL: usize = 256 * 1024;

/// Reads that start this far from the end never wait. A decoder probing for the end of the file
/// would otherwise hold playback until the whole track had downloaded.
const TAIL: u64 = 8 * 1024;

/// The content keys for one track, held for as long as anything can still read it.
struct Keys {
    cdm: Cdm,
    key_id: Vec<u8>,
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
    /// Where each fragment starts, in the file and on the media timeline. This is what turns a
    /// position into a place to start reading, so a seek needs no winding.
    fragments: Vec<Fragment>,
    samples: Vec<Sample>,
    /// Per sample, in `samples` order. A sample must be decrypted exactly once: running
    /// ciphertext through the cipher twice would corrupt it.
    cleared: Vec<bool>,
    keys: Option<Keys>,
    /// How many samples have gone through the CDM, and how long that took. Playback pays this
    /// as it reads, so when a track is slow to start these two numbers say whether the CDM is
    /// the reason.
    spent: (usize, Duration),
}

impl Cenc {
    /// Indexes whatever has arrived since the last walk. Cheap when nothing has: one box header
    /// is read and found to be incomplete.
    fn index(&mut self, buf: &mut [u8]) {
        let arrived = buf.len();
        if self.init_end.is_none() {
            let Some(init) = cenc::read_init(&buf[..arrived]) else {
                return;
            };
            log::debug!(
                "apple: init segment is {} bytes, {} encrypted track(s)",
                init.end,
                init.tracks.len()
            );
            // The relabel is applied once and never taken off: the fragment walk only ever
            // moves forward, so nothing reads the sample entry again.
            cenc::unlock(buf, &init);
            self.walked = init.end;
            self.init_end = Some(init.end);
            self.timescale = init.timescale;
            self.tracks = init.tracks;
        }
        while self.walked < arrived {
            match cenc::read_fragment(&buf[..arrived], self.walked, &self.tracks) {
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
                    self.cleared
                        .resize(self.samples.len() + samples.len(), false);
                    self.samples.extend(samples);
                    self.walked = next;
                }
                Step::Other { next } => self.walked = next,
                Step::Partial => break,
            }
        }
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
    fn limit(&mut self, buf: &mut [u8], complete: bool) -> usize {
        self.index(buf);
        match complete {
            true => buf.len(),
            false => self.walked,
        }
    }

    fn tail(&self) -> u64 {
        TAIL
    }

    /// Decrypts every sample overlapping the range that is not cleartext yet. Bytes outside any
    /// sample are framing, and cost nothing.
    fn ready(&mut self, buf: &mut [u8], range: Range<usize>) -> io::Result<()> {
        let Self {
            samples,
            cleared,
            keys,
            spent,
            ..
        } = self;
        // Samples are sorted and disjoint, so the first that can overlap is a search away.
        let mut index = samples.partition_point(|sample| sample.end() <= range.start);
        if index >= samples.len() || samples[index].start >= range.end {
            return Ok(());
        }
        let Some(keys) = keys.as_ref() else {
            return Err(io::Error::other("the track has no license loaded"));
        };
        while index < samples.len() && samples[index].start < range.end {
            if !cleared[index] {
                let sample = &samples[index];
                let span = sample.start..sample.end();
                let Some(bytes) = buf.get(span.clone()) else {
                    return Err(io::Error::other("a sample runs past the track"));
                };
                let began = Instant::now();
                let clear = keys
                    .cdm
                    .decrypt(bytes, &keys.key_id, &sample.iv, &sample.subs)
                    .map_err(|error| io::Error::other(format!("{error:#}")))?;
                spent.0 += 1;
                spent.1 += began.elapsed();
                if clear.len() != span.len() {
                    return Err(io::Error::other(
                        "the widevine cdm returned a sample of the wrong length",
                    ));
                }
                buf[span].copy_from_slice(&clear);
                cleared[index] = true;
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
    /// Starts pulling the response. Nothing can be decrypted until [`license`](Self::license)
    /// supplies the keys, so the license exchange can run while the body is arriving.
    pub fn new(response: reqwest::Response) -> Self {
        Self(Stream::new(response, Cenc::default()))
    }

    /// Hands the track its content keys. Until this lands, a read that needs a sample fails
    /// rather than waiting, so it is called before any reader exists.
    ///
    /// Nothing is decrypted here or in the background. A read decrypts what it lands on, and a
    /// seek splices straight to its own fragment rather than reading its way there.
    pub fn license(&self, cdm: Cdm, key_id: Vec<u8>) -> Result<()> {
        let installed = self.0.with(|cenc, _| {
            cenc.keys = Some(Keys { cdm, key_id });
        });
        match installed {
            Some(()) => Ok(()),
            None => bail!("the track buffer is poisoned"),
        }
    }

    /// Waits for the init segment and the preroll behind it, so the decoder opens against a
    /// cushion rather than an empty buffer.
    pub async fn prime(&mut self) -> Result<()> {
        loop {
            let (wanted, arrived) = self
                .0
                .with(|cenc, buf| {
                    cenc.index(buf);
                    (cenc.init_end.map(|end| end + PREROLL), buf.len())
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
            .with(|cenc, buf| (cenc.samples.len(), buf.len()))
            .unwrap_or_default();
        log::info!(
            "apple: prebuffer ready, {} KiB in, {samples} samples indexed",
            indexed / 1024
        );
        Ok(())
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

    /// Where to start reading to land on `at`, if the fragment covering it is indexed.
    pub fn entry(&self, at: Duration) -> Option<Entry> {
        let complete = self.0.done();
        self.0.with(|cenc, buf| {
            cenc.index(buf);
            cenc.entry(at, complete)
        })?
    }

    /// The init segment, once it has arrived: `ftyp` and `moov`, with the sample entry already
    /// relabelled. It carries the edit list, which is what says where the music starts.
    pub fn header(&self) -> Option<Vec<u8>> {
        self.0.with(|cenc, buf| {
            let end = cenc.init_end?;
            buf.get(..end).map(<[u8]>::to_vec)
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
                subs: Vec::new(),
            }],
            cleared: vec![false],
            ..Cenc::default()
        };
        let mut buf = vec![7u8; 64];
        assert!(cenc.ready(&mut buf, 0..8).is_ok(), "framing needs no keys");
        assert!(cenc.ready(&mut buf, 0..32).is_err(), "a sample does");
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
