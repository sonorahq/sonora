//! Spotify tracks downloaded for offline play, kept the way Spotify's own clients keep theirs:
//! the file as the CDN sent it, still encrypted, with its key and metadata beside it. The vault
//! seals all of that again under the vault's master key, so a download plays outside Sonora
//! only for someone who holds that key, and for them it is as easy to play as any file.

use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use super::access_point::KeyDenied;
use crate::spotify::proto::audio::StorageResolveResponse;
use crate::spotify::proto::metadata::ExtensionKind;
use crate::spotify::proto::metadata::Track as TrackMessage;
use crate::spotify::{collection, session::Session, transport, wire};
use aes::cipher::StreamCipherSeek as _;
use aes::cipher::{KeyIvInit as _, StreamCipher as _};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http::HeaderMap;
use prost::Message as _;
use reqwest::{Response, StatusCode, Url, header};
use rodio::Source as _;
use serde::{Deserialize, Serialize};

use crate::Quality;
use crate::engine::{self, Fetch, Loudness};
use crate::stream::{Plain, Source, Stream};
use crate::vault::{Recording, Shelf};
use crate::{Encoding, Fraction, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

const VORBIS_HEADER_LEN: u64 = 0xa7;
const VORBIS_GAIN_OFFSET: u64 = 144;
const MEDIA_COUNTER: [u8; 16] = [
    0x72, 0xe0, 0x67, 0xfb, 0xdd, 0xcb, 0xcf, 0x77, 0xeb, 0xe8, 0xbc, 0x64, 0x3f, 0x63, 0x0d, 0x93,
];

const FIRST_RANGE: u64 = 64 * 1024;
const MAX_RANGE: u64 = 2 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

type Cipher = ctr::Ctr128BE<aes::Aes128>;

/// The existing vault format for an encrypted Spotify file and its playback facts.
#[derive(Serialize, Deserialize)]
struct Facts {
    /// The track's `TRACK_V4` metadata, in base64.
    metadata: String,
    /// The file's format, as Spotify's protocol numbers it.
    format: i32,
    file_id: String,
    key: Option<[u8; 16]>,
}

/// Uses Sonora's audio engine for saved tracks when the account is offline.
pub(in crate::spotify) struct OfflineFactory;

impl PlaybackFactory for OfflineFactory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        let shelf = config.vault.as_ref().map(|vault| vault.shelf("spotify"));
        engine::start(
            Audio {
                shelf,
                session: None,
                quality: config.quality,
            },
            config,
        )
    }
}

/// Supplies encrypted vault entries or a progressive CDN stream to the shared decoder.
pub(super) struct Audio {
    pub(super) shelf: Option<Shelf>,
    pub(super) session: Option<Session>,
    pub(super) quality: Quality,
}

/// The metadata needed by the shared player before it opens a decoder.
#[derive(Clone)]
pub(super) struct Loaded {
    duration: Option<Duration>,
    loudness: Option<Loudness>,
    encoding: Option<Encoding>,
    arriving: Option<Stream<Plain>>,
    key: Option<[u8; 16]>,
    format: i32,
}

/// Opens the saved media with its stored key and presents only the audio to the decoder.
pub(super) struct Media {
    file: Box<dyn AudioReader>,
    total: Option<u64>,
    cipher: Option<Cipher>,
    header: u64,
    position: u64,
}

trait AudioReader: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> AudioReader for T {}

fn read_saved(shelf: Option<&Shelf>, id: &str) -> Result<(Facts, Media)> {
    let shelf = shelf.context("Spotify download vault is unavailable")?;
    let (facts, file) = shelf
        .open_pinned(id)
        .context("Spotify track is not downloaded")?;
    let facts: Facts =
        serde_json::from_slice(&facts).context("cannot read Spotify download metadata")?;
    let length = file.length();
    let media = Media::new(file, Some(length), facts.key, facts.format)?;
    Ok((facts, media))
}

