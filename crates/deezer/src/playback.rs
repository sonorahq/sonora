//! Playback for Deezer: what [`engine`] needs that is Deezer's own.
//!
//! One thing, really. The body arrives encrypted in 2048-byte stripes, so the stream decrypts
//! each one as it lands and what the buffer holds is always in the clear. From there rodio
//! decodes an ordinary file, and the threads, the queue, the preload and the gapless join are
//! the engine's.
//!
//! A seek past what has arrived of a FLAC or MP3 track does not wait for the download to get
//! there. A [`ranged::File`] starts a second download near the target, on a stripe boundary so
//! its body decrypts the stripes as the main one does.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use engine::{self, Fetch, Loudness};
use music::vault::{Recording, Shelf};
use music::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};
use reqwest::header::HeaderMap;
use rodio::Source as _;
use serde::{Deserialize, Serialize};
use sound::decode::{Decoded, Media};
use sound::ranged::{self, Address, Reach};

use crate::client::{DeezerClient, Opened};
use crate::decrypt::BLOCK;
use crate::{Stream, Striped};

/// A track downloading and decrypting, and how long and how loud Deezer says it is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    /// What a seek past the download enters the file with. `None` for a track from the vault,
    /// which is whole, and for a format a seek cannot enter part way.
    leap: Option<Arc<ranged::File<Striped>>>,
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
                media: ranged::media("deezer", || self.client.http().clone()),
                quality: config.quality,
                shelf: config.vault.as_ref().map(|vault| vault.shelf("deezer")),
            },
            config,
        )
    }
}

struct Deezer {
    client: DeezerClient,
    /// The client downloads a seek starts go through.
    media: reqwest::Client,
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

/// What a download asks Deezer for a fresh url of the stream it is fetching with.
struct Renewal {
    client: DeezerClient,
    id: String,
    /// The stream the download fetches, which a fresh url has to be for too.
    effective_id: String,
    /// The format the download fetches, which a fresh url is asked for in.
    format: &'static str,
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
            leap: None,
            duration: facts.duration_s.map(Duration::from_secs),
            loudness: facts.lufs.map(|lufs| Loudness { lufs, peak: None }),
        })
    }

    /// What a seek past the download of `opened` fetches it through. It has to be made on the
    /// runtime.
    fn reach(&self, id: &str, opened: &Opened) -> Reach<Striped> {
        let (media, url, key) = (self.media.clone(), opened.url.clone(), opened.key);
        let renewal: Arc<dyn ranged::Renew> = Arc::new(Renewal {
            client: self.client.clone(),
            id: id.to_owned(),
            effective_id: opened.effective_id.clone(),
            format: opened.format,
        });
        Reach::new("deezer", id, move |start| {
            let address = Address::new("deezer", media.clone(), url.clone());
            (address.renewed(renewal.clone()), Striped::at(&key, start))
        })
        .aligned(BLOCK as u64)
    }
}

#[async_trait]
impl ranged::Renew for Renewal {
    async fn url(&self) -> Result<(String, HeaderMap)> {
        let url = self
            .client
            .media_url(&self.id, &self.effective_id, self.format);
        Ok((url.await?, HeaderMap::new()))
    }
}

#[async_trait]
impl Fetch for Deezer {
    type Loaded = Loaded;

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
        let reach = self.reach(id, &opened);
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
            leap: ranged::File::detect(&stream, reach),
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

    /// Every seek opens a second decoder, so a seek past the download can open it over a
    /// download started at the target.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Decoded> {
        self.open_near(id, loaded, at).map(|(source, _)| source)
    }

    /// Builds a decoder placed at `at`. A file still arriving whose format allows it is entered
    /// part way, so a position past the download waits for one short stretch, never for the way
    /// there. An MP3 entered that way starts where its bitrate places it rather than at `at`.
    /// Otherwise, or when entering fails, the decoder opens seekable over the whole stream and is
    /// moved, which waits for the bytes at `at`. One that cannot be moved plays from the start.
    fn open_near(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<(Decoded, Duration)> {
        let entered = loaded
            .leap
            .as_ref()
            .zip(loaded.duration)
            .and_then(|(leap, length)| {
                leap.enter(&loaded.stream, length, at, |reader| {
                    ranged::forward("deezer", id, reader)
                })
            });
        let (mut decoder, start, seek) = match entered {
            Some((decoder, start)) => (decoder, start, false),
            None => {
                let mut builder = rodio::Decoder::builder()
                    .with_data(Media::new(loaded.stream.reader()))
                    .with_seekable(true);
                if let Some(total) = loaded.stream.total() {
                    builder = builder.with_byte_len(total);
                }
                let decoder = builder.build().inspect_err(|error| {
                    log::warn!("playback: cannot decode the deezer track {id}: {error}")
                });
                (decoder.ok()?, at, !at.is_zero())
            }
        };
        log::debug!(
            "playback: {id} decodes at {} Hz, {} channels, {:?} long",
            decoder.sample_rate(),
            decoder.channels(),
            decoder.total_duration()
        );
        if seek && let Err(error) = decoder.try_seek(at) {
            log::warn!(
                "playback: cannot start the deezer track {id} at {}s: {error}",
                at.as_secs()
            );
            return Some((Decoded::new(decoder), Duration::ZERO));
        }
        Some((Decoded::new(decoder), start))
    }
}
