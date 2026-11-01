//! Playback for Yandex Music: what [`crate::engine`] needs that is Yandex's own.
//!
//! The file arrives encrypted with AES in counter mode, so the stream decrypts each chunk as it
//! lands and what the buffer holds is always in the clear. From there rodio decodes an ordinary
//! FLAC, AAC or MP3 file, and the threads, the queue, the preload and the gapless join are the
//! engine's.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use rodio::Source as _;
use serde::{Deserialize, Serialize};

use crate::engine::{self, Fetch};
use crate::stream::Reader;
use crate::vault::{Recording, Shelf};
use crate::yandex::client::{self, YandexClient};
use crate::yandex::{Sealed, Stream};
use crate::{Encoding, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};

/// A track downloading and decrypting, and what it is encoded as.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    encoding: Option<Encoding>,
}

pub struct Factory {
    client: YandexClient,
}

impl Factory {
    pub fn new(client: YandexClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Yandex {
                client: self.client.clone(),
                quality: config.quality,
                shelf: config.vault.as_ref().map(|vault| vault.shelf("yandex")),
            },
            config,
        )
    }
}

struct Yandex {
    client: YandexClient,
    /// The level every track is asked for at.
    quality: Quality,
    /// Where played tracks are kept, still encrypted the way the storage sent them.
    shelf: Option<Shelf>,
}

/// What `get-file-info` said about a file, kept beside it in the vault. The key is per file and
/// cannot be derived again, so it is kept too, inside the vault's own encryption.
#[derive(Serialize, Deserialize)]
struct Facts {
    key: Option<String>,
    codec: Option<String>,
    bitrate: Option<u64>,
}

impl Yandex {
    /// The track from the vault, when it was kept at the level `served` names. The bytes come
    /// back still encrypted and are decrypted on their way into memory, as they are off the
    /// network.
    async fn recall(&self, id: &str, served: &str) -> Option<Loaded> {
        let kept = self.shelf.as_ref()?.find(id, served).await?;
        let facts = serde_json::from_slice::<Facts>(&kept.facts)
            .inspect_err(|error| {
                log::warn!("playback: the kept facts of {id} do not read: {error}")
            })
            .ok()?;
        let key = client::file_key(facts.key.as_deref())
            .inspect_err(|error| log::warn!("playback: the kept key of {id} is broken: {error:#}"))
            .ok()?;
        let stream = Stream::pulling(kept.source, Some(kept.length), Sealed::new(key.as_ref()))
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        log::debug!("playback: {id} started from the vault");
        Some(Loaded {
            stream,
            encoding: client::encoding(facts.codec.as_deref(), facts.bitrate),
        })
    }
}

#[async_trait]
impl Fetch for Yandex {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader<Sealed>>;

    fn name(&self) -> &'static str {
        "yandex"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Asks where the file is at the chosen level, or the best below it the account is
    /// allowed, and starts decrypting it as it arrives.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let served = YandexClient::served(self.quality);
        if let Some(loaded) = self.recall(id, served).await {
            return Ok(loaded);
        }
        let opened = self.client.open_stream(id, self.quality).await?;
        let key = client::file_key(opened.key.as_deref())?;
        let total = opened.response.content_length();
        let body = match &self.shelf {
            Some(shelf) => {
                let facts = Facts {
                    key: opened.key.clone(),
                    codec: opened.codec.clone(),
                    bitrate: opened.bitrate,
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
        let stream = Stream::pulling(body, total, Sealed::new(key.as_ref()))
            .primed()
            .await?;
        Ok(Loaded {
            stream,
            encoding: client::encoding(opened.codec.as_deref(), opened.bitrate),
        })
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        loaded.encoding
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
                log::warn!("playback: cannot decode the yandex track {id}: {error}");
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
                "playback: cannot start the yandex track {id} at {}s: {error}",
                at.as_secs()
            );
        }
        Some(decoder)
    }
}
