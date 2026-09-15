//! One playback engine, for every provider whose tracks are a file to decode.
//!
//! Two threads share the work the way the Spotify path does: a tokio thread fetches, and one
//! audio thread decodes and feeds [`crate::sink::Paced`]. Neither the audio callback nor the
//! runtime ever waits on the network, so a track starts as soon as its first seconds are in.
//! The engine owns everything that is the same whoever the provider is: the queue of commands,
//! the preload, the gapless join, where a position is reported from, and what a lost output
//! device does.
//!
//! A provider supplies [`Fetch`]: how to get a track and how to open a decoder over it. Three
//! do today, and they differ in about sixty lines each. Subsonic hands over a plain response,
//! Deezer decrypts Blowfish stripes as they arrive, and Apple Music indexes CENC fragments and
//! decrypts each sample through a CDM as the decoder reaches it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use rodio::Source as _;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::audio::{Chain, Volume};
use crate::sink::{Cue, Paced, packet};
use crate::spectrum::Spectrum;
use crate::{PlaybackConfig, PlaybackEvent, PlaybackEvents, Player};

/// How many frames the decoder hands over at a time. Small enough that a skip is heard at once,
/// large enough that the queue is not rebuilt for every few samples.
const CHUNK: usize = 4096;

/// What the engine needs from a provider, and all it needs.
///
/// `load` runs on the engine's tokio runtime; `open` runs on the audio thread, where it must
/// not wait on anything but bytes already arriving.
#[async_trait]
pub trait Fetch: Send + Sync + 'static {
    /// What a fetch produced. It is cloned to queue a track behind the one playing and to keep
    /// it for the load that follows, so it should be a handle to a download rather than the
    /// bytes themselves.
    type Loaded: Clone + Send + 'static;

    /// What a decoder over one of those yields.
    type Source: rodio::Source + Send + 'static;

    /// What to call this engine's threads.
    fn name(&self) -> &'static str;

    /// Fetches a track: enough of it to start decoding, not all of it.
    async fn load(&self, id: &str) -> Result<Self::Loaded>;

    /// How long the track is, when the fetch learned it. The engine announces it so a seek bar
    /// has something to draw before the decoder knows.
    fn length(&self, _loaded: &Self::Loaded) -> Option<Duration> {
        None
    }

    /// Opens a decoder placed at `at`. `None` means the track cannot be played, which the
    /// engine reports as unavailable.
    fn open(&self, id: &str, loaded: &Self::Loaded, at: Duration) -> Option<Self::Source>;

    /// Whether a seek opens a second decoder at the target rather than moving the running one.
    ///
    /// A fragmented stream has no index for a decoder to jump around in: asking one to seek
    /// leaves it producing nothing, which looks exactly like a track that ended.
    fn reopen_to_seek(&self) -> bool {
        false
    }
}

/// Starts an engine for `fetch`. The pair it hands back is what a `PlaybackFactory` returns.
pub fn start<F: Fetch>(
    fetch: F,
    config: PlaybackConfig,
) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
    let (commands, command_rx) = unbounded_channel();
    let (events, event_rx) = unbounded_channel();
    let spectrum = Spectrum::new();
    let engine_spectrum = spectrum.clone();
    let name = fetch.name();
    let spawned = std::thread::Builder::new()
        .name(format!("{name}-playback"))
        .spawn(move || run(Arc::new(fetch), config, command_rx, events, engine_spectrum));
    if let Err(error) = spawned {
        log::error!("playback: cannot spawn the {name} engine thread: {error}");
    }
    (
        Box::new(Handle { commands, spectrum }),
        Box::new(Events(event_rx)),
    )
}

enum Command {
    Load {
        id: String,
        at: Option<Duration>,
        play: bool,
        seamless: bool,
    },
    Preload {
        id: String,
        segue: bool,
    },
    Play,
    Pause,
    Seek(Duration),
    Gain(f32),
}

/// What the audio thread is told to do. Everything that touches the network has happened by the
/// time one of these is sent.
enum Job<F: Fetch> {
    /// Decode this track from `at`, dropping whatever was playing.
    Play {
        id: String,
        loaded: F::Loaded,
        at: Duration,
        playing: bool,
    },
    /// Line this track up behind the current one, for a gapless join.
    Queue {
        id: String,
        loaded: F::Loaded,
    },
    Resume,
    Pause,
    /// Move within the track being decoded. The bytes are already arriving, so this is local.
    Seek(Duration),
    Gain(f32),
}

