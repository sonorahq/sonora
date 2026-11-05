//! Playback for Tidal: what [`crate::engine`] needs that is Tidal's own.
//!
//! A lossless track is one FLAC file and plays like any other. A hi-res track is a DASH stream
//! of fMP4 segments, which [`Parts`] fetches one after another into one stream, so to the
//! decoder it is one fragmented file. The manifest says when every segment starts and
//! [`Timeline`] notes where each one lands in the stream, so a seek splices the init segment
//! onto the segment that covers the position instead of decoding its way there.
//!
//! A seek past what has arrived does not wait for the download to get there. It starts a second
//! download at the target, an [`Ahead`], from the covering segment or from a byte of the file a
//! little before the position, and decodes from that.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use rodio::Source as _;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::audio::Trimmed;
use crate::engine::{self, Fetch, Loudness};
use crate::stream::{self, Reader, Stream};
use crate::tidal::client::TidalClient;
use crate::tidal::lock;
use crate::tidal::manifest::Manifest;
use crate::vault::{Recording, Shelf};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};

/// How long one part keeps being asked for, with the wait between asks doubling from
/// `FIRST_WAIT` up to `LONGEST_WAIT`. A track is a hundred or so requests, so one that fails
/// now and then is ordinary, and a connection that drops for half a minute comes back to the
/// same track.
const RETRY_WINDOW: Duration = Duration::from_secs(45);
const FIRST_WAIT: Duration = Duration::from_millis(500);
const LONGEST_WAIT: Duration = Duration::from_secs(8);

/// How many times one track asks Tidal for fresh urls when the signed ones it holds are refused.
const RENEWALS: u32 = 3;

/// How long a media request may take to connect, and how long it may wait between two reads.
/// Both sit well under the patience of a reader waiting at the end of the download, so a stalled
/// request is asked again while that reader still waits.
const MEDIA_CONNECT: Duration = Duration::from_secs(5);
const MEDIA_STALL: Duration = Duration::from_secs(8);

/// How long a track may take to arrive as far as the preroll before its load fails.
const PRIMING: Duration = Duration::from_secs(20);

/// The most one segment may hold. A hi-res segment is a megabyte or two, so this only bounds
/// what a wrong answer can cost.
const SEGMENT_CEILING: usize = 64 * 1024 * 1024;

/// The most one Tidal track may spool. 24-bit FLAC at 192 kHz runs to about 40 MB a minute, so
/// this holds more than an hour and a half of it. On a 32-bit target it is as much as a size
/// can count.
const SPOOL_CEILING: u64 = 4 * 1024 * 1024 * 1024;

/// How far before a seek's target a FLAC file is entered, one lead per try. The file's bytes
/// only roughly follow its time, so the decoder enters short of the target and reads forward to
/// land on it exactly, and a try that overshot is made again from further back.
const LEADS: [Duration; 2] = [Duration::from_secs(15), Duration::from_secs(60)];

/// How far past what a file download has received a seek may enter and still wait for it,
/// rather than start another download there. It is a few seconds of FLAC, which any connection
/// that streams it brings in about as fast as a new request would.
const NEARBY: u64 = 2 * 1024 * 1024;

/// A decoder over a Tidal track, with what comes before the asked-for position trimmed off.
type Opened = Trimmed<rodio::Decoder<Reader<Long>>>;

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
    /// The init segment's url, then the media segments' in order from `first`.
    urls: Vec<String>,
    /// The index of the first media segment this download carries. It is past zero for a
    /// download a seek started.
    first: usize,
    /// The index in `urls` of the part to fetch next.
    next: usize,
    /// How many bytes have been handed to the stream, which is where the next part starts.
    sent: usize,
    timeline: Weak<Mutex<Timeline>>,
    renewal: Renewal,
}

/// A track that is one file, read as one response and asked for again from the first missing
/// byte when the connection drops. A refused url is renewed the way a segment's is.
struct Resuming {
    http: reqwest::Client,
    url: String,
    /// The byte of the file this download starts at. It is past zero for a download a seek
    /// started.
    from: u64,
    /// The response being read. `None` before the first request and between a drop and the
    /// request that resumes it.
    response: Option<reqwest::Response>,
    /// How many bytes have been handed to the stream, which is where a resumed request starts.
    received: u64,
    /// How many bytes this download carries, when the server said.
    total: Option<u64>,
    /// The whole file's length, when the server said, which a resumed answer has to match.
    size: Option<u64>,
    renewal: Renewal,
}

