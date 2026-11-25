//! Playback for Tidal: what [`engine`] needs that is Tidal's own.
//!
//! A lossless track is one FLAC file and plays like any other. A hi-res track is a DASH stream
//! of fMP4 segments, which [`Parts`] fetches one after another into one stream, so to the
//! decoder it is one fragmented file. The manifest says when every segment starts and
//! [`Timeline`] notes where each one lands in the stream, so a seek splices the init segment
//! onto the segment that covers the position instead of decoding its way there.
//!
//! A seek past what has arrived does not wait for the download to get there. It starts a second
//! download at the target and decodes from that: an [`Ahead`] from the covering segment, or for
//! a FLAC file a [`ranged::Reach`] download from a byte a little before the position.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use bytes::Bytes;
use engine::{self, Fetch, Loudness};
use music::vault::{Recording, Shelf};
use music::{Encoding, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};
use reqwest::header::HeaderMap;
use rodio::Source as _;
use serde::{Deserialize, Serialize};
use sound::aac::Aac;
use sound::decode::{Decoded, Media};
use sound::ranged::{self, Address, Backoff, Failure, Resuming, Span};
use sound::stream::{self, Reader, Stream};

use crate::client::TidalClient;
use crate::lock;
use crate::manifest::Manifest;

/// How many times one track asks Tidal for fresh urls when the signed ones it holds are refused.
const RENEWALS: u32 = 3;

/// How long a track may take to arrive as far as the preroll before its load fails.
const PRIMING: Duration = Duration::from_secs(20);

/// The most one segment may hold. A hi-res segment is a megabyte or two, so this only bounds
/// what a wrong answer can cost.
const SEGMENT_CEILING: usize = 64 * 1024 * 1024;

/// The most one Tidal track may spool. 24-bit FLAC at 192 kHz runs to about 40 MB a minute, so
/// this holds more than an hour and a half of it. On a 32-bit target it is as much as a size
/// can count.
const SPOOL_CEILING: u64 = 4 * 1024 * 1024 * 1024;

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

/// What a seek past the download needs to start another download at its target, and the last
/// one it started. A file or a track from the vault has none.
struct Reach {
    /// The runtime the downloads run on, which the audio thread enters to start one.
    runtime: tokio::runtime::Handle,
    /// The init segment's url, then the media segments'.
    urls: Vec<String>,
    codec: String,
    /// The download the last seek past the main one started. It is only replaced once a
    /// decoder over the next one has opened, so the decoder still playing never loses it.
    ahead: Mutex<Option<Ahead>>,
}

/// A download a seek started at its target, running to the end of the track beside the main
/// one: the init segment, then the media segments from `first` on, placed by their own timeline.
#[derive(Clone)]
struct Ahead {
    stream: Stream<Long>,
    first: usize,
    timeline: Arc<Mutex<Timeline>>,
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

/// The body of a Tidal track, taken as it comes but allowed to run as long as hi-res does.
struct Long;

/// A track arriving, and how long and how loud Tidal says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream<Long>,
    /// Where a segmented track's segments start. `None` for a track that is one file.
    timeline: Option<Arc<Mutex<Timeline>>>,
    /// What a seek past a segmented download needs. `None` for a file or a track from the vault.
    reach: Option<Arc<Reach>>,
    /// What a seek past a FLAC file's download needs. `None` for any other track or one from
    /// the vault.
    file: Option<Arc<ranged::Reach<Long>>>,
    duration: Option<Duration>,
    loudness: Option<Loudness>,
    encoding: Option<Encoding>,
}

/// Starts a playback engine for a signed-in session, which is what a [`music::ProviderSession`]
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
    /// How the kept stream is encoded. Missing from what an older Sonora kept.
    encoding: Option<Encoding>,
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
        let mut backoff = Backoff::new("tidal");
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

impl Renewal {
    /// A fresh manifest for the same track in the same codec. It fails once `RENEWALS` are
    /// spent, since a refusal that outlasts them is not an expired url.
    async fn manifest(&mut self) -> Result<Manifest> {
        if self.left == 0 {
            bail!("tidal keeps refusing the urls of {}", self.id);
        }
        self.left -= 1;
        log::info!("tidal: asking for fresh urls for {}", self.id);
        self.lookup().await
    }

