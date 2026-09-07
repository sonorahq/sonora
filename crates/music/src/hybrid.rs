use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use ytmusic::YtMusic;

use crate::youtube::{self, playback::refusal};
use crate::{
    MusicApi, PlaybackConfig, PlaybackEvent, PlaybackEvents, PlaybackFactory, Player, Spectrum,
};

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

struct Resolve {
    epoch: u64,
    kind: Kind,
    id: String,
    at: Option<Duration>,
    seamless: bool,
    segue: bool,
}

pub struct HybridFactory {
    spotify: Arc<dyn MusicApi>,
    youtube: Arc<YtMusic>,
}

impl HybridFactory {
    pub fn new(spotify: Arc<dyn MusicApi>, youtube: Arc<YtMusic>) -> Self {
        Self { spotify, youtube }
    }
}

impl PlaybackFactory for HybridFactory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        let (inner, inner_events) =
            youtube::playback::Factory::new(self.youtube.clone()).start(config);
        let spectrum = inner.spectrum();

        let (commands, command_rx) = unbounded_channel();
        let (events, event_rx) = unbounded_channel();
        let spotify = self.spotify.clone();
        let youtube = self.youtube.clone();
        let failure = events.clone();

        let spawned = std::thread::Builder::new()
            .name("hybrid-playback".to_string())
            .spawn(move || run(spotify, youtube, inner, inner_events, command_rx, events));
        if let Err(error) = spawned {
            log::error!("hybrid: cannot spawn bridge thread: {error}");
            failure.send(PlaybackEvent::Unavailable { id: None }).ok();
        }

        (
            Box::new(Engine { commands, spectrum }),
            Box::new(Events(event_rx)),
        )
    }
}

struct Engine {
    commands: UnboundedSender<Command>,
    spectrum: Option<Spectrum>,
}

impl Player for Engine {
    fn load(&self, track_id: &str, seamless: bool) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_string(),
                at: None,
                seamless,
            })
            .context("cannot reach the hybrid playback engine")
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        self.commands
            .send(Command::Load {
                id: track_id.to_string(),
                at: Some(at),
                seamless: false,
            })
            .context("cannot reach the hybrid playback engine")
    }

    fn preload(&self, track_id: &str, segue: bool) -> Result<()> {
        self.commands
            .send(Command::Preload {
                id: track_id.to_string(),
                segue,
            })
            .context("cannot reach the hybrid playback engine")
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
        self.spectrum.clone()
    }
}

struct Events(UnboundedReceiver<PlaybackEvent>);

#[async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        self.0.recv().await
    }
}

fn run(
    spotify: Arc<dyn MusicApi>,
    youtube: Arc<YtMusic>,
    inner: Box<dyn Player>,
    inner_events: Box<dyn PlaybackEvents>,
    commands: UnboundedReceiver<Command>,
    events: UnboundedSender<PlaybackEvent>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("hybrid: cannot build bridge runtime: {error}");
            return;
        }
    };
    runtime.block_on(engine_loop(
        spotify,
        youtube,
        inner,
        inner_events,
        commands,
        events,
    ));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Play,
    Ahead,
}

struct Resolved {
    epoch: u64,
    id: String,
    kind: Kind,
    at: Option<Duration>,
    seamless: bool,
    segue: bool,
    video: Result<String>,
}

