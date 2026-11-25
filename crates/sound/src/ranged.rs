//! Downloads of one file from a byte part way into it, which is what lets a seek past the
//! progressive download start reading at its target instead of waiting for the download to get
//! there.
//!
//! [`Resuming`] is one ranged download that asks again from the first missing byte when the
//! connection drops, and never splices two different files together. [`Reach`] is what a loaded
//! track keeps to serve a seek: it reads the main [`Stream`] when that has the byte or is close
//! to it, reuses the last download a seek started when that one covers it, and otherwise starts
//! a new one there, seeded with the file's head so a decoder opens over it as over the whole
//! file. A provider that knows the byte of a position calls [`Reach::exact`]. One that only knows
//! how long the track is and where its header ends describes the file as a [`Span`] and calls
//! [`Reach::enter`], which guesses a byte from the time and lets the decoder read forward to the
//! target.
//!
//! [`File`] puts those together for a plain audio file. It tells FLAC, Ogg and MP3 apart by
//! their first bytes and enters each the way its frames allow. [`ogg`] and [`mp3`] hold what is
//! particular to those two formats.

pub mod mp3;
pub mod ogg;

use std::io::{Read, Seek};
use std::iter;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT_RANGES, HeaderMap, RANGE};

use crate::aac::Aac;
use crate::decode::Media;
use crate::stream::{self, Body, Plain, Reader, Spool, Stream};

/// How long one thing keeps being asked for, with the wait between asks doubling from
/// `FIRST_WAIT` up to `LONGEST_WAIT`. A track can be a hundred or so requests, so one that fails
/// now and then is ordinary, and a connection that drops for half a minute comes back to the
/// same track.
const RETRY_WINDOW: Duration = Duration::from_secs(45);
const FIRST_WAIT: Duration = Duration::from_millis(500);
const LONGEST_WAIT: Duration = Duration::from_secs(8);

/// How long a media request may take to connect, and how long it may wait between two reads.
/// Both sit well under the patience of a reader waiting at the end of the download, so a stalled
/// request is asked again while that reader still waits.
const MEDIA_CONNECT: Duration = Duration::from_secs(5);
const MEDIA_STALL: Duration = Duration::from_secs(8);

/// How many fresh urls one download asks for once the one it holds is refused. A refusal that
/// outlasts them is not an expired url.
const RENEWALS: u32 = 3;

/// How far before a seek's target a file is entered, one lead per try. The file's bytes only
/// roughly follow its time, so the decoder enters short of the target and reads forward to land
/// on it exactly, and a try that overshot is made again from further back.
const LEADS: [Duration; 2] = [Duration::from_secs(15), Duration::from_secs(60)];

/// How far past what a download has received a seek may enter and still wait for it, rather
/// than start another download there. It is a few seconds of lossless audio, which any
/// connection that streams it brings in about as fast as a new request would.
const NEARBY: u64 = 2 * 1024 * 1024;

/// How far past a graft's byte a [`Resync`] looks for the first frame before it gives up and
/// splices at the byte itself. It holds several frames of any MPEG audio file.
const RESYNC_WINDOW: usize = 16 * 1024;

/// Finds where a file's header ends in what has arrived of it, which is what a decoder entered
/// part way needs in front of the frames. It answers `None` while too little has arrived to say
/// and `Some(None)` when the file is not in its format.
pub type Finder = fn(&Spool) -> Option<Option<usize>>;

/// Finds where the first whole frame starts in bytes read from part way into a file, so a
/// decoder entered there does not have to make sense of the tail of a frame first. It answers
/// `None` when it sees no frame it can vouch for in what it was given.
pub type Resync = fn(&[u8]) -> Option<usize>;

/// Builds the request and the body for a download that starts at a byte of the file. The byte
/// has already been rounded down to the reach's alignment.
type Start<B> = dyn Fn(u64) -> (Address, B) + Send + Sync;

