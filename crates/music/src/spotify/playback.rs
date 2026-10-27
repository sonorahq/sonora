use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use futures::future::BoxFuture;
use librespot_core::{Session, SpotifyUri};
use librespot_playback::config::{Bitrate, PlayerConfig};
use librespot_playback::mixer::NoOpVolume;
use librespot_playback::offline::Offline;
use librespot_playback::player::{Player, PlayerEvent};
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::audio::{Chain, Volume};

use crate::sink::Cue;
use crate::spectrum::Spectrum;
use crate::spotify::offline::{self, Kept};
use crate::spotify::sink::OutputSink;
use crate::vault::Shelf;
use crate::{
    Encoding, Fraction, Normalisation, Offer, PlaybackConfig, PlaybackEvent, PlaybackEvents,
    PlaybackFactory, Player as MusicPlayer, Quality, TrackSignal,
};

const TRACK_PREFIX: &str = "spotify:track:";
/// The levels Spotify serves, best first: Ogg Vorbis at 320, 160 and 96 kbps.
/// The pause between Spotify downloads once Spotify has refused one. It refuses audio keys asked
/// for in a burst, the same keys playback needs, so after a refusal downloads ask about as often as
/// listening does.
pub(crate) const DOWNLOAD_GAP: Duration = Duration::from_secs(4);
pub(crate) const QUALITIES: &[Offer] = &[
    Offer {
        quality: Quality::High,
        encoding: Encoding::Vorbis { kbps: 320 },
        up_to: false,
    },
    Offer {
        quality: Quality::Normal,
        encoding: Encoding::Vorbis { kbps: 160 },
        up_to: false,
    },
    Offer {
        quality: Quality::Low,
        encoding: Encoding::Vorbis { kbps: 96 },
        up_to: false,
    },
];
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
    /// The `Signal` events `Engine::learn` sends as it loads a track.
    signals: UnboundedReceiver<PlaybackEvent>,
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
                        // `forward` has armed the cue already, off the app's thread.
                        PlaybackEvent::Playing { .. } | PlaybackEvent::Seeked { .. } => {
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
                Some(event) = self.signals.recv() => return Some(event),
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
    /// Where `learn` reports each track's sound, read by `Events`.
    signals: UnboundedSender<PlaybackEvent>,
    gapless: bool,
    normalisation: bool,
    /// The encoding of the offer the quality asks librespot for.
    encoding: Option<Encoding>,
    /// The session downloads are fetched on.
    session: Session,
    /// Where downloads are kept, which librespot also plays them from.
    shelf: Option<Shelf>,
    /// The level downloads are fetched at, the same one plays ask for.
    bitrate: Bitrate,
}

impl Engine {
    fn start(session: Session, config: PlaybackConfig) -> (Self, Events) {
        let volume = Volume::new(config.gain);
        let (cue, written) = Cue::new();
        let spectrum = Spectrum::new();
        let (signals, signal_rx) = unbounded_channel();

        let shelf = config.vault.as_ref().map(|vault| vault.shelf("spotify"));
        let offline = shelf
            .clone()
            .map(|shelf| Arc::new(Kept(shelf)) as Arc<dyn Offline>);
        let player_config = PlayerConfig {
            bitrate: bitrate(config.quality),
            offline,
            normalisation: config.normalisation,
            gapless: config.gapless,
            position_update_interval: Some(config.position_interval),
            ..Default::default()
        };

        let sink_cue = cue.clone();
        let chain = Chain {
            volume: volume.clone(),
            effects: config.effects.clone(),
            equalizer: config.equalizer.clone(),
            spectrum: spectrum.clone(),
            meter: config.meter.clone(),
            analyzer: config.analyzer.clone(),
        };
        let (output_tx, output_rx) = unbounded_channel();
        let player = Player::new(
            player_config,
            session.clone(),
            Box::new(NoOpVolume),
            move || OutputSink::boxed(sink_cue, chain, output_tx.clone()),
        );

        let events = Events {
            player: forward(player.get_player_event_channel(), cue.clone()),
            output: output_rx,
            cue: cue.clone(),
            written,
            signals: signal_rx,
            held: None,
            stale: 0,
        };
        let engine = Self {
            player,
            volume,
            cue,
            spectrum,
            signals,
            gapless: config.gapless,
            normalisation: config.normalisation,
            encoding: offered(config.quality),
            session,
            shelf,
            bitrate: bitrate(config.quality),
        };
        (engine, events)
    }

