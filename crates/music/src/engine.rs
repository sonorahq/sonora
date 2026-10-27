//! One playback engine, for every provider whose tracks are a file to decode.
//!
//! Two threads share the work the way the Spotify path does: a tokio thread fetches, and one
//! audio thread decodes and feeds [`crate::sink::Paced`]. Neither the audio callback nor the
//! runtime ever waits on the network, so a track starts as soon as its first seconds are in.
//! The engine owns everything that is the same whoever the provider is: the queue of commands,
//! the preload, the gapless join, where a position is reported from, and what a lost output
//! device does.
//!
//! A provider supplies [`Fetch`]: how to get a track and how to open a decoder over it. Every
//! provider but Spotify does, whose decoding librespot owns. Local reads a file from disk,
//! YouTube asks for the file a range at a time, Subsonic hands over a plain response, Deezer
//! decrypts Blowfish stripes as they arrive, Tidal fetches DASH segments one after another, and
//! Apple Music indexes CENC fragments and decrypts each sample through a CDM as the decoder
//! reaches it.
//!
//! Loudness normalisation lives here too. A provider only reports how loud a track is, and the
//! engine decides what gain that earns.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, ensure};
use async_trait::async_trait;
use futures::future::BoxFuture;
use rodio::Source as _;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::audio::{Chain, Volume};
use crate::sink::{Cue, Paced, packet, watch_for_output};
use crate::spectrum::Spectrum;
use crate::vault::Shelf;
use crate::{
    Encoding, Fraction, Normalisation, PlaybackConfig, PlaybackEvent, PlaybackEvents, Player,
    TrackSignal,
};

/// How often a download reports how far it has got.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// How many frames the decoder hands over at a time. Small enough that a skip is heard at once,
/// large enough that the queue is not rebuilt for every few samples.
const CHUNK: usize = 4096;
/// The level normalisation brings every track to, in LUFS. Spotify and YouTube both play at
/// about this level, so a track sounds as loud here whichever service it came from.
const TARGET_LUFS: f32 = -14.0;
/// The level ReplayGain 2.0 measures its gain against, in LUFS.
const REPLAYGAIN_LUFS: f32 = -18.0;
/// The most normalisation may raise a track, about 12 dB, whatever headroom its peak claims.
const BOOST_CAP: f32 = 4.0;

/// How loud a track is, as its source measured it. The engine compares this with
/// `TARGET_LUFS` to decide the track's gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    /// Integrated loudness in LUFS.
    pub lufs: f32,
    /// The peak as a fraction of full scale, sample or true peak. Without one a quiet track is
    /// never raised, since nothing says how far it can go before it clips.
    pub peak: Option<f32>,
}