async fn engine_loop(
    spotify: Arc<dyn MusicApi>,
    youtube: Arc<YtMusic>,
    inner: Box<dyn Player>,
    mut inner_events: Box<dyn PlaybackEvents>,
    mut commands: UnboundedReceiver<Command>,
    events: UnboundedSender<PlaybackEvent>,
) {
    let mut cache: Option<(String, String)> = None;
    let (resolved, mut arrivals) = unbounded_channel::<Resolved>();

    let mut epoch = 0u64;
    let mut current_id: Option<String> = None;
    let mut pending: Option<u64> = None;
    let mut inflight: Option<tokio::task::AbortHandle> = None;

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::Load { id, at, seamless } => {
                        epoch += 1;
                        current_id = Some(id.clone());
                        if let Some(handle) = inflight.take() {
                            handle.abort();
                        }
                        pending = None;
                        let cached = cache.as_ref().filter(|(s, _)| *s == id).map(|(_, v)| v.clone());
                        if cached.is_none() {
                            pending = Some(epoch);
                            inflight = Some(spawn_resolve(
                                &spotify,
                                &youtube,
                                Resolve { epoch, kind: Kind::Play, id: id.clone(), at, seamless, segue: false },
                                &resolved,
                            ));
                        }
                        cache = None;
                        let Some(video_id) = cached else { continue };
                        cache = Some((id.clone(), video_id.clone()));
                        load(&*inner, &video_id, at, seamless);
                    }
                    Command::Preload { id, segue } => {
                        if cache.as_ref().is_some_and(|(s, _)| *s == id) {
                            continue;
                        }
                        spawn_resolve(
                            &spotify,
                            &youtube,
                            Resolve { epoch, kind: Kind::Ahead, id, at: None, seamless: false, segue },
                            &resolved,
                        );
                    }
                    Command::Play => inner.play(),
                    Command::Pause => inner.pause(),
                    Command::Seek(position) => inner.seek(position),
                    Command::Gain(gain) => inner.set_gain(gain),
                }
            }
            arrival = arrivals.recv() => {
                let Some(Resolved { epoch: arrived, id, kind, at, seamless, segue, video }) = arrival
                else {
                    break;
                };
                if arrived != epoch {
                    continue;
                }
                match kind {
                    Kind::Play => {
                        if pending != Some(arrived) {
                            continue;
                        }
                        pending = None;
                        inflight = None;
                        match video {
                            Ok(video_id) => {
                                cache = Some((id.clone(), video_id.clone()));
                                load(&*inner, &video_id, at, seamless);
                            }
                            Err(error) => {
                                log::warn!("hybrid: no YouTube Music match for {id}: {error:#}");
                                events.send(refusal(id, &error)).ok();
                            }
                        }
                    }
                    Kind::Ahead => {
                        let Ok(video_id) = video else { continue };
                        cache = Some((id.clone(), video_id.clone()));
                        if let Err(error) = inner.preload(&video_id, segue) {
                            log::warn!("hybrid: cannot preload {id}: {error:#}");
                        }
                    }
                }
            }
            event = inner_events.next() => {
                let Some(event) = event else { break };
                events.send(embed_id(event, current_id.as_deref())).ok();
            }
        }
    }
}

async fn resolve(
    spotify: &Arc<dyn MusicApi>,
    youtube: &Arc<YtMusic>,
    spotify_track_id: &str,
) -> Result<String> {
    let track = spotify
        .track(spotify_track_id)
        .await
        .context("cannot read the track's metadata from Spotify")?;

    let artists = match track.artist_refs.is_empty() {
        true => track.artists.clone(),
        false => track
            .artist_refs
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    };
    let query = if track.album.is_empty() {
        format!("{} {artists}", track.name)
    } else {
        format!("{} {artists} {}", track.name, track.album)
    };
    let candidates = youtube
        .search_songs(&query)
        .await
        .context("cannot search YouTube Music")?;

    candidates
        .into_iter()
        .find(|c| c.available && c.video_id.is_some())
        .and_then(|c| c.video_id)
        .context("no match on YouTube Music")
}

fn spawn_resolve(
    spotify: &Arc<dyn MusicApi>,
    youtube: &Arc<YtMusic>,
    req: Resolve,
    resolved: &UnboundedSender<Resolved>,
) -> tokio::task::AbortHandle {
    let spotify = spotify.clone();
    let youtube = youtube.clone();
    let resolved = resolved.clone();
    let Resolve {
        epoch,
        kind,
        id,
        at,
        seamless,
        segue,
    } = req;
    tokio::spawn(async move {
        let video = resolve(&spotify, &youtube, &id).await;
        resolved
            .send(Resolved {
                epoch,
                id,
                kind,
                at,
                seamless,
                segue,
                video,
            })
            .ok();
    })
    .abort_handle()
}

fn load(inner: &dyn Player, video_id: &str, at: Option<Duration>, seamless: bool) {
    let result = match at {
        Some(at) => inner.load_paused_at(video_id, at),
        None => inner.load(video_id, seamless),
    };
    if let Err(error) = result {
        log::error!("hybrid: cannot hand a track to the YouTube Music engine: {error:#}");
    }
}

fn embed_id(event: PlaybackEvent, id: Option<&str>) -> PlaybackEvent {
    match event {
        PlaybackEvent::Loading { at, .. } => PlaybackEvent::Loading {
            id: id.map(str::to_owned),
            at,
        },
        PlaybackEvent::Playing { at, .. } => PlaybackEvent::Playing {
            id: id.map(str::to_owned),
            at,
        },
        PlaybackEvent::Paused { at, .. } => PlaybackEvent::Paused {
            id: id.map(str::to_owned),
            at,
        },
        PlaybackEvent::Position { at, .. } => PlaybackEvent::Position {
            id: id.map(str::to_owned),
            at,
        },
        PlaybackEvent::Length { duration, .. } => PlaybackEvent::Length {
            id: id.map(str::to_owned),
            duration,
        },
        PlaybackEvent::Ended { .. } => PlaybackEvent::Ended {
            id: id.map(str::to_owned),
        },
        PlaybackEvent::Unavailable { .. } => PlaybackEvent::Unavailable {
            id: id.map(str::to_owned),
        },
        other => other,
    }
}