struct Handle {
    commands: UnboundedSender<Command>,
    spectrum: Spectrum,
}

impl Player for Handle {
    fn load(&self, track_id: &str, at: Duration, seamless: bool) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_owned(),
                at: (!at.is_zero()).then_some(at),
                play: true,
                seamless,
            })
            .context("cannot reach the playback engine")
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_owned(),
                at: Some(at),
                play: false,
                seamless: false,
            })
            .context("cannot reach the playback engine")
    }

    fn preload(&self, track_id: &str, segue: bool) -> Result<()> {
        self.commands
            .send(Command::Preload {
                id: track_id.to_owned(),
                segue,
            })
            .context("cannot reach the playback engine")
    }

    fn play(&self) {
        self.commands.send(Command::Play).ok();
    }

    fn pause(&self) {
        self.commands.send(Command::Pause).ok();
    }

    fn seek(&self, position: Duration) {
        self.commands.send(Command::Seek(position)).ok();
    }

    fn set_gain(&self, gain: f32) {
        self.commands.send(Command::Gain(gain)).ok();
    }

    fn spectrum(&self) -> Option<Spectrum> {
        Some(self.spectrum.clone())
    }
}

struct Events(UnboundedReceiver<PlaybackEvent>);

#[async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        self.0.recv().await
    }
}

/// What the fetch of one track came back with, and whether anything still wants it.
struct Fetched<F: Fetch> {
    epoch: u64,
    id: String,
    segue: bool,
    result: Result<F::Loaded>,
}

/// What the audio thread did when a track ran out: which one ended, and which one, if any, it
/// went on to decode from the queue. The engine follows this rather than the queue it handed
/// over, because a queued track can arrive after the end it was meant for, or fail to open.
struct Joined {
    ended: String,
    next: Option<String>,
}

/// Moves `current` on to what the audio thread is decoding, as long as the track it reports
/// ending is still the engine's current one. A load since then has already moved on.
fn settle(current: &mut Option<String>, join: Joined) {
    if current.as_deref() == Some(join.ended.as_str()) {
        *current = join.next;
    }
}

fn run<F: Fetch>(
    fetch: Arc<F>,
    config: PlaybackConfig,
    commands: UnboundedReceiver<Command>,
    events: UnboundedSender<PlaybackEvent>,
    spectrum: Spectrum,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("playback: cannot build the engine runtime: {error}");
            return;
        }
    };
    runtime.block_on(engine_loop(fetch, config, commands, events, spectrum));
}