impl Loudness {
    /// Reads a ReplayGain gain in dB and its peak, the form that file tags and Subsonic servers
    /// carry.
    pub fn replay_gain(gain_db: f32, peak: Option<f32>) -> Self {
        Self {
            lufs: REPLAYGAIN_LUFS - gain_db,
            peak: peak.filter(|peak| *peak > 0.0),
        }
    }
}

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

    /// Where this provider keeps tracks, when it keeps any. A provider with a shelf can download
    /// a track without playing it.
    fn shelf(&self) -> Option<&Shelf> {
        None
    }

    /// Whether a track on the shelf plays with no network. A provider that has to license every
    /// play says no, and its downloads only save the fetch.
    fn plays_offline(&self) -> bool {
        true
    }

    /// Fetches a track: enough of it to start decoding, not all of it.
    async fn load(&self, id: &str) -> Result<Self::Loaded>;

    /// How long the track is, when the fetch learned it. The engine announces it so a seek bar
    /// has something to draw before the decoder knows.
    fn length(&self, _loaded: &Self::Loaded) -> Option<Duration> {
        None
    }

    /// How loud the track is, when its source measured it. The engine turns this into the
    /// track's gain when normalisation is on.
    fn loudness(&self, _loaded: &Self::Loaded) -> Option<Loudness> {
        None
    }

    /// How the track is encoded, when the fetch learned it. The signal path shows it.
    fn encoding(&self, _loaded: &Self::Loaded) -> Option<Encoding> {
        None
    }

    /// Whether a failed load means the listener has to sign in. The engine reports that as
    /// gated rather than as the track being unavailable.
    fn gated(&self, _error: &anyhow::Error) -> bool {
        false
    }

    /// Resolves once the whole track has arrived, or its download has ended some other way.
    /// The engine reports it so the next track can be fetched without taking bandwidth from
    /// this one. The default answers at once, which suits a track that is already on disk.
    async fn downloaded(&self, _loaded: &Self::Loaded) {}

    /// How much of the track has arrived, from 0 to 1, when the fetch can tell. A download shows
    /// it as it goes.
    fn progress(&self, _loaded: &Self::Loaded) -> Option<f32> {
        None
    }

    /// Opens a decoder placed at `at`. `None` means the track cannot be played, which the
    /// engine reports as unavailable.
    fn open(&self, id: &str, loaded: &Self::Loaded, at: Duration) -> Option<Self::Source>;

    /// Opens a decoder as near `at` as the download allows and says where it really starts,
    /// which is where the engine reports the position from. The default is [`open`](Self::open)
    /// exactly at `at`. A provider that may have to start earlier than asked overrides this.
    fn open_near(
        &self,
        id: &str,
        loaded: &Self::Loaded,
        at: Duration,
    ) -> Option<(Self::Source, Duration)> {
        self.open(id, loaded, at).map(|source| (source, at))
    }

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
    let shelf = fetch.shelf().cloned();
    let offline = fetch.plays_offline();
    let spawned = std::thread::Builder::new()
        .name(format!("{name}-playback"))
        .spawn(move || run(Arc::new(fetch), config, command_rx, events, engine_spectrum));
    if let Err(error) = spawned {
        log::error!("playback: cannot spawn the {name} engine thread: {error}");
    }
    (
        Box::new(Handle {
            commands,
            spectrum,
            shelf,
            offline,
        }),
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
    /// Fetch a whole track into the downloads without playing it, and say how that went.
    Download {
        id: String,
        progress: Fraction,
        done: oneshot::Sender<Result<()>>,
    },
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
    /// Drop whatever is decoding and hold the output silent. A load sends this before it waits
    /// on the fetch, so the track being replaced cannot be heard again whatever happens next.
    Stop,
    Resume,
    Pause,
    /// Move within the track being decoded. The bytes are already arriving, so this is local.
    Seek(Duration),
    Gain(f32),
}

struct Handle {
    commands: UnboundedSender<Command>,
    spectrum: Spectrum,
    /// Where the provider keeps tracks, which says what is downloaded without asking the
    /// engine thread.
    shelf: Option<Shelf>,
    /// Whether what is on the shelf plays with no network, as [`Fetch::plays_offline`] said.
    offline: bool,
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

