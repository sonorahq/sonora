//! Playback for a Subsonic server: what [`crate::engine`] needs that is Subsonic's own.
//!
//! Almost nothing, as it turns out. The server hands over an ordinary audio file, so the stream
//! takes the bytes as they come and rodio decodes them. The threads, the queue, the preload and
//! the gapless join are the engine's.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::aac::Aac;
use crate::engine::{self, Fetch, Loudness};
use crate::mp4::{Input, Prepared, Remote};
use crate::stream::{Plain, Source, Stream};
use crate::subsonic::client::{Details, SubsonicClient};
use crate::vault::{Recording, Shelf};
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, STATION_PREFIX, Transcode};

/// How much of a station's stream has to arrive before it plays. A station sends audio about
/// as fast as it is heard, so the preroll of a whole track would hold it silent for seconds.
const STATION_PREROLL: usize = 32 * 1024;
/// The most of a `.pls` or `.m3u` file that is read for the stream it names. Such a file is a
/// few lines long.
const PLAYLIST_LIMIT: usize = 64 * 1024;

/// A track downloading, and what the server says about its length and loudness.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    details: Details,
    prepared: Prepared,
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
        })
    }

    /// Opens an internet radio station. A station that is listed by a `.pls` or `.m3u` file
    /// plays the first stream the file names. Nothing of it is kept in the vault, and it has
    /// no length, so it plays until the broadcast or the connection ends.
    async fn tune(&self, station: &str) -> Result<Loaded> {
        let (_, address) = self.client.station(station).await?;
        let mut response = self.client.open_station(&address).await?;
        if lists_streams(&response) {
            let listing = response.url().clone();
            let mut listed = Vec::new();
            while listed.len() < PLAYLIST_LIMIT
                && let Some(chunk) = Source::chunk(&mut response)
                    .await
                    .context("cannot read the radio station's playlist")?
            {
                listed.extend_from_slice(&chunk);
            }
            let address = first_stream(&String::from_utf8_lossy(&listed), &listing)
                .context("the radio station's playlist names no stream")?;
            response = self.client.open_station(address.as_str()).await?;
        }
        let mut stream = match carries_adts(&response) {
            true => Stream::pulling(Mpeg4Adts::new(response), None, Plain),
            false => Stream::pulling(response, None, Plain),
        };
        stream.wait_for(STATION_PREROLL).await?;
        log::debug!("playback: tuned in to the station {station}");
        Ok(Loaded {
            stream,
            details: Details::default(),
            prepared: Prepared::default(),
        })
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
        if let Some(station) = id.strip_prefix(STATION_PREFIX) {
            return self.tune(station).await;
        }
        if let Some(loaded) = self.recall(id).await {
            return Ok(loaded);
        }
        let told = Arc::new(OnceLock::new());
        let (stream, details) = tokio::join!(
            async {
                let response = self.client.open_stream(id, self.transcode).await?;
                let total = response.content_length();
                let remote = Remote::new(&response);
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
                let details = self.client.details(id).await;
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
        // A station always starts live. Seeking into one would read forward through audio not
        // yet broadcast, and hold the engine until it was.
        if !at.is_zero()
            && !id.starts_with(STATION_PREFIX)
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

/// A station's AAC stream with every ADTS frame header marked MPEG-4. MPEG-2 and MPEG-4 ADTS
/// carry the same AAC frames, but symphonia only recognises the MPEG-4 sync word, and probes an
/// MPEG-2 stream as noise that happens to look like MP3.
struct Mpeg4Adts {
    response: reqwest::Response,
    /// What arrived after the last whole header, kept until enough follows to read the next.
    held: Vec<u8>,
    /// How much of the current frame is still to pass through untouched.
    frame: usize,
}

impl Mpeg4Adts {
    /// The shortest ADTS header, which holds the frame length.
    const HEADER: usize = 7;

    fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            held: Vec::new(),
            frame: 0,
        }
    }

    /// Relabels the headers in `bytes`, which follow what is held, and holds back a header cut
    /// off at the end. Bytes before the first header pass through, as a stream joined part way
    /// through a frame starts with them.
    fn relabel(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut data = std::mem::take(&mut self.held);
        data.extend_from_slice(bytes);
        let mut out = Vec::with_capacity(data.len());
        let mut at = 0;
        while at < data.len() {
            if self.frame > 0 {
                let taken = self.frame.min(data.len() - at);
                out.extend_from_slice(&data[at..at + taken]);
                self.frame -= taken;
                at += taken;
                continue;
            }
            if data.len() - at < Self::HEADER {
                break;
            }
            let header = &data[at..at + Self::HEADER];
            let length = (usize::from(header[3] & 0x03) << 11)
                | (usize::from(header[4]) << 3)
                | usize::from(header[5] >> 5);
            if header[0] != 0xff || header[1] & 0xf6 != 0xf0 || length < Self::HEADER {
                out.push(data[at]);
                at += 1;
                continue;
            }
            out.extend_from_slice(&[0xff, header[1] & !0x08]);
            self.frame = length - 2;
            at += 2;
        }
        self.held = data[at..].to_vec();
        out
    }
}

impl Source for Mpeg4Adts {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        loop {
            let Some(chunk) = Source::chunk(&mut self.response).await? else {
                let rest = std::mem::take(&mut self.held);
                return Ok((!rest.is_empty()).then(|| Bytes::from(rest)));
            };
            let relabelled = self.relabel(&chunk);
            if !relabelled.is_empty() {
                return Ok(Some(Bytes::from(relabelled)));
            }
        }
    }
}

/// Whether a station streams raw AAC in ADTS frames, by the media type it labels the audio with.
fn carries_adts(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|kind| kind.to_str().ok())
        .is_some_and(|kind| {
            let kind = kind.to_ascii_lowercase();
            ["audio/aac", "audio/aacp", "audio/x-aac"]
                .iter()
                .any(|adts| kind.starts_with(adts))
        })
}

/// Whether a station's address answered with a `.pls` or `.m3u` file that names its stream,
/// rather than with the audio. A server that labels the file as something other than audio is
/// judged by the file's extension.
fn lists_streams(response: &reqwest::Response) -> bool {
    let kind = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|kind| kind.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ["audio/x-scpls", "audio/x-mpegurl", "audio/mpegurl"]
        .iter()
        .any(|listing| kind.starts_with(listing))
    {
        return true;
    }
    let path = response.url().path().to_ascii_lowercase();
    !kind.starts_with("audio/") && (path.ends_with(".pls") || path.ends_with(".m3u"))
}

/// The first stream a `.pls` or `.m3u` file names, with an entry relative to `listing`, the
/// address the file came from, resolved against it.
fn first_stream(listed: &str, listing: &reqwest::Url) -> Option<reqwest::Url> {
    let listed = listed.trim_start_matches('\u{feff}');
    let pls = listed
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("[playlist]");
    listed
        .lines()
        .map(str::trim)
        .filter_map(|line| match pls {
            true => line
                .split_once('=')
                .filter(|(key, _)| key.trim().to_ascii_lowercase().starts_with("file"))
                .map(|(_, entry)| entry.trim()),
            false => (!line.starts_with('#')).then_some(line),
        })
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| listing.join(entry).ok())
        .find(|address| matches!(address.scheme(), "http" | "https"))
}