async fn engine_loop<F: Fetch>(
    fetch: Arc<F>,
    config: PlaybackConfig,
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<PlaybackEvent>,
    spectrum: Spectrum,
) {
    let (cue, mut written) = Cue::new();
    let (changed, mut gone) = unbounded_channel();
    // The user's gain, the shared equalizer and the spectrum tap: what every engine hands the
    // output. A provider that knows a track's loudness applies it inside its own decoder.
    let chain = Chain {
        volume: Volume::new(config.gain),
        equalizer: config.equalizer.clone(),
        spectrum,
    };
    let (jobs, job_rx) = channel::<Job<F>>();
    let (joins, mut joined) = unbounded_channel::<Joined>();

    let audio_cue = cue.clone();
    let audio_events = events.clone();
    let audio_fetch = fetch.clone();
    let interval = config.position_interval;
    let spawned = std::thread::Builder::new()
        .name(format!("{}-audio", fetch.name()))
        .spawn(move || {
            audio_loop(
                audio_fetch,
                job_rx,
                audio_cue,
                chain,
                changed,
                joins,
                audio_events,
                interval,
            )
        });
    if let Err(error) = spawned {
        log::error!("playback: cannot spawn the audio thread: {error}");
        return;
    }

    // what the next admitted write means, held until the audio actually reaches the output
    let mut announcing: Option<PlaybackEvent> = None;
    let mut epoch = 0u64;
    let mut awaited: Option<u64> = None;
    let mut inflight: Option<tokio::task::AbortHandle> = None;
    let mut ahead: Option<(String, F::Loaded)> = None;
    // the track the audio thread is decoding, as far as the engine knows: set by a load, then
    // moved on by the audio thread's own word when a track runs out
    let mut current: Option<String> = None;
    // handed to the audio thread for a gapless join, so a later seamless load knows it started
    let mut segued: Option<String> = None;
    // A segue that arrived before the track it follows had even started decoding. Its `Queue`
    // has to wait for that track's `Play`, which clears whatever was queued behind the last
    // one, or the join is dropped before it can happen.
    let mut waiting: Option<(String, F::Loaded)> = None;
    // whether the listener wants sound. A play or pause during a fetch has to survive it, or
    // pressing play while a track loads would be forgotten by the time it arrives.
    let mut wanted = false;
    let (fetched, mut arrivals) = unbounded_channel::<Fetched<F>>();

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Load { id, at, play, seamless } => {
                        let joined = segued.as_deref() == Some(id.as_str());
                        if seamless
                            && at.is_none()
                            && (current.as_deref() == Some(id.as_str()) || joined)
                        {
                            // already decoding, either still or through a gapless join
                            if joined {
                                current = segued.take();
                            }
                            jobs.send(Job::Resume).ok();
                            continue;
                        }
                        segued = None;
                        waiting = None;
                        epoch += 1;
                        if let Some(handle) = inflight.take() {
                            handle.abort();
                        }
                        cue.clear();
                        current = Some(id.clone());
                        let position = at.unwrap_or_default();
                        wanted = play;
                        events.send(PlaybackEvent::Loading {
                            id: Some(id.clone()),
                            at: position,
                        }).ok();
                        announcing = Some(match play {
                            true => PlaybackEvent::Playing { id: Some(id.clone()), at: position },
                            false => PlaybackEvent::Paused { id: Some(id.clone()), at: position },
                        });

                        let held = ahead
                            .take_if(|(cached, _)| *cached == id)
                            .map(|(_, loaded)| loaded);
                        match held {
                            // Fetched already, so this starts on the next read.
                            Some(loaded) => {
                                announce_length(&events, &id, fetch.length(&loaded));
                                jobs.send(Job::Play {
                                    id,
                                    loaded,
                                    at: position,
                                    playing: wanted,
                                }).ok();
                                queue_segue(&jobs, &mut waiting);
                            }
                            None => {
                                ahead = None;
                                awaited = Some(epoch);
                                inflight = Some(spawn(&fetch, id, epoch, false, &fetched));
                            }
                        }
                    }
                    Command::Preload { id, segue } => {
                        let known = current.as_deref() == Some(id.as_str())
                            || ahead.as_ref().is_some_and(|(cached, _)| *cached == id);
                        if known || current.is_none() {
                            continue;
                        }
                        spawn(&fetch, id, epoch, segue, &fetched);
                    }
                    Command::Play => {
                        wanted = true;
                        announcing = announcing.map(playing_now);
                        jobs.send(Job::Resume).ok();
                    }
                    Command::Pause => {
                        wanted = false;
                        announcing = announcing.map(paused_now);
                        jobs.send(Job::Pause).ok();
                    }
                    Command::Seek(position) => {
                        if let Some(id) = current.clone() {
                            cue.clear();
                            announcing = Some(PlaybackEvent::Seeked {
                                id: Some(id),
                                at: position,
                            });
                            jobs.send(Job::Seek(position)).ok();
                        }
                    }
                    Command::Gain(level) => {
                        jobs.send(Job::Gain(level)).ok();
                    }
                }
            }
            arrival = arrivals.recv() => {
                let Some(Fetched { epoch: at, id, segue, result }) = arrival else { break };
                if at != epoch {
                    continue;
                }
                let loaded = match result {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        log::warn!("playback: cannot load {id}: {error:#}");
                        if awaited == Some(at) {
                            awaited = None;
                            inflight = None;
                            announcing = None;
                            events.send(PlaybackEvent::Unavailable { id: Some(id) }).ok();
                        }
                        continue;
                    }
                };
                if awaited == Some(at) && current.as_deref() == Some(id.as_str()) {
                    awaited = None;
                    inflight = None;
                    let position = announcing
                        .as_ref()
                        .and_then(position_of)
                        .unwrap_or_default();
                    announce_length(&events, &id, fetch.length(&loaded));
                    jobs.send(Job::Play {
                        id,
                        loaded,
                        at: position,
                        playing: wanted,
                    }).ok();
                    queue_segue(&jobs, &mut waiting);
                    continue;
                }
                if segue {
                    segued = Some(id.clone());
                    let held = (id.clone(), loaded.clone());
                    match awaited {
                        // The track this follows has not started decoding yet, and its `Play`
                        // will clear the queue. Hold the join back until then.
                        Some(_) => waiting = Some(held),
                        None => {
                            let (id, loaded) = held;
                            jobs.send(Job::Queue { id, loaded }).ok();
                        }
                    }
                }
                ahead = Some((id, loaded));
            }
            heard = written.recv() => {
                if heard.is_none() {
                    break;
                }
                if let Some(event) = announcing.take() {
                    events.send(event).ok();
                }
            }
            join = joined.recv() => {
                match join {
                    Some(join) => settle(&mut current, join),
                    None => break,
                }
            }
            lost = gone.recv() => {
                if lost.is_some() {
                    events.send(PlaybackEvent::OutputChanged).ok();
                }
                return;
            }
        }
    }
}