/// Asks the provider for a fresh url once the one a download holds is refused, as a signed url
/// that has expired is.
#[async_trait]
pub trait Renew: Send + Sync {
    /// A fresh url for the same file, and headers that replace the download's own of the same
    /// name, for a server that expects a url to be asked for the way the client that got it
    /// asks.
    async fn url(&self) -> Result<(String, HeaderMap)>;
}

/// Where one file is fetched from, and how every request for it is made.
pub struct Address {
    /// The subsystem the download logs as, such as `"tidal"`.
    name: &'static str,
    http: reqwest::Client,
    url: String,
    /// Sent with every request, such as the `Authorization` a server needs whose urls hold no
    /// credentials of their own.
    headers: HeaderMap,
    /// Swaps in a fresh url when the server refuses the one held. Without it a refusal is final.
    renew: Option<Arc<dyn Renew>>,
    /// How many more times `renew` may be asked.
    renewals: u32,
    /// The most one request asks for, for a server that throttles long ranges. Each request
    /// then names its end and the next picks up where it stopped.
    cap: Option<u64>,
}

/// One file downloaded from byte `from` on, read as one response and asked for again from the
/// first missing byte when the connection drops. A request past the first byte has to be
/// answered with exactly the rest of the same file, so two files are never spliced together.
pub struct Resuming {
    address: Address,
    /// The byte of the file this download starts at. It is past zero for a download a seek
    /// started.
    from: u64,
    /// The response being read. `None` before the first request and between a drop or the end
    /// of a capped range and the request that carries on.
    response: Option<reqwest::Response>,
    /// How many bytes have been handed on, which is where the next request starts.
    received: u64,
    /// The byte the response being read was asked to stop before, when requests are capped.
    until: Option<u64>,
    /// How many bytes this download carries, when the server said.
    total: Option<u64>,
    /// The whole file's length, which every answer has to match once it is known.
    size: Option<u64>,
}

/// What a loaded track keeps so a seek past its download can start another download of the
/// same file at the target, and the last one it started. Shared between the load task and the
/// audio thread that opens decoders.
pub struct Reach<B> {
    /// The subsystem the downloads log as.
    name: &'static str,
    /// The track, for the log.
    id: String,
    /// The runtime the downloads run on, which the audio thread enters to start one.
    runtime: tokio::runtime::Handle,
    start: Box<Start<B>>,
    /// What a new download's first byte is rounded down to a multiple of, for a body that can
    /// only start on a block boundary.
    align: u64,
    /// Moves a graft forward onto the first frame, for a format a decoder cannot be relied on to
    /// find its way into.
    resync: Option<Resync>,
    /// How far past what a download has received a seek may enter and still wait for it.
    nearby: u64,
    /// The download the last seek past the main one started. It is only replaced once a decoder
    /// over the next one has opened, so the decoder still playing never loses it.
    ahead: Mutex<Option<Ahead<B>>>,
}

/// A download a seek started at its target, running to the end of the track beside the main
/// one: the file's first `head` bytes, then the file from byte `from` on.
struct Ahead<B> {
    stream: Stream<B>,
    head: usize,
    from: u64,
}

/// A file format a seek can enter part way, told apart by the file's first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Flac,
    Ogg,
    Mp3,
}

/// A plain audio file a seek can enter part way: its format, and the reach that fetches it from
/// a byte. Shared between the load task and the audio thread that opens decoders.
pub struct File<B = Plain> {
    kind: Kind,
    reach: Reach<B>,
}

/// What placing a time in a file by its bytes goes on: how long the track runs, how long the
/// file is, and where its header ends and the frames begin.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub length: Duration,
    pub size: u64,
    pub head: usize,
}

/// The waits between asks for one thing, and when the asking stops.
pub struct Backoff {
    /// The subsystem the waits log as.
    name: &'static str,
    since: Instant,
    wait: Duration,
}

/// Why a media request failed, which decides what is tried next.
pub enum Failure {
    /// The url was refused or has gone, which a fresh url may cure.
    Expired(anyhow::Error),
    /// The connection or the server failed, which waiting may cure.
    Passing(anyhow::Error),
    /// Asking again would get the same answer.
    Final(anyhow::Error),
}

