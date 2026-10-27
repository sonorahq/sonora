//! Playback for SoundCloud: what [`crate::engine`] needs that is SoundCloud's own.
//!
//! Every track is AAC over HLS. A media playlist names one init segment and then a run of media
//! segments of fragmented MP4, which [`Parts`] fetches one after another into one stream, so to
//! the decoder it is one fragmented file. [`Timeline`] notes where each part lands in the
//! stream, so a seek splices the init segment onto the segment that covers the position.

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::audio::Trimmed;
use crate::engine::{self, Fetch};
use crate::soundcloud::client::{self, SoundCloudClient};
use crate::soundcloud::hls;
use crate::stream::{self, Plain, Reader, Stream};
use crate::vault::{Recording, Shelf};
use crate::{Encoding, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};

/// How long a part keeps being asked for, with the wait between asks doubling from
/// `FIRST_WAIT` up to `LONGEST_WAIT`. A track is a few dozen requests, so one that fails now
/// and then is ordinary.
const RETRY_WINDOW: Duration = Duration::from_secs(30);
const FIRST_WAIT: Duration = Duration::from_millis(500);
const LONGEST_WAIT: Duration = Duration::from_secs(4);

/// The most one segment may hold, so a wrong answer cannot fill memory.
const SEGMENT_CEILING: usize = 8 * 1024 * 1024;

/// A decoder over a SoundCloud track, with what comes before the position trimmed off.
type Opened = Trimmed<rodio::Decoder<Reader<Plain>>>;

/// Where the segments of a track start, in the music and in the stream.
struct Timeline {
    /// When each media segment starts, from the playlist.
    starts: Vec<Duration>,
    /// Where each part starts in the stream: the init segment first, then the media segments.
    /// It grows as the parts arrive.
    offsets: Vec<usize>,
}

/// Where to start reading to land on a position, and how much of the first segment to throw
/// away to land exactly on it.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// The end of the init segment, which is spliced in front of the segment.
    head: usize,
    /// Where the segment starts in the stream.
    graft: usize,
    /// How far into that segment the position is.
    into: Duration,
}

impl Timeline {
    /// The index of the segment covering `at`.
    fn index(&self, at: Duration) -> Option<usize> {
        self.starts
            .partition_point(|start| *start <= at)
            .checked_sub(1)
    }

    /// The segment covering `at`, once the download has reached it.
    fn entry(&self, at: Duration) -> Option<Entry> {
        let index = self.index(at)?;
        Some(Entry {
            head: *self.offsets.get(1)?,
            graft: *self.offsets.get(index + 1)?,
            into: at.saturating_sub(self.starts[index]),
        })
    }
}

/// The init segment and media segments of a track, fetched one after another and handed to the
/// stream as one body. Each segment is fetched whole before any of it is handed over, so a
/// connection that drops part way is asked again rather than ending the track. It writes where
/// each part starts into the timeline, and stops once nothing holds the timeline.
struct Parts {
    http: reqwest::Client,
    /// The init segment's url, then the media segments' in order.
    urls: Vec<String>,
    /// The index in `urls` of the part to fetch next.
    next: usize,
    /// How many bytes have been handed to the stream, which is where the next part starts.
    sent: usize,
    timeline: Weak<Mutex<Timeline>>,
}

impl stream::Source for Parts {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        if self.timeline.strong_count() == 0 {
            return Ok(None);
        }
        let Some(url) = self.urls.get(self.next).cloned() else {
            return Ok(None);
        };
        let part = whole(&self.http, &url).await?;
        if let Some(timeline) = self.timeline.upgrade() {
            lock(&timeline).offsets.push(self.sent);
        }
        self.sent += part.len();
        self.next += 1;
        Ok(Some(part))
    }
}

/// A track arriving, and how long SoundCloud says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream<Plain>,
    timeline: Arc<Mutex<Timeline>>,
    duration: Option<Duration>,
    /// How the track is encoded, from the preset it was served as.
    encoding: Option<Encoding>,
}

