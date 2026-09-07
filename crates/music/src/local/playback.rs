use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use rodio::Source as _;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::wire;
use crate::audio::{Output, Volume};
use crate::spectrum::Spectrum;
use crate::{PlaybackConfig, PlaybackEvent, PlaybackEvents, PlaybackFactory, Player};

const POLL: Duration = Duration::from_millis(20);

enum Command {
    Load {
        id: String,
        at: Option<Duration>,
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

#[derive(Default)]
pub struct Factory;

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        let (commands, command_rx) = unbounded_channel();
        let (events, event_rx) = unbounded_channel();
        let spectrum = Spectrum::new();
        let engine_spectrum = spectrum.clone();
        let spawned = std::thread::Builder::new()
            .name("local-playback".to_owned())
            .spawn(move || run(config, command_rx, events, engine_spectrum));
        if let Err(error) = spawned {
            log::error!("playback: cannot spawn local engine thread: {error}");
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
    fn load(&self, track_id: &str, seamless: bool) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_owned(),
                at: None,
                seamless,
            })
            .context("cannot reach local playback engine")
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_owned(),
                at: Some(at),
                seamless: false,
            })
            .context("cannot reach local playback engine")
    }

    fn preload(&self, track_id: &str, segue: bool) -> Result<()> {
        self.commands
            .send(Command::Preload {
                id: track_id.to_owned(),
                segue,
            })
            .context("cannot reach local playback engine")
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

struct Slot {
    id: String,
    length: Option<Duration>,
}

fn run(
    config: PlaybackConfig,
    commands: UnboundedReceiver<Command>,
    events: UnboundedSender<PlaybackEvent>,
    spectrum: Spectrum,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("playback: cannot build local engine runtime: {error}");
            return;
        }
    };
    runtime.block_on(engine_loop(config, commands, events, spectrum));
}

async fn engine_loop(
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

    let mut ticker = tokio::time::interval(POLL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let report_every = (config.position_interval.as_millis() / POLL.as_millis()).max(1) as u32;
    let mut ticks = 0u32;
    let mut output_ticks = 0u32;

    let mut playing = false;
    let mut current: Option<Slot> = None;
    let mut queued: Option<Slot> = None;
    let mut prev_len = 0usize;

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Load { id, at, seamless } => {
                        let segued = seamless
                            && at.is_none()
                            && current.as_ref().is_some_and(|slot| slot.id == id);
                        if segued {
                            playing = true;
                            sink.play();
                            if let Some(length) = current.as_ref().and_then(|slot| slot.length) {
                                events.send(PlaybackEvent::Length {
                                    id: Some(id.clone()),
                                    duration: length,
                                }).ok();
                            }
                            events.send(PlaybackEvent::Playing {
                                id: Some(id),
                                at: sink.get_pos(),
                            }).ok();
                            continue;
                        }
                        events.send(PlaybackEvent::Loading {
                            id: Some(id.clone()),
                            at: at.unwrap_or_default(),
                        }).ok();
                        sink.clear();
                        current = None;
                        queued = None;
                        prev_len = 0;
                        match load(&sink, &id) {
                            Ok(slot) => {
                                place(&sink, &id, at);
                                match at {
                                    Some(_) => sink.pause(),
                                    None => sink.play(),
                                }
                                if let Some(length) = slot.length {
                                    events.send(PlaybackEvent::Length {
                                        id: Some(id.clone()),
                                        duration: length,
                                    }).ok();
                                }
                                prev_len = sink.len();
                                current = Some(slot);
                                playing = at.is_none();
                                let position = at.unwrap_or_default();
                                events
                                    .send(match playing {
                                        true => PlaybackEvent::Playing {
                                            id: Some(id),
                                            at: position,
                                        },
                                        false => PlaybackEvent::Paused {
                                            id: Some(id),
                                            at: position,
                                        },
                                    })
                                    .ok();
                            }
                            Err(error) => {
                                log::warn!("playback: cannot load {id}: {error:#}");
                                events.send(PlaybackEvent::Unavailable { id: Some(id) }).ok();
                            }
                        }
                    }
                    Command::Preload { id, segue } => {
                        if !segue {
                            continue;
                        }
                        let known = current.as_ref().is_some_and(|slot| slot.id == id);
                        if known || current.is_none() || queued.is_some() {
                            continue;
                        }
                        match load(&sink, &id) {
                            Ok(slot) => {
                                queued = Some(slot);
                                prev_len = sink.len();
                            }
                            Err(error) => log::warn!("playback: cannot preload {id}: {error:#}"),
                        }
                    }
                    Command::Play => {
                        if output.failed() || output.changed() {
                            events.send(PlaybackEvent::OutputChanged).ok();
                            return;
                        }
                        if let Some(slot) = &current {
                            sink.play();
                            playing = true;
                            events.send(PlaybackEvent::Playing {
                                id: Some(slot.id.clone()),
                                at: sink.get_pos(),
                            }).ok();
                        }
                    }
                    Command::Pause => {
                        playing = false;
                        let position = sink.get_pos();
                        sink.pause();
                        if let Some(slot) = &current {
                            events.send(PlaybackEvent::Paused {
                                id: Some(slot.id.clone()),
                                at: position,
                            }).ok();
                        }
                    }
                    Command::Seek(position) => {
                        if let Some(slot) = &current {
                            if let Err(error) = sink.try_seek(position) {
                                log::warn!("playback: cannot seek: {error}");
                            }
                            events.send(PlaybackEvent::Position {
                                id: Some(slot.id.clone()),
                                at: sink.get_pos(),
                            }).ok();
                        }
                    }
                    Command::Gain(level) => output.set_volume(level),
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
                    if let Some(slot) = &current {
                        events.send(PlaybackEvent::Ended { id: Some(slot.id.clone()) }).ok();
                    }
                    current = queued.take();
                    playing = current.is_some();
                    if let Some(slot) = &current {
                        if let Some(length) = slot.length {
                            events.send(PlaybackEvent::Length {
                                id: Some(slot.id.clone()),
                                duration: length,
                            }).ok();
                        }
                        events.send(PlaybackEvent::Position {
                            id: Some(slot.id.clone()),
                            at: sink.get_pos(),
                        }).ok();
                    }
                } else if playing && ticks >= report_every {
                    ticks = 0;
                    if let Some(slot) = &current {
                        events.send(PlaybackEvent::Position {
                            id: Some(slot.id.clone()),
                            at: sink.get_pos(),
                        }).ok();
                    }
                }
                prev_len = len;
            }
        }
    }
}

fn place(sink: &rodio::Player, id: &str, at: Option<Duration>) {
    let Some(at) = at else {
        return;
    };
    if let Err(error) = sink.try_seek(at) {
        log::warn!("playback: cannot start {id} at {}s: {error}", at.as_secs());
    }
}

fn load(sink: &rodio::Player, id: &str) -> Result<Slot> {
    let path =
        wire::path_from_track_id(id).ok_or_else(|| anyhow!("{id} is not a local track id"))?;
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let length = file.metadata().ok().map(|meta| meta.len());
    let reader = std::io::BufReader::new(file);

    let mut builder = rodio::Decoder::builder()
        .with_data(reader)
        .with_seekable(true);
    if let Some(length) = length {
        builder = builder.with_byte_len(length);
    }
    let source = builder.build().context("cannot decode audio")?;
    let duration = source.total_duration();
    sink.append(source);

    Ok(Slot {
        id: id.to_owned(),
        length: duration,
    })
}