/// The same announcement, but as a start. A play pressed while the track is still loading has
/// to change what its first audio will be reported as.
fn playing_now(event: PlaybackEvent) -> PlaybackEvent {
    match event {
        PlaybackEvent::Paused { id, at } => PlaybackEvent::Playing { id, at },
        held => held,
    }
}

fn paused_now(event: PlaybackEvent) -> PlaybackEvent {
    match event {
        PlaybackEvent::Playing { id, at } => PlaybackEvent::Paused { id, at },
        held => held,
    }
}

fn position_of(event: &PlaybackEvent) -> Option<Duration> {
    match event {
        PlaybackEvent::Playing { at, .. }
        | PlaybackEvent::Paused { at, .. }
        | PlaybackEvent::Seeked { at, .. } => Some(*at),
        _ => None,
    }
}

fn announce_length(events: &UnboundedSender<PlaybackEvent>, id: &str, duration: Option<Duration>) {
    if let Some(duration) = duration {
        events
            .send(PlaybackEvent::Length {
                id: Some(id.to_owned()),
                duration,
            })
            .ok();
    }
}

/// Hands the audio thread a join that was waiting for the track it follows to start.
fn queue_segue<F: Fetch>(
    jobs: &std::sync::mpsc::Sender<Job<F>>,
    waiting: &mut Option<(String, F::Loaded)>,
) {
    if let Some((id, loaded)) = waiting.take() {
        jobs.send(Job::Queue { id, loaded }).ok();
    }
}

fn spawn<F: Fetch>(
    fetch: &Arc<F>,
    id: String,
    epoch: u64,
    segue: bool,
    fetched: &UnboundedSender<Fetched<F>>,
) -> tokio::task::AbortHandle {
    let fetch = fetch.clone();
    let fetched = fetched.clone();
    tokio::spawn(async move {
        let result = fetch.load(&id).await;
        fetched
            .send(Fetched {
                epoch,
                id,
                segue,
                result,
            })
            .ok();
    })
    .abort_handle()
}

/// The track the audio thread is decoding, and where its samples land in real time.
struct Playing<F: Fetch> {
    id: String,
    /// Kept so a seek can open a second decoder over the same download.
    loaded: F::Loaded,
    source: F::Source,
    channels: u16,
    rate: u32,
    base: Duration,
    /// Samples queued before this track's first one. On a gapless join the one before it is
    /// still being heard, so its own position only starts once the count passes this.
    offset: u64,
}

impl<F: Fetch> Playing<F> {
    /// Opens a decoder for one track, or reports that it cannot be played.
    fn open(fetch: &F, id: &str, loaded: F::Loaded, at: Duration, offset: u64) -> Option<Self> {
        let source = fetch.open(id, &loaded, at)?;
        Some(Self {
            id: id.to_owned(),
            channels: source.channels().get(),
            rate: source.sample_rate().get(),
            loaded,
            source,
            base: at,
            offset,
        })
    }

    fn mark(&self) -> Mark {
        Mark {
            id: self.id.clone(),
            channels: self.channels,
            rate: self.rate,
            base: self.base,
            offset: self.offset,
        }
    }