/// What a seek past the download needs to start another download at its target, and the last
/// one it started. A track from the vault has none, since all of it is there.
struct Reach {
    /// The runtime the downloads run on, which the audio thread enters to start one.
    runtime: tokio::runtime::Handle,
    /// The file's url, or the init segment's url and then the media segments'.
    urls: Vec<String>,
    codec: String,
    /// The download the last seek past the main one started. It is only replaced once a
    /// decoder over the next one has opened, so the decoder still playing never loses it.
    ahead: Mutex<Option<Ahead>>,
}

/// A download a seek started at its target, running to the end of the track beside the main
/// one.
#[derive(Clone)]
struct Ahead {
    stream: Stream<Long>,
    graft: Graft,
}

/// How an [`Ahead`] is laid out.
#[derive(Clone)]
enum Graft {
    /// The init segment, then the media segments from `first` on, placed by their own timeline.
    Segments {
        first: usize,
        timeline: Arc<Mutex<Timeline>>,
    },
    /// The file's first `head` bytes, then the file from byte `from` on.
    File { head: usize, from: u64 },
}

/// What placing a time in a FLAC file by its bytes goes on: how long the track runs, how long
/// the file is, and where its metadata ends and the frames begin.
#[derive(Clone, Copy)]
struct Flac {
    length: Duration,
    size: u64,
    head: usize,
}

/// A FLAC file's metadata followed by the file from some byte on, which a decoder reads as the
/// track entered part way.
struct Headed {
    /// The metadata, until it has been handed over.
    head: Option<Bytes>,
    file: Resuming,
}

/// What a download needs to ask Tidal for fresh urls once the signed ones it holds are refused.
struct Renewal {
    client: TidalClient,
    id: String,
    /// The codec the download started in, which fresh urls have to name too.
    codec: String,
    /// The level the download was asked for at, which fresh urls are asked for at too.
    quality: Quality,
    /// How many more times the urls may be renewed.
    left: u32,
}

/// The waits between asks for one thing, and when the asking stops.
struct Backoff {
    since: Instant,
    wait: Duration,
}

/// Why a media request failed, which decides what is tried next.
enum Failure {
    /// The signed url was refused or has gone, so Tidal is asked for fresh ones.
    Expired(anyhow::Error),
    /// The connection or the server failed, which waiting may cure.
    Passing(anyhow::Error),
    /// Asking again would get the same answer.
    Final(anyhow::Error),
}

/// The body of a Tidal track, taken as it comes but allowed to run as long as hi-res does.
struct Long;

/// A track arriving, and how long and how loud Tidal says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream<Long>,
    /// Where a segmented track's segments start. `None` for a track that is one file.
    timeline: Option<Arc<Mutex<Timeline>>>,
    /// What a seek past the download needs. `None` for a track from the vault.
    reach: Option<Arc<Reach>>,
    duration: Option<Duration>,
    loudness: Option<Loudness>,
}

/// Starts a playback engine for a signed-in session, which is what a [`crate::ProviderSession`]
/// hands the app.
pub struct Factory {
    client: TidalClient,
    media: reqwest::Client,
}

/// What the shared engine is given: how to fetch a Tidal track and open a decoder over it.
struct Tidal {
    client: TidalClient,
    /// The http client the track's bytes come through, which gives up on a stalled request
    /// sooner than the api client does.
    media: reqwest::Client,
    /// The level every track is asked for at.
    quality: Quality,
    /// Where played tracks are kept, so playing one again asks Tidal for nothing.
    shelf: Option<Shelf>,
}

/// What Tidal said about a track, kept beside it in the vault, and for a segmented track the
/// timeline its download built.
#[derive(Serialize, Deserialize)]
struct Facts {
    duration_ms: Option<u64>,
    lufs: Option<f32>,
    peak: Option<f32>,
    /// When each media segment starts in the music, in nanoseconds. Empty for one file.
    starts_ns: Vec<u64>,
    /// Where each part starts in the stream, the init segment first. Empty for one file.
    offsets: Vec<usize>,
}

impl Timeline {
    /// The index of the segment covering `at`.
    fn index(&self, at: Duration) -> Option<usize> {
        self.starts
            .partition_point(|start| *start <= at)
            .checked_sub(1)
    }