impl Address {
    pub fn new(name: &'static str, http: reqwest::Client, url: String) -> Self {
        Self {
            name,
            http,
            url,
            headers: HeaderMap::new(),
            renew: None,
            renewals: RENEWALS,
            cap: None,
        }
    }

    /// Sends `headers` with every request. reqwest drops an `Authorization` header on a
    /// redirect to another host, so credentials stay with the server they are for.
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers.extend(headers);
        self
    }

    /// Asks `renew` for a fresh url whenever the server answers 403 or 410, a few times at most.
    pub fn renewed(mut self, renew: Arc<dyn Renew>) -> Self {
        self.renew = Some(renew);
        self
    }

    /// Asks for at most `bytes` per request and chains the requests. The server then has to
    /// state the file's full size, or the download fails rather than guess where the file ends.
    pub fn capped(mut self, bytes: u64) -> Self {
        self.cap = Some(bytes.max(1));
        self
    }
}

impl Resuming {
    /// A download of the file from byte `from` on, which makes its first request when it is
    /// first read.
    pub fn new(address: Address, from: u64) -> Self {
        Self {
            address,
            from,
            response: None,
            received: 0,
            until: None,
            total: None,
            size: None,
        }
    }

    /// A download of the whole file, with its first request made, so its length is known before
    /// it is handed to a [`Stream`]. A failed first request is asked again the way a dropped
    /// connection is.
    pub async fn open(address: Address) -> Result<Self> {
        let name = address.name;
        let mut file = Self::new(address, 0);
        file.connect()
            .await
            .with_context(|| format!("cannot stream the {name} track"))?;
        Ok(file)
    }

    /// How many bytes this download carries, once the first response said.
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// Opens a response from the first byte not yet received. A failed connection or server is
    /// asked again with a growing wait, and a refused url is renewed first. The first response
    /// says how long the download and the file are.
    async fn connect(&mut self) -> Result<()> {
        let name = self.address.name;
        let mut backoff = Backoff::new(name);
        loop {
            match self.request().await {
                Ok((response, ranged, until)) => {
                    if self.received == 0 {
                        self.size = match ranged {
                            true => content_range(&response).and_then(|(_, size)| size),
                            false => response.content_length(),
                        };
                        self.total = match self.address.cap {
                            Some(_) => self.size.map(|size| size.saturating_sub(self.from)),
                            None => response.content_length(),
                        };
                    }
                    self.until = until;
                    self.response = Some(response);
                    return Ok(());
                }
                Err(Failure::Expired(error)) => {
                    let Some(renew) = &self.address.renew else {
                        return Err(error);
                    };
                    if self.address.renewals == 0 {
                        return Err(error.context(format!("{name} keeps refusing the url")));
                    }
                    self.address.renewals -= 1;
                    log::info!("{name}: asking for a fresh url after {error:#}");
                    let (url, headers) = renew.url().await?;
                    self.address.url = url;
                    self.address.headers.extend(headers);
                }
                Err(Failure::Passing(error)) => backoff.wait(error).await?,
                Err(Failure::Final(error)) => return Err(error),
            }
        }
    }