pub struct Factory {
    client: SoundCloudClient,
    media: reqwest::Client,
}

impl Factory {
    pub fn new(client: SoundCloudClient) -> Self {
        let media = media_http().unwrap_or_else(|error| {
            log::warn!("soundcloud: {error:#}, streaming through the api client");
            client.http_client().clone()
        });
        Self { client, media }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            SoundCloud {
                client: self.client.clone(),
                media: self.media.clone(),
                quality: config.quality,
                shelf: config.vault.as_ref().map(|vault| vault.shelf("soundcloud")),
            },
            config,
        )
    }
}

struct SoundCloud {
    client: SoundCloudClient,
    media: reqwest::Client,
    /// The level every track is asked for at.
    quality: Quality,
    /// Where played tracks are kept, as the segments arrived.
    shelf: Option<Shelf>,
}

/// What the api said about a track, kept beside it in the vault, and the timeline its download
/// built, so a kept track seeks exactly as it did then.
#[derive(Serialize, Deserialize)]
struct Facts {
    duration_s: Option<u64>,
    starts_ns: Vec<u64>,
    offsets: Vec<usize>,
}

impl SoundCloud {
    /// The track from the vault, when it was kept at the preset the level asks for.
    async fn recall(&self, id: &str, variant: &str) -> Option<Loaded> {
        let kept = self.shelf.as_ref()?.find(id, variant).await?;
        let facts = serde_json::from_slice::<Facts>(&kept.facts)
            .inspect_err(|error| {
                log::warn!("playback: the kept facts of {id} do not read: {error}")
            })
            .ok()?;
        let timeline = Arc::new(Mutex::new(Timeline {
            starts: facts
                .starts_ns
                .iter()
                .copied()
                .map(Duration::from_nanos)
                .collect(),
            offsets: facts.offsets,
        }));
        let stream = Stream::pulling(kept.source, Some(kept.length), Plain)
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        log::debug!("playback: {id} started from the vault");
        Some(Loaded {
            stream,
            timeline,
            duration: facts.duration_s.map(Duration::from_secs),
            encoding: encoding_for(variant),
        })
    }

    /// Downloads the playlist's segments and waits for the preroll.
    async fn arrive(&self, id: &str, opened: client::Opened) -> Result<Loaded> {
        let playlist = hls::open(&self.media, &opened.m3u8).await?;
        let urls: Vec<String> = std::iter::once(playlist.init)
            .chain(playlist.media.iter().map(|segment| segment.url.clone()))
            .collect();
        let starts: Vec<Duration> = playlist
            .media
            .iter()
            .scan(Duration::ZERO, |at, segment| {
                let start = *at;
                *at += segment.duration;
                Some(start)
            })
            .collect();
        let timeline = Arc::new(Mutex::new(Timeline {
            starts,
            offsets: Vec::new(),
        }));
        let parts = Parts {
            http: self.media.clone(),
            urls,
            next: 0,
            sent: 0,
            timeline: Arc::downgrade(&timeline),
        };
        let held = Arc::downgrade(&timeline);
        let facts = Facts {
            duration_s: opened.duration.map(|duration| duration.as_secs()),
            starts_ns: Vec::new(),
            offsets: Vec::new(),
        };
        let source = match &self.shelf {
            Some(shelf) => shelf.keep(
                id,
                opened.served,
                move || {
                    let timeline = held.upgrade()?;
                    let timeline = lock(&timeline);
                    (timeline.offsets.len() == timeline.starts.len() + 1)
                        .then(|| Facts {
                            starts_ns: timeline
                                .starts
                                .iter()
                                .map(|start| start.as_nanos() as u64)
                                .collect(),
                            offsets: timeline.offsets.clone(),
                            ..facts
                        })
                        .and_then(|facts| serde_json::to_vec(&facts).ok())
                },
                parts,
            ),
            None => Recording::through(parts),
        };
        let stream = Stream::pulling(source, None, Plain).primed().await?;
        Ok(Loaded {
            stream,
            timeline,
            duration: opened.duration,
            encoding: encoding_for(opened.served),
        })
    }
}

