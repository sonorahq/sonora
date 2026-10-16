use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use librespot_core::{Session, SpotifyUri};
use librespot_playback::config::{Bitrate, PlayerConfig};
use librespot_playback::mixer::NoOpVolume;
use librespot_playback::player::{Player, PlayerEvent};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::audio::{Chain, Volume};
use crate::route::{Codec, Encoding, Normalisation, Route, TrackSignal};
use crate::sink::Cue;
use crate::spectrum::Spectrum;
use crate::spotify::sink::OutputSink;
use crate::{
    PlaybackConfig, PlaybackEvent, PlaybackEvents, PlaybackFactory, Player as MusicPlayer,
};

const TRACK_PREFIX: &str = "spotify:track:";
/// Position reports that can still come from before a seek or load: one already in the channel
/// and one for a packet decoded before the player read the command.
const STALE_POSITIONS: u8 = 2;

/// The engine's events, with one correction: librespot says `Playing` when the decoder is
/// ready and `Seeked` when it has moved, both before it has buffered or written anything. Each
/// waits here for the sink's next write, which the same player thread makes strictly after it.
pub struct Events {
    player: UnboundedReceiver<PlayerEvent>,
    output: UnboundedReceiver<()>,
    cue: Cue,
    written: UnboundedReceiver<()>,
    /// The `Playing` or `Seeked` waiting for the sink; a newer one replaces an older one.
    held: Option<PlaybackEvent>,
    /// Position reports seen while the cue refuses writes. A seek or load that took keeps the
    /// player thread busy, so a run of them means it never left the old position.
    stale: u8,
}

#[async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        loop {
            // librespot's event always precedes the write it led to, so it is taken first when
            // both are ready at once.
            tokio::select! {
                biased;
                event = self.player.recv() => {
                    let Some(event) = translate(event?) else { continue };
                    match event {
                        PlaybackEvent::Playing { .. } | PlaybackEvent::Seeked { .. } => {
                            self.cue.arm();
                            self.stale = 0;
                            self.held = Some(event);
                        }
                        PlaybackEvent::Position { .. } if self.cue.cleared() => {
                            self.stale += 1;
                            if self.stale > STALE_POSITIONS {
                                log::warn!("playback: the engine kept its old position, letting audio through");
                                self.cue.arm();
                                self.stale = 0;
                            }
                            return Some(event);
                        }
                        PlaybackEvent::Position { .. } | PlaybackEvent::Length { .. } => {
                            return Some(event);
                        }
                        event => {
                            self.held = None;
                            self.stale = 0;
                            return Some(event);
                        }
                    }
                }
                written = self.written.recv() => {
                    written?;
                    if let Some(event) = self.held.take() {
                        return Some(event);
                    }
                }
                changed = self.output.recv() => {
                    changed?;
                    return Some(PlaybackEvent::OutputChanged);
                }
            }
        }
    }
}

/// Builds a Spotify engine on a connected session.
pub struct Factory(Session);

impl Factory {
    pub fn new(session: Session) -> Self {
        Self(session)
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn MusicPlayer>, Box<dyn PlaybackEvents>) {
        let (engine, events) = Engine::start(self.0.clone(), config);
        (Box::new(engine), Box::new(events))
    }
}

/// The librespot player with our sink behind it. Every method queues a command to librespot's
/// own thread, which answers through `Events`.
pub struct Engine {
    player: Arc<Player>,
    volume: Volume,
    cue: Cue,
    spectrum: Spectrum,
    route: Route,
    gapless: bool,
    normalisation: bool,
}

impl Engine {
    fn start(session: Session, config: PlaybackConfig) -> (Self, Events) {
        let volume = Volume::new(config.gain);
        let (cue, written) = Cue::new();
        let spectrum = Spectrum::new();
        let route = Route::default();

        let player_config = PlayerConfig {
            bitrate: Bitrate::Bitrate320,
            normalisation: config.normalisation,
            gapless: config.gapless,
            position_update_interval: Some(config.position_interval),
            ..Default::default()
        };

        let sink_cue = cue.clone();
        let chain = Chain {
            volume: volume.clone(),
            equalizer: config.equalizer.clone(),
            spectrum: spectrum.clone(),
            meter: config.meter.clone(),
            route: route.clone(),
        };
        let (output_tx, output_rx) = unbounded_channel();
        let player = Player::new(player_config, session, Box::new(NoOpVolume), move || {
            OutputSink::boxed(sink_cue, chain, output_tx.clone())
        });

        let events = Events {
            player: player.get_player_event_channel(),
            output: output_rx,
            cue: cue.clone(),
            written,
            held: None,
            stale: 0,
        };
        let engine = Self {
            player,
            volume,
            cue,
            spectrum,
            route,
            gapless: config.gapless,
            normalisation: config.normalisation,
        };
        (engine, events)
    }

