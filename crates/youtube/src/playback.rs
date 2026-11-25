//! Playback for YouTube Music: what [`engine`] needs that is YouTube's own.
//!
//! The track streams through [`sound::stream`] like every other provider's, fed by ytmusic's
//! ranged download, which asks again from the byte a stalled connection stopped at. The file is
//! DASH fMP4, so the decoder opens it unseekable and a seek splices the header onto the segment
//! the `sidx` names, the way Apple Music's does. A segment the download has not reached comes
//! from a [`ranged::Reach`] download of its own, started at the segment's first byte. The
//! threads, the queue, the preload, the gapless join and the loudness gain are the engine's.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use bytes::Bytes;
use engine::{self, Fetch, Loudness};
use music::vault::{Recording, Shelf};
use music::{Encoding, Offer, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sound::decode::Decoded;
use sound::ranged::{self, Address, Reach};
use sound::stream::{PREROLL, Plain, Source, Stream};
use sound::trim::{self, Trim};
use ytmusic::{AudioStream, Locator, YtMusic};

use crate::segments::Segments;

/// How long the stream metadata, and then the preroll, may each take before the attempt is
/// given up. The stream host has no timeout of its own, so a connection it dropped halfway
/// would otherwise hold the engine on a silent track forever.
const PATIENCE: Duration = Duration::from_secs(15);
/// The attempts a fetch gets before the track is reported unavailable.
const ATTEMPTS: u32 = 2;
/// The level the stream host measures `loudnessDb` against, in LUFS.
const REFERENCE_LUFS: f32 = -14.0;
/// How far past the download a seek may land and still wait for it rather than start another
/// download. The download asks for a mebibyte at a time at full speed, so a quarter of one
/// arrives about as soon as a new request would be answered.
const NEARBY: u64 = 256 * 1024;
/// The levels YouTube Music serves, best first. High is the best AAC stream there is, 256 kbps
/// for a Premium account and 128 kbps otherwise, normal is the 128 kbps one and low the 48 kbps
/// HE-AAC one.
pub(crate) const QUALITIES: &[Offer] = &[
    Offer {
        quality: Quality::High,
        encoding: Encoding::Aac { kbps: 256 },
        up_to: true,
    },
    Offer {
        quality: Quality::Normal,
        encoding: Encoding::Aac { kbps: 128 },
        up_to: false,
    },
    Offer {
        quality: Quality::Low,
        encoding: Encoding::Aac { kbps: 48 },
        up_to: false,
    },
];

/// A track arriving, with what the stream host said about its length and loudness and what its
/// header says about where the music is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    /// Where each segment starts. `None` for a file with no index, which a seek then has to
    /// decode its way through.
    segments: Option<Arc<Segments>>,
    /// What a seek past the download fetches its segment with. `None` for a file with no index
    /// and for a track from the vault, which is already whole.
    reach: Option<Arc<Reach<Plain>>>,
    /// The edit list, which says how much priming to drop and how long the music runs.
    edit: Option<Trim>,
    loudness_db: Option<f32>,
    duration: Option<Duration>,
    /// The format the stream host picked, `None` when it gave no bit rate.
    encoding: Option<Encoding>,
}

pub struct Factory {
    api: Arc<YtMusic>,
}

impl Factory {
    pub fn new(api: Arc<YtMusic>) -> Self {
        Self { api }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            YouTube {
                api: self.api.clone(),
                media: ranged::media("youtube", reqwest::Client::new),
                ceiling: ceiling(config.quality),
                shelf: config.vault.as_ref().map(|vault| vault.shelf("youtube")),
            },
            config,
        )
    }
}

struct YouTube {
    api: Arc<YtMusic>,
    /// The client a seek's own download asks the stream host with.
    media: reqwest::Client,
    /// The highest bitrate a stream is chosen at, in bits per second, or `None` for the best.
    ceiling: Option<u32>,
    /// Where played tracks are kept, so playing one again asks YouTube for nothing.
    shelf: Option<Shelf>,
}

/// What the stream host said about a track, kept beside it in the vault so a kept track plays
/// with the same length, loudness and signal path as a fresh one.
#[derive(Serialize, Deserialize)]
struct Facts {
    loudness_db: Option<f32>,
    duration_ms: Option<u64>,
    codec: String,
    kbps: u32,
}

/// YouTube's response for a track, read as the stream's source.
struct Download(AudioStream);

impl Source for Download {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        self.0.chunk().await
    }
}