    /// Whether the segment covering `at` is one of the next two parts to arrive, so waiting
    /// for it costs no more than a download started there would.
    fn due(&self, at: Duration) -> bool {
        self.index(at)
            .is_some_and(|index| index <= self.offsets.len())
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

impl Parts {
    /// Fetches the next part whole. A failed connection or server is asked again with a growing
    /// wait, and a refused url is renewed before the same part is asked for again. `None` once
    /// every part is in or the track was abandoned, which is checked before every attempt.
    async fn fetch(&mut self) -> Result<Option<Bytes>> {
        let mut backoff = Backoff::new();
        loop {
            if self.timeline.strong_count() == 0 {
                return Ok(None);
            }
            let Some(url) = self.urls.get(self.next) else {
                return Ok(None);
            };
            match whole(&self.http, url).await {
                Ok(part) => return Ok(Some(part)),
                Err(Failure::Expired(error)) => {
                    log::debug!("tidal: segment {} was refused: {error:#}", self.next);
                    self.renew().await?;
                }
                Err(Failure::Passing(error)) => backoff.wait(error).await?,
                Err(Failure::Final(error)) => return Err(error),
            }
        }
    }

    /// Swaps in fresh urls for the same segments, so the download carries on from the part it
    /// was on.
    async fn renew(&mut self) -> Result<()> {
        let Manifest::Segments { init, media, .. } = self.renewal.manifest().await? else {
            bail!("tidal no longer streams the track as segments");
        };
        let count = self.first + self.urls.len() - 1;
        if media.len() != count {
            bail!(
                "tidal now cuts the track into {} segments rather than {count}",
                media.len(),
            );
        }
        self.urls = std::iter::once(init)
            .chain(media.into_iter().skip(self.first))
            .collect();
        Ok(())
    }
}

impl stream::Source for Parts {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        let Some(part) = self
            .fetch()
            .await
            .context("cannot fetch a segment of the tidal track")?
        else {
            return Ok(None);
        };
        let Some(timeline) = self.timeline.upgrade() else {
            return Ok(None);
        };
        lock(&timeline).offsets.push(self.sent);
        self.sent += part.len();
        self.next += 1;
        Ok(Some(part))
    }
}

impl Resuming {
    /// A download of the file from byte `from` on, which makes its first request when it is
    /// first read.
    fn new(http: reqwest::Client, url: String, from: u64, renewal: Renewal) -> Self {
        Self {
            http,
            url,
            from,
            response: None,
            received: 0,
            total: None,
            size: None,
            renewal,
        }
    }

    /// Makes the first request, asking again the way a dropped connection would.
    async fn open(http: reqwest::Client, url: String, renewal: Renewal) -> Result<Self> {
        let mut file = Self::new(http, url, 0, renewal);
        file.connect()
            .await
            .context("cannot stream the tidal track")?;
        Ok(file)
    }

    /// Opens a response from the first byte not yet received. A failed connection or server is
    /// asked again with a growing wait, and a refused url is renewed first. The first response
    /// says how long the download and the file are.
    async fn connect(&mut self) -> Result<()> {
        let mut backoff = Backoff::new();
        loop {
            match self.request().await {
                Ok(response) => {
                    if self.received == 0 {
                        self.total = response.content_length();
                        self.size = match self.from {
                            0 => self.total,
                            _ => content_range(&response).and_then(|(_, size)| size),
                        };
                    }
                    self.response = Some(response);
                    return Ok(());
                }
                Err(Failure::Expired(error)) => {
                    log::debug!("tidal: the file was refused: {error:#}");
                    self.renew().await?;
                }
                Err(Failure::Passing(error)) => backoff.wait(error).await?,
                Err(Failure::Final(error)) => return Err(error),
            }
        }
    }

    async fn renew(&mut self) -> Result<()> {
        let Manifest::File { url, .. } = self.renewal.manifest().await? else {
            bail!("tidal no longer streams the track as one file");
        };
        self.url = url;
        Ok(())
    }

    /// One request from the first byte not yet received. A request past the first byte has to
    /// be answered with exactly the rest of the same file, or the track would splice two files
    /// together.
    async fn request(&self) -> Result<reqwest::Response, Failure> {
        let at = self.from + self.received;
        let mut request = self.http.get(&self.url);
        if at > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={at}-"));
        }
        let response = Failure::check(request.send().await?)?;
        if at == 0 {
            return Ok(response);
        }
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Failure::Final(anyhow!(
                "the tidal file server cannot start a download part way"
            )));
        }
        let lines_up = content_range(&response)
            .is_some_and(|(start, size)| start == at && (self.size.is_none() || size == self.size));
        match lines_up {
            true => Ok(response),
            false => Err(Failure::Final(anyhow!(
                "the tidal file server answered somewhere other than byte {at}"
            ))),
        }
    }
}