    fn load(&self, track_id: &str, at: Duration, seamless: bool) -> Result<()> {
        let uri = track_uri(track_id)?;

        if !seamless || !self.gapless {
            self.cue.clear();
        }
        self.learn(track_id);
        self.player.load(uri, true, at.as_millis() as u32);
        Ok(())
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        let uri = track_uri(track_id)?;

        self.cue.clear();
        self.learn(track_id);
        self.player.load(uri, false, at.as_millis() as u32);
        Ok(())
    }

    fn preload(&self, track_id: &str) -> Result<()> {
        self.player.preload(track_uri(track_id)?);
        Ok(())
    }

    fn play(&self) {
        self.player.play();
    }

    fn pause(&self) {
        self.player.pause();
    }

    fn seek(&self, position: Duration) {
        self.player.seek(position.as_millis() as u32);
        self.cue.clear();
    }

    fn set_gain(&self, gain: f32) {
        self.volume.set(gain);
    }

    fn spectrum(&self) -> Spectrum {
        self.spectrum.clone()
    }

    /// Notes what every Spotify track is: Ogg Vorbis, decoded and normalised by librespot.
    fn learn(&self, track_id: &str) {
        let encoding = Encoding {
            codec: Codec::Vorbis,
            bits: None,
            kbps: None,
        };
        self.route.learn(
            track_id,
            TrackSignal {
                encoding: Some(encoding),
                normalisation: match self.normalisation {
                    true => Normalisation::Limited,
                    false => Normalisation::Off,
                },
            },
        );
    }
}

impl MusicPlayer for Engine {
    fn load(&self, track_id: &str, at: Duration, seamless: bool) -> Result<()> {
        self.load(track_id, at, seamless)
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        self.load_paused_at(track_id, at)
    }

    fn preload(&self, track_id: &str, _segue: bool) -> Result<()> {
        self.preload(track_id)
    }

    fn play(&self) {
        self.play();
    }

    fn pause(&self) {
        self.pause();
    }

    fn seek(&self, position: Duration) {
        self.seek(position);
    }

    fn set_gain(&self, gain: f32) {
        self.set_gain(gain);
    }

    fn spectrum(&self) -> Option<Spectrum> {
        Some(self.spectrum())
    }

    fn route(&self) -> Option<Route> {
        Some(self.route.clone())
    }
}

fn track_uri(track_id: &str) -> Result<SpotifyUri> {
    SpotifyUri::from_uri(&format!("{TRACK_PREFIX}{track_id}"))
        .with_context(|| format!("{track_id} is not a track id"))
}

/// librespot's event as ours, or `None` for the ones the state has no use for. A denied audio
/// key is `Refused` and one refused for now is `Throttled`. Any other unavailability names the
/// track.
fn translate(event: PlayerEvent) -> Option<PlaybackEvent> {
    let millis = |position_ms: u32| Duration::from_millis(position_ms as u64);
    let track_id = |uri: SpotifyUri| Some(uri.to_id());

    match event {
        PlayerEvent::Loading {
            track_id: uri,
            position_ms,
            ..
        } => Some(PlaybackEvent::Loading {
            id: track_id(uri),
            at: millis(position_ms),
        }),
        PlayerEvent::Playing {
            track_id: uri,
            position_ms,
            ..
        } => Some(PlaybackEvent::Playing {
            id: track_id(uri),
            at: millis(position_ms),
        }),
        PlayerEvent::Paused {
            track_id: uri,
            position_ms,
            ..
        } => Some(PlaybackEvent::Paused {
            id: track_id(uri),
            at: millis(position_ms),
        }),
        PlayerEvent::PositionChanged {
            track_id: uri,
            position_ms,
            ..
        }
        | PlayerEvent::PositionCorrection {
            track_id: uri,
            position_ms,
            ..
        } => Some(PlaybackEvent::Position {
            id: track_id(uri),
            at: millis(position_ms),
        }),
        PlayerEvent::Seeked {
            track_id: uri,
            position_ms,
            ..
        } => Some(PlaybackEvent::Seeked {
            id: track_id(uri),
            at: millis(position_ms),
        }),
        PlayerEvent::Stopped { track_id: uri, .. }
        | PlayerEvent::EndOfTrack { track_id: uri, .. } => {
            Some(PlaybackEvent::Ended { id: track_id(uri) })
        }
        PlayerEvent::Unavailable { denied: true, .. } => Some(PlaybackEvent::Refused),
        PlayerEvent::Unavailable {
            throttled: true,
            track_id: uri,
            ..
        } => Some(PlaybackEvent::Throttled { id: track_id(uri) }),
        PlayerEvent::Unavailable { track_id: uri, .. } => {
            Some(PlaybackEvent::Unavailable { id: track_id(uri) })
        }
        _ => None,
    }
}
