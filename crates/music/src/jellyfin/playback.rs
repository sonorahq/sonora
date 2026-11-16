//! Playback for a Jellyfin server: what [`crate::engine`] needs that is Jellyfin's own.
//!
//! The server sends a track's original file, or converts it when the settings ask, so the
//! stream takes the bytes as they come and rodio decodes them. The threads, the queue, the
//! preload and the gapless join are the engine's. Played and downloaded tracks are kept in the
//! vault as plain files, so they play again with no network.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::engine::{self, Fetch, Loudness};
use crate::jellyfin::client::{Details, JellyfinClient};
use crate::mp4::{Input, Prepared, Remote};
use crate::stream::{Plain, Stream};
use crate::vault::{Recording, Shelf};
use crate::{Encoding, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Transcode};

/// How long playback lets the server take to say a track's length and loudness.
const DETAILS_PATIENCE: Duration = Duration::from_secs(5);

/// A track downloading, and what the server says about its length, loudness and encoding.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    details: Details,
    prepared: Prepared,
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
                transcode: config.transcode,
                shelf: config.vault.as_ref().map(|vault| {
                    vault
                        .shelf("jellyfin")
                        .within(self.client.identity())
                        .plain()
                }),
            },
            config,
        )
    }
}

struct Jellyfin {
    client: JellyfinClient,
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
    #[serde(default)]
    encoding: Option<Encoding>,
}

impl From<Details> for Facts {
    fn from(details: Details) -> Self {
        Self {
            duration_ms: details.duration.map(|duration| duration.as_millis() as u64),
            lufs: details.loudness.map(|loudness| loudness.lufs),
            peak: details.loudness.and_then(|loudness| loudness.peak),
            encoding: details.encoding,
        }
    }
}

impl Jellyfin {
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
        let mut stream = Stream::pulling(kept.source, Some(kept.length), Plain)
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        let prepared = Prepared::load(&mut stream, None).await.ok()?;
        log::debug!("playback: {id} started from the vault");
        // a copy kept before its facts arrived asks the server for them again
        let details = match facts {
            Some(facts) => Details {
                duration: facts.duration_ms.map(Duration::from_millis),
                loudness: facts.lufs.map(|lufs| Loudness {
                    lufs,
                    peak: facts.peak,
                }),
                encoding: facts.encoding,
                label: None,
            },
            None => self.details(id).await,
        };
        Some(Loaded {
            stream,
            details,
            prepared,
        })
    }

    /// The track's details from the server, or none once they take longer than
    /// [`DETAILS_PATIENCE`], since playback never waits on them.
    async fn details(&self, id: &str) -> Details {
        tokio::time::timeout(DETAILS_PATIENCE, self.client.details(id))
            .await
            .unwrap_or_default()
    }

    /// Whether the server sends the file as it is, rather than converting it.
    fn original(&self) -> bool {
        self.transcode == Transcode::default()
    }
}

#[async_trait]
impl Fetch for Jellyfin {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Input>;

    fn name(&self) -> &'static str {
        "jellyfin"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// Opens the stream and asks for the length and loudness at the same time, so neither round
    /// trip waits on the other. An MP4 file whose index sits at its end fetches that index by
    /// a range request before the rest arrives. A kept track asks the server nothing.
    async fn load(&self, id: &str) -> Result<Loaded> {
        if let Some(loaded) = self.recall(id).await {
            return Ok(loaded);
        }
        let told = Arc::new(OnceLock::new());
        let (stream, details) = tokio::join!(
            async {
                let response = self.client.open_stream(id, self.transcode).await?;
                let total = response.content_length();
                let remote = Remote::new(&response)
                    .map(|remote| remote.authorized(self.client.server(), &self.client.header));
                let body = match &self.shelf {
                    Some(shelf) => {
                        let told = told.clone();
                        let facts = move || {
                            let facts = told.get().cloned().map(Facts::from);
                            serde_json::to_vec(&facts).ok()
                        };
                        shelf.keep(id, &self.variant(), facts, response)
                    }
                    None => Recording::through(response),
                };
                let mut stream = Stream::pulling(body, total, Plain).primed().await?;
                let prepared = Prepared::load(
                    &mut stream,
                    remote.as_ref().map(|remote| (&self.client.http, remote)),
                )
                .await?;
                anyhow::Ok((stream, prepared))
            },
            async {
                let details = self.details(id).await;
                told.set(details.clone()).ok();
                details
            },
        );
        let (stream, prepared) = stream?;
        Ok(Loaded {
            stream,
            details,
            prepared,
        })
    }

    async fn label(&self, id: &str) -> Option<String> {
        self.client.details(id).await.label
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.details.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.details.loudness
    }

    /// The file's own encoding. A converted stream is whatever the conversion made, which the
    /// server does not describe, so it reports none.
    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        self.original().then_some(loaded.details.encoding).flatten()
    }

    fn gated(&self, error: &anyhow::Error) -> bool {
        error.downcast_ref::<crate::SignInFailure>().is_some()
    }

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        let total = loaded.stream.total().filter(|total| *total > 0)?;
        Some(loaded.stream.arrived() as f32 / total as f32)
    }

    /// Opens a decoder over a stream whose metadata was prepared on the cancellable load task,
    /// and places it at `at`.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let total = loaded.stream.total();
        let mut builder = rodio::Decoder::builder()
            .with_symphonia_decoder::<Aac>()
            .with_data(loaded.prepared.reader(&loaded.stream));
        if let Some(total) = total {
            builder = builder.with_byte_len(total);
        }
        let mut decoder = match builder
            .with_seekable(loaded.prepared.seekable(&loaded.stream))
            .build()
        {
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

    /// A converted stream arrives with no length, and a decoder opened before it finished can
    /// only read forward, so every seek starts a fresh one. The original file seeks in place.
    fn reopen_to_seek(&self) -> bool {
        !self.original()
    }
}