    /// Pulls up to `frames` frames out of the decoder. `None` means the track ended.
    fn take(&mut self, frames: usize) -> Option<Vec<f32>> {
        let wanted = frames * usize::from(self.channels).max(1);
        let mut samples = Vec::with_capacity(wanted);
        for sample in self.source.by_ref().take(wanted) {
            samples.push(sample);
        }
        (!samples.is_empty()).then_some(samples)
    }
}

/// The track being heard, which is not always the one being decoded: at the end of a track the
/// decoder has moved on, or stopped, while the queue still holds the last seconds of it.
struct Mark {
    id: String,
    channels: u16,
    rate: u32,
    base: Duration,
    offset: u64,
}

impl Mark {
    /// Where the sound is: what the output has drawn from this track, from where it started.
    fn at(&self, cue: &Cue) -> Duration {
        let mine = cue.played().saturating_sub(self.offset);
        let frames = mine / u64::from(self.channels).max(1);
        self.base + Duration::from_secs_f64(frames as f64 / f64::from(self.rate).max(1.0))
    }
}

/// A track whose last sample is queued but not yet heard. The events for the join wait for it,
/// so one track's end and the next one's start land together, on the sample.
struct Join {
    ended: String,
    next: Option<(String, Option<Duration>)>,
    at: u64,
}