#[async_trait]
impl Fetch for Audio {
    type Loaded = Loaded;
    type Source = rodio::Decoder<BufReader<Media>>;

    fn name(&self) -> &'static str {
        "spotify"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    async fn load(&self, id: &str) -> Result<Loaded> {
        if let Some(shelf) = &self.shelf {
            shelf.settle().await;
        }
        let shelf = self.shelf.clone();
        let saved_id = id.to_owned();
        let saved = tokio::task::spawn_blocking(move || {
            read_saved(shelf.as_ref(), &saved_id)
                .map(|(facts, mut media)| Loaded::from_facts(&facts, media.loudness()))
        })
        .await
        .context("cannot open Spotify download")?;
        if saved.is_ok() || self.session.is_none() {
            return saved;
        }

        let session = self
            .session
            .as_ref()
            .context("Spotify playback requires sign-in")?;
        let variant = match self.quality {
            Quality::Low => "vorbis-96",
            Quality::Normal => "vorbis-160",
            _ => "vorbis-320",
        };
        if let Some(shelf) = &self.shelf
            && let Some(kept) = shelf.find(id, variant).await
            && let Ok(facts) = serde_json::from_slice::<Facts>(&kept.facts)
        {
            return Loaded::streaming(&facts, kept.source, Some(kept.length)).await;
        }
        let mut media = resolve_media(session, id, self.quality).await?;
        let response = open_file(session, &media.facts.file_id).await?;
        media.fetch_key(session).await?;
        crate::soloist::granted();
        let facts = media.facts;
        let total = Some(response.length());
        let source = match &self.shelf {
            Some(shelf) => {
                let facts = serde_json::to_vec(&facts)?;
                shelf.keep(id, variant, move || Some(facts), response)
            }
            None => Recording::through(response),
        };
        Loaded::streaming(&facts, source, total).await
    }

    async fn downloaded(&self, loaded: &Loaded) {
        if let Some(arriving) = &loaded.arriving {
            arriving.finished().await;
        }
    }

    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        let arriving = loaded.arriving.as_ref()?;
        let total = arriving.total().filter(|total| *total > 0)?;
        Some((arriving.arrived() as f32 / total as f32).min(1.0))
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        loaded.encoding
    }

    fn reopen_to_seek(&self) -> bool {
        // An incomplete Ogg stream can scan forward from the start but cannot seek backward.
        true
    }

    fn refused(&self, error: &anyhow::Error) -> bool {
        if error.is::<KeyDenied>() {
            crate::soloist::denied();
            return true;
        }
        false
    }

    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let mut media = match &loaded.arriving {
            Some(stream) => {
                Media::new(stream.reader(), stream.total(), loaded.key, loaded.format).ok()?
            }
            None => read_saved(self.shelf.as_ref(), id).ok()?.1,
        };
        let len = media.length().filter(|_| {
            loaded.arriving.as_ref().is_none_or(|stream| {
                stream
                    .total()
                    .is_some_and(|total| stream.arrived() as u64 >= total)
            })
        });
        media.seek(SeekFrom::Start(0)).ok()?;
        let mut builder = rodio::Decoder::builder()
            .with_symphonia_decoder::<crate::aac::Aac>()
            .with_data(BufReader::new(media))
            .with_seekable(len.is_some())
            .with_gapless(true);
        if let Some(len) = len {
            builder = builder.with_byte_len(len);
        }
        let mut decoder = builder
            .build()
            .inspect_err(|error| log::warn!("spotify: cannot decode saved track {id}: {error}"))
            .ok()?;
        if !at.is_zero() {
            decoder.try_seek(at).ok()?;
        }
        Some(decoder)
    }
}

