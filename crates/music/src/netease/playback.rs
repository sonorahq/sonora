//! Playback for NetEase: what [`crate::engine`] needs that is NetEase's own.
//!
//! Very little. The site hands over an ordinary audio file behind a url that expires a while
//! after it is handed out, so the stream takes the bytes as they come and rodio decodes them.
//! The threads, the queue, the preload and the gapless join are the engine's.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::engine::{self, Fetch};
use crate::netease::client::NeteaseClient;
use crate::stream::{Plain, Reader, Stream};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// A track downloading.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
}

pub struct Factory {
    client: NeteaseClient,
}

impl Factory {
    pub fn new(client: NeteaseClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Netease {
                client: self.client.clone(),
            },
            config,
        )
    }
}

struct Netease {
    client: NeteaseClient,
}

#[async_trait]
impl Fetch for Netease {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader>;

    fn name(&self) -> &'static str {
        "netease"
    }

    /// Asks the site for the track's url and opens the download. A track the account's plan
    /// does not reach, or one only a trial clip is offered of, fails here, which the engine
    /// reports as unavailable and skips over.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let response = self.client.open_stream(id).await?;
        Ok(Loaded {
            stream: Stream::open(response, Plain).await?,
        })
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
                log::warn!("playback: cannot decode the netease track {id}: {error}");
                return None;
            }
        };
        // The output's own rate is fixed when the sink opens, and every source at another rate
        // is converted to it by discarding samples, so a track's rate is worth knowing: it is
        // the difference between hearing the bytes and hearing an approximation of them.
        log::debug!(
            "netease: {id} decodes as {} Hz, {} channels",
            rodio::Source::sample_rate(&decoder),
            rodio::Source::channels(&decoder),
        );
        if !at.is_zero()
            && let Err(error) = rodio::Source::try_seek(&mut decoder, at)
        {
            log::warn!(
                "playback: cannot start the netease track {id} at {}s: {error}",
                at.as_secs()
            );
        }
        Some(decoder)
    }
}
