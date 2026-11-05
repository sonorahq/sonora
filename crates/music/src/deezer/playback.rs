//! Playback for Deezer: what [`crate::engine`] needs that is Deezer's own.
//!
//! One thing, really. The body arrives encrypted in 2048-byte stripes, so the stream decrypts
//! each one as it lands and what the buffer holds is always in the clear. From there rodio
//! decodes an ordinary file, and the threads, the queue, the preload and the gapless join are
//! the engine's.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use rodio::Source as _;
use serde::{Deserialize, Serialize};

use crate::deezer::client::DeezerClient;
use crate::deezer::{Stream, Striped};
use crate::engine::{self, Fetch, Loudness};
use crate::stream::Reader;
use crate::vault::{Recording, Shelf};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};

/// A track downloading and decrypting, and how long and how loud Deezer says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    duration: Option<Duration>,
    loudness: Option<Loudness>,
}

pub struct Factory {
    client: DeezerClient,
}

impl Factory {
    pub fn new(client: DeezerClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Deezer {
                client: self.client.clone(),
                quality: config.quality,
                shelf: config.vault.as_ref().map(|vault| vault.shelf("deezer")),
            },
            config,
        )
    }
}

struct Deezer {
    client: DeezerClient,
    /// The level every track is asked for at.
    quality: Quality,
    /// Where played tracks are kept, still striped the way the CDN sent them.
    shelf: Option<Shelf>,
}

/// What the gateway said about a track, kept beside it in the vault. The stream's id is enough
/// to derive its key again, so the key itself is never stored.
#[derive(Serialize, Deserialize)]
struct Facts {
    effective_id: String,
    duration_s: Option<u64>,
    lufs: Option<f32>,
}

impl Deezer {
    /// The track from the vault, when it was kept as `served`, the format the account's plan is
    /// offered at the level asked for. Keying by that rather than by the level means a copy a
    /// smaller plan was served is not replayed once the plan covers more. The bytes come back
    /// still striped and are decrypted on their way into memory, as they are off the network.
    async fn recall(&self, id: &str, served: &str) -> Option<Loaded> {
        let kept = self.shelf.as_ref()?.find(id, served).await?;
        let facts = serde_json::from_slice::<Facts>(&kept.facts)
            .inspect_err(|error| {
                log::warn!("playback: the kept facts of {id} do not read: {error}")
            })
            .ok()?;
        let key = self.client.track_key(&facts.effective_id);
        let stream = Stream::pulling(kept.source, Some(kept.length), Striped::new(&key))
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        log::debug!("playback: {id} started from the vault");
        Some(Loaded {
            stream,
            duration: facts.duration_s.map(Duration::from_secs),
            loudness: facts.lufs.map(|lufs| Loudness { lufs, peak: None }),
        })
    }
}

#[async_trait]
impl Fetch for Deezer {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader<Striped>>;

    fn name(&self) -> &'static str {
        "deezer"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Resolves the stream url and its key together, since one answer carries both, at the
    /// chosen level or the best below it the license covers. The download starts decrypting as
    /// it arrives.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let served = self.client.served(self.quality).await;
        if let Some(loaded) = self.recall(id, served).await {
            return Ok(loaded);
        }
        let opened = self.client.open_stream(id, self.quality).await?;
        let total = opened.response.content_length();
        let body = match &self.shelf {
            Some(shelf) => {
                let facts = Facts {
                    effective_id: opened.effective_id.clone(),
                    duration_s: opened.duration.map(|duration| duration.as_secs()),
                    lufs: opened.loudness.map(|loudness| loudness.lufs),
                };
                shelf.keep(
                    id,
                    served,
                    move || serde_json::to_vec(&facts).ok(),
                    opened.response,
                )
            }
            None => Recording::through(opened.response),
        };
        let stream = Stream::pulling(body, total, Striped::new(&opened.key))
            .primed()
            .await?;
        Ok(Loaded {
            stream,
            duration: opened.duration,
            loudness: opened.loudness,
        })
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness
    }

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        let total = loaded.stream.total().filter(|total| *total > 0)?;
        Some(loaded.stream.arrived() as f32 / total as f32)
    }

    /// Builds a decoder over a stream and places it at `at`. The bytes past the preroll are
    /// still arriving, so this only reads the header.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let mut builder = rodio::Decoder::builder()
            .with_data(loaded.stream.reader())
            .with_seekable(true);
        if let Some(total) = loaded.stream.total() {
            builder = builder.with_byte_len(total);
        }
        let mut decoder = match builder.build() {
            Ok(decoder) => decoder,
            Err(error) => {
                log::warn!("playback: cannot decode the deezer track {id}: {error}");
                return None;
            }
        };
        log::debug!(
            "playback: {id} decodes at {} Hz, {} channels, {:?} long",
            decoder.sample_rate(),
            decoder.channels(),
            decoder.total_duration()
        );
        if !at.is_zero()
            && let Err(error) = decoder.try_seek(at)
        {
            log::warn!(
                "playback: cannot start the deezer track {id} at {}s: {error}",
                at.as_secs()
            );
        }
        Some(decoder)
    }
}
