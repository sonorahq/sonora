mod media;

pub(super) use media::OfflineFactory;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use futures::future::BoxFuture;
use tokio::sync::oneshot;

use music::connect::{Collection, Command, Connect, QueuedTrack, Request, Snapshot, Start};
use music::{
    Encoding, Fraction, Offer, PlaybackConfig, PlaybackEvents, PlaybackFactory,
    Player as MusicPlayer, Quality,
};

use super::session::Session;

/// Initial interval for adaptive Spotify download pacing.
pub(crate) const DOWNLOAD_GAP: Duration = Duration::from_secs(2);
/// Spotify's Vorbis levels, best first.
pub(crate) const QUALITIES: &[Offer] = &[VORBIS_HIGH, VORBIS_NORMAL, VORBIS_LOW];

pub(super) const VORBIS_LOW: Offer = vorbis(Quality::Low, 96);
pub(super) const VORBIS_NORMAL: Offer = vorbis(Quality::Normal, 160);
pub(super) const VORBIS_HIGH: Offer = vorbis(Quality::High, 320);

const fn vorbis(quality: Quality, kbps: u32) -> Offer {
    Offer {
        quality,
        encoding: Encoding::Vorbis { kbps },
        up_to: false,
    }
}

/// Chooses Sonora audio playback or the paired Soloist player.
pub struct Factory {
    session: Session,
    connect: Arc<Connect>,
}

impl Factory {
    pub fn new(session: Session, connect: Arc<Connect>) -> Self {
        Self { session, connect }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn MusicPlayer>, Box<dyn PlaybackEvents>) {
        let audio = media::Audio {
            shelf: config.vault.as_ref().map(|vault| vault.shelf("spotify")),
            session: Some(self.session.clone()),
            quality: config.quality,
        };
        if soloist::ready_for(self.session.username()) {
            let (daemon, events) = soloist::start(self.session.username(), &config);
            return (
                Box::new(Official {
                    _daemon: daemon,
                    connect: self.connect.clone(),
                    context: Mutex::default(),
                    audio,
                }),
                events,
            );
        }
        engine::start(audio, config)
    }

    fn qualities(&self) -> Option<&'static [Offer]> {
        soloist::ready_for(self.session.username()).then_some(soloist::QUALITIES)
    }
}

/// Owns the daemon and handles engine restores until the Connect roster takes over.
struct Official {
    _daemon: oneshot::Sender<()>,
    connect: Arc<Connect>,
    context: Mutex<Option<Collection>>,
    audio: media::Audio,
}

impl Official {
    fn command(&self, command: Command) {
        self.connect.send(Request::ControlLocal(command));
    }

    fn select(&self, id: &str, at: Duration, playing: bool) -> Result<()> {
        anyhow::ensure!(
            id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "invalid Spotify track ID"
        );
        let state = Snapshot {
            context: self.context.lock().unwrap().take(),
            track: Some(QueuedTrack::new(id.to_owned(), false)),
            position: at,
            playing,
            ..Default::default()
        };
        self.command(Command::Start(Start {
            state: Arc::new(state),
            restart: true,
            ..Default::default()
        }));
        Ok(())
    }
}

impl MusicPlayer for Official {
    fn download(&self, track_id: &str, progress: Fraction) -> BoxFuture<'static, Result<()>> {
        let (Some(session), Some(shelf)) = (self.audio.session.clone(), self.audio.shelf.clone())
        else {
            return Box::pin(async { Err(anyhow!("spotify keeps no downloads here")) });
        };
        let id = track_id.to_owned();
        let quality = self.audio.quality;
        Box::pin(async move { media::download(&session, &shelf, &id, quality, progress).await })
    }

    fn downloaded(&self, track_id: &str) -> bool {
        self.audio
            .shelf
            .as_ref()
            .is_some_and(|shelf| shelf.holds(track_id))
    }

    fn plays_offline(&self, _track_id: &str) -> bool {
        false
    }

    fn discard(&self, track_id: &str) {
        if let Some(shelf) = &self.audio.shelf {
            shelf.discard(track_id);
        }
    }

    fn load(&self, track_id: &str, at: Duration, _seamless: bool) -> Result<()> {
        self.select(track_id, at, true)
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        self.select(track_id, at, false)
    }

    fn preload(&self, _track_id: &str, _segue: bool) -> Result<()> {
        Ok(())
    }

    fn within(&self, collection: Option<&Collection>) {
        *self.context.lock().unwrap() = collection.cloned();
    }

    fn play(&self) {
        self.command(Command::Play);
    }

    fn pause(&self) {
        self.command(Command::Pause);
    }

    fn seek(&self, position: Duration) {
        self.command(Command::Seek(position));
    }

    fn set_gain(&self, gain: f32) {
        self.command(Command::Volume(gain.clamp(0.0, 1.0).cbrt()));
    }
}