impl Loaded {
    /// Primes either the cached or CDN stream before opening a decoder.
    async fn streaming(facts: &Facts, source: impl Source, total: Option<u64>) -> Result<Self> {
        let stream = Stream::pulling(source, total, Plain).primed().await?;
        let mut media = Media::new(stream.reader(), total, facts.key, facts.format)?;
        let mut loaded = Self::from_facts(facts, media.loudness());
        loaded.arriving = Some(stream);
        Ok(loaded)
    }

    fn from_facts(facts: &Facts, loudness: Option<Loudness>) -> Self {
        let duration = STANDARD
            .decode(&facts.metadata)
            .ok()
            .and_then(|raw| TrackMessage::decode(raw.as_slice()).ok())
            .and_then(|track| track.duration_ms)
            .and_then(|ms| u64::try_from(ms).ok())
            .map(Duration::from_millis);
        let encoding = match facts.format {
            0 => Some(Encoding::Vorbis { kbps: 96 }),
            1 => Some(Encoding::Vorbis { kbps: 160 }),
            2 => Some(Encoding::Vorbis { kbps: 320 }),
            _ => None,
        };
        Self {
            duration,
            loudness,
            encoding,
            arriving: None,
            key: facts.key,
            format: facts.format,
        }
    }
}

impl Media {
    fn new(
        file: impl AudioReader + 'static,
        total: Option<u64>,
        key: Option<[u8; 16]>,
        format: i32,
    ) -> Result<Self> {
        let header = if matches!(format, 0..=2) {
            VORBIS_HEADER_LEN
        } else {
            0
        };
        anyhow::ensure!(
            total.is_none_or(|total| total > header),
            "Spotify track has no audio"
        );
        let cipher = key.map(|key| Cipher::new(&key.into(), &MEDIA_COUNTER.into()));
        Ok(Self {
            file: Box::new(file),
            total,
            cipher,
            header,
            position: 0,
        })
    }

    fn length(&self) -> Option<u64> {
        self.total.map(|length| length - self.header)
    }

    fn seek_raw(&mut self, position: u64) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(position))?;
        if let Some(cipher) = &mut self.cipher {
            cipher.seek(position);
        }
        Ok(())
    }

    fn read_raw(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read(buffer)?;
        if let Some(cipher) = &mut self.cipher {
            cipher.apply_keystream(&mut buffer[..count]);
        }
        Ok(count)
    }

    fn loudness(&mut self) -> Option<Loudness> {
        if self.header == 0 {
            return None;
        }
        self.seek_raw(VORBIS_GAIN_OFFSET).ok()?;
        let mut values = [0; 8];
        let mut read = 0;
        while read < values.len() {
            let count = self.read_raw(&mut values[read..]).ok()?;
            if count == 0 {
                return None;
            }
            read += count;
        }
        let gain = f32::from_le_bytes(values[..4].try_into().ok()?);
        let peak = f32::from_le_bytes(values[4..].try_into().ok()?);
        gain.is_finite()
            .then(|| Loudness::replay_gain(gain, peak.is_finite().then_some(peak)))
    }
}

impl Read for Media {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.read_raw(buffer)?;
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for Media {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let position = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::Current(by) => self.position.checked_add_signed(by),
            SeekFrom::End(by) => self.length().and_then(|len| len.checked_add_signed(by)),
        };
        let invalid =
            || io::Error::new(io::ErrorKind::InvalidInput, "invalid Spotify download seek");
        let position = position.ok_or_else(invalid)?;
        self.seek_raw(position.checked_add(self.header).ok_or_else(invalid)?)?;
        self.position = position;
        Ok(position)
    }
}