    /// One request from the first byte not yet received. It answers the response, whether it
    /// was asked for a range, and the byte a capped request was asked to stop before.
    async fn request(&self) -> Result<(reqwest::Response, bool, Option<u64>), Failure> {
        let name = self.address.name;
        let at = self.from + self.received;
        let until = self.address.cap.map(|cap| {
            let end = at.saturating_add(cap);
            self.size.map_or(end, |size| end.min(size))
        });
        let mut request = self
            .address
            .http
            .get(&self.address.url)
            .headers(self.address.headers.clone());
        let Some(range) = range(at, until) else {
            let response = Failure::check(request.send().await?, name)?;
            return Ok((response, false, None));
        };
        request = request.header(RANGE, range);
        let response = Failure::check(request.send().await?, name)?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Failure::Final(match at {
                0 => anyhow!("the {name} file server does not answer ranges"),
                _ => anyhow!("the {name} file server cannot start a download part way"),
            }));
        }
        let answered = content_range(&response);
        if self.address.cap.is_some() && answered.is_none_or(|(_, size)| size.is_none()) {
            return Err(Failure::Final(anyhow!(
                "the {name} file server does not say how long the file is"
            )));
        }
        let lines_up = answered.is_some_and(|(start, size)| {
            let unsaid = self.received == 0 && size.is_none();
            start == at && (unsaid || self.size.is_none() || size == self.size)
        });
        match lines_up {
            true => Ok((response, true, until)),
            false => Err(Failure::Final(anyhow!(
                "the {name} file server answered somewhere other than byte {at}"
            ))),
        }
    }
}

impl stream::Source for Resuming {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        let name = self.address.name;
        let mut backoff = Backoff::new(name);
        loop {
            let Some(response) = &mut self.response else {
                self.connect()
                    .await
                    .with_context(|| format!("cannot resume the {name} track"))?;
                continue;
            };
            let error = match response.chunk().await {
                Ok(Some(chunk)) => {
                    self.received += chunk.len() as u64;
                    return Ok(Some(chunk));
                }
                Ok(None) => match (self.total, self.until) {
                    (Some(total), _) if self.received >= total => return Ok(None),
                    (_, Some(until)) if self.from + self.received == until => {
                        self.response = None;
                        backoff = Backoff::new(name);
                        continue;
                    }
                    (Some(total), _) => {
                        anyhow!(
                            "the {name} file ended {} bytes early",
                            total - self.received
                        )
                    }
                    (None, _) => return Ok(None),
                },
                Err(error) => error.without_url().into(),
            };
            self.response = None;
            backoff
                .wait(error)
                .await
                .with_context(|| format!("cannot stream the {name} track"))?;
        }
    }
}