impl stream::Source for Resuming {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        let mut backoff = Backoff::new();
        loop {
            let Some(response) = &mut self.response else {
                self.connect()
                    .await
                    .context("cannot resume the tidal track")?;
                continue;
            };
            let error = match response.chunk().await {
                Ok(Some(chunk)) => {
                    self.received += chunk.len() as u64;
                    return Ok(Some(chunk));
                }
                Ok(None) => match self.total {
                    Some(total) if self.received < total => {
                        anyhow!("the tidal file ended {} bytes early", total - self.received)
                    }
                    _ => return Ok(None),
                },
                Err(error) => error.without_url().into(),
            };
            self.response = None;
            backoff
                .wait(error)
                .await
                .context("cannot stream the tidal track")?;
        }
    }
}

impl stream::Source for Headed {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        if let Some(head) = self.head.take() {
            return Ok(Some(head));
        }
        self.file.chunk().await
    }
}

impl Renewal {
    /// A fresh manifest for the same track in the same codec. It fails once `RENEWALS` are
    /// spent, since a refusal that outlasts them is not an expired url.
    async fn manifest(&mut self) -> Result<Manifest> {
        if self.left == 0 {
            bail!("tidal keeps refusing the urls of {}", self.id);
        }
        self.left -= 1;
        log::info!("tidal: asking for fresh urls for {}", self.id);
        let manifest = self.client.playback(&self.id, self.quality).await?.manifest;
        if manifest.codec() != self.codec {
            bail!(
                "tidal now streams {} as {} rather than {}",
                self.id,
                manifest.codec(),
                self.codec
            );
        }
        Ok(manifest)
    }
}

impl Backoff {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            wait: FIRST_WAIT,
        }
    }

    /// Waits before the next ask, or hands `error` back once `RETRY_WINDOW` has passed since
    /// the first one.
    async fn wait(&mut self, error: anyhow::Error) -> Result<()> {
        if self.since.elapsed() >= RETRY_WINDOW {
            return Err(error);
        }
        log::debug!("tidal: asking again in {:?} after {error:#}", self.wait);
        tokio::time::sleep(self.wait).await;
        self.wait = (self.wait * 2).min(LONGEST_WAIT);
        Ok(())
    }
}

impl Failure {
    /// Sorts a response by its status, passing a successful one through.
    fn check(response: reqwest::Response) -> Result<reqwest::Response, Self> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let error = anyhow!("the tidal media server answered {status}");
        Err(match status {
            StatusCode::FORBIDDEN | StatusCode::GONE => Self::Expired(error),
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS => Self::Passing(error),
            status if status.is_server_error() => Self::Passing(error),
            _ => Self::Final(error),
        })
    }
}

impl From<reqwest::Error> for Failure {
    /// A request that could not be built fails the same way every time. Anything else went
    /// wrong on the way and may not again. The url is dropped, since it is signed and the error
    /// ends up in the log.
    fn from(error: reqwest::Error) -> Self {
        let lasting = error.is_builder();
        let error = error.without_url().into();
        match lasting {
            true => Self::Final(error),
            false => Self::Passing(error),
        }
    }
}

impl stream::Body for Long {
    fn ceiling(&self) -> usize {
        usize::try_from(SPOOL_CEILING).unwrap_or(usize::MAX)
    }
}

impl Factory {
    pub fn new(client: TidalClient) -> Self {
        let media = media_http().unwrap_or_else(|error| {
            log::warn!("tidal: {error:#}, streaming through the api client");
            client.http().clone()
        });
        Self { client, media }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Tidal {
                client: self.client.clone(),
                media: self.media.clone(),
                quality: config.quality,
                shelf: config.vault.as_ref().map(|vault| vault.shelf("tidal")),
            },
            config,
        )
    }
}

impl Tidal {
    /// The track from the vault, when it was kept at the level asked for. A segmented track
    /// gets back the timeline it was kept with, so a seek splices exactly as it did then.
    async fn recall(&self, id: &str) -> Option<Loaded> {
        let kept = self.shelf.as_ref()?.find(id, self.quality.id()).await?;
        let facts = serde_json::from_slice::<Facts>(&kept.facts)
            .inspect_err(|error| {
                log::warn!("playback: the kept facts of {id} do not read: {error}")
            })
            .ok()?;
        let timeline = (!facts.offsets.is_empty()).then(|| {
            Arc::new(Mutex::new(Timeline {
                starts: facts
                    .starts_ns
                    .iter()
                    .copied()
                    .map(Duration::from_nanos)
                    .collect(),
                offsets: facts.offsets.clone(),
            }))
        });
        let total = timeline.is_none().then_some(kept.length);
        let stream = Stream::pulling(kept.source, total, Long)
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        log::debug!("playback: {id} started from the vault");
        Some(Loaded {
            stream,
            timeline,
            reach: None,
            duration: facts.duration_ms.map(Duration::from_millis),
            loudness: facts.lufs.map(|lufs| Loudness {
                lufs,
                peak: facts.peak,
            }),
        })
    }

