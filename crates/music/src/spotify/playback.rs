use std::sync::Arc;
use std::time::{Duration, Instant};

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
use crate::soloist;
use crate::spectrum::Spectrum;
use crate::spotify::offline::{self, Kept};
use crate::spotify::sink::OutputSink;
use crate::vault::Shelf;
use crate::{
    Encoding, Fraction, Normalisation, Offer, PlaybackConfig, PlaybackEvent, PlaybackEvents,
    PlaybackFactory, Player as MusicPlayer, Quality, TrackSignal,
};

const TRACK_PREFIX: &str = "spotify:track:";
/// The pause between Spotify downloads once Spotify has refused one. It refuses audio keys asked
/// for in a burst, the same keys playback needs, so after a refusal downloads ask about as often as
/// listening does.
pub(crate) const DOWNLOAD_GAP: Duration = Duration::from_secs(4);
/// The levels librespot is served, best first: Ogg Vorbis at 320, 160 and 96 kbps.
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
/// How much of another track the output may still hold ahead of a write before its event waits
/// for that to be heard. Less than this is within a packet of the sound.
const AUDIBLE: Duration = Duration::from_millis(5);

/// The engine's events, with two corrections. librespot says `Playing` when the decoder is
/// ready and `Seeked` when it has moved, both before it has buffered or written anything, so
/// each waits here for the sink's next write, which the same player thread makes strictly after
/// it, and after a gapless join also for the end of the track before it to be heard. And every
/// position librespot gives is the decoder's, which runs ahead of the sound by what the output
/// has queued, so that is taken off.
pub struct Events {
    player: UnboundedReceiver<PlayerEvent>,
    output: UnboundedReceiver<()>,
    cue: Cue,
    written: UnboundedReceiver<()>,
    /// The `Signal` events `Engine::learn` sends as it loads a track.
    signals: UnboundedReceiver<PlaybackEvent>,
    /// The `Playing` or `Seeked` waiting for the sink; a newer one replaces an older one.
    held: Option<PlaybackEvent>,
    /// When the held event's audio is heard, once its write went through behind the end of the
    /// track before it.
    release: Option<Instant>,
    /// The session whose runtime waits `release` out. `next` is awaited on the app's own
    /// executor, where a Tokio timer has no reactor and panics, so the wait runs over there
    /// and its deadline comes back on `due`.
    session: Session,
    waited: UnboundedSender<Instant>,
    due: UnboundedReceiver<Instant>,
    /// The track whose audio was last let through, which a resume carries on.
    heard: Option<String>,
    /// Position reports seen while the cue refuses writes. A seek or load that took keeps the
    /// player thread busy, so a run of them means it never left the old position.
    stale: u8,
}

impl Events {
    /// Lets the held event out once its first write is through. What the output still holds
    /// ahead of that write is the track carrying on after a resume, so it comes off the
    /// position, and the end of another track after a gapless join, so the event waits it out.
    fn arrived(&mut self) -> Option<PlaybackEvent> {
        let event = self.held.take()?;
        let ahead = self.cue.until_heard();
        if ahead < AUDIBLE {
            return Some(self.let_through(event));
        }
        let resumed = event
            .id()
            .is_some_and(|id| self.heard.as_deref() == Some(id));
        match (resumed, event) {
            (true, PlaybackEvent::Playing { id, at }) if at >= ahead => {
                Some(self.let_through(PlaybackEvent::Playing { id, at: at - ahead }))
            }
            (_, event) => {
                let at = Instant::now() + ahead;
                self.held = Some(event);
                self.release = Some(at);
                let waited = self.waited.clone();
                self.session.spawn(async move {
                    tokio::time::sleep_until(at.into()).await;
                    waited.send(at).ok();
                });
                None
            }
        }
    }

    fn let_through(&mut self, event: PlaybackEvent) -> PlaybackEvent {
        self.release = None;
        self.heard = event.id().map(str::to_owned);
        event
    }

