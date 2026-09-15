//! Playback for a Subsonic server: what [`crate::engine`] needs that is Subsonic's own.
//!
//! Almost nothing, as it turns out. The server hands over an ordinary audio file, so the stream
//! takes the bytes as they come and rodio decodes them. The threads, the queue, the preload and
//! the gapless join are the engine's.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::engine::{self, Fetch};
use crate::stream::{Plain, Reader, Stream};
use crate::subsonic::client::SubsonicClient;
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// A track downloading, and how long the server says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    duration: Option<Duration>,
}

pub struct Factory {
    client: SubsonicClient,
}

impl Factory {
    pub fn new(client: SubsonicClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Subsonic {
                client: self.client.clone(),
            },
            config,
        )
    }
}

struct Subsonic {
    client: SubsonicClient,
}

#[async_trait]
impl Fetch for Subsonic {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader>;

    fn name(&self) -> &'static str {
        "subsonic"
    }

    /// Opens the stream and asks for the length at the same time, so neither round trip waits
    /// on the other; the preroll usually covers the length lookup entirely.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let (stream, duration) = tokio::join!(
            async { Stream::open(self.client.open_stream(id).await?, Plain).await },
            self.client.duration(id),
        );
        Ok(Loaded {
            stream: stream?,
            duration,
        })
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
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
                log::warn!("playback: cannot decode the subsonic track {id}: {error}");
                return None;
            }
        };
        if !at.is_zero()
            && let Err(error) = rodio::Source::try_seek(&mut decoder, at)
        {
            log::warn!(
                "playback: cannot start the subsonic track {id} at {}s: {error}",
                at.as_secs()
            );
        }
        Some(decoder)
    }
}