/// Decodes and writes until told otherwise. Every wait here is on the queue draining or on a
/// command, never on the network: the bytes are already arriving in the background.
#[allow(clippy::too_many_arguments)]
fn audio_loop<F: Fetch>(
    fetch: Arc<F>,
    jobs: Receiver<Job<F>>,
    cue: Cue,
    chain: Chain,
    changed: UnboundedSender<()>,
    joins: UnboundedSender<Joined>,
    events: UnboundedSender<PlaybackEvent>,
    interval: Duration,
) {
    let mut paced = match Paced::open(cue.clone(), chain, changed) {
        Ok(paced) => paced,
        Err(error) => return log::error!("playback: cannot open audio output: {error:#}"),
    };

    let mut current: Option<Playing<F>> = None;
    let mut written = 0u64;
    let mut joining: Option<Join> = None;
    let mut heard: Option<Mark> = None;
    let mut queued: Option<(String, F::Loaded)> = None;
    let mut playing = false;
    let mut reported_at = Instant::now();

    loop {
        // a full queue or nothing to decode means waiting for work rather than spinning
        let job = match current.is_some() && !paced.full() {
            true => match jobs.try_recv() {
                Ok(job) => Some(job),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            },
            false => match jobs.recv_timeout(Paced::poll()) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };

        if let Some(join) = &joining
            && cue.played() >= join.at
        {
            let Join { ended, next, .. } = joining.take().unwrap_or_else(|| unreachable!());
            events.send(PlaybackEvent::Ended { id: Some(ended) }).ok();
            heard = current.as_ref().map(Playing::mark);
            match next {
                Some((id, duration)) => {
                    announce_length(&events, &id, duration);
                    events
                        .send(PlaybackEvent::Playing {
                            id: Some(id),
                            at: Duration::ZERO,
                        })
                        .ok();
                }
                None => playing = false,
            }
        }

        // reported from the sound, so it keeps moving while the last seconds play out
        if playing
            && let Some(mark) = &heard
            && reported_at.elapsed() >= interval
            && !cue.cleared()
        {
            reported_at = Instant::now();
            events
                .send(PlaybackEvent::Position {
                    id: Some(mark.id.clone()),
                    at: mark.at(&cue),
                })
                .ok();
        }

        if let Some(job) = job {
            match job {
                Job::Play {
                    id,
                    loaded,
                    at,
                    playing: start,
                } => {
                    queued = None;
                    joining = None;
                    written = 0;
                    current = Playing::open(fetch.as_ref(), &id, loaded, at, 0);
                    heard = current.as_ref().map(Playing::mark);
                    playing = start && current.is_some();
                    match playing {
                        true if paced.play().is_err() => return,
                        true => {}
                        false => paced.pause(),
                    }
                    match current.is_some() {
                        // the next write is the new track, so it is the one worth reporting
                        true => cue.arm(),
                        false => {
                            events
                                .send(PlaybackEvent::Unavailable { id: Some(id) })
                                .ok();
                        }
                    }
                }
                Job::Queue { id, loaded } => queued = Some((id, loaded)),
                Job::Resume => {
                    playing = current.is_some();
                    if playing && paced.play().is_err() {
                        return;
                    }
                    if let Some(mark) = &heard {
                        events
                            .send(PlaybackEvent::Playing {
                                id: Some(mark.id.clone()),
                                at: mark.at(&cue),
                            })
                            .ok();
                    }
                }
                Job::Pause => {
                    playing = false;
                    paced.pause();
                    if let Some(mark) = &heard {
                        events
                            .send(PlaybackEvent::Paused {
                                id: Some(mark.id.clone()),
                                at: mark.at(&cue),
                            })
                            .ok();
                    }
                }
                Job::Seek(position) => {
                    let Some(held) = &mut current else { continue };
                    match fetch.reopen_to_seek() {
                        // A second decoder over the same download. Whatever is lined up behind
                        // this track is still what follows it; only the end it was waiting for
                        // has moved.
                        true => {
                            let (id, loaded) = (held.id.clone(), held.loaded.clone());
                            match Playing::open(fetch.as_ref(), &id, loaded, position, 0) {
                                Some(fresh) => current = Some(fresh),
                                None => {
                                    log::warn!("playback: cannot seek {id}");
                                    continue;
                                }
                            }
                        }
                        false => {
                            if let Err(error) = held.source.try_seek(position) {
                                log::warn!("playback: cannot seek {}: {error}", held.id);
                            }
                            held.base = position;
                            held.offset = 0;
                        }
                    }
                    joining = None;
                    written = 0;
                    heard = current.as_ref().map(Playing::mark);
                    cue.arm();
                }
                Job::Gain(level) => paced.set_volume(level),
            }
            continue;
        }

        let Some(held) = &mut current else { continue };
        if !playing || paced.full() {
            continue;
        }

        let Some(samples) = held.take(CHUNK) else {
            // the decoder is done, but its last samples are still queued
            let ended = current.take().map(|held| held.id).unwrap_or_default();
            let next = queued.take().and_then(|(id, loaded)| {
                let duration = fetch.length(&loaded);
                current = Playing::open(fetch.as_ref(), &id, loaded, Duration::ZERO, written);
                current.as_ref().map(|_| (id, duration))
            });
            joins
                .send(Joined {
                    ended: ended.clone(),
                    next: next.as_ref().map(|(id, _)| id.clone()),
                })
                .ok();
            // both events wait for the queue to reach here, so the join lands on the sample
            joining = Some(Join {
                ended,
                next,
                at: written,
            });
            continue;
        };

        let Some(chunk) = packet(&samples, held.channels, held.rate) else {
            continue;
        };
        if paced.write(chunk).is_err() {
            return;
        }
        written += samples.len() as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_moves_the_engines_current_track_on() {
        let mut current = Some("one".to_owned());
        settle(
            &mut current,
            Joined {
                ended: "one".to_owned(),
                next: Some("two".to_owned()),
            },
        );
        assert_eq!(current.as_deref(), Some("two"));
    }

    /// A load since the join has already moved on, so a late report must not undo it.
    #[test]
    fn a_join_for_a_track_that_is_no_longer_current_is_ignored() {
        let mut current = Some("three".to_owned());
        settle(
            &mut current,
            Joined {
                ended: "one".to_owned(),
                next: Some("two".to_owned()),
            },
        );
        assert_eq!(current.as_deref(), Some("three"));
    }

    #[test]
    fn a_track_that_ends_with_nothing_queued_leaves_no_current() {
        let mut current = Some("one".to_owned());
        settle(
            &mut current,
            Joined {
                ended: "one".to_owned(),
                next: None,
            },
        );
        assert_eq!(current, None);
    }

    /// Where the sound is, counted from what the output has played rather than from a clock.
    #[test]
    fn a_mark_reads_the_position_from_the_samples_heard() {
        let mark = Mark {
            id: "one".to_owned(),
            channels: 2,
            rate: 44_100,
            base: Duration::from_secs(10),
            offset: 0,
        };
        let (cue, _written) = Cue::new();
        cue.arm();
        assert_eq!(mark.at(&cue), Duration::from_secs(10));
    }
}
