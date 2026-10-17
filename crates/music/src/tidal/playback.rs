//! Playback for Tidal: what [`crate::engine`] needs that is Tidal's own.
//!
//! A lossless track is one FLAC file and plays like any other. A hi-res track is a DASH stream
//! of fMP4 segments, which [`Parts`] fetches one after another into one stream, so to the
//! decoder it is one fragmented file. The manifest says when every segment starts and
//! [`Timeline`] notes where each one lands in the stream, so a seek splices the init segment
//! onto the segment that covers the position instead of decoding its way there.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use bytes::Bytes;
use rodio::Source as _;

use crate::audio::Trimmed;
use crate::engine::{self, Fetch, Loudness};
use crate::stream::{self, Reader, Stream};
use crate::tidal::client::TidalClient;
use crate::tidal::lock;
use crate::tidal::manifest::Manifest;
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// How many times one segment is asked for before the track gives up on it. A track is a
/// hundred or so requests, so one that fails now and then is ordinary.
const PART_ATTEMPTS: u32 = 3;

/// The most one segment may hold. A hi-res segment is a megabyte or two, so this only bounds
/// what a wrong answer can cost.
const SEGMENT_CEILING: usize = 64 * 1024 * 1024;

/// A decoder over a Tidal track, with what comes before the asked-for position trimmed off.
type Opened = Trimmed<rodio::Decoder<Reader>>;

/// Where the segments of a segmented track start, in the music and in the stream.
struct Timeline {
    /// When each media segment starts, from the manifest.
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

/// The init segment and media segments of a DASH track, fetched one after another and handed
/// to the stream as one body. Each segment is fetched whole before any of it is handed over, so
/// a connection that drops part way is asked again rather than ending the track. It writes where
/// each part starts into the timeline, and stops once nothing holds the timeline, which is how
/// an abandoned track stops downloading.
struct Parts {
    http: reqwest::Client,
    urls: std::vec::IntoIter<String>,
    /// How many bytes have been handed to the stream, which is where the next part starts.
    sent: usize,
    timeline: Weak<Mutex<Timeline>>,
}

/// A track arriving, and how long and how loud Tidal says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    /// Where a segmented track's segments start. `None` for a track that is one file.
    timeline: Option<Arc<Mutex<Timeline>>>,
    duration: Option<Duration>,
    loudness: Option<Loudness>,
}

/// Starts a playback engine for a signed-in session, which is what a [`crate::ProviderSession`]
/// hands the app.
pub struct Factory {
    client: TidalClient,
}

/// What the shared engine is given: how to fetch a Tidal track and open a decoder over it.
struct Tidal {
    client: TidalClient,
}

impl Timeline {
    /// The segment covering `at`, once the download has reached it.
    fn entry(&self, at: Duration) -> Option<Entry> {
        let index = self
            .starts
            .partition_point(|start| *start <= at)
            .checked_sub(1)?;
        Some(Entry {
            head: *self.offsets.get(1)?,
            graft: *self.offsets.get(index + 1)?,
            into: at.saturating_sub(self.starts[index]),
        })
    }
}

impl Parts {
    /// Fetches one part whole, again after a short wait when the request, the server or the
    /// connection fails. `None` when the track was abandoned, which is checked before every
    /// attempt.
    async fn fetch(&self, url: &str) -> Result<Option<Bytes>> {
        let mut attempt = 1;
        loop {
            if self.timeline.strong_count() == 0 {
                return Ok(None);
            }
            match self.attempt(url).await {
                Ok(part) => return Ok(Some(part)),
                Err(error) if attempt >= PART_ATTEMPTS => {
                    return Err(error.context("cannot fetch a segment of the tidal track"));
                }
                Err(error) => {
                    log::debug!("tidal: asking for a segment again after {error:#}");
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                    attempt += 1;
                }
            }
        }
    }

    /// One request for a part, read whole. A part longer than `SEGMENT_CEILING` is refused
    /// before it can fill memory, since nothing past here would stop it.
    async fn attempt(&self, url: &str) -> Result<Bytes> {
        let mut response = self.http.get(url).send().await?.error_for_status()?;
        let mut part = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if part.len() + chunk.len() > SEGMENT_CEILING {
                anyhow::bail!("a segment runs past {SEGMENT_CEILING} bytes");
            }
            part.extend_from_slice(&chunk);
        }
        Ok(Bytes::from(part))
    }
}