    fn load(&self, track_id: &str, at: Duration, seamless: bool) -> Result<()> {
        let uri = track_uri(track_id)?;
        log::debug!("playback: spotify loads {track_id} at {at:?}, seamless {seamless}");

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

    /// Clears the queue before librespot hears of the seek. A seek in a file already on disk
    /// lands at once, and a clear after its `Seeked` would refuse the new audio with nothing left
    /// to let it through, while librespot decodes on unheard.
    fn seek(&self, position: Duration) {
        self.cue.clear();
        log::debug!("playback: spotify seeks to {position:?}");
        self.player.seek(position.as_millis() as u32);
    }

    fn set_gain(&self, gain: f32) {
        self.volume.set(gain);
    }

    fn spectrum(&self) -> Spectrum {
        self.spectrum.clone()
    }

    /// Reports what every Spotify track is: Ogg Vorbis at the offer asked for, decoded by
    /// librespot to 44.1 kHz stereo and normalised there.
    fn learn(&self, track_id: &str) {
        let track = TrackSignal {
            encoding: self.encoding,
            rate: SAMPLE_RATE,
            channels: NUM_CHANNELS.into(),
            normalisation: match self.normalisation {
                true => Normalisation::Limited,
                false => Normalisation::Off,
            },
        };
        let id = track_id.to_owned();
        self.signals.send(PlaybackEvent::Signal { id, track }).ok();
    }
}

impl MusicPlayer for Engine {
    fn download(&self, track_id: &str, progress: Fraction) -> BoxFuture<'static, Result<()>> {
        let Some(shelf) = self.shelf.clone() else {
            return Box::pin(async { Err(anyhow!("spotify keeps no downloads here")) });
        };
        let (done, answer) = oneshot::channel();
        let session = self.session.clone();
        let id = track_id.to_owned();
        let bitrate = self.bitrate;
        self.session.spawn(async move {
            let fetched = offline::download(&session, &shelf, &id, bitrate).await;
            progress.set(1.0);
            done.send(fetched).ok();
        });
        Box::pin(async move { answer.await.context("the spotify session stopped")? })
    }

    fn downloaded(&self, track_id: &str) -> bool {
        self.shelf
            .as_ref()
            .is_some_and(|shelf| shelf.holds(track_id))
    }

    fn plays_offline(&self, track_id: &str) -> bool {
        self.shelf
            .as_ref()
            .is_some_and(|shelf| shelf.may_hold(track_id))
    }

    fn discard(&self, track_id: &str) {
        if let Some(shelf) = &self.shelf {
            shelf.discard(track_id);
        }
    }

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
}

/// Passes librespot's events on and arms `cue` the moment one says audio from a new track or
/// position follows. Arming cannot wait for whoever reads the events: until it happens every
/// write is refused at once, so librespot decodes on unheard and the track would start seconds
/// in. Each event goes on before the cue is armed, so it is always ahead of the write the arm
/// lets through.
fn forward(mut events: UnboundedReceiver<PlayerEvent>, cue: Cue) -> UnboundedReceiver<PlayerEvent> {
    let (passed, received) = unbounded_channel();
    let spawned = std::thread::Builder::new()
        .name("spotify-events".into())
        .spawn(move || {
            while let Some(event) = events.blocking_recv() {
                let arms = matches!(
                    event,
                    PlayerEvent::Playing { .. } | PlayerEvent::Seeked { .. }
                );
                if passed.send(event).is_err() {
                    return;
                }
                if arms {
                    cue.arm();
                }
            }
        });
    if let Err(error) = spawned {
        log::error!("playback: cannot spawn the spotify event thread: {error}");
    }
    received
}

pub(crate) fn track_uri(track_id: &str) -> Result<SpotifyUri> {
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

/// The bitrate librespot asks for at a level. It steps down by itself when the account is not
/// allowed that one.
fn bitrate(quality: Quality) -> Bitrate {
    match quality.within(QUALITIES) {
        Quality::HiRes | Quality::Lossless | Quality::High => Bitrate::Bitrate320,
        Quality::Normal => Bitrate::Bitrate160,
        Quality::Low => Bitrate::Bitrate96,
    }
}

/// The encoding of the offer librespot is asked for at a level.
fn offered(quality: Quality) -> Option<Encoding> {
    let level = quality.within(QUALITIES);
    QUALITIES
        .iter()
        .find(|offer| offer.quality == level)
        .map(|offer| offer.encoding)
}