#[async_trait]
impl Fetch for SoundCloud {
    type Loaded = Loaded;
    type Source = Opened;

    fn name(&self) -> &'static str {
        "soundcloud"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    async fn load(&self, id: &str) -> Result<Loaded> {
        let served = client::served(self.quality);
        if let Some(loaded) = self.recall(id, served).await {
            return Ok(loaded);
        }
        let opened = self.client.open_stream(id, self.quality).await?;
        self.arrive(id, opened).await
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        loaded.encoding
    }

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    /// A segmented stream has no index for symphonia to jump around in, so a seek opens a
    /// second decoder spliced to the target.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        if at.is_zero() {
            let decoder = decoder(id, loaded.stream.reader())?;
            return Some(Trimmed::new(decoder, Duration::ZERO, None));
        }
        let entry = landing(&loaded.stream, &loaded.timeline, at)?;
        let reader = loaded.stream.spliced(entry.head, entry.graft);
        let decoder = decoder(id, reader)?;
        Some(Trimmed::new(decoder, entry.into, None))
    }
}

/// Where to read a segmented download to land on `at`: at once when it has the segment, after a
/// wait when it has not arrived yet. The wait is bounded by the stream's own patience.
fn landing(stream: &Stream<Plain>, timeline: &Mutex<Timeline>, at: Duration) -> Option<Entry> {
    if let Some(entry) = lock(timeline).entry(at) {
        return Some(entry);
    }
    stream.awaiting(|_, _, _| lock(timeline).entry(at))
}

/// Locks a mutex whose guarded value is still sound after a panic elsewhere.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One request for a part, read whole and asked again with a growing wait while a connection
/// keeps failing.
async fn whole(http: &reqwest::Client, url: &str) -> Result<Bytes> {
    let mut wait = FIRST_WAIT;
    let started = Instant::now();
    loop {
        match fetch(http, url).await {
            Ok(part) => return Ok(part),
            Err(error) if started.elapsed() < RETRY_WINDOW => {
                log::debug!("soundcloud: asking again in {:?} after {error:#}", wait);
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(LONGEST_WAIT);
            }
            Err(error) => return Err(error),
        }
    }
}

/// One request for a part, read whole. The url is dropped from the error, since it is signed
/// and the error ends up in the log.
async fn fetch(http: &reqwest::Client, url: &str) -> Result<Bytes> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(reqwest::Error::without_url)
        .context("cannot fetch a soundcloud segment")?
        .error_for_status()
        .map_err(reqwest::Error::without_url)
        .context("the soundcloud media server refused a segment")?;
    let mut response = response;
    let mut part = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if part.len() + chunk.len() > SEGMENT_CEILING {
            anyhow::bail!("a segment runs past {SEGMENT_CEILING} bytes");
        }
        part.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(part))
}

/// The http client a track's segments come through, which gives up on a stalled request sooner
/// than the api client does.
fn media_http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(8))
        .build()
        .context("cannot build the soundcloud media client")
}

/// The encoding of the preset a track was served as.
fn encoding_for(preset: &str) -> Option<Encoding> {
    match preset {
        "aac_160k" => Some(Encoding::Aac { kbps: 160 }),
        "aac_96k" => Some(Encoding::Aac { kbps: 96 }),
        _ => None,
    }
}

/// Builds a decoder that only reads forward, or reports a track that will not decode.
fn decoder(id: &str, reader: Reader<Plain>) -> Option<rodio::Decoder<Reader<Plain>>> {
    match rodio::Decoder::builder()
        .with_symphonia_decoder::<Aac>()
        .with_data(reader)
        .with_seekable(false)
        .build()
    {
        Ok(decoder) => Some(decoder),
        Err(error) => {
            log::warn!("playback: cannot decode the soundcloud track {id}: {error}");
            None
        }
    }
}