impl<B: Body> Reach<B> {
    /// A reach for track `id` whose downloads `start` builds from a byte. It has to be made on
    /// the tokio runtime, which is where its downloads later run.
    pub fn new(
        name: &'static str,
        id: &str,
        start: impl Fn(u64) -> (Address, B) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            id: id.to_owned(),
            runtime: tokio::runtime::Handle::current(),
            start: Box::new(start),
            align: 1,
            resync: None,
            nearby: NEARBY,
            ahead: Mutex::new(None),
        }
    }

    /// Waits for a download that is less than `bytes` short of a seek's target rather than
    /// starting another, in place of the `NEARBY` default, for a stream whose bytes hold more
    /// time than lossless audio does.
    pub fn nearby(mut self, bytes: u64) -> Self {
        self.nearby = bytes;
        self
    }

    /// Starts every new download on a multiple of `align` bytes, for a body that decrypts the
    /// file in blocks counted from its start. A reader still lands on the byte it asked for.
    pub fn aligned(mut self, align: u64) -> Self {
        self.align = align.max(1);
        self
    }

    /// Opens a decoder with `open` over the file's first `head` bytes followed by the file from
    /// byte `from`. It reads `main` when that has the byte or is close to it, the last download
    /// a seek started when that one is and has not broken, and otherwise a new download from
    /// there, which is kept once `open` succeeds. A reach for MP3 moves the splice onto the next
    /// frame.
    pub fn exact<D>(
        &self,
        main: &Stream<B>,
        head: usize,
        from: u64,
        open: impl FnOnce(Reader<B>) -> Option<D>,
    ) -> Option<D> {
        let covered = self.covers(main, from);
        self.graft(main, head, from, covered, open)
    }

    /// What [`exact`](Self::exact) does, for a caller that already knows whether `main` covers
    /// `from`, so the wait for a nearby byte is not made twice.
    fn graft<D>(
        &self,
        main: &Stream<B>,
        head: usize,
        from: u64,
        covered: bool,
        open: impl FnOnce(Reader<B>) -> Option<D>,
    ) -> Option<D> {
        let (stream, at, started) = self.locate(main, head, from, covered)?;
        let at = self.resync(&stream, at);
        let decoder = open(stream.spliced(head, at))?;
        if let Some(ahead) = started {
            *lock(&self.ahead) = Some(ahead);
        }
        Some(decoder)
    }

    /// The download that holds byte `from` or soon will, where that byte sits in its spool, and
    /// that download again when one had to be started for it. `covered` says whether `main`
    /// covers `from`.
    fn locate(
        &self,
        main: &Stream<B>,
        head: usize,
        from: u64,
        covered: bool,
    ) -> Option<(Stream<B>, usize, Option<Ahead<B>>)> {
        if covered {
            return Some((main.clone(), usize::try_from(from).ok()?, None));
        }
        let held = lock(&self.ahead).clone();
        if let Some(Ahead {
            stream,
            head,
            from: start,
        }) = held
            && start <= from
            && stream.failed().is_none()
            && let Ok(into) = usize::try_from(from - start)
            && self.covers(&stream, (head + into) as u64)
        {
            return Some((stream, head + into, None));
        }
        let bytes = main.with(|_, spool| spool.bytes(0..head))?.ok()?;
        let start = from - from % self.align;
        let into = usize::try_from(from - start).ok()?;
        let (address, body) = (self.start)(start);
        let mut file = Resuming::new(address, start);
        file.size = main.total().filter(|_| !main.done());
        let stream = {
            let _entered = self.runtime.enter();
            Stream::seeded(&bytes, file, None, body)
        };
        log::info!(
            "{}: fetching {} from byte {start} to reach a seek",
            self.name,
            self.id
        );
        let ahead = Ahead {
            stream: stream.clone(),
            head,
            from: start,
        };
        Some((stream, head + into, Some(ahead)))
    }

    /// Whether spool byte `at` of `stream` has arrived. A download still running and less than
    /// `nearby` short is waited for, and one that breaks, stalls or ends before it gets there
    /// does not cover the byte.
    fn covers(&self, stream: &Stream<B>, at: u64) -> bool {
        let arrived = stream.arrived() as u64;
        if at < arrived || at >= arrived + self.nearby {
            return at < arrived;
        }
        let reached = stream.awaiting(|_, spool, _| (spool.len() as u64 > at).then_some(()));
        reached.is_some()
    }

    /// Where the first frame at or after spool byte `at` of `stream` starts, once enough has
    /// arrived to say. It is `at` itself when the reach does not resync, when no frame turns up
    /// in [`RESYNC_WINDOW`], or when the bytes stop arriving.
    fn resync(&self, stream: &Stream<B>, at: usize) -> usize {
        let Some(resync) = self.resync else {
            return at;
        };
        let wanted = at.saturating_add(RESYNC_WINDOW);
        let found = stream.awaiting(|body, spool, complete| {
            let end = body.limit(spool, complete).min(wanted);
            let full = end == wanted || complete;
            if end <= at {
                return full.then_some(at);
            }
            if body.ready(spool, at..end).is_err() {
                return Some(at);
            }
            let Ok(window) = spool.view(at..end) else {
                return Some(at);
            };
            match resync(&window) {
                Some(found) => Some(at + found),
                None => full.then_some(at),
            }
        });
        found.unwrap_or(at)
    }

    /// Opens a decoder with `open` a little before `at` in a file that is still arriving and
    /// moves it to `at`, for a format whose frames say where they are in the track. Where to
    /// enter is guessed from the time, so a try that lands past `at` or fails to open is made
    /// again from further back, and the last try reads forward from the top of `main`. A `main`
    /// that broke gets a try from where it broke before that, since no decoder over it reads on
    /// past there.
    pub fn enter<D: rodio::Source>(
        &self,
        main: &Stream<B>,
        span: Span,
        at: Duration,
        mut open: impl FnMut(Reader<B>) -> Option<D>,
    ) -> Option<D> {
        let guesses = LEADS.iter().map(|lead| span.byte(at.saturating_sub(*lead)));
        let broke = iter::once_with(|| main.failed().map(|_| main.arrived() as u64)).flatten();
        let mut tries = guesses.chain(broke).chain([span.head as u64]);
        let entered = tries.find_map(|from| {
            self.exact(main, span.head, from, |reader| {
                let mut decoder = open(reader)?;
                let seek = decoder.try_seek(at).inspect_err(|error| {
                    log::debug!(
                        "playback: entering the {} track {} at byte {from} missed {}s: {error}",
                        self.name,
                        self.id,
                        at.as_secs()
                    )
                });
                seek.ok().map(|()| decoder)
            })
        });
        if entered.is_none() {
            log::warn!(
                "playback: cannot reach {}s in the {} track {}",
                at.as_secs(),
                self.name,
                self.id
            );
        }
        entered
    }
}

