mod media;

pub(super) use media::OfflineFactory;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use futures::future::BoxFuture;
use tokio::sync::oneshot;

use crate::connect::{Collection, Command, Connect, QueuedTrack, Request, Snapshot, Start};
use crate::{
    Encoding, Fraction, Offer, PlaybackConfig, PlaybackEvents, PlaybackFactory,
    Player as MusicPlayer, Quality,
};
use crate::{engine, soloist};

use super::session::Session;

/// The pause between Spotify downloads once Spotify has refused one. It refuses audio keys asked
/// for in a burst, the same keys playback needs, so after a refusal downloads ask about as often as
/// listening does.
pub(crate) const DOWNLOAD_GAP: Duration = Duration::from_secs(4);
/// Spotify’s Vorbis levels, best first: Ogg Vorbis at 320, 160 and 96 kbps.
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
            let (daemon, events) = soloist::start(&config);
            return (
                Box::new(Official {
                    _daemon: daemon,
                    connect: self.connect.clone(),
                    context: Mutex::default(),
                    held: Mutex::default(),
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
    held: Mutex<Option<Snapshot>>,
    audio: media::Audio,
}

impl Official {
    fn command(&self, command: Command) {
        self.connect.send(Request::ControlLocal(command));
    }

    fn selection(&self, id: &str, at: Duration) -> Result<Snapshot> {
        anyhow::ensure!(
            id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "invalid Spotify track ID"
        );
        Ok(Snapshot {
            context: self.context.lock().unwrap().take(),
            track: Some(QueuedTrack::new(id.to_owned(), false)),
            position: at,
            playing: true,
            ..Default::default()
        })
    }

    fn start(&self, state: Snapshot) {
        self.command(Command::Start(Start {
            state: Arc::new(state),
            restart: true,
            ..Default::default()
        }));
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
        let (done, answer) = oneshot::channel();
        tokio::spawn(async move {
            let _ = done.send(media::download(&session, &shelf, &id, quality, progress).await);
        });
        Box::pin(async move { answer.await.context("the spotify session stopped")? })
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
        self.held.lock().unwrap().take();
        self.start(self.selection(track_id, at)?);
        Ok(())
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        *self.held.lock().unwrap() = Some(self.selection(track_id, at)?);
        Ok(())
    }

    fn preload(&self, _track_id: &str, _segue: bool) -> Result<()> {
        Ok(())
    }

    fn within(&self, collection: Option<&Collection>) {
        *self.context.lock().unwrap() = collection.cloned();
    }

    fn play(&self) {
        match self.held.lock().unwrap().take() {
            Some(state) => self.start(state),
            None => self.command(Command::Play),
        }
    }

    fn pause(&self) {
        self.command(Command::Pause);
    }

    fn seek(&self, position: Duration) {
        if let Some(state) = self.held.lock().unwrap().as_mut() {
            state.position = position;
        } else {
            self.command(Command::Seek(position));
        }
    }

    fn set_gain(&self, gain: f32) {
        self.command(Command::Volume(gain.clamp(0.0, 1.0).cbrt()));
    }
}
