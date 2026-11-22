//! Playback for a Subsonic server: what [`crate::engine`] needs that is Subsonic's own.
//!
//! Almost nothing, as it turns out. The server hands over an ordinary audio file, so the stream
//! takes the bytes as they come and rodio decodes them. The threads, the queue, the preload and
//! the gapless join are the engine's.
//!
//! A seek past what has arrived of an original FLAC, Ogg or MP3 file does not wait for the
//! download to get there. It starts a second download at the target through a
//! [`ranged::File`], as long as the server said how long the file is and that it answers ranges.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::engine::{self, Fetch, Loudness};
use crate::mp4::{Input, Prepared, Remote};
use crate::ranged::{self, Address, Reach};
use crate::stream::{Plain, Reader, Stream};
use crate::subsonic::client::{Details, SubsonicClient};
use crate::vault::{Recording, Shelf};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Transcode};

/// A track downloading, and what the server says about its length and loudness.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    details: Details,
    prepared: Prepared,
    /// What a seek past the download fetches the file through. Only an original file the server
    /// sends with a length and answers ranges for has one, and a kept track is whole without it.
    file: Option<Arc<ranged::File>>,
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
                media: ranged::media_builder()
                    .build()
                    .unwrap_or_else(|_| self.client.http.clone()),
                transcode: config.transcode,
                shelf: config.vault.as_ref().map(|vault| {
                    vault
                        .shelf("subsonic")
                        .within(self.client.identity())
                        .plain()
                }),
            },
            config,
        )
    }
}

struct Subsonic {
    client: SubsonicClient,
    /// What a seek's download is fetched with, which gives up on a stalled request sooner than
    /// the api client.
    media: reqwest::Client,
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
        let mut stream = Stream::pulling(kept.source, Some(kept.length), Plain)
            .primed()
            .await
            .inspect_err(|error| log::warn!("playback: the kept copy of {id} broke: {error:#}"))
            .ok()?;
        let prepared = Prepared::load(&mut stream, None).await.ok()?;
        log::debug!("playback: {id} started from the vault");
        let details = facts.map_or_else(Details::default, |facts| Details {
            duration: facts.duration_ms.map(Duration::from_millis),
            loudness: facts.lufs.map(|lufs| Loudness {
                lufs,
                peak: facts.peak,
            }),
            label: None,
        });
        Some(Loaded {
            stream,
            details,
            prepared,
            file: None,
        })
    }

    /// Whether the server is asked for the file as it is, rather than converted.
    fn original(&self) -> bool {
        self.transcode == Transcode::default()
    }

    /// What a seek past the download of `stream` fetches track `id` through, from `url`, when
    /// the file is in a format a seek can enter part way. It has to be made on the runtime.
    fn file(&self, id: &str, stream: &Stream, url: String) -> Option<Arc<ranged::File>> {
        let media = self.media.clone();
        let reach = Reach::new("subsonic", id, move |_| {
            (Address::new("subsonic", media.clone(), url.clone()), Plain)
        });
        let file = ranged::File::detect(stream, reach)?;
        log::debug!("subsonic: {id} can seek ahead as {:?}", file.kind());
        Some(Arc::new(file))
    }
}

#[async_trait]
impl Fetch for Subsonic {
    type Loaded = Loaded;
    type Source = rodio::Decoder<Input>;

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
                let remote = Remote::new(&response);
                let seekable = (self.original() && ranged::ranges(&response))
                    .then(|| response.url().to_string());
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
                let file = seekable.and_then(|url| self.file(id, &stream, url));
                anyhow::Ok((stream, prepared, file))
            },
            async {
                let details = self.client.details(id).await;
                told.set(details.clone()).ok();
                details
            },
        );
        let (stream, prepared, file) = stream?;
        Ok(Loaded {
            stream,
            details,
            prepared,
            file,
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

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        let total = loaded.stream.total().filter(|total| *total > 0)?;
        Some(loaded.stream.arrived() as f32 / total as f32)
    }

    /// Opens a track whose metadata was prepared on the cancellable load task.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let total = loaded.stream.total();
        let mut builder = rodio::Decoder::builder()
            .with_symphonia_decoder::<Aac>()
            .with_data(loaded.prepared.reader(&loaded.stream));
        if let Some(total) = total {
            builder = builder.with_byte_len(total);
        }
        let mut decoder = match builder
            .with_seekable(loaded.prepared.seekable(total, &loaded.stream))
            .build()
        {
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

    /// Opens the track at `at`, from a download started near `at` when the main one is far from
    /// it and the file allows that. Otherwise the decoder waits for the main download to get
    /// there, as [`open`](Fetch::open) does.
    fn open_near(
        &self,
        id: &str,
        loaded: &Loaded,
        at: Duration,
    ) -> Option<(Self::Source, Duration)> {
        if let Some(entered) = ahead(id, loaded, at) {
            return Some(entered);
        }
        self.open(id, loaded, at).map(|source| (source, at))
    }

    /// A decoder opened before a converted track finished arriving can only read forward, so
    /// every seek starts a fresh one that knows the length if the download has ended by then.
    fn reopen_to_seek(&self) -> bool {
        true
    }
}

/// Opens a decoder part way into a track still arriving and says where it starts. `None` when
/// the track is whole, starts at the top, has no known length or cannot be entered part way,
/// which leaves it to [`Fetch::open`].
fn ahead(id: &str, loaded: &Loaded, at: Duration) -> Option<(rodio::Decoder<Input>, Duration)> {
    let stream = &loaded.stream;
    if at.is_zero() || (stream.done() && stream.failed().is_none()) {
        return None;
    }
    let file = loaded.file.as_ref()?;
    let length = loaded.details.duration?;
    file.enter(stream, length, at, |reader| forward_decoder(id, reader))
}

/// A decoder that only reads forward and is not told the length, so opening it part way into
/// a file reads no further than the first frame.
fn forward_decoder(id: &str, reader: Reader) -> Option<rodio::Decoder<Input>> {
    let built = rodio::Decoder::builder()
        .with_symphonia_decoder::<Aac>()
        .with_data(Input::spliced(reader))
        .with_seekable(false)
        .build();
    built
        .inspect_err(|error| log::warn!("playback: cannot decode the subsonic track {id}: {error}"))
        .ok()
}