    /// Wraps `source` so the track is kept once all of it has arrived, when keeping is on.
    /// `facts` runs then and answers `None` when the download cannot have been whole.
    fn record<S: stream::Source>(
        &self,
        id: &str,
        source: S,
        facts: impl FnOnce() -> Option<Facts> + Send + 'static,
    ) -> Recording<S> {
        match &self.shelf {
            Some(shelf) => shelf.keep(
                id,
                self.quality.id(),
                move || facts().and_then(|facts| serde_json::to_vec(&facts).ok()),
                source,
            ),
            None => Recording::through(source),
        }
    }

    /// What a download of `id` needs to ask Tidal for fresh urls in `codec`.
    fn renewal(&self, id: &str, codec: &str) -> Renewal {
        Renewal {
            client: self.client.clone(),
            id: id.to_owned(),
            codec: codec.to_owned(),
            quality: self.quality,
            left: RENEWALS,
        }
    }

    /// Starts the download a manifest describes and waits for the preroll. `facts` is what
    /// Tidal said about the track, kept beside it if it is kept. The track comes back without
    /// its length or loudness, which the caller has.
    async fn arrive(&self, id: &str, manifest: Manifest, facts: Facts) -> Result<Loaded> {
        let codec = manifest.codec().to_owned();
        let renewal = self.renewal(id, &codec);
        let (stream, timeline, urls) = match manifest {
            Manifest::File { url, .. } => {
                let file = Resuming::open(self.media.clone(), url.clone(), renewal).await?;
                let total = file.total;
                let file = self.record(id, file, move || Some(facts));
                (Stream::pulling(file, total, Long), None, vec![url])
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
                let urls: Vec<String> = std::iter::once(init).chain(media).collect();
                let parts = Parts {
                    http: self.media.clone(),
                    urls: urls.clone(),
                    first: 0,
                    next: 0,
                    sent: 0,
                    timeline: Arc::downgrade(&timeline),
                    renewal,
                };
                // The parts also end when nothing holds the timeline any more, so a track is
                // only whole when every part has a place in it.
                let held = Arc::downgrade(&timeline);
                let parts = self.record(id, parts, move || {
                    let timeline = held.upgrade()?;
                    let timeline = lock(&timeline);
                    (timeline.offsets.len() == timeline.starts.len() + 1).then(|| Facts {
                        starts_ns: timeline
                            .starts
                            .iter()
                            .map(|start| start.as_nanos() as u64)
                            .collect(),
                        offsets: timeline.offsets.clone(),
                        ..facts
                    })
                });
                (Stream::pulling(parts, None, Long), Some(timeline), urls)
            }
        };
        Ok(Loaded {
            stream: stream.primed().await?,
            timeline,
            reach: Some(Arc::new(Reach {
                runtime: tokio::runtime::Handle::current(),
                urls,
                codec,
                ahead: Mutex::new(None),
            })),
            duration: None,
            loudness: None,
        })
    }

    /// Opens a segmented track on the segment covering `at`, out of the main download when it
    /// has that segment or is about to, and out of a download started there when it is not.
    fn open_segments(
        &self,
        id: &str,
        loaded: &Loaded,
        timeline: &Mutex<Timeline>,
        at: Duration,
    ) -> Option<(Opened, Duration)> {
        if at.is_zero() {
            let decoder = forward_decoder(id, loaded.stream.reader())?;
            return Some((Trimmed::new(decoder, Duration::ZERO, None), Duration::ZERO));
        }
        let (reader, into, started) = match landing(&loaded.stream, timeline, at) {
            Some(entry) => (
                loaded.stream.spliced(entry.head, entry.graft),
                entry.into,
                None,
            ),
            None => self.leap(id, loaded, timeline, at)?,
        };
        let decoder = forward_decoder(id, reader)?;
        if let (Some(reach), Some(ahead)) = (&loaded.reach, started) {
            reach.keep(ahead);
        }
        Some((Trimmed::new(decoder, into, None), at))
    }