impl<B> Clone for Ahead<B> {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.clone(),
            head: self.head,
            from: self.from,
        }
    }
}

impl Kind {
    /// The format a file is in, from its first four bytes in `spool`. `None` for a file in none
    /// of these formats, or one with fewer than four bytes in.
    pub fn detect(spool: &Spool) -> Option<Self> {
        let mut magic = [0u8; 4];
        spool.read_at(0, &mut magic).ok()?;
        match &magic {
            b"fLaC" => Some(Self::Flac),
            b"OggS" => Some(Self::Ogg),
            _ if magic.starts_with(b"ID3") || mp3::Header::parse(&magic).is_some() => {
                Some(Self::Mp3)
            }
            _ => None,
        }
    }
}

impl<B: Body> File<B> {
    /// The file `main` is downloading, when its first bytes are in a format a seek can enter
    /// part way, with `reach` fitted to that format.
    pub fn detect(main: &Stream<B>, mut reach: Reach<B>) -> Option<Arc<Self>> {
        let kind = main.with(|_, spool| Kind::detect(spool)).flatten()?;
        if kind == Kind::Mp3 {
            reach.resync = Some(mp3::sync);
        }
        log::debug!("{}: {} can seek ahead as {kind:?}", reach.name, reach.id);
        Some(Arc::new(Self { kind, reach }))
    }

    /// Opens a decoder with `open` at `at` in a track of `length` that `main` is still
    /// downloading, and says where it starts. FLAC and Ogg frames carry their time, so the
    /// decoder lands on `at` exactly. An MP3 frame does not, so its start is an estimate from
    /// the Xing table or the bitrate, and an MP3 position the download has nearly reached is
    /// left to the caller, which lands on it exactly by counting frames from the top.
    ///
    /// `None` leaves the caller to open the file the way it would without a reach, as it does
    /// for a seek to the top or into a track already whole.
    pub fn enter<D: rodio::Source>(
        &self,
        main: &Stream<B>,
        length: Duration,
        at: Duration,
        open: impl FnMut(Reader<B>) -> Option<D>,
    ) -> Option<(D, Duration)> {
        if !ahead(main, at) {
            return None;
        }
        let finder = match self.kind {
            Kind::Flac => flac_head,
            Kind::Ogg => ogg::head,
            Kind::Mp3 => {
                let layout = mp3::Layout::measure(main, length)?;
                let from = layout.byte(at);
                if self.reach.covers(main, from) {
                    return None;
                }
                let decoder = self.reach.graft(main, layout.head, from, false, open)?;
                let start = layout.time(from);
                log::debug!(
                    "playback: entered the {} track {} at byte {from}, taken as {:.1}s for {:.1}s",
                    self.reach.name,
                    self.reach.id,
                    start.as_secs_f64(),
                    at.as_secs_f64()
                );
                return Some((decoder, start));
            }
        };
        let span = Span::measure(main, length, finder)?;
        let decoder = self.reach.enter(main, span, at, open)?;
        Some((decoder, at))
    }
}