/// Fetches Spotify media directly and seals it in the existing download format.
pub(super) async fn download(
    session: &Session,
    shelf: &Shelf,
    id: &str,
    quality: Quality,
    progress: Fraction,
) -> Result<()> {
    shelf.settle().await;
    if shelf.holds(id) {
        progress.set(1.0);
        return Ok(());
    }

    let mut media = resolve_media(session, id, quality).await?;
    if session.audio_keys().throttled()
        && let Err(error) = media.fetch_key(session).await
        && (error.is::<KeyDenied>() || error.is::<crate::Throttled>())
    {
        return Err(error);
    }
    let mut response = open_file(session, &media.facts.file_id).await?;
    let total = response.length();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        bytes.extend_from_slice(&chunk);
        progress.set((bytes.len() as f32 / total as f32).min(0.99));
    }
    anyhow::ensure!(
        bytes.len() > VORBIS_HEADER_LEN as usize && bytes.len() as u64 == total,
        "Spotify download is incomplete"
    );
    if media.facts.key.is_none() {
        media.fetch_key(session).await?;
    }
    let facts = serde_json::to_vec(&media.facts)?;
    let shelf = shelf.clone();
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || shelf.pin(&id, facts, &bytes))
        .await
        .context("the download stopped")??;
    progress.set(1.0);
    Ok(())
}

/// The selected file and the identity needed to request its authorized audio key.
struct ResolvedMedia {
    track: [u8; 16],
    file: [u8; 20],
    facts: Facts,
}

impl ResolvedMedia {
    async fn fetch_key(&mut self, session: &Session) -> Result<()> {
        self.facts.key = Some(
            session
                .audio_keys()
                .get(session, self.track, self.file)
                .await?,
        );
        Ok(())
    }
}

/// Resolves the track and its playable file for playback or saving.
async fn resolve_media(session: &Session, id: &str, quality: Quality) -> Result<ResolvedMedia> {
    let requested = format!("spotify:track:{id}");
    let (track, wire) = metadata(session, &[requested])
        .await?
        .into_iter()
        .next()
        .context("Spotify omitted the track for download")?;
    let (playable, wire) = if vorbis(&track, quality).is_some() {
        (track, wire)
    } else {
        let alternatives: Vec<_> = track
            .alternative
            .iter()
            .filter_map(|track| collection::base62(track.gid()))
            .map(|id| format!("spotify:track:{id}"))
            .collect();
        metadata(session, &alternatives)
            .await?
            .into_iter()
            .find(|(track, _)| vorbis(track, quality).is_some())
            .context("Spotify has no playable download for this track")?
    };
    let (format, file) = vorbis(&playable, quality).context("Spotify omitted the audio file")?;
    let track = <[u8; 16]>::try_from(playable.gid()).context("invalid Spotify audio track ID")?;
    let mut file_id = String::with_capacity(file.len() * 2);
    wire::append_hex(&mut file_id, &file);
    Ok(ResolvedMedia {
        track,
        file,
        facts: Facts {
            metadata: STANDARD.encode(wire),
            format,
            file_id,
            key: None,
        },
    })
}

/// Resolves a signed CDN URL and opens its initial byte range.
async fn open_file(session: &Session, file_id: &str) -> Result<Cdn> {
    let endpoint = format!("/storage-resolve/files/audio/interactive/{file_id}");
    let resolved: StorageResolveResponse =
        transport::get(session, &endpoint, HeaderMap::new()).await?;
    anyhow::ensure!(resolved.result == 0, "Spotify did not grant a CDN download");
    Cdn::open(resolved.cdnurl).await
}

/// Batches track metadata for the requested track and, when needed, its market alternatives.
async fn metadata(session: &Session, uris: &[String]) -> Result<Vec<(TrackMessage, Bytes)>> {
    let kind = ExtensionKind::TrackV4;
    let requests: Vec<_> = uris
        .iter()
        .map(|uri| (uri.as_str(), std::slice::from_ref(&kind)))
        .collect();
    let mut known = collection::extended_batch(session, &requests).await?;
    Ok(uris
        .iter()
        .filter_map(|uri| {
            let raw = known.remove(&(kind, uri.clone()))?;
            Some((TrackMessage::decode(raw.clone()).ok()?, raw))
        })
        .collect())
}