    /// A reader on the segment covering `at` out of a download started for a seek: the last
    /// one when it has that segment or is about to, and otherwise a new one from that segment
    /// on, which comes back to be kept once a decoder over it opens.
    fn leap(
        &self,
        id: &str,
        loaded: &Loaded,
        timeline: &Mutex<Timeline>,
        at: Duration,
    ) -> Option<(Reader<Long>, Duration, Option<Ahead>)> {
        let reach = loaded.reach.as_ref()?;
        let (index, starts) = {
            let main = lock(timeline);
            let index = main.index(at)?;
            (index, main.starts[index..].to_vec())
        };
        let held = lock(&reach.ahead).clone();
        if let Some(Ahead {
            stream,
            graft: Graft::Segments { first, timeline },
        }) = &held
            && *first <= index
            && let Some(entry) = landing(stream, timeline, at)
        {
            return Some((stream.spliced(entry.head, entry.graft), entry.into, None));
        }
        let timeline = Arc::new(Mutex::new(Timeline {
            starts,
            offsets: Vec::new(),
        }));
        let parts = Parts {
            http: self.media.clone(),
            urls: std::iter::once(reach.urls.first()?.clone())
                .chain(reach.urls.get(index + 1..)?.iter().cloned())
                .collect(),
            first: index,
            next: 0,
            sent: 0,
            timeline: Arc::downgrade(&timeline),
            renewal: self.renewal(id, &reach.codec),
        };
        let stream = {
            let _entered = reach.runtime.enter();
            Stream::pulling(parts, None, Long)
        };
        log::info!(
            "tidal: fetching {id} from segment {index} to reach {}s",
            at.as_secs()
        );
        let Some(entry) = stream.awaiting(|_, _, _| lock(&timeline).entry(at)) else {
            log::warn!(
                "playback: the tidal track {id} did not arrive at {}s: {}",
                at.as_secs(),
                stream.failed().as_deref().unwrap_or("it took too long")
            );
            return None;
        };
        let reader = stream.spliced(entry.head, entry.graft);
        let ahead = Ahead {
            stream,
            graft: Graft::Segments {
                first: index,
                timeline,
            },
        };
        Some((reader, entry.into, Some(ahead)))
    }

