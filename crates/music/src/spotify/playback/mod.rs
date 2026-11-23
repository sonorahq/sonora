mod access_point;
mod media;

pub(super) use access_point::AudioKeys;
pub(super) use media::OfflineFactory;

use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use futures::future::BoxFuture;
use tokio::sync::oneshot;

use crate::connect::Collection;
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
}

impl Factory {
    pub fn new(session: Session) -> Self {
        Self { session }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn MusicPlayer>, Box<dyn PlaybackEvents>) {
        let audio = media::Audio {
            shelf: config.vault.as_ref().map(|vault| vault.shelf("spotify")),
            session: Some(self.session.clone()),
            quality: config.quality,
        };
        if soloist::ready() {
            let (engine, events) = soloist::start(&config, self.session.soloist_playback().clone());
            return (Box::new(Official { engine, audio }), Box::new(events));
        }
        engine::start(audio, config)
    }

    fn qualities(&self) -> Option<&'static [Offer]> {
        soloist::ready().then_some(soloist::QUALITIES)
    }
}

/// Soloist as the Spotify engine. It streams every track, a downloaded one too, so nothing
/// plays without a network while it is in use. A track is only fetched ahead as the next in the
/// queue, which Soloist then joins to the current one by itself.
struct Official {
    engine: soloist::Engine,
    audio: media::Audio,
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
        validate_track(track_id)?;
        self.engine.load(track_id, at, true);
        Ok(())
    }

    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()> {
        validate_track(track_id)?;
        self.engine.load(track_id, at, false);
        Ok(())
    }

    fn preload(&self, track_id: &str, segue: bool) -> Result<()> {
        validate_track(track_id)?;
        if segue {
            self.engine.line_up(track_id);
        }
        Ok(())
    }

    fn unline(&self) {
        self.engine.line_down();
    }

    fn within(&self, collection: Option<&Collection>) {
        let uri = collection.and_then(|collection| match collection {
            Collection::Album(id) => Some(format!("spotify:album:{id}")),
            Collection::Playlist(id) => Some(format!("spotify:playlist:{id}")),
            Collection::Saved | Collection::Radio(_) | Collection::CollectionRadio(..) => None,
        });
        self.engine.within(uri);
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

/// Rejects malformed IDs before sending them to Soloist.
fn validate_track(id: &str) -> Result<()> {
    anyhow::ensure!(
        id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "invalid Spotify track ID"
    );
    Ok(())
}
