//! Playback for a Subsonic server: what [`crate::engine`] needs that is Subsonic's own.
//!
//! Almost nothing, as it turns out. The server hands over an ordinary audio file, so the stream
//! takes the bytes as they come and rodio decodes them. The threads, the queue, the preload and
//! the gapless join are the engine's.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::engine::{self, Fetch, Loudness};
use crate::stream::{Plain, Reader, Stream};
use crate::subsonic::client::{Details, SubsonicClient};
use crate::vault::{Recording, Shelf};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Transcode};

/// A track downloading, and what the server says about its length and loudness.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    details: Details,
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
                transcode: config.transcode,
                shelf: config
                    .vault
                    .as_ref()
                    .map(|vault| vault.shelf("subsonic").within(self.client.identity())),
            },
            config,
        )
    }
}

struct Subsonic {
    client: SubsonicClient,
    transcode: Transcode,
    /// Where played tracks are kept, so playing one again needs nothing from the server.
    shelf: Option<Shelf>,
}

/// What the server said about a track, kept beside it in the vault.
#[derive(Serialize, Deserialize)]
struct Facts {
    duration_ms: Option<u64>,
    lufs: Option<f32>,
    peak: Option<f32>,
}

impl From<Details> for Facts {
    fn from(details: Details) -> Self {
        Self {
            duration_ms: details.duration.map(|duration| duration.as_millis() as u64),
            lufs: details.loudness.map(|loudness| loudness.lufs),
            peak: details.loudness.and_then(|loudness| loudness.peak),
        }
    }
}

impl Subsonic {
    /// What tells two kept copies of one track id apart: the server it came from and the
    /// conversion it was asked for.
    fn variant(&self) -> String {
        format!("{}|{:?}", self.client.identity(), self.transcode)
    }

    /// The track from the vault, when it was kept from this server in this format.
    async fn recall(&self, id: &str) -> Option<Loaded> {
        let kept = self.shelf.as_ref()?.find(id, &self.variant()).await?;
        let facts = serde_json::from_slice::<Option<Facts>>(&kept.facts)
            .inspect_err(|error| {
                log::warn!("playback: the kept facts of {id} do not read: {error}")
            })
            .ok()?;
        let stream = Stream::pulling(kept.source, Some(kept.length), Plain)
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        log::debug!("playback: {id} started from the vault");
        let details = facts.map_or_else(Details::default, |facts| Details {
            duration: facts.duration_ms.map(Duration::from_millis),
            loudness: facts.lufs.map(|lufs| Loudness {
                lufs,
                peak: facts.peak,
            }),
        });
        Some(Loaded { stream, details })
    }
}

#[async_trait]
impl Fetch for Subsonic {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader>;

    fn name(&self) -> &'static str {
        "subsonic"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Opens the stream and asks for the length and loudness at the same time, so neither round
    /// trip waits on the other. The preroll usually covers the lookup entirely. A kept track
    /// asks the server nothing.
    async fn load(&self, id: &str) -> Result<Loaded> {
        if let Some(loaded) = self.recall(id).await {
            return Ok(loaded);
        }
        let told = Arc::new(OnceLock::new());
        let (stream, details) = tokio::join!(
            async {
                let response = self.client.open_stream(id, self.transcode).await?;
                let total = response.content_length();
                let body = match &self.shelf {
                    Some(shelf) => {
                        let told = told.clone();
                        let facts = move || {
                            let facts = told.get().copied().map(Facts::from);
                            serde_json::to_vec(&facts).ok()
                        };
                        shelf.keep(id, &self.variant(), facts, response)
                    }
                    None => Recording::through(response),
                };
                Stream::pulling(body, total, Plain).primed().await
            },
            async {
                let details = self.client.details(id).await;
                told.set(details).ok();
                details
            },
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
    ///
    /// A converted track has no length until all of it has arrived, and Symphonia's Ogg reader
    /// panics when it seeks a seekable source of unknown length. Until then the decoder only
    /// reads forward, which reaches any position from the start.
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

    /// A decoder opened before a converted track finished arriving can only read forward, so
    /// every seek starts a fresh one that knows the length if the download has ended by then.
    fn reopen_to_seek(&self) -> bool {
        true
    }
}
