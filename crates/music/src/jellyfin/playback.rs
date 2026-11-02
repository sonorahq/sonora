//! Playback for a Jellyfin server: what [`crate::engine`] needs that is Jellyfin's own.
//!
//! The server sends a track's original file, so the stream takes the bytes as they come and
//! rodio decodes them. The threads, the queue, the preload and the gapless join are the
//! engine's. Nothing is kept on disk, since the first version leaves downloads out.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::aac::Aac;
use crate::engine::{self, Fetch, Loudness};
use crate::jellyfin::client::{Details, JellyfinClient};
use crate::stream::{Plain, Reader, Stream};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// A track downloading, and what the server says about its length and loudness.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    details: Details,
}

pub struct Factory {
    client: JellyfinClient,
}

impl Factory {
    pub fn new(client: JellyfinClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Jellyfin {
                client: self.client.clone(),
            },
            config,
        )
    }
}

struct Jellyfin {
    client: JellyfinClient,
}

#[async_trait]
impl Fetch for Jellyfin {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader>;

    fn name(&self) -> &'static str {
        "jellyfin"
    }

    /// Opens the stream and asks for the length and loudness at the same time, so neither round
    /// trip waits on the other.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let (stream, details) = tokio::join!(
            async {
                let response = self.client.open_stream(id).await?;
                let total = response.content_length();
                Stream::pulling(response, total, Plain).primed().await
            },
            self.client.details(id),
        );
        Ok(Loaded {
            stream: stream?,
            details,
        })
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.details.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.details.loudness
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
        let total = loaded.stream.total();
        let mut builder = rodio::Decoder::builder()
            .with_symphonia_decoder::<Aac>()
            .with_data(loaded.stream.reader())
            .with_seekable(total.is_some());
        if let Some(total) = total {
            builder = builder.with_byte_len(total);
        }
        let mut decoder = match builder.build() {
            Ok(decoder) => decoder,
            Err(error) => {
                log::warn!("playback: cannot decode the jellyfin track {id}: {error}");
                return None;
            }
        };
        if !at.is_zero()
            && let Err(error) = rodio::Source::try_seek(&mut decoder, at)
        {
            log::warn!(
                "playback: cannot start the jellyfin track {id} at {}s: {error}",
                at.as_secs()
            );
        }
        Some(decoder)
    }
}