    /// `event` with the decoder's lead over the sound taken off its position. `None` while the
    /// output still plays the track before the one the decoder reports.
    fn heard(&self, event: PlaybackEvent) -> Option<PlaybackEvent> {
        let ahead = self.cue.queued();
        match event {
            PlaybackEvent::Position { id, at } => Some(PlaybackEvent::Position {
                id,
                at: at.checked_sub(ahead)?,
            }),
            PlaybackEvent::Paused { id, at } => Some(PlaybackEvent::Paused {
                id,
                at: at.saturating_sub(ahead),
            }),
            event => Some(event),
        }
    }
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
                            self.release = None;
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
                            // a report for a track the output has not reached is no use yet
                            if let Some(event) = self.heard(event) {
                                return Some(event);
                            }
                        }
                        event => {
                            self.held = None;
                            self.release = None;
                            self.stale = 0;
                            if let Some(event) = self.heard(event) {
                                return Some(event);
                            }
                        }
                    }
                }
                written = self.written.recv() => {
                    written?;
                    if let Some(event) = self.arrived() {
                        return Some(event);
                    }
                }
                Some(at) = self.due.recv() => {
                    // a wait that a newer event called off still reports its deadline
                    if self.release != Some(at) {
                        continue;
                    }
                    self.release = None;
                    if let Some(event) = self.held.take() {
                        return Some(self.let_through(event));
                    }
                }
                changed = self.output.recv() => {
                    changed?;
                    return Some(PlaybackEvent::OutputChanged { heard: None });
                }
                Some(event) = self.signals.recv() => return Some(event),
            }
        }
    }
}

/// Builds a Spotify engine on a session: Soloist's while the listener has it set up, and
/// librespot's otherwise.
pub struct Factory {
    session: Session,
    /// Whether Soloist may play. A session that never connects plays downloads through
    /// librespot whatever is set up, since Soloist only streams.
    soloist: bool,
}

impl Factory {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            soloist: true,
        }
    }

    /// A factory for a session that never connects.
    pub fn offline(session: Session) -> Self {
        Self {
            session,
            soloist: false,
        }
    }

    fn official(&self) -> bool {
        self.soloist && soloist::ready()
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn MusicPlayer>, Box<dyn PlaybackEvents>) {
        if self.official() {
            let keeper = Keeper::new(&self.session, &config);
            let (engine, events) = soloist::start(&config);
            return (Box::new(Official { engine, keeper }), Box::new(events));
        }
        let (engine, events) = Engine::start(self.session.clone(), config);
        (Box::new(engine), Box::new(events))
    }

    fn qualities(&self) -> Option<&'static [Offer]> {
        self.official().then_some(soloist::QUALITIES)
    }
}

/// The downloads of a Spotify account. Both engines keep one, since librespot is what fetches
/// a download whichever of them plays.
struct Keeper {
    /// The session downloads are fetched on.
    session: Session,
    /// Where downloads are kept, which librespot also plays them from.
    shelf: Option<Shelf>,
    /// The level downloads are fetched at, the same one librespot plays ask for.
    bitrate: Bitrate,
}

impl Keeper {
    fn new(session: &Session, config: &PlaybackConfig) -> Self {
        Self {
            session: session.clone(),
            shelf: config.vault.as_ref().map(|vault| vault.shelf("spotify")),
            bitrate: bitrate(config.quality),
        }
    }

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

    fn holds(&self, track_id: &str) -> bool {
        self.shelf
            .as_ref()
            .is_some_and(|shelf| shelf.holds(track_id))
    }

    fn may_hold(&self, track_id: &str) -> bool {
        self.shelf
            .as_ref()
            .is_some_and(|shelf| shelf.may_hold(track_id))
    }

    fn discard(&self, track_id: &str) {
        if let Some(shelf) = &self.shelf {
            shelf.discard(track_id);
        }
    }
}

/// Soloist as the Spotify engine. It streams every track, a downloaded one too, so nothing
/// plays without a network while it is in use. A track is only fetched ahead as the next in the
/// queue, which Soloist then joins to the current one by itself.
struct Official {
    engine: soloist::Engine,
    keeper: Keeper,
}