/// What a seek's download asks for a fresh url when the one it has expires.
struct Renewal(Locator);

#[async_trait]
impl ranged::Renew for Renewal {
    /// A fresh url, which reaches the main download too since both share the locator, and the
    /// user agent of the client it came from.
    async fn url(&self) -> Result<(String, HeaderMap)> {
        self.0.renew().await?;
        Ok(request(&self.0))
    }
}

#[async_trait]
impl Fetch for YouTube {
    type Loaded = Loaded;

    fn name(&self) -> &'static str {
        "yt"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Opens the stream and waits for the preroll, giving a stalled attempt one more go before
    /// failing. Only a timeout is retried, since a refusal from the stream host is as final the
    /// second time.
    async fn load(&self, id: &str) -> Result<Loaded> {
        if let Some(loaded) = self.recall(id).await {
            return Ok(loaded);
        }
        let mut attempt = 1;
        loop {
            let opened = attempt_open(
                &self.api,
                &self.media,
                id,
                self.ceiling,
                self.shelf.as_ref(),
            );
            match opened.await {
                Err(error) if attempt < ATTEMPTS && error.is::<tokio::time::error::Elapsed>() => {
                    log::warn!("playback: {id} stalled, trying again: {error:#}");
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness_db.map(|db| Loudness {
            lufs: REFERENCE_LUFS + db,
            peak: None,
        })
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        loaded.encoding
    }

    fn gated(&self, error: &anyhow::Error) -> bool {
        error.downcast_ref::<ytmusic::SignInRequired>().is_some()
    }

    fn blocked(&self, error: &anyhow::Error) -> bool {
        error.is::<ytmusic::NetworkBlocked>()
    }

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        let total = loaded.stream.total().filter(|total| *total > 0)?;
        Some(loaded.stream.arrived() as f32 / total as f32)
    }

    /// A fragmented stream opened unseekable cannot move, so a seek opens a second decoder
    /// spliced to the target.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    /// Builds a decoder over the stream, trimmed to what the edit list says is heard, and
    /// places it at `at`.
    ///
    /// The decoder is told neither the length nor that it may seek. Told either, symphonia walks
    /// every top-level box of the file before the first packet, which waits for the whole
    /// download. Starting anywhere but the beginning splices the header onto the segment that
    /// covers the position, and the part of that segment before it is trimmed off the front. A
    /// segment well past the download is read from a download of its own started at its first
    /// byte.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Decoded> {
        let priming = loaded.edit.map(|edit| edit.skip).unwrap_or_default();
        let entry = match at.is_zero() {
            true => None,
            false => loaded
                .segments
                .as_ref()
                .and_then(|segments| segments.entry(at + priming)),
        };
        let open = |reader| ranged::forward("youtube", id, reader);
        let decoder = match (entry, &loaded.reach) {
            (Some(entry), Some(reach)) => {
                reach.exact(&loaded.stream, entry.head, entry.graft as u64, open)
            }
            (Some(entry), None) => open(loaded.stream.spliced(entry.head, entry.graft)),
            (None, _) => open(loaded.stream.reader()),
        }?;
        let take = loaded.edit.and_then(|edit| edit.take);
        // Without an index the only way to a position is through every sample before it.
        let (skip, take) = match entry {
            Some(entry) => (entry.into, None),
            None => (priming + at, take.map(|take| take.saturating_sub(at))),
        };
        Some(Decoded::trimmed(decoder, skip, take))
    }
}

impl YouTube {
    /// The track from the vault, when it was kept at the quality asked for. Nothing here
    /// reaches YouTube, so a kept track needs no player request and no token.
    async fn recall(&self, id: &str) -> Option<Loaded> {
        let kept = self
            .shelf
            .as_ref()?
            .find(id, &variant(self.ceiling))
            .await?;
        let facts = serde_json::from_slice::<Facts>(&kept.facts)
            .inspect_err(|error| {
                log::warn!("playback: the kept facts of {id} do not read: {error}")
            })
            .ok()?;
        let stream = Stream::pulling(kept.source, Some(kept.length), Plain)
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        let (segments, edit) = front(&stream);
        log::debug!("playback: {id} started from the vault");
        Some(Loaded {
            stream,
            segments: segments.map(Arc::new),
            reach: None,
            edit,
            loudness_db: facts.loudness_db,
            duration: facts.duration_ms.map(Duration::from_millis),
            encoding: served(&facts.codec, facts.kbps),
        })
    }
}

/// Opens the stream and waits for the preroll, each under its own deadline. With a shelf, the
/// track is kept in the vault once all of it has arrived. A file with a segment index gets a
/// reach, whose downloads ask the stream host through `media`.
async fn attempt_open(
    api: &Arc<YtMusic>,
    media: &reqwest::Client,
    id: &str,
    ceiling: Option<u32>,
    shelf: Option<&Shelf>,
) -> Result<Loaded> {
    let started = Instant::now();
    let (format, audio) = tokio::time::timeout(PATIENCE, api.open_audio_within(id, ceiling))
        .await
        .context("stream metadata timed out")??;
    let total = audio.total();
    let locator = Arc::new(Renewal(audio.locator()));
    let audio = Download(audio);
    let audio = match shelf {
        Some(shelf) => {
            let facts = Facts {
                loudness_db: format.loudness_db,
                duration_ms: format.duration.map(|duration| duration.as_millis() as u64),
                codec: format.codec.clone(),
                kbps: format.bitrate / 1000,
            };
            shelf.keep(
                id,
                &variant(ceiling),
                move || serde_json::to_vec(&facts).ok(),
                audio,
            )
        }
        None => Recording::through(audio),
    };
    let stream = tokio::time::timeout(PATIENCE, Stream::pulling(audio, total, Plain).primed())
        .await
        .context("stream preroll timed out")??;
    let (segments, edit) = front(&stream);
    log::debug!(
        "playback: {id} started, itag {} {} {} kbps, {:.1} MiB, {} segments, in {:?}",
        format.itag,
        format.codec,
        format.bitrate / 1000,
        total.unwrap_or_default() as f64 / (1024.0 * 1024.0),
        segments.as_ref().map_or(0, Segments::count),
        started.elapsed()
    );
    match edit {
        Some(edit) => log::debug!(
            "playback: {id} trims {:?} of priming, plays {:?}",
            edit.skip,
            edit.take
        ),
        None => log::debug!("playback: {id} carries no edit list"),
    }
    let reach = segments.is_some().then(|| {
        let media = media.clone();
        let reach = Reach::new("youtube", id, move |_| {
            let (url, headers) = request(&locator.0);
            let address = Address::new("youtube", media.clone(), url)
                .capped(ytmusic::stream::CHUNK)
                .renewed(locator.clone());
            (address.headers(headers), Plain)
        });
        Arc::new(reach.nearby(NEARBY))
    });
    Ok(Loaded {
        stream,
        segments: segments.map(Arc::new),
        reach,
        edit,
        loudness_db: format.loudness_db,
        duration: format.duration,
        encoding: served(&format.codec, format.bitrate / 1000),
    })
}

/// The segment index and the edit list, which sit at the front of the file inside what the
/// preroll waited for.
fn front(stream: &Stream) -> (Option<Segments>, Option<Trim>) {
    stream
        .with(|_, spool| {
            let front = spool.bytes(0..spool.len().min(PREROLL)).unwrap_or_default();
            (Segments::read(&front), trim::from_mp4(&front))
        })
        .unwrap_or_default()
}

/// The url of the file and the headers ytmusic sends with every request for it, as reqwest
/// takes them.
fn request(locator: &Locator) -> (String, HeaderMap) {
    let (url, headers) = locator.request();
    let headers = headers.into_iter().filter_map(|(name, value)| {
        let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        Some((name, HeaderValue::from_str(value).ok()?))
    });
    (url, headers.collect())
}

/// What tells two kept copies of one track apart: the bitrate ceiling it was fetched under.
fn variant(ceiling: Option<u32>) -> String {
    ceiling.map_or_else(|| "best".to_owned(), |ceiling| ceiling.to_string())
}

/// The bitrate ceiling a level stands for, in bits per second. Each sits between two streams,
/// since the bitrate YouTube reports runs a little above the nominal one.
fn ceiling(quality: Quality) -> Option<u32> {
    match quality.within(QUALITIES) {
        Quality::HiRes | Quality::Lossless | Quality::High => None,
        Quality::Normal => Some(160_000),
        Quality::Low => Some(64_000),
    }
}

/// The encoding of the stream the host serves, by its codec name and bit rate in kilobits a
/// second, `None` when it gave no bit rate.
fn served(codec: &str, kbps: u32) -> Option<Encoding> {
    (kbps > 0).then(|| match codec {
        "opus" => Encoding::Opus { kbps },
        codec if codec.starts_with("mp4a") => Encoding::Aac { kbps },
        _ => Encoding::Other,
    })
}
