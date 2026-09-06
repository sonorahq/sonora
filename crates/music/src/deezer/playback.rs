use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use crate::audio::{Output, RAMP, SmoothGain, Volume};
use crate::deezer::client::Session;
use crate::spectrum::Spectrum;
use crate::{PlaybackConfig, PlaybackEvent, PlaybackEvents, PlaybackFactory, Player};

const NORMAL_CAP: f32 = 1.0;
const POLL: Duration = Duration::from_millis(20);

enum Command {
    Load { id: String, at: Option<Duration> },
    Preload { id: String },
    Play,
    Pause,
    Seek(Duration),
    Gain(f32),
}

pub struct Factory {
    session: Arc<Session>,
}

impl Factory {
    pub fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        let (commands, command_rx) = unbounded_channel();
        let (events, event_rx) = unbounded_channel();
        let session = self.session.clone();
        let spectrum = Spectrum::new();
        let engine_spectrum = spectrum.clone();
        let spawned = std::thread::Builder::new()
            .name("dz-playback".to_string())
            .spawn(move || run(session, config, command_rx, events, engine_spectrum));
        if let Err(error) = spawned {
            log::error!("playback: cannot spawn engine thread: {error}");
        }
        (
            Box::new(Engine { commands, spectrum }),
            Box::new(Events(event_rx)),
        )
    }
}

struct Engine {
    commands: UnboundedSender<Command>,
    spectrum: Spectrum,
}

impl Player for Engine {
    fn load(&self, track_id: &str, _seamless: bool) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_string(),
                at: None,
            })
            .context("cannot reach playback engine")
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_string(),
                at: Some(at),
            })
            .context("cannot reach playback engine")
    }

    fn preload(&self, track_id: &str) -> Result<()> {
        self.commands
            .send(Command::Preload {
                id: track_id.to_string(),
            })
            .context("cannot reach playback engine")
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

pub struct Events(UnboundedReceiver<PlaybackEvent>);

#[async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        self.0.recv().await
    }
}

#[derive(Clone)]
struct Loaded {
    data: Arc<Vec<u8>>,
    loudness_db: Option<f32>,
    duration: Option<Duration>,
}

struct Slot {
    id: String,
    length: Option<Duration>,
    envelope: Volume,
    gain: f32,
}

impl Slot {
    fn mute(&self) {
        self.envelope.set(0.0);
    }

    fn unmute(&self) {
        self.envelope.set(self.gain);
    }
}

enum Kind {
    Play,
    Ahead,
}

struct Fetched {
    epoch: u64,
    id: String,
    kind: Kind,
    result: Result<Loaded>,
}

fn run(
    api: Arc<Session>,
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
            log::error!("playback: cannot build engine runtime: {error}");
            return;
        }
    };
    runtime.block_on(engine_loop(api, config, commands, events, spectrum));
}