impl stream::Source for Parts {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        let Some(url) = self.urls.next() else {
            return Ok(None);
        };
        let Some(part) = self.fetch(&url).await? else {
            return Ok(None);
        };
        let Some(timeline) = self.timeline.upgrade() else {
            return Ok(None);
        };
        lock(&timeline).offsets.push(self.sent);
        self.sent += part.len();
        Ok(Some(part))
    }
}

impl Factory {
    pub fn new(client: TidalClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Tidal {
                client: self.client.clone(),
            },
            config,
        )
    }
}

#[async_trait]
impl Fetch for Tidal {
    type Loaded = Loaded;
    type Source = Opened;

    fn name(&self) -> &'static str {
        "tidal"
    }

    /// Asks for the best quality the account may play and starts the download, waiting only
    /// for the preroll.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let playback = self.client.playback(id).await?;
        log::info!(
            "tidal: {id} streams as {} {}",
            playback.quality,
            playback.manifest.codec()
        );
        let (stream, timeline) = match playback.manifest {
            Manifest::File { url, .. } => {
                let response = self
                    .client
                    .http()
                    .get(&url)
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                    .context("cannot stream the tidal track")?;
                (Stream::new(response, stream::Plain), None)
            }
            Manifest::Segments {
                init,
                media,
                starts,
                ..
            } => {
                let timeline = Arc::new(Mutex::new(Timeline {
                    starts,
                    offsets: Vec::new(),
                }));
                let parts = Parts {
                    http: self.client.http().clone(),
                    urls: std::iter::once(init)
                        .chain(media)
                        .collect::<Vec<_>>()
                        .into_iter(),
                    sent: 0,
                    timeline: Arc::downgrade(&timeline),
                };
                (Stream::pulling(parts, None, stream::Plain), Some(timeline))
            }
        };
        Ok(Loaded {
            stream: stream.primed().await?,
            timeline,
            duration: playback.duration,
            loudness: playback.loudness,
        })
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness
    }

    /// A track that failed because Tidal refused the session is the listener having to sign
    /// in again, not the track being unplayable.
    fn gated(&self, error: &anyhow::Error) -> bool {
        crate::tidal::auth::refused(error)
    }

    /// Resolves once the last segment or the whole file is in, so the next track waits rather
    /// than taking bandwidth from this one.
    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    /// A segmented stream has no index for symphonia to jump around in, so a seek opens a
    /// second decoder at the target. A file is simply opened there.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    /// Builds a decoder placed at `at`. Only the header is read here.
    ///
    /// A file is opened seekable and moved. A segmented track is opened without its length,
    /// which would have symphonia read every fragment to build a sample table, and a position
    /// splices the init segment onto the segment that covers it, trimming what comes before
    /// the position off the front.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        open(id, loaded, at)
    }
}

/// What [`Fetch::open`] does, which needs nothing from the client.
fn open(id: &str, loaded: &Loaded, at: Duration) -> Option<Opened> {
    let stream = &loaded.stream;
    let segmented = loaded.timeline.is_some();
    let entry = loaded
        .timeline
        .as_ref()
        .filter(|_| !at.is_zero())
        .and_then(|timeline| stream.awaiting(|_, _, _| lock(timeline).entry(at)));
    let builder = rodio::Decoder::builder();
    let builder = match (segmented, entry) {
        (true, Some(entry)) => builder
            .with_data(stream.spliced(entry.head, entry.graft))
            .with_seekable(false),
        (true, None) => builder.with_data(stream.reader()).with_seekable(false),
        (false, _) => {
            let builder = builder.with_data(stream.reader()).with_seekable(true);
            match stream.total() {
                Some(total) => builder.with_byte_len(total),
                None => builder,
            }
        }
    };
    let mut decoder = match builder.build() {
        Ok(decoder) => decoder,
        Err(error) => {
            log::warn!("playback: cannot decode the tidal track {id}: {error}");
            return None;
        }
    };
    log::debug!(
        "playback: {id} decodes at {} Hz, {} channels",
        decoder.sample_rate(),
        decoder.channels()
    );
    let skip = match (segmented, entry) {
        (true, Some(entry)) => entry.into,
        (true, None) => at,
        (false, _) => {
            if !at.is_zero()
                && let Err(error) = decoder.try_seek(at)
            {
                log::warn!(
                    "playback: cannot start the tidal track {id} at {}s: {error}",
                    at.as_secs()
                );
            }
            Duration::ZERO
        }
    };
    Some(Trimmed::new(decoder, skip, None))
}