    /// Opens a track that is one file at `at`. A FLAC file still arriving is entered part way,
    /// since moving a seekable decoder searches the whole file and would wait for most of it.
    /// Any other file is opened seekable and moved, which waits for the bytes at `at`.
    fn open_file(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<(Opened, Duration)> {
        let stream = &loaded.stream;
        let whole = stream.done() && stream.failed().is_none();
        if !at.is_zero()
            && !whole
            && let Some(reach) = &loaded.reach
            && reach.codec.eq_ignore_ascii_case("flac")
            && let Some(length) = loaded.duration.filter(|length| !length.is_zero())
            && let Some(size) = stream.total()
            && let Some(head) = stream.awaiting(|_, spool, _| flac_head(spool)).flatten()
        {
            let file = Flac { length, size, head };
            return self.enter_file(id, loaded, reach, file, at);
        }
        let mut decoder = file_decoder(id, stream)?;
        if at.is_zero() {
            return Some((Trimmed::new(decoder, Duration::ZERO, None), Duration::ZERO));
        }
        if let Err(error) = decoder.try_seek(at) {
            log::warn!(
                "playback: cannot start the tidal track {id} at {}s: {error}",
                at.as_secs()
            );
            return None;
        }
        Some((Trimmed::new(decoder, Duration::ZERO, None), at))
    }

    /// Opens a FLAC file that is still arriving a little before `at` and reads forward to it.
    /// Where to enter is guessed from the time, so a try that lands past `at` or fails to open
    /// is made again from further back, and the last try reads forward from the top of the
    /// main download.
    fn enter_file(
        &self,
        id: &str,
        loaded: &Loaded,
        reach: &Reach,
        file: Flac,
        at: Duration,
    ) -> Option<(Opened, Duration)> {
        let span = file.size.saturating_sub(file.head as u64) as f64;
        let guesses = LEADS.iter().map(|lead| {
            let share = at.saturating_sub(*lead).as_secs_f64() / file.length.as_secs_f64();
            file.head as u64 + (span * share.clamp(0.0, 1.0)) as u64
        });
        for from in guesses.chain([file.head as u64]) {
            let Some((reader, started)) = self.graft(id, loaded, reach, file.head, from) else {
                continue;
            };
            let Some(mut decoder) = forward_decoder(id, reader) else {
                continue;
            };
            match decoder.try_seek(at) {
                Ok(()) => {
                    if let Some(ahead) = started {
                        reach.keep(ahead);
                    }
                    return Some((Trimmed::new(decoder, Duration::ZERO, None), at));
                }
                Err(error) => log::debug!(
                    "playback: entering the tidal track {id} at byte {from} missed {}s: {error}",
                    at.as_secs()
                ),
            }
        }
        log::warn!(
            "playback: cannot reach {}s in the tidal track {id}",
            at.as_secs()
        );
        None
    }

    /// A reader that serves the file's first `head` bytes and then the file from byte `from`.
    /// It reads the main download when that has the byte or is close to it, the last download
    /// a seek started when that one is, and otherwise a new download from there, which comes
    /// back to be kept once a decoder over it lands.
    fn graft(
        &self,
        id: &str,
        loaded: &Loaded,
        reach: &Reach,
        head: usize,
        from: u64,
    ) -> Option<(Reader<Long>, Option<Ahead>)> {
        let main = &loaded.stream;
        if from < main.arrived() as u64 + NEARBY {
            return Some((main.spliced(head, usize::try_from(from).ok()?), None));
        }
        let held = lock(&reach.ahead).clone();
        if let Some(Ahead {
            stream,
            graft: Graft::File { head, from: start },
        }) = held
            && start <= from
            && let Ok(into) = usize::try_from(from - start)
            && ((head + into) as u64) < stream.arrived() as u64 + NEARBY
        {
            return Some((stream.spliced(head, head + into), None));
        }
        let bytes = main.with(|_, spool| spool.bytes(0..head))?.ok()?;
        let file = Resuming::new(
            self.media.clone(),
            reach.urls.first()?.clone(),
            from,
            self.renewal(id, &reach.codec),
        );
        let stream = {
            let _entered = reach.runtime.enter();
            let headed = Headed {
                head: Some(Bytes::from(bytes)),
                file,
            };
            Stream::pulling(headed, None, Long)
        };
        log::info!("tidal: fetching {id} from byte {from} to reach a seek");
        let reader = stream.reader();
        Some((
            reader,
            Some(Ahead {
                stream,
                graft: Graft::File { head, from },
            }),
        ))
    }
}

impl Reach {
    /// Keeps `ahead` as the download the last seek started, letting go of the one before.
    fn keep(&self, ahead: Ahead) {
        *lock(&self.ahead) = Some(ahead);
    }
}

#[async_trait]
impl Fetch for Tidal {
    type Loaded = Loaded;
    type Source = Opened;

