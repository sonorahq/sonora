//! Spotify tracks downloaded for offline play, kept the way Spotify's own clients keep theirs:
//! the file as the CDN sent it, still encrypted, with its key and metadata beside it. The vault
//! seals all of that again under the vault's master key, so a download plays outside Sonora
//! only for someone who holds that key, and for them it is as easy to play as any file.
//!
//! A track streaming from the CDN is decrypted as it lands, and a seek past what has arrived
//! starts a second download near the target through [`ranged`].

use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use crate::spotify::{auth::sign_in_error, collection, session::Session, wire};
use aes::cipher::StreamCipherSeek as _;
use aes::cipher::{KeyIvInit as _, StreamCipher as _};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::HeaderMap;
use rodio::Source as _;
use serde::{Deserialize, Serialize};
use spotifyapis::access_point::AudioKeyError;
use spotifyapis::proto::extended_metadata::Track as TrackMessage;
use spotifyapis::proto::extended_metadata::{ExtensionKind, MetadataPayload};

use crate::Quality;
use crate::engine::{self, Fetch, Loudness};
use crate::ranged::{self, Address, Reach, Resuming, Span};
use crate::stream::{Body, Source, Stream};
use crate::vault::{Recording, Shelf};
use crate::{Encoding, Fraction, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

const VORBIS_HEADER_LEN: u64 = 0xa7;
const VORBIS_GAIN_OFFSET: u64 = 144;
const MEDIA_COUNTER: [u8; 16] = [
    0x72, 0xe0, 0x67, 0xfb, 0xdd, 0xcb, 0xcf, 0x77, 0xeb, 0xe8, 0xbc, 0x64, 0x3f, 0x63, 0x0d, 0x93,
];

/// The most one request to the CDN asks for.
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
    arriving: Option<Stream<Unlocked>>,
    /// What a seek past the download fetches the file through. `None` for a track from the
    /// vault, which is whole.
    reach: Option<Arc<Reach<Unlocked>>>,
    format: i32,
}

/// Decrypts a file from the CDN as it lands, so the spool holds it in the clear, Spotify's own
/// header included. The keystream counts from the file's first byte, so a body for a download
/// that starts part way has to know where.
pub(super) struct Unlocked {
    cipher: Cipher,
}