impl MusicPlayer for Official {
    fn download(&self, track_id: &str, progress: Fraction) -> BoxFuture<'static, Result<()>> {
        self.keeper.download(track_id, progress)
    }

    fn downloaded(&self, track_id: &str) -> bool {
        self.keeper.holds(track_id)
    }

    fn plays_offline(&self, _track_id: &str) -> bool {
        false
    }

    fn discard(&self, track_id: &str) {
        self.keeper.discard(track_id);
    }

    fn load(&self, track_id: &str, at: Duration, _seamless: bool) -> Result<()> {
        track_uri(track_id)?;
        self.engine.load(track_id, at, true);
        Ok(())
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        track_uri(track_id)?;
        self.engine.load(track_id, at, false);
        Ok(())
    }

    fn preload(&self, track_id: &str, segue: bool) -> Result<()> {
        track_uri(track_id)?;
        if segue {
            self.engine.line_up(track_id);
        }
        Ok(())
    }

    fn play(&self) {
        self.engine.play();
    }

    fn pause(&self) {
        self.engine.pause();
    }

    fn seek(&self, position: Duration) {
        self.engine.seek(position);
    }

    fn set_gain(&self, gain: f32) {
        self.engine.set_gain(gain);
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
    keeper: Keeper,
}

impl Engine {
    fn start(session: Session, config: PlaybackConfig) -> (Self, Events) {
        let volume = Volume::new(config.gain);
        let (cue, written) = Cue::new();
        let spectrum = Spectrum::new();
        let (signals, signal_rx) = unbounded_channel();

        let keeper = Keeper::new(&session, &config);
        let offline = keeper
            .shelf
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
            equalizer: config.equalizer.clone(),
            spectrum: spectrum.clone(),
            meter: config.meter.clone(),
            analyzer: config.analyzer.clone(),
        };
        let (output_tx, output_rx) = unbounded_channel();
        let (waited, due) = unbounded_channel();
        let waiting = session.clone();
        let player = Player::new(player_config, session, Box::new(NoOpVolume), move || {
            OutputSink::boxed(sink_cue, chain, output_tx)
        });

        let events = Events {
            player: forward(player.get_player_event_channel(), cue.clone()),
            output: output_rx,
            cue: cue.clone(),
            written,
            signals: signal_rx,
            held: None,
            release: None,
            session: waiting,
            waited,
            due,
            heard: None,
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
            keeper,
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
            direct: false,
        };
        let id = track_id.to_owned();
        self.signals.send(PlaybackEvent::Signal { id, track }).ok();
    }
}

impl MusicPlayer for Engine {
    fn download(&self, track_id: &str, progress: Fraction) -> BoxFuture<'static, Result<()>> {
        self.keeper.download(track_id, progress)
    }

    fn downloaded(&self, track_id: &str) -> bool {
        self.keeper.holds(track_id)
    }

    fn plays_offline(&self, track_id: &str) -> bool {
        self.keeper.may_hold(track_id)
    }

    fn discard(&self, track_id: &str) {
        self.keeper.discard(track_id);
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

    fn set_initial_gain(&self, gain: f32) {
        self.volume.reset(gain);
    }

    fn spectrum(&self) -> Option<Spectrum> {
        Some(self.spectrum())
    }
}

/// Passes librespot's events on and arms `cue` the moment one says audio from a new track or
/// position follows. It also keeps the memory of a denied audio key, which decides whether
/// Soloist is offered at the next start: set on a denial and dropped once a track plays. Arming cannot wait for whoever reads the events: until it happens every
/// write is refused at once, so librespot decodes on unheard and the track would start seconds
/// in. Each event goes on before the cue is armed, so it is always ahead of the write the arm
/// lets through.
fn forward(mut events: UnboundedReceiver<PlayerEvent>, cue: Cue) -> UnboundedReceiver<PlayerEvent> {
    let (passed, received) = unbounded_channel();
    let spawned = std::thread::Builder::new()
        .name("spotify-events".into())
        .spawn(move || {
            while let Some(event) = events.blocking_recv() {
                match event {
                    PlayerEvent::Unavailable { denied: true, .. } => soloist::denied(),
                    PlayerEvent::Playing { .. } => soloist::granted(),
                    _ => {}
                }
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