    fn name(&self) -> &'static str {
        "tidal"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Asks for the chosen quality, or the best below it the account may play, and starts the
    /// download, waiting only for the preroll, and for that no longer than `PRIMING`.
    async fn load(&self, id: &str) -> Result<Loaded> {
        if let Some(loaded) = self.recall(id).await {
            return Ok(loaded);
        }
        let playback = self.client.playback(id, self.quality).await?;
        log::info!(
            "tidal: {id} streams as {} {}",
            playback.quality,
            playback.manifest.codec()
        );
        let facts = Facts {
            duration_ms: playback
                .duration
                .map(|duration| duration.as_millis() as u64),
            lufs: playback.loudness.map(|loudness| loudness.lufs),
            peak: playback.loudness.and_then(|loudness| loudness.peak),
            starts_ns: Vec::new(),
            offsets: Vec::new(),
        };
        let arriving = self.arrive(id, playback.manifest, facts);
        let loaded = tokio::time::timeout(PRIMING, arriving)
            .await
            .context("the tidal track did not start arriving in time")??;
        Ok(Loaded {
            duration: playback.duration,
            loudness: playback.loudness,
            ..loaded
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

    /// A file reports bytes against its length. A segmented track has no length until its last
    /// segment is in, since the manifest gives no sizes, so it reports segments against their
    /// count until then.
    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        if let Some(total) = loaded.stream.total().filter(|total| *total > 0) {
            return Some(loaded.stream.arrived() as f32 / total as f32);
        }
        let timeline = lock(loaded.timeline.as_ref()?);
        let parts = timeline.starts.len() + 1;
        Some(timeline.offsets.len().min(parts) as f32 / parts as f32)
    }

    /// A segmented stream has no index for symphonia to jump around in, so a seek opens a
    /// second decoder at the target. A file is simply opened there.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        self.open_near(id, loaded, at).map(|(source, _)| source)
    }

    /// Builds a decoder placed at `at`. Only the header is read here, and a position past the
    /// download waits for one segment or one short stretch of the file, never for the way there.
    ///
    /// A segmented track is opened without its length, which would have symphonia read every
    /// fragment to build a sample table, and a position splices the init segment onto the
    /// segment that covers it, trimming what comes before the position off the front. A file
    /// that has all arrived is opened seekable and moved. `None` when the position cannot be
    /// reached, which leaves a seek where it was rather than starting the track again.
    fn open_near(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<(Opened, Duration)> {
        let (source, start) = match &loaded.timeline {
            Some(timeline) => self.open_segments(id, loaded, timeline, at),
            None => self.open_file(id, loaded, at),
        }?;
        log::debug!(
            "playback: {id} decodes at {} Hz, {} channels",
            source.sample_rate(),
            source.channels()
        );
        Some((source, start))
    }
}

/// Where to read a segmented download to land on `at`: at once when it has the segment, after a
/// wait when the segment is about to arrive, and `None` when it is further off than that.
fn landing(stream: &Stream<Long>, timeline: &Mutex<Timeline>, at: Duration) -> Option<Entry> {
    let due = {
        let timeline = lock(timeline);
        if let Some(entry) = timeline.entry(at) {
            return Some(entry);
        }
        timeline.due(at)
    };
    match due {
        true => stream.awaiting(|_, _, _| lock(timeline).entry(at)),
        false => None,
    }
}

/// Where a FLAC file's metadata ends, once enough of it has arrived to say. `Some(None)` when
/// the file does not start the way FLAC does.
fn flac_head(spool: &stream::Spool) -> Option<Option<usize>> {
    let mut marker = [0u8; 4];
    spool.read_at(0, &mut marker).ok()?;
    if &marker != b"fLaC" {
        return Some(None);
    }
    let mut at = 4usize;
    loop {
        let mut block = [0u8; 4];
        spool.read_at(at, &mut block).ok()?;
        at += 4 + u32::from_be_bytes([0, block[1], block[2], block[3]]) as usize;
        if block[0] & 0x80 != 0 {
            return (at <= spool.len()).then_some(Some(at));
        }
    }
}

/// A decoder that only reads forward and is not told the length, so opening it reads no
/// further than the first frame or fragment.
fn forward_decoder(id: &str, reader: Reader<Long>) -> Option<rodio::Decoder<Reader<Long>>> {
    decoder(
        id,
        rodio::Decoder::builder()
            .with_symphonia_decoder::<Aac>()
            .with_data(reader)
            .with_seekable(false),
    )
}

/// A seekable decoder over the whole of a track that is one file.
fn file_decoder(id: &str, stream: &Stream<Long>) -> Option<rodio::Decoder<Reader<Long>>> {
    let builder = rodio::Decoder::builder()
        .with_symphonia_decoder::<Aac>()
        .with_data(stream.reader())
        .with_seekable(true);
    let builder = match stream.total() {
        Some(total) => builder.with_byte_len(total),
        None => builder,
    };
    decoder(id, builder)
}

/// One request for a part, read whole. A part longer than `SEGMENT_CEILING` is refused before
/// it can fill memory, since nothing past here would stop it.
async fn whole(http: &reqwest::Client, url: &str) -> Result<Bytes, Failure> {
    let mut response = Failure::check(http.get(url).send().await?)?;
    let mut part = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if part.len() + chunk.len() > SEGMENT_CEILING {
            return Err(Failure::Final(anyhow!(
                "a segment runs past {SEGMENT_CEILING} bytes"
            )));
        }
        part.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(part))
}

/// The first byte and the full length a `Content-Range` header names. The length is `None`
/// when the server left it as `*`.
fn content_range(response: &reqwest::Response) -> Option<(u64, Option<u64>)> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()))
}

/// The http client a track's bytes come through.
fn media_http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(MEDIA_CONNECT)
        .read_timeout(MEDIA_STALL)
        .build()
        .context("cannot build the tidal media client")
}

/// Builds a decoder, or reports a track that will not decode.
fn decoder(
    id: &str,
    builder: rodio::decoder::DecoderBuilder<Reader<Long>>,
) -> Option<rodio::Decoder<Reader<Long>>> {
    match builder.build() {
        Ok(decoder) => Some(decoder),
        Err(error) => {
            log::warn!("playback: cannot decode the tidal track {id}: {error}");
            None
        }
    }
}