/// What a download asks Spotify for a fresh CDN url of the file it is fetching with.
struct Renewal {
    session: Session,
    file_id: String,
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
            if self.session.is_none() || shelf.holds(id) {
                let shelf = shelf.clone();
                let id = id.to_owned();
                let saved = tokio::task::spawn_blocking(move || {
                    read_saved(Some(&shelf), &id)
                        .map(|(facts, mut media)| Loaded::from_facts(&facts, media.loudness()))
                })
                .await
                .context("cannot open Spotify download")?;
                if saved.is_ok() || self.session.is_none() {
                    return saved;
                }
            }
        }
        let session = self
            .session
            .as_ref()
            .context("Spotify playback requires sign-in")?;
        let variant = variant(self.quality);
        if let Some(shelf) = &self.shelf
            && let Some(kept) = shelf.find(id, variant).await
            && let Ok(facts) = serde_json::from_slice::<Facts>(&kept.facts)
        {
            return Loaded::streaming(&facts, kept.source, Some(kept.length), None).await;
        }
        let mut media = resolve_media(session, id, self.quality).await?;
        let (response, address) = open_file(session, &media.facts.file_id).await?;
        media.fetch_key(session).await?;
        crate::soloist::granted();
        let facts = media.facts;
        let total = response.total();
        let reach = facts.key.map(|key| {
            Reach::new("spotify", id, move |start| {
                (address.clone(), Unlocked::at(&key, start))
            })
        });
        let source = match &self.shelf {
            Some(shelf) => {
                let facts = serde_json::to_vec(&facts)?;
                shelf.keep(id, variant, move || Some(facts), response)
            }
            None => Recording::through(response),
        };
        Loaded::streaming(&facts, source, total, reach).await
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
        if error.downcast_ref::<AudioKeyError>() == Some(&AudioKeyError::Denied) {
            crate::soloist::denied();
            return true;
        }
        false
    }

    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let mut media = match &loaded.arriving {
            Some(stream) => {
                Media::new(stream.reader(), stream.total(), None, loaded.format).ok()?
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

    /// Opens a track still arriving part way at `at`, from a download started near `at` when
    /// the main one is far from it, so a seek past the download waits for one short stretch.
    /// Otherwise the decoder waits for the main download to get there, as
    /// [`open`](Fetch::open) does.
    fn open_near(
        &self,
        id: &str,
        loaded: &Loaded,
        at: Duration,
    ) -> Option<(Self::Source, Duration)> {
        let entered = loaded
            .arriving
            .as_ref()
            .zip(loaded.reach.as_ref())
            .zip(loaded.duration)
            .filter(|((stream, _), _)| ranged::ahead(stream, at))
            .and_then(|((stream, reach), length)| {
                let span = Span::measure(stream, length, |spool| {
                    ranged::ogg::head_after(spool, VORBIS_HEADER_LEN as usize)
                })?;
                reach.enter(stream, span, at, |reader| {
                    let mut media = Media::new(reader, None, None, loaded.format).ok()?;
                    media.seek(SeekFrom::Start(0)).ok()?;
                    ranged::forward("spotify", id, BufReader::new(media))
                })
            });
        match entered {
            Some(decoder) => Some((decoder, at)),
            None => self.open(id, loaded, at).map(|source| (source, at)),
        }
    }
}

impl Loaded {
    /// Primes either the cached or CDN stream before opening a decoder, decrypting it on the
    /// way into the spool.
    async fn streaming(
        facts: &Facts,
        source: impl Source,
        total: Option<u64>,
        reach: Option<Reach<Unlocked>>,
    ) -> Result<Self> {
        let key = facts.key.context("Spotify gave no key for the track")?;
        let stream = Stream::pulling(source, total, Unlocked::at(&key, 0))
            .primed()
            .await?;
        let mut media = Media::new(stream.reader(), total, None, facts.format)?;
        let mut loaded = Self::from_facts(facts, media.loudness());
        loaded.arriving = Some(stream);
        loaded.reach = reach.map(Arc::new);
        Ok(loaded)
    }

    fn from_facts(facts: &Facts, loudness: Option<Loudness>) -> Self {
        let duration = STANDARD
            .decode(&facts.metadata)
            .ok()
            .and_then(|raw| {
                MetadataPayload::from_bytes(raw.into())
                    .decode::<TrackMessage>()
                    .ok()
            })
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
            reach: None,
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
    if shelf.holds(id) || shelf.pin_cached(id, variant(quality)).await {
        progress.set(1.0);
        return Ok(());
    }

    let mut media = resolve_media(session, id, quality).await?;
    if session.audio_keys().throttled()
        && let Err(error) = media.fetch_key(session).await
        && (error.downcast_ref::<AudioKeyError>() == Some(&AudioKeyError::Denied)
            || error.is::<crate::Throttled>())
    {
        return Err(error);
    }
    let (mut response, _) = open_file(session, &media.facts.file_id).await?;
    let total = response
        .total()
        .context("Spotify CDN did not say how long the file is")?;
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
                .get(self.track, self.file)
                .await
                .map_err(sign_in_error)?,
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
    let (playable, wire, (format, file)) = match vorbis(&track, quality) {
        Some(file) => (track, wire, file),
        None => {
            let alternatives: Vec<_> = track
                .alternative
                .iter()
                .filter_map(|track| spotifyapis::id::base62(track.gid()))
                .map(|id| format!("spotify:track:{id}"))
                .collect();
            metadata(session, &alternatives)
                .await?
                .into_iter()
                .find_map(|(track, wire)| {
                    let file = vorbis(&track, quality)?;
                    Some((track, wire, file))
                })
                .context("Spotify has no playable download for this track")?
        }
    };
    let track = <[u8; 16]>::try_from(playable.gid()).context("invalid Spotify audio track ID")?;
    let mut file_id = String::with_capacity(file.len() * 2);
    wire::append_hex(&mut file_id, &file);
    Ok(ResolvedMedia {
        track,
        file,
        facts: Facts {
            metadata: STANDARD.encode(wire.as_bytes()),
            format,
            file_id,
            key: None,
        },
    })
}

/// Resolves the file's signed CDN urls and opens it at the first that answers, and says where.
async fn open_file(session: &Session, file_id: &str) -> Result<(Resuming, Address)> {
    let http = spotifyapis::media_client()?;
    let renewal: Arc<dyn ranged::Renew> = Arc::new(Renewal {
        session: session.clone(),
        file_id: file_id.to_owned(),
    });
    for url in session
        .api()
        .audio_urls(file_id)
        .await
        .map_err(sign_in_error)?
    {
        let address = Address::new("spotify", http.clone(), url)
            .capped(MAX_RANGE)
            .renewed(renewal.clone());
        let Ok(file) = Resuming::open(address.clone()).await else {
            continue;
        };
        anyhow::ensure!(
            file.total().is_some_and(|total| total <= MAX_DOWNLOAD),
            "Spotify track is too large"
        );
        return Ok((file, address));
    }
    anyhow::bail!("cannot open Spotify audio on its CDN")
}

/// Batches track metadata for the requested track and, when needed, its market alternatives.
async fn metadata(
    session: &Session,
    uris: &[String],
) -> Result<Vec<(TrackMessage, MetadataPayload)>> {
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
            Some((raw.decode().ok()?, raw))
        })
        .collect())
}

fn variant(quality: Quality) -> &'static str {
    match quality {
        Quality::Low => "vorbis-96",
        Quality::Normal => "vorbis-160",
        _ => "vorbis-320",
    }
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

impl Unlocked {
    /// A body for a download that starts at byte `start` of the file.
    fn at(key: &[u8; 16], start: u64) -> Self {
        let mut cipher = Cipher::new(&(*key).into(), &MEDIA_COUNTER.into());
        cipher.seek(start);
        Self { cipher }
    }
}

impl Body for Unlocked {
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        let at = out.len();
        out.extend_from_slice(chunk);
        self.cipher.apply_keystream(&mut out[at..]);
    }

    fn confidential(&self) -> bool {
        true
    }
}

#[async_trait]
impl ranged::Renew for Renewal {
    async fn url(&self) -> Result<(String, HeaderMap)> {
        let urls = self
            .session
            .api()
            .audio_urls(&self.file_id)
            .await
            .map_err(sign_in_error)?;
        let url = urls.into_iter().next().context("Spotify gave no CDN url")?;
        Ok((url, HeaderMap::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_ahead_decrypts_at_the_same_file_offset() {
        let key = [0x34; 16];
        let bytes = [0x56; 64];
        let mut whole = Vec::new();
        Unlocked::at(&key, 0).feed(&bytes, &mut whole);

        let mut ranged = Vec::new();
        Unlocked::at(&key, 19).feed(&bytes[19..], &mut ranged);
        assert_eq!(ranged, whole[19..]);
    }
}