impl Span {
    /// The span of a file that `main` is downloading, once it knows the file's size and
    /// `finder` has found where its header ends. This waits for the header, bounded by the
    /// patience of a read. `None` for a track of no known length or a file not in the format.
    pub fn measure<B: Body>(main: &Stream<B>, length: Duration, finder: Finder) -> Option<Self> {
        if length.is_zero() {
            return None;
        }
        let size = main.total()?;
        let head = main.awaiting(|_, spool, _| finder(spool)).flatten()?;
        Some(Self { length, size, head })
    }

    /// The byte `at` falls on if the frames spread evenly over the file after its header.
    pub fn byte(&self, at: Duration) -> u64 {
        let frames = self.size.saturating_sub(self.head as u64) as f64;
        let share = at.as_secs_f64() / self.length.as_secs_f64();
        self.head as u64 + (frames * share.clamp(0.0, 1.0)) as u64
    }
}

impl Backoff {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            since: Instant::now(),
            wait: FIRST_WAIT,
        }
    }

    /// Waits before the next ask, or hands `error` back once `RETRY_WINDOW` has passed since
    /// the first one.
    pub async fn wait(&mut self, error: anyhow::Error) -> Result<()> {
        if self.since.elapsed() >= RETRY_WINDOW {
            return Err(error);
        }
        log::debug!(
            "{}: asking again in {:?} after {error:#}",
            self.name,
            self.wait
        );
        tokio::time::sleep(self.wait).await;
        self.wait = (self.wait * 2).min(LONGEST_WAIT);
        Ok(())
    }
}

impl Failure {
    /// Sorts a response by its status, passing a successful one through. `name` is the
    /// subsystem the error names.
    pub fn check(response: reqwest::Response, name: &str) -> Result<reqwest::Response, Self> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let error = anyhow!("the {name} media server answered {status}");
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
    /// wrong on the way and may not again. The url is dropped, since it is often signed and the
    /// error ends up in the log.
    fn from(error: reqwest::Error) -> Self {
        let lasting = error.is_builder();
        let error = error.without_url().into();
        match lasting {
            true => Self::Final(error),
            false => Self::Passing(error),
        }
    }
}

/// An http client for media bytes, which gives up on a stalled request sooner than an api
/// client would, so a download asks again while its reader still waits. `fallback` stands in
/// when it cannot be built.
pub fn media(name: &str, fallback: impl FnOnce() -> reqwest::Client) -> reqwest::Client {
    let built = reqwest::Client::builder()
        .connect_timeout(MEDIA_CONNECT)
        .read_timeout(MEDIA_STALL)
        .build();
    built.unwrap_or_else(|error| {
        log::warn!("{name}: cannot build the media client, streaming through another: {error}");
        fallback()
    })
}

/// Whether a seek to `at` in the track `main` is downloading may start a download of its own,
/// which it may unless it goes to the top or the track is already whole.
pub fn ahead<B: Body>(main: &Stream<B>, at: Duration) -> bool {
    !at.is_zero() && !(main.done() && main.failed().is_none())
}

/// A decoder over `data` that only reads forward and is not told the length, so opening it part
/// way into a file reads no further than the first frame. `name` and `id` name the track in the
/// warning a file that will not decode leaves.
pub fn forward<R>(name: &str, id: &str, data: R) -> Option<rodio::Decoder<Media>>
where
    R: Read + Seek + Send + Sync + 'static,
{
    rodio::Decoder::builder()
        .with_symphonia_decoder::<Aac>()
        .with_data(Media::new(data))
        .with_seekable(false)
        .build()
        .inspect_err(|error| log::warn!("playback: cannot decode the {name} track {id}: {error}"))
        .ok()
}

/// Whether `response` says how long the file is and that its server answers byte ranges, which
/// a download started part way into the file needs.
pub fn ranges(response: &reqwest::Response) -> bool {
    response.content_length().is_some()
        && response
            .headers()
            .get_all(ACCEPT_RANGES)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .any(|unit| unit.trim().eq_ignore_ascii_case("bytes"))
}