    /// A fresh manifest for the same track, refused when it names another codec.
    async fn lookup(&self) -> Result<Manifest> {
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

#[async_trait]
impl ranged::Renew for Renewal {
    /// The url of a fresh manifest. The download counts and logs its own renewals.
    async fn url(&self) -> Result<(String, HeaderMap)> {
        let Manifest::File { url, .. } = self.lookup().await? else {
            bail!("tidal no longer streams the track as one file");
        };
        Ok((url, HeaderMap::new()))
    }
}

impl stream::Body for Long {
    fn ceiling(&self) -> usize {
        usize::try_from(SPOOL_CEILING).unwrap_or(usize::MAX)
    }
}

impl Factory {
    pub fn new(client: TidalClient) -> Self {
        let media = ranged::media("tidal", || client.http().clone());
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
            file: None,
            duration: facts.duration_ms.map(Duration::from_millis),
            loudness: facts.lufs.map(|lufs| Loudness {
                lufs,
                peak: facts.peak,
            }),
            encoding: facts.encoding,
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
        let (stream, timeline, reach, file) = match manifest {
            Manifest::File { url, .. } => {
                let renewal = Arc::new(renewal);
                let address = Address::new("tidal", self.media.clone(), url.clone());
                let file = Resuming::open(address.renewed(renewal.clone())).await?;
                let total = file.total();
                let file = self.record(id, file, move || Some(facts));
                let media = self.media.clone();
                let reach = codec.eq_ignore_ascii_case("flac").then(|| {
                    Arc::new(ranged::Reach::new("tidal", id, move |_| {
                        let address = Address::new("tidal", media.clone(), url.clone());
                        (address.renewed(renewal.clone()), Long)
                    }))
                });
                (Stream::pulling(file, total, Long), None, None, reach)
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
                let reach = Arc::new(Reach {
                    runtime: tokio::runtime::Handle::current(),
                    urls,
                    codec,
                    ahead: Mutex::new(None),
                });
                let stream = Stream::pulling(parts, None, Long);
                (stream, Some(timeline), Some(reach), None)
            }
        };
        Ok(Loaded {
            stream: stream.primed().await?,
            timeline,
            reach,
            file,
            duration: None,
            loudness: None,
            encoding: None,
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
    ) -> Option<(Decoded, Duration)> {
        if at.is_zero() {
            let decoder = ranged::forward("tidal", id, loaded.stream.reader())?;
            return Some((
                Decoded::trimmed(decoder, Duration::ZERO, None),
                Duration::ZERO,
            ));
        }
        let (reader, into, started) = match landing(&loaded.stream, timeline, at) {
            Some(entry) => (
                loaded.stream.spliced(entry.head, entry.graft),
                entry.into,
                None,
            ),
            None => self.leap(id, loaded, timeline, at)?,
        };
        let decoder = ranged::forward("tidal", id, reader)?;
        if let (Some(reach), Some(ahead)) = (&loaded.reach, started) {
            *lock(&reach.ahead) = Some(ahead);
        }
        Some((Decoded::trimmed(decoder, into, None), at))
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
            first,
            timeline,
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
            first: index,
            timeline,
        };
        Some((reader, entry.into, Some(ahead)))
    }

    /// Opens a track that is one file at `at`. A FLAC file still arriving is entered part way,
    /// since moving a seekable decoder searches the whole file and would wait for most of it.
    /// Any other file is opened seekable and moved, which waits for the bytes at `at`.
    fn open_file(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<(Decoded, Duration)> {
        let stream = &loaded.stream;
        if ranged::ahead(stream, at)
            && let Some(reach) = &loaded.file
            && let Some(length) = loaded.duration
            && let Some(span) = Span::measure(stream, length, ranged::flac_head)
        {
            let decoder =
                reach.enter(stream, span, at, |data| ranged::forward("tidal", id, data))?;
            return Some((Decoded::trimmed(decoder, Duration::ZERO, None), at));
        }
        let mut decoder = file_decoder(id, stream)?;
        if at.is_zero() {
            return Some((
                Decoded::trimmed(decoder, Duration::ZERO, None),
                Duration::ZERO,
            ));
        }
        if let Err(error) = decoder.try_seek(at) {
            log::warn!(
                "playback: cannot start the tidal track {id} at {}s: {error}",
                at.as_secs()
            );
            return None;
        }
        Some((Decoded::trimmed(decoder, Duration::ZERO, None), at))
    }
}

#[async_trait]
impl Fetch for Tidal {
    type Loaded = Loaded;

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
            encoding: playback.encoding,
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
            encoding: playback.encoding,
            ..loaded
        })
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        loaded.encoding
    }

    /// A track that failed because Tidal refused the session is the listener having to sign
    /// in again, not the track being unplayable.
    fn gated(&self, error: &anyhow::Error) -> bool {
        crate::auth::refused(error)
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

    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Decoded> {
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
    fn open_near(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<(Decoded, Duration)> {
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

/// A seekable decoder over the whole of a track that is one file.
fn file_decoder(id: &str, stream: &Stream<Long>) -> Option<rodio::Decoder<Media>> {
    let builder = rodio::Decoder::builder()
        .with_symphonia_decoder::<Aac>()
        .with_data(Media::new(stream.reader()))
        .with_seekable(true);
    let builder = match stream.total() {
        Some(total) => builder.with_byte_len(total),
        None => builder,
    };
    builder
        .build()
        .inspect_err(|error| log::warn!("playback: cannot decode the tidal track {id}: {error}"))
        .ok()
}

/// One request for a part, read whole. A part longer than `SEGMENT_CEILING` is refused before
/// it can fill memory, since nothing past here would stop it.
async fn whole(http: &reqwest::Client, url: &str) -> Result<Bytes, Failure> {
    let mut response = Failure::check(http.get(url).send().await?, "tidal")?;
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
