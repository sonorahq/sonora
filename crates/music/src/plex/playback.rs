//! Playback for a Plex Media Server: what [`crate::engine`] needs that is Plex's own.
//!
//! The server hands over the track's file as it is, so the stream takes the bytes as they come
//! and rodio decodes them. The threads, the queue, the preload and the gapless join are the
//! engine's.

use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::engine::{self, Fetch, Loudness};
use crate::plex::client::{Details, PlexClient};
use crate::stream::{Plain, Reader, Stream};
use crate::vault::{Recording, Shelf};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// A track downloading, and what the server says about its length and loudness.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    details: Details,
}

pub struct Factory {
    client: PlexClient,
}

impl Factory {
    pub fn new(client: PlexClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Plex {
                client: self.client.clone(),
                shelf: config
                    .vault
                    .as_ref()
                    .map(|vault| vault.shelf("plex").within(self.client.identity())),
            },
            config,
        )
    }
}

struct Plex {
    client: PlexClient,
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

impl From<&Details> for Facts {
    fn from(details: &Details) -> Self {
        Self {
            duration_ms: details.duration.map(|duration| duration.as_millis() as u64),
            lufs: details.loudness.map(|loudness| loudness.lufs),
            peak: details.loudness.and_then(|loudness| loudness.peak),
        }
    }
}

impl Plex {
    /// The track from the vault, when it was kept from this server.
    async fn recall(&self, id: &str) -> Option<Loaded> {
        let kept = self.shelf.as_ref()?.find(id, "file").await?;
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
            part: None,
        });
        Some(Loaded { stream, details })
    }
}

#[async_trait]
impl Fetch for Plex {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Reader>;

    fn name(&self) -> &'static str {
        "plex"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Asks the server where the file is and how loud it is, then opens it. A kept track asks
    /// the server nothing.
    async fn load(&self, id: &str) -> Result<Loaded> {
        if let Some(loaded) = self.recall(id).await {
            return Ok(loaded);
        }
        let details = self.client.details(id).await?;
        let part = details
            .part
            .as_deref()
            .context("plex has no file for the track")?;
        let response = self.client.open_stream(part).await?;
        let total = response.content_length();
        let body = match &self.shelf {
            Some(shelf) => {
                let facts = Facts::from(&details);
                let facts = move || serde_json::to_vec(&Some(facts)).ok();
                shelf.keep(id, "file", facts, response)
            }
            None => Recording::through(response),
        };
        let stream = Stream::pulling(body, total, Plain).primed().await?;
        Ok(Loaded { stream, details })
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
                log::warn!("playback: cannot decode the plex track {id}: {error}");
                return None;
            }
        };
        if !at.is_zero()
            && let Err(error) = rodio::Source::try_seek(&mut decoder, at)
        {
            log::warn!(
                "playback: cannot start the plex track {id} at {}s: {error}",
                at.as_secs()
            );
        }
        Some(decoder)
    }
}