/// Picks the requested Vorbis bitrate first, then looks for a supported fallback.
fn vorbis(track: &TrackMessage, quality: Quality) -> Option<(i32, [u8; 20])> {
    let choices = match quality {
        Quality::Normal => [1, 0, 2],
        Quality::Low => [0, 1, 2],
        _ => [2, 1, 0],
    };
    choices.into_iter().find_map(|format| {
        track
            .file
            .iter()
            .filter(|file| file.format == Some(format))
            .find_map(|file| Some((format, file.file_id().try_into().ok()?)))
    })
}

/// Delivers verified CDN ranges in file order to the shared playback spool and download vault.
struct Cdn {
    url: Url,
    response: Response,
    length: u64,
    at: u64,
    range_start: u64,
    range_end: u64,
    range_started: Instant,
}

impl Cdn {
    async fn open(addresses: Vec<String>) -> Result<Self> {
        for address in addresses {
            let Ok(url) = Url::parse(&address) else {
                continue;
            };
            if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
                continue;
            }
            let range_started = Instant::now();
            if let Ok((response, range_end, length)) = cdn_range(&url, 0, FIRST_RANGE).await {
                anyhow::ensure!(length <= MAX_DOWNLOAD, "Spotify track is too large");
                return Ok(Self {
                    url,
                    response,
                    length,
                    at: 0,
                    range_start: 0,
                    range_end,
                    range_started,
                });
            }
        }
        anyhow::bail!("cannot open Spotify audio on its CDN")
    }

    fn length(&self) -> u64 {
        self.length
    }
}

impl Source for Cdn {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        loop {
            if let Some(bytes) = self
                .response
                .chunk()
                .await
                .map_err(reqwest::Error::without_url)?
            {
                anyhow::ensure!(
                    bytes.len() as u64 <= self.range_end - self.at,
                    "Spotify CDN sent bytes past the requested range"
                );
                self.at += bytes.len() as u64;
                return Ok(Some(bytes));
            }
            anyhow::ensure!(
                self.at == self.range_end,
                "Spotify CDN range was incomplete"
            );
            if self.at == self.length {
                return Ok(None);
            }
            // Aim for about one second of measured transfer, with a bounded request size.
            let elapsed = self.range_started.elapsed().as_millis().max(1);
            let rate = u128::from(self.range_end - self.range_start) * 1000 / elapsed;
            let length = rate.clamp(u128::from(FIRST_RANGE), u128::from(MAX_RANGE)) as u64;
            self.range_started = Instant::now();
            let (response, end, total) =
                cdn_range(&self.url, self.at, length.min(self.length - self.at)).await?;
            anyhow::ensure!(total == self.length, "Spotify CDN file length changed");
            self.response = response;
            self.range_start = self.at;
            self.range_end = end;
        }
    }
}

/// Checks the returned range and returns its exclusive end and the full file length.
async fn cdn_range(url: &Url, start: u64, length: u64) -> Result<(Response, u64, u64)> {
    let last = start + length - 1;
    let response = transport::request(|client| {
        client
            .get(url.clone())
            .header(header::RANGE, format!("bytes={start}-{last}"))
    })
    .await
    .map_err(reqwest::Error::without_url)?;
    anyhow::ensure!(
        response.status() == StatusCode::PARTIAL_CONTENT,
        "Spotify CDN rejected an audio range ({})",
        response.status()
    );
    let range = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .and_then(|value| value.split_once('/'))
        .context("Spotify CDN returned no valid Content-Range")?;
    let (first, end) = range
        .0
        .split_once('-')
        .context("invalid Spotify CDN range")?;
    let total: u64 = range.1.parse().context("invalid Spotify CDN file length")?;
    let end: u64 = end.parse().context("invalid Spotify CDN range end")?;
    anyhow::ensure!(
        first.parse::<u64>()? == start && total > start && end == last.min(total - 1),
        "Spotify CDN returned the wrong audio range"
    );
    Ok((response, end + 1, total))
}