async fn engine_loop(
    api: Arc<Session>,
    config: PlaybackConfig,
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<PlaybackEvent>,
    spectrum: Spectrum,
) {
    let output = match Output::open(Volume::new(config.gain), spectrum) {
        Ok(output) => output,
        Err(error) => {
            log::error!("playback: cannot open audio output: {error:#}");
            return;
        }
    };
    let sink = output.sink().clone();
    sink.pause();

    let (fetched, mut arrivals) = unbounded_channel::<Fetched>();
    let mut ticker = tokio::time::interval(POLL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let report_every = (config.position_interval.as_millis() / POLL.as_millis()).max(1) as u32;
    let mut ticks = 0u32;
    let mut output_ticks = 0u32;

    let mut playing = false;
    let mut autostart = true;
    let mut hold: Option<Duration> = None;
    let mut epoch = 0u64;
    let mut pending: Option<u64> = None;
    let mut inflight: Option<tokio::task::AbortHandle> = None;
    let mut current: Option<Slot> = None;
    let mut queued: Option<Slot> = None;
    let mut ahead: Option<(String, Loaded)> = None;
    let mut prev_len = 0usize;

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Load { id, at } => {
                        if at.is_none() && current.as_ref().is_some_and(|slot| slot.id == id) {
                            playing = true;
                            autostart = true;
                            if let Some(slot) = &current {
                                slot.unmute();
                                if let Some(length) = slot.length {
                                    events.send(PlaybackEvent::Length(length)).ok();
                                }
                            }
                            sink.play();
                            events.send(PlaybackEvent::Playing(sink.get_pos())).ok();
                            continue;
                        }
                        epoch += 1;
                        if let Some(handle) = inflight.take() {
                            handle.abort();
                        }
                        let cached = ahead
                            .take_if(|(cached, _)| *cached == id)
                            .map(|(_, loaded)| loaded);
                        pending = None;
                        if cached.is_none() {
                            ahead = None;
                            pending = Some(epoch);
                            inflight =
                                Some(spawn(&api, id.clone(), epoch, Kind::Play, &fetched));
                        }
                        events.send(PlaybackEvent::Loading(at.unwrap_or_default())).ok();
                        if output.failed() || output.changed() {
                            events.send(PlaybackEvent::OutputChanged).ok();
                            return;
                        }
                        silence(&sink, current.as_ref()).await;
                        current = None;
                        queued = None;
                        playing = false;
                        autostart = at.is_none();
                        hold = at;
                        prev_len = 0;
                        let Some(loaded) = cached else { continue };
                        match begin(&sink, &id, &loaded, &config, autostart, hold.take()) {
                            Ok(slot) => {
                                announce(&events, &slot, autostart, at.unwrap_or_default());
                                prev_len = sink.len();
                                current = Some(slot);
                                playing = autostart;
                            }
                            Err(error) => {
                                log::warn!("playback: cannot decode {id}: {error:#}");
                                events.send(PlaybackEvent::Unavailable).ok();
                            }
                        }
                    }
                    Command::Preload { id } => {
                        let known = current.as_ref().is_some_and(|slot| slot.id == id)
                            || queued.as_ref().is_some_and(|slot| slot.id == id)
                            || ahead.as_ref().is_some_and(|(cached, _)| *cached == id);
                        if known || current.is_none() {
                            continue;
                        }
                        spawn(&api, id, epoch, Kind::Ahead, &fetched);
                    }
                    Command::Play => {
                        autostart = true;
                        if output.failed() || output.changed() {
                            events.send(PlaybackEvent::OutputChanged).ok();
                            return;
                        }
                        if let Some(slot) = &current {
                            sink.play();
                            slot.unmute();
                            playing = true;
                            events.send(PlaybackEvent::Playing(sink.get_pos())).ok();
                        }
                    }
                    Command::Pause => {
                        autostart = false;
                        playing = false;
                        let position = sink.get_pos();
                        if let Some(slot) = &current {
                            slot.mute();
                            await_drain(&sink).await;
                            sink.pause();
                        }
                        events.send(PlaybackEvent::Paused(position)).ok();
                    }
                    Command::Seek(position) => match &current {
                        None if hold.is_some() => hold = Some(position),
                        None => {}
                        Some(slot) => {
                            slot.mute();
                            await_drain(&sink).await;
                            if let Err(error) = sink.try_seek(position) {
                                log::warn!("playback: cannot seek: {error}");
                            }
                            if playing {
                                slot.unmute();
                            }
                            events.send(PlaybackEvent::Position(sink.get_pos())).ok();
                        }
                    },
                    Command::Gain(level) => output.set_volume(level),
                }
            }
            arrival = arrivals.recv() => {
                let Some(Fetched { epoch: at, id, kind, result }) = arrival else { break };
                if at != epoch {
                    continue;
                }
                match kind {
                    Kind::Play => {
                        if pending != Some(at) {
                            continue;
                        }
                        pending = None;
                        inflight = None;
                        let at = hold.take();
                        match result
                            .and_then(|loaded| begin(&sink, &id, &loaded, &config, autostart, at))
                        {
                            Ok(slot) => {
                                announce(&events, &slot, autostart, at.unwrap_or_default());
                                prev_len = sink.len();
                                current = Some(slot);
                                playing = autostart;
                            }
                            Err(error) => {
                                log::warn!("playback: cannot load {id}: {error:#}");
                                events.send(refusal(&error)).ok();
                            }
                        }
                    }
                    Kind::Ahead => {
                        let Ok(loaded) = result else {
                            continue;
                        };
                        if current.is_some() && queued.is_none() {
                            match append(&sink, &id, &loaded, &config, false) {
                                Ok(slot) => {
                                    log::debug!("playback: {id} is queued for a gapless segue");
                                    queued = Some(slot);
                                    prev_len = sink.len();
                                }
                                Err(error) => {
                                    log::warn!("playback: cannot decode preload {id}: {error:#}")
                                }
                            }
                        }
                        ahead = Some((id, loaded));
                    }
                }
            }
            _ = ticker.tick() => {
                output_ticks += 1;
                if playing && (output.failed() || output_ticks >= report_every && output.changed()) {
                    events.send(PlaybackEvent::OutputChanged).ok();
                    return;
                }
                if output_ticks >= report_every {
                    output_ticks = 0;
                }
                let len = sink.len();
                ticks += 1;
                if current.is_some() && playing && len < prev_len {
                    ticks = 0;
                    events.send(PlaybackEvent::Ended).ok();
                    current = queued.take();
                    ahead = None;
                    playing = current.is_some();
                    match &current {
                        Some(slot) => {
                            if let Some(length) = slot.length {
                                events.send(PlaybackEvent::Length(length)).ok();
                            }
                            events.send(PlaybackEvent::Position(sink.get_pos())).ok();
                        }
                        None => log::debug!("playback: track ended with nothing queued ahead"),
                    }
                } else if playing && ticks >= report_every {
                    ticks = 0;
                    events.send(PlaybackEvent::Position(sink.get_pos())).ok();
                }
                prev_len = len;
            }
        }
    }
}