/// Where a FLAC file's metadata ends, once enough of it has arrived to say. `Some(None)` when
/// the file does not start the way FLAC does.
pub fn flac_head(spool: &Spool) -> Option<Option<usize>> {
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

/// The `Range` header for a request from byte `at`, stopping before `until` when that is set.
/// `None` for an uncapped request from the first byte, which asks for the plain file.
fn range(at: u64, until: Option<u64>) -> Option<String> {
    match until {
        Some(until) => Some(format!("bytes={at}-{}", until.saturating_sub(1).max(at))),
        None if at > 0 => Some(format!("bytes={at}-")),
        None => None,
    }
}

/// The first byte and the full length a response's `Content-Range` header names.
fn content_range(response: &reqwest::Response) -> Option<(u64, Option<u64>)> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    parse_content_range(value)
}

/// The first byte and the full length a `Content-Range` value names. The length is `None` when
/// the server left it as `*`.
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()))
}

/// Locks a mutex whose guarded value is still sound after a panic elsewhere.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A spool in memory holding `bytes`, for the format tests.
#[cfg(test)]
fn spool(bytes: &[u8]) -> Spool {
    let mut spool = Spool::in_memory(None);
    spool.append(bytes).unwrap();
    spool
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FLAC header with one 34-byte STREAMINFO block and a last 4-byte padding block.
    fn flac() -> Vec<u8> {
        let mut file = b"fLaC".to_vec();
        file.extend([0x00, 0, 0, 34]);
        file.extend([0u8; 34]);
        file.extend([0x81, 0, 0, 4]);
        file.extend([0u8; 4]);
        file.extend([0xff, 0xf8, 0x69, 0x08]);
        file
    }

    #[test]
    fn a_content_range_names_its_start_and_the_file_size() {
        assert_eq!(
            parse_content_range("bytes 100-199/1000"),
            Some((100, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 100-199/*"), Some((100, None)));
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }

    #[test]
    fn a_capped_request_names_its_last_byte() {
        assert_eq!(range(0, None), None, "the plain file");
        assert_eq!(range(500, None).as_deref(), Some("bytes=500-"));
        assert_eq!(range(0, Some(100)).as_deref(), Some("bytes=0-99"));
        assert_eq!(range(500, Some(600)).as_deref(), Some("bytes=500-599"));
    }

    #[test]
    fn a_flac_head_ends_after_the_last_metadata_block() {
        let file = flac();
        assert_eq!(flac_head(&spool(&file)), Some(Some(50)));
        assert_eq!(flac_head(&spool(&file[..20])), None, "still arriving");
        assert_eq!(flac_head(&spool(b"OggS\0\0\0\0")), Some(None));
    }

    #[test]
    fn a_kind_is_told_by_the_first_bytes() {
        assert_eq!(Kind::detect(&spool(&flac())), Some(Kind::Flac));
        assert_eq!(Kind::detect(&spool(b"OggS\0\x02")), Some(Kind::Ogg));
        assert_eq!(Kind::detect(&spool(b"ID3\x04\0\0")), Some(Kind::Mp3));
        assert_eq!(
            Kind::detect(&spool(&[0xff, 0xfb, 0x90, 0x00])),
            Some(Kind::Mp3)
        );
        assert_eq!(Kind::detect(&spool(b"\0\0\0\x20ftypM4A ")), None);
        assert_eq!(Kind::detect(&spool(b"fLa")), None, "too short to tell");
    }

    #[test]
    fn a_span_places_time_evenly_after_the_header() {
        let span = Span {
            length: Duration::from_secs(100),
            size: 1100,
            head: 100,
        };
        assert_eq!(span.byte(Duration::ZERO), 100);
        assert_eq!(span.byte(Duration::from_secs(50)), 600);
        assert_eq!(
            span.byte(Duration::from_secs(500)),
            1100,
            "clamped to the end"
        );
    }
}