    fn download(&self, track_id: &str, progress: Fraction) -> BoxFuture<'static, Result<()>> {
        let (done, answer) = oneshot::channel();
        let sent = match self.shelf {
            Some(_) => self
                .commands
                .send(Command::Download {
                    id: track_id.to_owned(),
                    progress,
                    done,
                })
                .context("cannot reach the playback engine"),
            None => Err(anyhow!("this provider cannot download")),
        };
        Box::pin(async move {
            sent?;
            answer.await.context("the playback engine stopped")?
        })
    }

    fn downloaded(&self, track_id: &str) -> bool {
        self.shelf
            .as_ref()
            .is_some_and(|shelf| shelf.holds(track_id))
    }

    fn plays_offline(&self, track_id: &str) -> bool {
        self.offline
            && self
                .shelf
                .as_ref()
                .is_some_and(|shelf| shelf.may_hold(track_id))
    }

    fn discard(&self, track_id: &str) {
        if let Some(shelf) = &self.shelf {
            shelf.discard(track_id);
        }
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
/// went on to decode from the queue. It is sent once the last of the ended track has been
/// heard, since until then a seek still moves within that track. The engine follows this rather
/// than the queue it handed over, because a queued track can arrive after the end it was meant
/// for, or fail to open.
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
    let chain = Chain {
        volume: Volume::new(config.gain),
        effects: config.effects.clone(),
        equalizer: config.equalizer.clone(),
        spectrum,
        meter: config.meter.clone(),
        analyzer: config.analyzer.clone(),
    };
    let (jobs, job_rx) = channel::<Job<F>>();
    let (joins, mut joined) = unbounded_channel::<Joined>();

    let audio_cue = cue.clone();
    let audio_events = events.clone();
    let audio_fetch = fetch.clone();
    let interval = config.position_interval;
    let normalisation = config.normalisation;
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
                normalisation,
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
    // where the track being fetched will open. A seek while it loads moves this rather than the
    // decoder, since the decoder for it does not exist yet.
    let mut hold = Duration::ZERO;
    let (fetched, mut arrivals) = unbounded_channel::<Fetched<F>>();
    // the wait on the current track's download, so the state hears when all of it is in
    let mut watching: Option<tokio::task::AbortHandle> = None;
    let (downloads, mut downloaded) = unbounded_channel::<String>();

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
                                watch_ahead(&fetch, &current, &ahead, &downloads, &mut watching);
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
                        // The audio thread is still decoding the track this replaces. Clearing
                        // the queue only silences what is in it; without this the old decoder
                        // keeps running, and anything that arms the cue before the fetch lands
                        // puts it back on the output.
                        jobs.send(Job::Stop).ok();
                        if let Some(handle) = watching.take() {
                            handle.abort();
                        }
                        current = Some(id.clone());
                        let position = at.unwrap_or_default();
                        hold = position;
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
                                watching = Some(watch(&fetch, &id, &loaded, &downloads));
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
                        if current.is_none() || current.as_deref() == Some(id.as_str()) {
                            continue;
                        }
                        let held = ahead
                            .as_ref()
                            .filter(|(cached, _)| *cached == id)
                            .map(|(_, loaded)| loaded.clone());
                        match held {
                            // Fetched already, as the next track is once the current one is
                            // all in. A segue only has to line it up.
                            Some(loaded) => {
                                if !segue || segued.as_deref() == Some(id.as_str()) {
                                    continue;
                                }
                                segued = Some(id.clone());
                                match awaited {
                                    Some(_) => waiting = Some((id, loaded)),
                                    None => {
                                        jobs.send(Job::Queue { id, loaded }).ok();
                                    }
                                }
                            }
                            None => {
                                spawn(&fetch, id, epoch, segue, &fetched);
                            }
                        }
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
                        let Some(id) = current.clone() else { continue };
                        // Nothing is decoding this track while its fetch is out, so there is
                        // nothing to move: what a seek changes is where it will open. Sending
                        // the audio thread a seek here would land on the track this load
                        // replaced, which is the one that still holds a decoder.
                        if awaited.is_some() {
                            hold = position;
                            announcing = announcing.map(|event| moved(event, position));
                            continue;
                        }
                        cue.clear();
                        announcing = Some(PlaybackEvent::Seeked {
                            id: Some(id),
                            at: position,
                        });
                        jobs.send(Job::Seek(position)).ok();
                    }
                    Command::Gain(level) => {
                        jobs.send(Job::Gain(level)).ok();
                    }
                    Command::Download { id, progress, done } => {
                        let fetch = fetch.clone();
                        tokio::spawn(async move {
                            done.send(download(&*fetch, &id, &progress).await).ok();
                        });
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
                            let refusal = match fetch.gated(&error) {
                                true => PlaybackEvent::Gated,
                                false => PlaybackEvent::Unavailable { id: Some(id) },
                            };
                            events.send(refusal).ok();
                        }
                        continue;
                    }
                };
                if awaited == Some(at) && current.as_deref() == Some(id.as_str()) {
                    awaited = None;
                    inflight = None;
                    announce_length(&events, &id, fetch.length(&loaded));
                    watching = Some(watch(&fetch, &id, &loaded, &downloads));
                    jobs.send(Job::Play {
                        id,
                        loaded,
                        at: hold,
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
                // The audio thread has ended. One that lost its output tells `gone` before it
                // returns, but `select!` picks among ready branches at random and can see the
                // closed `joined` first. Without reading `gone` here the engine would end
                // without telling the state to rebuild it, and playback would stay silent.
                let Some(join) = join else {
                    if gone.try_recv().is_ok() {
                        events.send(PlaybackEvent::OutputChanged).ok();
                        return;
                    }
                    break;
                };
                let before = current.clone();
                settle(&mut current, join);
                if current != before {
                    watch_ahead(&fetch, &current, &ahead, &downloads, &mut watching);
                }
            }
            done = downloaded.recv() => {
                let Some(id) = done else { break };
                if current.as_deref() == Some(id.as_str()) {
                    events.send(PlaybackEvent::Downloaded { id: Some(id) }).ok();
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

/// The same announcement, at another position. A seek before the track has started decoding
/// moves where it will open, and must leave a paused start paused.
fn moved(event: PlaybackEvent, to: Duration) -> PlaybackEvent {
    match event {
        PlaybackEvent::Playing { id, .. } => PlaybackEvent::Playing { id, at: to },
        PlaybackEvent::Paused { id, .. } => PlaybackEvent::Paused { id, at: to },
        PlaybackEvent::Seeked { id, .. } => PlaybackEvent::Seeked { id, at: to },
        held => held,
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

/// Waits on the download of `id` and reports it on `downloads` once all of it is in. The handle
/// is aborted when the track stops being current, so a skipped track is not held open.
fn watch<F: Fetch>(
    fetch: &Arc<F>,
    id: &str,
    loaded: &F::Loaded,
    downloads: &UnboundedSender<String>,
) -> tokio::task::AbortHandle {
    let (fetch, id, loaded, downloads) = (
        fetch.clone(),
        id.to_owned(),
        loaded.clone(),
        downloads.clone(),
    );
    tokio::spawn(async move {
        fetch.downloaded(&loaded).await;
        downloads.send(id).ok();
    })
    .abort_handle()
}

/// Moves the download watch onto `current` after a gapless join, when its fetch is still held
/// in `ahead`. A track that is not there goes unwatched, and the state falls back to
/// preloading near the end.
fn watch_ahead<F: Fetch>(
    fetch: &Arc<F>,
    current: &Option<String>,
    ahead: &Option<(String, F::Loaded)>,
    downloads: &UnboundedSender<String>,
    watching: &mut Option<tokio::task::AbortHandle>,
) {
    if let Some(handle) = watching.take() {
        handle.abort();
    }
    let Some(id) = current.as_deref() else {
        return;
    };
    if let Some((_, loaded)) = ahead.as_ref().filter(|(cached, _)| cached == id) {
        *watching = Some(watch(fetch, id, loaded, downloads));
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
    /// The track's id, as the provider knows it and the engine's events report it.
    id: String,

    /// Kept so a seek can open a second decoder over the same download.
    loaded: F::Loaded,

    /// The decoder. `Paced` resamples what it decodes to the device's rate.
    source: F::Source,

    /// The number of channels the source has.
    channels: u16,

    /// The source's rate, which its packets are written at.
    rate: u32,

    /// Where in the track decoding started: the open, or the last seek.
    base: Duration,

    /// Samples queued before this track's first one. On a gapless join the one before it is
    /// still being heard, so its own position only starts once the count passes this. The end
    /// of that one may still be in the resampler then, so this counts what it will add too.
    offset: u64,

    /// The track's normalisation gain, applied to every sample as it is decoded. Each track
    /// carries its own, so a gapless join changes it on the first sample of the next one.
    gain: f32,
}

impl<F: Fetch> Playing<F> {
    /// Opens a decoder for one track and reports what it decodes to as a `Signal` event, or
    /// returns `None` when it cannot be played.
    fn open(
        fetch: &F,
        id: &str,
        loaded: F::Loaded,
        at: Duration,
        offset: u64,
        normalise: bool,
        events: &UnboundedSender<PlaybackEvent>,
    ) -> Option<Self> {
        let (source, base) = fetch.open_near(id, &loaded, at)?;
        let gain = normalisation(normalise, fetch.loudness(&loaded));
        let (channels, rate) = (source.channels().get(), source.sample_rate().get());
        let track = TrackSignal {
            encoding: fetch.encoding(&loaded),
            rate,
            channels,
            normalisation: match normalise {
                true => Normalisation::Gain(20. * gain.log10()),
                false => Normalisation::Off,
            },
        };
        events
            .send(PlaybackEvent::Signal {
                id: id.to_owned(),
                track,
            })
            .ok();
        Some(Self {
            id: id.to_owned(),
            channels,
            rate,
            loaded,
            source,
            base,
            offset,
            gain,
        })
    }

    /// Where this track's sound is read from, given `rate`, the device's, which is what the queue
    /// counts samples at.
    fn mark(&self, rate: u32) -> Mark {
        Mark {
            id: self.id.clone(),
            channels: self.channels,
            rate,
            base: self.base,
            offset: self.offset,
        }
    }

    /// How many samples `frames` frames of this track hold, across every channel.
    fn samples(&self, frames: usize) -> usize {
        frames * usize::from(self.channels).max(1)
    }

    /// Pulls up to `frames` frames out of the decoder into `samples`, which starts empty.
    /// `None` means the track ended.
    fn take(&mut self, frames: usize, mut samples: Vec<f32>) -> Option<Vec<f32>> {
        let wanted = self.samples(frames);
        for sample in self.source.by_ref().take(wanted) {
            samples.push(sample * self.gain);
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
/// so one track's end and the next one's start land together, on the sample. The ended track's
/// download is kept so a seek before then can reopen it.
struct Join<F: Fetch> {
    ended: String,
    loaded: F::Loaded,
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
    normalise: bool,
) {
    let mut paced = match Paced::open(cue.clone(), chain, changed.clone()) {
        Ok(paced) => paced,
        Err(error) => {
            // Nothing can be decoded without an output, so the thread only holds its channels
            // until the engine is dropped. Returning would close `joins` and end the engine, and
            // the watch below ends with the engine before its first try, so playback would stay
            // silent. A device that would not open once usually opens on the watch's next try.
            log::error!("playback: cannot open audio output: {error:#}");
            watch_for_output(changed);
            while jobs.recv().is_ok() {}
            return;
        }
    };

    let mut current: Option<Playing<F>> = None;
    let mut written = 0u64;
    let mut joining: Option<Join<F>> = None;
    let mut heard: Option<Mark> = None;
    let mut queued: Option<(String, F::Loaded)> = None;
    let mut playing = false;
    let mut reported_at = Instant::now();

    loop {
        // anything but decoding means waiting for work rather than spinning: no
        // track, a paused one, or a full queue. A restored track sits paused
        // with an empty queue, which the old condition mistook for decoding.
        let idle = current.is_none() || !playing || paced.full();
        let job = match idle {
            false => match jobs.try_recv() {
                Ok(job) => Some(job),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            },
            true => match jobs.recv_timeout(Paced::poll()) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };

        if let Some(join) = &joining
            && cue.played() >= join.at
        {
            let Join { ended, next, .. } = joining.take().unwrap_or_else(|| unreachable!());
            joins
                .send(Joined {
                    ended: ended.clone(),
                    next: next.as_ref().map(|(id, _)| id.clone()),
                })
                .ok();
            events.send(PlaybackEvent::Ended { id: Some(ended) }).ok();
            heard = current.as_ref().map(|held| held.mark(paced.rate()));
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
                    current = Playing::open(fetch.as_ref(), &id, loaded, at, 0, normalise, &events);
                    heard = current.as_ref().map(|held| held.mark(paced.rate()));
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
                Job::Stop => {
                    current = None;
                    queued = None;
                    joining = None;
                    heard = None;
                    written = 0;
                    playing = false;
                    paced.pause();
                }
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
                Job::Seek(position) if joining.is_some() => {
                    // The decoder has run past the end of the track still being heard, onto the
                    // one queued behind it or onto nothing. The seek is in the heard track, so
                    // that track reopens and the next one goes back in the queue.
                    let Some(join) = joining.take() else { continue };
                    if let Some(next) = current.take() {
                        queued = Some((next.id, next.loaded));
                    }
                    current = Playing::open(
                        fetch.as_ref(),
                        &join.ended,
                        join.loaded,
                        position,
                        0,
                        normalise,
                        &events,
                    );
                    if current.is_none() {
                        log::warn!("playback: cannot seek {}", join.ended);
                        events
                            .send(PlaybackEvent::Unavailable {
                                id: Some(join.ended),
                            })
                            .ok();
                        continue;
                    }
                    written = 0;
                    heard = current.as_ref().map(|held| held.mark(paced.rate()));
                    cue.arm();
                }
                Job::Seek(position) => {
                    let Some(held) = &mut current else { continue };
                    match fetch.reopen_to_seek() {
                        // A second decoder over the same download. Whatever is lined up behind
                        // this track is still what follows it; only the end it was waiting for
                        // has moved.
                        true => {
                            let (id, loaded) = (held.id.clone(), held.loaded.clone());
                            match Playing::open(
                                fetch.as_ref(),
                                &id,
                                loaded,
                                position,
                                0,
                                normalise,
                                &events,
                            ) {
                                Some(fresh) => current = Some(fresh),
                                // The seek has already cleared the queue, so the old decoder
                                // would go on unheard. The app moves on as from a failed load.
                                None => {
                                    log::warn!("playback: cannot seek {id}");
                                    events
                                        .send(PlaybackEvent::Unavailable { id: Some(id) })
                                        .ok();
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
                    heard = current.as_ref().map(|held| held.mark(paced.rate()));
                    cue.arm();
                }
                Job::Gain(level) => paced.set_volume(level),
            }
            continue;
        }

        let Some(held) = &mut current else { continue };
        if idle {
            continue;
        }

        let buffer = paced.buffer(held.samples(CHUNK));
        let Some(samples) = held.take(CHUNK, buffer) else {
            // the decoder is done, but its last samples are still queued
            let Some(Playing {
                id: ended, loaded, ..
            }) = current.take()
            else {
                continue;
            };
            let next = queued.take().and_then(|(id, loaded)| {
                let duration = fetch.length(&loaded);
                current = Playing::open(
                    fetch.as_ref(),
                    &id,
                    loaded,
                    Duration::ZERO,
                    0,
                    normalise,
                    &events,
                );
                current.as_ref().map(|_| (id, duration))
            });
            // A track that follows is written straight in behind this one, so the resampler
            // carries on across the join, as one signal at the same format or a new span at
            // another, and says where it lands. With nothing to follow, what it holds is let out
            // so the end is heard.
            let at = match &mut current {
                Some(following) => {
                    following.offset = written + paced.held_back() as u64;
                    following.offset
                }
                None => {
                    written += paced.flush() as u64;
                    written
                }
            };
            // both events wait for the queue to reach here, so the join lands on the sample
            joining = Some(Join {
                ended,
                loaded,
                next,
                at,
            });
            continue;
        };

        let Some(chunk) = packet(samples, held.channels, held.rate) else {
            continue;
        };
        match paced.write(chunk) {
            Ok(queued) => written += queued as u64,
            Err(_) => return,
        }
    }
}

/// The gain a track plays at: unity with normalisation off or no loudness known, and otherwise
/// whatever brings it to `TARGET_LUFS`. A loud track is always turned down. A quiet one is
/// raised only as far as its peak leaves room for, so normalising never clips a track that
/// did not clip already.
fn normalisation(enabled: bool, loudness: Option<Loudness>) -> f32 {
    let Some(loudness) = loudness.filter(|_| enabled) else {
        return 1.0;
    };
    let wanted = 10f32.powf((TARGET_LUFS - loudness.lufs) / 20.0);
    let headroom = loudness
        .peak
        .map_or(1.0, |peak| (1.0 / peak).clamp(1.0, BOOST_CAP));
    wanted.min(headroom)
}

/// Fetches all of track `id` into the downloads, the way a play would fetch it, and holds on to
/// it until the last byte is in. A track already downloaded costs nothing.
async fn download<F: Fetch>(fetch: &F, id: &str, progress: &Fraction) -> Result<()> {
    let shelf = fetch.shelf().context("this provider keeps nothing")?;
    shelf.settle().await;
    if shelf.holds(id) {
        return Ok(());
    }
    shelf.want(id);
    let fetched = async {
        let loaded = fetch.load(id).await?;
        let arrived = fetch.downloaded(&loaded);
        tokio::pin!(arrived);
        let mut ticks = tokio::time::interval(PROGRESS_EVERY);
        loop {
            tokio::select! {
                () = &mut arrived => break,
                _ = ticks.tick() => {
                    if let Some(fraction) = fetch.progress(&loaded) {
                        progress.set(fraction);
                    }
                }
            }
        }
        progress.set(1.0);
        anyhow::Ok(())
    }
    .await;
    shelf.unwant(id);
    fetched?;
    ensure!(shelf.holds(id), "the download of {id} did not finish");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(lufs: f32, peak: Option<f32>) -> Option<Loudness> {
        Some(Loudness { lufs, peak })
    }

    #[test]
    fn normalisation_attenuates_loud_tracks() {
        let factor = normalisation(true, measured(TARGET_LUFS + 6.0, None));
        assert!(factor < 0.51 && factor > 0.49);
    }

    #[test]
    fn normalisation_boosts_only_into_headroom() {
        assert_eq!(normalisation(true, measured(TARGET_LUFS - 3.0, None)), 1.0);
        assert_eq!(
            normalisation(true, measured(TARGET_LUFS - 12.0, Some(0.5))),
            2.0
        );
        assert_eq!(normalisation(false, measured(TARGET_LUFS + 6.0, None)), 1.0);
        assert_eq!(normalisation(true, None), 1.0);
    }

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