fn spawn(
    api: &Arc<Session>,
    id: String,
    epoch: u64,
    kind: Kind,
    fetched: &UnboundedSender<Fetched>,
) -> tokio::task::AbortHandle {
    let api = api.clone();
    let fetched = fetched.clone();
    tokio::spawn(async move {
        let result = fetch(&api, &id).await;
        fetched
            .send(Fetched {
                epoch,
                id,
                kind,
                result,
            })
            .ok();
    })
    .abort_handle()
}

async fn silence(sink: &rodio::Player, slot: Option<&Slot>) {
    let Some(slot) = slot else {
        sink.clear();
        return;
    };
    slot.mute();
    await_drain(sink).await;
    sink.clear();
}

async fn await_drain(sink: &rodio::Player) {
    if sink.is_paused() {
        return;
    }
    tokio::time::sleep(RAMP).await;
}

fn begin(
    sink: &rodio::Player,
    id: &str,
    loaded: &Loaded,
    config: &PlaybackConfig,
    start: bool,
    at: Option<Duration>,
) -> Result<Slot> {
    sink.clear();
    let slot = append(sink, id, loaded, config, true)?;
    if let Some(at) = at
        && let Err(error) = sink.try_seek(at)
    {
        log::warn!("playback: cannot start {id} at {}s: {error}", at.as_secs());
    }
    match start {
        true => sink.play(),
        false => sink.pause(),
    }
    Ok(slot)
}

fn append(
    sink: &rodio::Player,
    id: &str,
    loaded: &Loaded,
    config: &PlaybackConfig,
    fade: bool,
) -> Result<Slot> {
    let gain = normalisation(config.normalisation, loaded.loudness_db);
    let envelope = Volume::new(gain);
    let initial = match fade {
        true => 0.0,
        false => gain,
    };
    let source = decode(loaded.data.clone())?;
    sink.append(SmoothGain::new(source, envelope.clone(), initial, RAMP));
    Ok(Slot {
        id: id.to_string(),
        length: loaded.duration,
        envelope,
        gain,
    })
}

fn announce(
    events: &UnboundedSender<PlaybackEvent>,
    slot: &Slot,
    playing: bool,
    position: Duration,
) {
    if let Some(length) = slot.length {
        events.send(PlaybackEvent::Length(length)).ok();
    }
    let event = match playing {
        true => PlaybackEvent::Playing(position),
        false => PlaybackEvent::Paused(position),
    };
    events.send(event).ok();
}

fn refusal(_error: &anyhow::Error) -> PlaybackEvent {
    PlaybackEvent::Unavailable
}

async fn fetch(api: &Session, id: &str) -> Result<Loaded> {
    let data = api.audio(id).await?;
    Ok(Loaded {
        data: Arc::new(data),
        loudness_db: None,
        duration: None,
    })
}

fn decode(data: Arc<Vec<u8>>) -> Result<impl rodio::Source + Send + 'static> {
    let length = data.len() as u64;
    let cursor = Cursor::new(Bytes(data.clone()));
    rodio::Decoder::builder()
        .with_data(cursor)
        .with_byte_len(length)
        .with_seekable(true)
        .build()
        .or_else(|_| {
            rodio::Decoder::builder()
                .with_data(Cursor::new(Bytes(data)))
                .with_byte_len(length)
                .build()
        })
        .context("cannot decode audio")
}

struct Bytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

fn normalisation(enabled: bool, loudness_db: Option<f32>) -> f32 {
    match (enabled, loudness_db) {
        (true, Some(db)) => 10f32.powf(-db / 20.0).min(NORMAL_CAP),
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_attenuates_loud_tracks() {
        let factor = normalisation(true, Some(6.0));
        assert!(factor < 0.51 && factor > 0.49);
    }

    #[test]
    fn normalisation_never_boosts() {
        assert_eq!(normalisation(true, Some(-3.0)), NORMAL_CAP);
        assert_eq!(normalisation(false, Some(6.0)), 1.0);
        assert_eq!(normalisation(true, None), 1.0);
    }
}
